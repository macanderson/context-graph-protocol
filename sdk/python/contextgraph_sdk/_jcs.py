"""RFC 8785 JSON Canonicalization Scheme (JCS), in the standard library alone.

Why this is not ``json.dumps(sort_keys=True, separators=(",", ":"))``
---------------------------------------------------------------------

That one-liner is the canonicalizer everybody writes first, and it disagrees
with RFC 8785 in three places, each of which changes a ``record_hash``:

1. **Numbers.** JCS serializes every number as ECMAScript's
   ``Number::toString`` would (RFC 8785 §3.2.2.3). Python's ``repr`` picks the
   same shortest round-trip *digits* but lays them out differently: it
   switches to exponent form at ``1e16`` where ECMAScript waits for ``1e21``,
   it writes ``1e-06`` where ECMAScript writes ``0.000001``, it pads the
   exponent (``e-07`` rather than ``e-7``), and it keeps a trailing ``.0``.
   And an ``int`` is not a JSON number to JCS at all until it has been through
   an IEEE 754 double, so ``2**53 + 1`` serializes as ``9007199254740992``.
   :func:`_number` takes ``repr``'s digits and re-lays them out by the
   ECMAScript algorithm; ``tests/test_record.py`` pins it against RFC 8785
   Appendix B's table, which exists precisely because these thresholds are
   where implementations drift.
2. **Member order.** JCS sorts property names by their **UTF-16 code units**
   (§3.2.3); Python sorts ``str`` by code point. The two orders differ exactly
   when a name holds a character above U+FFFF next to one in
   U+E000–U+FFFF: the supplementary character's leading surrogate (0xD8xx)
   sorts *below* U+FB33, although its code point is far above it.
3. **Strings.** JCS escapes only ``"``, ``\\`` and the C0 controls, and those
   with the short forms where JSON has them (§3.2.2.2); everything else,
   U+007F and U+2028 included, is written as literal UTF-8. ``json.dumps``
   escapes all non-ASCII as ``\\uXXXX`` unless told otherwise, and with
   ``ensure_ascii=False`` it passes a lone surrogate straight through.

A document JCS must refuse
--------------------------

``NaN`` and the infinities have no JSON spelling, and a lone surrogate has no
UTF-8 one; RFC 8785 requires a canonicalizer to reject all three rather than
invent a spelling. Python produces every one of them happily (``float("nan")``,
``json.loads('"\\ud800"')``), so :func:`canonicalize` checks and raises
:class:`CanonicalizationError`.

Values are the ones :func:`json.loads` returns — ``dict``, ``list``, ``str``,
``int``, ``float``, ``bool``, ``None`` — plus any ``Mapping`` or ``tuple`` a
caller built by hand. Anything else is refused rather than guessed at.
"""

from __future__ import annotations

import math
from typing import Any, List, Mapping, Tuple, Union

__all__ = ["CanonicalizationError", "canonicalize"]


class CanonicalizationError(ValueError):
    """The value has no RFC 8785 canonical form.

    A real finding about the value, not a library hiccup: JCS *must* refuse a
    non-finite number or a lone surrogate (RFC 8785 §3.2.2.2, §3.2.2.3), and a
    value that is not JSON at all has nothing to canonicalize.
    """


# The escapes RFC 8785 §3.2.2.2 prescribes, and no others. Every other C0
# control becomes \u00xx in *lowercase* hex; nothing above U+001F is escaped.
_SHORT_ESCAPES = {
    0x08: "\\b",
    0x09: "\\t",
    0x0A: "\\n",
    0x0C: "\\f",
    0x0D: "\\r",
    0x22: '\\"',
    0x5C: "\\\\",
}


def canonicalize(value: Any) -> bytes:
    """The RFC 8785 canonical UTF-8 bytes of a JSON value.

    :raises CanonicalizationError: the value holds a non-finite number, a lone
        surrogate, a non-string member name, or something that is not JSON, or
        it nests deeper than the interpreter's recursion limit allows.
    """
    out: List[str] = []
    try:
        _write(value, out)
    except RecursionError as error:
        # A decoded document can sit just under ``json.loads``'s own depth
        # limit and still exhaust the stack here, where each object level
        # costs two frames. A named refusal, not a bare interpreter error, is
        # what a caller hashing untrusted wire records can act on.
        raise CanonicalizationError(
            "the value nests too deeply to canonicalize"
        ) from error
    try:
        return "".join(out).encode("utf-8")
    except UnicodeEncodeError as error:
        # Only a lone surrogate reaches here: every string was already sorted
        # by UTF-16 (which catches one in a member name) but not encoded.
        raise CanonicalizationError(
            f"a string holds a lone surrogate, which RFC 8785 §3.2.2.2 "
            f"requires a canonicalizer to refuse: {error}"
        ) from error


