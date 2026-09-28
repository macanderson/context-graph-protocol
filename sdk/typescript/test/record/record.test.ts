/**
 * The TypeScript `record_hash` and `RecordAttestation` port (#119),
 * reconciled against the published record vectors.
 *
 * Every expected value is read from `tests/fixtures/`: the canonical text and
 * hash of each record from `record-hash-vectors.json`, and the attestation and
 * its key from `record-attestation.json` and `record-attestation-key.json`.
 * The Rust reference (`contextgraph-types/src/record_attest.rs`) produced and
 * pins them. Nothing here asserts against a value this file computed.
 *
 * This file lives in its own directory so CI runs it as its own named step
 * (`sdk (typescript) is a conformant implementation`), separate from the
 * provenance-attestation vectors.
 */

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { fromHex, parseDigest, publicKeyFor, signCommitment, toHex } from "../../src/attest.js";
import {
  canonicalizeJson,
  RECORD_ATTESTATION_DOMAIN,
  RECORD_HASH_MEMBER,
  RecordHashError,
  recordAttestationMessage,
  recordHash,
  recordHashIsCurrent,
  recordHashPreimage,
  signRecord,
  signRecordAttestation,
  verifyRecordAttestation,
  verifySignedRecordHash,
  type RecordAttestation,
} from "../../src/record.js";

/**
 * The repository's `tests/fixtures/` directory, found by walking up from this
 * file rather than counting `..` segments — the compiled test runs from
 * `dist/test/record/` and the source from `test/record/`.
 */
function fixturesDir(): string {
  let dir = dirname(fileURLToPath(import.meta.url));
  for (let i = 0; i < 12; i += 1) {
    const candidate = join(dir, "tests", "fixtures");
    try {
      readFileSync(join(candidate, "record-hash-vectors.json"));
      return candidate;
    } catch {
      const parent = resolve(dir, "..");
      if (parent === dir) break;
      dir = parent;
    }
  }
  throw new Error("tests/fixtures/record-hash-vectors.json not found above this file");
}

const FIXTURES = fixturesDir();
const fixture = (name: string): any => JSON.parse(readFileSync(join(FIXTURES, name), "utf8"));

interface HashVector {
  record_file: string;
  jcs_utf8: string;
  record_hash: string;
}

const VECTORS: HashVector[] = fixture("record-hash-vectors.json").vectors;
const ATTESTATION: RecordAttestation = fixture("record-attestation.json");
const KEY = fixture("record-attestation-key.json");
const SIGNED_RECORD = (): Record<string, unknown> => fixture(KEY.signs);
const PUBLIC_KEY = (): Uint8Array => fromHex(KEY.public_key)!;
const SEED = (): Uint8Array => fromHex(KEY.signing_key_seed)!;
const UTF8 = new TextDecoder("utf-8", { fatal: true });

/** Assert `fn` throws a {@link RecordHashError} of the given kind. */
function throwsKind(fn: () => unknown, kind: RecordHashError["kind"], why?: string): void {
  assert.throws(fn, (error: unknown) => error instanceof RecordHashError && error.kind === kind, why);
}

// -- the published vectors (#119 DoD 3) --------------------------------------

test("the vector file covers every record fixture it names", () => {
  assert.ok(VECTORS.length >= 12, `expected at least 12 vectors, found ${VECTORS.length}`);
  assert.equal(new Set(VECTORS.map((v) => v.record_file)).size, VECTORS.length);
});

test("every record canonicalizes to the published jcs_utf8, byte for byte", () => {
  for (const vector of VECTORS) {
    const record: unknown = fixture(vector.record_file);
    assert.equal(
      UTF8.decode(recordHashPreimage(record)),
      vector.jcs_utf8,
      `${vector.record_file}: the RFC 8785 preimage differs from the published one`,
    );
  }
});

test("every record hashes to the published record_hash", () => {
  for (const vector of VECTORS) {
    const record: unknown = fixture(vector.record_file);
    assert.equal(recordHash(record), vector.record_hash, vector.record_file);
    // Each fixture also stores its own hash, and it is the current one.
    assert.ok(recordHashIsCurrent(record), `${vector.record_file}'s stored record_hash is stale`);
  }
});

