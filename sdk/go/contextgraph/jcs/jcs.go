// Package jcs implements the JSON Canonicalization Scheme of RFC 8785: one
// byte sequence per JSON value, so two parties can hash or sign a document
// and agree on what they hashed.
//
// The record layer needs it. A lifecycle record's record_hash is SHA-256 over
// the JCS form of the record with its own record_hash member removed (profile
// LH1, ADR 0017), and attest.RecordHash is built on this package. The frame
// layer does not: a provenance link is six optional strings with a typed
// encoding of its own (SPEC.md §6.5.1, ADR 0010), and nothing on that path
// touches JSON.
//
// # Why a parser of its own rather than encoding/json
//
// encoding/json is the wrong front end for a canonicalizer, in three ways that
// each produce a different hash rather than an error:
//
//   - It decodes numbers to float64 by default, and with UseNumber hands back
//     the source text, which is not canonical either. JCS needs the IEEE 754
//     double the text denotes, formatted the ECMAScript way. This package
//     parses the text itself and converts it exactly once, with
//     strconv.ParseFloat, which rounds correctly.
//   - It replaces a lone UTF-16 surrogate escape ("\ud800") and invalid UTF-8
//     with U+FFFD without saying so. RFC 8785 §3.2.2.2 requires a
//     canonicalizer to refuse both, and the Rust reference does.
//   - It keeps the last of two members with the same name. RFC 8785 §3.1
//     requires I-JSON input (RFC 7493 §2.3: member names MUST be unique), and a
//     document that two parsers read two ways is a parser-differential waiting
//     to be exploited, so this package refuses it with [ErrDuplicateMember].
//
// So [Canonicalize] reads JSON text with a small strict parser and writes the
// canonical form in the same pass. [Marshal] is the convenience for a Go
// value: it encodes with encoding/json, then canonicalizes those bytes.
//
// # Numbers
//
// [FormatNumber] is ECMAScript's Number::toString, which RFC 8785 §3.2.2.3
// adopts. strconv.FormatFloat alone is not: it switches to exponent notation
// at different thresholds and pads the exponent to two digits, so
// FormatFloat(f, 'g', -1, 64) writes 1234567 as "1.234567e+06" and 0.00001 as
// "1e-05" where ECMAScript writes "1234567" and "0.00001". Go's shortest
// round-trip digits are the right digits; only the layout around them is
// rewritten here, and jcs_test.go pins the RFC's Appendix B table against it.
package jcs

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"math"
	"sort"
	"strconv"
	"strings"
	"unicode/utf16"
	"unicode/utf8"
)

// The named reasons a document is refused. Each is wrapped with the byte
// offset it was found at, so errors.Is works and the message still says where.
var (
	// ErrSyntax is JSON text that RFC 8259 does not admit.
	ErrSyntax = errors.New("jcs: invalid JSON")
	// ErrInvalidUTF8 is a string holding bytes that are not UTF-8.
	ErrInvalidUTF8 = errors.New("jcs: string is not valid UTF-8")
	// ErrLoneSurrogate is a \u escape naming half of a UTF-16 surrogate pair
	// with the other half missing (RFC 8785 §3.2.2.2).
	ErrLoneSurrogate = errors.New("jcs: lone UTF-16 surrogate")
	// ErrDuplicateMember is an object naming the same member twice (RFC 7493
	// §2.3, required by RFC 8785 §3.1).
	ErrDuplicateMember = errors.New("jcs: duplicate object member")
	// ErrNumberOutOfRange is a number whose magnitude no IEEE 754 double holds.
	ErrNumberOutOfRange = errors.New("jcs: number is out of IEEE 754 double range")
	// ErrNotFinite is NaN or an infinity handed to [FormatNumber]. JSON text
	// cannot spell either, but a Go float64 can hold both.
	ErrNotFinite = errors.New("jcs: NaN and infinities have no JSON form")
	// ErrTooDeep is nesting deeper than [MaxDepth].
	ErrTooDeep = errors.New("jcs: document nests too deeply")
	// ErrNotObject is a document whose top level is not an object, handed to
	// [CanonicalizeWithout].
	ErrNotObject = errors.New("jcs: top-level value is not an object")
)

// MaxDepth bounds how deeply arrays and objects may nest.
//
// A lifecycle record nests a handful of levels. The bound exists so hostile
// input costs bounded work, and it sits well above the recursion limit of the
// reference implementation's parser, so nothing the reference canonicalizes is
// refused here.
const MaxDepth = 1000

