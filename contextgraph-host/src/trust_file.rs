//! The trust-store file: a documented, versioned JSON file an operator names,
//! and the loader and saver that turn it into a [`TrustStore`] and back
//! ([ADR 0029](https://github.com/macanderson/context-graph-protocol/blob/main/docs/adr/0029-the-trust-store-file.md)).
//!
//! [`TrustStore`] was always serde-able, and nothing read or wrote one, so
//! every host started empty and verified nothing unless it called
//! [`Host::trust_key`](crate::Host::trust_key) in code. An empty store fails to
//! a silent [`NoTrustedKey`](crate::AttestationState::NoTrustedKey) on every
//! frame — correct under F9, and easy never to notice. This module is the
//! missing path from a file to a store.
//!
//! # The format
//!
//! ```json
//! {
//!   "format": "contextgraph-trust/1",
//!   "providers": {
//!     "docs": [
//!       {
//!         "key_id": "docs-2026-08",
//!         "algorithm": "ed25519",
//!         "public_key": "8a88e3dd7409f195fd52db2d3cba5d72ca6709bf1d94121bf3748801b40f6f5c",
//!         "fingerprint": "sha256:…",
//!         "not_before": "2026-08-01T00:00:00Z",
//!         "not_after": "2027-07-31T23:59:59Z"
//!       }
//!     ]
//!   }
//! }
//! ```
//!
//! - `format` is required and must be exactly [`TRUST_FILE_FORMAT`]. A file
//!   written by a later format is refused by name, never half-read.
//! - `providers` maps a provider's **local id** — the id the operator
//!   registered it under, the same key consent is recorded under — to the keys
//!   it may sign with. Omitted or empty: a store that trusts nobody, written
//!   down on purpose.
//! - `key_id`, `algorithm` and `public_key` are required. `algorithm` must be
//!   `ed25519`, and `public_key` is its 32-byte key as 64 hex characters.
//! - `fingerprint` is optional on read and always written. When present it
//!   must equal [`TrustedKey::fingerprint`] of `public_key`, so a person who
//!   compared fingerprints with the provider's operator out of band can paste
//!   the one they compared, and a key that was edited afterwards is refused.
//! - `not_before` / `not_after` are the optional, inclusive validity window
//!   ([ADR 0028](https://github.com/macanderson/context-graph-protocol/blob/main/docs/adr/0028-key-validity-windows-are-evaluated-at-receipt.md)).
//! - `tier` is `"configured"` (the default, omitted on write) or `"pinned"`
//!   for a key the host recorded on first use
//!   ([ADR 0030](https://github.com/macanderson/context-graph-protocol/blob/main/docs/adr/0030-a-pinned-trust-tier-below-configured.md)).
//!   Persisting a pin is what makes a later change of key noticeable after a
//!   restart; an operator promotes a pinned key by deleting the member.
//! - **Any other member is an error.** This file decides what a host believes;
//!   a misspelt `not_afer` silently ignored would widen trust to forever.
//!
//! # Only a path the operator names
//!
//! Trusting a key is an operator act (ADR 0016), so there is no default
//! location, no search path, and no discovery: [`TrustStore::load`] reads the
//! path it is given and nothing else. A missing file is an error like any
//! other unreadable one — a host that wants "empty when absent" decides that
//! itself, visibly, rather than inheriting it from here.
//!
//! # Never a silently empty store
//!
//! Every way a file can fail — unreadable, too large, not JSON, the wrong
//! format, an unknown member, a key that does not decode, a fingerprint that
//! does not match, an inverted window, the same `key_id` twice — is a named
//! [`TrustFileError`]. Nothing here returns a partial store.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};

use contextgraph_types::{ALGORITHM_ED25519, KeyValidity, KeyValidityError};
use serde::{Deserialize, Serialize};

use crate::trust::{TrustStore, TrustTier, TrustedKey};

/// The `format` value this build reads and writes.
pub const TRUST_FILE_FORMAT: &str = "contextgraph-trust/1";