test("the published attestation verifies under the published key, over a recomputed hash", () => {
  const verdict = verifyRecordAttestation(SIGNED_RECORD(), ATTESTATION, PUBLIC_KEY());
  assert.deepEqual(verdict, { verdict: "valid" });
  assert.equal(ATTESTATION.signed_record_hash, recordHash(SIGNED_RECORD()));
});

test("the signed message is the domain tag then the 32 raw digest bytes, as published", () => {
  const message = recordAttestationMessage(ATTESTATION.signed_record_hash);
  assert.equal(toHex(message), KEY.signed_message_hex);
  const tag = new TextEncoder().encode(RECORD_ATTESTATION_DOMAIN);
  assert.equal(message.length, tag.length + 32);
  assert.deepEqual(message.subarray(0, tag.length), tag);
  assert.deepEqual(message.subarray(tag.length), parseDigest(ATTESTATION.signed_record_hash));
});

test("signing the published record with the published seed reproduces the published attestation", () => {
  assert.equal(toHex(publicKeyFor(SEED())), KEY.public_key);
  const signed = signRecord(
    SIGNED_RECORD(),
    SEED(),
    ATTESTATION.key_id,
    ATTESTATION.attester_id,
    ATTESTATION.issued_at,
  );
  assert.deepEqual(signed, ATTESTATION);
  assert.deepEqual(
    signRecordAttestation(
      ATTESTATION.signed_record_hash,
      SEED(),
      ATTESTATION.key_id,
      ATTESTATION.attester_id,
      ATTESTATION.issued_at,
    ),
    ATTESTATION,
  );
});

// -- the omit-self rule (profile LH1) ----------------------------------------

test("a record's own hash is removed from its preimage, not blanked", () => {
  const withHash = SIGNED_RECORD();
  const absent = SIGNED_RECORD();
  delete absent[RECORD_HASH_MEMBER];
  const wrong = { ...SIGNED_RECORD(), [RECORD_HASH_MEMBER]: `sha256:${"f".repeat(64)}` };

  const preimage = UTF8.decode(recordHashPreimage(withHash));
  assert.equal(UTF8.decode(recordHashPreimage(absent)), preimage);
  assert.equal(UTF8.decode(recordHashPreimage(wrong)), preimage);
  assert.ok(!preimage.includes(RECORD_HASH_MEMBER), "a record must never hash over its own hash");
  assert.ok(!recordHashIsCurrent(absent), "an unhashed record is unhashed, not current");
  assert.ok(!recordHashIsCurrent(wrong));
});

test("only the top-level record_hash member is removed", () => {
  const nested = { ...SIGNED_RECORD(), extensions: { record_hash: "sha256:nested" } };
  assert.ok(UTF8.decode(recordHashPreimage(nested)).includes("sha256:nested"));
  assert.notEqual(recordHash(nested), recordHash(SIGNED_RECORD()));
});

test("member order does not change the hash", () => {
  assert.equal(
    recordHash(JSON.parse(`{"a":1,"b":2,"record_hash":"x"}`)),
    recordHash(JSON.parse(`{"record_hash":"x","b":2,"a":1}`)),
  );
});

test("a non-object is not a record", () => {
  for (const value of [[1, 2, 3], null, "record", 7, undefined]) {
    throwsKind(() => recordHash(value), "not_an_object", JSON.stringify(value));
  }
});

test("an undefined member is omitted as JSON.stringify omits it; anything else non-JSON is refused", () => {
  const record = { a: 1, b: undefined };
  assert.equal(recordHash(record), recordHash(JSON.parse(JSON.stringify(record))));

  throwsKind(() => recordHash({ at: new Date(0) }), "not_canonicalizable", "a Date");
  throwsKind(() => recordHash({ n: 1n }), "not_canonicalizable", "a bigint");
  throwsKind(() => recordHash({ list: [1, undefined] }), "not_canonicalizable", "an undefined element");
});

test("an own __proto__ member is content, not a prototype", () => {
  // `JSON.parse` keeps `__proto__` as an ordinary own member; a canonicalizer
  // that copied the record by assignment would silently drop it.
  const record = JSON.parse(`{"__proto__":{"x":1},"a":1}`);
  assert.equal(UTF8.decode(recordHashPreimage(record)), `{"__proto__":{"x":1},"a":1}`);
});