// Canonicalize returns the RFC 8785 canonical form of the JSON text in data.
func Canonicalize(data []byte) ([]byte, error) {
	return canonicalize(data, "", false)
}

// CanonicalizeWithout returns the RFC 8785 canonical form of the JSON object in
// data with its top-level member named member removed, and [ErrNotObject] if
// data is not an object.
//
// Only the top level is touched: a member of the same name nested anywhere
// below stays, because it is ordinary content. The member is removed after the
// duplicate check, so a document naming it twice is still refused rather than
// having one copy silently hashed away.
//
// This is the shape a self-describing content address needs — profile LH1's
// record_hash is SHA-256 over CanonicalizeWithout(record, "record_hash").
func CanonicalizeWithout(data []byte, member string) ([]byte, error) {
	return canonicalize(data, member, true)
}

// Marshal returns the RFC 8785 canonical form of v, encoded with
// encoding/json first.
//
// Everything encoding/json does on the way in still happens: struct tags and
// omitempty apply, and a Go string holding invalid UTF-8 is coerced to U+FFFD
// by encoding/json before this package sees it. Hand [Canonicalize] the bytes
// you received when you have them; that is the faithful path.
func Marshal(v any) ([]byte, error) {
	raw, err := json.Marshal(v)
	if err != nil {
		return nil, err
	}
	return Canonicalize(raw)
}

func canonicalize(data []byte, omit string, requireObject bool) ([]byte, error) {
	p := parser{data: data}
	p.skipWhitespace()
	if requireObject && (p.pos >= len(p.data) || p.data[p.pos] != '{') {
		// Read the value anyway, so malformed text is reported as malformed
		// rather than as well-formed JSON of the wrong shape.
		var discard bytes.Buffer
		if err := p.value(&discard, 0); err != nil {
			return nil, err
		}
		return nil, ErrNotObject
	}
	var out bytes.Buffer
	if requireObject {
		if err := p.object(&out, 0, omit, true); err != nil {
			return nil, err
		}
	} else if err := p.value(&out, 0); err != nil {
		return nil, err
	}
	p.skipWhitespace()
	if p.pos != len(p.data) {
		return nil, p.fail(ErrSyntax, "trailing content after the top-level value")
	}
	return out.Bytes(), nil
}

// parser is a strict RFC 8259 reader that writes each value's canonical form
// as it goes. Objects are the one place it buffers: members are canonicalized
// one by one, then sorted and written.
type parser struct {
	data []byte
	pos  int
}

func (p *parser) fail(kind error, detail string) error {
	return fmt.Errorf("%w at offset %d: %s", kind, p.pos, detail)
}

func (p *parser) skipWhitespace() {
	for p.pos < len(p.data) {
		switch p.data[p.pos] {
		case ' ', '\t', '\n', '\r':
			p.pos++
		default:
			return
		}
	}
}

func (p *parser) value(out *bytes.Buffer, depth int) error {
	p.skipWhitespace()
	if p.pos >= len(p.data) {
		return p.fail(ErrSyntax, "unexpected end of input")
	}
	switch c := p.data[p.pos]; {
	case c == '{':
		return p.object(out, depth, "", false)
	case c == '[':
		return p.array(out, depth)
	case c == '"':
		s, err := p.readString()
		if err != nil {
			return err
		}
		writeString(out, s)
		return nil
	case c == '-' || (c >= '0' && c <= '9'):
		return p.number(out)
	default:
		for _, literal := range []string{"true", "false", "null"} {
			if bytes.HasPrefix(p.data[p.pos:], []byte(literal)) {
				p.pos += len(literal)
				out.WriteString(literal)
				return nil
			}
		}
		return p.fail(ErrSyntax, fmt.Sprintf("unexpected byte %q", c))
	}
}

// member is one object member: its name, and its value already in canonical
// form.
type member struct {
	name  string
	units []uint16
	value []byte
}

