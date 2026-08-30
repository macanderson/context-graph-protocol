/**
 * The TypeScript attestation port, reconciled against the published vectors.
 *
 * Every expected value is read from `tests/vectors/attestation-vectors.json`,
 * which `contextgraph-types/tests/attestation_vectors.rs` mirrors and pins.
 * Nothing here asserts against a value this file computed: a port that agrees
 * with itself is what this suite exists to catch.
 */

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import {
  ALGORITHM_ED25519,
  digestString,
  encodeProvenanceLink,
  frameCommitment,
  fromHex,
  inclusionProof,
  isValid,
  merkleRoot,
  parseDigest,
  provenanceChainHead,
  rootFromProof,
  toHex,
  verifyCommitment,
  verifyFrameAttestation,
  type AttestableFrame,
  type ProvenanceAttestation,
} from "../src/attest.js";
import type { Provenance } from "../src/types.js";

/**
 * Walk up from this file until the vector fixture appears, rather than
 * counting `..` segments — the compiled test runs from `dist/test/` and the
 * source from `test/`, and a hardcoded depth is right for exactly one of them.
 */
function loadVectors(): any {
  let dir = dirname(fileURLToPath(import.meta.url));
  for (let i = 0; i < 10; i += 1) {
    const candidate = join(dir, "tests", "vectors", "attestation-vectors.json");
    try {
      return JSON.parse(readFileSync(candidate, "utf8"));
    } catch {
      const parent = resolve(dir, "..");
      if (parent === dir) break;
      dir = parent;
    }
  }
  throw new Error("tests/vectors/attestation-vectors.json not found above this file");
}

const V = loadVectors();

const link = (name: string): Provenance => V.links[name] as Provenance;

/** The Merkle leaves the fixture's roots are taken over, in canonical order. */
function merkleLeaves(count: number): Uint8Array[] {
  const providerId: string = V.merkle.provider_id;
  return (V.merkle.leaf_frames as AttestableFrame[])
    .slice(0, count)
    .map((frame) => frameCommitment(providerId, frame));
}

const attestation = (): ProvenanceAttestation => ({ ...V.signature.attestation });
const publicKey = (): Uint8Array => fromHex(V.signature.public_key_hex)!;
const signedCommitment = (): Uint8Array => parseDigest(V.signature.attestation.signed_commitment)!;

test("the link encoding matches the published bytes", () => {
  assert.equal(toHex(encodeProvenanceLink(link("ascii_minimal"))), V.link_encodings_hex.ascii_minimal);
  assert.equal(toHex(encodeProvenanceLink(link("unicode"))), V.link_encodings_hex.unicode);
});

test("the length prefix counts UTF-8 bytes, not what .length returns", () => {
  // The trap, demonstrated in this runtime rather than asserted about it: if
  // these three numbers were equal, the vector above could not tell a correct
  // port from one using `.length`.
  const trap = V.unicode_length_trap;
  const uri: string = link("unicode").uri!;
  const by: string = link("unicode").by!;
  assert.equal(new TextEncoder().encode(uri).length, trap.uri.utf8_bytes);
  assert.equal([...uri].length, trap.uri.code_points);
  assert.equal(uri.length, trap.uri.utf16_code_units);
  assert.equal(new TextEncoder().encode(by).length, trap.by.utf8_bytes);
  assert.equal([...by].length, trap.by.code_points);
  assert.equal(by.length, trap.by.utf16_code_units);
  assert.notEqual(
    trap.by.utf8_bytes,
    trap.by.utf16_code_units,
    "the astral character must make the two answers differ, or this proves nothing",
  );
});

test("an absent field never encodes like an empty one", () => {
  const absent: Provenance = { type: "file" };
  const empty: Provenance = { type: "file", uri: "" };
  assert.notDeepEqual(encodeProvenanceLink(absent), encodeProvenanceLink(empty));
});

test("the chain heads match the published vectors", () => {
  assert.equal(digestString(provenanceChainHead([])), V.chain_heads.empty);
  assert.equal(digestString(provenanceChainHead([link("file")])), V.chain_heads.file);
  assert.equal(
    digestString(provenanceChainHead([link("file"), link("derivation")])),
    V.chain_heads.file_then_derivation,
  );
  assert.equal(digestString(provenanceChainHead([link("unicode")])), V.chain_heads.unicode);
});

test("reordering the chain changes the head", () => {
  assert.notEqual(
    digestString(provenanceChainHead([link("file"), link("derivation")])),
    digestString(provenanceChainHead([link("derivation"), link("file")])),
  );
});

test("the frame commitment matches the published vector", () => {
  const spec = V.frame_commitment;
  const frame: AttestableFrame = {
    id: spec.frame.id,
    content_digest: spec.frame.content_digest,
    provenance: (spec.frame.provenance as string[]).map(link),
  };
  assert.equal(digestString(frameCommitment(spec.provider_id, frame)), spec.commitment);
});

test("the Merkle roots match the published vectors, odd leaf counts included", () => {
  for (const [count, root] of Object.entries(V.merkle.roots_by_leaf_count)) {
    assert.equal(
      digestString(merkleRoot(merkleLeaves(Number(count)))),
      root,
      `the ${count}-leaf root`,
    );
  }
});

