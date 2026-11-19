/**
 * Lexical path manipulation for POSIX and Windows syntax.
 *
 * The module exports two frozen namespaces, `posix` and `win32`, with the same
 * synchronous functions: `join`, `normalize`, `basename`, `dirname`, `extname`,
 * `isAbsolute`, `parse`, `format`, `common`, `resolve`, `relative`, and
 * `toNamespacedPath`, plus `sep` and `delimiter`. Named exports select
 * `win32` on Windows and `posix` elsewhere, independently of `neo:os`.
 *
 * These are pure lexical operations. No function reads or writes the
 * filesystem or environment variables; none resolves symlinks, junctions,
 * drive mappings, or short names, and none checks existence. Windows
 * `resolve` and `relative` compare roots case-insensitively; `relative` also
 * compares components case-insensitively. No Unicode normalization occurs. A path is a
 * sequence of UTF-16 code units, and every code unit except the separators and
 * dots described below is opaque. Unpaired surrogates are preserved, not
 * rejected or repaired. A NUL (U+0000) in any argument throws `TypeError`: a
 * NUL terminates C paths and cannot appear in a usable path, so returning a
 * lexical result containing one would look usable when it is not.
 *
 * Arguments and errors:
 *
 * - Every path argument must be a primitive string. A boxed `String`, `URL`,
 *   buffer, number, `null`, or `undefined` throws `TypeError`; nothing is
 *   coerced. Windows functions accept both `\` and `/` as separators, but the
 *   argument itself is still a string.
 * - `basename`'s `suffix` must be a string. `format`'s argument must be a
 *   non-null, non-array object, and each of `root`, `dir`, `base`, `ext`, and
 *   `name` must be a string or absent. `undefined` means absent; `null` throws.
 * - `resolve`, `relative`, and Windows `toNamespacedPath` use the runtime's
 *   current directory only when their inputs do not supply a complete base.
 *   This exposes cwd text only with admitted filesystem or process/stdio
 *   authority and only in the native dialect. They do not read per-drive
 *   directories or fall back to the process cwd. Use explicit absolute bases
 *   to work independently of cwd.
 *
 * A path that resolves to nothing uses the platform's conventional dot: an
 * empty path normalizes to `.`, `join()` with no non-empty segment is `.`, and
 * `dirname('')` is `.`. An empty path has an empty `basename`, no extension,
 * and an all-empty `parse` result. `isAbsolute('')` is false and `format({})`
 * is the empty string.
 *
 * POSIX (`/` separator, `:` delimiter):
 *
 * - `normalize` collapses runs of `/`, removes `.` segments, and applies `..`
 *   to the preceding segment. Leading `..` is preserved in a relative path and
 *   dropped at an absolute root. At most one trailing `/` is preserved. `//`
 *   and `///` collapse to `/`; this module does not implement a distinct
 *   implementation-defined `//` network root.
 * - `join` validates every argument before dropping empty ones, concatenates
 *   with `/`, and normalizes. It does not restart at an absolute later
 *   argument: `join('a', '/b')` is `a/b`.
 * - `basename` ignores trailing `/` and returns the final segment. A path that
 *   is only separators, including `/`, has an empty basename. The suffix is
 *   removed only when it is shorter than the name and matches exactly, so
 *   `basename('a.txt', 'a.txt')` is unchanged.
 * - `dirname` drops trailing separators, returns the text before the last
 *   separator with trailing separators removed, and returns `/` for a path
 *   that is only separators. A root-only path is `/`; an empty or relative
 *   path with no parent is `.`.
 * - `extname` ignores trailing separators, so `extname('a/b.txt/')` is
 *   `.txt`. Dot files (`'.bashrc'`), `.`, `..`, and paths with no dot have no
 *   extension; `a.` and `a..` have extension `.`. The last dot of the final
 *   segment starts the extension.
 * - `isAbsolute` is true only for a non-empty path whose first code unit is
 *   `/`. A backslash is an ordinary filename character.
 * - `parse` returns `{root, dir, base, ext, name}`. `root` is `/` for an
 *   absolute path, otherwise `''`; `base` is `''` when the final component is
 *   empty (for example `'/'` or an empty path); `dir` is the root or the text
 *   before the final separator with trailing separators removed. It does not
 *   normalize the whole path, so `parse('a//b').dir` is `a`.
 * - `format` joins `dir` (or `root` when `dir` is empty) and `base` (or
 *   `name + ext` when `base` is empty) with `/`. It does not normalize or
 *   validate consistency. Empty strings are treated as absent for `dir`,
 *   `root`, and `base`; `base` wins over `name` and `ext`. A `base` equal to
 *   `/` is appended literally, so `format({dir: 'foo', base: '/'})` is
 *   `'foo//'`; `base` is never silently dropped.
 *
 * Windows (`\` separator, `;` delimiter; both `\` and `/` accepted on input):
 *
 * - Roots are recognized in this order: a UNC root `\\server\share` with a
 *   non-empty server and share; an ASCII drive root `C:\` (letter, colon,
 *   separator); a drive-relative root `C:` (letter and colon, no separator); a
 *   rooted path starting with one separator; otherwise a relative path.
 *   Drive letters must be ASCII `A`-`Z` or `a`-`z`; case is preserved.
 *   Server and share names are opaque, may contain any non-separator
 *   characters, and are not validated or resolved.
 *   The original parsing and normalization helpers give `\\\\?\\` and
 *   `\\\\.\\` no special treatment: `\\\\?\\C:\\foo`
 *   parses as a UNC-shaped root (`\\\\?\\C:\\`) followed by `foo`, and
 *   `\\\\?\\UNC\\server\\share\\foo` parses with root `\\\\?\\UNC\\`.
 * - Exactly two leading separators followed by a non-empty server and share
 *   form a UNC root. Three or more leading separators, or a `\\` prefix with a
 *   missing server or share, are not UNC: they collapse to one rooted
 *   separator. A UNC root alone normalizes to `\\server\share\`.
 * - `normalize` converts separators to `\`, collapses runs, removes `.`,
 *   applies `..`, preserves the root, and preserves at most one trailing `\`.
 *   `..` is preserved above a relative or drive-relative root and dropped at
 *   an absolute root. A bare drive normalizes to `C:.`; `C:..` is preserved.
 *   No per-drive working directory is consulted. A colon is ordinary text
 *   unless exactly one ASCII letter precedes it at the start of the path, so
 *   `1:\foo` is relative and normalizes to itself; no `.\` prefix or drive
 *   correction is added.
 * - `join` works like POSIX `join` but joins with `\` and takes care not to
 *   turn a leading run of separators into an unintended UNC root. The first
 *   non-empty segment may itself be a UNC path. Joining a bare drive with a
 *   segment makes it rooted: `join('C:', 'foo')` is `C:\foo`.
 * - `basename` recognizes both separators and skips a drive prefix, so
 *   `C:\` and `C:` have an empty basename. A complete UNC root also has an
 *   empty basename, and `extname` is empty for it, even when the share
 *   contains a dot; the share is a root component, not a file. `dirname`,
 *   `extname`, `isAbsolute`, `parse`, and `format` follow the root rules
 *   above.
 *   `dirname` of a root-only path returns the root as written (`C:/` stays
 *   `C:/`), and `dirname('\\\\server\\share\\foo')` is
 *   `\\\\server\\share\\`.
 * - `parse` and `dirname` preserve the input's separator characters in the
 *   substrings they return; `join`, `normalize`, and `format` produce `/` or
 *   `\` as appropriate. `parse` removes trailing separators from `dir` except
 *   a root's own separator, so `parse('a//b').dir` is `a`.
 * - `format` follows the same joining rule with `\`. A `base` equal to `\` is
 *   appended literally rather than replacing `dir`; like the POSIX form, it is
 *   not silently dropped and the result is not normalized.
 *
 * Shared shape notes:
 *
 * - `parse` returns a fresh plain object on every call; mutating a result does
 *   not affect another call or any module state.
 * - `format(parse(path))` preserves the path's components and spelling in the
 *   common case, but it does not promise byte-for-byte round tripping: a
 *   trailing separator is dropped, and redundant separators around `dir` are
 *   collapsed because `parse` trims them.
 * - `posix.normalize` is idempotent. `win32.normalize` is idempotent except
 *   for a relative path that reduces to a bare drive-relative name, such as
 *   `'./a:'`: the result is `'a:'`, which normalizes again to `'a:.'`. No
 *   `.\` guard is added, so treat a normalized relative Windows path as a new
 *   path rather than a fixed point. POSIX `join` is exactly concatenation
 *   followed by `normalize`; the Windows `join` applies the UNC guard described
 *   above before normalizing, so a relative first segment cannot become a root
 *   by accident.
 *
 * Not a security boundary. Path strings are not validated against traversal,
 * symlinks or junctions, case-insensitive aliasing, Windows reserved device
 * names, alternate data streams, Unicode confusables, or network shares.
 * `normalize` resolves `..` by string manipulation, not filesystem semantics:
 * `a/../../b` becomes `../b`, which may or may not name a file outside a
 * directory depending on where it is used. Do not use this module to decide
 * whether a path is safely contained in a directory; enforce that with the
 * filesystem and the process's OS permissions, after resolving the real path
 * with the platform's APIs. `neo:fs` and `neo:exec` do not consult this
 * module.
 *
 * Deliberately unsupported, compared with Node's `path` and `@neotales/path`:
 * `toFileUrl`, `fromFileUrl`, and the glob helpers. URL
 * inputs, buffers, and type coercion are not accepted. There is no filesystem
 * or symlink resolution or validation of Windows device paths,
 * no POSIX `//` root preservation, and no asynchronous form. For a path that
 * is a drive root or a complete UNC root, returned `basename` and `parse`
 * fields are empty rather than a separator. This module is not a drop-in
 * replacement for either reference.
 */