// -- RFC 8785 conformance -----------------------------------------------------

test("RFC 8785 §3.2.4: the specification's worked example canonicalizes byte for byte", () => {
  const input = JSON.parse(String.raw`{
    "numbers": [333333333.33333329, 1E30, 4.50, 2e-3, 0.000000000000000000000000001],
    "string": "€$\u000F\u000aA'B"\\\\"\/",
    "literals": [null, true, false]
  }`);
  // Section 3.2.4's hexadecimal listing, verbatim.
  const expected =
    "7b226c69746572616c73223a5b6e756c6c2c747275652c66616c73655d2c226e756d62657273223a5b33333333" +
    "33333333332e333333333333332c31652b33302c342e352c302e3030322c31652d32375d2c22737472696e6722" +
    "3a22e282ac245c75303030665c6e4127425c225c5c5c5c5c222f227d";
  assert.equal(toHex(new TextEncoder().encode(canonicalizeJson(input))), expected);
});

test("RFC 8785 §3.2.3: property names sort by UTF-16 code unit", () => {
  const input = JSON.parse(`{
    "\\u20ac": "Euro Sign",
    "\\r": "Carriage Return",
    "\\ufb33": "Hebrew Letter Dalet With Dagesh",
    "1": "One",
    "\\ud83d\\ude00": "Emoji: Grinning Face",
    "\\u0080": "Control",
    "\\u00f6": "Latin Small Letter O With Diaeresis"
  }`);
  const canonical = canonicalizeJson(input);
  const order = [
    "Carriage Return",
    "One",
    "Control",
    "Latin Small Letter O With Diaeresis",
    "Euro Sign",
    "Emoji: Grinning Face",
    "Hebrew Letter Dalet With Dagesh",
  ];
  let cursor = 0;
  for (const value of order) {
    const at = canonical.indexOf(value, cursor);
    assert.ok(at >= 0, `${value} missing or out of order in ${canonical}`);
    cursor = at + value.length;
  }
});

test("RFC 8785 Appendix B: every IEEE 754 sample serializes to the pinned ECMAScript text", () => {
  // Bit patterns and expected text from Appendix B, Table 1 — the same table
  // the Rust reference pins. NaN and the infinities are the refusal test below.
  const samples: [bigint, string][] = [
    [0x0000000000000000n, "0"],
    [0x8000000000000000n, "0"],
    [0x0000000000000001n, "5e-324"],
    [0x8000000000000001n, "-5e-324"],
    [0x7fefffffffffffffn, "1.7976931348623157e+308"],
    [0xffefffffffffffffn, "-1.7976931348623157e+308"],
    [0x4340000000000000n, "9007199254740992"],
    [0xc340000000000000n, "-9007199254740992"],
    [0x4430000000000000n, "295147905179352830000"],
    [0x44b52d02c7e14af5n, "9.999999999999997e+22"],
    [0x44b52d02c7e14af6n, "1e+23"],
    [0x44b52d02c7e14af7n, "1.0000000000000001e+23"],
    [0x444b1ae4d6e2ef4en, "999999999999999700000"],
    [0x444b1ae4d6e2ef4fn, "999999999999999900000"],
    [0x444b1ae4d6e2ef50n, "1e+21"],
    [0x3eb0c6f7a0b5ed8cn, "9.999999999999997e-7"],
    [0x3eb0c6f7a0b5ed8dn, "0.000001"],
    [0x41b3de4355555553n, "333333333.3333332"],
    [0x41b3de4355555554n, "333333333.33333325"],
    [0x41b3de4355555555n, "333333333.3333333"],
    [0x41b3de4355555556n, "333333333.3333334"],
    [0x41b3de4355555557n, "333333333.33333343"],
    [0xbecbf647612f3696n, "-0.0000033333333333333333"],
    [0x43143ff3c1cb0959n, "1424953923781206.2"],
  ];
  const view = new DataView(new ArrayBuffer(8));
  for (const [bits, expected] of samples) {
    view.setBigUint64(0, bits);
    const n = view.getFloat64(0);
    assert.equal(canonicalizeJson({ n }), `{"n":${expected}}`, `0x${bits.toString(16).padStart(16, "0")}`);
  }
});

