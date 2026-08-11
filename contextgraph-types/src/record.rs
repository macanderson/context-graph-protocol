//! `ContextRecord` — the immutable, provenance-bearing unit of the
//! **Context Exchange Provider** lifecycle profile
//! (`contextgraph/lifecycle/1.0-draft`, issue #28).
//!
//! Where a [`ContextFrame`](crate::ContextFrame) is the *read* unit a provider
//! returns from `context/query`, a `ContextRecord` is the *exchange* unit a
//! Context Exchange Provider appends, gets, and resolves: a durable, content-
//! addressed record with a common envelope and a discriminated `record_kind`
//! body. The profile — not the frozen `contextgraph/1.0` core — owns this layer
//! (ADR 0007 §4, `docs/profiles/context-exchange-provider.md`).
//!
//! ## Shape (mirrors `schema/contextgraph-lifecycle-record.schema.json`)
//!
//! Every record carries the same **envelope** (`schema_version`, `record_id`,
//! `lineage_id`, `record_status`, `scope`, `sharing_scope`, `observed_at`,
//! `origin`, `provenance`, `record_hash`, and the optional temporal/confidence/
//! link/extension fields) plus a flat, `record_kind`-discriminated body. The
//! JSON is flat and snake_case: the discriminant `record_kind` sits at the same
//! level as the body's fields, exactly like the envelope's internally-tagged
//! `type` on the wire.
//!
//! ## Immutability & identity
//!
//! A record is never mutated in place. A correction is a **new** record sharing
//! the earlier one's `lineage_id`; `record_status` moves `active → retracted`
//! or `active → archived` (three values — "superseded" is *derived* from
//! `lineage_id`, never stored, per reconciliation row B5). `record_hash` is the
//! `sha256:<hex>` over the RFC 8785 (JCS) canonicalization of the record with
//! its own `record_hash` member omitted from the preimage; the detached
//! [`RecordAttestation`] signs that hash and travels as ledger metadata beside
//! the record, never inside its hash preimage (reconciliation row C5).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::validate::{is_protocol_timestamp, is_well_formed_digest};

/// The profile version every `ContextRecord.schema_version` names. Distinct
/// from the wire [`PROTOCOL_VERSION`](crate::PROTOCOL_VERSION)
/// (`contextgraph/1.0`): the lifecycle layer is a *profile* on top of the
/// base family (ADR 0007 §5, reconciliation row D4), so it version-stamps
/// itself rather than riding the core version.
pub const LIFECYCLE_SCHEMA_VERSION: &str = "contextgraph/lifecycle/1.0-draft";

/// Lifecycle status of a record (reconciliation row B5). Exactly three values:
/// a host may keep richer internal states, but the wire status is these three.
/// `superseded` is **not** here — it is derived from a later record on the same
/// [`lineage_id`](ContextRecord::lineage_id), never stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordStatus {
    /// The record is in force.
    Active,
    /// Withdrawn by its author or authority; no longer asserted.
    Retracted,
    /// Retired from active use but retained for audit.
    Archived,
}

/// Who a record is shared with (reconciliation row E3). Conjunctive with
/// [`RecordScope`]: `sharing_scope` widens visibility *within* the scope keys
/// present, it does not replace them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SharingScope {
    /// Visible only to the owning user.
    User,
    /// Visible across the repository.
    Repository,
    /// Visible across the workspace.
    Workspace,
    /// Visible across the organization.
    Organization,
}

/// The coarse origin class of a record, keyed by the origin→derivation validity
/// matrix (reconciliation row C5). The full structured detail lives in
/// [`RecordProvenance`]; this is the one axis that constrains which
/// `provenance.derivation_kind` values are meaningful:
///
/// | `origin`  | valid `provenance.derivation_kind` |
/// |-----------|-------------------------------------|
/// | `observed` | absent (a first-hand observation is not derived) |
/// | `derived`  | required (`summarization`, `inference`, `transformation`, …) |
/// | `declared` | absent (an authored assertion) |
/// | `imported` | optional (may name the upstream derivation) |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OriginClass {
    /// First-hand observation of a trace, log, or event.
    Observed,
    /// Produced from other records by summarization/inference/transformation.
    Derived,
    /// Authored directly by a human or agent as an assertion.
    Declared,
    /// Ingested from an upstream store or provider.
    Imported,
}

