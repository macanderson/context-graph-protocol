//! Host-side end-to-end provenance-digest verification (`SPEC.md` §6.2, §F5;
//! issue #12) — the *bytes* half of F5.
//!
//! `contextgraph-types`' [`is_well_formed_digest`](contextgraph_types::is_well_formed_digest)
//! and the provider-facing `frame-validity` check together enforce F5's
//! **grammar** (`sha256:<64 lowercase hex>`). Neither can say whether a digest
//! actually matches the bytes it claims to cover: that requires re-reading the
//! source, which only a host can do. This module is that verifier.
//!
//! It is **not** [`Host::verify_frames`](crate::Host::verify_frames): that asks
//! the *provider* whether a held frame is still current (`context/verify`, §9).
//! This re-reads local bytes the host can see and hashes them itself, trusting
//! no one — the two are different guarantees that happen to share the verb.
//!
//! ## Digested bytes (§6.2)
//!
//! The digest covers the exact UTF-8 source bytes addressed by `uri` + `range`
//! at read time, with **no normalization**: no line-ending translation, no
//! trailing-newline adjustment. Provenance without a `range` digests the whole
//! resource. Only `file` provenance is held to F5 — a `derivation` or `episode`
//! link has no addressable bytes.
//!
//! ## Range grammar
//!
//! `SPEC.md` §6.2 does not fix a `range` grammar; the only convention in this
//! codebase is line ranges, `L<start>` or `L<start>-<end>` (1-indexed,
//! inclusive), which this verifier supports. A line's bytes **include** its
//! terminating `\n` as it appears on disk (host-defined, since the spec is
//! silent on terminator inclusion); no `\r` is ever stripped, honoring the
//! "no line-ending translation" clause. An unrecognized range grammar is an
//! honest [`Unreadable`](DigestVerification::Unreadable), never a silent
//! whole-file fallback that would digest the wrong bytes.
//!
//! ## Scope and safety
//!
//! This is a **host API a host invokes deliberately**, over sources it trusts —
//! not an automatic re-read of any `uri` a *provider* names. Re-reading a
//! provider-supplied path is a capability decision (path confinement, consent)
//! the host runtime does not yet make (see the filesystem-confinement note in
//! the crate docs), so this is deliberately *not* wired into
//! [`Host::query_all`](crate::Host::query_all). Wiring it into an end-to-end
//! host-side conformance gate — with confinement — is tracked by the host-side
//! harness (issue #14). It is a synchronous utility; a caller on an async path
//! wraps it in `spawn_blocking`.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use contextgraph_types::{ContextFrame, Provenance};

/// The outcome of verifying one `file`-provenance digest against the bytes it
/// addresses (`SPEC.md` §6.2). Evidence-carrying rather than a bare bool, so a
/// failure says exactly what diverged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DigestVerification {
    /// The declared digest equals the sha256 of the addressed bytes.
    Verified,
    /// The addressed bytes are readable, but their sha256 does **not** match the
    /// declared digest — tampering, a source that changed, or a mis-declared
    /// range. Carries both digests so a report can show the divergence.
    /// `expected` is what the provider declared; `actual` is what the bytes
    /// hash to now.
    Mismatch { expected: String, actual: String },
    /// The addressed bytes could not be read at all: a missing file, a `uri`
    /// that is not a resolvable local `file://`, a range the grammar does not
    /// define, or file provenance carrying no `uri`/`digest` to check. Carries a
    /// human-readable reason. Silence is not validity — an unreadable source is
    /// a failure to confirm, never a pass.
    Unreadable { reason: String },
    /// The provenance does not address file bytes (`type` != `"file"`), so there
    /// is nothing on disk to re-read and F5 does not bind it (§6.2).
    NotFileProvenance,
}

impl DigestVerification {
    /// Whether the addressed bytes hashed exactly to the declared digest.
    pub fn is_verified(&self) -> bool {
        matches!(self, DigestVerification::Verified)
    }
}