// Provenance: the segment, root, and component algorithms are adapted from
// @neotales/path in the js-libraries repository, directory `std/jsr/path`, at
// revision c7e88644f45a78f65bf639c9d365bb9eff21bcfe (2026-09-28), MIT.
// Upstream copyright notices: Copyright 2022-2026 Neotales; Copyright
// 2018-2022 the Deno authors; Copyright (c) 2013 James Halliday. See
// ./path.LICENSE.md for the adapted-file list, the exact license-file hash,
// and the complete MIT permission text. The adaptation is not verbatim: this
// module adds the two-namespace shape, validation, the NUL rule, and the edge
// cases documented above.

// The host builds this API in the fresh realm, before running user code.
const NativeError = Error;
const NativeTypeError = TypeError;
const defineProperty = Object.defineProperty;
const freeze = Object.freeze;
const isArray = Array.isArray;
const charCodeAt = Function.prototype.call.bind(String.prototype.charCodeAt);
const slice = Function.prototype.call.bind(String.prototype.slice);
const split = Function.prototype.call.bind(String.prototype.split);
const lower = Function.prototype.call.bind(String.prototype.toLowerCase);
const indexOf = Function.prototype.call.bind(String.prototype.indexOf);
const lastIndexOf = Function.prototype.call.bind(String.prototype.lastIndexOf);
const joinArray = Function.prototype.call.bind(Array.prototype.join);
const sliceArray = Function.prototype.call.bind(Array.prototype.slice);

