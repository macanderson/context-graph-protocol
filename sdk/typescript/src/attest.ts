/**
 * Provenance attestation — the `SPEC.md` §6.5 constructions, in TypeScript.
 *
 * This is a port of `contextgraph_types::attest`, and the Rust crate is the
 * reference: the vectors in `tests/vectors/attestation-vectors.json` come from
 * it, and `test/attest.test.ts` reconciles every function here against them.
 *
 * # The trap this file exists to avoid
 *
 * §6.5.1 length-prefixes each field with the **UTF-8 byte length** of its
 * value. `String.prototype.length` is a count of UTF-16 code units, which is a
 * different number for every string outside Latin-1 and off by an extra one
 * per astral-plane character, where JavaScript stores a surrogate pair. A port
 * that reaches for `.length` produces a self-consistent chain head that no
 * other implementation agrees with, and an ASCII-only test suite never notices.
 * Nothing in this file measures a string; {@link encodeString} measures the
 * bytes `TextEncoder` produced.
 *
 * # No JSON canonicalizer
 *
 * The encoding was chosen over RFC 8785 (JCS) precisely so this port needs
 * none (ADR 0010). A provenance link is six optional strings; if you find
 * yourself serializing one to JSON here, re-read §6.5.1.
 */

import { createHash, createPublicKey, verify as cryptoVerify } from "node:crypto";

import type { Provenance } from "./types.js";

/** The signature algorithm this revision defines (`SPEC.md` §6.5). */
export const ALGORITHM_ED25519 = "ed25519";

/**
 * The domain-separation tags and Merkle prefixes the hashing rules use
 * (`SPEC.md` §6.5.1). These exact byte strings are normative — a port that
 * spells one differently computes different commitments and interoperates
 * with nothing.
 */
const DOMAIN = {
  genesis: "contextgraph/attest/1/genesis",
  link: "contextgraph/attest/1/link",
  frame: "contextgraph/attest/1/frame",
  merkleEmpty: "contextgraph/attest/1/merkle-empty",
} as const;

/** RFC 6962 leaf prefix, distinct from the node prefix so a subtree can never be presented as a single frame. */
const MERKLE_LEAF = Uint8Array.of(0x00);
/** RFC 6962 interior-node prefix. */
const MERKLE_NODE = Uint8Array.of(0x01);

const UTF8 = new TextEncoder();

/** A detached attestation binding one frame's provenance to a signing identity (`SPEC.md` §6.5). */
export interface ProvenanceAttestation {
  /** The `sha256:<hex>` commitment this attestation signs. */
  signed_commitment: string;
  /** The signing key's id. Rotation issues a new id; it never reuses one. */
  key_id: string;
  /** The signature scheme, e.g. {@link ALGORITHM_ED25519}. */
  algorithm: string;
  /** The attesting authority — who is accountable, as distinct from which key signed. */
  attester_id: string;
  /** The detached signature, lowercase hex. */
  signature: string;
  /** When the attestation was issued (a `SPEC.md` §F4 protocol timestamp). */
  issued_at: string;
}

/**
 * The part of a frame a commitment covers.
 *
 * Structural rather than `ContextFrame`, because only these three fields enter
 * the preimage — a caller holding a frame from elsewhere should not have to
 * fabricate a `score` and a `token_cost` to compute a commitment.
 */
export interface AttestableFrame {
  id: string;
  content_digest?: string;
  provenance?: readonly Provenance[];
}

/** One step of an {@link InclusionProof}: the sibling hash and which side it sits on. */
export interface InclusionStep {
  /** The sibling subtree hash, `sha256:<hex>`. */
  sibling: string;
  /** Whether the sibling is the **left** operand at this level. */
  sibling_is_left: boolean;
}

/** A proof that one frame commitment is a leaf of a signed {@link merkleRoot} (`SPEC.md` §6.5.3). */
export interface InclusionProof {
  /** The leaf's index in canonical order. */
  leaf_index: number;
  /** How many leaves the tree held — a root alone does not pin the tree's size. */
  leaf_count: number;
  /** Sibling hashes from the leaf upward. */
  path: InclusionStep[];
}

/**
 * The outcome of checking a {@link ProvenanceAttestation} (`SPEC.md` §6.5.4).
 *
 * A tagged union rather than a boolean, because §6.5.4 requires a verifier to
 * distinguish these: "the frame changed after signing" and "the key is wrong"
 * send an operator in opposite directions, and F8 treats "I cannot check this"
 * as a third answer again.
 */
