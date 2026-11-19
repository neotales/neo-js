---
title: "Path utilities"
description: "Lexical POSIX and Windows path operations, errors, and limits."
---

`neo:path` provides synchronous, purely lexical operations for POSIX and
Windows path syntax. It needs no permission flag and grants no host capability:
no function reads or writes files, the working directory, environment
variables, the locale, or the platform, and none resolves symlinks, junctions,
drive mappings, or short names, checks existence, folds case, or normalizes
Unicode. The two namespaces are selected explicitly, so a Linux host can parse
Windows paths and a Windows host can parse POSIX paths. There is no host-default
namespace and no `resolve`, because resolving a relative path would read the
working directory.

```js
import { posix, win32 } from "neo:path";

posix.join("/srv", "app", "..", "data"); // "/srv/data"
posix.dirname("/srv/data/file.txt"); // "/srv/data"
posix.basename("/srv/data/file.txt"); // "file.txt"
posix.extname("/srv/data/file.txt"); // ".txt"
posix.parse("/srv/data/file.txt");
// { root: "/", dir: "/srv/data", base: "file.txt", ext: ".txt", name: "file" }

win32.normalize("C:/work/out/../log"); // "C:\\work\\log"
win32.parse("\\\\server\\share\\file");
// { root: "\\\\server\\share\\", dir: "\\\\server\\share\\",
//   base: "file", ext: "", name: "file" }
```

Each namespace exports `join`, `normalize`, `basename`, `dirname`, `extname`,
`isAbsolute`, `parse`, `format`, `sep`, and `delimiter`. `sep` is `/` for
`posix` and `\` for `win32`; `delimiter` is `:` or `;` and is only a constant,
not used by the functions. The namespaces are frozen. Every function is
synchronous and repeatable and returns a string or a fresh plain object;
`parse` results are not shared or cached.

## Arguments and errors

Every path must be a primitive string. A boxed `String`, `URL`, buffer, number,
`null`, `undefined`, or other value throws `TypeError`; nothing is coerced.
Windows functions accept both `\` and `/` as separators on input, but the
argument is still a string. `basename`'s optional suffix must be a string.
`format` requires a non-null, non-array object, and each of `root`, `dir`,
`base`, `ext`, and `name` must be a string or absent. A NUL (U+0000) in any
argument throws `TypeError`: such a string cannot name a usable file, and
returning a lexical result that contains one would look usable when it is not.
Error messages name the rejected argument and never repeat its value. These are
the module's only errors; a getter or Proxy trap on a `format` argument may
throw its own. There are no I/O, permission, or host failures.

Empty input follows the conventional dot where a path is required:
`normalize("")`, `join()` with no non-empty segment, and `dirname("")` are
`"."`. `basename("")` and `extname("")` are empty, `parse("")` has five empty
fields, `isAbsolute("")` is false, and `format({})` is empty.

## POSIX rules

- The separator is `/`, and only `/`. A backslash is an ordinary filename
  character. `sep` is `/` and `delimiter` is `:`.
- `normalize` collapses runs of `/`, removes `.` segments, and applies `..` to
  the preceding segment. Leading `..` is preserved in a relative path and
  dropped at an absolute root. At most one trailing `/` is preserved, so
  `normalize("a/../")` is `"./"`. `//` and `///` collapse to `/`; this module
  does not implement a distinct implementation-defined `//` network root.
- `join` validates every argument before dropping empty ones, concatenates
  with `/`, and normalizes. It does not restart at an absolute later segment:
  `join("a", "/b")` is `"a/b"`, not `"/b"`.
- `basename` ignores trailing separators. A path that is only separators,
  including `/`, has an empty basename. The suffix is removed only when it is
  non-empty, shorter than the name, and matches exactly, so
  `basename("a.txt", "a.txt")` is unchanged.
- `dirname` returns the text before the final separator with trailing
  separators removed: `dirname("a//b")` is `"a"`. A path that is only
  separators returns `/`; an empty or relative path with no parent returns
  `"."`.
- `extname` ignores trailing separators, including after the final component,
  so `extname("a/b.txt/")` is `".txt"`. Dot files (`".bashrc"`), `.`, `..`,
  and paths with no dot have no extension. `a.` and `a..` have extension `"."`;
  the last dot of the final segment starts the extension.
- `isAbsolute` is true only for a non-empty path whose first code unit is `/`.
- `parse` does not normalize the whole path. `root` is `/` for an absolute
  path, otherwise empty. `base` is empty when the final component is empty
  (for example `/` or an empty path). `dir` is the root, or the text before the
  final separator with trailing separators removed, so `parse("a//b").dir` is
  `"a"`.
- `format` joins `dir` (or `root` when `dir` is empty) and `base` (or
  `name + ext` when `base` is empty) with `/`. Empty strings are treated as
  absent for `dir`, `root`, and `base`; `base` wins over `name` and `ext`. It
  does not normalize or validate consistency. A non-empty `dir` with an empty
  `base` produces a trailing separator (`format({ dir: "a" })` is `"a/"`). A
  non-empty `base` is appended as supplied, even when it is `/`:
  `format({ dir: "foo", base: "/" })` is `"foo//"`, never a silently dropped
  `base`.

## Windows rules

