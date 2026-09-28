// RFC 8785 conformance for the Go canonicalizer.
//
// The expected values are the RFC's own: Section 3.2.4's byte listing,
// Section 3.2.3's sort-order data, and Appendix B's number table, entered
// verbatim from https://www.rfc-editor.org/rfc/rfc8785.txt. The Rust reference
// pins the same three in contextgraph-types/src/record_attest.rs, so the two
// implementations are held to one oracle rather than to each other.
package jcs

import (
	"encoding/hex"
	"errors"
	"math"
	"strconv"
	"strings"
	"testing"
)

func mustCanonicalize(t *testing.T, input string) string {
	t.Helper()
	out, err := Canonicalize([]byte(input))
	if err != nil {
		t.Fatalf("Canonicalize(%q): %v", input, err)
	}
	return string(out)
}

// TestRFC8785Section324WorkedExampleCanonicalizesByteForByte is the
// specification's worked example. The expected bytes are Section 3.2.4's
// hexadecimal listing.
func TestRFC8785Section324WorkedExampleCanonicalizesByteForByte(t *testing.T) {
	input := `{
		"numbers": [333333333.33333329, 1E30, 4.50,
		            2e-3, 0.000000000000000000000000001],
		"string": "\u20ac$\u000F\u000aA'\u0042\u0022\u005c\\\"\/",
		"literals": [null, true, false]
	}`
	want, err := hex.DecodeString("" +
		"7b226c69746572616c73223a5b6e756c6c2c747275652c66616c73655d2c226e" +
		"756d62657273223a5b3333333333333333332e333333333333332c31652b3330" +
		"2c342e352c302e3030322c31652d32375d2c22737472696e67223a22e282ac24" +
		"5c75303030665c6e4127425c225c5c5c5c5c222f227d")
	if err != nil {
		t.Fatal(err)
	}
	if got := mustCanonicalize(t, input); got != string(want) {
		t.Errorf("RFC 8785 §3.2.4\n got %s\nwant %s", got, want)
	}
}

// TestRFC8785Section323SortsMemberNamesByUTF16CodeUnit is the RFC's sorting
// data. The emoji is a surrogate pair whose leading code unit is 0xD83D, so it
// sorts before U+FB33 although its code point is far higher — the case a sort
// by code point or by UTF-8 bytes gets wrong.
func TestRFC8785Section323SortsMemberNamesByUTF16CodeUnit(t *testing.T) {
	input := `{
		"\u20ac": "Euro Sign",
		"\r": "Carriage Return",
		"\ufb33": "Hebrew Letter Dalet With Dagesh",
		"1": "One",
		"\ud83d\ude00": "Emoji: Grinning Face",
		"\u0080": "Control",
		"\u00f6": "Latin Small Letter O With Diaeresis"
	}`
	want := "{" +
		"\"\\r\":\"Carriage Return\"," +
		"\"1\":\"One\"," +
		"\"\u0080\":\"Control\"," +
		"\"\u00f6\":\"Latin Small Letter O With Diaeresis\"," +
		"\"\u20ac\":\"Euro Sign\"," +
		"\"\U0001F600\":\"Emoji: Grinning Face\"," +
		"\"\ufb33\":\"Hebrew Letter Dalet With Dagesh\"" +
		"}"
	if got := mustCanonicalize(t, input); got != want {
		t.Errorf("RFC 8785 §3.2.3\n got %s\nwant %s", got, want)
	}
}