const SLASH = 47;
const BACKSLASH = 92;
const DOT = 46;
const COLON = 58;

function isPosixSeparator(code) {
  return code === SLASH;
}

function isWin32Separator(code) {
  return code === SLASH || code === BACKSLASH;
}

/** True for an ASCII letter, the only drive-letter form Windows accepts. */
function isDeviceRoot(code) {
  return (code >= 65 && code <= 90) || (code >= 97 && code <= 122);
}

// True when the whole path is a recognized UNC root: exactly two leading
// separators, a non-empty server, at least one separator, a non-empty share,
// and nothing after the share except separators. This is the same root that
// `parse` reports as `{root, dir, base: '', ext: '', name: ''}`. A run of
// three or more leading separators is not a UNC root.
function isWin32UncRootOnly(path) {
  const length = path.length;
  if (length < 4) return false;
  if (
    !isWin32Separator(charCodeAt(path, 0)) ||
    !isWin32Separator(charCodeAt(path, 1))
  ) {
    return false;
  }

  let index = 2;
  const serverStart = index;
  while (index < length && !isWin32Separator(charCodeAt(path, index))) ++index;
  if (index === serverStart || index >= length) return false;

  const separatorStart = index;
  while (index < length && isWin32Separator(charCodeAt(path, index))) ++index;
  if (index === separatorStart || index >= length) return false;

  const shareStart = index;
  while (index < length && !isWin32Separator(charCodeAt(path, index))) ++index;
  if (index === shareStart) return false;

  while (index < length) {
    if (!isWin32Separator(charCodeAt(path, index))) return false;
    ++index;
  }
  return true;
}

function assertString(value, name) {
  if (typeof value !== 'string')
    throw new NativeTypeError(name + ' must be a string');
  if (indexOf(value, '\u0000') !== -1)
    throw new NativeTypeError(name + ' must not contain a NUL character');
  return value;
}

function assertPath(value) {
  return assertString(value, 'path');
}

function assertSuffix(value) {
  return assertString(value, 'suffix');
}

// Resolve `.` and `..` segments. `allowAboveRoot` decides whether a `..` with
// no preceding segment is kept. A trailing separator is not handled here.
function normalizeString(path, allowAboveRoot, separator, isSeparator) {
  let result = '';
  let lastSegmentLength = 0;
  let lastSlash = -1;
  let dots = 0;
  let code;

  for (let i = 0; i <= path.length; ++i) {
    if (i < path.length) code = charCodeAt(path, i);
    else if (isSeparator(code)) break;
    else code = SLASH;

    if (isSeparator(code)) {
      if (lastSlash === i - 1 || dots === 1) {
        // Empty segment or `.`: nothing to add.
      } else if (lastSlash !== i - 1 && dots === 2) {
        if (
          result.length < 2 ||
          lastSegmentLength !== 2 ||
          charCodeAt(result, result.length - 1) !== DOT ||
          charCodeAt(result, result.length - 2) !== DOT
        ) {
          if (result.length > 2) {
            const lastSlashIndex = lastIndexOf(result, separator);
            if (lastSlashIndex === -1) {
              result = '';
              lastSegmentLength = 0;
            } else {
              result = slice(result, 0, lastSlashIndex);
              lastSegmentLength =
                result.length - 1 - lastIndexOf(result, separator);
            }
            lastSlash = i;
            dots = 0;
            continue;
          }
          if (result.length === 2 || result.length === 1) {
            result = '';
            lastSegmentLength = 0;
            lastSlash = i;
            dots = 0;
            continue;
          }
        }
        if (allowAboveRoot) {
          result = result.length > 0 ? result + separator + '..' : '..';
          lastSegmentLength = 2;
        }
      } else {
        const segment = slice(path, lastSlash + 1, i);
        result = result.length > 0 ? result + separator + segment : segment;
        lastSegmentLength = i - lastSlash - 1;
      }
      lastSlash = i;
      dots = 0;
    } else if (code === DOT && dots !== -1) {
      ++dots;
    } else {
      dots = -1;
    }
  }

  return result;
}

