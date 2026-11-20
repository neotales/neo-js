---
title: "Text utilities"
description: "Unicode characters, string building, and ANSI style helpers."
---

`neo:chars`, `neo:string-builder`, and `neo:ansi` are synchronous, pure
JavaScript modules. They need no permission flag and grant no host capability:
they do not read files, environment variables, process arguments, or terminal
state, and they install nothing on the global object.

## Characters

`neo:chars` classifies Unicode code points and offers a few code-point
operations. Predicates take a code point number, never a one-character string,
and return `false` for anything that is not a Unicode scalar value: a
non-number, a non-integer, a value outside 0 through 0x10FFFF, or a surrogate
code point. Code-point predicates never throw; the `...At` variants validate
their arguments as described below.

```js
import {
	equalFold,
	isAscii,
	isDigit,
	isLetter,
	isSpace,
	isSymbol,
	toLower,
	toUpper,
} from "neo:chars";

isAscii("A".codePointAt(0)); // true
isAscii("é".codePointAt(0)); // false
isLetter("Ж".codePointAt(0)); // true
isDigit("٥".codePointAt(0)); // true (Arabic-Indic five)
isSpace(0x3000); // true (ideographic space)
isSymbol("😀".codePointAt(0)); // true
isLetter("😀".codePointAt(0)); // false
toLower(0x41); // 0x61, "A" to "a"
equalFold(0x3c2, 0x3c3); // true (final sigma and sigma)
```

`isAscii` and `isLatin1` are code point ranges. The other predicates follow the
host engine's Unicode property tables, so a different QuickJS build may
classify a newly assigned code point differently. `isDigit` means Unicode
category `Nd`; superscript and other numerals are not digits. `isSpace` covers
ASCII tab through carriage return, U+0085, and the Unicode separator
categories; U+200B ZERO WIDTH SPACE is not whitespace.

The `...At` functions, such as `isLetterAt`, `isSpaceAt`, and `isAsciiAt`, take
a UTF-16 code unit index, like `String.prototype.codePointAt`. An index in the
middle of a surrogate pair reads the trailing surrogate, which is not a scalar
value, so every predicate returns `false`. An out-of-range index also returns
`false`; a non-string text or a non-integer or negative index throws
`TypeError`.

`toLower` and `toUpper` apply the host mapping only when it produces exactly
one code point. A full mapping that expands, such as `ß` to `SS`, leaves the
input unchanged. `equalFold` compares through the host engine's Unicode simple
case-fold canonicalization, so the relation is reflexive, symmetric, and
transitive; the Greek theta class U+0398, U+03B8, U+03D1, and U+03F4 is one
class. It is not Go's `unicode.SimpleFold` orbit and not full case folding: an
expanding mapping is not applied, so `ß` (0xDF) and `s` (0x73) compare as
different. The engine's Unicode version decides which pairs fold. This module
is not a port of `@neotales/chars` and ships no Go lookup tables.

## StringBuilder

`neo:string-builder` exports a `StringBuilder` class. `append` accepts a
primitive string, number, boolean, or bigint and converts the last three with
`String(...)`; `undefined`, `null`, symbols, objects, and boxed primitives
throw `TypeError` rather than being coerced. `appendCodePoint` accepts only a
Unicode scalar value, so it rejects surrogate code points. `clear` empties the
builder and leaves it usable.

```js
import { StringBuilder } from "neo:string-builder";

const builder = new StringBuilder();
builder.append("Hello").append(", ").append(42).append("!");
builder.toString(); // "Hello, 42!"
builder.length; // 10, UTF-16 code units
builder.clear().append("again").toString(); // "again"
```

`length` is the number of UTF-16 code units, matching `String.prototype.length`;
`codePointLength` counts Unicode code points, so `appendCodePoint(0x1f600)`
adds two code units and one code point. `.string` is an alias for
`toString()`. The builder stores UTF-16 strings and joins them lazily. It is
not `@neotales/slices`' `CharArrayBuilder`: there is no code-point buffer, no
`appendSlice` or `appendCharArray`, no `Date` support, and no capacity
argument; the internal chunk array grows as text is appended.

## ANSI styles

`neo:ansi` exports style functions for text attributes, 16 foreground and 16
background colors, 256-color, and 24-bit color. Color is on by default and is
never detected: no environment variable, process argument, or terminal check
changes the output, and there is no module-global toggle. Pass
`{ color: false }` to return the input unchanged.

```js
import { apply, bgBlue, blue, bold, red, rgb24, stripAnsi } from "neo:ansi";

red("error"); // "\x1b[31merror\x1b[39m"
red("error", { color: false }); // "error"
apply("notice", [bold, blue]); // blue wraps bold; the last style is outermost
rgb24("warm", 0xff8c00); // SGR 38;2;255;140;0
stripAnsi("\x1b[31merror\x1b[39m"); // "error"
```

Each style wraps a value as `open + value + close`. If the exact closing
sequence for that style appears inside the value, it is replaced with the
opening sequence. Only that exact sequence is handled: a full reset
(`ESC[0m`), another style's reset, or any other SGR sequence in the value is
passed through and can end or change the style early. Styling does not
sanitize or normalize escape sequences. `apply` takes an array of style
functions and applies them left to right; an empty array returns the value
unchanged.

`stripAnsi` removes CSI sequences introduced by `ESC [` or the single-byte C1
control U+009B, OSC strings (including OSC-8 hyperlinks), DCS/SOS/PM/APC
strings, ESC c (RIS), and two-byte escape sequences whose second byte is 0x40
through 0x5F. A control string ends at BEL, ST (`ESC \`), or the single-byte
C1 ST U+009C; text after the terminator is retained. Other text is preserved
exactly, including lone escape bytes and all other control characters.

`stripAnsi` is best-effort cleanup, not a security sanitizer: terminal escape
protocols are larger than this scanner, an unterminated sequence is removed
through the end of the input, and malformed or unhandled sequences may remain.
Do not rely on it alone to make untrusted text safe to print.

Deliberate differences from `@neotales/ansi`: this module has no
`detectMode`, `AnsiSettings`, or `setColorEnabled`, no automatic color-depth
downgrade, and no hyperlink helper. `apply` takes an array of styles rather
than rest arguments, so an explicit options object can follow, and the
stripper is named `stripAnsi` rather than `stripAnsiCode`. The default is
always color, so writing to a redirected stream emits escape bytes unless
`{ color: false }` is passed.

Declarations: [`neo:chars`](/api/modules/chars/), [`neo:string-builder`](/api/modules/string-builder/), [`neo:ansi`](/api/modules/ansi/).