def _write(value: Any, out: List[str]) -> None:
    # bool before int: True is an int to Python and a literal to JSON.
    if value is None:
        out.append("null")
    elif value is True:
        out.append("true")
    elif value is False:
        out.append("false")
    elif isinstance(value, str):
        out.append(_string(value))
    elif isinstance(value, (int, float)):
        out.append(_number(value))
    elif isinstance(value, Mapping):
        _write_object(value, out)
    elif isinstance(value, (list, tuple)):
        out.append("[")
        for index, item in enumerate(value):
            if index:
                out.append(",")
            _write(item, out)
        out.append("]")
    else:
        raise CanonicalizationError(
            f"{type(value).__name__} is not a JSON value and has no RFC 8785 form"
        )


def _utf16_key(name: str) -> bytes:
    """A sort key that orders strings by UTF-16 code unit (RFC 8785 §3.2.3).

    Big-endian UTF-16 compares bytewise in exactly code-unit order, because each
    unit is two bytes with the high byte first. ``surrogatepass`` is *not*
    used: a lone surrogate in a member name is refused here, as it must be.
    """
    try:
        return name.encode("utf-16-be")
    except UnicodeEncodeError as error:
        raise CanonicalizationError(
            f"a member name holds a lone surrogate, which RFC 8785 requires a "
            f"canonicalizer to refuse: {error}"
        ) from error


def _write_object(obj: Mapping[Any, Any], out: List[str]) -> None:
    members: List[Tuple[bytes, str, Any]] = []
    for name, member in obj.items():
        if not isinstance(name, str):
            raise CanonicalizationError(
                f"a JSON member name must be a string, not {type(name).__name__}"
            )
        members.append((_utf16_key(name), name, member))
    # Distinct str keys have distinct UTF-16 encodings, so the sort never
    # compares the values in the tuple's third slot.
    members.sort(key=lambda entry: entry[0])
    out.append("{")
    for index, (_, name, member) in enumerate(members):
        if index:
            out.append(",")
        out.append(_string(name))
        out.append(":")
        _write(member, out)
    out.append("}")


def _string(text: str) -> str:
    """A JSON string literal escaped exactly as RFC 8785 §3.2.2.2 prescribes."""
    parts = ['"']
    for char in text:
        code = ord(char)
        short = _SHORT_ESCAPES.get(code)
        if short is not None:
            parts.append(short)
        elif code < 0x20:
            parts.append("\\u%04x" % code)
        else:
            parts.append(char)
    parts.append('"')
    return "".join(parts)


def _number(value: Union[int, float]) -> str:
    """ECMAScript ``Number::toString(value)`` for a finite IEEE 754 double.

    ECMA-262 §6.1.6.1.20 (``Number::toString``, radix 10), which RFC 8785
    §3.2.2.3 adopts. With ``k`` significant digits ``s`` and the value equal to
    ``0.s × 10^n``:

    - ``k ≤ n ≤ 21``: the digits, then ``n - k`` zeros (``1e20`` →
      ``100000000000000000000``);
    - ``0 < n ≤ 21``: a decimal point inside the digits;
    - ``-6 < n ≤ 0``: ``0.``, then ``-n`` zeros, then the digits
      (``1e-6`` → ``0.000001``);
    - otherwise exponent form ``d[.ddd]e±x``, with no padding on ``x``
      (``1e21`` → ``1e+21``, ``1e-7`` → ``1e-7``).

    The digits themselves are Python's ``repr`` digits. ``repr`` returns the
    shortest decimal string that round-trips to the same double, choosing the
    one nearest the exact value when several are equally short — which is the
    digit string ECMA-262 specifies. Only the layout differs, and the layout is
    all this function rewrites.
    """
    # ``float(int)`` rounds to nearest-even exactly as a JSON parser producing a
    # double would, and raises OverflowError past the double range — the same
    # refusal as an infinity, because that is what such a parser would produce.
    try:
        number = float(value)
    except OverflowError:
        number = math.inf
    if not math.isfinite(number):
        raise CanonicalizationError(
            f"{value!r} is not a finite number; RFC 8785 §3.2.2.3 requires a "
            "canonicalizer to refuse NaN and the infinities"
        )
    if number == 0:
        return "0"  # -0 included: ECMAScript prints both zeros as "0".
    if number < 0:
        return "-" + _number(-number)

    # ``float.__repr__`` rather than ``repr``: a float subclass may override
    # its repr, and only the builtin's shortest round-trip digits are wanted.
    text = float.__repr__(number)
    mantissa, _, exponent_text = text.partition("e")
    exponent = int(exponent_text) if exponent_text else 0
    whole, _, fraction = mantissa.partition(".")
    digits = whole + fraction
    # The decimal point sits after ``point`` digits: value = 0.digits × 10^point.
    point = len(whole) + exponent
    stripped = digits.lstrip("0")
    point -= len(digits) - len(stripped)
    digits = stripped.rstrip("0")
    k = len(digits)
    n = point

    if k <= n <= 21:
        return digits + "0" * (n - k)
    if 0 < n <= 21:
        return digits[:n] + "." + digits[n:]
    if -6 < n <= 0:
        return "0." + "0" * (-n) + digits
    e = n - 1
    sign = "+" if e >= 0 else "-"
    head = digits if k == 1 else digits[0] + "." + digits[1:]
    return head + "e" + sign + str(abs(e))
