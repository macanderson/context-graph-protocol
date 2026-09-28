// Inclusion-proof shape, checked before any hashing (SPEC.md §6.5.3, ADR 0031).
//
// RFC 6962 fixes the path for leaf i of a tree of n leaves. A verifier that
// checked only i < n could be shown a genuine path under an index or a count
// that describes a different tree, and would recompute the same root for
// both. These witnesses pin the Go port to the Rust reference's
// InclusionProof::is_well_shaped and inclusion_path_sides.
package attest

import (
	"math"
	"strconv"
	"testing"
)

// TestInclusionPathSidesMatchesTheReference pins the shapes the Rust
// reference's doc test states for inclusion_path_sides.
func TestInclusionPathSidesMatchesTheReference(t *testing.T) {
	cases := []struct {
		index, count int
		want         []bool
		ok           bool
	}{
		{0, 1, []bool{}, true},
		{3, 7, []bool{true, true, false}, true},
		{6, 7, []bool{true, true}, true},
		{7, 7, nil, false},
		{0, 0, nil, false},
		{-1, 7, nil, false},
	}
	for _, c := range cases {
		got, ok := InclusionPathSides(c.index, c.count)
		if ok != c.ok {
			t.Errorf("(%d, %d): ok = %v, want %v", c.index, c.count, ok, c.ok)
			continue
		}
		if len(got) != len(c.want) {
			t.Errorf("(%d, %d): got %v, want %v", c.index, c.count, got, c.want)
			continue
		}
		for i := range got {
			if got[i] != c.want[i] {
				t.Errorf("(%d, %d): got %v, want %v", c.index, c.count, got, c.want)
				break
			}
		}
	}
}

// TestInclusionPathSidesIsArithmeticForAHugeCount covers a provider-supplied
// count near math.MaxInt: the split point is bit arithmetic, so the walk
// neither overflows nor loops, and stays within 64 steps.
func TestInclusionPathSidesIsArithmeticForAHugeCount(t *testing.T) {
	for _, index := range []int{0, math.MaxInt / 2, math.MaxInt - 1} {
		sides, ok := InclusionPathSides(index, math.MaxInt)
		if !ok {
			t.Fatalf("index %d of MaxInt is in range", index)
		}
		if len(sides) == 0 || len(sides) > MaxInclusionPathSteps {
			t.Errorf("index %d of MaxInt: %d steps", index, len(sides))
		}
	}
}

// TestEveryBuiltProofIsWellShaped checks every leaf of every tree up to 40
// leaves, as the Rust reference does: no honest proof changes its verdict.
func TestEveryBuiltProofIsWellShaped(t *testing.T) {
	for count := 1; count <= 40; count++ {
		leaves := make([][32]byte, 0, count)
		for i := 0; i < count; i++ {
			leaves = append(leaves, FrameCommitment("shape", Frame{ID: strconv.Itoa(i)}))
		}
		root := MerkleRoot(leaves)
		for index := 0; index < count; index++ {
			proof, ok := BuildInclusionProof(leaves, index)
			if !ok {
				t.Fatalf("no proof for leaf %d of %d", index, count)
			}
			if !proof.IsWellShaped() {
				t.Errorf("leaf %d of %d: a built proof must be well-shaped", index, count)
				continue
			}
			got, ok := RootFromProof(leaves[index], proof)
			if !ok || got != root {
				t.Errorf("leaf %d of %d: a built proof must recompute the root", index, count)
			}
		}
	}
}

