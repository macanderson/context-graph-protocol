//! Stdio transport: a child-process Context Graph Protocol provider spoken to over its
//! stdin/stdout (`SPEC.md` §3 "local providers: child
//! processes over stdio").
//!
//! Two layers:
//!
//! - [`RawStdioConnection`] — the low-level framed pipe. Public because
//!   conformance tooling needs byte-level control (e.g. injecting a
//!   malformed line to probe provider robustness, SPEC.md §11). It owns the child,
//!   spawns it under the Context Graph Protocol isolation contract, and guarantees the process
//!   group dies on drop/shutdown.
//! - [`StdioProvider`] — a [`ContextProvider`] built on the connection: it
//!   handshakes once, caches the provider's identity + capabilities, and then
//!   splits the pipe into independently-lockable halves. A dedicated reader
//!   task demultiplexes replies on their correlation `id`, and the write half
//!   is locked only for the length of one line — so a provider that negotiated
//!   `correlation` can have several queries in flight at once (a slow one no
//!   longer head-of-line blocks the rest), while a non-correlating provider and
//!   every `verify` stay strictly lock-step, behaving exactly as the original
//!   single-mutex transport did ([ADR 0002](../../docs/adr/0002-request-correlation-and-the-json-rpc-question.md)).
//!
//! ## Isolation (`SPEC.md` §4 and §10, `SPEC.md` §7)
//!
//! The child is spawned with a **scrubbed environment** — `env_clear()` then
//! an allowlist of only `PATH` (so the program resolves) and `HOME`. No
//! inherited credentials, no ambient secrets: a provider sees exactly the
//! query payload and whatever it indexed through its own declared inputs,
//! nothing the host holds. On Unix the child leads its own process group so
//! the whole subtree is signalled at once and can never outlive the host.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use async_trait::async_trait;
use contextgraph_types::{
    Capabilities, ContextQuery, ContextQueryResult, PROTOCOL_VERSION, ProviderInfo, VerifyRequest,
    VerifyResponse,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{Mutex as TokioMutex, oneshot};
use tokio::task::JoinHandle;

use crate::error::HostError;
use crate::provider::ContextProvider;
use crate::wire::{
    Envelope, decode_line, encode_line, envelope_kind, next_correlation_id, verify_correlation,
    versions_compatible,
};

/// How long the handshake waits for a provider's ack before giving up —
/// bounds the "version mismatch = never a hang" guarantee even against a
/// provider that never answers (task deliverable 1).
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a graceful `shutdown` waits for the child to exit before the
/// process-group kill backstop fires (task deliverable 2).
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

/// Maximum bytes accepted for a single framed line before the child is treated
/// as malformed. Guards the host against a provider that streams without ever
/// emitting a newline (the timeouts bound time, not memory). 16 MiB is far
/// above any legitimate framed message.
const MAX_LINE_BYTES: usize = 16 * 1024 * 1024;

/// Read one NDJSON line from a provider's stdout, or `None` at EOF, bounded to
/// [`MAX_LINE_BYTES`] via an incremental `fill_buf`/`consume` loop so a child
/// that streams without ever emitting a newline cannot OOM the host.
///
/// Shared by [`RawStdioConnection::read_raw_line`] and the [`StdioProvider`]
/// reader task, so the memory bound has exactly one implementation.
async fn read_framed_line(
    stdout: &mut BufReader<ChildStdout>,
    label: &str,
) -> Result<Option<String>, HostError> {
    let transport = |message: String| HostError::Transport {
        id: label.to_string(),
        message,
    };
    let mut bytes: Vec<u8> = Vec::new();
    loop {
        let buf = stdout
            .fill_buf()
            .await
            .map_err(|e| transport(e.to_string()))?;
        if buf.is_empty() {
            break; // EOF — deliver any final unterminated line, else None.
        }
        if let Some(pos) = buf.iter().position(|&b| b == b'\n') {
            bytes.extend_from_slice(&buf[..=pos]);
            stdout.consume(pos + 1);
            break;
        }
        bytes.extend_from_slice(buf);
        let consumed = buf.len();
        stdout.consume(consumed);
        if bytes.len() > MAX_LINE_BYTES {
            return Err(transport(format!(
                "provider emitted a line exceeding {MAX_LINE_BYTES} bytes without a newline"
            )));
        }
    }
    if bytes.is_empty() {
        Ok(None)
    } else {
        Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
    }
}

/// Write an already-framed line to a provider's stdin, appending a trailing
/// `\n` if missing. A closed pipe (the child is gone) surfaces as
/// [`HostError::ProviderCrashed`] — the write-side twin of the read-side EOF —
/// and any other IO error as [`HostError::Transport`], so both halves report a
/// dead child the same way. Shared by [`RawStdioConnection::send_raw_line`] and
/// the [`StdioProvider`] write path.
async fn write_framed_line(
    stdin: &mut ChildStdin,
    line: &str,
    label: &str,
) -> Result<(), HostError> {
    let transport = |e: std::io::Error| match e.kind() {
        std::io::ErrorKind::BrokenPipe => HostError::ProviderCrashed {
            id: label.to_string(),
        },
        _ => HostError::Transport {
            id: label.to_string(),
            message: e.to_string(),
        },
    };
    stdin.write_all(line.as_bytes()).await.map_err(transport)?;
    if !line.ends_with('\n') {
        stdin.write_all(b"\n").await.map_err(transport)?;
    }
    stdin.flush().await.map_err(transport)?;
    Ok(())
}

/// Encode `env` and write it as one NDJSON line to `stdin`.
async fn write_envelope(
    stdin: &mut ChildStdin,
    env: &Envelope,
    label: &str,
) -> Result<(), HostError> {
    let line = encode_line(env)?;
    write_framed_line(stdin, &line, label).await
}

/// A raw, framed connection to a child-process Context Graph Protocol provider. The low-level
/// primitive [`StdioProvider`] is built on; public so conformance tools can
/// drive the wire directly.
pub struct RawStdioConnection {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    child: Child,
    /// Process-group id (== the child pid, which `setsid` made a group
    /// leader) for the backstop kill. `None` off Unix, where `kill_on_drop`
    /// reaps the direct child only.
    #[cfg_attr(not(unix), allow(dead_code))]
    pgid: Option<i32>,
    /// A stable label for error messages before the handshake names the
    /// provider.
    label: String,
}

impl RawStdioConnection {
    /// Spawn `program` with `args` as a CGP provider child, under the
    /// isolation contract (scrubbed env, own process group). Does **not**
    /// handshake — call [`RawStdioConnection::handshake`] next.
    pub async fn spawn(program: &str, args: &[String]) -> Result<Self, HostError> {
        let mut cmd = Command::new(program);
        cmd.args(args);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        // Provider diagnostics flow to the host's own stderr — never captured
        // as frames, never mistaken for protocol.
        cmd.stderr(Stdio::inherit());
        cmd.kill_on_drop(true);

        // Scrub the environment: no inherited credentials. Allowlist
        // only PATH (so `program` resolves) and HOME.
        cmd.env_clear();
        if let Ok(path) = std::env::var("PATH") {
            cmd.env("PATH", path);
        }
        if let Ok(home) = std::env::var("HOME") {
            cmd.env("HOME", home);
        }

        // New session/process group so the whole subtree can be signalled at
        // once on drop/shutdown.
        #[cfg(unix)]
        {
            // SAFETY: `setsid` is async-signal-safe and only reparents the
            // child's own process-group membership in the window between fork
            // and exec — the same narrowly-scoped OS-boundary use
            // `stella-tools`' bash tool makes.
            unsafe {
                cmd.pre_exec(|| {
                    libc::setsid();
                    Ok(())
                });
            }
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| HostError::Spawn(format!("{program}: {e}")))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| HostError::Spawn(format!("{program}: child has no stdin pipe")))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| HostError::Spawn(format!("{program}: child has no stdout pipe")))?;

        #[cfg(unix)]
        let pgid = child.id().map(|id| id as i32);
        #[cfg(not(unix))]
        let pgid = None;

        Ok(Self {
            stdin,
            stdout: BufReader::new(stdout),
            child,
            pgid,
            label: program.to_string(),
        })
    }

    /// Override the connection's error label (defaults to the program name).
    /// [`StdioProvider`] sets it to the provider's host-facing id.
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// Send one envelope as an NDJSON line.
    pub async fn send(&mut self, env: &Envelope) -> Result<(), HostError> {
        let line = encode_line(env)?;
        self.send_raw_line(&line).await
    }

    /// Write a raw line to the provider's stdin verbatim — the escape hatch
    /// conformance uses to inject a malformed line (SPEC.md §11). A trailing `\n` is
    /// appended if missing so the provider's line reader unblocks.
    pub async fn send_raw_line(&mut self, line: &str) -> Result<(), HostError> {
        // A write into a closed stdin means the child is gone — the write-side
        // twin of the read-side EOF in `recv`, so both surface as
        // ProviderCrashed. Delegated to the shared writer so the raw path and
        // the pipelined `StdioProvider` path frame lines identically.
        write_framed_line(&mut self.stdin, line, &self.label).await
    }

    /// Read the next raw line, or `None` at EOF (the child closed stdout).
    ///
    /// Bounded to [`MAX_LINE_BYTES`] via an incremental `fill_buf`/`consume`
    /// loop: a buggy or hostile child that streams bytes without ever emitting
    /// a newline would otherwise grow a single `String` without limit (the
    /// handshake/query timeouts bound *time*, not *memory*) and OOM the host.
    pub async fn read_raw_line(&mut self) -> Result<Option<String>, HostError> {
        // Delegated to the shared reader so the `MAX_LINE_BYTES` memory bound
        // has one implementation across the raw path and the pipelined
        // `StdioProvider` reader task.
        read_framed_line(&mut self.stdout, &self.label).await
    }

    /// Read the next envelope. A closed stream (the child died) is
    /// [`HostError::ProviderCrashed`] — never a hang or a panic
    /// (task deliverable 5).
    pub async fn recv(&mut self) -> Result<Envelope, HostError> {
        match self.read_raw_line().await? {
            Some(line) => decode_line(&line),
            None => Err(HostError::ProviderCrashed {
                id: self.label.clone(),
            }),
        }
    }

    /// Perform the Context Graph Protocol handshake (SPEC.md §3): send `handshake`, expect
    /// `handshake_ack`, and reject an incompatible protocol version with a
    /// named error. Bounded by [`HANDSHAKE_TIMEOUT`] so a silent provider
    /// fails cleanly rather than hanging (task deliverable 1).
    pub async fn handshake(&mut self) -> Result<(ProviderInfo, Capabilities), HostError> {
        self.send(&Envelope::Handshake {
            protocol_version: PROTOCOL_VERSION.to_string(),
        })
        .await?;

        let ack = match tokio::time::timeout(HANDSHAKE_TIMEOUT, self.recv()).await {
            Ok(result) => result?,
            Err(_) => {
                return Err(HostError::Timeout {
                    id: self.label.clone(),
                    timeout_ms: HANDSHAKE_TIMEOUT.as_millis() as u64,
                });
            }
        };

        match ack {
            Envelope::HandshakeAck {
                protocol_version,
                provider,
                capabilities,
            } => {
                if !versions_compatible(PROTOCOL_VERSION, &protocol_version) {
                    return Err(HostError::VersionMismatch {
                        host: PROTOCOL_VERSION.to_string(),
                        provider: provider.name,
                        provider_version: protocol_version,
                    });
                }
                Ok((provider, capabilities))
            }
            other => Err(HostError::UnexpectedEnvelope {
                id: self.label.clone(),
                expected: "handshake_ack".into(),
                got: envelope_kind(&other).into(),
            }),
        }
    }

    /// Send `shutdown` and wait a bounded grace for the child to exit,
    /// killing the process group if it overstays (task deliverable 2). A
    /// provider that already died is not treated as a shutdown error.
    pub async fn shutdown(&mut self) -> Result<(), HostError> {
        let _ = self.send(&Envelope::Shutdown).await;
        let label = self.label.clone();
        match tokio::time::timeout(SHUTDOWN_GRACE, self.child.wait()).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(HostError::Transport {
                id: label,
                message: e.to_string(),
            }),
            Err(_) => {
                self.kill_group();
                Ok(())
            }
        }
    }

    /// SIGKILL the whole process group (Unix) and the direct child. Idempotent
    /// — signalling an already-dead group is a harmless, ignored `ESRCH`.
    fn kill_group(&mut self) {
        #[cfg(unix)]
        if let Some(pgid) = self.pgid {
            // SAFETY: `-pgid` targets the process group this connection
            // created via `setsid`; a stale/dead group returns `ESRCH`,
            // which we ignore.
            unsafe {
                libc::kill(-pgid, libc::SIGKILL);
            }
        }
        let _ = self.child.start_kill();
    }

    /// Decompose the connection into the independently-owned halves the
    /// pipelined [`StdioProvider`] runs on: the write half ([`ChildStdin`]), the
    /// read half ([`BufReader<ChildStdout>`]), and a [`StdioControl`] over the
    /// child and its process group. Any bytes the `BufReader` buffered past the
    /// handshake travel with the read half, so nothing is lost across the split.
    ///
    /// Consumes `self` **without** running [`Drop`] — its `Drop` kills the
    /// process group, and here we are keeping the child alive to keep talking to
    /// it. Private: this is `StdioProvider`'s internal seam, not part of the
    /// public raw send/recv API conformance tooling drives.
    fn into_parts(self) -> (ChildStdin, BufReader<ChildStdout>, StdioControl) {
        // `RawStdioConnection: Drop`, so its fields cannot be moved out by an
        // ordinary destructuring move. Suppress the destructor and read each
        // field out exactly once instead.
        let this = std::mem::ManuallyDrop::new(self);
        // SAFETY: every non-`Copy` field is read out exactly once via
        // `ptr::read`; `this` is a `ManuallyDrop`, so its destructor never runs
        // and no field is dropped twice; and `this` is never touched again after
        // this block. `pgid` is `Copy` and read by value.
        unsafe {
            let stdin = std::ptr::read(&this.stdin);
            let stdout = std::ptr::read(&this.stdout);
            let child = std::ptr::read(&this.child);
            let label = std::ptr::read(&this.label);
            let pgid = this.pgid;
            (stdin, stdout, StdioControl { child, pgid, label })
        }
    }
}

