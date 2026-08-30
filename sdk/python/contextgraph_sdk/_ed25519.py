"""A self-contained strict Ed25519 **verifier** (RFC 8032, Ed25519ph excluded).

Why this exists
---------------

``SPEC.md`` §6.5.4 says verification is offline and pure: a commitment, an
attestation and a public key are sufficient. Python's standard library ships
SHA-256 and SHA-512 but no Ed25519, and this SDK's whole pitch is that a
provider can be written against it with nothing else installed. The three
alternatives were each worse:

- a hard dependency on ``cryptography`` breaks the zero-dependency promise for
  every user, including the ones who never verify an attestation;
- an optional dependency makes ``verify`` return a different answer depending
  on what happens to be installed, which for a *verifier* is the one behaviour
  that cannot be tolerated;
- no verifier at all leaves the Python SDK unable to do the half of §6.5 that
  turns a trace into evidence.

**This module verifies and never signs.** Signing is where key material,
nonce generation and side channels live, and none of that is present here.
Producing an attestation from Python means handing the 32 commitment bytes to
a real signing backend — an HSM, a KMS, ``cryptography``, ``PyNaCl`` — exactly
as the Rust reference's own doc comment recommends for a provider holding keys
anywhere but in memory. A verifier is a public-input pure function whose only
failure mode is answering wrongly, and ``tests/test_attest.py`` checks that
against RFC 8032's own vectors, against the repository's published signature
vector produced by ``ed25519-dalek``, and — when ``cryptography`` happens to be
installed — differentially against it on every case.

Strictness
----------

§6.5.4 says a verifier **SHOULD** reject small-order public keys and
non-canonical encodings, because a signature two conforming verifiers can
disagree about is not evidence. This module matches ``ed25519_dalek``'s
``verify_strict``:

- the cofactorless equation ``[S]B = R + [k]A``, never the cofactored one;
- ``S`` rejected unless it is reduced mod the group order ``L``;
- ``A`` and ``R`` rejected if non-canonically encoded (``y >= p``);
- ``A`` and ``R`` rejected if of small order (``8P`` is the identity).

Structure follows RFC 8032 §6's own reference: extended homogeneous
coordinates ``(X : Y : Z : T)`` with ``x = X/Z``, ``y = Y/Z``, ``xy = T/Z``, so
a scalar multiplication needs no modular inversion.
"""

from __future__ import annotations

import hashlib
from typing import Optional, Tuple

__all__ = ["is_usable_public_key", "verify"]

