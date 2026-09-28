//! `contextgraph-inspect record …` against the lifecycle-profile fixtures
//! (issue #121).
//!
//! These run the real binary, not the functions behind it: what a CEP author
//! gets is the process — its stdout bytes and its exit status — and that is
//! what a script diffing two preimages or gating on `record verify` relies on.
//!
//! The expected values come from the published vectors in `tests/fixtures/`
//! (`record-hash-vectors.json`, `record-attestation.json`,
//! `record-attestation-key.json`), which `lifecycle_profile_examples.rs`
//! already pins against the library. So each test here checks one link: that
//! the binary reports what the library and the vectors agree on.
//!
//! Exit statuses under test: 0 for a positive answer, 1 for a negative one
//! (stale, unhashed, an attestation that does not verify), 2 for input that
//! could not be checked at all.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use contextgraph_types::record_attest::{RECORD_HASH_MEMBER, record_hash};
use serde_json::Value;

const OBSERVATION: &str = "observation.json";

fn inspect() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_contextgraph-inspect"));
    // Plain text, so assertions match words rather than ANSI escapes. Stdout
    // is a pipe here, which already turns color off; `NO_COLOR` says so
    // explicitly, and removing `CLICOLOR_FORCE` keeps a runner that forces
    // color from overriding either.
    command.env("NO_COLOR", "1").env_remove("CLICOLOR_FORCE");
    command
}

/// Run `contextgraph-inspect` with `args`, feeding `stdin` if given.
fn run(args: &[&str], stdin: Option<&[u8]>) -> Output {
    let mut command = inspect();
    command
        .args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn contextgraph-inspect");
    if let Some(bytes) = stdin {
        child
            .stdin
            .take()
            .expect("piped stdin")
            .write_all(bytes)
            .expect("write stdin");
    }
    child
        .wait_with_output()
        .expect("wait for contextgraph-inspect")
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout is UTF-8")
}