impl Drop for RawStdioConnection {
    fn drop(&mut self) {
        // Backstop: even if a caller forgot `shutdown`, the child tree dies
        // with the host (`SPEC.md` §8 — no orphaned children).
        self.kill_group();
    }
}

/// The child + its process group, held by a [`StdioProvider`] solely for
/// teardown. Splitting it out of the connection is what lets `query`/`verify`
/// touch only the stdin and reader halves, while `shutdown` (and `Drop`) retain
/// exclusive control of the process — preserving the original
/// [`SHUTDOWN_GRACE`] + `kill_group` teardown exactly.
struct StdioControl {
    child: Child,
    /// Process-group id (== the child pid made a group leader by `setsid`) for
    /// the backstop kill. `None` off Unix.
    #[cfg_attr(not(unix), allow(dead_code))]
    pgid: Option<i32>,
    /// The provider's host-facing id, for teardown error messages.
    label: String,
}

impl StdioControl {
    /// Wait a bounded [`SHUTDOWN_GRACE`] for the child to exit, killing the
    /// process group if it overstays. The caller sends the `shutdown` envelope
    /// over stdin first; this is the grace-then-kill backstop, byte for byte the
    /// tail of the original `RawStdioConnection::shutdown`.
    async fn wait_or_kill(&mut self) -> Result<(), HostError> {
        match tokio::time::timeout(SHUTDOWN_GRACE, self.child.wait()).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(HostError::Transport {
                id: self.label.clone(),
                message: e.to_string(),
            }),
            Err(_) => {
                self.kill_group();
                Ok(())
            }
        }
    }

    /// SIGKILL the whole process group (Unix) and the direct child. Idempotent —
    /// signalling an already-dead group is a harmless, ignored `ESRCH`.
    /// Identical to `RawStdioConnection::kill_group`.
    fn kill_group(&mut self) {
        #[cfg(unix)]
        if let Some(pgid) = self.pgid {
            // SAFETY: `-pgid` targets the process group created via `setsid`; a
            // stale/dead group returns `ESRCH`, which we ignore.
            unsafe {
                libc::kill(-pgid, libc::SIGKILL);
            }
        }
        let _ = self.child.start_kill();
    }
}

