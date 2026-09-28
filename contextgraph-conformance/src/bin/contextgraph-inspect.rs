//! `contextgraph-inspect` — an interactive Context Graph Protocol prober, analogous to MCP's inspector
//! (`SPEC.md` §11). Point it at a provider; it completes the
//! handshake, prints the negotiated capabilities, optionally fires a test
//! query, and runs the conformance suite — all in human-readable colored
//! output.
//!
//! It also checks the lifecycle profile's record layer offline, with no
//! provider at all: a record's `record_hash`, the canonical RFC 8785 (JCS)
//! preimage that hash is taken over, whether a stored hash is current, and a
//! detached record attestation (profile LH1 and LC3, ADR 0017, issue #121).
//!
//! ```text
//! contextgraph-inspect stdio [--query GOAL] [--json] -- <program> [args...]
//! contextgraph-inspect http <url> [--query GOAL] [--json]
//! contextgraph-inspect host [--json]
//! contextgraph-inspect record hash <record.json>
//! contextgraph-inspect record preimage <record.json>
//! contextgraph-inspect record verify <record.json>
//! contextgraph-inspect record attest <attestation.json> (--record <record.json> | --record-hash <digest>) --key <hex>
//! ```
//!
//! Every file argument to a `record` verb may be `-` to read standard input.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use colored::Colorize;
use contextgraph_conformance::{
    CheckStatus, ConformanceReport, ProviderTarget, run_conformance, run_host_conformance,
};
use contextgraph_host::{ConsentRecord, Host};
use contextgraph_types::record_attest::{
    RECORD_HASH_MEMBER, record_hash, record_hash_is_current, record_hash_preimage,
    verify_record_attestation, verify_signed_record_hash,
};
use contextgraph_types::{
    AttestationVerdict, Capabilities, ContextQuery, ProviderInfo, RecordAttestation,
};
use serde_json::Value;

#[derive(Parser)]
#[command(
    name = "contextgraph-inspect",
    about = "Probe and conformance-test a CGP provider (SPEC.md §11)."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Probe a stdio child-process provider.
    Stdio {
        /// Fire a test query with this goal after the handshake.
        #[arg(long)]
        query: Option<String>,
        /// Emit the conformance report as JSON instead of colored text.
        #[arg(long)]
        json: bool,
        /// The provider command, after `--`: `<program> [args...]`.
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },
    /// Probe a remote HTTP provider.
    Http {
        /// The provider URL.
        url: String,
        #[arg(long)]
        query: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Run the host-side conformance suite against the reference host
    /// (`SPEC.md` §11.1, issue #14). Takes no provider — the harness drives the
    /// reference `Host` against adversarial in-process providers itself.
    Host {
        /// Emit the conformance report as JSON instead of colored text.
        #[arg(long)]
        json: bool,
    },
    /// Check a lifecycle-profile record offline: its `record_hash`, the
    /// canonical preimage that hash covers, or a detached record attestation
    /// (profile LH1 and LC3, ADR 0017). Needs no provider.
    ///
    /// Exits 0 on a positive answer, 1 when the record or attestation checked
    /// out wrong, and 2 when the input could not be checked at all.
    Record {
        #[command(subcommand)]
        verb: RecordVerb,
    },
}