/// The largest trust file this loader reads. A store of a thousand keys is
/// well under 1 MiB; the cap is what keeps a mistyped path — a log, a disk
/// image, `/dev/zero` — from being read without bound.
pub const MAX_TRUST_FILE_BYTES: u64 = 1024 * 1024;

/// Why a trust file could not be loaded or saved. Every variant names the
/// problem; none of them is ever turned into an empty store.
#[derive(Debug, thiserror::Error)]
pub enum TrustFileError {
    /// The file could not be opened or read — missing, a directory, no
    /// permission.
    #[error("cannot read trust file {}: {source}", shown(.path))]
    Unreadable {
        /// The path the operator named.
        path: PathBuf,
        /// The I/O failure.
        #[source]
        source: std::io::Error,
    },
    /// The file could not be written, or the finished write could not be
    /// moved into place. The previous file, if any, is untouched.
    #[error("cannot write trust file {}: {source}", shown(.path))]
    Unwritable {
        /// The path the operator named.
        path: PathBuf,
        /// The I/O failure.
        #[source]
        source: std::io::Error,
    },
    /// The contents are not a valid trust file.
    #[error("{}{problem}", located(.path))]
    Invalid {
        /// The file, when the contents came from one.
        path: Option<PathBuf>,
        /// What is wrong with it.
        problem: TrustFileProblem,
    },
}

/// A path as an error message shows it.
fn shown(path: &Path) -> String {
    path.display().to_string()
}

/// The `trust file <path>: ` prefix a [`TrustFileError::Invalid`] carries
/// when its contents came from a file, and nothing when they did not.
fn located(path: &Option<PathBuf>) -> String {
    match path {
        Some(path) => format!("trust file {}: ", path.display()),
        None => String::new(),
    }
}

impl TrustFileError {
    /// The content problem, for an [`Invalid`](Self::Invalid) file.
    pub fn problem(&self) -> Option<&TrustFileProblem> {
        match self {
            Self::Invalid { problem, .. } => Some(problem),
            _ => None,
        }
    }

    fn invalid(problem: TrustFileProblem) -> Self {
        Self::Invalid {
            path: None,
            problem,
        }
    }

    fn at(self, file: &Path) -> Self {
        match self {
            Self::Invalid {
                path: None,
                problem,
            } => Self::Invalid {
                path: Some(file.to_path_buf()),
                problem,
            },
            other => other,
        }
    }
}

/// What is wrong with the contents of a trust file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustFileProblem {
    /// Larger than [`MAX_TRUST_FILE_BYTES`].
    TooLarge,
    /// Not JSON at all.
    NotJson(String),
    /// Missing, or not the [`TRUST_FILE_FORMAT`] this build reads. `found` is
    /// the value the file declared, if it declared a string.
    UnsupportedFormat {
        /// The declared `format`, if any.
        found: Option<String>,
    },
    /// JSON, and not the documented shape: a missing or mistyped member, or a
    /// member the format does not define.
    Shape(String),
    /// A provider id that is empty.
    EmptyProviderId,
    /// A key that cannot be trusted as written.
    InvalidKey {
        /// The provider it was listed under.
        provider_id: String,
        /// Its `key_id`, possibly empty.
        key_id: String,
        /// Why.
        reason: String,
    },
    /// A validity window that is malformed or inverted.
    InvalidWindow {
        /// The provider it was listed under.
        provider_id: String,
        /// The key it belongs to.
        key_id: String,
        /// The window's own error.
        error: KeyValidityError,
    },
    /// The same `key_id` listed twice for one provider. Refused rather than
    /// resolved: whichever copy won, the other was somebody's intent.
    DuplicateKeyId {
        /// The provider.
        provider_id: String,
        /// The repeated `key_id`.
        key_id: String,
    },
    /// A recorded `fingerprint` that does not match the key beside it — the
    /// key was edited after the fingerprint was compared, or pasted wrong.
    FingerprintMismatch {
        /// The provider.
        provider_id: String,
        /// The key.
        key_id: String,
        /// The fingerprint the file records.
        recorded: String,
        /// The fingerprint of the key the file holds.
        computed: String,
    },
}