impl Drop for StdioControl {
    fn drop(&mut self) {
        // Backstop: the child tree dies with the host even if `shutdown` was
        // never called (`SPEC.md` §8 — no orphaned children).
        self.kill_group();
    }
}

/// A reply delivered to an in-flight exchange: the decoded envelope, or the
/// error that ended the connection.
type Reply = Result<Envelope, HostError>;

/// The table of correlated exchanges awaiting their reply, keyed by the `id` the
/// host minted. The reader task removes and fulfills the matching sender as each
/// `frames`/`error` arrives.
type PendingTable = Arc<StdMutex<HashMap<String, oneshot::Sender<Reply>>>>;

/// The single fallback slot for an id-less reply (a non-correlating `query`, or
/// any `verify`). Serialized by [`StdioProvider`]'s `no_id_lock`, so at most one
/// sender is ever registered at a time.
type NoIdSlot = Arc<StdMutex<Option<oneshot::Sender<Reply>>>>;

/// Why the reader loop is terminating. `HostError` is not `Clone`, so the loop
/// carries the *reason* and mints a fresh error of the right shape for each
/// waiter it drains.
enum ReaderExit {
    /// The child closed stdout mid-exchange — it crashed or exited.
    Crashed,
    /// A transport error reading stdout.
    Transport(String),
    /// A line that would not decode into an envelope.
    Decode(String),
}

