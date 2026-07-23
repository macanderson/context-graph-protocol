"""Context Graph Protocol wire types, mirrored from the JSON Schema.

These are ``TypedDict``s: at runtime a frame or envelope is a plain ``dict`` (the
wire is JSON), and these give editors and type-checkers the exact shape. Keys
typed ``Optional``/absent are simply omitted when you have no value — the SDK
never emits explicit ``null`` for them.
"""

from __future__ import annotations

from typing import Any, Literal, TypedDict

PROTOCOL_VERSION = "contextgraph/1.0-draft"

FrameKind = Literal["snippet", "symbol", "fact", "doc", "memory", "episode", "graph"]
Representation = Literal["full", "compact", "reference"]
ContentFidelity = Literal["exact", "normalized", "summarized", "omitted"]
VerdictStatus = Literal["valid", "stale", "gone", "unknown"]


class Provenance(TypedDict, total=False):
    type: str  # required; the wire name for "kind"
    uri: str
    range: str
    digest: str
    method: str
    by: str


class Relation(TypedDict, total=False):
    rel: str  # required
    target_uri: str  # required
    display_name: str


class ContentRef(TypedDict, total=False):
    provider_id: str  # required
    uri: str  # required
    expires_at: str


class ContextFrame(TypedDict, total=False):
    id: str  # required
    kind: FrameKind  # required
    title: str  # required
    content: str  # absent for reference frames
    content_digest: str
    uri: str
    representation: Representation
    content_fidelity: ContentFidelity
    canonical_content_hash: str
    content_ref: ContentRef
    transform: dict[str, str]
    minimum_content_fidelity: ContentFidelity
    inline_content_requirement: Literal["required", "resolvable_reference_allowed"]
    score: float  # required
    token_cost: int  # required; ceil(utf8_len(content)/4)
    canonical_token_cost: int
    tokenizer_ref: str
    valid_from: str
    valid_to: str
    recorded_at: str
    provenance: list[Provenance]
    citation_label: str
    embedding: dict[str, Any]
    relations: list[Relation]


class ContextQuery(TypedDict, total=False):
    goal: str  # required
    query_text: str
    embedding: list[float]
    kinds: list[FrameKind]
    anchors: list[str]
    max_frames: int  # required
    max_tokens: int  # required
    as_of: str
    representation_preferences: list[Representation]


class ContextQueryResult(TypedDict, total=False):
    frames: list[ContextFrame]  # required
    truncated: bool  # required
    dropped_estimate: int


class DataFlow(TypedDict, total=False):
    reads: bool  # required
    writes: bool  # required
    egress: bool  # required
    egress_scopes: list[str]


class ProviderInfo(TypedDict, total=False):
    name: str  # required
    version: str  # required
    data_flow: DataFlow  # required


class Capabilities(TypedDict, total=False):
    query: dict[str, Any]  # required, e.g. {"kinds": ["doc"]}
    correlation: bool
    graph: bool
    embeddings_fingerprint: str | None
    verify: bool
    representations: list[Representation]
    resolve: bool


class FrameId(TypedDict, total=False):
    provider_id: str  # required
    frame_id: str  # required
    content_digest: str


class FrameVerdict(TypedDict, total=False):
    frame: FrameId  # required
    status: VerdictStatus  # required
    replacement_digest: str


class VerifyRequest(TypedDict, total=False):
    frames: list[FrameId]  # required


class VerifyResponse(TypedDict, total=False):
    verdicts: list[FrameVerdict]  # required