export type AttestationVerdict =
  | { readonly verdict: "valid" }
  | { readonly verdict: "commitment_mismatch"; readonly expected: string; readonly signed: string }
  | { readonly verdict: "bad_signature" }
  | { readonly verdict: "unknown_algorithm"; readonly algorithm: string }
  | { readonly verdict: "malformed_key" }
  | { readonly verdict: "malformed_signature" }
  | { readonly verdict: "malformed_commitment" };

/**
 * Whether a verdict is `valid`.
 *
 * No other verdict is provisionally acceptable: the point of an attestation is
 * that "I could not check it" and "it is good" are never the same answer.
 */
export function isValid(verdict: AttestationVerdict): boolean {
  return verdict.verdict === "valid";
}

// ---------------------------------------------------------------------------
// Canonical encoding (`SPEC.md` §6.5.1)
// ---------------------------------------------------------------------------

/** The largest unsigned value a four-byte prefix can carry. */
const MAX_PREFIX = 0xffff_ffff;

function concatBytes(parts: readonly Uint8Array[]): Uint8Array {
  let total = 0;
  for (const part of parts) total += part.length;
  const out = new Uint8Array(total);
  let at = 0;
  for (const part of parts) {
    out.set(part, at);
    at += part.length;
  }
  return out;
}

/**
 * `uint32be(utf8_byte_length(s)) || utf8(s)`.
 *
 * The length is taken from the encoded bytes, never from `s.length`. Unsigned
 * and big-endian, both normative.
 */
function encodeString(s: string): Uint8Array {
  const bytes = UTF8.encode(s);
  if (bytes.length > MAX_PREFIX) {
    throw new RangeError(
      `a provenance field of ${bytes.length} bytes overflows the §6.5.1 uint32 length prefix`,
    );
  }
  const prefix = new Uint8Array(4);
  // `false` is the big-endian argument. Writing it out rather than relying on
  // the default, because the default is little-endian and the spec is not.
  new DataView(prefix.buffer).setUint32(0, bytes.length, false);
  return concatBytes([prefix, bytes]);
}

/**
 * `0x00` for absent, `0x01 || encodeString(s)` for present.
 *
 * The presence byte keeps absent distinct from empty. Without it `uri: null`
 * and `uri: ""` encode identically and a URI could be deleted from a signed
 * chain without disturbing the hash — so `undefined` and `null` are absent,
 * and `""` is present.
 */
function encodeOptional(s: string | null | undefined): Uint8Array {
  if (s === undefined || s === null) return Uint8Array.of(0x00);
  return concatBytes([Uint8Array.of(0x01), encodeString(s)]);
}

/**
 * The canonical encoding of one provenance link (`SPEC.md` §6.5.1).
 *
 * Field order is normative: `type`, `uri`, `range`, `digest`, `method`, `by`.
 */
export function encodeProvenanceLink(link: Provenance): Uint8Array {
  return concatBytes([
    encodeString(link.type),
    encodeOptional(link.uri),
    encodeOptional(link.range),
    encodeOptional(link.digest),
    encodeOptional(link.method),
    encodeOptional(link.by),
  ]);
}

function sha256(parts: readonly (Uint8Array | string)[]): Uint8Array {
  const hash = createHash("sha256");
  for (const part of parts) {
    hash.update(typeof part === "string" ? UTF8.encode(part) : part);
  }
  return new Uint8Array(hash.digest());
}

/** Render 32 raw bytes as this protocol's `sha256:<hex>` digest string. */
export function digestString(bytes: Uint8Array): string {
  return `sha256:${toHex(bytes)}`;
}

/** Lowercase hex, two characters per byte. */
export function toHex(bytes: Uint8Array): string {
  let out = "";
  for (const b of bytes) out += b.toString(16).padStart(2, "0");
  return out;
}

/** Parse lowercase hex. `null` on any non-hex character or an odd length. */
export function fromHex(hex: string): Uint8Array | null {
  if (hex.length % 2 !== 0 || !/^[0-9a-f]*$/.test(hex)) return null;
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i += 1) {
    out[i] = Number.parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}

/** Parse a `sha256:<hex>` digest string into its 32 raw bytes. `null` if malformed. */
export function parseDigest(digest: string): Uint8Array | null {
  if (!digest.startsWith("sha256:")) return null;
  const bytes = fromHex(digest.slice("sha256:".length));
  return bytes !== null && bytes.length === 32 ? bytes : null;
}

// ---------------------------------------------------------------------------
// Chain head, frame commitment, Merkle tree (`SPEC.md` §6.5.2–§6.5.3)
// ---------------------------------------------------------------------------

/**
 * The head of a frame's provenance hash chain (`SPEC.md` §6.5.2).
 *
 * Links fold **source-first**, in the order §6 requires them to be carried, so
 * each step consumes the previous head and no link can be inserted, dropped,
 * reordered or edited without changing the result. An empty chain hashes to
 * the genesis value rather than to zero, so "no provenance" is a stated claim
 * a signature can cover.
 */