impl ReaderExit {
    fn error(&self, label: &str) -> HostError {
        match self {
            ReaderExit::Crashed => HostError::ProviderCrashed {
                id: label.to_string(),
            },
            ReaderExit::Transport(message) => HostError::Transport {
                id: label.to_string(),
                message: message.clone(),
            },
            ReaderExit::Decode(message) => HostError::Wire(message.clone()),
        }
    }
}

/// The [`StdioProvider`] reader task. It owns the read half for the life of the
/// connection and is the *only* reader, so replies can be demultiplexed on
/// `id`. For each envelope it either matches a correlated waiter in `pending` or
/// hands an id-less reply to the single `no_id_slot`. On EOF, or a decode /
/// transport error, it drains **every** waiter with a terminal error so no
/// in-flight `query`/`verify` can hang past the connection's death (ADR 0002 —
/// the crash-consistency contract).
async fn run_reader(
    mut stdout: BufReader<ChildStdout>,
    label: String,
    pending: PendingTable,
    no_id_slot: NoIdSlot,
) {
    loop {
        let exit = match read_framed_line(&mut stdout, &label).await {
            Ok(Some(line)) => match decode_line(&line) {
                Ok(env) => {
                    dispatch(env, &pending, &no_id_slot, &label);
                    continue;
                }
                // A line we cannot attribute to any waiter: the stream is no
                // longer trustworthy, so fail every exchange rather than let
                // them hang on a reply that will never parse.
                Err(err) => ReaderExit::Decode(err.to_string()),
            },
            // EOF: the child closed stdout. Every waiter must learn, or
            // query()/verify() would hang forever.
            Ok(None) => ReaderExit::Crashed,
            Err(HostError::Transport { message, .. }) => ReaderExit::Transport(message),
            Err(other) => ReaderExit::Transport(other.to_string()),
        };
        drain_waiters(&pending, &no_id_slot, &exit, &label);
        return;
    }
}

/// Route one decoded provider→host envelope to its waiter. A `frames`/`error`
/// carrying an `id` is demultiplexed against `pending`; anything else (an
/// id-less `frames`/`error`, a `verified`, or an unexpected envelope) goes to
/// the lock-step `no_id_slot`. A reply with no matching waiter is logged to the
/// host's stderr and dropped — never a panic (ADR 0002: an unmatched or stale
/// id must not take the connection down).
fn dispatch(env: Envelope, pending: &PendingTable, no_id_slot: &NoIdSlot, label: &str) {
    let correlated = match &env {
        Envelope::Frames { id: Some(id), .. } | Envelope::Error { id: Some(id), .. } => {
            Some(id.clone())
        }
        _ => None,
    };
    if let Some(id) = correlated {
        let waiter = pending.lock().expect("pending mutex poisoned").remove(&id);
        match waiter {
            Some(tx) => {
                let _ = tx.send(Ok(env));
            }
            None => eprintln!(
                "contextgraph-host: stdio provider `{label}` sent a reply with id `{id}` matching no in-flight query; dropping"
            ),
        }
        return;
    }
    let waiter = no_id_slot.lock().expect("no_id_slot mutex poisoned").take();
    match waiter {
        Some(tx) => {
            let _ = tx.send(Ok(env));
        }
        None => eprintln!(
            "contextgraph-host: stdio provider `{label}` sent an unsolicited `{}` envelope with no in-flight lock-step exchange; dropping",
            envelope_kind(&env)
        ),
    }
}