impl fmt::Display for TrustFileProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => write!(f, "larger than {MAX_TRUST_FILE_BYTES} bytes"),
            Self::NotJson(why) => write!(f, "not JSON: {why}"),
            Self::UnsupportedFormat { found: Some(found) } => write!(
                f,
                "declares format `{found}`; this host reads `{TRUST_FILE_FORMAT}`"
            ),
            Self::UnsupportedFormat { found: None } => write!(
                f,
                "declares no string `format`; this host reads `{TRUST_FILE_FORMAT}`"
            ),
            Self::Shape(why) => write!(f, "not a {TRUST_FILE_FORMAT} document: {why}"),
            Self::EmptyProviderId => write!(f, "a provider id is empty"),
            Self::InvalidKey {
                provider_id,
                key_id,
                reason,
            } => write!(f, "key `{key_id}` for provider `{provider_id}`: {reason}"),
            Self::InvalidWindow {
                provider_id,
                key_id,
                error,
            } => write!(f, "key `{key_id}` for provider `{provider_id}`: {error}"),
            Self::DuplicateKeyId {
                provider_id,
                key_id,
            } => write!(
                f,
                "key `{key_id}` is listed twice for provider `{provider_id}`"
            ),
            Self::FingerprintMismatch {
                provider_id,
                key_id,
                recorded,
                computed,
            } => write!(
                f,
                "key `{key_id}` for provider `{provider_id}`: the recorded fingerprint \
                 {recorded} does not match the key, whose fingerprint is {computed}"
            ),
        }
    }
}

/// The document, exactly as the format defines it. Private: the public
/// surface is [`TrustStore`], and this type exists so the file can be strict
/// (`deny_unknown_fields`) without the in-memory store having to be.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrustFileDocument {
    format: String,
    #[serde(default)]
    providers: BTreeMap<String, Vec<TrustFileKey>>,
}

/// One key entry in the file.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrustFileKey {
    key_id: String,
    algorithm: String,
    public_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    not_before: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    not_after: Option<String>,
    #[serde(default, skip_serializing_if = "TrustTier::is_configured")]
    tier: TrustTier,
}

