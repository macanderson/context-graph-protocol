//! F5 end to end over the published range vectors
//! (`tests/vectors/range-vectors.json`, `SPEC.md` §6.2.1, F17).
//!
//! Every vector's resource is written to a real file, and the host's
//! [`verify_provenance_digest`] re-reads it over the vector's `range`:
//!
//! - a vector publishing bytes and a digest must verify — so the host hashes
//!   exactly the bytes a second implementation computed from the SPEC prose;
//! - a vector marked unverifiable must be [`Unreadable`], even when handed the
//!   digest of the whole resource — never a whole-resource fallback that would
//!   confirm bytes the range never named, and never a [`Mismatch`], which is
//!   the tampering signal and would turn a grammar disagreement into a false
//!   accusation.
//!
//! [`Unreadable`]: DigestVerification::Unreadable
//! [`Mismatch`]: DigestVerification::Mismatch

use std::path::PathBuf;

use contextgraph_host::{DigestVerification, verify_provenance_digest};
use contextgraph_types::Provenance;

/// A temp file removed on drop, even if an assertion panics.
struct TempFile(PathBuf);

impl TempFile {
    fn with_bytes(name: &str, bytes: &[u8]) -> Self {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "cgp-range-vector-{}-{name}.txt",
            std::process::id()
        ));
        std::fs::write(&path, bytes).expect("temp file must be writable");
        Self(path)
    }

    fn uri(&self) -> String {
        format!("file://{}", self.0.display())
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn file_link(uri: &str, range: Option<&str>, digest: &str) -> Provenance {
    Provenance {
        kind: "file".into(),
        uri: Some(uri.into()),
        range: range.map(str::to_owned),
        digest: Some(digest.into()),
        method: None,
        by: None,
    }
}

#[test]
fn the_host_verifies_every_published_range_digest_and_refuses_every_unverifiable_range() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/vectors/range-vectors.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} is the published range fixture: {e}", path.display()));
    let v: serde_json::Value = serde_json::from_str(&raw).expect("the fixture is JSON");

    let resources = v["resources"].as_object().expect("resources is an object");
    // The whole-resource digest of each resource, for the fallback probe below.
    let whole: std::collections::HashMap<&str, &str> = v["cases"]
        .as_array()
        .expect("cases is an array")
        .iter()
        .filter(|case| case.get("range").is_none())
        .filter_map(|case| Some((case["resource"].as_str()?, case["digest"].as_str()?)))
        .collect();

    for (index, case) in v["cases"].as_array().unwrap().iter().enumerate() {
        let name = case["resource"].as_str().unwrap();
        let file = TempFile::with_bytes(
            &format!("{index}-{name}"),
            resources[name].as_str().unwrap().as_bytes(),
        );
        let range = case.get("range").and_then(|r| r.as_str());
        let note = &case["note"];

        if let Some(digest) = case.get("digest").and_then(|d| d.as_str()) {
            assert_eq!(
                verify_provenance_digest(&file_link(&file.uri(), range, digest)),
                DigestVerification::Verified,
                "{name} {range:?}: {note}"
            );
            continue;
        }

        // Unverifiable. Offer the whole resource's digest (or any well-formed
        // one, for a resource with no whole-resource case): a verifier that
        // fell back to the whole file would report Verified, and one that
        // treated the range as a byte disagreement would report Mismatch.
        let bait = whole
            .get(name)
            .copied()
            .unwrap_or("sha256:0000000000000000000000000000000000000000000000000000000000000000");
        match verify_provenance_digest(&file_link(&file.uri(), range, bait)) {
            DigestVerification::Unreadable { reason } => {
                assert!(reason.contains("range"), "{name} {range:?}: {reason}");
            }
            other => panic!("{name} {range:?} must be Unreadable, got {other:?} ({note})"),
        }
    }
}
