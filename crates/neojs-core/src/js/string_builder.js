/**
 * A synchronous string builder for assembling text in pieces.
 *
 * The builder stores appended text as UTF-16 strings and joins them lazily.
 * `length` is the number of UTF-16 code units in `toString()`, matching the
 * `.length` of the resulting JavaScript string. `codePointLength` counts
 * Unicode code points instead, so an astral character counts as one. A lone
 * surrogate counts as one code point in both the string and the count; this
 * module never repairs or rejects text supplied to `append`.
 *
 * `append` accepts a primitive string, number, boolean, or bigint and converts
 * the last three with `String(...)`. `undefined`, `null`, symbols, objects,
 * functions, and boxed primitives throw `TypeError`; they are not coerced.
 * An empty string appends nothing and leaves the builder unchanged.
 * `appendCodePoint` accepts only a Unicode scalar value: it rejects surrogate
 * code points, so the builder cannot produce a lone surrogate through that
 * method.
 *
 * This module is not a subclass of `@neotales/slices`' `CharArrayBuilder`. It
 * keeps no `Uint32Array` code-point buffer, has no `appendCharArray` or
 * `appendSlice` methods, accepts no `Date`, and takes no capacity argument;
 * the internal chunk array grows as text is appended.
 *
 * The module is synchronous, needs no permission flag, and grants no host
 * capability.
 */

// Capture the builtins used below at module evaluation. This does not isolate
// the builder from earlier replacements or arbitrary later realm mutations
// (including changes to the builder's own methods).
const NativeTypeError = TypeError;
const NativeRangeError = RangeError;
const isInteger = Number.isInteger;
const fromCodePoint = String.fromCodePoint;
const stringFrom = String;
const join = Function.prototype.call.bind(Array.prototype.join);
const codePointAt = Function.prototype.call.bind(String.prototype.codePointAt);
const defineProperty = Object.defineProperty;

export class StringBuilder {
  #parts;
  #length;
  #cache;

  /** Create an empty builder. */
  constructor() {
    this.#parts = [];
    this.#length = 0;
    this.#cache = '';
  }

  /** Number of UTF-16 code units in the current text. */
  get length() {
    return this.#length;
  }

  /** Number of Unicode code points in the current text. */
  get codePointLength() {
    const text = this.toString();
    let count = 0;
    for (let index = 0; index < text.length; index++) {
      const code = codePointAt(text, index);
      if (code > 0xffff) index++;
      count++;
    }
    return count;
  }

  /** The current text, the same value as `toString()`. */
  get string() {
    return this.toString();
  }

  /**
   * Append text and return this builder.
   *
   * A primitive string is used as supplied. A number, boolean, or bigint is
   * converted with `String(...)`. Anything else throws `TypeError`.
   */
  append(value) {
    const type = typeof value;
    let text;
    if (type === 'string') text = value;
    else if (type === 'number' || type === 'boolean' || type === 'bigint')
      text = stringFrom(value);
    else
      throw new NativeTypeError(
        'value must be a string, number, boolean, or bigint',
      );
    if (text.length !== 0) {
      // Define an own data property instead of assigning through a possibly
      // inherited Array.prototype index setter. The descriptor must not
      // inherit Object.prototype.get/set, which defineProperty would read.
      defineProperty(this.#parts, this.#parts.length, {
        __proto__: null,
        value: text,
        enumerable: true,
        configurable: true,
        writable: true,
      });
      this.#length += text.length;
      this.#cache = null;
    }
    return this;
  }

  /**
   * Append one Unicode scalar value and return this builder.
   *
   * A surrogate code point is rejected with `RangeError` rather than stored as
   * a lone surrogate. A non-number throws `TypeError`; a non-integer or
   * out-of-range number throws `RangeError`.
   */
  appendCodePoint(codePoint) {
    if (typeof codePoint !== 'number')
      throw new NativeTypeError('codePoint must be a number');
    if (!isInteger(codePoint) || codePoint < 0 || codePoint > 0x10ffff ||
      (codePoint >= 0xd800 && codePoint <= 0xdfff))
      throw new NativeRangeError('codePoint must be a Unicode scalar value');
    return this.append(fromCodePoint(codePoint));
  }

  /**
   * Remove all text and return this builder. The builder remains usable.
   */
  clear() {
    this.#parts.length = 0;
    this.#length = 0;
    this.#cache = '';
    return this;
  }

  /** Return the current text. The result is cached until the next change. */
  toString() {
    if (this.#cache === null) this.#cache = join(this.#parts, '');
    return this.#cache;
  }
}