/// The 7-key portable scope (reconciliation row E3). Every key is optional and
/// the present keys are **conjunctive** (AND): a record scoped to
/// `{repository_id, workspace_id}` belongs to that repository *and* that
/// workspace. `tenant_id` and `project_id` are deliberately **absent** from the
/// portable core — there is no cross-provider registry contract for them yet
/// (rows E2/E3), so a host keys on them only internally.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordScope {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
}

impl RecordScope {
    /// Whether any scope key is set. An all-empty scope names nowhere and is a
    /// smell a provider **SHOULD** reject, though the type permits it so a
    /// deserializer never fails on a sparse record.
    pub fn is_empty(&self) -> bool {
        self.user_id.is_none()
            && self.organization_id.is_none()
            && self.repository_id.is_none()
            && self.workspace_id.is_none()
            && self.environment_id.is_none()
            && self.session_id.is_none()
            && self.task_id.is_none()
    }
}

/// Structured provenance for a record (reconciliation row C5). Distinct from the
/// frame-layer [`Provenance`](crate::Provenance): a record's provenance names
/// the *producing* provider and authority and how the value was derived, not a
/// file/range digest chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordProvenance {
    /// The provider that first produced this record.
    pub origin_provider_id: String,
    /// The authority (tenant/principal namespace) the record was produced under.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_authority_id: Option<String>,
    /// The class of producer. Open vocabulary; recommended values `human`,
    /// `agent`, `tool`, `system`.
    pub producer_kind: String,
    /// A stable reference to the producer (an agent id, tool name, user id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub producer_ref: Option<String>,
    /// How a `derived`/`imported` record was produced. Open vocabulary;
    /// recommended values `summarization`, `inference`, `transformation`,
    /// `import`. Absent for `observed`/`declared` origins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derivation_kind: Option<String>,
    /// Records or frames this one was derived from, closest-source first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_refs: Vec<String>,
}

/// A typed link from one record to another (an open `rel` vocabulary, namespaced
/// per SPEC.md §13 U3). Distinct from `evidence_links`, which are bare refs to
/// supporting evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordLink {
    /// The relationship, e.g. `supersedes`, `refines`, `contradicts`. Open;
    /// a vendor-specific rel MUST be namespaced (`vendor:rel`).
    pub rel: String,
    /// The `record_id` this link points at.
    pub target_record_id: String,
}

/// A detached attestation over a record's `record_hash` (reconciliation row C5,
/// shared with issue #12). It is **never** part of the record or its hash
/// preimage — it travels as ledger metadata beside the record, so re-signing or
/// key rotation never perturbs the content-addressed identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordAttestation {
    /// The `sha256:<hex>` `record_hash` this attestation signs.
    pub signed_record_hash: String,
    /// The signing key's id; validity windows govern rotation.
    pub key_id: String,
    /// The signature algorithm, e.g. `ed25519`.
    pub algorithm: String,
    /// The attesting authority.
    pub attester_id: String,
    /// The detached signature (base64/hex per algorithm).
    pub signature: String,
    /// When the attestation was issued (protocol timestamp).
    pub issued_at: String,
}

/// A knowledge record's sub-kind (reconciliation rows B2/D1). `memory` and
/// `fact` are **not** directive kinds; `fact` is a knowledge kind here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeKind {
    Fact,
    Assumption,
    Decision,
}

/// The four **portable** directive kinds (ADR 0007 §4, reconciliation row B3).
/// The six-kind taxonomy in the superseded downstream drafts is a host-runtime
/// convenience, not a wire contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectiveKind {
    Preference,
    Rule,
    Constraint,
    Procedure,
}