/// Re-read the bytes one `file`-provenance entry addresses and check their
/// sha256 against its declared `digest` (`SPEC.md` §6.2, §F5).
///
/// Returns [`NotFileProvenance`](DigestVerification::NotFileProvenance) for a
/// non-`file` link (F5 does not bind it), [`Unreadable`](DigestVerification::Unreadable)
/// when the addressed bytes cannot be read, and otherwise
/// [`Verified`](DigestVerification::Verified) or
/// [`Mismatch`](DigestVerification::Mismatch). A grammar-malformed declared
/// digest simply cannot equal a well-formed hash, so it surfaces as `Mismatch`;
/// digest *grammar* is the provider-facing `frame-validity` check's job, not
/// this one's.
///
/// ```text
/// use contextgraph_host::verify::{verify_provenance_digest, DigestVerification};
/// // provenance: file:///repo/src/net.rs, range L120-160, digest sha256:<64 hex>
/// match verify_provenance_digest(&provenance) {
///     DigestVerification::Verified => { /* the bytes still hash as claimed */ }
///     DigestVerification::Mismatch { expected, actual } => { /* tampered or moved */ }
///     other => { /* not a file link, or unreadable */ }
/// }
/// ```
pub fn verify_provenance_digest(provenance: &Provenance) -> DigestVerification {
    if !provenance.is_file_provenance() {
        return DigestVerification::NotFileProvenance;
    }
    let Some(declared) = provenance.digest.as_deref() else {
        return DigestVerification::Unreadable {
            reason: "file provenance carries no digest to verify (§F5)".to_string(),
        };
    };
    let Some(uri) = provenance.uri.as_deref() else {
        return DigestVerification::Unreadable {
            reason: "file provenance carries no uri to re-read".to_string(),
        };
    };
    let path = match file_uri_to_path(uri) {
        Ok(path) => path,
        Err(reason) => return DigestVerification::Unreadable { reason },
    };
    let bytes = match addressed_bytes(&path, provenance.range.as_deref()) {
        Ok(bytes) => bytes,
        Err(reason) => return DigestVerification::Unreadable { reason },
    };
    let actual = sha256_digest(&bytes);
    if actual == declared {
        DigestVerification::Verified
    } else {
        DigestVerification::Mismatch {
            expected: declared.to_string(),
            actual,
        }
    }
}

/// Verify every `file`-provenance digest a frame declares against the bytes on
/// disk, returning one `(provenance index, outcome)` per `file` entry in
/// provenance order.
///
/// Non-`file` provenance is omitted (F5 does not bind it), so an **empty** result
/// means the frame declares no file provenance to check — *not* that it passed.
/// The index is into `frame.provenance`, so a host can name the exact offending
/// link in a report.
pub fn verify_file_provenance(frame: &ContextFrame) -> Vec<(usize, DigestVerification)> {
    frame
        .provenance
        .iter()
        .enumerate()
        .filter(|(_, provenance)| provenance.is_file_provenance())
        .map(|(index, provenance)| (index, verify_provenance_digest(provenance)))
        .collect()
}

/// Read the exact bytes a `file` provenance addresses: the whole resource when
/// `range` is absent, else the addressed line span (§6.2). No normalization is
/// applied — the bytes are returned exactly as they sit on disk.
fn addressed_bytes(path: &Path, range: Option<&str>) -> Result<Vec<u8>, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("cannot read `{}`: {error}", path.display()))?;
    match range {
        None => Ok(bytes),
        Some(spec) => extract_line_range(&bytes, spec),
    }
}

