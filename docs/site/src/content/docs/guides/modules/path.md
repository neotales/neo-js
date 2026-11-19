---
title: "Path utilities"
description: "Lexical POSIX and Windows path operations, errors, and limits."
---

`neo:path` provides synchronous, lexical operations for POSIX and Windows
paths. Importing it requires no permission flag. Choose `posix` or `win32` to
use a fixed dialect on any host. The other named exports select the host
dialect: Windows on Windows, POSIX elsewhere. There is no default export.

The module does not read or write files, check existence, resolve symlinks,
junctions, drive mappings, or short names, or read environment variables.
It does not normalize Unicode. Operations that need an implicit resolution base
may read the runtime-local working directory under the rules below.

```js
import { posix, win32, SEPARATOR, SEPARATOR_PATTERN } from "neo:path";

["app", "data"].join(SEPARATOR); // "app/data" or "app\\data"
"app///data".split(SEPARATOR_PATTERN); // ["app", "data"]

posix.join("/srv", "app", "..", "data"); // "/srv/data"
posix.dirname("/srv/data/file.txt"); // "/srv/data"
posix.basename("/srv/data/file.txt"); // "file.txt"
posix.extname("/srv/data/file.txt"); // ".txt"
posix.parse("/srv/data/file.txt");
// { root: "/", dir: "/srv/data", base: "file.txt", ext: ".txt", name: "file" }

win32.normalize("C:/work/out/../log"); // "C:\\work\\log"
win32.resolve("C:/work/out", "../log"); // "C:\\work\\log", on any host
win32.parse("\\\\server\\share\\file");
// { root: "\\\\server\\share\\", dir: "\\\\server\\share\\",
//   base: "file", ext: "", name: "file" }
```

The frozen namespaces both provide `join`, `normalize`, `basename`, `dirname`,
`extname`, `isAbsolute`, `parse`, `format`, `common`, `resolve`, `relative`,
and `toNamespacedPath`. Each is also available as a host-selected named export.
`SEPARATOR` and `DELIMITER` match the selected namespace's `sep` and
`delimiter`: `/` and `:` on POSIX, `\` and `;` on Windows. The delimiter is
a path-list constant. The functions do not use it. `SEPARATOR_PATTERN` is a
regular expression that matches runs of host separators. On Windows it
matches both `/` and `\`. `parse` returns a fresh plain object on each call.

## Arguments and errors

Every path must be a primitive string. A boxed `String`, `URL`, buffer, number,
`null`, `undefined`, or other value throws `TypeError`; nothing is coerced.
Windows functions accept both `\` and `/` as separators on input, except
`common`, which splits on `\` only. `common` takes an array of paths.
`basename`'s optional suffix must be a string.
`format` requires a non-null, non-array object, and each of `root`, `dir`,
`base`, `ext`, and `name` must be a string or absent. A NUL (U+0000) in any
argument throws `TypeError`: such a string cannot name a usable file, and
returning a lexical result that contains one would look usable when it is not.
The module's error messages do not repeat the rejected path. Getters and
Proxy traps on a `format` argument or `common` array can throw their own errors.

Empty input follows the conventional dot where a path is required:
`normalize("")`, `join()` with no non-empty segment, and `dirname("")` are
`"."`. `basename("")` and `extname("")` are empty, `parse("")` has five empty
fields, `isAbsolute("")` is false, and `format({})` is empty.

## Common prefixes and resolution

- `common(paths)` compares components as written, without normalizing or
  folding case. It retains the separator when paths diverge after a shared
  component. It returns `""` if the paths share no prefix or the array is empty.
- `resolve(...paths)` works from right to left. It validates every segment,
  ignores empty strings, and eliminates `.` and `..`. The result is absolute
  and has no trailing separator except at a root. On Windows, `\child`
  needs to inherit a drive or share even though `isAbsolute("\\child")` is
  true. A drive-relative path such as `C:child` needs a base on the same drive.
  There is no per-drive environment lookup or implicit drive-root fallback.
- `relative(from, to)` resolves both operands and returns the path from `from`
  to `to`, or `""` if they resolve to the same path. Windows compares roots and
  components case-insensitively but preserves destination spelling. It returns
  the absolute destination if the drive, share, or namespace differs.
- POSIX `toNamespacedPath(path)` returns the validated string unchanged.
  Windows converts resolved drive paths to `\\?\` form and UNC paths to
  `\\?\UNC\` form. It leaves empty, already namespaced, and device paths
  unchanged. It does not validate paths for Windows I/O.

## Runtime working directory

An explicit, fully qualified base for `resolve`, or fully qualified operands
for `relative`, need no permission flag. This works even in the non-host
dialect. On a POSIX host, for example, `win32.resolve("C:\\work", "C:child")`
works without a flag. `win32.resolve("C:child")` needs an explicit base on
that host. To convert a relative Windows path with `toNamespacedPath`, pass an
absolute path first or resolve it with an explicit Windows base.

When `resolve` (including `resolve()`), `relative`, or Windows
`toNamespacedPath` needs an implicit base, only the host dialect can read the
runtime-local working directory. Either of these pairs is sufficient:

- Filesystem permission (`--allow-fs`) and an allowlist that admits `neo:fs`.
- Process permission (`--allow-run`) or standard I/O permission
  (`--allow-stdio`), and an allowlist that admits `neo:process`.

Admitting `neo:path` alone does not grant cwd access. The fallback observes
`neo:process.chdir()`. It does not change, reread, or fall back to the host
process working directory. See [Host permissions and module access](/runtime/permissions/).

| Error code          | Meaning                                                                                        |
| ------------------- | ---------------------------------------------------------------------------------------------- |
| `PATH_PERMISSION`   | The operation needs the runtime cwd, but neither qualifying permission pair exists.            |
| `PATH_CWD`          | The operation may read the runtime cwd, but the cwd is unavailable.                            |
| `PATH_CWD_DIALECT`  | The non-host dialect cannot use cwd fallback. Supply an explicit base.                         |
| `PATH_WINDOWS_ROOT` | The operation could not obtain a matching drive or share, or the namespace root is incomplete. |

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
- `parse` and `normalize` retain their UNC-shaped treatment of `\\?\` and
  `\\.\`. For example, `parse` reports `\\?\UNC\` as the root of
  `\\?\UNC\server\share\foo`. In contrast, `resolve` treats the full
  namespaced server and share as one root. The module does not enumerate
  devices or validate reserved device names or alternate data streams.

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
`@neotales/path`. `toFileUrl`, `fromFileUrl`, and glob helpers are not
implemented. The module does not accept URL objects or buffers or coerce
types. It does not preserve the POSIX `//` root. For a drive root or a
complete UNC root,
`basename` and the `parse` component fields are empty rather than a separator,
and Windows `dirname("C:/")` preserves the root as written instead of stripping
its separator. There is no asynchronous form.

The path algorithms are adapted from `@neotales/path` under the MIT license.
See `crates/neo-js/src/js/path.LICENSE.md` for the exact upstream revision, the
adapted-file list, and the complete upstream copyright and permission notices.
The same text is embedded in the library and binary: print it with
`neo-js --third-party-notices` or call `neo_js::third_party_notices()`. It is
not a complete inventory of every dependency's license.

## Path operations do not enforce containment

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