/// What a `constraint` directive does. Deliberately only `require`/`forbid` —
/// **never `allow`**: authorization stays host-side (ADR 0007 §3, row B3). A
/// record carrying a constraint is a stored value, not a grant of authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintEffect {
    Require,
    Forbid,
}

/// How strongly a directive is meant to bind. `blocking` is a *recorded intent*,
/// not an enforcement grant — the host still decides whether to enforce it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Enforcement {
    Advisory,
    Blocking,
}

/// The outcome of a contract validation or a single requirement check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationOutcome {
    Pass,
    Fail,
    Inconclusive,
}

/// One requirement of an artifact contract (reconciliation row D6). The
/// `requirement_kind` is an open vocabulary; the reference validator recognises
/// ten kinds. A `command` requirement carries an `execution_approval_ref` — a
/// pointer to an out-of-band approval, **not** an authorization to execute:
/// contract *execution* is a host concern (ADR 0007 §3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractRequirement {
    /// e.g. `file_exists`, `content_matches`, `command`, `schema_valid`. Open.
    pub requirement_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Present for `command` requirements: a reference to the approval that
    /// authorizes running it. The protocol carries the reference; it never
    /// authorizes execution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_approval_ref: Option<String>,
}

/// The result of checking one requirement, carried by a `contract_validation`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequirementResult {
    pub requirement_kind: String,
    pub outcome: ValidationOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// The `record_kind`-discriminated body of a [`ContextRecord`] — the 12 portable
/// record kinds (reconciliation row D1). Internally tagged by `record_kind` and
/// flattened into the envelope, so a record's JSON is one flat object.
///
/// Every variant's *schema* is portable; the *execution*, *promotion*, and
/// *judging* they might imply are host concerns and stay out of the protocol
/// (ADR 0007 §3, rows D6/D7).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "record_kind", rename_all = "snake_case")]
pub enum RecordBody {
    /// A first-hand observation of a trace, log, git event, or user behavior.
    Observation {
        statement: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subject_ref: Option<String>,
    },
    /// A fact, assumption, or decision (see [`KnowledgeKind`]).
    Knowledge {
        knowledge_kind: KnowledgeKind,
        statement: String,
    },
    /// A remembered episode or salient fact. A distinct record kind — memory is
    /// **not** a directive kind (ADR 0007 §4).
    Memory {
        statement: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        salience: Option<f64>,
    },
    /// A portable directive: preference/rule/constraint/procedure. Carrying a
    /// directive record is not the same as a frame instructing a model — the
    /// host still decides whether it is admitted or enforced (ADR 0007 §4).
    Directive {
        directive_kind: DirectiveKind,
        statement: String,
        /// Required when `directive_kind == constraint` (see
        /// [`ContextRecord::envelope_invariants`]); absent otherwise.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        constraint_effect: Option<ConstraintEffect>,
        /// Absent ⇒ `advisory`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        enforcement: Option<Enforcement>,
        /// Ordered steps for a `procedure` directive.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        procedure_steps: Vec<String>,
    },
    /// A proposal that some record be created/promoted — recorded so the
    /// *decision* is auditable. The decision itself is a host concern (row D7).
    RecordProposal {
        proposed_kind: String,
        rationale: String,
    },
    /// Supporting evidence for another record.
    Evidence {
        statement: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        evidence_kind: Option<String>,
    },
    /// A contract an artifact must satisfy (reconciliation row D6). The protocol
    /// carries it; the host executes it.
    ArtifactContract {
        contract_name: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        requirements: Vec<ContractRequirement>,
    },
    /// The recorded result of validating an [`ArtifactContract`](RecordBody::ArtifactContract).
    ContractValidation {
        contract_ref: String,
        outcome: ValidationOutcome,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        requirement_results: Vec<RequirementResult>,
    },
    /// A recorded assessment of an outcome. Semantic judging is host-side; this
    /// records the judgment as an immutable event (row D7).
    OutcomeAssessment {
        subject_ref: String,
        assessment: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rating: Option<f64>,
    },
    /// An immutable event recording that a host promoted a record. When to
    /// promote (thresholds, policy) stays host-side (row D7).
    PromotionEvent {
        subject_ref: String,
        to_status: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from_status: Option<String>,
    },
    /// A record that some context (record/frame) was used in a task. Overlaps
    /// the usage-report U1 surface; carried here for durable audit (row D7).
    ContextUse {
        used_record_ref: String,
        selected: bool,
        rendered: bool,
        cited: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        task_ref: Option<String>,
    },
    /// Feedback on a prior [`ContextUse`](RecordBody::ContextUse).
    ContextUseFeedback {
        context_use_ref: String,
        feedback: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rating: Option<f64>,
    },
}