/// Deliver a terminal error to every waiter — the correlated `pending` table and
/// the id-less slot — so a dead or garbage-emitting provider fails all its
/// in-flight exchanges instead of hanging them (ADR 0002).
fn drain_waiters(pending: &PendingTable, no_id_slot: &NoIdSlot, exit: &ReaderExit, label: &str) {
    let waiters: Vec<oneshot::Sender<Reply>> = {
        let mut map = pending.lock().expect("pending mutex poisoned");
        map.drain().map(|(_, tx)| tx).collect()
    };
    for tx in waiters {
        let _ = tx.send(Err(exit.error(label)));
    }
    let leftover = { no_id_slot.lock().expect("no_id_slot mutex poisoned").take() };
    if let Some(tx) = leftover {
        let _ = tx.send(Err(exit.error(label)));
    }
}

/// A [`ContextProvider`] backed by a child process over stdio.
///
/// Handshakes once on construction, caches the negotiated identity +
/// capabilities, then splits the connection into independently-lockable halves:
/// the write half (`stdin`) is locked only for the length of one line write, a
/// dedicated reader task owns the read half and demultiplexes replies on their
/// correlation `id`, and a [`StdioControl`] holds the child for teardown. A
/// provider that negotiated [`Capabilities::correlation`](contextgraph_types::Capabilities::correlation)
/// can therefore have several `query`s in flight at once — a slow one no longer
/// head-of-line blocks the rest. A provider that did **not** negotiate
/// correlation, and every `verify` (whose envelopes carry no `id` and so cannot
/// be demultiplexed), stay strictly lock-step via `no_id_lock`, behaving exactly
/// as the single-mutex transport did before (ADR 0002).
pub struct StdioProvider {
    id: String,
    info: ProviderInfo,
    capabilities: Capabilities,
    /// Write half. Locked only long enough to write one framed line, then
    /// released — a correlated `query` holds it for a send, never a round-trip.
    stdin: TokioMutex<ChildStdin>,
    /// Correlated in-flight exchanges, keyed by the host-minted `id`.
    pending: PendingTable,
    /// The fallback slot the reader delivers id-less replies to.
    no_id_slot: NoIdSlot,
    /// Serializes the id-less / non-correlating exchanges (a non-correlating
    /// `query`, all `verify`) into lock-step, so at most one id-less reply is
    /// outstanding and a non-correlating provider is provably unchanged.
    no_id_lock: TokioMutex<()>,
    /// Child + process group, touched only by `shutdown`/`Drop`.
    control: TokioMutex<StdioControl>,
    /// The reader task; aborted on `Drop` as a backstop (the child's death
    /// already ends it via EOF).
    reader: JoinHandle<()>,
}

impl StdioProvider {
    /// Spawn a child-process provider, complete the handshake, and cache its
    /// declared identity + capabilities. `id` is the host-facing routing and
    /// consent key. Fails cleanly (killing the child) on a bad or incompatible
    /// handshake. On success the connection is split and the reader task
    /// launched, so replies can be demultiplexed from here on.
    pub async fn spawn(
        id: impl Into<String>,
        program: &str,
        args: &[String],
    ) -> Result<Self, HostError> {
        let id = id.into();
        let mut conn = RawStdioConnection::spawn(program, args)
            .await?
            .with_label(id.clone());
        let (info, capabilities) = conn.handshake().await?;

        // Handshake done: split the connection. The `BufReader` carries any
        // bytes it buffered past the ack, so nothing is lost across the move.
        let (stdin, stdout, control) = conn.into_parts();

        let pending: PendingTable = Arc::new(StdMutex::new(HashMap::new()));
        let no_id_slot: NoIdSlot = Arc::new(StdMutex::new(None));
        let reader = tokio::spawn(run_reader(
            stdout,
            id.clone(),
            Arc::clone(&pending),
            Arc::clone(&no_id_slot),
        ));

        Ok(Self {
            id,
            info,
            capabilities,
            stdin: TokioMutex::new(stdin),
            pending,
            no_id_slot,
            no_id_lock: TokioMutex::new(()),
            control: TokioMutex::new(control),
            reader,
        })
    }

    /// Lock-step exchange for an id-less request (a non-correlating `query`, or
    /// any `verify`): hold `no_id_lock` across the whole round-trip so exactly
    /// one id-less reply is outstanding, register the fallback slot, write the
    /// request, and await the reader's delivery. Byte-for-byte the behaviour of
    /// the old single-mutex path, so a non-correlating provider is unchanged.
    async fn exchange_lockstep(&self, request: Envelope) -> Result<Envelope, HostError> {
        let _lockstep = self.no_id_lock.lock().await;
        let (tx, rx) = oneshot::channel();
        // Safe to overwrite: `no_id_lock` guarantees the slot is empty here.
        *self.no_id_slot.lock().expect("no_id_slot mutex poisoned") = Some(tx);

        let sent = {
            let mut stdin = self.stdin.lock().await;
            write_envelope(&mut stdin, &request, &self.id).await
        };
        if let Err(e) = sent {
            // Undo the registration so a failed write cannot leak the slot.
            self.no_id_slot
                .lock()
                .expect("no_id_slot mutex poisoned")
                .take();
            return Err(e);
        }

        match rx.await {
            Ok(reply) => reply,
            // The reader drains on exit, so a canceled receiver means the task
            // is gone without having delivered — a crash, never a hang.
            Err(_) => Err(HostError::ProviderCrashed {
                id: self.id.clone(),
            }),
        }
    }
}