/// The record-layer verbs (issue #121).
///
/// `preimage` is the one that earns the rest their place. A digest that
/// disagrees between two implementations says only that their
/// canonicalizations differ; a byte diff of the two preimages says where.
#[derive(Subcommand)]
enum RecordVerb {
    /// Print the record's `record_hash`: `sha256:<hex>` over the JCS
    /// canonicalization of the record without its own `record_hash` member.
    Hash {
        /// The record, as a JSON file (`-` for standard input).
        record: PathBuf,
    },
    /// Print the exact canonical bytes the `record_hash` is taken over, with
    /// no trailing newline, so piping them to `sha256sum` reproduces the
    /// digest and diffing two of them locates a canonicalization disagreement.
    Preimage {
        /// The record, as a JSON file (`-` for standard input).
        record: PathBuf,
    },
    /// Check that the record's stored `record_hash` is the one its content
    /// produces. A record with no stored hash is reported as unhashed and
    /// exits 1.
    Verify {
        /// The record, as a JSON file (`-` for standard input).
        record: PathBuf,
    },
    /// Verify a detached record attestation under an Ed25519 public key.
    ///
    /// With `--record`, the record's hash is recomputed from its content rather
    /// than read from its stored member, so a record edited after signing is
    /// caught. With `--record-hash`, the attestation is checked against a hash
    /// you already hold: the auditor's case, where the record is not at hand.
    Attest {
        /// The `RecordAttestation`, as a JSON file (`-` for standard input).
        attestation: PathBuf,
        /// The record the attestation claims to sign.
        #[arg(
            long,
            value_name = "FILE",
            required_unless_present = "record_hash",
            conflicts_with = "record_hash"
        )]
        record: Option<PathBuf>,
        /// The `sha256:<hex>` record hash the attestation claims to sign, in
        /// place of the record.
        #[arg(long, value_name = "DIGEST")]
        record_hash: Option<String>,
        /// The attester's Ed25519 public key: 32 bytes as 64 hex digits.
        #[arg(long, value_name = "HEX")]
        key: String,
    },
}

/// Exit status for a record verb that reached a negative answer: a stale hash,
/// an unhashed record, or an attestation that does not verify.
const EXIT_NEGATIVE: u8 = 1;
/// Exit status for a record verb that could not reach an answer at all: an
/// unreadable file, input that is not JSON, or a value that is not a record.
/// Distinct from `EXIT_NEGATIVE` so a script can tell "this record is wrong"
/// from "this was not a record"; it matches clap's own status for a usage
/// error, which is the same kind of failure.
const EXIT_UNCHECKABLE: u8 = 2;

/// A reconstructable target descriptor — `contextgraph-inspect` establishes the
/// provider twice (once interactively, once for the conformance run), so it
/// keeps the fields rather than a one-shot [`ProviderTarget`].
enum Descriptor {
    Stdio { program: String, args: Vec<String> },
    Http { url: String },
}

