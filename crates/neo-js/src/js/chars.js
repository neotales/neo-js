/**
 * Unicode and ASCII character predicates and code-point operations.
 *
 * Every predicate takes a Unicode code point as a number. It returns `false`
 * for anything that is not a Unicode scalar value: a non-number, a
 * non-integer, a value outside 0 through 0x10FFFF, or a surrogate code point
 * (0xD800 through 0xDFFF). Code-point predicates never throw; the `...At`
 * variants validate their string and index and are described below.
 *
 * `isAscii` and `isLatin1` describe code point ranges. The other predicates
 * follow the host engine's Unicode property tables (`\p{L}`, `\p{Nd}`, and so
 * on), so a newer or older QuickJS build may classify a newly assigned code
 * point differently. This module is not a port of `@neotales/chars`; it ships
 * no Go lookup tables and does not claim byte-for-byte agreement with them.
 *
 * The `...At` variants take a UTF-16 code unit index, the same index
 * `String.prototype.codePointAt` uses. An index in the middle of a surrogate
 * pair reads the trailing surrogate, which is not a scalar value, so every
 * predicate returns `false` for that index. Out-of-range indexes also return
 * `false`; a non-string text or a non-integer or negative index throws
 * `TypeError`.
 *
 * This module is synchronous, needs no permission flag, and grants no host
 * capability.
 */

// Capture the values this module relies on at module evaluation so later
// replacements do not change classification. This is not isolation from a
// realm that replaced them before the import, and not a sandbox for the rest
// of the realm.
const NativeTypeError = TypeError;
const NativeRangeError = RangeError;
const NativeRegExp = RegExp;
const isInteger = Number.isInteger;
const fromCodePoint = String.fromCodePoint;
const codePointAt = Function.prototype.call.bind(String.prototype.codePointAt);
// Bind the builtin `exec`, not `test`: `RegExp.prototype.test` performs a
// dynamic `Get(R, "exec")`, so replacing RegExp.prototype.exec after import
// would change every classification. Calling the captured exec directly with
// the captured `Function.prototype.call` avoids that lookup.
const regexpExec = Function.prototype.call.bind(RegExp.prototype.exec);
const toHex = Function.prototype.call.bind(Number.prototype.toString);

function matches(pattern, text) {
  return regexpExec(pattern, text) !== null;
}

const LETTER = /\p{L}/u;
const DIGIT = /\p{Nd}/u;
const UPPER = /\p{Lu}/u;
const LOWER = /\p{Ll}/u;
const CONTROL = /\p{Cc}/u;
const PUNCTUATION = /\p{P}/u;
const SYMBOL = /\p{S}/u;
// Go's unicode.IsSpace, expressed as a property class: ASCII tab through
// carriage return, U+0085 NEL (a control, not a separator), and the Unicode
// space separators. U+200B ZERO WIDTH SPACE is not whitespace here.
const WHITESPACE = /[\t\n\v\f\r \u0085\p{Zs}\p{Zl}\p{Zp}]/u;

/** Largest Unicode code point, 0x10FFFF. */
export const MAX_CODE_POINT = 0x10ffff;
/** Largest ASCII code point, 0x7F. */
export const ASCII_MAX = 0x7f;
/** Largest Latin-1 code point, 0xFF. */
export const LATIN1_MAX = 0xff;
/** First UTF-16 surrogate code point, 0xD800. */
export const SURROGATE_MIN = 0xd800;
/** Last UTF-16 surrogate code point, 0xDFFF. */
export const SURROGATE_MAX = 0xdfff;

/**
 * True when `value` is a Unicode scalar value: an integer from 0 through
 * 0x10FFFF that is not a surrogate code point.
 */
export function isChar(value) {
  return (
    typeof value === 'number' &&
    isInteger(value) &&
    value >= 0 &&
    value <= MAX_CODE_POINT &&
    (value < SURROGATE_MIN || value > SURROGATE_MAX)
  );
}