impl TrustStore {
    /// Load the trust file at `path` — a path **the operator named**
    /// (ADR 0016, ADR 0029). Nothing is searched for or discovered.
    ///
    /// Every failure is a named [`TrustFileError`], including a missing file;
    /// a malformed or unreadable file is never a silently empty store.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, TrustFileError> {
        let path = path.as_ref();
        let unreadable = |source| TrustFileError::Unreadable {
            path: path.to_path_buf(),
            source,
        };
        let file = std::fs::File::open(path).map_err(unreadable)?;
        let mut text = String::new();
        file.take(MAX_TRUST_FILE_BYTES + 1)
            .read_to_string(&mut text)
            .map_err(unreadable)?;
        if text.len() as u64 > MAX_TRUST_FILE_BYTES {
            return Err(TrustFileError::invalid(TrustFileProblem::TooLarge).at(path));
        }
        Self::from_trust_file_json(&text).map_err(|error| error.at(path))
    }

    /// Parse a trust file's contents. [`load`](Self::load) is this plus the
    /// read; it exists on its own for a host that keeps the document somewhere
    /// other than a file.
    pub fn from_trust_file_json(text: &str) -> Result<Self, TrustFileError> {
        if text.len() as u64 > MAX_TRUST_FILE_BYTES {
            return Err(TrustFileError::invalid(TrustFileProblem::TooLarge));
        }
        // Two passes, so a document from a later format is refused *as* a later
        // format rather than as an unknown member of this one.
        let value: serde_json::Value = serde_json::from_str(text).map_err(|error| {
            TrustFileError::invalid(TrustFileProblem::NotJson(error.to_string()))
        })?;
        let declared = value.get("format").and_then(serde_json::Value::as_str);
        if declared != Some(TRUST_FILE_FORMAT) {
            return Err(TrustFileError::invalid(
                TrustFileProblem::UnsupportedFormat {
                    found: declared.map(str::to_string),
                },
            ));
        }
        let document: TrustFileDocument = serde_json::from_value(value)
            .map_err(|error| TrustFileError::invalid(TrustFileProblem::Shape(error.to_string())))?;

        let mut store = TrustStore::new();
        for (provider_id, keys) in document.providers {
            if provider_id.is_empty() {
                return Err(TrustFileError::invalid(TrustFileProblem::EmptyProviderId));
            }
            let mut seen = BTreeSet::new();
            for entry in keys {
                let key = trusted_key_from_entry(&provider_id, entry)?;
                if !seen.insert(key.key_id.clone()) {
                    return Err(TrustFileError::invalid(TrustFileProblem::DuplicateKeyId {
                        provider_id,
                        key_id: key.key_id,
                    }));
                }
                store.trust(provider_id.clone(), key);
            }
        }
        Ok(store)
    }

    /// This store as a trust file: pretty-printed, providers and keys in a
    /// deterministic order, every key's fingerprint recorded beside it.
    ///
    /// Refuses a store holding a key the loader would refuse — hex that does
    /// not decode, an inverted window — so a file this writes is always one
    /// [`load`](Self::load) reads back.
    pub fn to_trust_file_json(&self) -> Result<String, TrustFileError> {
        let mut providers = BTreeMap::new();
        for provider_id in self.providers() {
            if provider_id.is_empty() {
                return Err(TrustFileError::invalid(TrustFileProblem::EmptyProviderId));
            }
            let mut entries = Vec::new();
            for key in self.keys_for(provider_id) {
                entries.push(entry_from_trusted_key(provider_id, key)?);
            }
            providers.insert(provider_id.to_string(), entries);
        }
        let document = TrustFileDocument {
            format: TRUST_FILE_FORMAT.to_string(),
            providers,
        };
        let mut text = serde_json::to_string_pretty(&document)
            .map_err(|error| TrustFileError::invalid(TrustFileProblem::Shape(error.to_string())))?;
        text.push('\n');
        Ok(text)
    }

    /// Write this store to `path` as a trust file.
    ///
    /// The write is atomic: the document goes to a temporary file beside
    /// `path`, is flushed to disk, and is renamed over `path` only once
    /// complete, so a crash mid-write leaves the previous file intact rather
    /// than a truncated one the next [`load`](Self::load) would refuse.
    ///
    /// The file holds public keys only, so it is not secret — but whoever can
    /// write it decides what this host believes. Keep it writable by the
    /// operator alone.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), TrustFileError> {
        let path = path.as_ref();
        let text = self.to_trust_file_json().map_err(|error| error.at(path))?;
        let unwritable = |source| TrustFileError::Unwritable {
            path: path.to_path_buf(),
            source,
        };
        let file_name = path.file_name().ok_or_else(|| {
            unwritable(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "the path names no file",
            ))
        })?;
        let mut temp_name = std::ffi::OsString::from(".");
        temp_name.push(file_name);
        temp_name.push(format!(".tmp-{}", std::process::id()));
        let temp = path.with_file_name(temp_name);

        let write = || -> std::io::Result<()> {
            use std::io::Write;
            let mut file = std::fs::File::create(&temp)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            std::fs::rename(&temp, path)
        };
        write().map_err(|source| {
            // Best effort: a failed write must not leave its temporary behind.
            let _ = std::fs::remove_file(&temp);
            unwritable(source)
        })
    }
}