#[async_trait]
impl ContextProvider for StdioProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn info(&self) -> &ProviderInfo {
        &self.info
    }

    fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    async fn query(&self, query: &ContextQuery) -> Result<ContextQueryResult, HostError> {
        if !self.capabilities.correlation {
            // Non-correlating provider: lock-step, provably identical to before.
            return match self
                .exchange_lockstep(Envelope::Query {
                    id: None,
                    query: query.clone(),
                })
                .await?
            {
                Envelope::Frames { result, .. } => Ok(result),
                Envelope::Error { message, code, .. } => Err(HostError::Provider {
                    id: self.id.clone(),
                    code,
                    message,
                }),
                other => Err(HostError::UnexpectedEnvelope {
                    id: self.id.clone(),
                    expected: "frames".into(),
                    got: envelope_kind(&other).into(),
                }),
            };
        }

        // Correlated: register the waiter keyed by a fresh id BEFORE sending, so
        // the reader can never deliver a reply we have not yet recorded. Then
        // lock stdin only long enough to write the line, and await the reply
        // with NO lock held — this is what lets two queries interleave.
        let sent_id = next_correlation_id();
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .expect("pending mutex poisoned")
            .insert(sent_id.clone(), tx);

        let sent = {
            let mut stdin = self.stdin.lock().await;
            write_envelope(
                &mut stdin,
                &Envelope::Query {
                    id: Some(sent_id.clone()),
                    query: query.clone(),
                },
                &self.id,
            )
            .await
        };
        if let Err(e) = sent {
            // Undo the registration so a failed write cannot leak a waiter.
            self.pending
                .lock()
                .expect("pending mutex poisoned")
                .remove(&sent_id);
            return Err(e);
        }

        let reply = match rx.await {
            Ok(reply) => reply?,
            // The reader drains on exit, so a canceled receiver means the task
            // ended without delivering — a crash, never a hang.
            Err(_) => {
                return Err(HostError::ProviderCrashed {
                    id: self.id.clone(),
                });
            }
        };
        match reply {
            Envelope::Frames { id: echoed, result } => {
                // The reader matched this reply to us by id, so the echo already
                // agrees; verifying keeps the §H4 guarantee explicit and local.
                verify_correlation(&self.id, Some(sent_id.as_str()), echoed.as_deref())?;
                Ok(result)
            }
            Envelope::Error { message, code, .. } => Err(HostError::Provider {
                id: self.id.clone(),
                code,
                message,
            }),
            other => Err(HostError::UnexpectedEnvelope {
                id: self.id.clone(),
                expected: "frames".into(),
                got: envelope_kind(&other).into(),
            }),
        }
    }

    async fn verify(&self, request: &VerifyRequest) -> Result<VerifyResponse, HostError> {
        // `verify`/`verified` carry no id (they correlate by echoing the frame
        // identity in full), so they cannot be demultiplexed — they stay
        // lock-step, exactly as ADR 0002 scopes them.
        match self
            .exchange_lockstep(Envelope::Verify {
                request: request.clone(),
            })
            .await?
        {
            Envelope::Verified { response } => Ok(response),
            Envelope::Error { message, code, .. } => Err(HostError::Provider {
                id: self.id.clone(),
                code,
                message,
            }),
            other => Err(HostError::UnexpectedEnvelope {
                id: self.id.clone(),
                expected: "verified".into(),
                got: envelope_kind(&other).into(),
            }),
        }
    }

    async fn shutdown(&self) -> Result<(), HostError> {
        // Best-effort `shutdown` envelope over the write half, then the bounded
        // grace + process-group kill backstop — the original teardown, intact.
        {
            let mut stdin = self.stdin.lock().await;
            let _ = write_envelope(&mut stdin, &Envelope::Shutdown, &self.id).await;
        }
        let mut control = self.control.lock().await;
        control.wait_or_kill().await
    }
}