fn describe(output: &Output) -> String {
    format!(
        "status {:?}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("tests")
        .join("fixtures")
}

fn fixture(name: &str) -> String {
    fixtures_dir().join(name).display().to_string()
}

fn read_json(name: &str) -> Value {
    let path = fixtures_dir().join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

/// `(record_file, jcs_utf8, record_hash)` for every published vector.
fn vectors() -> Vec<(String, String, String)> {
    let vectors = read_json("record-hash-vectors.json");
    let entries = vectors["vectors"]
        .as_array()
        .expect("record-hash-vectors.json has a `vectors` array");
    assert!(!entries.is_empty(), "no record-hash vectors published");
    entries
        .iter()
        .map(|entry| {
            let field = |name: &str| {
                entry[name]
                    .as_str()
                    .unwrap_or_else(|| panic!("vector member `{name}` is a string"))
                    .to_string()
            };
            (
                field("record_file"),
                field("jcs_utf8"),
                field("record_hash"),
            )
        })
        .collect()
}

/// The published test key's public half, as the hex `--key` takes.
fn published_public_key() -> String {
    read_json("record-attestation-key.json")["public_key"]
        .as_str()
        .expect("record-attestation-key.json has a `public_key`")
        .to_string()
}

#[test]
fn record_help_lists_every_verb() {
    let output = run(&["record", "--help"], None);
    assert!(output.status.success(), "{}", describe(&output));
    let help = stdout(&output);
    for verb in ["hash", "preimage", "verify", "attest"] {
        assert!(
            help.contains(verb),
            "`record --help` does not list `{verb}`:\n{help}"
        );
    }
}

// ── record hash ──────────────────────────────────────────────────────────

#[test]
fn hash_prints_the_published_record_hash_of_every_fixture() {
    for (file, _, expected) in vectors() {
        let output = run(&["record", "hash", &fixture(&file)], None);
        assert!(output.status.success(), "{file}: {}", describe(&output));
        assert_eq!(
            stdout(&output),
            format!("{expected}\n"),
            "{file}: `record hash` must print the published record_hash and nothing else"
        );
    }
}

#[test]
fn hash_refuses_a_value_that_is_not_a_record() {
    let output = run(&["record", "hash", "-"], Some(b"[1, 2, 3]".as_slice()));
    assert_eq!(output.status.code(), Some(2), "{}", describe(&output));
    assert!(output.stdout.is_empty(), "{}", describe(&output));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("JSON object"),
        "{}",
        describe(&output)
    );
}

#[test]
fn hash_reports_an_unreadable_file_as_uncheckable() {
    let missing = fixture("no-such-record.json");
    let output = run(&["record", "hash", &missing], None);
    assert_eq!(output.status.code(), Some(2), "{}", describe(&output));
}

// ── record preimage ──────────────────────────────────────────────────────

#[test]
fn preimage_prints_the_published_canonical_bytes_of_every_fixture() {
    for (file, jcs, _) in vectors() {
        let output = run(&["record", "preimage", &fixture(&file)], None);
        assert!(output.status.success(), "{file}: {}", describe(&output));
        // Byte-exact, with no trailing newline: piping this into `sha256sum`
        // must reproduce the record_hash, so a single extra byte is a bug.
        assert_eq!(
            output.stdout,
            jcs.as_bytes(),
            "{file}: `record preimage` must print exactly the published JCS bytes"
        );
    }
}

#[test]
fn preimage_reads_standard_input_and_locates_an_edit() {
    let (_, jcs, _) = vectors()
        .into_iter()
        .find(|(file, _, _)| file == OBSERVATION)
        .expect("a vector for observation.json");
    let original = std::fs::read(fixtures_dir().join(OBSERVATION)).expect("read observation");

    let output = run(&["record", "preimage", "-"], Some(original.as_slice()));
    assert!(output.status.success(), "{}", describe(&output));
    assert_eq!(output.stdout, jcs.as_bytes());

    // The case the verb exists for: two parties disagree on a digest, and the
    // preimage shows where. An edited member appears in the bytes verbatim.
    let mut edited = read_json(OBSERVATION);
    edited["statement"] = Value::from("an edited statement");
    let output = run(
        &["record", "preimage", "-"],
        Some(edited.to_string().as_bytes()),
    );
    assert!(output.status.success(), "{}", describe(&output));
    let preimage = stdout(&output);
    assert_ne!(preimage, jcs);
    assert!(
        preimage.contains(r#""statement":"an edited statement""#),
        "{preimage}"
    );
    assert!(
        !preimage.contains(RECORD_HASH_MEMBER),
        "the record's own hash is removed from its preimage: {preimage}"
    );
}

// ── record verify ────────────────────────────────────────────────────────

#[test]
fn verify_accepts_every_fixture_as_current() {
    for (file, _, expected) in vectors() {
        let output = run(&["record", "verify", &fixture(&file)], None);
        assert!(output.status.success(), "{file}: {}", describe(&output));
        assert_eq!(stdout(&output), format!("current {expected}\n"), "{file}");
    }
}

#[test]
fn verify_reports_an_edited_record_as_stale() {
    let mut edited = read_json(OBSERVATION);
    let stored = edited[RECORD_HASH_MEMBER]
        .as_str()
        .expect("observation.json carries a record_hash")
        .to_string();
    edited["statement"] = Value::from("an edited statement");
    let recomputed = record_hash(&edited).expect("the edited record still hashes");

    let output = run(
        &["record", "verify", "-"],
        Some(edited.to_string().as_bytes()),
    );
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    let text = stdout(&output);
    assert!(text.starts_with("stale "), "{text}");
    assert!(text.contains(&stored), "names the stored hash: {text}");
    assert!(
        text.contains(&recomputed),
        "names the hash the content produces: {text}"
    );
}

#[test]
fn verify_reports_a_record_without_a_hash_as_unhashed() {
    let mut unhashed = read_json(OBSERVATION);
    let expected = unhashed[RECORD_HASH_MEMBER]
        .as_str()
        .expect("observation.json carries a record_hash")
        .to_string();
    unhashed
        .as_object_mut()
        .expect("a record is an object")
        .remove(RECORD_HASH_MEMBER);

    let output = run(
        &["record", "verify", "-"],
        Some(unhashed.to_string().as_bytes()),
    );
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    let text = stdout(&output);
    assert!(text.starts_with("unhashed "), "{text}");
    // Removing the member does not change the preimage, so the computed hash is
    // still the published one (profile LH1).
    assert!(text.contains(&expected), "{text}");
}

// ── record attest ────────────────────────────────────────────────────────

#[test]
fn attest_verifies_the_published_attestation_under_the_published_key() {
    let key = published_public_key();
    let output = run(
        &[
            "record",
            "attest",
            &fixture("record-attestation.json"),
            "--record",
            &fixture(OBSERVATION),
            "--key",
            &key,
        ],
        None,
    );
    assert!(output.status.success(), "{}", describe(&output));
    assert!(
        stdout(&output).starts_with("valid "),
        "{}",
        describe(&output)
    );
}

#[test]
fn attest_accepts_a_record_hash_in_place_of_the_record() {
    let key = published_public_key();
    let hash = read_json(OBSERVATION)[RECORD_HASH_MEMBER]
        .as_str()
        .expect("observation.json carries a record_hash")
        .to_string();
    let output = run(
        &[
            "record",
            "attest",
            &fixture("record-attestation.json"),
            "--record-hash",
            &hash,
            "--key",
            &key,
        ],
        None,
    );
    assert!(output.status.success(), "{}", describe(&output));
    assert!(
        stdout(&output).starts_with("valid "),
        "{}",
        describe(&output)
    );
}

#[test]
fn attest_rejects_the_attestation_over_a_different_record() {
    let key = published_public_key();
    let output = run(
        &[
            "record",
            "attest",
            &fixture("record-attestation.json"),
            "--record",
            &fixture("memory.json"),
            "--key",
            &key,
        ],
        None,
    );
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    let text = stdout(&output);
    assert!(text.starts_with("invalid "), "{text}");
    assert!(text.contains("commitment mismatch"), "{text}");
}

#[test]
fn attest_rejects_a_tampered_signature() {
    let key = published_public_key();
    let mut attestation = read_json("record-attestation.json");
    let signature = attestation["signature"]
        .as_str()
        .expect("the attestation carries a signature")
        .to_string();
    // Flip the first hex digit: still 64 bytes of hex, so the signature is
    // well-formed and the only thing wrong with it is that it does not verify.
    let flipped = if signature.starts_with('0') { "1" } else { "0" };
    attestation["signature"] = Value::from(format!("{flipped}{}", &signature[1..]));

    let output = run(
        &[
            "record",
            "attest",
            "-",
            "--record",
            &fixture(OBSERVATION),
            "--key",
            &key,
        ],
        Some(attestation.to_string().as_bytes()),
    );
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    let text = stdout(&output);
    assert!(text.starts_with("invalid "), "{text}");
    assert!(text.contains("bad signature"), "{text}");
}

#[test]
fn attest_rejects_a_key_of_the_wrong_length() {
    let output = run(
        &[
            "record",
            "attest",
            &fixture("record-attestation.json"),
            "--record",
            &fixture(OBSERVATION),
            "--key",
            "abcd",
        ],
        None,
    );
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    assert!(
        stdout(&output).contains("malformed key"),
        "{}",
        describe(&output)
    );
}

#[test]
fn attest_reports_a_key_that_is_not_hex_as_uncheckable() {
    let output = run(
        &[
            "record",
            "attest",
            &fixture("record-attestation.json"),
            "--record",
            &fixture(OBSERVATION),
            "--key",
            "not-hex!",
        ],
        None,
    );
    assert_eq!(output.status.code(), Some(2), "{}", describe(&output));
    assert!(output.stdout.is_empty(), "{}", describe(&output));
}

#[test]
fn attest_requires_the_signed_record_or_its_hash() {
    let key = published_public_key();
    let output = run(
        &[
            "record",
            "attest",
            &fixture("record-attestation.json"),
            "--key",
            &key,
        ],
        None,
    );
    // clap's usage-error status, the same "could not check" status the verbs
    // themselves use.
    assert_eq!(output.status.code(), Some(2), "{}", describe(&output));
}