// appendixB is RFC 8785 Appendix B, Table 1: IEEE 754 bit patterns and the
// ECMAScript text JCS requires for each. NaN and the infinities are left to
// TestNaNAndInfinitiesHaveNoCanonicalForm, because JSON text cannot carry them.
var appendixB = []struct {
	bits uint64
	want string
}{
	{0x0000000000000000, "0"},
	{0x8000000000000000, "0"},
	{0x0000000000000001, "5e-324"},
	{0x8000000000000001, "-5e-324"},
	{0x7fefffffffffffff, "1.7976931348623157e+308"},
	{0xffefffffffffffff, "-1.7976931348623157e+308"},
	{0x4340000000000000, "9007199254740992"},
	{0xc340000000000000, "-9007199254740992"},
	{0x4430000000000000, "295147905179352830000"},
	{0x44b52d02c7e14af5, "9.999999999999997e+22"},
	{0x44b52d02c7e14af6, "1e+23"},
	{0x44b52d02c7e14af7, "1.0000000000000001e+23"},
	{0x444b1ae4d6e2ef4e, "999999999999999700000"},
	{0x444b1ae4d6e2ef4f, "999999999999999900000"},
	{0x444b1ae4d6e2ef50, "1e+21"},
	{0x3eb0c6f7a0b5ed8c, "9.999999999999997e-7"},
	{0x3eb0c6f7a0b5ed8d, "0.000001"},
	{0x41b3de4355555553, "333333333.3333332"},
	{0x41b3de4355555554, "333333333.33333325"},
	{0x41b3de4355555555, "333333333.3333333"},
	{0x41b3de4355555556, "333333333.3333334"},
	{0x41b3de4355555557, "333333333.33333343"},
	{0xbecbf647612f3696, "-0.0000033333333333333333"},
	{0x43143ff3c1cb0959, "1424953923781206.2"},
}

func TestRFC8785AppendixBNumberSerialization(t *testing.T) {
	for _, c := range appendixB {
		got, err := FormatNumber(math.Float64frombits(c.bits))
		if err != nil {
			t.Errorf("%#016x: %v", c.bits, err)
			continue
		}
		if got != c.want {
			t.Errorf("RFC 8785 Appendix B pins %#016x as %s, got %s", c.bits, c.want, got)
		}
	}
}

// TestRFC8785AppendixBThroughTheParser runs the same table through
// Canonicalize, from text Go writes for each double. That covers the half
// FormatNumber alone cannot: that parsing the text lands on the same double.
func TestRFC8785AppendixBThroughTheParser(t *testing.T) {
	for _, c := range appendixB {
		text := strconv.FormatFloat(math.Float64frombits(c.bits), 'g', -1, 64)
		got := mustCanonicalize(t, `{"n":`+text+`}`)
		if want := `{"n":` + c.want + `}`; got != want {
			t.Errorf("%#016x written as %s\n got %s\nwant %s", c.bits, text, got, want)
		}
	}
}

// TestExponentThresholdsFollowECMAScript covers the boundaries on either side
// of each ECMAScript notation rule, which are exactly where strconv's own
// layout differs.
func TestExponentThresholdsFollowECMAScript(t *testing.T) {
	cases := []struct {
		in   float64
		want string
	}{
		{1, "1"},
		{-1, "-1"},
		{1234567, "1234567"},
		{1e20, "100000000000000000000"},
		{1.2345678901234568e20, "123456789012345680000"},
		{1e21, "1e+21"},
		{1.5e21, "1.5e+21"},
		{0.1, "0.1"},
		// The double 0.1 + 0.2 evaluates to at run time. Written as a literal
		// because Go folds the constant expression 0.1 + 0.2 exactly, to 0.3.
		{0.30000000000000004, "0.30000000000000004"},
		{0.00001, "0.00001"},
		{1e-6, "0.000001"},
		{1.5e-6, "0.0000015"},
		{1e-7, "1e-7"},
		{1.5e-7, "1.5e-7"},
		{0.82, "0.82"},
		{math.Copysign(0, -1), "0"},
	}
	for _, c := range cases {
		got, err := FormatNumber(c.in)
		if err != nil {
			t.Errorf("%v: %v", c.in, err)
			continue
		}
		if got != c.want {
			t.Errorf("FormatNumber(%v) = %s, ECMAScript writes %s", c.in, got, c.want)
		}
	}
}

func TestNaNAndInfinitiesHaveNoCanonicalForm(t *testing.T) {
	for _, f := range []float64{math.NaN(), math.Inf(1), math.Inf(-1)} {
		if _, err := FormatNumber(f); !errors.Is(err, ErrNotFinite) {
			t.Errorf("FormatNumber(%v): got %v, want ErrNotFinite", f, err)
		}
	}
}