// Remove trailing separators without shortening the root. `minimum` is the
// number of leading code units that belong to the root.
function stripTrailingSeparators(segment, isSeparator, minimum) {
  let end = segment.length;
  for (let i = segment.length - 1; i >= minimum; --i) {
    if (!isSeparator(charCodeAt(segment, i))) break;
    end = i;
  }
  return slice(segment, 0, end);
}

function stripSuffix(name, suffix) {
  if (suffix.length === 0 || suffix.length >= name.length) return name;
  const offset = name.length - suffix.length;
  for (let i = suffix.length - 1; i >= 0; --i) {
    if (charCodeAt(name, offset + i) !== charCodeAt(suffix, i)) return name;
  }
  return slice(name, 0, -suffix.length);
}

function basenamePath(path, suffix, isSeparator, hasDrivePrefix) {
  let end = path.length;
  while (end > 0 && isSeparator(charCodeAt(path, end - 1))) end--;

  let start = 0;
  if (
    hasDrivePrefix &&
    end >= 2 &&
    isDeviceRoot(charCodeAt(path, 0)) &&
    charCodeAt(path, 1) === COLON
  ) {
    start = 2;
  }

  if (end <= start) return '';

  let nameStart = end - 1;
  while (nameStart > start && !isSeparator(charCodeAt(path, nameStart - 1)))
    nameStart--;

  return stripSuffix(slice(path, nameStart, end), suffix);
}

function extnamePath(path, isSeparator, start) {
  let startDot = -1;
  let startPart = start;
  let end = -1;
  let matchedSlash = true;
  let preDotState = 0;

  for (let i = path.length - 1; i >= start; --i) {
    const code = charCodeAt(path, i);
    if (isSeparator(code)) {
      if (!matchedSlash) {
        startPart = i + 1;
        break;
      }
      continue;
    }
    if (end === -1) {
      matchedSlash = false;
      end = i + 1;
    }
    if (code === DOT) {
      if (startDot === -1) startDot = i;
      else if (preDotState !== 1) preDotState = 1;
    } else if (startDot !== -1) {
      preDotState = -1;
    }
  }

  if (
    startDot === -1 ||
    end === -1 ||
    preDotState === 0 ||
    (preDotState === 1 && startDot === end - 1 && startDot === startPart + 1)
  ) {
    return '';
  }
  return slice(path, startDot, end);
}

function formatPath(separator, pathObject) {
  if (
    pathObject === null ||
    typeof pathObject !== 'object' ||
    isArray(pathObject)
  ) {
    throw new NativeTypeError('pathObject must be an object');
  }

  const root = readField(pathObject, 'root');
  const dirValue = readField(pathObject, 'dir');
  const baseValue = readField(pathObject, 'base');
  const name = readField(pathObject, 'name');
  const ext = readField(pathObject, 'ext');

  const dir = dirValue.length > 0 ? dirValue : root;
  const base = baseValue.length > 0 ? baseValue : name + ext;

  if (dir.length === 0) return base;
  if (dir === root) return dir + base;
  return dir + separator + base;
}

function readField(pathObject, name) {
  const value = pathObject[name];
  if (value === undefined) return '';
  return assertString(value, name);
}

function normalizePosix(path) {
  assertPath(path);

  const absolute = path.length > 0 && isPosixSeparator(charCodeAt(path, 0));
  const trailing =
    path.length > 0 && isPosixSeparator(charCodeAt(path, path.length - 1));

  let tail = normalizeString(path, !absolute, '/', isPosixSeparator);
  if (tail.length === 0 && !absolute) tail = '.';
  if (tail.length > 0 && trailing) tail += '/';

  return absolute ? '/' + tail : tail;
}

function joinPosix(...paths) {
  const parts = [];
  for (let i = 0; i < paths.length; i++) {
    const part = assertPath(paths[i]);
    if (part.length > 0) parts[parts.length] = part;
  }
  if (parts.length === 0) return '.';
  return normalizePosix(joinArray(parts, '/'));
}

function basenamePosix(path, suffix = '') {
  assertPath(path);
  assertSuffix(suffix);
  return basenamePath(path, suffix, isPosixSeparator, false);
}

function dirnamePosix(path) {
  assertPath(path);

  let end = -1;
  let matchedNonSeparator = false;
  for (let i = path.length - 1; i >= 1; --i) {
    if (isPosixSeparator(charCodeAt(path, i))) {
      if (matchedNonSeparator) {
        end = i;
        break;
      }
    } else {
      matchedNonSeparator = true;
    }
  }

  const absolute = path.length > 0 && isPosixSeparator(charCodeAt(path, 0));
  if (end === -1) return absolute ? '/' : '.';

  return stripTrailingSeparators(
    slice(path, 0, end),
    isPosixSeparator,
    absolute ? 1 : 0,
  );
}

function extnamePosix(path) {
  assertPath(path);
  return extnamePath(path, isPosixSeparator, 0);
}

function isAbsolutePosix(path) {
  assertPath(path);
  return path.length > 0 && isPosixSeparator(charCodeAt(path, 0));
}