// object reads an object starting at '{'. omit names a member to drop, and is
// honoured only when topLevel is set.
func (p *parser) object(out *bytes.Buffer, depth int, omit string, topLevel bool) error {
	if depth >= MaxDepth {
		return p.fail(ErrTooDeep, fmt.Sprintf("more than %d levels", MaxDepth))
	}
	p.pos++ // '{'
	var members []member
	seen := make(map[string]struct{})
	p.skipWhitespace()
	if p.pos < len(p.data) && p.data[p.pos] == '}' {
		p.pos++
		out.WriteString("{}")
		return nil
	}
	for {
		p.skipWhitespace()
		if p.pos >= len(p.data) || p.data[p.pos] != '"' {
			return p.fail(ErrSyntax, "expected a member name")
		}
		name, err := p.readString()
		if err != nil {
			return err
		}
		if _, dup := seen[name]; dup {
			return p.fail(ErrDuplicateMember, strconv.Quote(name))
		}
		seen[name] = struct{}{}
		p.skipWhitespace()
		if p.pos >= len(p.data) || p.data[p.pos] != ':' {
			return p.fail(ErrSyntax, "expected ':' after a member name")
		}
		p.pos++
		var value bytes.Buffer
		if err := p.value(&value, depth+1); err != nil {
			return err
		}
		if !(topLevel && name == omit) {
			members = append(members, member{
				name:  name,
				units: utf16.Encode([]rune(name)),
				value: value.Bytes(),
			})
		}
		p.skipWhitespace()
		if p.pos >= len(p.data) {
			return p.fail(ErrSyntax, "unterminated object")
		}
		if p.data[p.pos] == ',' {
			p.pos++
			continue
		}
		if p.data[p.pos] == '}' {
			p.pos++
			break
		}
		return p.fail(ErrSyntax, "expected ',' or '}' in an object")
	}

	// RFC 8785 §3.2.3: sort by the member names' UTF-16 code units, compared
	// as unsigned integers. Not by UTF-8 bytes and not by code point — the two
	// differ from UTF-16 order for any name holding a character above U+FFFF
	// beside one in U+E000–U+FFFF.
	sort.Slice(members, func(i, j int) bool {
		return lessUTF16(members[i].units, members[j].units)
	})
	out.WriteByte('{')
	for i, m := range members {
		if i > 0 {
			out.WriteByte(',')
		}
		writeString(out, m.name)
		out.WriteByte(':')
		out.Write(m.value)
	}
	out.WriteByte('}')
	return nil
}

func lessUTF16(a, b []uint16) bool {
	for i := 0; i < len(a) && i < len(b); i++ {
		if a[i] != b[i] {
			return a[i] < b[i]
		}
	}
	return len(a) < len(b)
}

func (p *parser) array(out *bytes.Buffer, depth int) error {
	if depth >= MaxDepth {
		return p.fail(ErrTooDeep, fmt.Sprintf("more than %d levels", MaxDepth))
	}
	p.pos++ // '['
	out.WriteByte('[')
	p.skipWhitespace()
	if p.pos < len(p.data) && p.data[p.pos] == ']' {
		p.pos++
		out.WriteByte(']')
		return nil
	}
	for first := true; ; first = false {
		if !first {
			out.WriteByte(',')
		}
		if err := p.value(out, depth+1); err != nil {
			return err
		}
		p.skipWhitespace()
		if p.pos >= len(p.data) {
			return p.fail(ErrSyntax, "unterminated array")
		}
		if p.data[p.pos] == ',' {
			p.pos++
			continue
		}
		if p.data[p.pos] == ']' {
			p.pos++
			out.WriteByte(']')
			return nil
		}
		return p.fail(ErrSyntax, "expected ',' or ']' in an array")
	}
}

// readString reads a string starting at '"' and returns its decoded value.
func (p *parser) readString() (string, error) {
	p.pos++ // opening quote
	var sb strings.Builder
	for {
		if p.pos >= len(p.data) {
			return "", p.fail(ErrSyntax, "unterminated string")
		}
		c := p.data[p.pos]
		switch {
		case c == '"':
			p.pos++
			return sb.String(), nil
		case c == '\\':
			r, err := p.escape()
			if err != nil {
				return "", err
			}
			sb.WriteRune(r)
		case c < 0x20:
			return "", p.fail(ErrSyntax, "unescaped control character in a string")
		case c < utf8.RuneSelf:
			sb.WriteByte(c)
			p.pos++
		default:
			// utf8.DecodeRune reports an encoded surrogate (ED A0..BF xx), an
			// overlong form, and a truncated sequence alike as (RuneError, 1),
			// which is exactly the set RFC 3629 excludes.
			r, size := utf8.DecodeRune(p.data[p.pos:])
			if r == utf8.RuneError && size == 1 {
				return "", p.fail(ErrInvalidUTF8, fmt.Sprintf("byte 0x%02x", c))
			}
			sb.Write(p.data[p.pos : p.pos+size])
			p.pos += size
		}
	}
}