func TestNumbersCanonicalizeToTheDoubleTheyDenote(t *testing.T) {
	cases := map[string]string{
		`[1.0]`:              `[1]`,
		`[-0]`:               `[0]`,
		`[-0.0e5]`:           `[0]`,
		`[1E2]`:              `[100]`,
		`[1e-400]`:           `[0]`,
		`[9007199254740993]`: `[9007199254740992]`,
		`[0.820]`:            `[0.82]`,
	}
	for in, want := range cases {
		if got := mustCanonicalize(t, in); got != want {
			t.Errorf("%s\n got %s\nwant %s", in, got, want)
		}
	}
}

func TestStringsEscapeOnlyWhatTheRFCNames(t *testing.T) {
	// C0 controls: the five short forms, and lowercase \u00xx for the rest.
	// '/', DEL and U+2028 are written as themselves.
	in := `["\u0000\u0008\u0009\u000a\u000c\u000d\u001f\u001F", "\/\u007f\u2028", "\"\\"]`
	want := `["\u0000\b\t\n\f\r\u001f\u001f","/` + "\u007f\u2028" + `","\"\\"]`
	if got := mustCanonicalize(t, in); got != want {
		t.Errorf("string escaping\n got %s\nwant %s", got, want)
	}
}

func TestMemberOrderAndWhitespaceDoNotChangeTheCanonicalForm(t *testing.T) {
	a := mustCanonicalize(t, `{"b":[1, 2, {"z":true,"a":null}],"a":"x"}`)
	b := mustCanonicalize(t, "\n{ \"a\" : \"x\" ,\t\"b\" : [ 1 ,2,{ \"a\":null, \"z\":true } ] }\r\n")
	if a != b {
		t.Errorf("two serializations of one value disagree\n%s\n%s", a, b)
	}
	if want := `{"a":"x","b":[1,2,{"a":null,"z":true}]}`; a != want {
		t.Errorf("got %s, want %s", a, want)
	}
}

func TestEmptyContainersAndScalarsAtTheTopLevel(t *testing.T) {
	for in, want := range map[string]string{
		`{}`: `{}`, `[]`: `[]`, ` { } `: `{}`, `"x"`: `"x"`,
		`true`: `true`, `null`: `null`, `-12.50`: `-12.5`,
	} {
		if got := mustCanonicalize(t, in); got != want {
			t.Errorf("%q: got %s, want %s", in, got, want)
		}
	}
}

// TestRefusalsAreNamed covers everything RFC 8785 and RFC 8259 make a
// canonicalizer refuse — each of which encoding/json would instead have
// accepted, repaired, or resolved silently.
func TestRefusalsAreNamed(t *testing.T) {
	cases := []struct {
		name  string
		input string
		want  error
	}{
		{"lone high surrogate", `["\ud800"]`, ErrLoneSurrogate},
		{"lone high surrogate at the end", `["\ud83d"]`, ErrLoneSurrogate},
		{"lone low surrogate", `["\ude00"]`, ErrLoneSurrogate},
		{"high surrogate then a non-surrogate", `["\ud83d\u0041"]`, ErrLoneSurrogate},
		{"lone surrogate in a member name", `{"\udc00":1}`, ErrLoneSurrogate},
		{"invalid UTF-8", "[\"\xff\"]", ErrInvalidUTF8},
		{"UTF-8-encoded surrogate", "[\"\xed\xa0\x80\"]", ErrInvalidUTF8},
		{"overlong UTF-8", "[\"\xc0\xaf\"]", ErrInvalidUTF8},
		{"duplicate member", `{"a":1,"a":2}`, ErrDuplicateMember},
		{"duplicate member spelled two ways", `{"a":1,"\u0061":2}`, ErrDuplicateMember},
		{"overflowing number", `[1e400]`, ErrNumberOutOfRange},
		{"leading zero", `[01]`, ErrSyntax},
		{"bare fraction point", `[1.]`, ErrSyntax},
		{"bare exponent", `[1e]`, ErrSyntax},
		{"leading plus", `[+1]`, ErrSyntax},
		{"trailing comma in an array", `[1,]`, ErrSyntax},
		{"trailing comma in an object", `{"a":1,}`, ErrSyntax},
		{"unquoted name", `{a:1}`, ErrSyntax},
		{"truncated literal", `[tru]`, ErrSyntax},
		{"raw control character", "[\"a\tb\"]", ErrSyntax},
		{"unknown escape", `["\x41"]`, ErrSyntax},
		{"trailing content", `{} {}`, ErrSyntax},
		{"byte order mark", "\ufeff{}", ErrSyntax},
		{"empty input", ``, ErrSyntax},
		{"unterminated string", `["abc`, ErrSyntax},
		{"NaN", `[NaN]`, ErrSyntax},
	}
	for _, c := range cases {
		_, err := Canonicalize([]byte(c.input))
		if !errors.Is(err, c.want) {
			t.Errorf("%s (%q): got %v, want %v", c.name, c.input, err, c.want)
		}
	}
}