impl Descriptor {
    fn to_target(&self) -> ProviderTarget {
        match self {
            Descriptor::Stdio { program, args } => ProviderTarget::Stdio {
                program: program.clone(),
                args: args.clone(),
            },
            Descriptor::Http { url } => ProviderTarget::Http { url: url.clone() },
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let (descriptor, query_goal, json) = match cli.command {
        Command::Stdio {
            query,
            json,
            command,
        } => {
            let mut parts = command.into_iter();
            let program = parts.next().unwrap_or_default();
            let args: Vec<String> = parts.collect();
            (Descriptor::Stdio { program, args }, query, json)
        }
        Command::Http { url, query, json } => (Descriptor::Http { url }, query, json),
        // The host suite needs no provider target: it drives the reference host
        // against its own adversarial in-process providers and reports directly.
        Command::Host { json } => {
            let report = run_host_conformance().await;
            print_report(&report, json);
            return if report.passed() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            };
        }
        // The record verbs are offline and synchronous: no provider, no host.
        Command::Record { verb } => return run_record_verb(verb),
    };

    // ── Phase 1: interactive handshake + optional query ──────────────────
    interactive_probe(&descriptor, query_goal.as_deref()).await;

    // ── Phase 2: the conformance verdict ─────────────────────────────────
    let report = run_conformance(descriptor.to_target()).await;
    print_report(&report, json);

    if report.passed() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

async fn interactive_probe(descriptor: &Descriptor, query_goal: Option<&str>) {
    let mut host = Host::new();
    let id = "provider";
    let added = match descriptor {
        Descriptor::Stdio { program, args } => host.add_stdio(id, program, args).await,
        Descriptor::Http { url } => host.add_http(id, url.clone(), None).await,
    };

    match added {
        Ok(()) => {
            let (info, caps) = match host.provider(id) {
                Some(provider) => (provider.info().clone(), provider.capabilities().clone()),
                None => {
                    println!(
                        "{} provider vanished immediately after a successful handshake",
                        "internal error:".red().bold()
                    );
                    return;
                }
            };
            print_capabilities(&info, &caps);

            if info.data_flow.egress {
                // The operator ran the probe deliberately — consent to the
                // declared flow so the demo query can run.
                host.record_consent(ConsentRecord::new(
                    id,
                    info.data_flow,
                    "contextgraph-inspect interactive probe",
                ));
            }

            if let Some(goal) = query_goal {
                fire_query(&host, id, goal).await;
            }

            let _ = host.shutdown().await;
        }
        Err(error) => {
            println!("{} {error}", "handshake failed:".red().bold());
        }
    }
}

fn print_capabilities(info: &ProviderInfo, caps: &Capabilities) {
    println!(
        "{}",
        "── Context Graph Protocol provider ──────────────────────────────".dimmed()
    );
    println!(
        "  {} {} {}",
        "provider".bold(),
        info.name.cyan(),
        format!("v{}", info.version).dimmed()
    );

    let flow = format!(
        "reads={} writes={} egress={}",
        info.data_flow.reads, info.data_flow.writes, info.data_flow.egress
    );
    let flow = if info.data_flow.egress {
        flow.yellow()
    } else {
        flow.green()
    };
    println!("  {} {flow}", "data-flow".bold());
    if info.data_flow.egress {
        println!(
            "  {}",
            "⚠ egress: this provider can send data off-machine — consent required (SPEC.md §4)"
                .yellow()
        );
    }

    println!(
        "  {} kinds={:?} graph={}",
        "capabilities".bold(),
        caps.query.kinds,
        caps.graph
    );
    if let Some(fingerprint) = &caps.embeddings_fingerprint {
        println!("  {} {fingerprint}", "embedder".bold());
    }
}

async fn fire_query(host: &Host, id: &str, goal: &str) {
    let query = ContextQuery {
        goal: goal.into(),
        query_text: Some(goal.into()),
        embedding: None,
        kinds: vec![],
        anchors: vec![],
        max_frames: 8,
        max_tokens: 4096,
        as_of: None,
        representation_preferences: vec![],
    };
    match host.query_provider(id, &query).await {
        Ok(result) => {
            println!(
                "{}",
                format!("── {} frame(s) for “{goal}” ──", result.frames.len()).dimmed()
            );
            for frame in &result.frames {
                // Cite by human label, never the raw id (SPEC.md §6).
                let label = frame
                    .citation_label
                    .as_deref()
                    .filter(|l| !l.trim().is_empty())
                    .unwrap_or(&frame.title);
                println!(
                    "  {} {}  {}",
                    format!("[{:.2}]", frame.score).dimmed(),
                    label.cyan(),
                    format!("{}tok", frame.token_cost).dimmed()
                );
            }
            if !result.respects_budget(query.max_tokens) {
                println!(
                    "  {}",
                    "⚠ frames exceed the requested budget — a budget-honesty violation".red()
                );
            }
        }
        Err(error) => println!("  {} {error}", "query failed:".red()),
    }
}

fn print_report(report: &ConformanceReport, json: bool) {
    if json {
        match serde_json::to_string_pretty(report) {
            Ok(text) => println!("{text}"),
            Err(error) => eprintln!("could not serialize report: {error}"),
        }
        return;
    }

    println!(
        "{}",
        format!("── conformance: {} ──", report.target).dimmed()
    );
    for check in &report.checks {
        let mark = match check.status {
            CheckStatus::Pass => "✓".green().bold(),
            CheckStatus::Fail => "✗".red().bold(),
            CheckStatus::Skipped => "–".dimmed(),
        };
        let name = match check.status {
            CheckStatus::Pass => check.name.green(),
            CheckStatus::Fail => check.name.red(),
            CheckStatus::Skipped => check.name.dimmed(),
        };
        println!("  {mark} {name}");
        println!("      {}", check.evidence.dimmed());
    }

    let (passed, failed, skipped) = report.tally();
    let verdict = if report.passed() {
        format!("CONFORMANT — {passed} passed, {skipped} skipped")
            .green()
            .bold()
    } else {
        format!("NOT CONFORMANT — {failed} failed, {passed} passed, {skipped} skipped")
            .red()
            .bold()
    };
    println!("  {verdict}");
}

// ── Record layer (issue #121) ────────────────────────────────────────────
//
// Every computation below calls `contextgraph_types::record_attest`, the same
// functions the lifecycle-profile suite and a Rust CEP call. A copy of the rule
// kept here could agree with this binary's own tests and with nothing that
// ships, which is the one failure a debugging tool must not have.

/// Run one record verb and map its outcome onto the exit statuses documented
/// on `Command::Record`: 0 positive, 1 negative, 2 uncheckable.
fn run_record_verb(verb: RecordVerb) -> ExitCode {
    let outcome = match verb {
        RecordVerb::Hash { record } => record_hash_verb(&record),
        RecordVerb::Preimage { record } => record_preimage_verb(&record),
        RecordVerb::Verify { record } => record_verify_verb(&record),
        RecordVerb::Attest {
            attestation,
            record,
            record_hash,
            key,
        } => record_attest_verb(
            &attestation,
            record.as_deref(),
            record_hash.as_deref(),
            &key,
        ),
    };
    match outcome {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(EXIT_NEGATIVE),
        Err(error) => {
            eprintln!("{} {error}", "error:".red().bold());
            ExitCode::from(EXIT_UNCHECKABLE)
        }
    }
}

/// `record hash`: print the record's `record_hash` and nothing else, so the
/// output is usable in a script as-is.
fn record_hash_verb(path: &Path) -> Result<bool, String> {
    let record = read_json(path)?;
    let hash = record_hash(&record).map_err(|error| error.to_string())?;
    println!("{hash}");
    Ok(true)
}

/// `record preimage`: write the canonical bytes exactly, with no trailing
/// newline and no color, because a single added byte would make them hash to
/// something other than the `record_hash` they explain.
fn record_preimage_verb(path: &Path) -> Result<bool, String> {
    let record = read_json(path)?;
    let preimage = record_hash_preimage(&record).map_err(|error| error.to_string())?;
    let mut stdout = std::io::stdout().lock();
    stdout
        .write_all(&preimage)
        .map_err(|error| format!("could not write the preimage: {error}"))?;
    stdout
        .flush()
        .map_err(|error| format!("could not write the preimage: {error}"))?;
    Ok(true)
}

/// `record verify`: whether the stored `record_hash` is current.
///
/// The verdict is [`record_hash_is_current`]'s, not a comparison written here.
/// The recomputed hash is printed beside it, so a stale record says what its
/// hash should be.
fn record_verify_verb(path: &Path) -> Result<bool, String> {
    let record = read_json(path)?;
    let computed = record_hash(&record).map_err(|error| error.to_string())?;
    let current = record_hash_is_current(&record).map_err(|error| error.to_string())?;
    if current {
        println!("{} {computed}", "current".green().bold());
        return Ok(true);
    }
    match record.get(RECORD_HASH_MEMBER) {
        None => {
            println!(
                "{} the record carries no `{RECORD_HASH_MEMBER}` member",
                "unhashed".yellow().bold()
            );
            println!("  computed {computed}");
        }
        Some(stored) => {
            // The stored member is shown as JSON, so a non-string value (a
            // number, a null) is visible as what it is rather than coerced.
            println!(
                "{} the stored `{RECORD_HASH_MEMBER}` does not match the record's content",
                "stale".red().bold()
            );
            println!("  stored   {stored}");
            println!("  computed {computed}");
        }
    }
    Ok(false)
}

/// `record attest`: verify a detached record attestation.
///
/// Exactly one of `record_path` and `record_hash` is present; clap enforces
/// that before this runs.
fn record_attest_verb(
    attestation_path: &Path,
    record_path: Option<&Path>,
    record_hash: Option<&str>,
    key_hex: &str,
) -> Result<bool, String> {
    let attestation: RecordAttestation = serde_json::from_value(read_json(attestation_path)?)
        .map_err(|error| {
            format!(
                "{} is not a RecordAttestation: {error}",
                display_path(attestation_path)
            )
        })?;
    let public_key =
        parse_hex(key_hex).ok_or_else(|| format!("--key is not hex-encoded bytes: {key_hex:?}"))?;

    let verdict = match (record_path, record_hash) {
        (Some(path), _) => {
            let record = read_json(path)?;
            verify_record_attestation(&record, &attestation, &public_key)
                .map_err(|error| error.to_string())?
        }
        (None, Some(hash)) => verify_signed_record_hash(hash, &attestation, &public_key),
        (None, None) => return Err("name the signed record with --record or --record-hash".into()),
    };

    let valid = verdict.is_valid();
    let label = if valid {
        "valid".green().bold()
    } else {
        "invalid".red().bold()
    };
    println!("{label} {}", describe_verdict(&verdict));
    println!("  attester {}", attestation.attester_id);
    println!("  key_id   {}", attestation.key_id);
    println!("  signs    {}", attestation.signed_record_hash);
    Ok(valid)
}

/// One line naming what a verdict means for a record attestation.
///
/// The wildcard arm is deliberate: a verdict this function does not describe
/// is still printed, by its debug form, rather than this binary failing to
/// compile when the shared verdict vocabulary grows a variant.
fn describe_verdict(verdict: &AttestationVerdict) -> String {
    match verdict {
        AttestationVerdict::Valid => {
            "the signature verifies over the record's hash under the given key".into()
        }
        AttestationVerdict::CommitmentMismatch { expected, signed } => format!(
            "commitment mismatch: the record hashes to {expected}, \
             but the attestation signs {signed}"
        ),
        AttestationVerdict::BadSignature => {
            "bad signature: the hashes match but the signature does not verify under the given key"
                .into()
        }
        AttestationVerdict::UnknownAlgorithm(algorithm) => {
            format!("unknown algorithm: {algorithm:?} is not one this build can check")
        }
        AttestationVerdict::MalformedKey => {
            "malformed key: not a well-formed Ed25519 public key (32 bytes)".into()
        }
        AttestationVerdict::MalformedSignature => {
            "malformed signature: not a well-formed Ed25519 signature (64 bytes of hex)".into()
        }
        AttestationVerdict::MalformedCommitment => {
            "malformed commitment: signed_record_hash is not a sha256:<64 lowercase hex> digest"
                .into()
        }
        other => format!("{other:?}"),
    }
}

/// Read and parse one JSON document; `-` reads standard input.
fn read_json(path: &Path) -> Result<Value, String> {
    let text = if path == Path::new("-") {
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .map_err(|error| format!("could not read standard input: {error}"))?;
        text
    } else {
        std::fs::read_to_string(path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?
    };
    serde_json::from_str(&text)
        .map_err(|error| format!("{} is not JSON: {error}", display_path(path)))
}

/// How an input path is named in a message: `-` reads as standard input.
fn display_path(path: &Path) -> String {
    if path == Path::new("-") {
        "standard input".into()
    } else {
        path.display().to_string()
    }
}

/// Parse hex (either case) into bytes. `None` on an odd length or a non-hex
/// digit. Length is not checked here: a key of the wrong size is the
/// verifier's `MalformedKey`, which names the problem better than this could.
fn parse_hex(text: &str) -> Option<Vec<u8>> {
    let text = text.trim();
    if !text.len().is_multiple_of(2) {
        return None;
    }
    let (pairs, _) = text.as_bytes().as_chunks::<2>();
    pairs
        .iter()
        .map(|pair| {
            let hi = (pair[0] as char).to_digit(16)?;
            let lo = (pair[1] as char).to_digit(16)?;
            Some((hi * 16 + lo) as u8)
        })
        .collect()
}