/// Extract the byte span of an `L<start>[-<end>]` line range (1-indexed,
/// inclusive; each line's trailing `\n` included). An unrecognized grammar is an
/// error, not a whole-file fallback — digesting bytes the range never named is
/// exactly the silent wrongness this check exists to prevent.
fn extract_line_range(bytes: &[u8], spec: &str) -> Result<Vec<u8>, String> {
    let digits = spec
        .strip_prefix('L')
        .ok_or_else(|| unsupported_range(spec))?;
    let (start, end) = match digits.split_once('-') {
        Some((first, last)) => (parse_line(first, spec)?, parse_line(last, spec)?),
        None => {
            let single = parse_line(digits, spec)?;
            (single, single)
        }
    };
    if start == 0 || end < start {
        return Err(format!("range `{spec}` is empty or inverted"));
    }

    // Per-line byte spans, each including its terminating `\n`. A trailing `\n`
    // does not open an extra empty line (matches `str::lines()` line counting),
    // and no `\r` is stripped (no line-ending translation, §6.2).
    let mut line_spans: Vec<(usize, usize)> = Vec::new();
    let mut line_start = 0usize;
    for (i, &byte) in bytes.iter().enumerate() {
        if byte == b'\n' {
            line_spans.push((line_start, i + 1));
            line_start = i + 1;
        }
    }
    if line_start < bytes.len() {
        line_spans.push((line_start, bytes.len()));
    }

    let count = line_spans.len();
    if start > count {
        return Err(format!(
            "range `{spec}` starts at line {start} but the resource has {count} line(s)"
        ));
    }
    // Clamp the end to EOF: a range that reaches past the last line addresses
    // through the end of the resource.
    let end = end.min(count);
    let from = line_spans[start - 1].0;
    let to = line_spans[end - 1].1;
    Ok(bytes[from..to].to_vec())
}

fn parse_line(field: &str, spec: &str) -> Result<usize, String> {
    field.parse::<usize>().map_err(|_| unsupported_range(spec))
}

fn unsupported_range(spec: &str) -> String {
    format!(
        "unsupported range `{spec}`; expected a line range `L<start>` or `L<start>-<end>` (§6.2)"
    )
}

/// Resolve a `file://` uri to a local path. Accepts an empty authority
/// (`file:///path`) or `localhost`; a non-local host is not re-readable. Percent
/// escapes in the path are decoded, so a `uri` naming a path with spaces or
/// other reserved bytes resolves correctly.
fn file_uri_to_path(uri: &str) -> Result<PathBuf, String> {
    let rest = uri.strip_prefix("file://").ok_or_else(|| {
        format!("provenance uri `{uri}` is not a `file://` uri; only local file provenance is re-readable (§6.2)")
    })?;
    let (authority, path_part) = match rest.find('/') {
        Some(0) => ("", rest),
        Some(index) => (&rest[..index], &rest[index..]),
        None => return Err(format!("`file://` uri `{uri}` has no absolute path")),
    };
    if !authority.is_empty() && authority != "localhost" {
        return Err(format!(
            "`file://` uri `{uri}` names a non-local host `{authority}`; only local files are re-readable"
        ));
    }
    let decoded = percent_decode(path_part);
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Ok(PathBuf::from(std::ffi::OsStr::from_bytes(&decoded)))
    }
    #[cfg(not(unix))]
    {
        Ok(PathBuf::from(
            String::from_utf8_lossy(&decoded).into_owned(),
        ))
    }
}