/** True for an ASCII code point, 0 through 0x7F. */
export function isAscii(value) {
  return isChar(value) && value <= ASCII_MAX;
}

/** True for a Latin-1 code point, 0 through 0xFF. */
export function isLatin1(value) {
  return isChar(value) && value <= LATIN1_MAX;
}

/** True for a C0 or C1 control character (Unicode category Cc). */
export function isControl(value) {
  return isChar(value) && matches(CONTROL, fromCodePoint(value));
}

/** True for a decimal digit (Unicode category Nd), not for other numerals. */
export function isDigit(value) {
  return isChar(value) && matches(DIGIT, fromCodePoint(value));
}

/** True for a letter (Unicode category L). */
export function isLetter(value) {
  return isChar(value) && matches(LETTER, fromCodePoint(value));
}

/** True for a letter or a decimal digit. */
export function isLetterOrDigit(value) {
  return isChar(value) &&
    (matches(LETTER, fromCodePoint(value)) || matches(DIGIT, fromCodePoint(value)));
}

/** True for a lowercase letter (Unicode category Ll). */
export function isLower(value) {
  return isChar(value) && matches(LOWER, fromCodePoint(value));
}

/** True for an uppercase letter (Unicode category Lu). */
export function isUpper(value) {
  return isChar(value) && matches(UPPER, fromCodePoint(value));
}

/** True for a punctuation character (Unicode category P). */
export function isPunc(value) {
  return isChar(value) && matches(PUNCTUATION, fromCodePoint(value));
}

/**
 * True for a whitespace character: ASCII tab, line feed, vertical tab, form
 * feed, carriage return, and space, plus U+0085 and the Unicode separator
 * categories Zs, Zl, and Zp.
 */
export function isSpace(value) {
  return isChar(value) && matches(WHITESPACE, fromCodePoint(value));
}

/** True for a symbol (Unicode category S), including emoji. */
export function isSymbol(value) {
  return isChar(value) && matches(SYMBOL, fromCodePoint(value));
}

function rejectBadIndex(text, index) {
  if (typeof text !== 'string') throw new NativeTypeError('text must be a string');
  if (typeof index !== 'number' || !isInteger(index) || index < 0)
    throw new NativeTypeError('index must be a non-negative integer');
}

// Return the code point starting at a UTF-16 index, or -1 for an index past
// the end. A negative sentinel is never a scalar value, so every predicate
// correctly rejects it.
function codePointAtOrMissing(text, index) {
  rejectBadIndex(text, index);
  const code = codePointAt(text, index);
  return code === undefined ? -1 : code;
}

/** True for an ASCII code point starting at UTF-16 index `index`. */
export function isAsciiAt(text, index) {
  return isAscii(codePointAtOrMissing(text, index));
}

/** True for a Latin-1 code point starting at UTF-16 index `index`. */
export function isLatin1At(text, index) {
  return isLatin1(codePointAtOrMissing(text, index));
}

/** True for a control character starting at UTF-16 index `index`. */
export function isControlAt(text, index) {
  return isControl(codePointAtOrMissing(text, index));
}

/** True for a decimal digit starting at UTF-16 index `index`. */
export function isDigitAt(text, index) {
  return isDigit(codePointAtOrMissing(text, index));
}

/** True for a letter starting at UTF-16 index `index`. */
export function isLetterAt(text, index) {
  return isLetter(codePointAtOrMissing(text, index));
}

/** True for a letter or decimal digit starting at UTF-16 index `index`. */
export function isLetterOrDigitAt(text, index) {
  return isLetterOrDigit(codePointAtOrMissing(text, index));
}

/** True for a lowercase letter starting at UTF-16 index `index`. */
export function isLowerAt(text, index) {
  return isLower(codePointAtOrMissing(text, index));
}

/** True for an uppercase letter starting at UTF-16 index `index`. */
export function isUpperAt(text, index) {
  return isUpper(codePointAtOrMissing(text, index));
}