export function provenanceChainHead(links: readonly Provenance[] = []): Uint8Array {
  let head = sha256([DOMAIN.genesis]);
  for (const link of links) {
    head = sha256([DOMAIN.link, head, encodeProvenanceLink(link)]);
  }
  return head;
}

/**
 * The commitment binding one frame's identity to its provenance chain
 * (`SPEC.md` §6.5.2) — the preimage a single-frame attestation signs.
 *
 * The `(provider_id, frame id, content_digest)` triple is not optional: two
 * frames citing the same source share a chain head, so a signature over the
 * head alone lifts from one frame onto another.
 */
export function frameCommitment(providerId: string, frame: AttestableFrame): Uint8Array {
  const chainHead = provenanceChainHead(frame.provenance ?? []);
  const preimage = concatBytes([
    encodeString(providerId),
    encodeString(frame.id),
    encodeOptional(frame.content_digest),
  ]);
  return sha256([DOMAIN.frame, preimage, chainHead]);
}

function leafHash(commitment: Uint8Array): Uint8Array {
  return sha256([MERKLE_LEAF, commitment]);
}

function nodeHash(left: Uint8Array, right: Uint8Array): Uint8Array {
  return sha256([MERKLE_NODE, left, right]);
}

/** The largest power of two strictly less than `n` (RFC 6962's split point). Only meaningful for `n >= 2`. */
function splitPoint(n: number): number {
  let k = 1;
  while (k * 2 < n) k *= 2;
  return k;
}

/**
 * The Merkle root over a set of frame commitments (`SPEC.md` §6.5.3).
 *
 * RFC 6962's shape, not the "duplicate the last leaf on an odd level"
 * shortcut, which admits two distinct leaf sets with the same root. The two
 * agree on any power-of-two leaf count, which is why the published vectors
 * include three and seven.
 */
export function merkleRoot(commitments: readonly Uint8Array[]): Uint8Array {
  if (commitments.length === 0) return sha256([DOMAIN.merkleEmpty]);
  if (commitments.length === 1) return leafHash(commitments[0]!);
  const k = splitPoint(commitments.length);
  return nodeHash(merkleRoot(commitments.slice(0, k)), merkleRoot(commitments.slice(k)));
}

/** Build an {@link InclusionProof} for `leafIndex`. `null` if the index is out of range. */
export function inclusionProof(
  commitments: readonly Uint8Array[],
  leafIndex: number,
): InclusionProof | null {
  if (!Number.isInteger(leafIndex) || leafIndex < 0 || leafIndex >= commitments.length) {
    return null;
  }
  const path: InclusionStep[] = [];
  collectPath(commitments, leafIndex, path);
  return { leaf_index: leafIndex, leaf_count: commitments.length, path };
}

/** Walk down the tree accumulating sibling hashes, leaf-upward. */
function collectPath(
  commitments: readonly Uint8Array[],
  index: number,
  path: InclusionStep[],
): void {
  if (commitments.length <= 1) return;
  const k = splitPoint(commitments.length);
  if (index < k) {
    collectPath(commitments.slice(0, k), index, path);
    path.push({ sibling: digestString(merkleRoot(commitments.slice(k))), sibling_is_left: false });
  } else {
    collectPath(commitments.slice(k), index - k, path);
    path.push({ sibling: digestString(merkleRoot(commitments.slice(0, k))), sibling_is_left: true });
  }
}

/**
 * Recompute a Merkle root from a leaf commitment and its proof.
 *
 * The whole offline story: an auditor holding one frame, its proof and a
 * signed root needs nothing else. `null` if any sibling in the path is
 * malformed, or the index does not sit inside the stated leaf count.
 */
export function rootFromProof(
  commitment: Uint8Array,
  proof: InclusionProof,
): Uint8Array | null {
  if (proof.leaf_index >= proof.leaf_count) return null;
  let acc = leafHash(commitment);
  for (const step of proof.path) {
    const sibling = parseDigest(step.sibling);
    if (sibling === null) return null;
    acc = step.sibling_is_left ? nodeHash(sibling, acc) : nodeHash(acc, sibling);
  }
  return acc;
}

// ---------------------------------------------------------------------------
// Verification (`SPEC.md` §6.5.4)
// ---------------------------------------------------------------------------