function parsePosix(path) {
  assertPath(path);

  const result = { root: '', dir: '', base: '', ext: '', name: '' };
  if (path.length === 0) return result;

  const absolute = isPosixSeparator(charCodeAt(path, 0));
  let start;
  if (absolute) {
    result.root = '/';
    start = 1;
  } else {
    start = 0;
  }

  let startDot = -1;
  let startPart = 0;
  let end = -1;
  let matchedSlash = true;
  let preDotState = 0;

  for (let i = path.length - 1; i >= start; --i) {
    const code = charCodeAt(path, i);
    if (isPosixSeparator(code)) {
      if (!matchedSlash) {
        startPart = i + 1;
        break;
      }
      continue;
    }
    if (end === -1) {
      matchedSlash = false;
      end = i + 1;
    }
    if (code === DOT) {
      if (startDot === -1) startDot = i;
      else if (preDotState !== 1) preDotState = 1;
    } else if (startDot !== -1) {
      preDotState = -1;
    }
  }

  if (
    startDot === -1 ||
    end === -1 ||
    preDotState === 0 ||
    (preDotState === 1 && startDot === end - 1 && startDot === startPart + 1)
  ) {
    if (end !== -1) {
      const componentStart = startPart === 0 && absolute ? 1 : startPart;
      result.base = result.name = slice(path, componentStart, end);
    }
  } else {
    const componentStart = startPart === 0 && absolute ? 1 : startPart;
    result.name = slice(path, componentStart, startDot);
    result.base = slice(path, componentStart, end);
    result.ext = slice(path, startDot, end);
  }

  if (startPart > 0) {
    result.dir = stripTrailingSeparators(
      slice(path, 0, startPart - 1),
      isPosixSeparator,
      absolute ? 1 : 0,
    );
  } else if (absolute) {
    result.dir = '/';
  }

  return result;
}

function formatPosix(pathObject) {
  return formatPath('/', pathObject);
}

function normalizeWin32(path) {
  assertPath(path);

  const length = path.length;
  let rootEnd = 0;
  let device;
  let absolute = false;
  const code = charCodeAt(path, 0);

  if (length > 1) {
    if (isWin32Separator(code)) {
      // A leading separator is rooted. Two separators are a UNC root only when
      // a non-empty server and share follow; otherwise the run collapses.
      absolute = true;

      if (isWin32Separator(charCodeAt(path, 1))) {
        let j = 2;
        let last = j;
        while (j < length && !isWin32Separator(charCodeAt(path, j))) ++j;
        if (j < length && j !== last) {
          const firstPart = slice(path, last, j);
          last = j;
          while (j < length && isWin32Separator(charCodeAt(path, j))) ++j;
          if (j < length && j !== last) {
            last = j;
            while (j < length && !isWin32Separator(charCodeAt(path, j))) ++j;
            if (j === length) {
              return '\\\\' + firstPart + '\\' + slice(path, last) + '\\';
            }
            if (j !== last) {
              device = '\\\\' + firstPart + '\\' + slice(path, last, j);
              rootEnd = j;
            }
          }
        }
      } else {
        rootEnd = 1;
      }
    } else if (
      isDeviceRoot(code) &&
      charCodeAt(path, 1) === COLON
    ) {
      device = slice(path, 0, 2);
      rootEnd = 2;
      if (length > 2) {
        if (isWin32Separator(charCodeAt(path, 2))) {
          absolute = true;
          rootEnd = 3;
        }
      }
    }
  } else if (isWin32Separator(code)) {
    return '\\';
  }

  let tail;
  if (rootEnd < length) {
    tail = normalizeString(
      slice(path, rootEnd),
      !absolute,
      '\\',
      isWin32Separator,
    );
  } else {
    tail = '';
  }

  if (tail.length === 0 && !absolute) tail = '.';
  if (tail.length > 0 && isWin32Separator(charCodeAt(path, length - 1)))
    tail += '\\';

  if (device === undefined) {
    if (absolute) return tail.length > 0 ? '\\' + tail : '\\';
    return tail;
  }
  if (absolute) return tail.length > 0 ? device + '\\' + tail : device + '\\';
  return device + tail;
}

function joinWin32(...paths) {
  const parts = [];
  for (let i = 0; i < paths.length; i++) {
    const part = assertPath(paths[i]);
    if (part.length > 0) parts[parts.length] = part;
  }
  if (parts.length === 0) return '.';

  let needsReplace = true;
  let slashCount = 0;
  const firstPart = parts[0];
  if (isWin32Separator(charCodeAt(firstPart, 0))) {
    ++slashCount;
    if (firstPart.length > 1) {
      if (isWin32Separator(charCodeAt(firstPart, 1))) {
        ++slashCount;
        if (firstPart.length > 2) {
          if (isWin32Separator(charCodeAt(firstPart, 2))) ++slashCount;
          else needsReplace = false;
        }
      }
    }
  }

  let joined = joinArray(parts, '\\');
  if (needsReplace) {
    for (; slashCount < joined.length; ++slashCount) {
      if (!isWin32Separator(charCodeAt(joined, slashCount))) break;
    }
    if (slashCount >= 2) joined = '\\' + slice(joined, slashCount);
  }

  return normalizeWin32(joined);
}

function basenameWin32(path, suffix = '') {
  assertPath(path);
  assertSuffix(suffix);
  // A complete UNC root is a directory, not a file, even when the share name
  // contains a dot. `parse` reports an empty `base` for the same path.
  if (isWin32UncRootOnly(path)) return '';
  return basenamePath(path, suffix, isWin32Separator, true);
}