#: The field prime, ``2^255 - 19``.
P = 2**255 - 19
#: The prime order of the base point's subgroup.
L = 2**252 + 27742317777372353535851937790883648493
#: The curve constant ``d = -121665/121666``.
D = -121665 * pow(121666, P - 2, P) % P
#: A square root of -1 mod ``P``, used to fix up the candidate ``x``.
SQRT_M1 = pow(2, (P - 1) // 4, P)

Point = Tuple[int, int, int, int]

#: The neutral element ``(0 : 1 : 1 : 0)``.
IDENTITY: Point = (0, 1, 1, 0)


def _add(p: Point, q: Point) -> Point:
    """The unified twisted-Edwards addition for ``a = -1``.

    Unified means it is also correct for ``p == q``, which is what lets
    :func:`_mul` double with the same routine.
    """
    a = (p[1] - p[0]) * (q[1] - q[0]) % P
    b = (p[1] + p[0]) * (q[1] + q[0]) % P
    c = 2 * p[3] * q[3] * D % P
    d = 2 * p[2] * q[2] % P
    e, f, g, h = b - a, d - c, d + c, b + a
    return (e * f % P, g * h % P, f * g % P, e * h % P)


def _mul(scalar: int, point: Point) -> Point:
    """``[scalar]point``, double-and-add over the bits of ``scalar``.

    Not constant-time, and it does not need to be: every input here is a
    public value — a public key, a signature component, a message hash.
    """
    acc = IDENTITY
    while scalar > 0:
        if scalar & 1:
            acc = _add(acc, point)
        point = _add(point, point)
        scalar >>= 1
    return acc


def _equal(p: Point, q: Point) -> bool:
    """Projective equality: ``X1*Z2 == X2*Z1`` and ``Y1*Z2 == Y2*Z1``."""
    return (p[0] * q[2] - q[0] * p[2]) % P == 0 and (
        p[1] * q[2] - q[1] * p[2]
    ) % P == 0


def _recover_x(y: int, sign: int) -> Optional[int]:
    """The ``x`` matching ``y`` with the requested low bit, or ``None``.

    ``None`` covers three distinct refusals, all of which mean the same thing
    to a caller: this is not a point. ``y >= P`` is a non-canonical encoding;
    a ``x^2`` with no square root is off the curve; and ``x == 0`` with the
    sign bit set is the one encoding of the identity that is not canonical.
    """
    if y >= P:
        return None
    x2 = (y * y - 1) * pow(D * y * y + 1, P - 2, P) % P
    if x2 == 0:
        return None if sign else 0
    x = pow(x2, (P + 3) // 8, P)
    if (x * x - x2) % P != 0:
        x = x * SQRT_M1 % P
    if (x * x - x2) % P != 0:
        return None
    if (x & 1) != sign:
        x = P - x
    return x


def _decompress(data: bytes) -> Optional[Point]:
    """Decode a 32-byte compressed point. ``None`` if it encodes no point."""
    if len(data) != 32:
        return None
    value = int.from_bytes(data, "little")
    sign = value >> 255
    y = value & ((1 << 255) - 1)
    x = _recover_x(y, sign)
    if x is None:
        return None
    return (x, y, 1, x * y % P)


def _is_small_order(point: Point) -> bool:
    """Whether ``8 * point`` is the identity — the cofactor test.

    Computed rather than looked up in a table, so there is no list to get
    wrong. The published table in ``tests/vectors/attestation-vectors.json``
    (which the TypeScript and Go ports do use, having no field arithmetic of
    their own) is checked against this function by ``tests/test_attest.py``.
    """
    return _equal(_mul(8, point), IDENTITY)


def _base_point() -> Point:
    y = 4 * pow(5, P - 2, P) % P
    x = _recover_x(y, 0)
    assert x is not None, "the RFC 8032 base point is on the curve"
    return (x, y, 1, x * y % P)


B = _base_point()


def is_usable_public_key(key: bytes) -> bool:
    """Whether a strict verifier will accept this as a verification key.

    Rejects a wrong length, a non-canonical encoding, and a small-order point.
    The last one matters: a signature under a small-order key verifies against
    arbitrary messages, so accepting one turns "this attestation is valid" into
    a statement about nothing.
    """
    point = _decompress(key)
    return point is not None and not _is_small_order(point)


def verify(public_key: bytes, message: bytes, signature: bytes) -> bool:
    """Check a detached Ed25519 signature, strictly.

    Returns ``False`` for every refusal rather than raising, because a caller
    in :mod:`contextgraph_sdk.attest` has already separated "malformed" from
    "does not verify" and needs one bit here.
    """
    if len(signature) != 64:
        return False
    a = _decompress(public_key)
    if a is None or _is_small_order(a):
        return False
    r_bytes = signature[:32]
    r = _decompress(r_bytes)
    if r is None or _is_small_order(r):
        return False
    s = int.from_bytes(signature[32:], "little")
    if s >= L:
        # A non-reduced S is the malleability RFC 8032 §8.4 warns about: two
        # distinct signature encodings a lax verifier accepts for one message.
        return False
    k = (
        int.from_bytes(
            hashlib.sha512(r_bytes + public_key + message).digest(), "little"
        )
        % L
    )
    # Cofactorless: [S]B == R + [k]A. Multiplying both sides by the cofactor
    # would accept signatures dalek's verify_strict rejects, and the point of
    # matching it is that two verifiers never disagree.
    return _equal(_mul(s, B), _add(r, _mul(k, a)))