/**
 * The eight canonical encodings of a point `P` with `8P = identity`.
 *
 * §6.5.4 asks for a verifier that rejects small-order public keys, and Node's
 * Ed25519 (OpenSSL) does not — it checks a canonical `S` and stops there. A
 * signature under a small-order key verifies against arbitrary messages, so
 * accepting one turns "this attestation is valid" into a statement about
 * nothing. Published in `tests/vectors/attestation-vectors.json` under
 * `verifier_strictness`, where the Python suite recomputes `8P = identity` for
 * every entry from its own field arithmetic rather than trusting the list.
 */
const SMALL_ORDER_KEYS: readonly string[] = [
  "0000000000000000000000000000000000000000000000000000000000000000",
  "0000000000000000000000000000000000000000000000000000000000000080",
  "0100000000000000000000000000000000000000000000000000000000000000",
  "26e8958fc2b227b045c3f489f2ef98f0d5dfac05d3c63339b13802886d53fc05",
  "26e8958fc2b227b045c3f489f2ef98f0d5dfac05d3c63339b13802886d53fc85",
  "c7176a703d4dd84fba3c0b760d10670f2a2053fa2c39ccc64ec7fd7792ac037a",
  "c7176a703d4dd84fba3c0b760d10670f2a2053fa2c39ccc64ec7fd7792ac03fa",
  "ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
];

/** `2^255 - 19`, the field prime — a key whose `y` reaches it is not canonically encoded. */
const FIELD_PRIME = (1n << 255n) - 19n;

/** The little-endian `y` a compressed point encodes, with the sign bit masked off. */
function compressedY(key: Uint8Array): bigint {
  let y = 0n;
  for (let i = key.length - 1; i >= 0; i -= 1) {
    y = (y << 8n) | BigInt(key[i]!);
  }
  return y & ((1n << 255n) - 1n);
}

/**
 * Whether this public key is one a strict verifier declines to use.
 *
 * Two reasons, both from §6.5.4: the point has small order, or its `y` is not
 * reduced — `p` and `p + 1` encode nothing, but a verifier that reduces would
 * read them as the small-order points `y = 0` and `y = 1`.
 */
function isRejectableKey(key: Uint8Array): boolean {
  return SMALL_ORDER_KEYS.includes(toHex(key)) || compressedY(key) >= FIELD_PRIME;
}

/** The DER SubjectPublicKeyInfo header for an Ed25519 key — 12 bytes, then the raw 32. */
const ED25519_SPKI_PREFIX = Uint8Array.of(
  0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
);

/**
 * Verify a detached attestation over an already-computed commitment — a
 * {@link merkleRoot} for a result set, or a {@link frameCommitment}.
 *
 * Pure and offline: a commitment, an attestation and a public key are
 * sufficient. `publicKey` is raw 32 bytes, matching the Rust reference, so no
 * caller has to know what a `KeyObject` is.
 */
export function verifyCommitment(
  expected: Uint8Array,
  attestation: ProvenanceAttestation,
  publicKey: Uint8Array,
): AttestationVerdict {
  if (attestation.algorithm !== ALGORITHM_ED25519) {
    return { verdict: "unknown_algorithm", algorithm: attestation.algorithm };
  }
  const signed = parseDigest(attestation.signed_commitment);
  if (signed === null) return { verdict: "malformed_commitment" };

  // Compare commitments *before* touching the signature. A mismatch means the
  // frame changed after signing, and reporting that as a bad signature sends
  // an operator hunting a key-management bug when the finding is tampering.
  if (!equalBytes(signed, expected)) {
    return {
      verdict: "commitment_mismatch",
      expected: digestString(expected),
      signed: attestation.signed_commitment,
    };
  }

  if (publicKey.length !== 32 || isRejectableKey(publicKey)) {
    return { verdict: "malformed_key" };
  }
  const signature = fromHex(attestation.signature);
  if (signature === null || signature.length !== 64) {
    return { verdict: "malformed_signature" };
  }

  let key;
  try {
    key = createPublicKey({
      key: Buffer.from(concatBytes([ED25519_SPKI_PREFIX, publicKey])),
      format: "der",
      type: "spki",
    });
  } catch {
    return { verdict: "malformed_key" };
  }
  // `null` is the algorithm argument Ed25519 takes: it hashes internally, so
  // there is no digest to name.
  const ok = cryptoVerify(null, Buffer.from(signed), key, Buffer.from(signature));
  return ok ? { verdict: "valid" } : { verdict: "bad_signature" };
}

/** Verify a detached attestation over a single frame (`SPEC.md` §6.5.4). */
export function verifyFrameAttestation(
  providerId: string,
  frame: AttestableFrame,
  attestation: ProvenanceAttestation,
  publicKey: Uint8Array,
): AttestationVerdict {
  return verifyCommitment(frameCommitment(providerId, frame), attestation, publicKey);
}

function equalBytes(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i += 1) {
    if (a[i] !== b[i]) return false;
  }
  return true;
}
