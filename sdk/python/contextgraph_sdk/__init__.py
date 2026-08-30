"""``contextgraph-sdk`` — a zero-dependency Python SDK for building conformant
Context Graph Protocol providers.

    from contextgraph_sdk import run_stdio_provider, budget_tokens

    class MyProvider:
        def info(self):
            return {"name": "my-provider", "version": "0.1.0",
                    "data_flow": {"reads": True, "writes": False, "egress": False,
                                  "egress_scopes": ["local-only"]}}
        def capabilities(self):
            return {"query": {"kinds": ["doc"]}, "correlation": True}
        def query(self, query):
            return {"frames": [], "truncated": False}

    run_stdio_provider(MyProvider())
"""

from .attest import (
    ALGORITHM_ED25519,
    AttestableFrame,
    AttestationVerdict,
    InclusionProof,
    InclusionStep,
    ProvenanceAttestation,
    Verdict,
    digest_string,
    encode_provenance_link,
    frame_commitment,
    inclusion_proof,
    merkle_root,
    parse_digest,
    provenance_chain_head,
    root_from_proof,
    verify_commitment,
    verify_frame_attestation,
)
from .budget import BYTES_PER_BUDGET_TOKEN, budget_tokens
from .http import handle_envelope, make_wsgi_app, respond_to_body
from .provider import Provider, ProviderError, run_stdio_provider
from .types import PROTOCOL_VERSION

__all__ = [
    "PROTOCOL_VERSION",
    "BYTES_PER_BUDGET_TOKEN",
    "budget_tokens",
    "Provider",
    "ProviderError",
    "run_stdio_provider",
    "handle_envelope",
    "respond_to_body",
    "make_wsgi_app",
    "ALGORITHM_ED25519",
    "AttestableFrame",
    "AttestationVerdict",
    "InclusionProof",
    "InclusionStep",
    "ProvenanceAttestation",
    "Verdict",
    "digest_string",
    "encode_provenance_link",
    "frame_commitment",
    "inclusion_proof",
    "merkle_root",
    "parse_digest",
    "provenance_chain_head",
    "root_from_proof",
    "verify_commitment",
    "verify_frame_attestation",
]