impl RecordBody {
    /// The wire `record_kind` discriminant for this body.
    pub fn record_kind(&self) -> &'static str {
        match self {
            RecordBody::Observation { .. } => "observation",
            RecordBody::Knowledge { .. } => "knowledge",
            RecordBody::Memory { .. } => "memory",
            RecordBody::Directive { .. } => "directive",
            RecordBody::RecordProposal { .. } => "record_proposal",
            RecordBody::Evidence { .. } => "evidence",
            RecordBody::ArtifactContract { .. } => "artifact_contract",
            RecordBody::ContractValidation { .. } => "contract_validation",
            RecordBody::OutcomeAssessment { .. } => "outcome_assessment",
            RecordBody::PromotionEvent { .. } => "promotion_event",
            RecordBody::ContextUse { .. } => "context_use",
            RecordBody::ContextUseFeedback { .. } => "context_use_feedback",
        }
    }
}

/// One immutable, content-addressed exchange record. The common envelope plus a
/// flat, `record_kind`-discriminated [`RecordBody`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextRecord {
    /// Always [`LIFECYCLE_SCHEMA_VERSION`].
    pub schema_version: String,
    /// Stable, provider-scoped identity of this exact record.
    pub record_id: String,
    /// Groups every revision of the same logical item; supersession is derived
    /// from this, never stored as a status (row B5).
    pub lineage_id: String,
    /// Lifecycle status — three values (row B5).
    pub record_status: RecordStatus,
    /// The 7-key conjunctive scope (row E3).
    pub scope: RecordScope,
    /// Who the record is shared with (row E3).
    pub sharing_scope: SharingScope,
    /// Sensitivity class. Open vocabulary; recommended `public`, `internal`,
    /// `confidential`, `restricted`. Absent ⇒ provider default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensitivity: Option<String>,
    /// When the provider observed/produced this record (protocol timestamp).
    pub observed_at: String,
    /// When the record's assertion became true in the world. Absent ⇒ unbounded
    /// into the past.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<String>,
    /// Producer confidence in `[0, 1]` when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    /// Coarse origin class, keyed by the origin→derivation matrix (row C5).
    pub origin: OriginClass,
    /// Bare references to supporting evidence records/frames.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_links: Vec<String>,
    /// Typed links to other records.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub record_links: Vec<RecordLink>,
    /// `sha256:<hex>` over the JCS canonicalization of this record with
    /// `record_hash` omitted from the preimage (row C5, profile §hashing).
    pub record_hash: String,
    /// Structured provenance (row C5).
    pub provenance: RecordProvenance,
    /// Namespaced extension members (SPEC.md §13 U3). The reference type models
    /// the common string-valued case; the wire schema permits an open object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<BTreeMap<String, String>>,
    /// The flat, `record_kind`-discriminated body.
    #[serde(flatten)]
    pub body: RecordBody,
}