/// Decode `%XX` percent escapes into raw bytes; any other byte passes through.
/// A `%` not followed by two hex digits is left literal.
fn percent_decode(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

/// A protocol content digest over `bytes`: `sha256:<64 lowercase hex>` (§F5).
/// Lowercase is mandated so a byte-for-byte comparison never yields a spurious
/// case-only mismatch.
fn sha256_digest(bytes: &[u8]) -> String {
    let hash = Sha256::digest(bytes);
    let mut out = String::with_capacity("sha256:".len() + 64);
    out.push_str("sha256:");
    for byte in hash {
        out.push(char::from_digit((byte >> 4) as u32, 16).unwrap());
        out.push(char::from_digit((byte & 0x0f) as u32, 16).unwrap());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use contextgraph_types::{ContextFrame, FrameKind};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A real temp file with Drop-cleanup — `tempfile` is not a dependency, so
    /// this uses `std::env::temp_dir()` and removes itself even if a test panics.
    struct TempFile {
        path: PathBuf,
    }

    impl TempFile {
        fn with_bytes(bytes: &[u8]) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let mut path = std::env::temp_dir();
            path.push(format!(
                "cgp-verify-{}-{}.bin",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::write(&path, bytes).expect("temp file must be writable");
            Self { path }
        }

        /// A `file://` uri for this file. The temp path is absolute and ASCII, so
        /// no percent-encoding is needed to round-trip it.
        fn file_uri(&self) -> String {
            format!("file://{}", self.path.display())
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    fn file_provenance(uri: &str, range: Option<&str>, digest: &str) -> Provenance {
        Provenance {
            kind: "file".to_string(),
            uri: Some(uri.to_string()),
            range: range.map(str::to_string),
            digest: Some(digest.to_string()),
            method: None,
            by: None,
        }
    }

    #[test]
    fn sha256_digest_matches_the_standard_known_answer_vectors() {
        // Anchor the primitive to ground truth, not just to itself: the whole
        // point of this verifier is that its digest equals what any conforming
        // SHA-256 (an external provider, `sha256sum`) computes for the same
        // bytes. Every other test compares two outputs of this same helper, so
        // a nibble-swapped or uppercase digest would slip past them all — but
        // not past these NIST vectors.
        assert_eq!(
            sha256_digest(b"abc"),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_digest(b""),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn a_digest_matching_the_whole_file_bytes_verifies() {
        let content = b"the exact bytes on disk, no more\n";
        let file = TempFile::with_bytes(content);
        let digest = sha256_digest(content);
        let provenance = file_provenance(&file.file_uri(), None, &digest);
        assert_eq!(
            verify_provenance_digest(&provenance),
            DigestVerification::Verified
        );
    }

    #[test]
    fn a_tampered_digest_is_a_mismatch_carrying_both_sides() {
        let content = b"the real source bytes\n";
        let file = TempFile::with_bytes(content);
        // A well-formed digest of *different* bytes — what a tampered or stale
        // claim looks like. Both sides of the mismatch are valid sha256.
        let wrong = sha256_digest(b"bytes the provider never served\n");
        let provenance = file_provenance(&file.file_uri(), None, &wrong);
        match verify_provenance_digest(&provenance) {
            DigestVerification::Mismatch { expected, actual } => {
                assert_eq!(expected, wrong, "the declared digest is echoed back");
                assert_eq!(actual, sha256_digest(content), "actual is the bytes' hash");
                assert_ne!(expected, actual);
            }
            other => panic!("expected a Mismatch, got {other:?}"),
        }
    }

    #[test]
    fn a_line_scoped_digest_verifies_over_exactly_that_span() {
        // Four newline-terminated lines; the range addresses lines 2-3.
        let lines = ["line one", "line two", "line three", "line four"];
        let content = format!("{}\n", lines.join("\n"));
        let file = TempFile::with_bytes(content.as_bytes());

        // Independently compute the expected sub-range bytes — lines 2 and 3,
        // each with its trailing newline — and hash them directly, so this is
        // not a tautology against the verifier's own range logic.
        let expected_span = format!("{}\n", lines[1..3].join("\n"));
        assert_eq!(expected_span, "line two\nline three\n");
        let digest = sha256_digest(expected_span.as_bytes());

        let provenance = file_provenance(&file.file_uri(), Some("L2-3"), &digest);
        assert_eq!(
            verify_provenance_digest(&provenance),
            DigestVerification::Verified
        );

        // A single-line range works too.
        let single = sha256_digest(b"line one\n");
        let provenance = file_provenance(&file.file_uri(), Some("L1"), &single);
        assert_eq!(
            verify_provenance_digest(&provenance),
            DigestVerification::Verified
        );
    }

    #[test]
    fn a_missing_file_is_unreadable_not_a_silent_pass() {
        let file = TempFile::with_bytes(b"gone in a moment\n");
        let uri = file.file_uri();
        let digest = sha256_digest(b"gone in a moment\n");
        drop(file); // remove the file, then verify against its now-dead uri
        let provenance = file_provenance(&uri, None, &digest);
        match verify_provenance_digest(&provenance) {
            DigestVerification::Unreadable { reason } => {
                assert!(
                    reason.contains("cannot read"),
                    "reason names the failure: {reason}"
                );
            }
            other => panic!("expected Unreadable for a missing file, got {other:?}"),
        }
    }

    #[test]
    fn no_line_ending_translation_is_applied_to_the_digested_bytes() {
        // The normative §6.2 clause, on the whole-file path: a CRLF/mixed file's
        // exact bytes are digested. A verifier that normalized `\r\n` to `\n`
        // would compute a different hash and spuriously report a mismatch.
        let content = b"first\r\nsecond\nthird\r\n";
        let file = TempFile::with_bytes(content);
        let digest = sha256_digest(content);
        let provenance = file_provenance(&file.file_uri(), None, &digest);
        assert_eq!(
            verify_provenance_digest(&provenance),
            DigestVerification::Verified,
            "the exact on-disk bytes, carriage returns included, must be what is hashed"
        );
    }

    #[test]
    fn non_file_provenance_is_reported_as_not_bound_by_f5() {
        // A `derivation` link has no addressable bytes to re-read (§6.2).
        let provenance = Provenance {
            kind: "derivation".to_string(),
            uri: None,
            range: None,
            digest: None,
            method: Some("paste".to_string()),
            by: Some("contextgraph-ingest".to_string()),
        };
        assert_eq!(
            verify_provenance_digest(&provenance),
            DigestVerification::NotFileProvenance
        );
    }

    #[test]
    fn an_unrecognized_range_grammar_is_unreadable_never_a_whole_file_fallback() {
        let content = b"one\ntwo\nthree\n";
        let file = TempFile::with_bytes(content);
        // A byte-offset grammar the spec never defined must not silently digest
        // the whole file — that would confirm bytes the range never named.
        let provenance = file_provenance(&file.file_uri(), Some("0-5"), &sha256_digest(content));
        match verify_provenance_digest(&provenance) {
            DigestVerification::Unreadable { reason } => {
                assert!(reason.contains("unsupported range"), "reason: {reason}");
            }
            other => panic!("expected Unreadable for an unknown range grammar, got {other:?}"),
        }
    }

    #[test]
    fn a_non_file_uri_is_unreadable() {
        let provenance = file_provenance(
            "context://provider/artifacts/abc",
            None,
            &sha256_digest(b"x"),
        );
        assert!(matches!(
            verify_provenance_digest(&provenance),
            DigestVerification::Unreadable { .. }
        ));
    }

    #[test]
    fn a_percent_encoded_path_resolves_to_the_real_file() {
        // A path with a space, addressed by a correctly percent-encoded uri.
        let content = b"space in the name\n";
        let mut path = std::env::temp_dir();
        path.push(format!("cgp verify {}.bin", std::process::id()));
        std::fs::write(&path, content).expect("writable");
        let encoded_uri = format!("file://{}", path.display()).replace(' ', "%20");
        let provenance = file_provenance(&encoded_uri, None, &sha256_digest(content));
        let outcome = verify_provenance_digest(&provenance);
        let _ = std::fs::remove_file(&path);
        assert_eq!(outcome, DigestVerification::Verified);
    }

    #[test]
    fn the_frame_level_api_returns_one_result_per_file_link_in_order() {
        let content = b"framed bytes\n";
        let file = TempFile::with_bytes(content);
        let good = sha256_digest(content);

        let mut frame = ContextFrame::full("frm_1", FrameKind::Snippet, "t", "c", 0.5, 1);
        frame.provenance = vec![
            // A non-file link is skipped, so it does not shift the reported index.
            Provenance {
                kind: "derivation".to_string(),
                uri: None,
                range: None,
                digest: None,
                method: None,
                by: None,
            },
            file_provenance(&file.file_uri(), None, &good),
            file_provenance(&file.file_uri(), None, &sha256_digest(b"different\n")),
        ];

        let results = verify_file_provenance(&frame);
        assert_eq!(results.len(), 2, "only the two file links are checked");
        assert_eq!(results[0].0, 1, "index is into frame.provenance");
        assert_eq!(results[0].1, DigestVerification::Verified);
        assert_eq!(results[1].0, 2);
        assert!(matches!(results[1].1, DigestVerification::Mismatch { .. }));

        // No file provenance ⇒ empty result: "nothing to check", not a pass.
        let mut bare = frame.clone();
        bare.provenance.clear();
        assert!(verify_file_provenance(&bare).is_empty());
    }
}