- Both `\` and `/` are separators on input; `join`, `normalize`, and `format`
  produce `\`. `sep` is `\` and `delimiter` is `;`.
- Roots are recognized in this order: a UNC root `\\server\share` with a
  non-empty server and share; an ASCII drive root `C:\` (letter, colon,
  separator); a drive-relative root `C:` (letter and colon, no separator); a
  rooted path starting with one separator; otherwise a relative path. Drive
  letters must be ASCII `A`-`Z` or `a`-`z`, and case is preserved. A colon is
  ordinary text unless exactly one ASCII letter precedes it at the start, so
  `1:\foo` is relative and normalizes to itself. Server and share names are
  opaque: any non-separator characters are accepted and no name, share, or
  device validation is performed.
- Exactly two leading separators followed by a non-empty server and share form
  a UNC root. Three or more leading separators, or a `\\` prefix with a missing
  server or share, are not UNC: they collapse to one rooted separator. A UNC
  root alone normalizes to `\\server\share\`.
- `normalize` converts separators to `\`, collapses runs, removes `.`, applies
  `..`, preserves the root, and preserves at most one trailing `\`. `..` is
  preserved above a relative or drive-relative root and dropped at an absolute
  root. A bare drive normalizes to `C:.`; `C:..` is preserved. No per-drive
  working directory is consulted.
- `join` works like POSIX `join` but takes care not to turn a leading run of
  separators into an unintended UNC root. The first non-empty segment may
  itself be a UNC path, so `join("//server", "share")` is
  `"\\\\server\\share\\"`. Joining a bare drive with a segment makes it rooted:
  `join("C:", "foo")` is `"C:\\foo"`.
- `basename` recognizes both separators and skips a drive prefix, so `C:\` and
  `C:` have an empty basename. A complete UNC root also has an empty basename,
  even when its share contains a dot: `basename("\\\\server\\share.txt\\")` is
  `""`; the share is not treated as a file component.
- `dirname`, `extname`, `isAbsolute`, `parse`, and `format` follow the root
  rules above. `dirname` of a root-only path returns the root as written, so
  `C:/` stays `C:/`, and `dirname("\\\\server\\share\\foo")` is
  `"\\\\server\\share\\"`. Trailing separators are removed from the returned
  parent when it is not a root, so `dirname("a\\\\b")` is `"a"`. `extname`
  ignores trailing separators, so `extname("C:\\foo\\bar.txt\\")` is `".txt"`;
  a complete UNC root has an empty extension because its share is not a file
  component.
- `parse` and `dirname` preserve the input's separator characters in the
  substrings they return. `parse` removes trailing separators from `dir`
  except a root's own separator, so `parse("a//b").dir` is `"a"`. `base` is
  empty when there is no final component, as for `C:`, `C:/`, or a UNC root
  with nothing after the share.
- `format` follows the same joining rule with `\`. A non-empty `base` is
  appended as supplied, even when it is `\`; it is never silently dropped, and
  the result is not normalized.
- `\\?\` and `\\.\` receive no special treatment. `\\\\?\\C:\\foo` parses as a
  UNC-shaped root (`\\\\?\\C:\\`) followed by `foo`, and
  `\\\\?\\UNC\\server\\share\\foo` parses with root `\\\\?\\UNC\\`.
  Extended-length translation, device enumeration, reserved device names, and
  alternate data streams are not implemented.

The Windows rules are pure string logic and are exercised on Linux by the test
suite; no Windows host was used to run it.

## Round trips and deliberate differences

`format(parse(path))` preserves the path's components and spelling in the
common case, but it is not a byte-for-byte round trip: a trailing separator is
dropped, and redundant separators around `dir` are collapsed because `parse`
trims them. `posix.normalize` is idempotent. `win32.normalize` is idempotent
except when a relative input reduces to a bare drive-relative name:
`normalize("./a:")` is `"a:"`, which normalizes again to `"a:."`. No `.\` guard
is added, so treat a normalized relative Windows path as a new path rather than
a fixed point. POSIX `join` is exactly concatenation followed by `normalize`;
the Windows `join` applies its UNC guard first.

This module is not a drop-in replacement for Node's `path` or
`@neotales/path`. `resolve`, `relative`, `common`, `toFileUrl`, `fromFileUrl`,
`toNamespacedPath`, and the glob helpers are absent. URL inputs, buffers, and
type coercion are not accepted. Windows long-path translation, device names,
case folding, and `\\?\` handling are absent, as is POSIX `//` root
preservation. For a path that is a drive root or a complete UNC root,
`basename` and the `parse` component fields are empty rather than a separator,
and Windows `dirname("C:/")` preserves the root as written instead of stripping
its separator. There is no asynchronous form.

The path algorithms are adapted from `@neotales/path` under the MIT license.
See `crates/neo-js/src/js/path.LICENSE.md` for the exact upstream revision, the
adapted-file list, and the complete upstream copyright and permission notices.
The same text is embedded in the library and binary: print it with
`neo-js --third-party-notices` or call `neo_js::third_party_notices()`. It is
not a complete inventory of every dependency's license.

## Security: not a safe jail

`neo:path` is lexical string manipulation, not a security boundary. It does not
validate against traversal, symlinks or junctions, case-insensitive aliasing,
Windows reserved device names, alternate data streams, Unicode confusables, or
network shares, and it does not check that a result stays inside a directory.
`normalize("a/../../b")` is `"../b"`; on a real filesystem a symlink can also
redirect a path that looks contained. Do not use this module to decide
containment, and do not treat a normalized path as authorized input to a file
operation. Enforce containment with the operating system after resolving the
real path and checking the opened object, under the process's existing
permissions. `neo:fs` and `neo:exec` do not consult this module.

Declarations: [`neo:path`](/api/modules/path/).
