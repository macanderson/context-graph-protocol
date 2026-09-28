/**
 * Record content addressing and record attestation — the lifecycle profile's
 * `record_hash` and `RecordAttestation`, in TypeScript
 * (`docs/profiles/context-exchange-provider.md` §3 and §7, ADR 0017).
 *
 * This is a port of `contextgraph_types::record_attest`, and the Rust crate is
 * the reference: `tests/fixtures/record-hash-vectors.json` publishes the exact
 * canonical text and hash of every record fixture, and
 * `test/record/record.test.ts` reconciles this file against every one of them,
 * byte for byte, and against the published record attestation.
 *
 * Two constructions:
 *
 * 1. **`record_hash`** ({@link recordHash}) — `sha256:<hex>` over the RFC 8785
 *    (JCS) canonicalization of a record **with its own top-level
 *    `record_hash` member removed** (profile LH1).
 * 2. **{@link RecordAttestation}** ({@link verifyRecordAttestation}) — a
 *    detached Ed25519 signature over {@link RECORD_ATTESTATION_DOMAIN}
 *    followed by that hash's 32 raw bytes (profile LC4), checked against a
 *    hash **recomputed** from the record rather than the stored member (LC5).
 *
 * # Why this port carries its own RFC 8785 canonicalizer
 *
 * JCS is defined in terms of ECMAScript: numbers serialize as
 * `Number.prototype.toString` does, strings escape as `JSON.stringify` does
 * (ES2019's well-formed variant), and members sort by UTF-16 code unit, which
 * is what `Array.prototype.sort` compares by default. JavaScript is the one
 * language where a conforming canonicalizer is a short walk over the value
 * with the runtime's own primitives, so the SDK's zero-dependency promise
 * costs nothing here. What the runtime does *not* do for free is refuse the
 * inputs RFC 8785 refuses — `NaN`, the infinities, lone surrogates — and
 * `JSON.stringify` silently turns the first two into `null` and escapes the
 * third. {@link canonicalizeJson} refuses all three rather than hash a value
 * that no other implementation would agree with.
 *
 * # Why the omitted member is removed, not blanked
 *
 * A record hashes identically whether it carries no `record_hash`, the right
 * one, or a wrong one, so a producer never invents a placeholder and a
 * verifier never has to know which placeholder was chosen. Only the
 * **top-level** member is removed: a `record_hash` nested inside `extensions`
 * or a body member is ordinary content and stays in the preimage.
 */

import { createHash } from "node:crypto";

import {
  ALGORITHM_ED25519,
  digestString,
  parseDigest,
  signEd25519,
  toHex,
  verifyEd25519,
  type AttestationVerdict,
  type SigningKey,
} from "./attest.js";

/** The envelope member a record's own hash lives in, and the one member removed from its preimage (profile LH1). */
export const RECORD_HASH_MEMBER = "record_hash";

/**
 * The domain-separation tag a {@link RecordAttestation} signs under (profile LC4).
 *
 * Normative: the signed message is these bytes followed by the 32 raw bytes of
 * `signed_record_hash`. A `record_hash` is a plain SHA-256 over a JSON
 * document, which unrelated systems also compute; signing it bare would let
 * one signature mean whatever its presenter claims. Both halves are fixed
 * length, so the message is injective without a length prefix.
 */
export const RECORD_ATTESTATION_DOMAIN = "contextgraph/attest/1/record";

/** A detached Ed25519 signature over a record's `record_hash` (profile §7, ADR 0017). */
export interface RecordAttestation {
  /** The `sha256:<hex>` record hash this attestation signs. */
  signed_record_hash: string;
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
 * Why a `record_hash` could not be computed or used. Mirrors the Rust
 * reference's `RecordHashError`, less `NotSerializable`, which has no
 * JavaScript analogue: there is no typed record to fail serializing.
 *
 * - `not_an_object` — the value is not a JSON object, so it is not a record.
 *   A caller bug.
 * - `not_canonicalizable` — the value holds something RFC 8785 refuses
 *   (`NaN`, an infinity, a lone surrogate) or something that is not JSON at
 *   all (a `bigint`, a function, a class instance). A finding about the
 *   record, not a library hiccup.
 * - `malformed_digest` — a digest string was not the `sha256:<64 lowercase
 *   hex>` the protocol grammar requires (`SPEC.md` §6.2).
 */
export type RecordHashErrorKind = "not_an_object" | "not_canonicalizable" | "malformed_digest";

/** Thrown when a record cannot be hashed, or a digest cannot be used. `kind` names which. */
export class RecordHashError extends Error {
  /** Which of the three failures this is. */
  readonly kind: RecordHashErrorKind;