func TestNestingIsBounded(t *testing.T) {
	deep := strings.Repeat("[", MaxDepth+1) + strings.Repeat("]", MaxDepth+1)
	if _, err := Canonicalize([]byte(deep)); !errors.Is(err, ErrTooDeep) {
		t.Errorf("%d levels: got %v, want ErrTooDeep", MaxDepth+1, err)
	}
	ok := strings.Repeat("[", MaxDepth) + strings.Repeat("]", MaxDepth)
	if _, err := Canonicalize([]byte(ok)); err != nil {
		t.Errorf("%d levels must be accepted: %v", MaxDepth, err)
	}
}

func TestCanonicalizeWithoutRemovesOnlyTheTopLevelMember(t *testing.T) {
	in := `{"record_hash":"sha256:top","b":{"record_hash":"sha256:nested"},"a":1}`
	got, err := CanonicalizeWithout([]byte(in), "record_hash")
	if err != nil {
		t.Fatal(err)
	}
	if want := `{"a":1,"b":{"record_hash":"sha256:nested"}}`; string(got) != want {
		t.Errorf("got %s, want %s", got, want)
	}

	absent, err := CanonicalizeWithout([]byte(`{"a":1,"b":{"record_hash":"sha256:nested"}}`), "record_hash")
	if err != nil {
		t.Fatal(err)
	}
	if string(absent) != string(got) {
		t.Errorf("removal, not blanking: an absent member must canonicalize identically\n%s\n%s", absent, got)
	}
}

func TestCanonicalizeWithoutStillRefusesADuplicatedOmittedMember(t *testing.T) {
	// Removing one copy and hashing the rest would let two parsers disagree
	// about which copy was the record's own.
	_, err := CanonicalizeWithout([]byte(`{"record_hash":"x","a":1,"record_hash":"y"}`), "record_hash")
	if !errors.Is(err, ErrDuplicateMember) {
		t.Errorf("got %v, want ErrDuplicateMember", err)
	}
}

func TestCanonicalizeWithoutNeedsAnObject(t *testing.T) {
	for _, in := range []string{`[1,2,3]`, `"record"`, ` null `} {
		if _, err := CanonicalizeWithout([]byte(in), "record_hash"); !errors.Is(err, ErrNotObject) {
			t.Errorf("%s: got %v, want ErrNotObject", in, err)
		}
	}
	// Malformed text is reported as malformed, not as the wrong shape.
	if _, err := CanonicalizeWithout([]byte(`[1,`), "record_hash"); !errors.Is(err, ErrSyntax) {
		t.Errorf("got %v, want ErrSyntax", err)
	}
}

func TestMarshalCanonicalizesAGoValue(t *testing.T) {
	got, err := Marshal(map[string]any{
		"b": []any{1e21, 0.000001, "<&>"},
		"a": true,
		"c": nil,
	})
	if err != nil {
		t.Fatal(err)
	}
	// encoding/json escapes <, & and > for HTML; the canonical form does not.
	if want := `{"a":true,"b":[1e+21,0.000001,"<&>"],"c":null}`; string(got) != want {
		t.Errorf("got %s, want %s", got, want)
	}
	if _, err := Marshal(math.NaN()); err == nil {
		t.Error("encoding/json refuses NaN, and so must Marshal")
	}
}