test("the inclusion proof matches the published vector and recomputes the root", () => {
  const spec = V.merkle.inclusion_proof;
  const leaves = merkleLeaves(spec.leaf_count);
  const proof = inclusionProof(leaves, spec.leaf_index);
  assert.ok(proof !== null);
  assert.equal(proof.leaf_count, spec.leaf_count);
  assert.equal(proof.leaf_index, spec.leaf_index);
  assert.deepEqual(proof.path, spec.path);
  assert.equal(
    digestString(rootFromProof(leaves[spec.leaf_index]!, proof)!),
    V.merkle.roots_by_leaf_count[String(spec.leaf_count)],
  );
});

test("a proof does not validate a commitment that was not in the set", () => {
  const leaves = merkleLeaves(7);
  const proof = inclusionProof(leaves, 3)!;
  const outsider = frameCommitment("repo-graph", { id: "intruder" });
  assert.notEqual(
    digestString(rootFromProof(outsider, proof)!),
    V.merkle.roots_by_leaf_count["7"],
  );
});

test("the published signature verifies against the published key", () => {
  const verdict = verifyCommitment(signedCommitment(), attestation(), publicKey());
  assert.deepEqual(verdict, { verdict: "valid" });
  assert.ok(isValid(verdict));
  assert.equal(attestation().algorithm, ALGORITHM_ED25519);
});

test("perturbing one byte of the commitment is caught as a mismatch, not a bad signature", () => {
  const commitment = signedCommitment();
  const tampered = Uint8Array.from(commitment);
  tampered[0] = tampered[0]! ^ 0x01;
  const verdict = verifyCommitment(tampered, attestation(), publicKey());
  assert.equal(verdict.verdict, "commitment_mismatch");
  assert.ok(!isValid(verdict));
  // §6.5.4's ordering rule: the frame changed after signing, and saying "bad
  // signature" would send an operator after a key-management bug instead.
  assert.equal(
    (verdict as { signed: string }).signed,
    V.signature.attestation.signed_commitment,
  );
});

test("perturbing one byte of the signature is caught as a bad signature", () => {
  const forged = attestation();
  const bytes = fromHex(forged.signature)!;
  bytes[0] = bytes[0]! ^ 0x01;
  forged.signature = toHex(bytes);
  assert.deepEqual(
    verifyCommitment(signedCommitment(), forged, publicKey()),
    { verdict: "bad_signature" },
  );
});

test("perturbing one byte of the frame is caught through verifyFrameAttestation", () => {
  const spec = V.frame_commitment;
  const provenance = (spec.frame.provenance as string[]).map(link);
  const honest: AttestableFrame = {
    id: spec.frame.id,
    content_digest: spec.frame.content_digest,
    provenance,
  };
  assert.deepEqual(
    verifyFrameAttestation(spec.provider_id, honest, attestation(), publicKey()),
    { verdict: "valid" },
    "precondition: the published attestation signs this exact frame",
  );

  // The tamper a bare digest cannot see, because the tamperer rewrites the
  // digest too.
  const tampered: AttestableFrame = {
    ...honest,
    provenance: [{ ...provenance[0]!, uri: "src/evil.rs" }],
  };
  assert.equal(
    verifyFrameAttestation(spec.provider_id, tampered, attestation(), publicKey()).verdict,
    "commitment_mismatch",
  );
  // And the identity binding: same bytes, different provider.
  assert.equal(
    verifyFrameAttestation("impostor", honest, attestation(), publicKey()).verdict,
    "commitment_mismatch",
  );
});

test("every failure is named rather than collapsed into a boolean", () => {
  const key = publicKey();
  const commitment = signedCommitment();

  assert.deepEqual(
    verifyCommitment(commitment, { ...attestation(), algorithm: "dilithium3" }, key),
    { verdict: "unknown_algorithm", algorithm: "dilithium3" },
  );
  assert.deepEqual(
    verifyCommitment(commitment, { ...attestation(), signed_commitment: "not-a-digest" }, key),
    { verdict: "malformed_commitment" },
  );
  assert.deepEqual(
    verifyCommitment(commitment, { ...attestation(), signature: "abcd" }, key),
    { verdict: "malformed_signature" },
  );
  assert.deepEqual(
    verifyCommitment(commitment, attestation(), new Uint8Array(5)),
    { verdict: "malformed_key" },
  );
});

test("a strict verifier declines a small-order or non-canonical public key", () => {
  const strictness = V.verifier_strictness;
  const commitment = signedCommitment();
  const rejectable: string[] = [
    ...strictness.small_order_public_keys_hex,
    ...strictness.non_canonical_public_keys_hex,
  ];
  assert.ok(rejectable.length > 0);
  for (const hex of rejectable) {
    assert.deepEqual(
      verifyCommitment(commitment, attestation(), fromHex(hex)!),
      { verdict: "malformed_key" },
      `${hex} must not be usable as a verification key`,
    );
  }
});
