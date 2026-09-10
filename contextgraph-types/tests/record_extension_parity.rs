//! A `ContextRecord` must be able to *hold* the wire it was handed.
//!
//! `SPEC.md` §13 U1 tells a receiver to ignore members it does not recognise,
//! and §13 U3 makes namespaced extension members a first-class part of the
//! wire. A record is also content-addressed (profile LH1), so "ignore" has to
//! mean "carry through untouched": a host that parses an extended record, drops
//! the member it did not model, and re-hashes computes a **different** identity
//! for the same record — a wrong digest, reported as a success (issue #118).
//!
//! This suite holds three properties that together close that hole:
//!
//!   1. [`RESERVED_RECORD_MEMBERS`] is exactly the set of top-level member
//!      names the reference types emit, so the catch-all
//!      [`ContextRecord::extra`] and the flattened [`RecordBody`] stay disjoint
//!      as the types change. This is the load-bearing invariant: both fields are
//!      `#[serde(flatten)]`, and serde offers *every* leftover member to *each*
//!      of them.
//!   2. An unmodelled member round-trips byte-for-byte, and hashes the same
//!      whether the record is hashed as a value or as a type.
//!   3. `extensions` accepts any JSON value, as its schema declares — not only
//!      the string-valued case.
//!
//! The fixture-wide version of property 2 lives in `contextgraph-conformance`'s
//! `record_hash_typed_wire_parity`, beside the fixtures it reads.

use std::collections::{BTreeMap, BTreeSet};

use contextgraph_types::extension::ExtensionValue;
use contextgraph_types::record::{RESERVED_RECORD_MEMBERS, is_reserved_record_member};
use contextgraph_types::{
    ConstraintEffect, ContextRecord, ContractRequirement, DirectiveKind, Enforcement,
    KnowledgeKind, LIFECYCLE_SCHEMA_VERSION, OriginClass, RecordBody, RecordLink, RecordProvenance,
    RecordScope, RecordStatus, RequirementResult, SharingScope, ValidationOutcome,
};
use serde::Deserialize;
use serde::de::{IgnoredAny, MapAccess, Visitor};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// Property 1 — the reserved list is exactly the reference types' vocabulary.
// ---------------------------------------------------------------------------

#[test]
fn reserved_members_are_sorted_unique_and_unnamespaced() {
    let mut sorted = RESERVED_RECORD_MEMBERS.to_vec();
    sorted.sort_unstable();
    assert_eq!(
        sorted.as_slice(),
        RESERVED_RECORD_MEMBERS,
        "RESERVED_RECORD_MEMBERS must stay sorted — membership is a binary search"
    );
    let unique: BTreeSet<&&str> = RESERVED_RECORD_MEMBERS.iter().collect();
    assert_eq!(unique.len(), RESERVED_RECORD_MEMBERS.len(), "duplicate entry");
    for name in RESERVED_RECORD_MEMBERS {
        assert!(
            !name.contains(':'),
            "{name} contains ':', so a SPEC.md §13 U3 vendor member could collide with it"
        );
    }
}

/// The drift guard. If a member is added to the envelope or to any `RecordBody`
/// variant and not to [`RESERVED_RECORD_MEMBERS`], `extra` would capture it on
/// the way in and emit it a second time on the way out; if a name is removed
/// from the types and left in the list, a record carrying it would lose it.
/// Both directions fail here rather than in a digest.
#[test]
fn reserved_members_are_exactly_what_the_types_emit() {
    let mut emitted = BTreeSet::new();
    for body in every_body() {
        let value = serde_json::to_value(fully_populated_record(body)).expect("serializes");
        let object = value.as_object().expect("a record is a JSON object");
        emitted.extend(object.keys().cloned());
    }
    let reserved: BTreeSet<String> = RESERVED_RECORD_MEMBERS
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    assert_eq!(
        emitted, reserved,
        "RESERVED_RECORD_MEMBERS must equal the union of every member the \
         envelope and the twelve record bodies serialize"
    );
}

// ---------------------------------------------------------------------------
// Property 2 — an unmodelled member survives, and hashes the same either way.
// ---------------------------------------------------------------------------

