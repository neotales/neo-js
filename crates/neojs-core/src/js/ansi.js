/**
 * ANSI SGR styling helpers: text attributes, 16-color foreground and
 * background colors, 256-color and 24-bit color, and a stripper.
 *
 * Color is **on by default and never detected**. These functions do not read
 * environment variables, process arguments, or whether standard output is a
 * terminal, and there is no module-global toggle. Running them while output is
 * redirected writes the escape bytes to the destination like any other text.
 * Pass `{ color: false }` as the last argument to return the input unchanged;
 * the same option is accepted by `apply` and by every style function.
 *
 * Styling wraps the value as `open + value + close`. If the exact closing
 * sequence for that style appears inside the value, it is replaced with the
 * opening sequence. Only that exact sequence is handled: a full reset
 * (`ESC[0m`), another style's reset, or any other SGR sequence in the value is
 * passed through and can end or change the style early. Styling does not
 * sanitize or normalize escape sequences.
 *
 * `stripAnsi` removes CSI sequences introduced by `ESC [` or the single-byte
 * C1 control U+009B, OSC strings, DCS/SOS/PM/APC strings, ESC c (RIS), and
 * two-byte escape sequences whose second byte is 0x40 through 0x5F. Control
 * strings end at BEL, ST (`ESC \`), or the single-byte C1 ST U+009C. It is a
 * best-effort text cleanup, not a security sanitizer and not a terminal
 * emulator: terminal escape protocols are larger than this scanner, other
 * control characters are preserved, an unterminated sequence is removed
 * through the end of the input, and an escape byte that does not begin a
 * recognized sequence is preserved. Do not rely on it alone to make untrusted
 * text safe to print.
 *
 * This module is synchronous, needs no permission flag, and grants no host
 * capability.
 */

const NativeTypeError = TypeError;
const NativeRangeError = RangeError;
const isInteger = Number.isInteger;
const isFiniteNumber = Number.isFinite;
const isArray = Array.isArray;
const join = Function.prototype.call.bind(Array.prototype.join);
const indexOf = Function.prototype.call.bind(String.prototype.indexOf);
const slice = Function.prototype.call.bind(String.prototype.slice);
const charCodeAt = Function.prototype.call.bind(String.prototype.charCodeAt);
const trunc = Math.trunc;

const ESC = '\u001b[';
const TEXT = '\u001b';
const BEL = 0x07;
const ST = 0x5c;

function requireText(value) {
  if (typeof value !== 'string') throw new NativeTypeError('value must be a string');
  return value;
}

function colorEnabled(options) {
  if (options === undefined) return true;
  if (options === null || typeof options !== 'object')
    throw new NativeTypeError('options must be an object');
  const color = options.color;
  if (color === undefined) return true;
  if (typeof color !== 'boolean')
    throw new NativeTypeError('options.color must be a boolean');
  return color;
}

function replaceAll(text, search, replacement) {
  let at = indexOf(text, search);
  if (at < 0) return text;
  let out = '';
  let from = 0;
  while (at >= 0) {
    out += slice(text, from, at) + replacement;
    from = at + search.length;
    at = indexOf(text, search, from);
  }
  return out + slice(text, from);
}

function sgr(value, open, close, options) {
  requireText(value);
  if (!colorEnabled(options)) return value;
  const start = ESC + join(open, ';') + 'm';
  const end = ESC + close + 'm';
  return start + (indexOf(value, end) < 0 ? value : replaceAll(value, end, start)) + end;
}

function clampByte(value, name) {
  if (typeof value !== 'number') throw new NativeTypeError(name + ' must be a number');
  if (!isFiniteNumber(value)) throw new NativeRangeError(name + ' must be finite');
  const whole = trunc(value);
  return whole < 0 ? 0 : whole > 255 ? 255 : whole;
}

function rgbTriple(color) {
  if (typeof color === 'number') {
    if (!isInteger(color) || color < 0 || color > 0xffffff)
      throw new NativeRangeError('color must be an integer from 0 through 0xFFFFFF');
    return [(color >> 16) & 0xff, (color >> 8) & 0xff, color & 0xff];
  }
  if (color === null || typeof color !== 'object')
    throw new NativeTypeError('color must be a number or an object with r, g, and b');
  return [
    clampByte(color.r, 'color.r'),
    clampByte(color.g, 'color.g'),
    clampByte(color.b, 'color.b'),
  ];
}

/** Reset all attributes (SGR 0). */
export function reset(value, options) {
  return sgr(value, [0], '0', options);
}

/** Bold (SGR 1, closed by 22). */
export function bold(value, options) {
  return sgr(value, [1], '22', options);
}

/** Dim or faint (SGR 2, closed by 22). */
export function dim(value, options) {
  return sgr(value, [2], '22', options);
}

/** Italic (SGR 3, closed by 23). */
export function italic(value, options) {
  return sgr(value, [3], '23', options);
}

/** Underline (SGR 4, closed by 24). */
export function underline(value, options) {
  return sgr(value, [4], '24', options);
}