/// Turn one file entry into a [`TrustedKey`], or name why it cannot be one.
fn trusted_key_from_entry(
    provider_id: &str,
    entry: TrustFileKey,
) -> Result<TrustedKey, TrustFileError> {
    let invalid_key = |key_id: &str, reason: String| {
        TrustFileError::invalid(TrustFileProblem::InvalidKey {
            provider_id: provider_id.to_string(),
            key_id: key_id.to_string(),
            reason,
        })
    };
    if entry.key_id.is_empty() {
        return Err(invalid_key("", "the key_id is empty".into()));
    }
    if entry.algorithm != ALGORITHM_ED25519 {
        return Err(invalid_key(
            &entry.key_id,
            format!(
                "algorithm `{}` is not one this host can verify (`{ALGORITHM_ED25519}`)",
                entry.algorithm
            ),
        ));
    }
    let Some(key) =
        TrustedKey::ed25519_hex(entry.key_id.clone(), entry.public_key.to_ascii_lowercase())
    else {
        return Err(invalid_key(
            &entry.key_id,
            "public_key is not a 32-byte ed25519 key as 64 hex characters".into(),
        ));
    };
    let validity = KeyValidity::new(entry.not_before, entry.not_after).map_err(|error| {
        TrustFileError::invalid(TrustFileProblem::InvalidWindow {
            provider_id: provider_id.to_string(),
            key_id: entry.key_id.clone(),
            error,
        })
    })?;
    if let Some(recorded) = entry.fingerprint {
        // `ed25519_hex` accepted the key, so it decodes and has a fingerprint.
        let computed = key.fingerprint().unwrap_or_default();
        if recorded != computed {
            return Err(TrustFileError::invalid(
                TrustFileProblem::FingerprintMismatch {
                    provider_id: provider_id.to_string(),
                    key_id: entry.key_id,
                    recorded,
                    computed,
                },
            ));
        }
    }
    Ok(TrustedKey {
        tier: entry.tier,
        ..key.with_validity(validity)
    })
}