/// The exact bug in issue #118: a namespaced member no reference type models.
fn extended_wire() -> Value {
    let mut wire = knowledge_wire();
    let object = wire.as_object_mut().expect("object");
    object.insert(
        "acme:review".to_string(),
        json!({"channel": "slack:#context-review", "escalations": 2, "urgent": false}),
    );
    object.insert("acme:weight".to_string(), json!(0.25));
    object.insert("acme:tags".to_string(), json!(["a", "b"]));
    object.insert("acme:absent".to_string(), Value::Null);
    wire
}

#[test]
fn an_unmodelled_member_survives_the_round_trip() {
    let wire = extended_wire();
    let record: ContextRecord = serde_json::from_value(wire.clone()).expect("parses");

    assert_eq!(
        record.extra.keys().cloned().collect::<Vec<_>>(),
        vec![
            "acme:absent".to_string(),
            "acme:review".to_string(),
            "acme:tags".to_string(),
            "acme:weight".to_string()
        ],
        "every unmodelled member is kept, and only those"
    );
    assert_eq!(record.extra["acme:absent"], ExtensionValue::Null);
    assert_eq!(record.extra["acme:weight"], ExtensionValue::Float(0.25));

    assert_eq!(
        serde_json::to_value(&record).expect("re-serializes"),
        wire,
        "an extended record must re-serialize to the bytes it was parsed from"
    );
    let encoded = serde_json::to_string(&record).expect("re-serializes");
    let keys = top_level_keys(&encoded);
    let unique: BTreeSet<&String> = keys.iter().collect();
    assert_eq!(
        keys.len(),
        unique.len(),
        "the two flattened fields must not both emit the same member: {encoded}"
    );
}

#[test]
fn a_record_without_extensions_is_byte_identical_to_before() {
    let wire = knowledge_wire();
    let record: ContextRecord = serde_json::from_value(wire.clone()).expect("parses");
    assert!(record.extra.is_empty(), "nothing unmodelled to keep");
    assert_eq!(
        serde_json::to_value(&record).expect("re-serializes"),
        wire,
        "an empty catch-all must flatten to no members at all"
    );
}

#[cfg(feature = "record-hash")]
#[test]
fn typed_and_wire_hash_agree_for_an_unmodelled_member() {
    use contextgraph_types::record_attest::{record_hash, record_hash_of};

    let wire = extended_wire();
    let record: ContextRecord = serde_json::from_value(wire.clone()).expect("parses");
    assert_eq!(
        record_hash_of(&record).expect("typed hash"),
        record_hash(&wire).expect("wire hash"),
        "hashing a relayed record as a type must not differ from hashing its bytes"
    );
}

/// The member has to be *in* the preimage, not merely survive parsing: a record
/// that carries an extension is a different record from one that does not.
#[cfg(feature = "record-hash")]
#[test]
fn an_unmodelled_member_changes_the_record_hash() {
    use contextgraph_types::record_attest::record_hash;

    assert_ne!(
        record_hash(&extended_wire()).expect("hash"),
        record_hash(&knowledge_wire()).expect("hash"),
        "an extension member is content, so it must move the content address"
    );
}

/// A hand-built `extra` cannot smuggle a duplicate onto the wire, whichever
/// reserved name it names.
#[cfg(feature = "record-hash")]
#[test]
fn extra_never_emits_a_member_the_types_already_carry() {
    use contextgraph_types::record_attest::{record_hash, record_hash_of};

    let clean = fully_populated_record(RecordBody::Knowledge {
        knowledge_kind: KnowledgeKind::Fact,
        statement: "the retry ceiling is five".into(),
    });
    let mut smuggled = clean.clone();
    for name in RESERVED_RECORD_MEMBERS {
        smuggled
            .extra
            .insert((*name).to_string(), ExtensionValue::String("forged".into()));
    }

    let encoded = serde_json::to_string(&smuggled).expect("serializes");
    let keys = top_level_keys(&encoded);
    let unique: BTreeSet<&String> = keys.iter().collect();
    assert_eq!(keys.len(), unique.len(), "duplicate member on the wire: {encoded}");
    assert_eq!(
        record_hash_of(&smuggled).expect("hash"),
        record_hash_of(&clean).expect("hash"),
        "a reserved name in `extra` is inert"
    );
    assert_eq!(
        record_hash_of(&clean).expect("hash"),
        record_hash(&serde_json::to_value(&clean).expect("serializes")).expect("hash"),
    );
}