// TestThePublishedProofIsWellShaped walks the published proof exactly as the
// vector file spells it, not as this package rebuilds it, and checks every
// leaf of every published tree against its published root.
func TestThePublishedProofIsWellShaped(t *testing.T) {
	v := loadVectors(t)
	spec := v.Merkle.InclusionProof
	proof := InclusionProof{LeafIndex: spec.LeafIndex, LeafCount: spec.LeafCount, Path: spec.Path}
	if !proof.IsWellShaped() {
		t.Fatalf("the published proof (leaf %d of %d) must be well-shaped", spec.LeafIndex, spec.LeafCount)
	}
	leaves := merkleLeaves(v, spec.LeafCount)
	root, ok := RootFromProof(leaves[spec.LeafIndex], proof)
	if !ok {
		t.Fatal("the published proof must recompute a root")
	}
	want := v.Merkle.RootsByLeafCount[strconv.Itoa(spec.LeafCount)]
	if got := DigestString(root); got != want {
		t.Errorf("recomputed root\n got %s\nwant %s", got, want)
	}

	for key, want := range v.Merkle.RootsByLeafCount {
		count, err := strconv.Atoi(key)
		if err != nil {
			t.Fatalf("leaf count %q: %v", key, err)
		}
		leaves := merkleLeaves(v, count)
		for index := 0; index < count; index++ {
			proof, ok := BuildInclusionProof(leaves, index)
			if !ok || !proof.IsWellShaped() {
				t.Errorf("leaf %d of %d: no well-shaped proof", index, count)
				continue
			}
			root, ok := RootFromProof(leaves[index], proof)
			if !ok || DigestString(root) != want {
				t.Errorf("leaf %d of %d does not recompute the published root", index, count)
			}
		}
	}
}

// TestAGenuinePathUnderAFalseIndexOrCountIsRefused is the witness ADR 0031
// asks of every port: the published path for leaf 3 of 7 is genuine, and
// presenting it as coming from another position or another tree is refused
// before anything is hashed.
func TestAGenuinePathUnderAFalseIndexOrCountIsRefused(t *testing.T) {
	v := loadVectors(t)
	spec := v.Merkle.InclusionProof
	leaf := merkleLeaves(v, spec.LeafCount)[spec.LeafIndex]
	genuine := func() InclusionProof {
		path := make([]InclusionStep, len(spec.Path))
		copy(path, spec.Path)
		return InclusionProof{LeafIndex: spec.LeafIndex, LeafCount: spec.LeafCount, Path: path}
	}

	falseCount4 := genuine()
	falseCount4.LeafCount = 4 // leaf 3 of 4 sits two levels down, not three
	falseCount16 := genuine()
	falseCount16.LeafCount = 16 // four levels down
	falseCount0 := genuine()
	falseCount0.LeafCount = 0
	falseIndex2 := genuine()
	falseIndex2.LeafIndex = 2 // same length, different sides
	falseIndex6 := genuine()
	falseIndex6.LeafIndex = 6 // the lone right leaf is two levels down
	negativeIndex := genuine()
	negativeIndex.LeafIndex = -1
	flipped := genuine()
	flipped.Path[0].SiblingIsLeft = !flipped.Path[0].SiblingIsLeft
	longer := genuine()
	longer.Path = append(longer.Path, InclusionStep{Sibling: longer.Path[0].Sibling})
	shorter := genuine()
	shorter.Path = shorter.Path[:len(shorter.Path)-1]

	for name, proof := range map[string]InclusionProof{
		"leaf_count 4":               falseCount4,
		"leaf_count 16":              falseCount16,
		"leaf_count 0":               falseCount0,
		"leaf_index 2":               falseIndex2,
		"leaf_index 6":               falseIndex6,
		"leaf_index -1":              negativeIndex,
		"a flipped side":             flipped,
		"one step too many":          longer,
		"one step too few":           shorter,
		"a nil path for 3 of 7":      {LeafIndex: 3, LeafCount: 7},
		"a one-step path for 1 of 1": {LeafIndex: 0, LeafCount: 1, Path: spec.Path[:1]},
	} {
		if proof.IsWellShaped() {
			t.Errorf("%s: must not be well-shaped", name)
		}
		if _, ok := RootFromProof(leaf, proof); ok {
			t.Errorf("%s: RootFromProof must refuse a mis-shaped proof", name)
		}
	}

	if !genuine().IsWellShaped() {
		t.Error("the genuine proof must stay well-shaped")
	}
}