/** Inverse or reverse video (SGR 7, closed by 27). */
export function inverse(value, options) {
  return sgr(value, [7], '27', options);
}

/** Hidden or concealed text (SGR 8, closed by 28). */
export function hidden(value, options) {
  return sgr(value, [8], '28', options);
}

/** Strikethrough (SGR 9, closed by 29). */
export function strikethrough(value, options) {
  return sgr(value, [9], '29', options);
}

/** Black foreground (SGR 30, closed by 39). */
export function black(value, options) {
  return sgr(value, [30], '39', options);
}

/** Red foreground (SGR 31, closed by 39). */
export function red(value, options) {
  return sgr(value, [31], '39', options);
}

/** Green foreground (SGR 32, closed by 39). */
export function green(value, options) {
  return sgr(value, [32], '39', options);
}

/** Yellow foreground (SGR 33, closed by 39). */
export function yellow(value, options) {
  return sgr(value, [33], '39', options);
}

/** Blue foreground (SGR 34, closed by 39). */
export function blue(value, options) {
  return sgr(value, [34], '39', options);
}

/** Magenta foreground (SGR 35, closed by 39). */
export function magenta(value, options) {
  return sgr(value, [35], '39', options);
}

/** Cyan foreground (SGR 36, closed by 39). */
export function cyan(value, options) {
  return sgr(value, [36], '39', options);
}

/** White foreground (SGR 37, closed by 39). */
export function white(value, options) {
  return sgr(value, [37], '39', options);
}

/** Bright black foreground (SGR 90, closed by 39). */
export function brightBlack(value, options) {
  return sgr(value, [90], '39', options);
}

/** Bright red foreground (SGR 91, closed by 39). */
export function brightRed(value, options) {
  return sgr(value, [91], '39', options);
}

/** Bright green foreground (SGR 92, closed by 39). */
export function brightGreen(value, options) {
  return sgr(value, [92], '39', options);
}

/** Bright yellow foreground (SGR 93, closed by 39). */
export function brightYellow(value, options) {
  return sgr(value, [93], '39', options);
}

/** Bright blue foreground (SGR 94, closed by 39). */
export function brightBlue(value, options) {
  return sgr(value, [94], '39', options);
}

/** Bright magenta foreground (SGR 95, closed by 39). */
export function brightMagenta(value, options) {
  return sgr(value, [95], '39', options);
}

/** Bright cyan foreground (SGR 96, closed by 39). */
export function brightCyan(value, options) {
  return sgr(value, [96], '39', options);
}

/** Bright white foreground (SGR 97, closed by 39). */
export function brightWhite(value, options) {
  return sgr(value, [97], '39', options);
}

/** Alias for `brightBlack`. */
export function gray(value, options) {
  return brightBlack(value, options);
}

/** Black background (SGR 40, closed by 49). */
export function bgBlack(value, options) {
  return sgr(value, [40], '49', options);
}

/** Red background (SGR 41, closed by 49). */
export function bgRed(value, options) {
  return sgr(value, [41], '49', options);
}

/** Green background (SGR 42, closed by 49). */
export function bgGreen(value, options) {
  return sgr(value, [42], '49', options);
}

/** Yellow background (SGR 43, closed by 49). */
export function bgYellow(value, options) {
  return sgr(value, [43], '49', options);
}

/** Blue background (SGR 44, closed by 49). */
export function bgBlue(value, options) {
  return sgr(value, [44], '49', options);
}

/** Magenta background (SGR 45, closed by 49). */
export function bgMagenta(value, options) {
  return sgr(value, [45], '49', options);
}

/** Cyan background (SGR 46, closed by 49). */
export function bgCyan(value, options) {
  return sgr(value, [46], '49', options);
}

/** White background (SGR 47, closed by 49). */
export function bgWhite(value, options) {
  return sgr(value, [47], '49', options);
}

/** Bright black background (SGR 100, closed by 49). */
export function bgBrightBlack(value, options) {
  return sgr(value, [100], '49', options);
}

/** Bright red background (SGR 101, closed by 49). */
export function bgBrightRed(value, options) {
  return sgr(value, [101], '49', options);
}

/** Bright green background (SGR 102, closed by 49). */
export function bgBrightGreen(value, options) {
  return sgr(value, [102], '49', options);
}

/** Bright yellow background (SGR 103, closed by 49). */
export function bgBrightYellow(value, options) {
  return sgr(value, [103], '49', options);
}

/** Bright blue background (SGR 104, closed by 49). */
export function bgBrightBlue(value, options) {
  return sgr(value, [104], '49', options);
}

/** Bright magenta background (SGR 105, closed by 49). */
export function bgBrightMagenta(value, options) {
  return sgr(value, [105], '49', options);
}

/** Bright cyan background (SGR 106, closed by 49). */
export function bgBrightCyan(value, options) {
  return sgr(value, [106], '49', options);
}

/** Bright white background (SGR 107, closed by 49). */
export function bgBrightWhite(value, options) {
  return sgr(value, [107], '49', options);
}