function dirnameWin32(path) {
  assertPath(path);

  const length = path.length;
  let rootEnd = -1;
  let end = -1;
  let matchedSlash = true;
  let offset = 0;
  const code = charCodeAt(path, 0);

  if (length > 1) {
    if (isWin32Separator(code)) {
      rootEnd = offset = 1;

      if (isWin32Separator(charCodeAt(path, 1))) {
        let j = 2;
        let last = j;
        while (j < length && !isWin32Separator(charCodeAt(path, j))) ++j;
        if (j < length && j !== last) {
          last = j;
          while (j < length && isWin32Separator(charCodeAt(path, j))) ++j;
          if (j < length && j !== last) {
            last = j;
            while (j < length && !isWin32Separator(charCodeAt(path, j))) ++j;
            if (j === length) return path;
            if (j !== last) rootEnd = offset = j + 1;
          }
        }
      }
    } else if (isDeviceRoot(code) && charCodeAt(path, 1) === COLON) {
      rootEnd = offset = 2;
      if (length > 2 && isWin32Separator(charCodeAt(path, 2)))
        rootEnd = offset = 3;
    }
  } else if (isWin32Separator(code)) {
    return path;
  }

  for (let i = length - 1; i >= offset; --i) {
    if (isWin32Separator(charCodeAt(path, i))) {
      if (!matchedSlash) {
        end = i;
        break;
      }
    } else {
      matchedSlash = false;
    }
  }

  if (end === -1) {
    if (rootEnd === -1) return '.';
    return slice(path, 0, rootEnd);
  }

  return stripTrailingSeparators(
    slice(path, 0, end),
    isWin32Separator,
    rootEnd === -1 ? 0 : rootEnd,
  );
}

function extnameWin32(path) {
  assertPath(path);
  // The share name of a complete UNC root is not a file component, so a dot
  // in it is not an extension. This matches `parse`, which reports an empty
  // `base` for such a path.
  if (isWin32UncRootOnly(path)) return '';

  let start = 0;
  if (
    path.length >= 2 &&
    charCodeAt(path, 1) === COLON &&
    isDeviceRoot(charCodeAt(path, 0))
  ) {
    start = 2;
  }
  return extnamePath(path, isWin32Separator, start);
}

function isAbsoluteWin32(path) {
  assertPath(path);

  const length = path.length;
  if (length === 0) return false;

  const code = charCodeAt(path, 0);
  if (isWin32Separator(code)) return true;
  if (
    isDeviceRoot(code) &&
    length > 2 &&
    charCodeAt(path, 1) === COLON &&
    isWin32Separator(charCodeAt(path, 2))
  ) {
    return true;
  }
  return false;
}

function parseWin32(path) {
  assertPath(path);

  const result = { root: '', dir: '', base: '', ext: '', name: '' };
  const length = path.length;
  if (length === 0) return result;

  let rootEnd = 0;
  let code = charCodeAt(path, 0);

  if (length > 1) {
    if (isWin32Separator(code)) {
      rootEnd = 1;

      if (isWin32Separator(charCodeAt(path, 1))) {
        let j = 2;
        let last = j;
        while (j < length && !isWin32Separator(charCodeAt(path, j))) ++j;
        if (j < length && j !== last) {
          last = j;
          while (j < length && isWin32Separator(charCodeAt(path, j))) ++j;
          if (j < length && j !== last) {
            last = j;
            while (j < length && !isWin32Separator(charCodeAt(path, j))) ++j;
            if (j === length) {
              rootEnd = j;
            } else if (j !== last) {
              rootEnd = j + 1;
            }
          }
        }
      }
    } else if (isDeviceRoot(code) && charCodeAt(path, 1) === COLON) {
      rootEnd = 2;
      if (length > 2) {
        if (isWin32Separator(charCodeAt(path, 2))) {
          if (length === 3) {
            result.root = result.dir = path;
            return result;
          }
          rootEnd = 3;
        }
      } else {
        result.root = result.dir = path;
        return result;
      }
    }
  } else if (isWin32Separator(code)) {
    result.root = result.dir = path;
    return result;
  }

  if (rootEnd > 0) result.root = slice(path, 0, rootEnd);

  let startDot = -1;
  let startPart = rootEnd;
  let end = -1;
  let matchedSlash = true;
  let preDotState = 0;

  for (let i = length - 1; i >= rootEnd; --i) {
    code = charCodeAt(path, i);
    if (isWin32Separator(code)) {
      if (!matchedSlash) {
        startPart = i + 1;
        break;
      }
      continue;
    }
    if (end === -1) {
      matchedSlash = false;
      end = i + 1;
    }
    if (code === DOT) {
      if (startDot === -1) startDot = i;
      else if (preDotState !== 1) preDotState = 1;
    } else if (startDot !== -1) {
      preDotState = -1;
    }
  }

  if (
    startDot === -1 ||
    end === -1 ||
    preDotState === 0 ||
    (preDotState === 1 && startDot === end - 1 && startDot === startPart + 1)
  ) {
    if (end !== -1) result.base = result.name = slice(path, startPart, end);
  } else {
    result.name = slice(path, startPart, startDot);
    result.base = slice(path, startPart, end);
    result.ext = slice(path, startDot, end);
  }

  if (startPart > 0 && startPart !== rootEnd) {
    result.dir = stripTrailingSeparators(
      slice(path, 0, startPart - 1),
      isWin32Separator,
      rootEnd,
    );
  } else {
    result.dir = result.root;
  }

  return result;
}

