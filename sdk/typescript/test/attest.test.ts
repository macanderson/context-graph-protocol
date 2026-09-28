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

import { createPublicKey, generateKeyPairSync } from "node:crypto";

import {
  ALGORITHM_ED25519,
  bindsContent,
  digestString,
  encodeProvenanceLink,
  frameCommitment,
  fromHex,
  inclusionPathSides,
  inclusionProof,
  isValid,
  isWellShaped,
  MAX_INCLUSION_PATH_STEPS,
  merkleRoot,
  parseDigest,
  provenanceChainHead,
  publicKeyFor,
  rootFromProof,
  signatureVerifies,
  signCommitment,
  signFrameAttestation,
  signingKeyFromSeed,
  toHex,
  verifyCommitment,
  verifyFrameAttestation,
  verifyFrameInclusion,
  type AttestableFrame,
  type InclusionProof,
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

test("a present-but-empty field encodes to the published bytes, distinct from an absent one", () => {
  // "These two differ" is the local test above; this is the stronger claim
  // that they are these exact bytes (#125). A port that collapsed `""` to
  // absent would satisfy the first and fail here, by name.
  //
  // The fixture must still spell the member out: a reader that normalized
  // `"uri": ""` away on load would make both vectors below test the absent
  // case twice and pass for the wrong reason.
  assert.ok(Object.hasOwn(link("empty_uri"), "uri"), "the fixture's empty_uri link lost its `uri` member");
  assert.equal(link("empty_uri").uri, "");
  assert.ok(!Object.hasOwn(link("absent_uri"), "uri"), "the fixture's absent_uri link grew a `uri` member");

  assert.equal(toHex(encodeProvenanceLink(link("empty_uri"))), V.link_encodings_hex.empty_uri);
  assert.equal(toHex(encodeProvenanceLink(link("absent_uri"))), V.link_encodings_hex.absent_uri);
  assert.equal(digestString(provenanceChainHead([link("empty_uri")])), V.chain_heads.empty_uri);
  assert.equal(digestString(provenanceChainHead([link("absent_uri")])), V.chain_heads.absent_uri);
  assert.notEqual(V.chain_heads.empty_uri, V.chain_heads.absent_uri);
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

test("hex is accepted in exactly one spelling", () => {
  // The protocol's grammar is lowercase (`is_well_formed_digest`). Accepting
  // uppercase would mean two implementations disagreeing about whether the
  // same attestation is well-formed, which is the class of divergence this
  // whole port exists to close.
  const commitment = signedCommitment();
  const upper = attestation();
  upper.signature = upper.signature.toUpperCase();
  assert.deepEqual(
    verifyCommitment(commitment, upper, publicKey()),
    { verdict: "malformed_signature" },
  );

  const upperCommitment = attestation();
  upperCommitment.signed_commitment =
    "sha256:" + V.signature.attestation.signed_commitment.slice(7).toUpperCase();
  assert.deepEqual(
    verifyCommitment(commitment, upperCommitment, publicKey()),
    { verdict: "malformed_commitment" },
  );
  assert.equal(parseDigest(upperCommitment.signed_commitment), null);
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

// ---------------------------------------------------------------------------
// Signing (#127, ADR 0033). Pinned to the published signature, never
// round-tripped against this port's own verifier alone: a signer and a
// verifier that share a bug agree with each other and with nothing else.
// ---------------------------------------------------------------------------

const seed = (): Uint8Array => fromHex(V.signature.signing_key_seed_hex)!;

test("the published seed derives the published public key", () => {
  assert.equal(toHex(publicKeyFor(seed())), V.signature.public_key_hex);
  assert.equal(toHex(publicKeyFor(signingKeyFromSeed(seed()))), V.signature.public_key_hex);
});

test("signing the published commitment with the published seed reproduces the published signature byte for byte", () => {
  const published: ProvenanceAttestation = V.signature.attestation;
  const signed = signCommitment(
    signedCommitment(),
    seed(),
    published.key_id,
    published.attester_id,
    published.issued_at,
  );
  // Ed25519 is deterministic (RFC 8032), so equality here is exact, not
  // "also verifies".
  assert.equal(signed.signature, published.signature);
  assert.deepEqual(signed, published);
  assert.deepEqual(verifyCommitment(signedCommitment(), signed, publicKey()), { verdict: "valid" });
});

test("a KeyObject built once signs the same bytes as the raw seed", () => {
  const key = signingKeyFromSeed(seed());
  const published: ProvenanceAttestation = V.signature.attestation;
  assert.equal(
    signCommitment(signedCommitment(), key, "key-1", "oxagen", published.issued_at).signature,
    published.signature,
  );
});

test("signFrameAttestation over the published frame reproduces the published attestation", () => {
  const spec = V.frame_commitment;
  const frame: AttestableFrame = {
    id: spec.frame.id,
    content_digest: spec.frame.content_digest,
    provenance: (spec.frame.provenance as string[]).map(link),
  };
  const published: ProvenanceAttestation = V.signature.attestation;
  const signed = signFrameAttestation(
    spec.provider_id,
    frame,
    seed(),
    published.key_id,
    published.attester_id,
    published.issued_at,
  );
  assert.deepEqual(signed, published);
  assert.deepEqual(verifyFrameAttestation(spec.provider_id, frame, signed, publicKey()), {
    verdict: "valid",
  });
});

test("signFrameAttestation refuses a frame that declares no content_digest (ADR 0018)", () => {
  for (const frame of [
    { id: "no-digest" },
    { id: "null-digest", content_digest: null as unknown as string },
  ] as AttestableFrame[]) {
    assert.throws(
      () => signFrameAttestation("repo-graph", frame, seed(), "key-1", "oxagen", "2026-08-27T00:00:00Z"),
      TypeError,
    );
  }
});

test("a verified signature over a digest-less frame is identity-only, not valid (ADR 0018)", () => {
  // Such signatures predate the refusal above and must still be readable —
  // labelled, not rejected, and never mistaken for a binding of the bytes.
  const frame: AttestableFrame = { id: "legacy", provenance: [link("file")] };
  const legacy = signCommitment(
    frameCommitment("repo-graph", frame),
    seed(),
    "key-1",
    "oxagen",
    "2026-08-27T00:00:00Z",
  );
  const verdict = verifyFrameAttestation("repo-graph", frame, legacy, publicKey());
  assert.deepEqual(verdict, { verdict: "valid_identity_only" });
  assert.ok(!isValid(verdict));
  assert.ok(signatureVerifies(verdict));
  assert.ok(!bindsContent(verdict));

  // Adding a digest after the fact is a different commitment, not an upgrade.
  assert.equal(
    verifyFrameAttestation(
      "repo-graph",
      { ...frame, content_digest: `sha256:${"ab".repeat(32)}` },
      legacy,
      publicKey(),
    ).verdict,
    "commitment_mismatch",
  );
});

test("a signing key that is not a private Ed25519 key is refused, not misused", () => {
  const commitment = signedCommitment();
  assert.throws(() => signCommitment(commitment, new Uint8Array(31), "k", "a", "t"), RangeError);
  assert.throws(() => signCommitment(new Uint8Array(31), seed(), "k", "a", "t"), RangeError);

  const publicOnly = createPublicKey(signingKeyFromSeed(seed()));
  assert.throws(() => signCommitment(commitment, publicOnly, "k", "a", "t"), TypeError);

  const { privateKey: ecKey } = generateKeyPairSync("ec", { namedCurve: "P-256" });
  assert.throws(() => signCommitment(commitment, ecKey, "k", "a", "t"), TypeError);
});

test("verifyFrameInclusion checks a frame against a signed root, and bounds the walk", () => {
  const providerId: string = V.merkle.provider_id;
  const frames = V.merkle.leaf_frames as AttestableFrame[];
  const leaves = merkleLeaves(7);
  const root = merkleRoot(leaves);
  assert.equal(digestString(root), V.merkle.roots_by_leaf_count["7"]);
  const signedRoot = signCommitment(root, seed(), "key-1", "oxagen", "2026-08-27T00:00:00Z");
  const proof = inclusionProof(leaves, 3)!;

  assert.deepEqual(verifyFrameInclusion(providerId, frames[3]!, proof, signedRoot, publicKey()), {
    verdict: "valid",
  });
  // Another frame under the same proof recomputes a different root.
  assert.equal(
    verifyFrameInclusion(providerId, frames[2]!, proof, signedRoot, publicKey()).verdict,
    "commitment_mismatch",
  );

  // ADR 0018 holds through the tree: a digest-less leaf is identity-only.
  const bare: AttestableFrame = { id: "bare" };
  const bareLeaves = [frameCommitment(providerId, bare), ...leaves.slice(1)];
  const bareRoot = signCommitment(merkleRoot(bareLeaves), seed(), "key-1", "oxagen", "2026-08-27T00:00:00Z");
  const bareVerdict = verifyFrameInclusion(providerId, bare, inclusionProof(bareLeaves, 0)!, bareRoot, publicKey());
  assert.deepEqual(bareVerdict, { verdict: "valid_identity_only" });
  assert.ok(!isValid(bareVerdict));

  // Every step costs a hash, and the provider chose the path: an over-long
  // one is refused on its length before anything is hashed.
  const overlong = {
    ...proof,
    path: Array.from({ length: MAX_INCLUSION_PATH_STEPS + 1 }, () => proof.path[0]!),
  };
  assert.deepEqual(verifyFrameInclusion(providerId, frames[3]!, overlong, signedRoot, publicKey()), {
    verdict: "malformed_commitment",
  });
  // So is a leaf index outside the tree the proof describes.
  assert.deepEqual(
    verifyFrameInclusion(providerId, frames[3]!, { ...proof, leaf_index: 7 }, signedRoot, publicKey()),
    { verdict: "malformed_commitment" },
  );
});

// ---------------------------------------------------------------------------
// Inclusion-proof shape (`SPEC.md` §6.5.3, ADR 0031)
// ---------------------------------------------------------------------------

test("the shape function predicts every honest proof, odd tree sizes included", () => {
  // Mirrors the Rust reference's
  // `every_honest_proof_is_well_shaped_and_its_sides_are_predicted`.
  for (let count = 1; count <= 40; count += 1) {
    const commitments = Array.from({ length: count }, (_, i) =>
      frameCommitment("repo-graph", { id: `f${i}` }),
    );
    const root = digestString(merkleRoot(commitments));
    for (let index = 0; index < count; index += 1) {
      const proof = inclusionProof(commitments, index)!;
      assert.deepEqual(
        inclusionPathSides(index, count),
        proof.path.map((step) => step.sibling_is_left),
        `leaf ${index} of ${count}`,
      );
      assert.ok(isWellShaped(proof), `leaf ${index} of ${count}`);
      assert.equal(digestString(rootFromProof(commitments[index]!, proof)!), root);
    }
  }
});

test("the shape function's published examples, and every index that is not one", () => {
  assert.deepEqual(inclusionPathSides(0, 1), []);
  assert.deepEqual(inclusionPathSides(3, 7), [true, true, false]);
  assert.deepEqual(inclusionPathSides(6, 7), [true, true]);
  assert.equal(inclusionPathSides(7, 7), null);
  assert.equal(inclusionPathSides(0, 0), null, "an empty tree has no leaf");
  // Rust types these as `usize`; JSON hands this port anything, and a count
  // or an index that is not a non-negative safe integer describes no tree.
  for (const [index, count] of [
    [-1, 7],
    [1.5, 7],
    [3, 7.5],
    [Number.NaN, 7],
    [0, Number.POSITIVE_INFINITY],
    [0, 2 ** 53],
  ] as const) {
    assert.equal(inclusionPathSides(index, count), null, `leaf ${index} of ${count}`);
  }
  // A provider-supplied count at the top of the safe range is arithmetic, not
  // a long loop: 53 levels.
  assert.equal(inclusionPathSides(0, Number.MAX_SAFE_INTEGER)?.length, 53);
});

test("every published inclusion vector is well-shaped and still verifies", () => {
  // The fixture's own proof object, not one rebuilt here: the refusal must
  // never reach a proof the reference emitted.
  const spec = V.merkle.inclusion_proof as InclusionProof;
  assert.ok(isWellShaped(spec));
  assert.equal(
    digestString(rootFromProof(merkleLeaves(spec.leaf_count)[spec.leaf_index]!, spec)!),
    V.merkle.roots_by_leaf_count[String(spec.leaf_count)],
  );
  // And every leaf of every published root.
  for (const [count, root] of Object.entries(V.merkle.roots_by_leaf_count)) {
    const leaves = merkleLeaves(Number(count));
    leaves.forEach((leaf, index) => {
      const proof = inclusionProof(leaves, index)!;
      assert.ok(isWellShaped(proof), `leaf ${index} of ${count}`);
      assert.equal(digestString(rootFromProof(leaf, proof)!), root, `leaf ${index} of ${count}`);
    });
  }
});

test("a genuine path under a false leaf_count or leaf_index is refused before hashing", () => {
  // Mirrors the Rust reference's
  // `a_proof_is_refused_when_its_stated_tree_does_not_produce_its_path`.
  const providerId: string = V.merkle.provider_id;
  const frames = V.merkle.leaf_frames as AttestableFrame[];
  const leaves = merkleLeaves(7);
  const signedRoot = signCommitment(merkleRoot(leaves), seed(), "key-1", "oxagen", "2026-08-27T00:00:00Z");
  const honest = V.merkle.inclusion_proof as InclusionProof;
  const verify = (proof: InclusionProof) =>
    verifyFrameInclusion(providerId, frames[3]!, proof, signedRoot, publicKey());
  assert.deepEqual(verify(honest), { verdict: "valid" });

  // The witness: the siblings are genuine, so a walk that ignored the stated
  // tree would recompute the signed root and call these "valid". Leaf 3 of 4
  // sits two levels down, not three; leaf 3 of 9 and of 16 sit four down.
  // (Leaf 3 of 5, 6 or 8 has the same three sides as leaf 3 of 7, so no shape
  // check can tell them apart; a host refuses those on F12, by comparing
  // `leaf_count` with the frames the answer carries.)
  for (const leaf_count of [0, 1, 2, 3, 4, 9, 16, 1000]) {
    const resized = { ...honest, leaf_count };
    assert.ok(!isWellShaped(resized), `leaf 3 of ${leaf_count}`);
    assert.equal(rootFromProof(leaves[3]!, resized), null, `leaf 3 of ${leaf_count}`);
    assert.deepEqual(verify(resized), { verdict: "malformed_commitment" }, `leaf 3 of ${leaf_count}`);
  }
  // Every other index of the same seven-leaf tree has a different shape, so
  // the same path relabelled with it is refused too.
  for (const leaf_index of [0, 1, 2, 4, 5, 6, 7, -1, 3.5]) {
    const relabelled = { ...honest, leaf_index };
    assert.ok(!isWellShaped(relabelled), `leaf ${leaf_index} of 7`);
    assert.equal(rootFromProof(leaves[3]!, relabelled), null, `leaf ${leaf_index} of 7`);
    assert.deepEqual(verify(relabelled), { verdict: "malformed_commitment" }, `leaf ${leaf_index} of 7`);
  }

  // A side flipped: the walk would compute a different root, and the shape
  // check refuses it before it does.
  const flipped = {
    ...honest,
    path: honest.path.map((step, i) => (i === 0 ? { ...step, sibling_is_left: !step.sibling_is_left } : step)),
  };
  assert.equal(rootFromProof(leaves[3]!, flipped), null);
  assert.deepEqual(verify(flipped), { verdict: "malformed_commitment" });

  // One step too many, one too few, and a side that is not a boolean.
  const longer = { ...honest, path: [...honest.path, honest.path[0]!] };
  const shorter = { ...honest, path: honest.path.slice(0, -1) };
  const stringly = {
    ...honest,
    path: honest.path.map((step) => ({ ...step, sibling_is_left: String(step.sibling_is_left) as unknown as boolean })),
  };
  for (const [name, proof] of [
    ["longer", longer],
    ["shorter", shorter],
    ["stringly", stringly],
  ] as const) {
    assert.ok(!isWellShaped(proof), name);
    assert.equal(rootFromProof(leaves[3]!, proof), null, name);
    assert.deepEqual(verify(proof), { verdict: "malformed_commitment" }, name);
  }
});