// ---------------------------------------------------------------------------
// Property 3 — `extensions` is the open object its schema declares.
// ---------------------------------------------------------------------------

#[test]
fn extensions_hold_any_json_value() {
    let mut wire = knowledge_wire();
    wire.as_object_mut().expect("object").insert(
        "extensions".to_string(),
        json!({
            "acme:channel": "slack:#context-review",
            "acme:retries": 3,
            "acme:ratio": 0.5,
            "acme:enabled": true,
            "acme:nested": {"depth": 2},
            "acme:list": [1, "two"],
            "acme:none": null
        }),
    );

    let record: ContextRecord = serde_json::from_value(wire.clone()).expect("parses");
    let extensions = record.extensions.as_ref().expect("present");
    assert_eq!(
        extensions["acme:retries"],
        ExtensionValue::UnsignedInteger(3),
        "an integer must not be widened to a float, or `3` would re-serialize as `3.0`"
    );
    assert_eq!(
        extensions["acme:channel"].as_str(),
        Some("slack:#context-review")
    );
    assert_eq!(
        serde_json::to_value(&record).expect("re-serializes"),
        wire,
        "an open extensions object must round-trip byte-for-byte"
    );
}

#[test]
fn is_reserved_record_member_agrees_with_the_list() {
    assert!(is_reserved_record_member("record_kind"));
    assert!(is_reserved_record_member("statement"));
    assert!(is_reserved_record_member("schema_version"));
    assert!(!is_reserved_record_member("acme:review"));
    assert!(!is_reserved_record_member("statements"));
}

// ---------------------------------------------------------------------------
// Helpers.
// ---------------------------------------------------------------------------

/// The `knowledge.json` fixture's shape, inline so this suite does not depend
/// on a file outside the published crate.
fn knowledge_wire() -> Value {
    json!({
        "schema_version": LIFECYCLE_SCHEMA_VERSION,
        "record_id": "rec_know_0001",
        "lineage_id": "lin_know_0001",
        "record_status": "active",
        "scope": {"organization_id": "org_acme", "repository_id": "repo_stella"},
        "sharing_scope": "organization",
        "observed_at": "2026-07-29T14:00:00Z",
        "origin": "declared",
        "record_hash": "sha256:2d3a4530de7392338a66f321e798e65398c1b51955f622ceceaac24c709212c2",
        "provenance": {
            "origin_provider_id": "provider_example",
            "producer_kind": "human",
            "producer_ref": "user_mac"
        },
        "confidence": 0.95,
        "record_kind": "knowledge",
        "knowledge_kind": "fact",
        "statement": "the retry ceiling for the deploy pipeline is five attempts"
    })
}

/// An envelope with **every** optional member present, so the union taken in
/// `reserved_members_are_exactly_what_the_types_emit` is complete.
fn fully_populated_record(body: RecordBody) -> ContextRecord {
    ContextRecord {
        schema_version: LIFECYCLE_SCHEMA_VERSION.to_string(),
        record_id: "rec_0001".into(),
        lineage_id: "lin_0001".into(),
        record_status: RecordStatus::Active,
        scope: RecordScope {
            repository_id: Some("repo_stella".into()),
            ..RecordScope::default()
        },
        sharing_scope: SharingScope::Repository,
        sensitivity: Some("internal".into()),
        observed_at: "2026-07-29T14:00:00Z".into(),
        valid_from: Some("2026-07-29T00:00:00Z".into()),
        confidence: Some(0.9),
        origin: OriginClass::Observed,
        evidence_links: vec!["rec_evid_0001".into()],
        record_links: vec![RecordLink {
            rel: "refines".into(),
            target_record_id: "rec_prior_0001".into(),
        }],
        record_hash: format!("sha256:{}", "a".repeat(64)),
        provenance: RecordProvenance {
            origin_provider_id: "provider_example".into(),
            origin_authority_id: Some("authority_acme".into()),
            producer_kind: "agent".into(),
            producer_ref: Some("agent://trace-miner".into()),
            derivation_kind: None,
            source_refs: vec!["rec_src_0001".into()],
        },
        extensions: Some(BTreeMap::from([(
            "acme:channel".to_string(),
            ExtensionValue::String("slack:#context-review".into()),
        )])),
        body,
        extra: BTreeMap::new(),
    }
}