  constructor(kind: RecordHashErrorKind, message: string) {
    super(message);
    this.name = "RecordHashError";
    this.kind = kind;
  }
}

const UTF8 = new TextEncoder();

// ---------------------------------------------------------------------------
// RFC 8785 (JCS)
// ---------------------------------------------------------------------------

/**
 * The RFC 8785 (JCS) canonical text of a JSON value.
 *
 * The input is a JSON value as `JSON.parse` produces it: `null`, booleans,
 * finite numbers, strings, arrays, and plain objects. An object member whose
 * value is `undefined` is omitted, exactly as `JSON.stringify` omits it, so a
 * record assembled in TypeScript with an unset optional field hashes as the
 * JSON it would be sent as. Anything else throws rather than being coerced.
 *
 * Exposed because a hash mismatch between two implementations is unreadable
 * and a diff of the canonical text is not.
 *
 * @throws RecordHashError (`not_canonicalizable`) on `NaN`, an infinity, a
 *   lone surrogate in a string or member name, or a non-JSON value.
 */
export function canonicalizeJson(value: unknown): string {
  const out: string[] = [];
  writeCanonical(value, out, null);
  return out.join("");
}

/** Whether `value` is an object `JSON.parse` could have produced — no class instances, no arrays. */
function isPlainObject(value: unknown): value is Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return false;
  const proto: unknown = Object.getPrototypeOf(value);
  return proto === Object.prototype || proto === null;
}

/**
 * Append the canonical text of `value` to `out`.
 *
 * `omit` names one member to drop from **this** object only. It is passed as
 * `null` to every recursive call, which is what makes the omit-self rule
 * top-level-only by construction rather than by a check someone can forget.
 */
function writeCanonical(value: unknown, out: string[], omit: string | null): void {
  if (value === null) {
    out.push("null");
    return;
  }
  switch (typeof value) {
    case "boolean":
      out.push(value ? "true" : "false");
      return;
    case "number":
      // RFC 8785 §3.2.2.3 is ECMAScript `Number::toString`, which is what
      // `JSON.stringify` applies to a finite number — shortest round-trip
      // digits, the 1e21 / 1e-7 exponent thresholds, and `-0` as `0`. It
      // writes `null` for the non-finite values JCS must refuse, so those are
      // caught first.
      if (!Number.isFinite(value)) {
        throw new RecordHashError(
          "not_canonicalizable",
          `RFC 8785 refuses the non-finite number ${String(value)}`,
        );
      }
      out.push(JSON.stringify(value));
      return;
    case "string":
      out.push(canonicalString(value));
      return;
    case "object":
      break;
    default:
      throw new RecordHashError(
        "not_canonicalizable",
        `a ${typeof value} is not a JSON value and has no RFC 8785 form`,
      );
  }

  if (Array.isArray(value)) {
    out.push("[");
    for (let i = 0; i < value.length; i += 1) {
      if (i > 0) out.push(",");
      const element: unknown = value[i];
      if (element === undefined) {
        // A hole or an explicit `undefined`: `JSON.stringify` would write
        // `null`, which is a different value from the one the caller holds.
        throw new RecordHashError(
          "not_canonicalizable",
          `array element ${i} is undefined, which is not a JSON value`,
        );
      }
      writeCanonical(element, out, null);
    }
    out.push("]");
    return;
  }

  if (!isPlainObject(value)) {
    throw new RecordHashError(
      "not_canonicalizable",
      "only plain objects are JSON objects; serialize a class instance (a Date, a Map) to JSON first",
    );
  }
  const object = value;

  // RFC 8785 §3.2.3: members sorted by their names' UTF-16 code units. The
  // default `sort` comparator compares exactly that — which is why U+1F600 (a
  // surrogate pair led by 0xD83D) sorts before U+FB33 despite its higher code
  // point.
  const names = Object.keys(object)
    .filter((name) => name !== omit && object[name] !== undefined)
    .sort();
  out.push("{");
  names.forEach((name, i) => {
    if (i > 0) out.push(",");
    out.push(canonicalString(name), ":");
    writeCanonical(object[name], out, null);
  });
  out.push("}");
}