test("RFC 8785 refuses NaN, the infinities, and lone surrogates rather than coercing them", () => {
  for (const n of [Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY]) {
    throwsKind(() => canonicalizeJson({ n }), "not_canonicalizable", String(n));
  }
  for (const s of ["\ud800", "a\udc00b", "\ud83d", "\ude00\ud83d"]) {
    throwsKind(() => canonicalizeJson({ s }), "not_canonicalizable", "a lone surrogate in a value");
    throwsKind(() => canonicalizeJson({ [s]: 1 }), "not_canonicalizable", "a lone surrogate in a name");
  }
  // A well-formed pair is fine, and written literally rather than escaped.
  assert.equal(canonicalizeJson({ s: "😀" }), `{"s":"\u{1F600}"}`);
});

// -- attestation (profile LC4, LC5) -------------------------------------------

test("editing the record after signing is caught as a mismatch, not a bad signature", () => {
  const tampered = { ...SIGNED_RECORD(), statement: "the api handler never retries" };
  const verdict = verifyRecordAttestation(tampered, ATTESTATION, PUBLIC_KEY());
  assert.equal(verdict.verdict, "commitment_mismatch");
});

test("rewriting the stored record_hash does not launder a tampered record (LC5)", () => {
  // Edit the content, then restate `record_hash` so the record is internally
  // consistent again. Verifying against the stored member would pass it.
  const laundered: Record<string, unknown> = { ...SIGNED_RECORD(), statement: "the api handler never retries" };
  laundered[RECORD_HASH_MEMBER] = recordHash(laundered);
  assert.ok(recordHashIsCurrent(laundered));
  assert.equal(verifyRecordAttestation(laundered, ATTESTATION, PUBLIC_KEY()).verdict, "commitment_mismatch");
});

test("a wrong key is a bad signature, not a mismatch", () => {
  const other = publicKeyFor(new Uint8Array(32).fill(3));
  assert.deepEqual(verifyRecordAttestation(SIGNED_RECORD(), ATTESTATION, other), {
    verdict: "bad_signature",
  });
});

test("a frame-layer signature over the same digest is not a record attestation", () => {
  // What the domain tag buys: sign the record hash's 32 bytes as a frame
  // commitment, and the signature must still not verify at the record layer.
  const raw = parseDigest(ATTESTATION.signed_record_hash)!;
  const frameSigned = signCommitment(raw, SEED(), ATTESTATION.key_id, ATTESTATION.attester_id, ATTESTATION.issued_at);
  const lifted: RecordAttestation = { ...ATTESTATION, signature: frameSigned.signature };
  assert.deepEqual(verifyRecordAttestation(SIGNED_RECORD(), lifted, PUBLIC_KEY()), {
    verdict: "bad_signature",
  });
});

test("every failure is named rather than collapsed into a boolean", () => {
  const record = SIGNED_RECORD();
  const key = PUBLIC_KEY();
  assert.deepEqual(verifyRecordAttestation(record, { ...ATTESTATION, algorithm: "dilithium3" }, key), {
    verdict: "unknown_algorithm",
    algorithm: "dilithium3",
  });
  assert.deepEqual(verifyRecordAttestation(record, { ...ATTESTATION, signed_record_hash: "not-a-digest" }, key), {
    verdict: "malformed_commitment",
  });
  assert.deepEqual(verifyRecordAttestation(record, { ...ATTESTATION, signature: "abcd" }, key), {
    verdict: "malformed_signature",
  });
  assert.deepEqual(verifyRecordAttestation(record, ATTESTATION, new Uint8Array(5)), {
    verdict: "malformed_key",
  });
  throwsKind(() => recordAttestationMessage("sha256:short"), "malformed_digest");
  throwsKind(() => signRecordAttestation("sha256:short", SEED(), "k", "a", "t"), "malformed_digest");
  throwsKind(() => verifyRecordAttestation([], ATTESTATION, key), "not_an_object");
});

test("a detached attestation verifies from the hash alone", () => {
  assert.deepEqual(verifySignedRecordHash(ATTESTATION.signed_record_hash, ATTESTATION, PUBLIC_KEY()), {
    verdict: "valid",
  });
});