function formatWin32(pathObject) {
  return formatPath('\\', pathObject);
}

/** Find the shared spelling at component boundaries, without normalizing. */
function commonPath(paths, separator) {
  if (!isArray(paths)) throw new NativeTypeError('paths must be an array');
  const length = paths.length;
  if (length === 0) return '';
  const first = split(assertPath(paths[0]), separator);
  let end = first.length;
  let append = '';
  for (let j = 1; j < length; ++j) {
    const parts = split(assertPath(paths[j]), separator);
    if (parts.length <= end) {
      end = parts.length;
      append = '';
    }
    for (let i = 0; i < end; ++i) {
      if (first[i] !== parts[i]) {
        end = i;
        append = i === 0 ? '' : separator;
        break;
      }
    }
  }
  return joinArray(sliceArray(first, 0, end), separator) + append;
}

function pathError(code, message) {
  const error = new NativeError(message);
  defineProperty(error, 'code', { value: code, enumerable: true });
  return error;
}

/** Obtain only the runtime cwd for the host dialect; never translate it. */
function requireCwd(windows) {
  if (windows !== hostWindows) {
    throw pathError('PATH_CWD_DIALECT', 'provide an absolute base for the non-host path dialect');
  }
  const directory = readCwd();
  if (directory === undefined) {
    throw pathError('PATH_PERMISSION', 'provide an absolute base or grant access to the runtime working directory');
  }
  if (directory === null) {
    throw pathError('PATH_CWD', 'the runtime working directory is unavailable; provide an absolute base');
  }
  return directory;
}

/** Resolve from the rightmost absolute segment, or the current runtime cwd. */
function resolvePosix(...paths) {
  for (let i = 0; i < paths.length; ++i) assertPath(paths[i]);
  const tails = [];
  for (let i = paths.length - 1; i >= -1; --i) {
    const path = i === -1 ? requireCwd(false) : paths[i];
    if (path === '') continue;
    tails[tails.length] = path;
    if (isPosixSeparator(charCodeAt(path, 0))) break;
  }
  return '/' + normalizedTail(tails, '/', isPosixSeparator);
}

function normalizedTail(tails, separator, isSeparator) {
  // Segments were collected right to left. Swap in place before normalizing.
  for (let i = 0, j = tails.length - 1; i < j; ++i, --j) {
    const tail = tails[i];
    tails[i] = tails[j];
    tails[j] = tail;
  }
  return normalizeString(joinArray(tails, separator), false, separator, isSeparator);
}

function skipWin32Separators(path, start) {
  while (isWin32Separator(charCodeAt(path, start))) ++start;
  return start;
}

function win32ComponentEnd(path, start) {
  while (start < path.length && !isWin32Separator(charCodeAt(path, start))) ++start;
  return start;
}

/** A namespace prefix such as `\\?\` or `\\.\`, including slash spellings. */
function hasNamespacePrefix(path) {
  const marker = charCodeAt(path, 2);
  return isWin32Separator(charCodeAt(path, 0)) && isWin32Separator(charCodeAt(path, 1)) &&
    (marker === 63 || marker === DOT) && isWin32Separator(charCodeAt(path, 3));
}

/** Extract a device/share as a unit so `..` cannot climb above it. */
function resolutionRootWin32(path) {
  const first = charCodeAt(path, 0);
  if (isDeviceRoot(first) && charCodeAt(path, 1) === COLON) {
    const device = slice(path, 0, 2);
    const absolute = isWin32Separator(charCodeAt(path, 2));
    return { device, drive: device, absolute, end: absolute ? 3 : 2 };
  }
  if (!isWin32Separator(first)) return { device: '', drive: '', absolute: false, end: 0 };
  if (isWin32Separator(charCodeAt(path, 1))) {
    const serverEnd = win32ComponentEnd(path, 2);
    const shareStart = skipWin32Separators(path, serverEnd);
    let end = win32ComponentEnd(path, shareStart);
    if (serverEnd > 2 && end > shareStart) {
      const server = slice(path, 2, serverEnd);
      const share = slice(path, shareStart, end);
      let device = '\\\\' + server + '\\' + share;
      let drive = '';
      if (server === '?' && lower(share) === 'unc') {
        const hostStart = skipWin32Separators(path, end);
        const hostEnd = win32ComponentEnd(path, hostStart);
        const volumeStart = skipWin32Separators(path, hostEnd);
        end = win32ComponentEnd(path, volumeStart);
        if (hostStart === hostEnd || volumeStart === end) {
          throw pathError('PATH_WINDOWS_ROOT', 'provide a server and share for a namespaced UNC path');
        }
        device += '\\' + slice(path, hostStart, hostEnd) + '\\' + slice(path, volumeStart, end);
      } else if (server === '?' && share.length === 2 &&
        isDeviceRoot(charCodeAt(share, 0)) && charCodeAt(share, 1) === COLON) {
        drive = share;
      }
      return { device, drive, absolute: true, end };
    }
  }
  if (hasNamespacePrefix(path)) {
    throw pathError('PATH_WINDOWS_ROOT', 'provide a complete Windows namespace root');
  }
  return { device: '', drive: '', absolute: true, end: 1 };
}