// escape reads one escape sequence starting at '\' and returns the character
// it denotes, joining a surrogate pair into one rune.
func (p *parser) escape() (rune, error) {
	if p.pos+1 >= len(p.data) {
		return 0, p.fail(ErrSyntax, "unterminated escape")
	}
	c := p.data[p.pos+1]
	p.pos += 2
	switch c {
	case '"':
		return '"', nil
	case '\\':
		return '\\', nil
	case '/':
		return '/', nil
	case 'b':
		return '\b', nil
	case 'f':
		return '\f', nil
	case 'n':
		return '\n', nil
	case 'r':
		return '\r', nil
	case 't':
		return '\t', nil
	case 'u':
	default:
		return 0, p.fail(ErrSyntax, fmt.Sprintf("unknown escape \\%c", c))
	}
	first, err := p.hex4()
	if err != nil {
		return 0, err
	}
	switch {
	case first >= 0xDC00 && first <= 0xDFFF:
		return 0, p.fail(ErrLoneSurrogate, fmt.Sprintf("low surrogate \\u%04x with no high surrogate before it", first))
	case first >= 0xD800 && first <= 0xDBFF:
		if p.pos+1 >= len(p.data) || p.data[p.pos] != '\\' || p.data[p.pos+1] != 'u' {
			return 0, p.fail(ErrLoneSurrogate, fmt.Sprintf("high surrogate \\u%04x with no low surrogate after it", first))
		}
		p.pos += 2
		second, err := p.hex4()
		if err != nil {
			return 0, err
		}
		if second < 0xDC00 || second > 0xDFFF {
			return 0, p.fail(ErrLoneSurrogate, fmt.Sprintf("high surrogate \\u%04x followed by \\u%04x", first, second))
		}
		return utf16.DecodeRune(rune(first), rune(second)), nil
	default:
		return rune(first), nil
	}
}

func (p *parser) hex4() (uint16, error) {
	if p.pos+4 > len(p.data) {
		return 0, p.fail(ErrSyntax, "truncated \\u escape")
	}
	var v uint16
	for _, c := range p.data[p.pos : p.pos+4] {
		v <<= 4
		switch {
		case c >= '0' && c <= '9':
			v |= uint16(c - '0')
		case c >= 'a' && c <= 'f':
			v |= uint16(c-'a') + 10
		case c >= 'A' && c <= 'F':
			v |= uint16(c-'A') + 10
		default:
			return 0, p.fail(ErrSyntax, "non-hex digit in a \\u escape")
		}
	}
	p.pos += 4
	return v, nil
}

// number reads a number per the RFC 8259 grammar and writes its canonical
// form: the IEEE 754 double the text denotes, in ECMAScript notation.
func (p *parser) number(out *bytes.Buffer) error {
	start := p.pos
	digits := func() int {
		n := 0
		for p.pos < len(p.data) && p.data[p.pos] >= '0' && p.data[p.pos] <= '9' {
			p.pos++
			n++
		}
		return n
	}
	if p.data[p.pos] == '-' {
		p.pos++
	}
	if p.pos < len(p.data) && p.data[p.pos] == '0' {
		// A leading zero stands alone: "01" is not JSON.
		p.pos++
	} else if digits() == 0 {
		return p.fail(ErrSyntax, "a number needs an integer part")
	}
	if p.pos < len(p.data) && p.data[p.pos] == '.' {
		p.pos++
		if digits() == 0 {
			return p.fail(ErrSyntax, "a fraction needs a digit")
		}
	}
	if p.pos < len(p.data) && (p.data[p.pos] == 'e' || p.data[p.pos] == 'E') {
		p.pos++
		if p.pos < len(p.data) && (p.data[p.pos] == '+' || p.data[p.pos] == '-') {
			p.pos++
		}
		if digits() == 0 {
			return p.fail(ErrSyntax, "an exponent needs a digit")
		}
	}
	text := string(p.data[start:p.pos])
	// ParseFloat rounds correctly (round half to even on the exact decimal
	// value), which is what "the double this text denotes" means. It reports
	// overflow as ErrRange with an infinity; an underflow rounds to zero, as
	// every IEEE 754 parser does.
	f, err := strconv.ParseFloat(text, 64)
	if err != nil {
		p.pos = start
		return p.fail(ErrNumberOutOfRange, text)
	}
	formatted, err := FormatNumber(f)
	if err != nil {
		p.pos = start
		return p.fail(err, text)
	}
	out.WriteString(formatted)
	return nil
}