impl ContextRecord {
    /// The record's `record_kind` discriminant.
    pub fn record_kind(&self) -> &'static str {
        self.body.record_kind()
    }

    /// Whether every temporal field present is in the protocol timestamp
    /// profile (`SPEC.md` §6.1/F4).
    pub fn has_valid_temporal_fields(&self) -> bool {
        [self.observed_at.as_str()]
            .into_iter()
            .chain(self.valid_from.as_deref())
            .all(is_protocol_timestamp)
    }

    /// Checks the profile's envelope invariants, returning the exact violation
    /// so a conformance failure is actionable. Mirrors
    /// [`ContextFrame::representation_invariants`](crate::ContextFrame::representation_invariants).
    pub fn envelope_invariants(&self) -> Result<(), String> {
        if self.schema_version != LIFECYCLE_SCHEMA_VERSION {
            return Err(format!(
                "schema_version must be {LIFECYCLE_SCHEMA_VERSION}, found {}",
                self.schema_version
            ));
        }
        if !is_well_formed_digest(&self.record_hash) {
            return Err(format!(
                "record_hash must be a sha256:<64 lowercase hex> digest, found {}",
                self.record_hash
            ));
        }
        if let Some(confidence) = self.confidence
            && !(0.0..=1.0).contains(&confidence)
        {
            return Err(format!("confidence must be in [0, 1], found {confidence}"));
        }
        if !self.has_valid_temporal_fields() {
            return Err("observed_at/valid_from must be protocol timestamps".into());
        }
        // Origin→derivation validity matrix (row C5).
        match self.origin {
            OriginClass::Observed | OriginClass::Declared => {
                if self.provenance.derivation_kind.is_some() {
                    return Err(format!(
                        "origin {:?} must not carry a provenance.derivation_kind",
                        self.origin
                    ));
                }
            }
            OriginClass::Derived => {
                if self.provenance.derivation_kind.is_none() {
                    return Err("origin derived requires a provenance.derivation_kind".into());
                }
            }
            OriginClass::Imported => {}
        }
        // A constraint directive must state its effect (row B3).
        if let RecordBody::Directive {
            directive_kind: DirectiveKind::Constraint,
            constraint_effect,
            ..
        } = &self.body
            && constraint_effect.is_none()
        {
            return Err("a constraint directive requires constraint_effect".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal well-formed envelope carrying the given body, for round-trip
    /// tests. Values are chosen so `envelope_invariants` passes.
    fn record_with(body: RecordBody) -> ContextRecord {
        ContextRecord {
            schema_version: LIFECYCLE_SCHEMA_VERSION.to_string(),
            record_id: "rec_0001".into(),
            lineage_id: "lin_0001".into(),
            record_status: RecordStatus::Active,
            scope: RecordScope {
                repository_id: Some("repo_42".into()),
                workspace_id: Some("ws_7".into()),
                ..RecordScope::default()
            },
            sharing_scope: SharingScope::Repository,
            sensitivity: Some("internal".into()),
            observed_at: "2026-07-29T00:00:00Z".into(),
            valid_from: None,
            confidence: Some(0.9),
            origin: OriginClass::Observed,
            evidence_links: Vec::new(),
            record_links: Vec::new(),
            record_hash: format!("sha256:{}", "a".repeat(64)),
            provenance: RecordProvenance {
                origin_provider_id: "provider_example".into(),
                origin_authority_id: Some("authority_1".into()),
                producer_kind: "agent".into(),
                producer_ref: Some("agent://coder".into()),
                derivation_kind: None,
                source_refs: Vec::new(),
            },
            extensions: None,
            body,
        }
    }

    fn all_bodies() -> Vec<RecordBody> {
        vec![
            RecordBody::Observation {
                statement: "the build failed on a flaky test".into(),
                subject_ref: Some("run_991".into()),
            },
            RecordBody::Knowledge {
                knowledge_kind: KnowledgeKind::Fact,
                statement: "the retry ceiling is 5".into(),
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
                procedure_steps: Vec::new(),
            },
            RecordBody::RecordProposal {
                proposed_kind: "directive".into(),
                rationale: "recurred across three sessions".into(),
            },
            RecordBody::Evidence {
                statement: "log line at 12:04 shows the timeout".into(),
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
                    detail: None,
                }],
            },
            RecordBody::OutcomeAssessment {
                subject_ref: "rec_task_1".into(),
                assessment: "resolved the issue".into(),
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
                feedback: "was not helpful".into(),
                rating: Some(0.2),
            },
        ]
    }

    #[test]
    fn every_record_kind_round_trips_through_json_and_stays_flat() {
        for body in all_bodies() {
            let kind = body.record_kind().to_string();
            let record = record_with(body);
            let json = serde_json::to_string(&record).unwrap();

            // The discriminant is flat: `record_kind` sits beside envelope
            // fields, not nested under `body`.
            assert!(
                json.contains(&format!("\"record_kind\":\"{kind}\"")),
                "record_kind must be a flat member for {kind}: {json}"
            );
            assert!(
                !json.contains("\"body\""),
                "the body must flatten, not nest under `body`: {json}"
            );

            let back: ContextRecord = serde_json::from_str(&json).unwrap();
            assert_eq!(back, record, "{kind} did not survive a serde round-trip");
            assert_eq!(back.record_kind(), kind);
            back.envelope_invariants()
                .unwrap_or_else(|e| panic!("{kind} envelope invalid: {e}"));
        }
    }

    #[test]
    fn optional_envelope_fields_are_omitted_when_absent() {
        let mut record = record_with(RecordBody::Observation {
            statement: "x".into(),
            subject_ref: None,
        });
        record.sensitivity = None;
        record.valid_from = None;
        record.confidence = None;
        record.evidence_links.clear();
        record.record_links.clear();
        record.extensions = None;
        let json = serde_json::to_string(&record).unwrap();
        for absent in [
            "sensitivity",
            "valid_from",
            "confidence",
            "evidence_links",
            "record_links",
            "extensions",
            "subject_ref",
        ] {
            assert!(!json.contains(absent), "{absent} should be omitted: {json}");
        }
    }

    #[test]
    fn a_constraint_directive_without_an_effect_is_rejected() {
        let record = record_with(RecordBody::Directive {
            directive_kind: DirectiveKind::Constraint,
            statement: "…".into(),
            constraint_effect: None,
            enforcement: None,
            procedure_steps: Vec::new(),
        });
        assert!(record.envelope_invariants().is_err());
    }

    #[test]
    fn a_derived_record_must_name_its_derivation_and_an_observed_one_must_not() {
        // derived without derivation_kind → invalid.
        let mut derived = record_with(RecordBody::Knowledge {
            knowledge_kind: KnowledgeKind::Decision,
            statement: "chose retry ceiling 5".into(),
        });
        derived.origin = OriginClass::Derived;
        assert!(derived.envelope_invariants().is_err());
        derived.provenance.derivation_kind = Some("inference".into());
        assert!(derived.envelope_invariants().is_ok());

        // observed WITH derivation_kind → invalid.
        let mut observed = record_with(RecordBody::Observation {
            statement: "x".into(),
            subject_ref: None,
        });
        observed.provenance.derivation_kind = Some("summarization".into());
        assert!(observed.envelope_invariants().is_err());
    }

    #[test]
    fn a_bad_schema_version_or_hash_is_rejected() {
        let mut record = record_with(RecordBody::Observation {
            statement: "x".into(),
            subject_ref: None,
        });
        record.schema_version = "contextgraph/1.0".into();
        assert!(record.envelope_invariants().is_err());

        let mut record = record_with(RecordBody::Observation {
            statement: "x".into(),
            subject_ref: None,
        });
        record.record_hash = "sha256:abc".into();
        assert!(record.envelope_invariants().is_err());
    }

    #[test]
    fn an_attestation_round_trips_and_is_not_part_of_the_record() {
        let attestation = RecordAttestation {
            signed_record_hash: format!("sha256:{}", "a".repeat(64)),
            key_id: "key_2026".into(),
            algorithm: "ed25519".into(),
            attester_id: "provider_example".into(),
            signature: "MEUCIQ…".into(),
            issued_at: "2026-07-29T00:00:00Z".into(),
        };
        let json = serde_json::to_string(&attestation).unwrap();
        let back: RecordAttestation = serde_json::from_str(&json).unwrap();
        assert_eq!(back, attestation);

        // The record type has no attestation field — it is detached.
        let record_json = serde_json::to_string(&record_with(RecordBody::Observation {
            statement: "x".into(),
            subject_ref: None,
        }))
        .unwrap();
        assert!(!record_json.contains("signature"));
    }
}