/** True for punctuation starting at UTF-16 index `index`. */
export function isPuncAt(text, index) {
  return isPunc(codePointAtOrMissing(text, index));
}

/** True for whitespace starting at UTF-16 index `index`. */
export function isSpaceAt(text, index) {
  return isSpace(codePointAtOrMissing(text, index));
}

/** True for a symbol starting at UTF-16 index `index`. */
export function isSymbolAt(text, index) {
  return isSymbol(codePointAtOrMissing(text, index));
}

function requireChar(value, name) {
  if (typeof value !== 'number') throw new NativeTypeError(name + ' must be a number');
  if (!isChar(value)) throw new NativeRangeError(name + ' must be a Unicode scalar value');
  return value;
}

// Apply a String case method to one code point, but keep the result only when
// it is a single code point. A full case mapping can expand (ß to SS, İ to
// i plus a combining dot); those mappings are left alone rather than returning
// a number that cannot represent the result.
function mapCase(value, method) {
  const text = fromCodePoint(requireChar(value, 'char'));
  const mapped = method(text);
  const first = codePointAt(mapped, 0);
  const width = first > 0xffff ? 2 : 1;
  return mapped.length === width ? first : value;
}

const lowerCase = Function.prototype.call.bind(String.prototype.toLowerCase);
const upperCase = Function.prototype.call.bind(String.prototype.toUpperCase);

/**
 * Return the lowercase code point for `value`, or `value` when the host
 * mapping is not a single code point.
 *
 * @throws TypeError when `value` is not a number.
 * @throws RangeError when `value` is not a Unicode scalar value.
 */
export function toLower(value) {
  return mapCase(value, lowerCase);
}

/**
 * Return the uppercase code point for `value`, or `value` when the host
 * mapping is not a single code point.
 *
 * @throws TypeError when `value` is not a number.
 * @throws RangeError when `value` is not a Unicode scalar value.
 */
export function toUpper(value) {
  return mapCase(value, upperCase);
}

/**
 * Flip the case of a single code point: uppercase when a one-code-point
 * uppercase mapping exists, otherwise lowercase, otherwise the value itself.
 *
 * @throws TypeError when `value` is not a number.
 * @throws RangeError when `value` is not a Unicode scalar value.
 */
export function simpleFold(value) {
  const upper = toUpper(value);
  return upper !== value ? upper : toLower(value);
}

/**
 * Compare two code points case-insensitively under the host engine's Unicode
 * simple case-fold canonicalization.
 *
 * The comparison is an equivalence relation: it is reflexive, symmetric, and
 * transitive. One code point is used as an anchored case-insensitive Unicode
 * pattern and the other is tested against it. Because the engine canonicalizes
 * each code point to one value, related characters cannot disagree depending
 * on comparison order. The Greek theta class U+0398, U+03B8, U+03D1, and
 * U+03F4 is one class; `A`/`a`, Greek final sigma and sigma, Kelvin sign and
 * `k`, long s and `s`, and micro sign and Greek mu also match.
 *
 * This is not Go's `unicode.SimpleFold` orbit and not full Unicode case
 * folding. A mapping that expands to more than one code point is not applied,
 * so `ß` (0xDF) and `s` (0x73) compare as different. The engine's Unicode
 * version decides which pairs are folded.
 *
 * @throws TypeError when either argument is not a number.
 * @throws RangeError when either argument is not a Unicode scalar value.
 */
export function equalFold(a, b) {
  requireChar(a, 'a');
  requireChar(b, 'b');
  if (a === b) return true;
  if (a < 0x80 && b < 0x80)
    return (a >= 0x41 && a <= 0x5a ? a + 0x20 : a) ===
      (b >= 0x41 && b <= 0x5a ? b + 0x20 : b);
  const pattern = new NativeRegExp('^\\u{' + toHex(a, 16) + '}$', 'iu');
  return matches(pattern, fromCodePoint(b));
}