/// Every variant, with every optional body member populated.
fn every_body() -> Vec<RecordBody> {
    vec![
        RecordBody::Observation {
            statement: "the build failed".into(),
            subject_ref: Some("run_991".into()),
        },
        RecordBody::Knowledge {
            knowledge_kind: KnowledgeKind::Fact,
            statement: "the retry ceiling is five".into(),
        },
        RecordBody::Memory {
            statement: "the user prefers terse diffs".into(),
            salience: Some(0.7),
        },
        RecordBody::Directive {
            directive_kind: DirectiveKind::Constraint,
            statement: "never write secrets to logs".into(),
            constraint_effect: Some(ConstraintEffect::Forbid),
            enforcement: Some(Enforcement::Blocking),
            procedure_steps: vec!["redact".into()],
        },
        RecordBody::RecordProposal {
            proposed_kind: "directive".into(),
            rationale: "recurred three times".into(),
        },
        RecordBody::Evidence {
            statement: "log line shows the timeout".into(),
            evidence_kind: Some("log".into()),
        },
        RecordBody::ArtifactContract {
            contract_name: "api-handler".into(),
            requirements: vec![ContractRequirement {
                requirement_kind: "command".into(),
                description: Some("cargo test passes".into()),
                execution_approval_ref: Some("approval_1".into()),
            }],
        },
        RecordBody::ContractValidation {
            contract_ref: "rec_contract_1".into(),
            outcome: ValidationOutcome::Pass,
            requirement_results: vec![RequirementResult {
                requirement_kind: "command".into(),
                outcome: ValidationOutcome::Pass,
                detail: Some("42 tests".into()),
            }],
        },
        RecordBody::OutcomeAssessment {
            subject_ref: "rec_task_1".into(),
            assessment: "resolved".into(),
            rating: Some(0.8),
        },
        RecordBody::PromotionEvent {
            subject_ref: "rec_dir_1".into(),
            to_status: "active".into(),
            from_status: Some("proposed".into()),
        },
        RecordBody::ContextUse {
            used_record_ref: "rec_know_1".into(),
            selected: true,
            rendered: true,
            cited: false,
            task_ref: Some("task_9".into()),
        },
        RecordBody::ContextUseFeedback {
            context_use_ref: "rec_use_1".into(),
            feedback: "not helpful".into(),
            rating: Some(0.2),
        },
    ]
}

/// The top-level member names of a JSON object **in wire order, with
/// duplicates**. `serde_json::Value` silently collapses a repeated name, which
/// is exactly the failure this has to be able to see.
fn top_level_keys(encoded: &str) -> Vec<String> {
    struct Keys(Vec<String>);
    impl<'de> Visitor<'de> for Keys {
        type Value = Vec<String>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a JSON object")
        }
        fn visit_map<A: MapAccess<'de>>(mut self, mut map: A) -> Result<Self::Value, A::Error> {
            while let Some(name) = map.next_key::<String>()? {
                map.next_value::<IgnoredAny>()?;
                self.0.push(name);
            }
            Ok(self.0)
        }
    }
    struct Wrapper(Vec<String>);
    impl<'de> Deserialize<'de> for Wrapper {
        fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            d.deserialize_map(Keys(Vec::new())).map(Wrapper)
        }
    }
    serde_json::from_str::<Wrapper>(encoded)
        .expect("valid JSON object")
        .0
}