/**
 * RFC 8785 §3.2.2.2 string serialization, refusing lone surrogates.
 *
 * `JSON.stringify` on a well-formed string is exactly the JCS escaping: `"`
 * and `\` escaped, the control characters U+0000–U+001F as `\b \t \n \f \r`
 * or `\u00xx` in lowercase hex, and everything else — `/`, non-ASCII,
 * astral — written literally. On a lone surrogate it writes a `\udxxx`
 * escape instead of failing, which JCS forbids, so that case is checked
 * first.
 */
function canonicalString(s: string): string {
  for (let i = 0; i < s.length; i += 1) {
    const unit = s.charCodeAt(i);
    if (unit >= 0xd800 && unit <= 0xdbff) {
      // `charCodeAt` past the end is `NaN`, which fails both comparisons, so a
      // high surrogate at the very end is caught here too.
      const next = s.charCodeAt(i + 1);
      if (next >= 0xdc00 && next <= 0xdfff) {
        i += 1;
        continue;
      }
      throw loneSurrogate(unit, i);
    }
    if (unit >= 0xdc00 && unit <= 0xdfff) throw loneSurrogate(unit, i);
  }
  return JSON.stringify(s);
}

function loneSurrogate(unit: number, at: number): RecordHashError {
  return new RecordHashError(
    "not_canonicalizable",
    `RFC 8785 refuses the lone surrogate U+${unit.toString(16).toUpperCase()} at UTF-16 offset ${at}`,
  );
}

// ---------------------------------------------------------------------------
// record_hash (profile LH1, LH2)
// ---------------------------------------------------------------------------

/**
 * The exact bytes a record's `record_hash` is taken over: the UTF-8 of the
 * RFC 8785 canonicalization of the record with its top-level `record_hash`
 * member removed (profile LH1).
 *
 * The function to reach for when two implementations disagree about a hash —
 * and the one the golden vectors' `jcs_utf8` pins.
 *
 * @throws RecordHashError `not_an_object` if `record` is not a plain JSON
 *   object; `not_canonicalizable` if it holds something JCS refuses.
 */
export function recordHashPreimage(record: unknown): Uint8Array {
  if (!isPlainObject(record)) {
    throw new RecordHashError("not_an_object", "a record must be a JSON object");
  }
  // The member is skipped during the walk rather than deleted from a copy:
  // copying with `{ ...record }` and `delete` would be equivalent, but an
  // assignment-based copy would turn an own `__proto__` member (which
  // `JSON.parse` produces faithfully) into a prototype change.
  const out: string[] = [];
  writeCanonical(record, out, RECORD_HASH_MEMBER);
  return UTF8.encode(out.join(""));
}

/**
 * A record's content-addressed identity (profile LH1):
 * `"sha256:" + hex(sha256(JCS(record without its record_hash member)))`.
 *
 * @throws RecordHashError as {@link recordHashPreimage} does.
 */
export function recordHash(record: unknown): string {
  const digest = createHash("sha256").update(recordHashPreimage(record)).digest();
  return digestString(new Uint8Array(digest));
}

/**
 * Whether a record's stored `record_hash` is the one its content produces.
 *
 * `false` is the interesting answer: the record was edited after it was
 * hashed, or was hashed by an implementation that canonicalizes differently.
 * A record with no `record_hash` member is `false`, not an error — it is
 * unhashed, not malformed.
 *
 * @throws RecordHashError as {@link recordHashPreimage} does.
 */
export function recordHashIsCurrent(record: unknown): boolean {
  const computed = recordHash(record);
  // `recordHash` has already proved `record` is a plain object.
  const stored = (record as Record<string, unknown>)[RECORD_HASH_MEMBER];
  return typeof stored === "string" && stored === computed;
}

// ---------------------------------------------------------------------------
// RecordAttestation (profile LC4, LC5)
// ---------------------------------------------------------------------------