impl Drop for StdioProvider {
    fn drop(&mut self) {
        // The reader task ends on its own when the child's stdout closes, but
        // abort it eagerly so a wedged pipe cannot keep the task alive after the
        // provider is gone. `control`'s own `Drop` kills the child.
        self.reader.abort();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use contextgraph_types::{ContextFrame, FrameKind};

    /// Build a one-shot bash "provider" that emits `script` lines. Bash's
    /// `read`/`printf` are builtins, so it works under the scrubbed env
    /// (only PATH/HOME forwarded).
    fn bash_provider(script: &str) -> (String, Vec<String>) {
        (
            "bash".to_string(),
            vec!["-c".to_string(), script.to_string()],
        )
    }

    fn ack_line(version: &str) -> String {
        // A minimal, well-formed handshake_ack the bash provider can echo.
        let ack = Envelope::HandshakeAck {
            protocol_version: version.to_string(),
            provider: ProviderInfo {
                name: "bash-fixture".into(),
                version: "0.0.1".into(),
                data_flow: contextgraph_types::DataFlow {
                    reads: true,
                    writes: false,
                    egress: false,
                    egress_scopes: vec![],
                },
            },
            capabilities: Capabilities {
                query: contextgraph_types::capability::QueryCapability {
                    kinds: vec!["doc".into()],
                },
                ..Capabilities::default()
            },
        };
        serde_json::to_string(&ack).unwrap()
    }

    fn frames_line() -> String {
        let frame = ContextFrame {
            id: "frm_1".into(),
            kind: FrameKind::Doc,
            title: "README".into(),
            content: Some("hello from a stdio provider".into()),
            content_digest: None,
            uri: Some("file:///README.md".into()),
            representation: Default::default(),
            content_fidelity: None,
            canonical_content_hash: None,
            content_ref: None,
            transform: None,
            minimum_content_fidelity: None,
            inline_content_requirement: None,
            score: 0.7,
            token_cost: 12,
            canonical_token_cost: None,
            tokenizer_ref: None,
            valid_from: None,
            valid_to: None,
            recorded_at: None,
            provenance: vec![],
            citation_label: Some("README.md".into()),
            embedding: None,
            relations: vec![],
        };
        let env = Envelope::Frames {
            id: None,
            result: ContextQueryResult {
                frames: vec![frame],
                truncated: false,
                dropped_estimate: None,
            },
        };
        serde_json::to_string(&env).unwrap()
    }

    /// A handshake ack that negotiates `correlation`, so a `StdioProvider` built
    /// on it takes the pipelined (demux-on-id) `query` path.
    fn ack_line_correlating(version: &str) -> String {
        let ack = Envelope::HandshakeAck {
            protocol_version: version.to_string(),
            provider: ProviderInfo {
                name: "bash-fixture".into(),
                version: "0.0.1".into(),
                data_flow: contextgraph_types::DataFlow {
                    reads: true,
                    writes: false,
                    egress: false,
                    egress_scopes: vec![],
                },
            },
            capabilities: Capabilities {
                query: contextgraph_types::capability::QueryCapability {
                    kinds: vec!["doc".into()],
                },
                correlation: true,
                ..Capabilities::default()
            },
        };
        serde_json::to_string(&ack).unwrap()
    }

    /// A `frames` envelope with `__ID__` (the correlation id) and `__CONTENT__`
    /// (the frame's content + title) as substitution placeholders, so the bash
    /// witness fixture can echo each query's own id and goal back verbatim.
    fn frames_template() -> String {
        let frame = ContextFrame {
            id: "frm_1".into(),
            kind: FrameKind::Doc,
            title: "__CONTENT__".into(),
            content: Some("__CONTENT__".into()),
            content_digest: None,
            uri: Some("file:///README.md".into()),
            representation: Default::default(),
            content_fidelity: None,
            canonical_content_hash: None,
            content_ref: None,
            transform: None,
            minimum_content_fidelity: None,
            inline_content_requirement: None,
            score: 0.7,
            token_cost: 12,
            canonical_token_cost: None,
            tokenizer_ref: None,
            valid_from: None,
            valid_to: None,
            recorded_at: None,
            provenance: vec![],
            citation_label: Some("README.md".into()),
            embedding: None,
            relations: vec![],
        };
        let env = Envelope::Frames {
            id: Some("__ID__".into()),
            result: ContextQueryResult {
                frames: vec![frame],
                truncated: false,
                dropped_estimate: None,
            },
        };
        serde_json::to_string(&env).unwrap()
    }

    /// A bash "provider" that reads BOTH queries before answering either, then
    /// answers the second-received query FIRST. `@ACK@` / `@FRAMES_TMPL@` are
    /// substituted in from Rust; the fixture pulls each query's own `id` and
    /// `goal` off the wire and pairs them into the reply it emits for that id.
    ///
    /// Reading two queries before replying is the crux: a lock-step transport
    /// would not send the second query until the first's reply was consumed, so
    /// the fixture's second `read` would block and the whole exchange would
    /// deadlock. Only id-demultiplexing lets both queries be in flight at once,
    /// which is exactly what this witnesses.
    const OUT_OF_ORDER_WITNESS_SCRIPT: &str = r#"
read -r handshake
printf '%s\n' '@ACK@'
read -r q1
read -r q2
tmpl='@FRAMES_TMPL@'
idre='"id":"([^"]+)"'
goalre='"goal":"([^"]+)"'
[[ $q1 =~ $idre ]]; id1=${BASH_REMATCH[1]}
[[ $q1 =~ $goalre ]]; g1=${BASH_REMATCH[1]}
[[ $q2 =~ $idre ]]; id2=${BASH_REMATCH[1]}
[[ $q2 =~ $goalre ]]; g2=${BASH_REMATCH[1]}
r1=${tmpl//__ID__/$id1}; r1=${r1//__CONTENT__/$g1}
r2=${tmpl//__ID__/$id2}; r2=${r2//__CONTENT__/$g2}
printf '%s\n' "$r2"
printf '%s\n' "$r1"
"#;

    fn sample_query() -> ContextQuery {
        ContextQuery {
            goal: "g".into(),
            query_text: None,
            embedding: None,
            kinds: vec![],
            anchors: vec![],
            max_frames: 5,
            max_tokens: 4000,
            as_of: None,
            representation_preferences: vec![],
        }
    }

    #[tokio::test]
    async fn full_handshake_and_query_round_trip_over_stdio() {
        // Reads the handshake, acks; reads the query, replies with frames.
        let script = format!(
            "read h; printf '%s\\n' '{}'; read q; printf '%s\\n' '{}'",
            ack_line(PROTOCOL_VERSION),
            frames_line()
        );
        let (program, args) = bash_provider(&script);
        let provider = StdioProvider::spawn("docs", &program, &args)
            .await
            .expect("handshake should succeed");
        assert_eq!(provider.id(), "docs");
        assert_eq!(provider.info().name, "bash-fixture");
        assert!(provider.capabilities().query.kinds.contains(&"doc".into()));

        let result = provider.query(&sample_query()).await.expect("query ok");
        assert_eq!(result.frames.len(), 1);
        assert_eq!(result.frames[0].title, "README");
    }

    /// ADR 0002's witness: two concurrent correlated `query`s over one stdio
    /// connection, answered **out of order**, must demultiplex back to their own
    /// callers — proving the transport pipelines on `id` rather than serializing
    /// on a single mutex.
    ///
    /// The fixture reads both queries before answering either and answers the
    /// second-received one first (see [`OUT_OF_ORDER_WITNESS_SCRIPT`]). Under the
    /// old lock-step transport this deadlocks, because the host would not send
    /// the second query until the first's reply was consumed; only demux
    /// completes both. It pairs each reply's `id` with that query's own `goal`,
    /// so the assertions catch mis-routing, not merely liveness.
    #[tokio::test]
    async fn two_correlated_queries_answered_out_of_order_demux_to_their_own_callers() {
        let script = OUT_OF_ORDER_WITNESS_SCRIPT
            .replace("@ACK@", &ack_line_correlating(PROTOCOL_VERSION))
            .replace("@FRAMES_TMPL@", &frames_template());
        let (program, args) = bash_provider(&script);

        let provider = StdioProvider::spawn("docs", &program, &args)
            .await
            .expect("handshake should succeed");
        assert!(
            provider.capabilities().correlation,
            "fixture must negotiate correlation for the pipelined path"
        );

        let mut query_alpha = sample_query();
        query_alpha.goal = "alpha".into();
        let mut query_bravo = sample_query();
        query_bravo.goal = "bravo".into();

        // Fire both concurrently. A bounded timeout turns a demux regression
        // (which manifests as a hang) into a visible failure instead of a wedged
        // suite — the hang is the bug, this just surfaces it.
        let (result_alpha, result_bravo) = tokio::time::timeout(Duration::from_secs(10), async {
            tokio::join!(provider.query(&query_alpha), provider.query(&query_bravo))
        })
        .await
        .expect("two concurrent correlated queries must not hang — demux, not lock-step");

        let result_alpha = result_alpha.expect("alpha query ok");
        let result_bravo = result_bravo.expect("bravo query ok");

        assert_eq!(result_alpha.frames.len(), 1);
        assert_eq!(result_bravo.frames.len(), 1);
        // Each caller received the frames the fixture built for ITS id, despite
        // the replies arriving in the opposite order — the demux-by-id witness.
        assert_eq!(
            result_alpha.frames[0].content.as_deref(),
            Some("alpha"),
            "the alpha caller must receive alpha's frames, never bravo's"
        );
        assert_eq!(
            result_bravo.frames[0].content.as_deref(),
            Some("bravo"),
            "the bravo caller must receive bravo's frames, never alpha's"
        );
    }

    #[tokio::test]
    async fn an_incompatible_protocol_version_is_a_named_error_not_a_hang() {
        let script = format!("read h; printf '%s\\n' '{}'", ack_line("contextgraph/2.0"));
        let (program, args) = bash_provider(&script);
        let err = match StdioProvider::spawn("docs", &program, &args).await {
            Ok(_) => panic!("a version mismatch must reject the provider"),
            Err(e) => e,
        };
        match err {
            HostError::VersionMismatch {
                provider_version, ..
            } => assert_eq!(provider_version, "contextgraph/2.0"),
            other => panic!("expected VersionMismatch, got {other}"),
        }
    }

    #[tokio::test]
    async fn a_child_dying_after_handshake_surfaces_as_provider_crashed() {
        // Acks the handshake, then exits before the query — the crash path.
        let script = format!(
            "read h; printf '%s\\n' '{}'; exit 0",
            ack_line(PROTOCOL_VERSION)
        );
        let (program, args) = bash_provider(&script);
        let provider = StdioProvider::spawn("docs", &program, &args)
            .await
            .expect("handshake ok");
        let err = provider
            .query(&sample_query())
            .await
            .expect_err("a dead child must error, not hang");
        assert!(
            matches!(err, HostError::ProviderCrashed { .. }),
            "expected ProviderCrashed, got {err}"
        );
    }

    #[tokio::test]
    async fn the_child_is_spawned_with_a_scrubbed_environment() {
        // Pick a variable the parent test process has that scrubbing must
        // strip — anything but the PATH/HOME allowlist and bash's own
        // re-injected names. `cargo test` always sets CARGO_* vars, so one
        // exists.
        let injected = ["PWD", "SHLVL", "_", "HOME", "PATH", "OLDPWD"];
        let leaked = std::env::vars()
            .map(|(k, _)| k)
            .find(|k| !injected.contains(&k.as_str()) && !k.is_empty())
            .expect("the test process has at least one non-allowlisted env var");

        // A raw connection running `env`; read its environment dump.
        let mut conn = RawStdioConnection::spawn("bash", &["-c".into(), "env".into()])
            .await
            .expect("spawn env");
        let mut child_keys = Vec::new();
        while let Some(line) = conn.read_raw_line().await.expect("read env line") {
            if let Some((key, _)) = line.trim_end().split_once('=') {
                child_keys.push(key.to_string());
            }
        }
        assert!(
            !child_keys.contains(&leaked),
            "scrubbed child leaked parent env var `{leaked}` — credentials must not cross"
        );
    }
}
