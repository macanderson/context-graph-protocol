"""Sign a frame and a lifecycle record, then verify both — the worked example
for ``SPEC.md`` §6.5 and the lifecycle profile's ``RecordAttestation``::

    pip install "contextgraph-sdk[signing]"
    python3 sdk/python/examples/sign_and_verify.py

Signing needs the optional ``cryptography`` backend; without it this script
says so and exits non-zero rather than pretending. Verification never needs it.

The seed below is a published test key (``tests/vectors/attestation-vectors.json``
signs with the same 32 bytes of ``0x07``). Anything it signs is forgeable by
anyone. A real provider holds its key in an HSM or KMS, signs
``frame_commitment(...)`` or ``record_attestation_message(...)`` there, and
never has a seed in memory — see the README's "Key custody" section.
"""

from __future__ import annotations

import hashlib
import os
import sys

# Allow running the example directly from the repo without installing the SDK.
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from contextgraph_sdk import (  # noqa: E402
    AttestableFrame,
    SigningUnavailableError,
    public_key_for,
    record_hash,
    sign_frame_attestation,
    sign_record,
    verify_frame_attestation,
    verify_record_attestation,
)
from contextgraph_sdk.types import Provenance  # noqa: E402

#: PUBLISHED TEST SEED — never use it for anything real.
TEST_SEED = bytes([0x07] * 32)


def main() -> int:
    content = "Retry three times with exponential backoff, then surface a 502."
    frame = AttestableFrame(
        id="retry-policy",
        content_digest="sha256:" + hashlib.sha256(content.encode("utf-8")).hexdigest(),
        provenance=(Provenance(type="file", uri="runbooks/retries.md", range="L1-12"),),
    )
    record = {
        "schema_version": "contextgraph/lifecycle/1.0-draft",
        "record_id": "rec_obs_example",
        "record_kind": "observation",
        "statement": content,
        "confidence": 0.82,
    }

    try:
        public_key = public_key_for(TEST_SEED)
        frame_attestation = sign_frame_attestation(
            "example-provider", frame, TEST_SEED, "key-1", "example", "2026-09-28T00:00:00Z"
        )
        record_attestation = sign_record(
            record, TEST_SEED, "key-1", "example", "2026-09-28T00:00:00Z"
        )
    except SigningUnavailableError as missing:
        print(f"cannot sign here: {missing}", file=sys.stderr)
        return 2

    print("frame attestation:", frame_attestation.to_wire())
    print("record_hash:", record_hash(record))
    print("record attestation:", record_attestation.to_wire())

    frame_verdict = verify_frame_attestation(
        "example-provider", frame, frame_attestation, public_key
    )
    record_verdict = verify_record_attestation(record, record_attestation, public_key)
    print("frame verdict:", frame_verdict.verdict)
    print("record verdict:", record_verdict.verdict)
    return 0 if frame_verdict.is_valid() and record_verdict.is_valid() else 1


if __name__ == "__main__":
    sys.exit(main())