/// Turn one [`TrustedKey`] into a file entry, refusing one the loader would
/// refuse.
fn entry_from_trusted_key(
    provider_id: &str,
    key: &TrustedKey,
) -> Result<TrustFileKey, TrustFileError> {
    let public_key = key.public_key.to_ascii_lowercase();
    // The loader's own acceptance test, applied before writing.
    let loadable = !key.key_id.is_empty()
        && TrustedKey::ed25519_hex(key.key_id.clone(), public_key.clone()).is_some();
    let Some(fingerprint) = key.fingerprint().filter(|_| loadable) else {
        return Err(TrustFileError::invalid(TrustFileProblem::InvalidKey {
            provider_id: provider_id.to_string(),
            key_id: key.key_id.clone(),
            reason: "not a well-formed ed25519 key with a non-empty key_id".into(),
        }));
    };
    key.validity.validate().map_err(|error| {
        TrustFileError::invalid(TrustFileProblem::InvalidWindow {
            provider_id: provider_id.to_string(),
            key_id: key.key_id.clone(),
            error,
        })
    })?;
    Ok(TrustFileKey {
        key_id: key.key_id.clone(),
        algorithm: ALGORITHM_ED25519.to_string(),
        public_key,
        fingerprint: Some(fingerprint),
        not_before: key.validity.not_before.clone(),
        not_after: key.validity.not_after.clone(),
        tier: key.tier,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use contextgraph_types::public_key_for;

    const SEED: [u8; 32] = [5u8; 32];

    fn key_hex() -> String {
        TrustedKey::ed25519_bytes("k", &public_key_for(&SEED)).public_key
    }

    fn problem(text: &str) -> TrustFileProblem {
        TrustStore::from_trust_file_json(text)
            .expect_err("the document must be refused")
            .problem()
            .cloned()
            .expect("a content problem, not an I/O one")
    }

    fn document(entry: &str) -> String {
        format!(r#"{{ "format": "contextgraph-trust/1", "providers": {{ "docs": [ {entry} ] }} }}"#)
    }

    #[test]
    fn a_well_formed_file_loads_every_key_with_its_window() {
        let text = document(&format!(
            r#"{{ "key_id": "docs-1", "algorithm": "ed25519", "public_key": "{}",
                 "not_before": "2026-01-01T00:00:00Z", "not_after": "2026-12-31T23:59:59Z" }}"#,
            key_hex()
        ));
        let store = TrustStore::from_trust_file_json(&text).expect("loads");
        let key = store.key("docs", "docs-1").expect("the key is trusted");
        assert_eq!(key.public_key, key_hex());
        assert_eq!(
            key.validity.not_after.as_deref(),
            Some("2026-12-31T23:59:59Z")
        );
    }

    #[test]
    fn a_file_that_trusts_nobody_says_so_and_loads_empty() {
        let store =
            TrustStore::from_trust_file_json(r#"{ "format": "contextgraph-trust/1" }"#).unwrap();
        assert!(store.is_empty());
    }

    #[test]
    fn text_that_is_not_json_is_named() {
        assert!(matches!(problem(""), TrustFileProblem::NotJson(_)));
        assert!(matches!(
            problem("{ not json"),
            TrustFileProblem::NotJson(_)
        ));
    }

    #[test]
    fn a_missing_or_foreign_format_is_refused_by_name() {
        assert_eq!(
            problem(r#"{ "providers": {} }"#),
            TrustFileProblem::UnsupportedFormat { found: None }
        );
        assert_eq!(
            problem(r#"{ "format": "contextgraph-trust/2", "anything": true }"#),
            TrustFileProblem::UnsupportedFormat {
                found: Some("contextgraph-trust/2".into())
            }
        );
        // The in-memory serde form of a store is not the file format.
        let serde_form = serde_json::to_string(&TrustStore::new()).unwrap();
        assert!(matches!(
            problem(&serde_form),
            TrustFileProblem::UnsupportedFormat { .. }
        ));
    }

    #[test]
    fn an_unknown_member_is_an_error_not_a_silent_widening() {
        // `not_afer`: ignored, it would make a windowed key trusted forever.
        let text = document(&format!(
            r#"{{ "key_id": "docs-1", "algorithm": "ed25519", "public_key": "{}",
                 "not_afer": "2026-12-31T23:59:59Z" }}"#,
            key_hex()
        ));
        assert!(matches!(problem(&text), TrustFileProblem::Shape(_)));
        assert!(matches!(
            problem(r#"{ "format": "contextgraph-trust/1", "providrs": {} }"#),
            TrustFileProblem::Shape(_)
        ));
    }

    #[test]
    fn a_key_that_cannot_be_trusted_as_written_is_named() {
        let bad_hex =
            document(r#"{ "key_id": "docs-1", "algorithm": "ed25519", "public_key": "not hex" }"#);
        assert!(matches!(
            problem(&bad_hex),
            TrustFileProblem::InvalidKey { ref key_id, .. } if key_id == "docs-1"
        ));

        let foreign = document(&format!(
            r#"{{ "key_id": "docs-1", "algorithm": "ml-dsa-65", "public_key": "{}" }}"#,
            key_hex()
        ));
        assert!(matches!(
            problem(&foreign),
            TrustFileProblem::InvalidKey { .. }
        ));

        let anonymous = document(&format!(
            r#"{{ "key_id": "", "algorithm": "ed25519", "public_key": "{}" }}"#,
            key_hex()
        ));
        assert!(matches!(
            problem(&anonymous),
            TrustFileProblem::InvalidKey { .. }
        ));

        let nobody = format!(
            r#"{{ "format": "contextgraph-trust/1", "providers": {{ "": [ {{ "key_id": "k",
                 "algorithm": "ed25519", "public_key": "{}" }} ] }} }}"#,
            key_hex()
        );
        assert_eq!(problem(&nobody), TrustFileProblem::EmptyProviderId);
    }

    #[test]
    fn an_inverted_window_is_named() {
        let text = document(&format!(
            r#"{{ "key_id": "docs-1", "algorithm": "ed25519", "public_key": "{}",
                 "not_before": "2027-01-01T00:00:00Z", "not_after": "2026-01-01T00:00:00Z" }}"#,
            key_hex()
        ));
        assert!(matches!(
            problem(&text),
            TrustFileProblem::InvalidWindow {
                error: KeyValidityError::Inverted { .. },
                ..
            }
        ));
    }

    #[test]
    fn the_same_key_id_twice_is_refused_rather_than_resolved() {
        let entry = format!(
            r#"{{ "key_id": "docs-1", "algorithm": "ed25519", "public_key": "{}" }}"#,
            key_hex()
        );
        let text = document(&format!("{entry}, {entry}"));
        assert_eq!(
            problem(&text),
            TrustFileProblem::DuplicateKeyId {
                provider_id: "docs".into(),
                key_id: "docs-1".into()
            }
        );
    }

    #[test]
    fn a_recorded_fingerprint_must_match_the_key_beside_it() {
        let good = TrustedKey::ed25519_bytes("docs-1", &public_key_for(&SEED))
            .fingerprint()
            .unwrap();
        let ok = document(&format!(
            r#"{{ "key_id": "docs-1", "algorithm": "ed25519", "public_key": "{}",
                 "fingerprint": "{good}" }}"#,
            key_hex()
        ));
        assert!(TrustStore::from_trust_file_json(&ok).is_ok());

        let other = TrustedKey::ed25519_bytes("x", &public_key_for(&[6u8; 32]))
            .fingerprint()
            .unwrap();
        let edited = document(&format!(
            r#"{{ "key_id": "docs-1", "algorithm": "ed25519", "public_key": "{}",
                 "fingerprint": "{other}" }}"#,
            key_hex()
        ));
        assert!(matches!(
            problem(&edited),
            TrustFileProblem::FingerprintMismatch { .. }
        ));
    }

    #[test]
    fn a_store_the_loader_would_refuse_is_never_written() {
        let mut store = TrustStore::new();
        store.trust(
            "docs",
            TrustedKey {
                public_key: "zz".repeat(32),
                ..TrustedKey::ed25519_bytes("docs-1", &public_key_for(&SEED))
            },
        );
        assert!(matches!(
            store.to_trust_file_json().unwrap_err().problem(),
            Some(TrustFileProblem::InvalidKey { .. })
        ));
    }

    #[test]
    fn the_written_form_is_deterministic_and_records_every_fingerprint() {
        let mut store = TrustStore::new();
        for provider in ["zeta", "alpha"] {
            for key_id in ["k-2", "k-1"] {
                store.trust(
                    provider,
                    TrustedKey::ed25519_bytes(key_id, &public_key_for(&SEED)),
                );
            }
        }
        let text = store.to_trust_file_json().unwrap();
        assert_eq!(text, store.clone().to_trust_file_json().unwrap());
        assert!(text.find("\"alpha\"").unwrap() < text.find("\"zeta\"").unwrap());
        assert!(text.find("\"k-1\"").unwrap() < text.find("\"k-2\"").unwrap());
        assert_eq!(text.matches("\"fingerprint\"").count(), 4);
        assert_eq!(TrustStore::from_trust_file_json(&text).unwrap(), store);
    }

    #[test]
    fn a_pin_survives_the_file_and_a_configured_key_writes_no_tier() {
        let mut store = TrustStore::new();
        store.trust(
            "docs",
            TrustedKey::ed25519_bytes("configured", &public_key_for(&SEED)),
        );
        let _ = store.pin(
            "docs",
            &crate::wire::AttesterKey {
                key_id: "pinned".into(),
                algorithm: ALGORITHM_ED25519.into(),
                public_key: key_hex(),
            },
        );
        let text = store.to_trust_file_json().unwrap();
        assert_eq!(text.matches("\"tier\"").count(), 1, "{text}");
        assert!(text.contains("\"tier\": \"pinned\""), "{text}");

        let back = TrustStore::from_trust_file_json(&text).unwrap();
        assert_eq!(back, store);
        assert_eq!(
            back.key("docs", "pinned").map(|key| key.tier),
            Some(TrustTier::Pinned)
        );
        assert_eq!(
            back.key("docs", "configured").map(|key| key.tier),
            Some(TrustTier::Configured)
        );
    }

    #[test]
    fn an_oversized_document_is_refused_before_it_is_parsed() {
        let text = " ".repeat(MAX_TRUST_FILE_BYTES as usize + 1);
        assert_eq!(problem(&text), TrustFileProblem::TooLarge);
    }
}