/**
 * The message an Ed25519 record attestation signs:
 * {@link RECORD_ATTESTATION_DOMAIN} followed by the digest's 32 raw bytes.
 *
 * Public because it is the normative rule, not an implementation detail: a
 * provider signing in an HSM or a KMS builds these bytes, signs them with its
 * own backend, and never hands this SDK a secret.
 *
 * @throws RecordHashError `malformed_digest` if `recordHash` is not
 *   `sha256:<64 lowercase hex>`.
 */
export function recordAttestationMessage(recordHash: string): Uint8Array {
  const raw = parseDigest(recordHash);
  if (raw === null) {
    throw new RecordHashError(
      "malformed_digest",
      `expected a sha256:<64 lowercase hex> digest, found ${recordHash}`,
    );
  }
  const tag = UTF8.encode(RECORD_ATTESTATION_DOMAIN);
  const message = new Uint8Array(tag.length + raw.length);
  message.set(tag, 0);
  message.set(raw, tag.length);
  return message;
}

/**
 * Verify a detached attestation against an already-computed `record_hash`.
 *
 * The primitive for an auditor holding the hash and the signature but not the
 * record — which is the point of a detached attestation over a content
 * address. The check order matches the frame layer's: algorithm, then the
 * signed hash's grammar, then the hashes compared *before* the signature is
 * touched, so a record edited after signing is reported as
 * `commitment_mismatch` rather than as a bad signature.
 */
export function verifySignedRecordHash(
  expectedRecordHash: string,
  attestation: RecordAttestation,
  publicKey: Uint8Array,
): AttestationVerdict {
  if (attestation.algorithm !== ALGORITHM_ED25519) {
    return { verdict: "unknown_algorithm", algorithm: attestation.algorithm };
  }
  let message: Uint8Array;
  try {
    message = recordAttestationMessage(attestation.signed_record_hash);
  } catch {
    return { verdict: "malformed_commitment" };
  }
  if (attestation.signed_record_hash !== expectedRecordHash) {
    return {
      verdict: "commitment_mismatch",
      expected: expectedRecordHash,
      signed: attestation.signed_record_hash,
    };
  }
  return verifyEd25519(message, attestation.signature, publicKey);
}

/**
 * Verify a detached attestation against the record it claims to sign.
 *
 * Recomputes the record's hash rather than reading the stored member (profile
 * LC5), so a record whose content was edited and whose `record_hash` was then
 * rewritten to match is caught as a `commitment_mismatch` — verifying against
 * the stored member would pass it.
 *
 * @throws RecordHashError if the record cannot be hashed at all. That is a
 *   distinct outcome from every verdict: "this is not a record" is not a
 *   statement about the signature.
 */
export function verifyRecordAttestation(
  record: unknown,
  attestation: RecordAttestation,
  publicKey: Uint8Array,
): AttestationVerdict {
  return verifySignedRecordHash(recordHash(record), attestation, publicKey);
}

/**
 * Sign a `record_hash` in-process (ADR 0033). Mirrors the Rust reference's
 * `sign_record_attestation`.
 *
 * For a provider content to hold key material in memory; see the README's
 * "Key custody" section for what that costs. A provider using an HSM or KMS
 * signs {@link recordAttestationMessage}'s bytes with its backend instead and
 * assembles the {@link RecordAttestation} itself.
 *
 * @throws RecordHashError `malformed_digest` if `recordHash` is malformed.
 */
export function signRecordAttestation(
  recordHash: string,
  signingKey: SigningKey,
  keyId: string,
  attesterId: string,
  issuedAt: string,
): RecordAttestation {
  const message = recordAttestationMessage(recordHash);
  return {
    signed_record_hash: recordHash,
    key_id: keyId,
    algorithm: ALGORITHM_ED25519,
    attester_id: attesterId,
    signature: toHex(signEd25519(message, signingKey)),
    issued_at: issuedAt,
  };
}

/**
 * Sign a record's own recomputed hash — the convenience a provider appending
 * a record wants, so the signed hash cannot drift from the content by a
 * copy-paste. Mirrors the Rust reference's `sign_record`.
 *
 * @throws RecordHashError if the record cannot be hashed.
 */
export function signRecord(
  record: unknown,
  signingKey: SigningKey,
  keyId: string,
  attesterId: string,
  issuedAt: string,
): RecordAttestation {
  return signRecordAttestation(recordHash(record), signingKey, keyId, attesterId, issuedAt);
}