/** Resolve on one drive/share, without consulting any per-drive environment. */
function resolveWin32(...paths) {
  for (let i = 0; i < paths.length; ++i) assertPath(paths[i]);
  let device = '';
  let driveRelative = false;
  let absolute = false;
  const tails = [];
  for (let i = paths.length - 1; i >= -1; --i) {
    const path = i === -1 ? requireCwd(true) : paths[i];
    if (path === '') continue;
    const root = resolutionRootWin32(path);
    if (root.device !== '') {
      if (device === '') {
        device = root.device;
        driveRelative = !root.absolute;
      } else if (lower(device) !== lower(root.device)) {
        // Windows canonicalization can store a verbatim drive cwd. Match a
        // drive-relative input to this drive, but keep the verbatim spelling.
        if (driveRelative && lower(device) === lower(root.drive)) device = root.device;
        else continue;
      }
    }
    if (!absolute) {
      tails[tails.length] = slice(path, root.end);
      absolute = root.absolute;
    }
    if (absolute && device !== '') break;
  }
  if (!absolute || device === '') {
    throw pathError('PATH_WINDOWS_ROOT', 'provide an absolute Windows base on the requested drive or share');
  }
  return device + '\\' + normalizedTail(tails, '\\', isWin32Separator);
}

/** Compare whole components, retaining the destination's original spelling. */
function relativeTail(from, to, separator, foldCase) {
  const source = from === '' ? [] : split(from, separator);
  const target = to === '' ? [] : split(to, separator);
  let i = 0;
  while (i < source.length && i < target.length &&
    (foldCase ? lower(source[i]) === lower(target[i]) : source[i] === target[i])) ++i;
  const result = [];
  for (let j = i; j < source.length; ++j) result[result.length] = '..';
  for (; i < target.length; ++i) result[result.length] = target[i];
  const path = joinArray(result, separator);
  // A destination component that starts with a drive letter and colon
  // must not turn this relative result into a drive-relative path.
  return foldCase && isDeviceRoot(charCodeAt(path, 0)) && charCodeAt(path, 1) === COLON
    ? '.\\' + path : path;
}

function relativePosix(from, to) {
  assertPath(from);
  assertPath(to);
  return relativeTail(slice(resolvePosix(from), 1), slice(resolvePosix(to), 1), '/', false);
}

function relativeWin32(from, to) {
  assertPath(from);
  assertPath(to);
  const source = resolveWin32(from);
  const target = resolveWin32(to);
  const sourceRoot = resolutionRootWin32(source);
  const targetRoot = resolutionRootWin32(target);
  if (lower(sourceRoot.device) !== lower(targetRoot.device)) return target;
  return relativeTail(
    slice(source, skipWin32Separators(source, sourceRoot.end)),
    slice(target, skipWin32Separators(target, targetRoot.end)),
    '\\',
    true,
  );
}

function toNamespacedPathWin32(path) {
  assertPath(path);
  if (path === '' || hasNamespacePrefix(path)) return path;
  const resolved = resolveWin32(path);
  if (hasNamespacePrefix(resolved)) return resolved;
  if (charCodeAt(resolved, 0) === BACKSLASH) return '\\\\?\\UNC\\' + slice(resolved, 2);
  return '\\\\?\\' + resolved;
}

const posix = freeze({
  sep: '/',
  delimiter: ':',
  join: joinPosix,
  normalize: normalizePosix,
  basename: basenamePosix,
  dirname: dirnamePosix,
  extname: extnamePosix,
  isAbsolute: isAbsolutePosix,
  parse: parsePosix,
  format: formatPosix,
  common: (paths) => commonPath(paths, '/'),
  resolve: resolvePosix,
  relative: relativePosix,
  toNamespacedPath: assertPath,
});

const win32 = freeze({
  sep: '\\',
  delimiter: ';',
  join: joinWin32,
  normalize: normalizeWin32,
  basename: basenameWin32,
  dirname: dirnameWin32,
  extname: extnameWin32,
  isAbsolute: isAbsoluteWin32,
  parse: parseWin32,
  format: formatWin32,
  common: (paths) => commonPath(paths, '\\'),
  resolve: resolveWin32,
  relative: relativeWin32,
  toNamespacedPath: toNamespacedPathWin32,
});

const native = hostWindows ? win32 : posix;
const { join, normalize, basename, dirname, extname, isAbsolute, parse, format,
  common, resolve, relative, toNamespacedPath } = native;
return {
  posix, win32, join, normalize, basename, dirname, extname, isAbsolute, parse, format,
  common, resolve, relative, toNamespacedPath,
  SEPARATOR: native.sep,
  DELIMITER: native.delimiter,
  // Public RegExp is intentionally independent of the internal algorithms.
  SEPARATOR_PATTERN: hostWindows ? /[\\/]+/ : /\/+/,
};