// writeString writes s in RFC 8785 §3.2.2.2 form: quoted, with '"', '\' and
// the C0 controls escaped, and every other character — including '/', DEL and
// U+2028 — written as its own UTF-8 bytes.
func writeString(out *bytes.Buffer, s string) {
	const hexDigits = "0123456789abcdef"
	out.WriteByte('"')
	for i := 0; i < len(s); i++ {
		c := s[i]
		switch c {
		case '"':
			out.WriteString(`\"`)
		case '\\':
			out.WriteString(`\\`)
		case '\b':
			out.WriteString(`\b`)
		case '\t':
			out.WriteString(`\t`)
		case '\n':
			out.WriteString(`\n`)
		case '\f':
			out.WriteString(`\f`)
		case '\r':
			out.WriteString(`\r`)
		default:
			if c < 0x20 {
				// Lowercase hex, which is the spelling the RFC requires.
				out.WriteString(`\u00`)
				out.WriteByte(hexDigits[c>>4])
				out.WriteByte(hexDigits[c&0x0f])
			} else {
				out.WriteByte(c)
			}
		}
	}
	out.WriteByte('"')
}

// FormatNumber returns ECMAScript's Number::toString(f), the number
// serialization RFC 8785 §3.2.2.3 requires, or [ErrNotFinite] for NaN and the
// infinities.
//
// The algorithm is ECMA-262 §6.1.6.1.20 (Number::toString) step for step. Let
// the shortest decimal digits that round-trip to f be s, k of them, with f =
// s × 10^(n−k). Then:
//
//   - k ≤ n ≤ 21: the digits, then n−k zeros ("100000000000000000000")
//   - 0 < n ≤ 21: the first n digits, '.', the rest ("333333333.3333333")
//   - −6 < n ≤ 0: "0.", −n zeros, the digits ("0.000001")
//   - otherwise: exponent notation, one digit before the point, the exponent
//     unpadded with an explicit sign ("1e+21", "9.999999999999997e-7")
//
// Negative zero is "0". strconv supplies s and n: FormatFloat with format 'e'
// and precision -1 is the shortest representation that round-trips, which is
// the s ECMAScript requires.
func FormatNumber(f float64) (string, error) {
	if math.IsNaN(f) || math.IsInf(f, 0) {
		return "", ErrNotFinite
	}
	if f == 0 {
		return "0", nil
	}
	sign := ""
	if f < 0 {
		sign = "-"
		f = -f
	}
	// "d.ddde±XX", or "de±XX" for a single digit.
	scientific := strconv.FormatFloat(f, 'e', -1, 64)
	mantissa, exponentText, found := strings.Cut(scientific, "e")
	if !found {
		// Unreachable: format 'e' always writes an exponent.
		panic("jcs: strconv.FormatFloat(f, 'e', -1, 64) wrote no exponent: " + scientific)
	}
	exponent, err := strconv.Atoi(exponentText)
	if err != nil {
		panic("jcs: strconv.FormatFloat wrote an unreadable exponent: " + scientific)
	}
	digits := strings.Replace(mantissa, ".", "", 1)
	k := len(digits)
	n := exponent + 1

	var b strings.Builder
	b.WriteString(sign)
	switch {
	case k <= n && n <= 21:
		b.WriteString(digits)
		b.WriteString(strings.Repeat("0", n-k))
	case 0 < n && n <= 21:
		b.WriteString(digits[:n])
		b.WriteByte('.')
		b.WriteString(digits[n:])
	case -6 < n && n <= 0:
		b.WriteString("0.")
		b.WriteString(strings.Repeat("0", -n))
		b.WriteString(digits)
	default:
		b.WriteByte(digits[0])
		if k > 1 {
			b.WriteByte('.')
			b.WriteString(digits[1:])
		}
		b.WriteByte('e')
		if n-1 >= 0 {
			b.WriteByte('+')
		} else {
			b.WriteByte('-')
		}
		e := n - 1
		if e < 0 {
			e = -e
		}
		b.WriteString(strconv.Itoa(e))
	}
	return b.String(), nil
}
