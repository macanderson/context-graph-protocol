"""The Ed25519 **signing** path — optional, and delegated to ``cryptography``.

Why this is a separate module from :mod:`contextgraph_sdk._ed25519`
-------------------------------------------------------------------

The in-package verifier is defensible precisely because it touches nothing
secret: every input is public, so its arithmetic need not be constant-time and
its only failure mode is answering wrongly, which the published vectors catch.
Signing is the opposite. Ed25519 signing is deterministic and needs no nonce
source, but it hashes and multiplies by the *secret* scalar, and a hand-written
big-integer implementation in Python leaks that scalar through timing to anyone
who can measure it. So this SDK does not sign in pure Python, and never will.

Signing therefore goes through the ``cryptography`` package — OpenSSL's
constant-time Ed25519 — installed as an **optional** extra::

    pip install "contextgraph-sdk[signing]"

The zero-dependency promise holds for everyone who does not sign: nothing here
is imported until a signing function is called, and verification never comes
through this module.

A missing backend is an error, never a different answer
-------------------------------------------------------

When ``cryptography`` is absent (or its OpenSSL build lacks Ed25519), every
entry point raises :class:`SigningUnavailableError`, naming the extra to
install. There is no fallback: a signer that quietly switched implementations
depending on what happened to be installed is exactly the behaviour
:mod:`contextgraph_sdk._ed25519`'s header rules out for the verifier, and the
argument is stronger for the half that holds a secret.

Key custody
-----------

The functions here take a raw 32-byte seed because that is the one shape every
backend can produce and every published vector uses. Holding a long-lived
signing seed in application memory is a choice with a cost — it is readable by
anything that can read the process (a core dump, a debugger, a heap-disclosure
bug, a compromised dependency), and ``bytes`` objects cannot be wiped. A
provider whose key lives in an HSM or KMS should not call these at all: it
computes :func:`contextgraph_sdk.attest.frame_commitment` or
:func:`contextgraph_sdk.record.record_attestation_message`, has the backend sign
those bytes, and assembles the attestation itself. The README's "Key custody"
section says the same thing at more length.

``importlib`` rather than an ``import`` statement so the package typechecks
under ``mypy --strict`` whether or not ``cryptography`` is installed; the
backend's values are ``Any`` at this boundary and are narrowed to ``bytes``
before they leave it.
"""

from __future__ import annotations

import importlib
from typing import Any

__all__ = ["SEED_LENGTH", "SigningUnavailableError", "public_key", "sign"]

#: An Ed25519 signing seed is exactly 32 bytes (RFC 8032 §5.1.5).
SEED_LENGTH = 32

#: The extra that installs the signing backend, named once so every message
#: agrees with ``pyproject.toml``.
_EXTRA_HINT = 'pip install "contextgraph-sdk[signing]"'


class SigningUnavailableError(RuntimeError):
    """Signing was requested but no Ed25519 signing backend is available.

    Raised instead of falling back to anything: the SDK carries a verifier but
    deliberately no signer, so the only honest answer without the optional
    ``cryptography`` backend is "cannot sign here". Verification is unaffected
    and never raises this.
    """


def _check_seed(seed: bytes) -> bytes:
    """Refuse anything but 32 raw bytes, before any backend is consulted.

    Checked first so that a malformed seed is a :class:`ValueError` on every
    machine, rather than a :class:`SigningUnavailableError` on some and a
    :class:`ValueError` on others.
    """
    if not isinstance(seed, (bytes, bytearray)):
        raise TypeError(
            f"an Ed25519 signing seed must be bytes, not {type(seed).__name__}"
        )
    if len(seed) != SEED_LENGTH:
        raise ValueError(
            f"an Ed25519 signing seed is {SEED_LENGTH} bytes, got {len(seed)}"
        )
    return bytes(seed)


def _private_key(seed: bytes) -> Any:
    """Load the backend and build its private-key object from a seed."""
    try:
        backend = importlib.import_module(
            "cryptography.hazmat.primitives.asymmetric.ed25519"
        )
    except ImportError as error:
        raise SigningUnavailableError(
            "Ed25519 signing needs the optional 'cryptography' package, which "
            f"is not installed: {_EXTRA_HINT}. This SDK verifies without it but "
            "never signs in pure Python (see contextgraph_sdk._signing)."
        ) from error
    try:
        return backend.Ed25519PrivateKey.from_private_bytes(seed)
    except Exception as error:  # cryptography's UnsupportedAlgorithm, by name
        # An OpenSSL build without Ed25519 raises UnsupportedAlgorithm here.
        # The seed was already checked, so any failure is the backend's.
        raise SigningUnavailableError(
            "the installed 'cryptography' backend cannot sign Ed25519 "
            f"({type(error).__name__}: {error}); upgrade it: {_EXTRA_HINT}"
        ) from error


def sign(seed: bytes, message: bytes) -> bytes:
    """A detached Ed25519 signature (64 raw bytes) over ``message``.

    Deterministic (RFC 8032 §5.1.6): one seed and one message always produce
    the same signature, which is what lets a test pin this against a published
    vector byte for byte.
    """
    key = _private_key(_check_seed(seed))
    signature = bytes(key.sign(bytes(message)))
    if len(signature) != 64:  # pragma: no cover - a backend contract breach
        raise SigningUnavailableError(
            f"the signing backend returned {len(signature)} bytes, not 64"
        )
    return signature


def public_key(seed: bytes) -> bytes:
    """The raw 32-byte Ed25519 public key a seed signs under.

    Derived by the backend, not by :mod:`contextgraph_sdk._ed25519`: deriving a
    public key multiplies by the secret scalar, which is signing's risk, not
    verification's.
    """
    key = _private_key(_check_seed(seed))
    serialization = importlib.import_module(
        "cryptography.hazmat.primitives.serialization"
    )
    raw = bytes(
        key.public_key().public_bytes(
            encoding=serialization.Encoding.Raw,
            format=serialization.PublicFormat.Raw,
        )
    )
    return raw