/**
 * Apply 256-color foreground `index` (SGR 38;5;n, closed by 39).
 *
 * `index` is truncated and clamped to 0 through 255. A non-number throws
 * `TypeError`; a non-finite number throws `RangeError`.
 */
export function rgb8(value, index, options) {
  return sgr(value, [38, 5, clampByte(index, 'index')], '39', options);
}

/**
 * Apply 256-color background `index` (SGR 48;5;n, closed by 49).
 *
 * `index` is truncated and clamped to 0 through 255.
 */
export function bgRgb8(value, index, options) {
  return sgr(value, [48, 5, clampByte(index, 'index')], '49', options);
}

/**
 * Apply 24-bit foreground color (SGR 38;2;r;g;b, closed by 39).
 *
 * `color` is a 24-bit integer from 0x000000 through 0xFFFFFF, or an object
 * with `r`, `g`, and `b`. Object channels are truncated and clamped to 0
 * through 255; the integer form is range-checked instead of masked.
 */
export function rgb24(value, color, options) {
  const [r, g, b] = rgbTriple(color);
  return sgr(value, [38, 2, r, g, b], '39', options);
}

/**
 * Apply 24-bit background color (SGR 48;2;r;g;b, closed by 49).
 *
 * The `color` argument follows `rgb24`.
 */
export function bgRgb24(value, color, options) {
  const [r, g, b] = rgbTriple(color);
  return sgr(value, [48, 2, r, g, b], '49', options);
}

/**
 * Apply style functions from left to right.
 *
 * `styles` must be an array of functions with the same `(value, options)`
 * shape. The first function wraps the original value most tightly; each later
 * function wraps the previous result. An empty array returns the value
 * unchanged. Invalid arguments throw `TypeError` even when color is disabled.
 */
export function apply(value, styles, options) {
  requireText(value);
  if (!isArray(styles)) throw new NativeTypeError('styles must be an array');
  for (let index = 0; index < styles.length; index++) {
    if (typeof styles[index] !== 'function')
      throw new NativeTypeError('each style must be a function');
  }
  if (!colorEnabled(options)) return value;
  let out = value;
  for (let index = 0; index < styles.length; index++) out = styles[index](out);
  return out;
}

function skipCsi(value, index) {
  while (index < value.length) {
    const code = charCodeAt(value, index);
    index++;
    if (code >= 0x40 && code <= 0x7e) break;
  }
  return index;
}

function skipControlString(value, index) {
  while (index < value.length) {
    const code = charCodeAt(value, index);
    if (code === BEL || code === 0x9c) return index + 1;
    if (code === 0x1b && index + 1 < value.length &&
      charCodeAt(value, index + 1) === ST) return index + 2;
    index++;
  }
  return index;
}

/**
 * Remove ANSI escape sequences from `value`.
 *
 * This removes CSI sequences introduced by `ESC [` or U+009B, OSC strings,
 * DCS/SOS/PM/APC strings, ESC c (RIS), and two-byte escape sequences whose
 * second byte is 0x40 through 0x5F. A control string ends at BEL, ST
 * (`ESC \`), or the single-byte C1 ST U+009C; text after the terminator is
 * retained. Other text is preserved exactly, including any lone or
 * unrecognized escape byte and all other control characters.
 *
 * This is best-effort cleanup, not a security sanitizer. Escape protocols are
 * more complex than this scanner, and malformed or unhandled sequences may
 * remain.
 */
export function stripAnsi(value) {
  requireText(value);
  let out = '';
  let index = 0;
  while (index < value.length) {
    const code = charCodeAt(value, index);
    if (code === 0x9b) {
      // Single-byte C1 CSI: consume through the first final byte.
      index = skipCsi(value, index + 1);
      continue;
    }
    if (code !== 0x1b) {
      out += value[index];
      index++;
      continue;
    }
    const next = index + 1 < value.length ? charCodeAt(value, index + 1) : -1;
    if (next === 0x5b) {
      // CSI: consume through the first final byte, 0x40 through 0x7E.
      index = skipCsi(value, index + 2);
      continue;
    }
    if (next === 0x5d || next === 0x50 || next === 0x58 || next === 0x5e || next === 0x5f) {
      // OSC, DCS, SOS, PM, APC: consume through BEL, C1 ST, or ST (ESC backslash).
      index = skipControlString(value, index + 2);
      continue;
    }
    if (next === 0x63) {
      // ESC c: RIS, a full terminal reset.
      index += 2;
      continue;
    }
    if (next >= 0x40 && next <= 0x5f) {
      index += 2;
      continue;
    }
    if (next >= 0x20 && next <= 0x2f) {
      // nF escape: intermediates followed by a final byte, 0x30 through 0x7E.
      index += 2;
      while (index < value.length) {
        const final = charCodeAt(value, index);
        index++;
        if (final >= 0x30 && final <= 0x7e) break;
      }
      continue;
    }
    // A lone escape byte, or one that does not introduce a sequence.
    out += TEXT;
    index++;
  }
  return out;
}
