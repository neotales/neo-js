/// <reference lib="esnext.disposable" />

// Core declarations for neojs-core. Optional modules owned by separate crates
// are added by composition; see types/README.md. This file deliberately omits
// `neo:os`, the scheduler generators, and `neo:sqlite`, because neojs-core
// does not compile or register them.

/** Available only with --allow-env. */
declare const env: {
  get(name: string): string | undefined;
  has(name: string): boolean;
};

/**
 * The `neo:fs` API. Available only with `--allow-fs` and when the runtime's
 * module allowlist admits `neo:fs`. Relative paths use the runtime's working
 * directory, not the host process's. Only `readFile` is asynchronous.
 *
 * This is not a sandbox: a script with this object has the host user's
 * filesystem authority. Reads and writes are bounded; see the `neo:fs`
 * declaration for the limits and the symlink and deletion rules.
 */
declare const fs: {
  readText(path: string): string;
  /** Same text validation and error precedence as `neo:fs.writeText`. */
  writeText(path: string, data: string): void;
  /**
   * Read bytes during managed module evaluation. Outside it, this throws
   * `FS_ASYNC_OUTSIDE_TURN` synchronously before I/O. At most four reads are
   * outstanding and two can run on workers. There is no caller cancellation
   * API; teardown cancels cooperatively and may wait for an OS call.
   */
  readFile(path: string): Promise<Uint8Array>;
  readFileSync(path: string): Uint8Array;
  writeFile(
    path: string,
    data: Uint8Array,
    options?: import("neo:fs").WriteFileOptions,
  ): void;
  stat(path: string): import("neo:fs").FileInfo;
  lstat(path: string): import("neo:fs").FileInfo;
  exists(path: string): boolean;
  readDir(path: string): import("neo:fs").DirEntry[];
  mkdir(path: string, options?: import("neo:fs").MkdirOptions): void;
  remove(path: string, options?: import("neo:fs").RemoveOptions): void;
  /** Same entry, replacement, and error semantics as `neo:fs.rename`. */
  rename: typeof import("neo:fs").rename;
  /** Same bounded, no-clobber copy as `neo:fs.copyFile`. */
  copyFile: typeof import("neo:fs").copyFile;
  FsError: typeof import("neo:fs").FsError;
  MAX_PATH_BYTES: number;
  MAX_READ_BYTES: number;
  MAX_WRITE_BYTES: number;
  MAX_LIST_ENTRIES: number;
  MAX_LIST_NAME_BYTES: number;
  MAX_LIST_TOTAL_BYTES: number;
};

declare module "neo:env" {
  /**
   * Read one environment variable. Returns `undefined` when it is absent or
   * its value is not valid Unicode. Requires --allow-env.
   */
  export function get(name: string): string | undefined;
  /**
   * Test whether one environment variable is present, including one whose
   * value is not valid Unicode. Requires --allow-env.
   */
  export function has(name: string): boolean;
}

/**
 * Bounded filesystem access. Requires `--allow-fs` and a runtime
 * allowlist that admits `neo:fs`.
 *
 * During managed module evaluation, `readFile` reads on a worker and returns
 * a `Promise<Uint8Array>`. `readFileSync` and the other operations block the
 * JavaScript thread. Relative paths resolve against the runtime's working
 * directory, with no fallback to the host directory. If that directory is
 * unavailable, `readFile` rejects with `FS_CWD` during a managed turn and
 * synchronous operations throw `FS_CWD`. Outside a managed turn, `readFile`
 * instead throws `FS_ASYNC_OUTSIDE_TURN` before I/O. Absolute paths and `..`
 * components still reach everything the host user can reach. This is not a
 * sandbox.
 *
 * `readFile`, `readFileSync`, `readText`, `writeFile`, and `writeText` require a regular file.
 * An open may occur before a non-regular object is rejected. If the type is
 * identified, the error is `FS_IS_DIRECTORY` or `FS_TYPE`; an open failure
 * can take precedence. Device-open side effects and latency are not bounded.
 * Reads are bounded at `MAX_READ_BYTES` and writes at `MAX_WRITE_BYTES`
 * before an unbounded allocation. A path is
 * bounded at `MAX_PATH_BYTES`. `FS_LIMIT` also covers quota, storage-full,
 * and fallible allocation failures; this cannot catch every failure inside
 * QuickJS or the process allocator.
 *
 * `stat`, `readFile`, `readFileSync`, `readText`, `writeFile`, `writeText`, `exists`, and
 * `readDir` follow a symlink at the final path component. `lstat` does not.
 * `readDir` does not follow entries and reports a symlink entry with
 * `isSymlink: true`. `remove` never follows a symlink at the target, and
 * recursive deletion does not follow symlinks inside the tree.
 * `remove` rejects parsed roots (including Windows UNC share roots), paths
 * ending in a separator, and final `.` or `..` components with `FS_PATH`.
 * A Windows directory symlink named directly as the target is rejected with
 * `FS_UNSUPPORTED`, even if dangling.
 *
 * Options accept only the documented own, enumerable data properties.
 * Accessors, non-enumerable properties, and unknown keys throw `TypeError`;
 * inherited properties are ignored. A Proxy can run traps during reflective
 * validation. The runtime-local absolute path is snapshotted before those
 * traps run, so a trap that changes `neo:process.chdir()` cannot redirect that
 * call. A trap may still reenter the API or run arbitrary script code.
 *
 * `readDir` is not an atomic snapshot, and recursive deletion can be partial
 * if an OS error interrupts traversal. `exists` and `stat` are
 * time-of-check/time-of-use snapshots. Do not use them
 * for access-control or race-free decisions. `writeFile` and `writeText`
 * truncate before writing and are not atomic; a failure can leave a partial
 * file. No file locking is performed. The opened descriptor is verified
 * before truncation. Unix data opens (including copy) use `O_NONBLOCK` to
 * avoid waiting for a FIFO peer and `O_NOCTTY` to avoid acquiring a controlling
 * terminal. Other device-open side effects remain possible, and device and
 * network filesystem latency is not bounded.
 *
 * Filesystem failures are `FsError` values with a stable `code` and fixed
 * messages that never contain the supplied path or an OS error string.
 * An argument of the wrong type throws `TypeError`; exceptions from option
 * proxy traps can propagate.
 */
declare module "neo:fs" {
  /**
   * Stable failure codes reported by `FsError.code`. `FS_PATH` includes
   * invalid or unsupported path syntax and, for `remove`, `rename`, and `copyFile`,
   * parsed roots, trailing separators, and final `.` or `..` components.
   */
  export type FsErrorCode =
    | "FS_PATH"
    | "FS_NOT_FOUND"
    | "FS_NOT_DIRECTORY"
    | "FS_IS_DIRECTORY"
    | "FS_ALREADY_EXISTS"
    | "FS_NOT_EMPTY"
    | "FS_CROSS_DEVICE"
    | "FS_DENIED"
    | "FS_LIMIT"
    | "FS_ENCODING"
    | "FS_DETACHED"
    | "FS_OUT_OF_BOUNDS"
    | "FS_CWD"
    | "FS_TYPE"
    | "FS_UNSUPPORTED"
    | "FS_ASYNC_OUTSIDE_TURN"
    | "FS_CANCELLED"
    | "FS_IO";

  /**
   * A filesystem failure. `code` is stable; `message` is fixed text and does
   * not include the supplied path or file contents.
   */
  export class FsError extends Error {
    readonly code: FsErrorCode;
  }

  /**
   * A metadata snapshot from `stat` or `lstat`.
   *
   * `size` is a decimal string of the exact byte size, never a JavaScript
   * number, so a `u64` value above `Number.MAX_SAFE_INTEGER` keeps every
   * digit. For a symlink reported by `lstat`, `size` is the host's symlink
   * size, not the target's size. `mtimeMs` is whole milliseconds since the
   * Unix epoch as a decimal string, truncated toward zero (a pre-epoch time
   * less than one millisecond from 1970 is `"0"`), or `null` when the host
   * cannot report the modification time.
   *
   * `stat` follows a final symlink, so its result has `isSymlink: false`.
   * `lstat` does not, so a link reports `isSymlink: true` with `isFile` and
   * `isDirectory` both `false`. A non-regular path that is neither a file, a
   * directory, nor a symlink has all three flags `false`.
   */
  export interface FileInfo {
    readonly isFile: boolean;
    readonly isDirectory: boolean;
    readonly isSymlink: boolean;
    readonly size: string;
    readonly mtimeMs: string | null;
  }

  /** One entry from `readDir`. The type is the entry itself, never its target. */
  export interface DirEntry {
    readonly name: string;
    readonly isFile: boolean;
    readonly isDirectory: boolean;
    readonly isSymlink: boolean;
  }

  /** Options for `writeFile`. `writeText` has no options and always overwrites. */
  export interface WriteFileOptions {
    /**
     * `true` (the default) creates or truncates the file. `false` uses
     * create-new semantics and throws `FS_ALREADY_EXISTS` if the path already
     * exists, including a dangling symlink at the path. The decision is made
     * by the open call, not a prior `exists` check.
     */
    overwrite?: boolean;
  }

  /** Options for `mkdir`. */
  export interface MkdirOptions {
    /**
     * Create missing parent directories. With `true`, an existing directory
     * is accepted. No `mode` option is provided; the host default and umask
     * apply.
     */
    recursive?: boolean;
  }

  /** Options for `remove`. */
  export interface RemoveOptions {
    /**
     * Remove a directory and everything below it, without following symlinks.
     * Without this option a non-empty directory
     * throws `FS_NOT_EMPTY`. There is no count limit and no undo.
     */
    recursive?: boolean;
  }

  /** Largest accepted path, in UTF-8 bytes. */
  export const MAX_PATH_BYTES: number;
  /** Largest accepted read or copy, in bytes. */
  export const MAX_READ_BYTES: number;
  /** Largest accepted binary or text write, in bytes. */
  export const MAX_WRITE_BYTES: number;
  /** Largest number of entries `readDir` returns. */
  export const MAX_LIST_ENTRIES: number;
  /** Largest accepted single directory-entry name, in UTF-8 bytes. */
  export const MAX_LIST_NAME_BYTES: number;
  /** Largest sum of directory-entry names `readDir` returns, in UTF-8 bytes. */
  export const MAX_LIST_TOTAL_BYTES: number;

  /**
   * Read a regular file as UTF-8 text.
   *
   * The file must be a regular file. Files larger than `MAX_READ_BYTES`
   * throw `FS_LIMIT` before the file is read. Invalid UTF-8 throws
   * `FS_ENCODING`. A missing path throws `FS_NOT_FOUND`; a directory or
   * other non-regular object may be opened before rejection, and the OS
   * open error can take precedence. This preserves the original `readText`
   * signature; the new bound and `FsError` shape are the documented migration.
   */
  export function readText(path: string): string;

  /**
   * Write UTF-8 text, creating the file or truncating an existing one.
   *
   * Parent directories are not created. A final symlink is followed, so a
   * dangling link can create its target. Writing is not atomic.
   * Text exceeding `MAX_WRITE_BYTES` in UTF-16 code units first throws
   * `FS_LIMIT`. Otherwise any unpaired surrogate throws `FS_ENCODING`,
   * even if the text also exceeds the UTF-8 byte limit. Well-formed text
   * exceeding `MAX_WRITE_BYTES` in UTF-8 bytes then throws `FS_LIMIT`.
   */
  export function writeText(path: string, data: string): void;

  /**
   * Read a regular file on a worker, using the runtime directory captured
   * at the call. Only available during managed module evaluation; otherwise
   * throws `FS_ASYNC_OUTSIDE_TURN` before I/O. In a managed turn, invalid
   * arguments and I/O errors reject the Promise. A module turn accepts up
   * to four outstanding reads (including undelivered results) and runs up
   * to two on workers. Each outstanding read charges `MAX_READ_BYTES` plus
   * the encoded byte length of its resolved path against a turn budget of
   * `4 * (MAX_READ_BYTES + MAX_PATH_BYTES)`. Excess work rejects with
   * `FS_LIMIT`. This budget does not cover buffers JavaScript retains after
   * delivery. There is no caller cancellation API. Turn teardown cancels
   * cooperatively and may wait for an OS call to return. Use `readFileSync`
   * to block.
   */
  export function readFile(path: string): Promise<Uint8Array>;

  /**
   * Read a regular file as bytes, blocking the JavaScript thread.
   *
   * Returns a `Uint8Array` view over a new buffer. The same regular-file and
   * `MAX_READ_BYTES` rules as `readText` apply. The result is a copy, not a
   * live view of the file.
   */
  export function readFileSync(path: string): Uint8Array;

  /**
   * Synchronously stream bytes from a regular file into a new regular file.
   * Both relative paths use the same runtime cwd. Roots, trailing separators,
   * and final `.` or `..` components throw `FS_PATH`. On Windows, copy rejects
   * UNC, device, drive-relative and drive-less rooted paths, reserved device names, alternate
   * data streams, and ambiguous trailing dots or spaces in components.
   *
   * Unlike the reference copy API, this never overwrites or copies metadata.
   * The source may be a symlink to a regular file; exclusive creation
   * refuses an existing destination (`FS_ALREADY_EXISTS` when reported by
   * the OS), including aliases and dangling links. Parents are not created.
   * Copy can span filesystems. The limit is `MAX_READ_BYTES`; an oversize
   * source detected at open throws `FS_LIMIT` before creating anything.
   *
   * This is not atomic or durable. A failure after creation (including
   * growth past the limit) can leave a partial file, which is not removed.
   * Concurrent renames or writes can change which paths or bytes are seen.
   * No-clobber depends on the filesystem honoring exclusive creation.
   */
  export function copyFile(source: string, destination: string): void;

  /**
   * Write bytes to a regular file.
   *
   * The input must be a `Uint8Array`; a `DataView`, another typed array, or a
   * `Proxy` throws `TypeError`. A detached buffer throws `FS_DETACHED`, an
   * out-of-bounds view throws `FS_OUT_OF_BOUNDS`, and a `SharedArrayBuffer`
   * view throws `FS_UNSUPPORTED`. The view's live bytes are copied before
   * reflecting on options. By default the file is created or truncated.
   * Parent directories are not created, and the write is not atomic.
   */
  export function writeFile(
    path: string,
    data: Uint8Array,
    options?: WriteFileOptions,
  ): void;

  /**
   * Metadata for a path, following a final symlink.
   *
   * See `FileInfo` for the decimal-string size and time representation.
   */
  export function stat(path: string): FileInfo;

  /**
   * Metadata for a path without following a final symlink.
   *
   * A symlink is reported with `isSymlink: true`.
   */
  export function lstat(path: string): FileInfo;

  /**
   * Test whether a path exists, following a final symlink.
   *
   * Returns `false` for a missing path, a dangling symlink, or a path below a
   * non-directory. Other failures, notably permission denial, throw. This is
   * a snapshot and must not be used for race-free access-control decisions.
   */
  export function exists(path: string): boolean;

  /**
   * List a directory, non-recursively.
   *
   * Entries are sorted by name in UTF-8 byte order (Unicode code-point
   * order). Entry types are not followed: a symlink is reported as
   * `isSymlink: true`. A name that is not valid UTF-8 throws `FS_ENCODING`.
   * The listing is bounded by `MAX_LIST_ENTRIES`, `MAX_LIST_NAME_BYTES`, and
   * `MAX_LIST_TOTAL_BYTES`. A final symlink to a directory is followed.
   */
  export function readDir(path: string): DirEntry[];

  /**
   * Create a directory.
   *
   * Without `recursive`, an existing path throws `FS_ALREADY_EXISTS` and a
   * missing parent throws `FS_NOT_FOUND`. With `recursive: true`, missing
   * parents are created and an existing directory is accepted. No `mode`
   * option is provided.
   */
  export function mkdir(path: string, options?: MkdirOptions): void;

  /**
   * Remove a file, symlink, or directory.
   *
   * Without `recursive`, a non-empty directory throws `FS_NOT_EMPTY`. With
   * `recursive: true`, the whole tree is removed without following symlinks.
   * A final symlink is removed as a link, except that a Windows directory
   * symlink is rejected with `FS_UNSUPPORTED`. Parsed roots, trailing
   * separators, and final `.` or `..` components throw `FS_PATH`. There is
   * no undo and no count limit.
   */
  export function remove(path: string, options?: RemoveOptions): void;

  /**
   * Synchronously rename a file, directory, or symlink entry. Both relative
   * paths use the same runtime working directory. Final symlinks are not
   * followed, and replacement follows the host OS rules.
   * A rejected replacement of a non-empty directory can throw
   * `FS_ALREADY_EXISTS` or `FS_NOT_EMPTY`, depending on the OS error.
   *
   * Both paths must name specific entries: parsed roots, trailing separators,
   * final `.` or `..`, and Windows drive-relative paths throw `FS_PATH`.
   * Neither path is canonicalized on disk; joining a relative path to a
   * Windows verbatim working directory can normalize intermediate components
   * lexically.
   * The runtime never pre-deletes the destination, creates parents, or falls
   * back to copy/delete. An OS cross-device error throws `FS_CROSS_DEVICE`.
   * Paths can change between resolution and use; there is no sandbox, rollback,
   * or guarantee of the outcome after an OS or network filesystem failure.
   */
  export function rename(source: string, destination: string): void;
}

/**
 * Semantic version parsing, formatting, incrementing, and range matching.
 *
 * The module is synchronous and needs no permission flag. It reads no file,
 * environment variable, or network resource. Versions follow SemVer 2.0.0 and
 * are parsed by the Rust `semver` crate; no regular expression is used.
 *
 * Unlike npm's `semver`, every numeric component is a decimal string, so an
 * identifier above `Number.MAX_SAFE_INTEGER` keeps its exact digits. Core
 * components are limited to `u64::MAX`. A malformed version or range throws
 * `SemVerError`; `valid` and `validRange` return `false` instead.
 *
 * Ranges use Cargo's interpretation: one or more comma-separated comparators
 * or `*`. There is no `||` union, no hyphen range, and no whitespace-separated
 * implicit AND. A bare version uses Cargo's default caret operator: `1.2`
 * means `^1.2`, so `1.9.0` matches and `1.1.9` does not; `1.2.3` means
 * `^1.2.3`, not an exact match, so use `=1.2.3` for exactness. It is not an
 * npm x-range. `0.2` matches only `0.2.x`, and `0` matches any `0.x.y`. A
 * prerelease version matches only when a comparator names the same
 * `MAJOR.MINOR.PATCH` with a non-empty prerelease, so `*` matches no
 * prerelease. At most 32 comparators are accepted; comparator build metadata
 * is ignored.
 *
 * Input is bounded: a version string at `MAX_VERSION_BYTES`, a range at
 * `MAX_RANGE_BYTES`, one core component at `MAX_CORE_BYTES`, 32 prerelease and
 * 32 build identifiers, and 256 bytes per identifier. Exceeding a bound throws
 * `SEMVER_LIMIT`; `valid` and `validRange` return `false` instead.
 */
declare module "neo:semver" {
  /** Stable failure codes reported by `SemVerError.code`. */
  export type SemVerErrorCode =
    | "SEMVER_INVALID"
    | "SEMVER_RANGE"
    | "SEMVER_INCREMENT"
    | "SEMVER_OVERFLOW"
    | "SEMVER_LIMIT";

  /** Largest accepted version string, in UTF-8 bytes. */
  export const MAX_VERSION_BYTES: 1024;
  /** Largest accepted range string, in UTF-8 bytes. */
  export const MAX_RANGE_BYTES: 4096;
  /** Largest accepted core component, in UTF-8 bytes. */
  export const MAX_CORE_BYTES: 20;
  /** Maximum prerelease identifiers and maximum build identifiers. */
  export const MAX_IDENTIFIERS: 32;
  /** Largest accepted single prerelease or build identifier, in UTF-8 bytes. */
  export const MAX_IDENTIFIER_BYTES: 256;

  /**
   * A malformed version, range, increment request, an increment overflow, or
   * an input that exceeds a documented limit.
   *
   * `code` is stable; `message` may add detail and is not stable. A non-string
   * argument throws `TypeError` instead.
   */
  export class SemVerError extends Error {
    readonly code: SemVerErrorCode;
  }

  /**
   * A parsed version with exact decimal-string components.
   *
   * The object has a null prototype and own data properties. `prerelease` and
   * `build` are arrays of identifier strings; a numeric prerelease identifier
   * is not converted to a number. `version` is the canonical string and equals
   * the input, which has no surrounding whitespace.
   */
  export interface SemVerVersion {
    readonly version: string;
    readonly major: string;
    readonly minor: string;
    readonly patch: string;
    readonly prerelease: readonly string[];
    readonly build: readonly string[];
  }

  /**
   * Components accepted by `format`.
   *
   * `major`, `minor`, and `patch` are canonical decimal strings or non-negative
   * safe integers. Use strings for values above `Number.MAX_SAFE_INTEGER`.
   * `prerelease` and `build` are arrays of complete identifiers, not dotted
   * strings. Omitted arrays mean no prerelease or build metadata.
   */
  export interface SemVerComponents {
    readonly major: string | number;
    readonly minor: string | number;
    readonly patch: string | number;
    readonly prerelease?: readonly string[];
    readonly build?: readonly string[];
  }

  /** Release names accepted by `increment`. */
  export type SemVerRelease =
    | "major"
    | "minor"
    | "patch"
    | "premajor"
    | "preminor"
    | "prepatch"
    | "prerelease";

  /**
   * True for a syntactically valid SemVer version. A non-string throws
   * `TypeError`; a malformed or over-limit string returns `false`.
   *
   * Preserved from the original module. This is equivalent to attempting
   * `parse` and catching `SEMVER_INVALID` or `SEMVER_LIMIT`.
   */
  export function valid(version: string): boolean;

  /**
   * Compare precedence, ignoring build metadata.
   *
   * Preserved from the original module. Throws `SemVerError` with
   * `SEMVER_INVALID` for a malformed version and `SEMVER_LIMIT` for one
   * longer than `MAX_VERSION_BYTES`.
   */
  export function compare(a: string, b: string): -1 | 0 | 1;

  /**
   * Parse a version into exact components.
   *
   * Throws `SemVerError` with `SEMVER_INVALID` for malformed input and
   * `SEMVER_LIMIT` for an over-limit version.
   */
  export function parse(version: string): SemVerVersion;

  /**
   * Format components into a canonical version string.
   *
   * Each prerelease and build array element is one complete identifier; a dot
   * belongs between elements, not inside one. Arrays are limited to
   * `MAX_IDENTIFIERS` entries and each identifier to `MAX_IDENTIFIER_BYTES`;
   * the rendered version must fit `MAX_VERSION_BYTES`. Throws `SemVerError`
   * with `SEMVER_INVALID` for a leading zero, an empty or illegal identifier,
   * or an invalid combination, `SEMVER_OVERFLOW` when a core component exceeds
   * `u64::MAX`, and `SEMVER_LIMIT` for a size or count bound.
   */
  export function format(components: SemVerComponents): string;

  /**
   * Increment a version and return its canonical string.
   *
   * Build metadata is always removed. `major`, `minor`, and `patch` bump the
   * named component, reset lower components to zero, and drop the prerelease.
   * The `pre*` releases start a prerelease at `0` or `<identifier>.0`.
   * `prerelease` increments the last numeric identifier, appends `.0` when
   * there is none, or restarts at `<identifier>.0` when the first identifier
   * differs. `identifier` is one complete prerelease identifier, not a dotted
   * list. Unlike npm, `1.0.0-alpha` with `major` becomes `2.0.0`, not
   * `1.0.0`. Throws `SEMVER_INCREMENT` for an unknown release or invalid
   * identifier, `SEMVER_LIMIT` for an over-limit version or identifier, and
   * `SEMVER_OVERFLOW` when a component cannot be incremented.
   */
  export function increment(
    version: string,
    release: SemVerRelease,
    identifier?: string,
  ): string;

  /**
   * True when a version satisfies a Cargo-style range.
   *
   * Throws `SemVerError` with `SEMVER_INVALID` or `SEMVER_RANGE` for malformed
   * input and `SEMVER_LIMIT` for an over-limit version or range rather than
   * returning `false`.
   */
  export function satisfies(version: string, range: string): boolean;

  /**
   * True for a syntactically valid range. A malformed or over-limit string
   * returns `false`; a non-string throws `TypeError`.
   */
  export function validRange(range: string): boolean;
}

/**
 * Unicode and ASCII character predicates and code-point operations. Requires
 * no permission flag.
 *
 * A predicate accepts a Unicode code point and returns `false` for anything
 * that is not a Unicode scalar value: a non-number, a non-integer, a value
 * outside 0 through 0x10FFFF, or a surrogate code point. Code-point
 * predicates never throw. Classification follows the host engine's Unicode
 * property tables, so a different QuickJS build may classify a newly assigned
 * code point differently.
 *
 * The `...At` functions take a UTF-16 code unit index, like
 * `String.prototype.codePointAt`. An index in the middle of a surrogate pair
 * reads the trailing surrogate, which is not a scalar value, so they return
 * `false`. An out-of-range index returns `false`; a non-string text or an
 * invalid index throws `TypeError`.
 */
declare module "neo:chars" {
  /** A Unicode code point, 0 through 0x10FFFF. */
  export type Char = number;

  /** Largest Unicode code point, 0x10FFFF. */
  export const MAX_CODE_POINT: number;
  /** Largest ASCII code point, 0x7F. */
  export const ASCII_MAX: number;
  /** Largest Latin-1 code point, 0xFF. */
  export const LATIN1_MAX: number;
  /** First UTF-16 surrogate code point, 0xD800. */
  export const SURROGATE_MIN: number;
  /** Last UTF-16 surrogate code point, 0xDFFF. */
  export const SURROGATE_MAX: number;

  /** True for a Unicode scalar value (not a surrogate code point). */
  export function isChar(value: unknown): value is Char;
  /** True for an ASCII code point, 0 through 0x7F. */
  export function isAscii(value: unknown): boolean;
  /** True for a Latin-1 code point, 0 through 0xFF. */
  export function isLatin1(value: unknown): boolean;
  /** True for a C0 or C1 control character (Unicode category Cc). */
  export function isControl(value: unknown): boolean;
  /** True for a decimal digit (Unicode category Nd), not other numerals. */
  export function isDigit(value: unknown): boolean;
  /** True for a letter (Unicode category L). */
  export function isLetter(value: unknown): boolean;
  /** True for a letter or a decimal digit. */
  export function isLetterOrDigit(value: unknown): boolean;
  /** True for a lowercase letter (Unicode category Ll). */
  export function isLower(value: unknown): boolean;
  /** True for an uppercase letter (Unicode category Lu). */
  export function isUpper(value: unknown): boolean;
  /** True for punctuation (Unicode category P). */
  export function isPunc(value: unknown): boolean;
  /** True for whitespace, including U+0085 and categories Zs, Zl, and Zp. */
  export function isSpace(value: unknown): boolean;
  /** True for a symbol (Unicode category S), including emoji. */
  export function isSymbol(value: unknown): boolean;

  /**
   * True for an ASCII code point at UTF-16 code unit index `index`.
   *
   * An index in the middle of a surrogate pair addresses the trailing
   * surrogate and returns `false`, as does an out-of-range index. A non-string
   * `text` or an invalid `index` (non-number, non-integer, or negative)
   * throws `TypeError`.
   */
  export function isAsciiAt(text: string, index: number): boolean;
  /**
   * True for a Latin-1 code point at UTF-16 code unit index `index`.
   *
   * An index in the middle of a surrogate pair addresses the trailing
   * surrogate and returns `false`, as does an out-of-range index. A non-string
   * `text` or an invalid `index` (non-number, non-integer, or negative)
   * throws `TypeError`.
   */
  export function isLatin1At(text: string, index: number): boolean;
  /**
   * True for a control character at UTF-16 code unit index `index`.
   *
   * An index in the middle of a surrogate pair addresses the trailing
   * surrogate and returns `false`, as does an out-of-range index. A non-string
   * `text` or an invalid `index` (non-number, non-integer, or negative)
   * throws `TypeError`.
   */
  export function isControlAt(text: string, index: number): boolean;
  /**
   * True for a decimal digit at UTF-16 code unit index `index`.
   *
   * An index in the middle of a surrogate pair addresses the trailing
   * surrogate and returns `false`, as does an out-of-range index. A non-string
   * `text` or an invalid `index` (non-number, non-integer, or negative)
   * throws `TypeError`.
   */
  export function isDigitAt(text: string, index: number): boolean;
  /**
   * True for a letter at UTF-16 code unit index `index`.
   *
   * An index in the middle of a surrogate pair addresses the trailing
   * surrogate and returns `false`, as does an out-of-range index. A non-string
   * `text` or an invalid `index` (non-number, non-integer, or negative)
   * throws `TypeError`.
   */
  export function isLetterAt(text: string, index: number): boolean;
  /**
   * True for a letter or decimal digit at UTF-16 code unit index `index`.
   *
   * An index in the middle of a surrogate pair addresses the trailing
   * surrogate and returns `false`, as does an out-of-range index. A non-string
   * `text` or an invalid `index` (non-number, non-integer, or negative)
   * throws `TypeError`.
   */
  export function isLetterOrDigitAt(text: string, index: number): boolean;
  /**
   * True for a lowercase letter at UTF-16 code unit index `index`.
   *
   * An index in the middle of a surrogate pair addresses the trailing
   * surrogate and returns `false`, as does an out-of-range index. A non-string
   * `text` or an invalid `index` (non-number, non-integer, or negative)
   * throws `TypeError`.
   */
  export function isLowerAt(text: string, index: number): boolean;
  /**
   * True for an uppercase letter at UTF-16 code unit index `index`.
   *
   * An index in the middle of a surrogate pair addresses the trailing
   * surrogate and returns `false`, as does an out-of-range index. A non-string
   * `text` or an invalid `index` (non-number, non-integer, or negative)
   * throws `TypeError`.
   */
  export function isUpperAt(text: string, index: number): boolean;
  /**
   * True for punctuation at UTF-16 code unit index `index`.
   *
   * An index in the middle of a surrogate pair addresses the trailing
   * surrogate and returns `false`, as does an out-of-range index. A non-string
   * `text` or an invalid `index` (non-number, non-integer, or negative)
   * throws `TypeError`.
   */
  export function isPuncAt(text: string, index: number): boolean;
  /**
   * True for whitespace at UTF-16 code unit index `index`.
   *
   * An index in the middle of a surrogate pair addresses the trailing
   * surrogate and returns `false`, as does an out-of-range index. A non-string
   * `text` or an invalid `index` (non-number, non-integer, or negative)
   * throws `TypeError`.
   */
  export function isSpaceAt(text: string, index: number): boolean;
  /**
   * True for a symbol at UTF-16 code unit index `index`.
   *
   * An index in the middle of a surrogate pair addresses the trailing
   * surrogate and returns `false`, as does an out-of-range index. A non-string
   * `text` or an invalid `index` (non-number, non-integer, or negative)
   * throws `TypeError`.
   */
  export function isSymbolAt(text: string, index: number): boolean;

  /**
   * Return the lowercase code point, or `value` when the host mapping is not
   * a single code point.
   *
   * @throws TypeError when `value` is not a number.
   * @throws RangeError when `value` is not a Unicode scalar value.
   */
  export function toLower(value: number): Char;

  /**
   * Return the uppercase code point, or `value` when the host mapping is not
   * a single code point.
   *
   * @throws TypeError when `value` is not a number.
   * @throws RangeError when `value` is not a Unicode scalar value.
   */
  export function toUpper(value: number): Char;

  /** Flip the case of one code point, or return it unchanged. */
  export function simpleFold(value: number): Char;

  /**
   * Compare two code points case-insensitively under the host engine's Unicode
   * simple case-fold canonicalization.
   *
   * The relation is reflexive, symmetric, and transitive. The Greek theta
   * class U+0398, U+03B8, U+03D1, and U+03F4 is one class. This is not Go's
   * `unicode.SimpleFold` orbit and not full case folding: a mapping that
   * expands to more than one code point is not applied, so `ß` (0xDF) and `s`
   * (0x73) compare as different. The engine's Unicode version decides which
   * pairs fold.
   */
  export function equalFold(a: number, b: number): boolean;
}

/**
 * A synchronous string builder. Requires no permission flag.
 *
 * Text is stored as UTF-16 strings and joined lazily. `length` counts UTF-16
 * code units, like `String.prototype.length`; `codePointLength` counts Unicode
 * code points. This is not `@neotales/slices`' `CharArrayBuilder`: it has no
 * code-point buffer, no `appendSlice` or `appendCharArray`, accepts no `Date`,
 * and takes no capacity argument.
 */
declare module "neo:string-builder" {
  export class StringBuilder {
    /** Create an empty builder. */
    constructor();

    /** Number of UTF-16 code units in the current text. */
    readonly length: number;

    /** Number of Unicode code points in the current text. */
    readonly codePointLength: number;

    /** The current text, the same value as `toString()`. */
    readonly string: string;

    /**
     * Append text and return this builder.
     *
     * A primitive string is used as supplied. A number, boolean, or bigint is
     * converted with `String(...)`. Anything else throws `TypeError`; values
     * are not coerced. An empty string appends nothing.
     */
    append(value: string | number | boolean | bigint): this;

    /**
     * Append one Unicode scalar value and return this builder.
     *
     * @throws TypeError when `codePoint` is not a number.
     * @throws RangeError when it is not an integer from 0 through 0x10FFFF, or
     * when it is a surrogate code point.
     */
    appendCodePoint(codePoint: number): this;

    /** Remove all text and return this builder. It remains usable. */
    clear(): this;

    /** Return the current text. The result is cached until the next change. */
    toString(): string;
  }
}

/**
 * ANSI SGR styling helpers. Requires no permission flag.
 *
 * Color is on by default and never detected: these functions read no
 * environment variables, process arguments, or terminal state. Pass
 * `{ color: false }` to return the input unchanged. There is no module-global
 * toggle or automatic downgrade of color depth.
 *
 * Styling wraps a value as `open + value + close`. If the exact closing
 * sequence for that style appears inside the value it is replaced with the
 * opening sequence. Only that exact sequence is handled: a full reset
 * (`ESC[0m`), another style's reset, or any other SGR sequence in the value is
 * passed through and can end or change the style early. Styling does not
 * sanitize or normalize escape sequences.
 */
declare module "neo:ansi" {
  export interface StyleOptions {
    /** `false` returns the input unchanged. Defaults to `true`. */
    color?: boolean;
  }

  /** A style function. */
  export type Style = (value: string, options?: StyleOptions) => string;

  /** A 24-bit color as three channels. */
  export interface Rgb {
    r: number;
    g: number;
    b: number;
  }

  export function reset(value: string, options?: StyleOptions): string;
  export function bold(value: string, options?: StyleOptions): string;
  export function dim(value: string, options?: StyleOptions): string;
  export function italic(value: string, options?: StyleOptions): string;
  export function underline(value: string, options?: StyleOptions): string;
  export function inverse(value: string, options?: StyleOptions): string;
  export function hidden(value: string, options?: StyleOptions): string;
  export function strikethrough(value: string, options?: StyleOptions): string;

  export function black(value: string, options?: StyleOptions): string;
  export function red(value: string, options?: StyleOptions): string;
  export function green(value: string, options?: StyleOptions): string;
  export function yellow(value: string, options?: StyleOptions): string;
  export function blue(value: string, options?: StyleOptions): string;
  export function magenta(value: string, options?: StyleOptions): string;
  export function cyan(value: string, options?: StyleOptions): string;
  export function white(value: string, options?: StyleOptions): string;
  export function brightBlack(value: string, options?: StyleOptions): string;
  export function brightRed(value: string, options?: StyleOptions): string;
  export function brightGreen(value: string, options?: StyleOptions): string;
  export function brightYellow(value: string, options?: StyleOptions): string;
  export function brightBlue(value: string, options?: StyleOptions): string;
  export function brightMagenta(value: string, options?: StyleOptions): string;
  export function brightCyan(value: string, options?: StyleOptions): string;
  export function brightWhite(value: string, options?: StyleOptions): string;
  /** Alias for `brightBlack`. */
  export function gray(value: string, options?: StyleOptions): string;

  export function bgBlack(value: string, options?: StyleOptions): string;
  export function bgRed(value: string, options?: StyleOptions): string;
  export function bgGreen(value: string, options?: StyleOptions): string;
  export function bgYellow(value: string, options?: StyleOptions): string;
  export function bgBlue(value: string, options?: StyleOptions): string;
  export function bgMagenta(value: string, options?: StyleOptions): string;
  export function bgCyan(value: string, options?: StyleOptions): string;
  export function bgWhite(value: string, options?: StyleOptions): string;
  export function bgBrightBlack(value: string, options?: StyleOptions): string;
  export function bgBrightRed(value: string, options?: StyleOptions): string;
  export function bgBrightGreen(value: string, options?: StyleOptions): string;
  export function bgBrightYellow(value: string, options?: StyleOptions): string;
  export function bgBrightBlue(value: string, options?: StyleOptions): string;
  export function bgBrightMagenta(value: string, options?: StyleOptions): string;
  export function bgBrightCyan(value: string, options?: StyleOptions): string;
  export function bgBrightWhite(value: string, options?: StyleOptions): string;

  /**
   * Apply 256-color foreground `index` (SGR 38;5;n).
   *
   * `index` is truncated and clamped to 0 through 255. A non-number throws
   * `TypeError`; a non-finite number throws `RangeError`.
   */
  export function rgb8(value: string, index: number, options?: StyleOptions): string;

  /** Apply 256-color background `index` (SGR 48;5;n), clamped like `rgb8`. */
  export function bgRgb8(value: string, index: number, options?: StyleOptions): string;

  /**
   * Apply 24-bit foreground color (SGR 38;2;r;g;b).
   *
   * `color` is an integer from 0x000000 through 0xFFFFFF, or an object whose
   * channels are truncated and clamped to 0 through 255.
   */
  export function rgb24(value: string, color: number | Rgb, options?: StyleOptions): string;

  /** Apply 24-bit background color (SGR 48;2;r;g;b). */
  export function bgRgb24(value: string, color: number | Rgb, options?: StyleOptions): string;

  /**
   * Apply style functions from left to right.
   *
   * @throws TypeError when `styles` is not an array, an element is not a
   * function, `value` is not a string, or the options are invalid.
   */
  export function apply(
    value: string,
    styles: readonly Style[],
    options?: StyleOptions,
  ): string;

  /**
   * Remove CSI sequences introduced by `ESC [` or U+009B, OSC strings,
   * DCS/SOS/PM/APC strings, ESC c (RIS), and two-byte escapes whose second
   * byte is 0x40 through 0x5F. A control string ends at BEL, ST (`ESC \`), or
   * U+009C, and text after the terminator is retained. Other text is preserved
   * exactly, including lone escape bytes and all other control characters.
   *
   * This is best-effort cleanup, not a security sanitizer. Escape protocols
   * are more complex than this scanner, and malformed or unhandled sequences
   * may remain; do not rely on it alone to make untrusted text safe to print.
   */
  export function stripAnsi(value: string): string;
}

/**
 * Lexical path operations for POSIX and Windows syntax. Importing this module
 * does not require a permission flag.
 *
 * Choose `posix` or `win32` explicitly, or use named functions and constants
 * for the host dialect (Windows on Windows, POSIX elsewhere). These functions
 * do not perform filesystem I/O or check containment, existence, symlinks or
 * junctions. They never read the process environment or per-drive directories.
 *
 * Every path must be a primitive string; a boxed `String`, number, `null`,
 * `undefined`, or other value throws `TypeError`, as does any NUL (U+0000) in
 * a string argument. Only `fromFileUrl` accepts a URL object, and it must
 * have this realm's private URL brand. `basename`'s `suffix` must be a string.
 * `format` requires a non-null, non-array object whose `root`, `dir`, `base`,
 * `ext`, and `name` fields are strings or absent.
 *
 * `resolve`, `relative`, and Windows `toNamespacedPath` can disclose the
 * runtime-local cwd if the input does not provide a complete absolute base.
 * This fallback is available only in the host dialect and requires admitted
 * `neo:fs` with filesystem permission, or admitted `neo:process` with
 * process or stdio permission. It observes runtime `chdir` and never changes
 * or falls back to the host process's cwd. Glob and file URL helpers are
 * lexical and never use cwd.
 */
declare module "neo:path" {
  /**
   * An instance of neo-js's realm-local `globalThis.URL`, installed before
   * user code when `neo:path` is admitted. This is a file-only subset, not a
   * DOM URL: there is no `searchParams`, `URLSearchParams`, static URL
   * parsing helper, object URL API, or support for non-file schemes and
   * bases. The constructor accepts a primitive string or a same-realm URL
   * and an optional file URL base.
   * This declaration deliberately does not augment the global DOM/Worker
   * `URL` type. TypeScript checks this interface structurally; at runtime the
   * conversion checks the object's private realm-local brand.
   */
  export interface FileURL {
    href: string;
    /** Always the string `"null"` in neo-js. */
    readonly origin: string;
    protocol: string;
    username: string;
    password: string;
    host: string;
    hostname: string;
    port: string;
    pathname: string;
    search: string;
    hash: string;
    toString(): string;
    toJSON(): string;
  }

  /**
   * Type of the realm's global constructor for projects without DOM or
   * WebWorker libs. For example, `(globalThis as { URL?: FileURLConstructor }).URL`
   * is defined when `neo:path` is admitted. Prefer {@link toFileUrl} when
   * constructing a URL from a path.
   */
  export interface FileURLConstructor {
    new(input: string | FileURL, base?: string | FileURL): FileURL;
    readonly prototype: FileURL;
  }

  /** A parsed path; every field is a string. */
  export interface ParsedPath {
    /** `/`, or a Windows root such as `C:\`, `C:`, `\\server\share\`, or `""`. */
    root: string;
    /** The parent text before the final component, or the root. */
    dir: string;
    /** The final component, or `""` when there is none. */
    base: string;
    /** The extension including `.`, or `""`. */
    ext: string;
    /** The final component without its extension. */
    name: string;
  }

  /** Options for compiling a glob. Absent, inherited or undefined fields use defaults. */
  export interface GlobOptions {
    /** Enable `?(...)`, `@(...)`, `*(...)`, `+(...)`, and `!(...)`. Default: true. */
    extended?: boolean;
    /**
     * Enable whole-component `**`. Default: true for compilation.
     * In `normalizeGlob` and `joinGlobs`, preserve `..` directly following
     * `**` instead; default: false.
     */
    globstar?: boolean;
    /** Use JavaScript's `i` flag. Default: false, including on Windows. */
    caseInsensitive?: boolean;
  }

  /** The shape shared by the `posix` and `win32` namespaces. */
  export interface PathApi<Separator extends string, Delimiter extends string> {
    /** Path separator: `/` for POSIX, `\` for Windows. */
    readonly sep: Separator;
    /** PATH-list separator: `:` for POSIX, `;` for Windows. */
    readonly delimiter: Delimiter;

    /**
     * Concatenate the non-empty segments with the namespace separator and
     * normalize. No arguments, or only empty segments, returns `"."`. Every
     * argument is validated before empty ones are dropped. A later absolute
     * segment does not restart the result.
     */
    join(...paths: string[]): string;

    /**
     * Lexically resolve `.` and `..`, collapse separator runs, and preserve at
     * most one trailing separator. `""` becomes `"."`; an empty absolute
     * result is the root.
     */
    normalize(path: string): string;

    /**
     * Return the final component, ignoring trailing separators. A root-only
     * path, including a complete UNC root, has an empty basename. `suffix` is
     * removed only when non-empty, shorter than the name, and an exact match.
     */
    basename(path: string, suffix?: string): string;

    /**
     * Return the parent text with trailing separators removed. A root-only
     * path returns the root as written; a relative path with no parent
     * returns `"."`; `""` returns `"."`.
     */
    dirname(path: string): string;

    /**
     * Return the final component's extension including `.`, or `""` for a dot
     * file, `.`, `..`, a complete UNC root, or no dot. Trailing separators are
     * ignored, so `extname("a/b.txt/")` is `".txt"`.
     */
    extname(path: string): string;

    /**
     * True for a syntactically absolute path. A Windows rooted path such as
     * `\child` is absolute here but still needs a drive or share in `resolve`.
     */
    isAbsolute(path: string): boolean;

    /** Split into `root`, `dir`, `base`, `ext`, and `name` without normalizing. */
    parse(path: string): ParsedPath;

    /**
     * Join `dir` (or `root`) and `base` (or `name + ext`) with the namespace
     * separator. Empty `dir`, `root`, and `base` are treated as absent. A
     * non-empty `base` is appended as supplied, including when it equals the
     * separator, so it is never silently dropped. Does not normalize or
     * validate consistency.
     */
    format(pathObject: Partial<ParsedPath>): string;

    /**
     * Return the common prefix at component boundaries, or `""` if there is
     * none (including an empty array). Compare spelling exactly, retaining
     * the separator when paths diverge after a shared component. Does not
     * normalize or read cwd; Windows splits on `\` only, as in the reference.
     */
    common(paths: readonly string[]): string;

    /**
     * Resolve right to left into an absolute path without a trailing
     * separator, except at the root. Empty segments are ignored after
     * validation. Reads the runtime cwd only if a complete base is missing;
     * the non-host dialect requires an explicit base. On Windows, a
     * drive-relative segment requires a matching drive base (or matching
     * native cwd), and a rooted segment without a drive inherits a drive or
     * share. UNC shares are indivisible roots. No per-drive cwd or drive-root
     * fallback is used. Can throw `Error` with `PATH_PERMISSION`,
     * `PATH_CWD_DIALECT`, `PATH_CWD`, or `PATH_WINDOWS_ROOT`.
     *
     * @example Resolve without access to the runtime cwd.
     * ```ts
     * import { posix } from "neo:path";
     * const absolute = posix.resolve("/project", "src", "../README.md");
     * // "/project/README.md"
     * ```
     */
    resolve(...paths: string[]): string;

    /**
     * Resolve both operands as `resolve` would, then return the path from
     * `from` to `to`. Returns `""` for equal paths. Windows compares roots
     * and components case-insensitively, preserves destination spelling, and
     * returns the absolute destination if the drive/share/namespace differs.
     * Has the same cwd requirements and errors as `resolve`.
     */
    relative(from: string, to: string): string;

    /**
     * POSIX returns the validated input unchanged. Windows converts a
     * resolved drive path to `\\?\` form or a UNC path to `\\?\UNC\` form.
     * Empty and already-namespaced/device paths are returned unchanged.
     * Conversion is lexical and does not validate paths for Windows I/O.
     * When resolution is needed, its cwd requirements and errors apply.
     */
    toNamespacedPath(path: string): string;

    /**
     * Convert an absolute path to a branded file URL without using cwd or
     * doing I/O. POSIX collapses leading slashes; Windows recognizes UNC
     * hosts, translates separators, and percent-encodes line breaks in
     * the pathname. URL dot segments are normalized, lone surrogates
     * become U+FFFD, and carets in the pathname serialize as `%5E`.
     * Relative paths and NUL throw `TypeError`. Input and serialized URL
     * length limits are 65,536 UTF-16 code units each, or 8,192 code units
     * for host text. Overflow throws `RangeError`; these are not memory caps.
     */
    toFileUrl(path: string): FileURL;

    /**
     * Decode only the pathname of a file URL string or branded URL from
     * this realm. POSIX ignores the host; Windows keeps remote hosts as
     * UNC roots, including when the first segment looks like a drive.
     * Query and fragment do not enter the path. Escaped `/` and `\` are
     * decoded, so the result is not a containment check. Bare or malformed
     * percent escapes remain literal; invalid escaped UTF-8 throws
     * `URIError`. Invalid URLs, a raw NUL in a string argument, and decoded
     * pathname NUL throw `TypeError`.
     * Uses the same limits as {@link PathApi.toFileUrl}.
     */
    fromFileUrl(url: string | FileURL): string;

    /**
     * Heuristically detect glob-like syntax, using backslash as the escape
     * in both dialects. A true result does not imply that compilation will
     * succeed. Rejects non-strings and NUL with `TypeError`; over 4096 UTF-16
     * code units throws `RangeError` with code `PATH_GLOB_LIMIT`.
     */
    isGlob(glob: string): boolean;

    /**
     * Produce a fresh, whole-string `RegExp`; an empty glob never matches.
     * Supports `*`, `?` (one UTF-16 code unit except line terminators),
     * comma-separated braces (not numeric/letter sequences), ranges,
     * `[!...]`, named classes such as `[[:digit:]]`, and extended groups.
     * `*`, `?`, classes and groups cannot cross separators. POSIX escapes
     * with backslash; Windows accepts both separators and escapes with a
     * backtick. Neither escape can quote a separator. Repeated and trailing
     * separators are tolerated. `!(x)` tests for a rejected prefix, so
     * `!(foo).js` also excludes `foobar.js`.
     * `/` matches separator-only paths, not the empty string. An intermediate
     * `**` consumes zero or more separator-terminated components; a nullable
     * suffix such as `{,x}` does not let it consume a final bare component.
     * Unlike upstream, separators in braces or extglobs are syntax errors.
     *
     * Invalid types and NUL throw `TypeError`; invalid syntax throws
     * `SyntaxError` (`PATH_GLOB_SYNTAX`). Limits on pattern length (4096 code
     * units), RegExp constructor source (16384 code units), group depth (16),
     * alternative combinations (64), and variable repetitions (6) throw
     * `RangeError` (`PATH_GLOB_LIMIT`). `*(...)` and `+(...)` require nonempty
     * fixed-width alternatives with distinguishable literal prefixes when
     * there is more than one alternative. Do not use the returned `RegExp`
     * when either the pattern or subject is untrusted: compilation limits
     * do not bound regex execution. Only `match` (default options) has an
     * evaluation work budget. Neither is a security boundary.
     *
     * @example
     * ```ts
     * import { posix } from "neo:path";
     * posix.globToRegExp("src/" + "**" + "/*.ts", { caseInsensitive: true })
     *   .test("src/tools/PATH.TS"); // true
     * ```
     */
    globToRegExp(glob: string, options?: GlobOptions): RegExp;

    /**
     * Normalize as a lexical path, without parsing its glob syntax. With
     * `{ globstar: true }`, treat `..` immediately after `**` as literal.
     * Rejects non-strings and NUL with `TypeError`; over 4096 UTF-16 code
     * units throws `RangeError` (`PATH_GLOB_LIMIT`).
     */
    normalizeGlob(glob: string, options?: Pick<GlobOptions, "globstar">): string;

    /**
     * Join nonempty parts without modifying the input. An empty result
     * from filtering the input is `"."`. With `{ globstar: true }`, join
     * literally and use `normalizeGlob`; otherwise use normal `join` rules.
     * Reads array length once and does not coerce it. Rejects invalid types,
     * invalid array lengths, holes and NUL with `TypeError`. More than 256
     * parts or 4096 joined code units throws `RangeError` (`PATH_GLOB_LIMIT`).
     */
    joinGlobs(globs: readonly string[], options?: Pick<GlobOptions, "globstar">): string;

    /**
     * Match against the default glob grammar with no cwd, I/O, or implicit
     * normalization. Has the same input and pattern errors as `globToRegExp`;
     * subjects over 16384 UTF-16 code units or evaluations beyond a fixed
     * work budget throw `RangeError` (`PATH_GLOB_LIMIT`).
     */
    match(path: string, pattern: string): boolean;

    /** Exact alias for {@link PathApi.match}. */
    matchesGlob(path: string, pattern: string): boolean;
  }

  /** POSIX syntax: `/` separator, `:` delimiter. */
  export const posix: Readonly<PathApi<"/", ":">>;

  /** Windows syntax: `\` separator, `;` delimiter; both separators accepted. */
  export const win32: Readonly<PathApi<"\\", ";">>;

  /** Host path separator: `\` on Windows, `/` elsewhere. */
  export const SEPARATOR: "/" | "\\";
  /** Host PATH-list separator: `;` on Windows, `:` elsewhere. */
  export const DELIMITER: ":" | ";";
  /** Matches runs of host path separators (Windows accepts both spellings). */
  export const SEPARATOR_PATTERN: RegExp;

  /** The host dialect's {@link PathApi.join}. */
  export const join: PathApi<string, string>["join"];
  /** The host dialect's {@link PathApi.normalize}. */
  export const normalize: PathApi<string, string>["normalize"];
  /** The host dialect's {@link PathApi.basename}. */
  export const basename: PathApi<string, string>["basename"];
  /** The host dialect's {@link PathApi.dirname}. */
  export const dirname: PathApi<string, string>["dirname"];
  /** The host dialect's {@link PathApi.extname}. */
  export const extname: PathApi<string, string>["extname"];
  /** The host dialect's {@link PathApi.isAbsolute}. */
  export const isAbsolute: PathApi<string, string>["isAbsolute"];
  /** The host dialect's {@link PathApi.parse}. */
  export const parse: PathApi<string, string>["parse"];
  /** The host dialect's {@link PathApi.format}. */
  export const format: PathApi<string, string>["format"];
  /** The host dialect's {@link PathApi.common}. */
  export const common: PathApi<string, string>["common"];
  /** The host dialect's {@link PathApi.resolve}. */
  export const resolve: PathApi<string, string>["resolve"];
  /** The host dialect's {@link PathApi.relative}. */
  export const relative: PathApi<string, string>["relative"];
  /** The host dialect's {@link PathApi.toNamespacedPath}. */
  export const toNamespacedPath: PathApi<string, string>["toNamespacedPath"];
  /** The host dialect's {@link PathApi.toFileUrl}. */
  export const toFileUrl: PathApi<string, string>["toFileUrl"];
  /** The host dialect's {@link PathApi.fromFileUrl}. */
  export const fromFileUrl: PathApi<string, string>["fromFileUrl"];
  /** The host dialect's {@link PathApi.isGlob}. */
  export const isGlob: PathApi<string, string>["isGlob"];
  /** The host dialect's {@link PathApi.globToRegExp}. */
  export const globToRegExp: PathApi<string, string>["globToRegExp"];
  /** The host dialect's {@link PathApi.normalizeGlob}. */
  export const normalizeGlob: PathApi<string, string>["normalizeGlob"];
  /** The host dialect's {@link PathApi.joinGlobs}. */
  export const joinGlobs: PathApi<string, string>["joinGlobs"];
  /** The host dialect's {@link PathApi.match}. */
  export const match: PathApi<string, string>["match"];
  /** Exact alias for the host dialect's {@link match}. */
  export const matchesGlob: typeof match;
}

/**
 * Synchronous parsing of `.env` content strings. Requires no permission flag
 * and grants none.
 *
 * The module does not read files, consult or mutate the process environment,
 * expand variables, or run shell commands. With --allow-fs, read a file using
 * `neo:fs.readText` and pass the text here. With --allow-env, resolve values
 * using `neo:env.get` if expansion is wanted; this module never does it.
 *
 * The native callback is private to the module and is not installed on the
 * global object. The returned record has a null prototype. Keys such as
 * `__proto__`, `constructor`, and `prototype` are ordinary own enumerable data
 * properties, so no assignment changes the prototype of the result or of
 * `Object.prototype`.
 * Duplicate keys are last-value-wins. Property enumeration is JavaScript's own
 * order, not source order: array-index keys (canonical numeric strings) come
 * first in ascending numeric order, then other string keys in first-insertion
 * order. For example, source order `10, 2, b, a` enumerates as `2, 10, b, a`.
 *
 * Syntax: a leading BOM is ignored; CRLF and lone CR are normalized to LF;
 * UTF-8 is required (lone UTF-16 surrogates are rejected); NUL is rejected,
 * including a `\u0000` or `\U00000000` escape. Only ASCII space and tab count
 * as whitespace. A line whose first non-space character is `#` is a comment.
 * `export` is a prefix only when followed by horizontal space. Keys are one or
 * more ASCII letters, digits, or `_`, up to 1,024 UTF-8 bytes; duplicate keys
 * are allowed. Values are unquoted (trailing horizontal space trimmed), single
 * quoted (only `\'` is an escape), double quoted (`\n`, `\r`, `\t`, `\b`,
 * `\\`, `\"`, `\'`, a backslash before a backtick, `\uXXXX`, `\UXXXXXXXX`;
 * unknown escapes stay literal), or backtick-delimited raw text (no escapes).
 * Quotes may span lines. After a closing quote only horizontal space and an
 * optional `#` comment may follow. In an unquoted value `#` starts a comment
 * only at the start of the value or after a space or tab, so
 * `PASSWORD=abc#123` keeps `abc#123`; `VALUE=abc # note` is `abc`.
 *
 * Command substitution is not interpreted, so a double-quoted value ends at
 * the first unescaped quote even inside `$(...)`; escape that quote or use a
 * single-quoted or backtick-delimited value instead.
 *
 * Deliberate reference differences: this parser has no `stringify`, document
 * token stream, `load`, or implicit expansion; it rejects unterminated quotes,
 * bare lines without `=`, and malformed `\u`/`\U` escapes; and it protects
 * object keys instead of dropping `__proto__`.
 *
 * Input is limited to 1 MiB of UTF-8 bytes before line-ending normalization,
 * 16,384 assignments, and 1,024-byte keys. The native callback checks the
 * primitive string's UTF-16 length before conversion and the exact UTF-8 byte
 * count before parsing. An over-limit source reports `DOTENV_LIMIT` before
 * parsing. Otherwise the first error in source order is thrown. Error messages
 * never include source text.
 */
declare module "neo:dotenv" {
  /** Stable failure code; match on this, never on the message. */
  export type DotenvErrorCode =
    | "DOTENV_SYNTAX"
    | "DOTENV_KEY"
    | "DOTENV_QUOTE"
    | "DOTENV_ESCAPE"
    | "DOTENV_LIMIT"
    | "DOTENV_ENCODING"
    | "DOTENV_NUL";

  /** Thrown by `parse` for malformed source. */
  export class DotenvParseError extends Error {
    name: "DotenvParseError";
    /** Stable code; match on this, never on the message. */
    code: DotenvErrorCode;
    /** One-based line in the normalized source. */
    line: number;
    /** One-based column in Unicode scalar values, not bytes or UTF-16 units. */
    column: number;
  }

  /** Largest accepted source, in UTF-8 bytes (1 MiB). */
  export const MAX_SOURCE_BYTES: 1048576;
  /** Largest number of assignments, including duplicates. */
  export const MAX_ENTRIES: 16384;
  /** Largest accepted key, in UTF-8 bytes. */
  export const MAX_KEY_BYTES: 1024;

  /**
   * Parse `.env` content.
   *
   * @throws TypeError when `source` is not a primitive string.
   * @throws DotenvParseError for malformed or over-limit source. The first
   * error in source order wins.
   */
  export function parse(source: string): Record<string, string>;

  export default parse;
}

/**
 * Parse TOML text. Requires no permission flag and performs no file, network,
 * environment, or process access.
 *
 * Failures are thrown as `TomlParseError`. Preflight depth or work limits
 * take precedence over semantic errors, including earlier duplicate keys.
 * TOML date and time
 * values, non-finite floats, and integers outside ±(2^53 - 1) are rejected
 * rather than converted. Duplicate keys are rejected. Objects have a null
 * prototype, so keys such as `__proto__` are ordinary own properties.
 */
declare module "neo:toml" {
  /**
   * A parsed TOML value. Objects are ordinary object shapes at runtime with a
   * null prototype. TOML has no null value.
   */
  export type TomlValue =
    | boolean
    | number
    | string
    | TomlValue[]
    | { [key: string]: TomlValue };

  /** Stable failure code; match on this, never on the message. */
  export type TomlErrorCode =
    | "TOML_SYNTAX"
    | "TOML_DATETIME"
    | "TOML_INTEGER"
    | "TOML_FLOAT"
    | "TOML_DEPTH"
    | "TOML_LIMIT"
    | "TOML_ENCODING"
    | "TOML_NUL";

  /** Thrown by `parse` for malformed or unsupported TOML. */
  export class TomlParseError extends Error {
    name: "TomlParseError";
    /** Stable code; match on this, never on the message. */
    code: TomlErrorCode;
    /**
     * One-based source line; 0 when a conversion rejection has no span, and
     * 1 for the source-size limit.
     */
    line: number;
    /** One-based Unicode-scalar column, or 0 when unknown. */
    column: number;
  }

  /** Largest accepted source, in UTF-8 bytes (1 MiB). */
  export const MAX_SOURCE_BYTES: 1048576;
  /** Maximum collection nesting depth, counting the root collection as one. */
  export const MAX_DEPTH: 64;
  /** Maximum number of scalar and collection nodes. */
  export const MAX_NODES: 100000;
  /** Maximum entries in one array or table. */
  export const MAX_ITEMS: 100000;
  /** Upper bound on the JSON response built for JavaScript, in bytes. */
  export const MAX_OUTPUT_BYTES: 16777216;

  /**
   * Parse TOML source.
   *
   * @throws TypeError when `source` is not a primitive string.
   * @throws TomlParseError for malformed or unsupported source.
   */
  export function parse(source: string): TomlValue;

  export default parse;
}

/**
 * Parse YAML text. Requires no permission flag and performs no file, network,
 * environment, or process access.
 *
 * Exactly one document is accepted. Aliases, explicit tags, non-string mapping
 * keys, duplicate keys, non-finite floats, and integers outside
 * ±(2^53 - 1) are rejected. Plain scalars follow the YAML 1.2 core schema, so
 * a date-like scalar is a string. Objects have a null prototype, so keys such
 * as `__proto__` are ordinary own properties.
 */
declare module "neo:yaml" {
  /**
   * A parsed YAML value. Objects are ordinary object shapes at runtime with a
   * null prototype.
   */
  export type YamlValue =
    | null
    | boolean
    | number
    | string
    | YamlValue[]
    | { [key: string]: YamlValue };

  /** Stable failure code; match on this, never on the message. */
  export type YamlErrorCode =
    | "YAML_SYNTAX"
    | "YAML_ALIAS"
    | "YAML_TAG"
    | "YAML_KEY"
    | "YAML_DUPLICATE"
    | "YAML_MULTIDOC"
    | "YAML_INTEGER"
    | "YAML_FLOAT"
    | "YAML_DEPTH"
    | "YAML_LIMIT"
    | "YAML_ENCODING"
    | "YAML_NUL";

  /** Thrown by `parse` for malformed or unsupported YAML. */
  export class YamlParseError extends Error {
    name: "YamlParseError";
    /** Stable code; match on this, never on the message. */
    code: YamlErrorCode;
    /** One-based source line; 1 for the source-size limit. */
    line: number;
    /** One-based Unicode-scalar column. */
    column: number;
  }

  /** Largest accepted source, in UTF-8 bytes (1 MiB). */
  export const MAX_SOURCE_BYTES: 1048576;
  /** Maximum collection nesting depth, counting the root collection as one. */
  export const MAX_DEPTH: 64;
  /** Maximum number of scalar and collection nodes. */
  export const MAX_NODES: 100000;
  /** Maximum entries in one sequence or mapping. */
  export const MAX_ITEMS: 100000;
  /** Upper bound on the JSON response built for JavaScript, in bytes. */
  export const MAX_OUTPUT_BYTES: 16777216;

  /**
   * Parse one YAML document.
   *
   * @throws TypeError when `source` is not a primitive string.
   * @throws YamlParseError for malformed or unsupported source.
   */
  export function parse(source: string): YamlValue;

  export default parse;
}

/** Synchronous assertions. There are no asynchronous forms. */
declare module "neo:assert" {
  type UnsupportedCallbackResult =
    | { then: (...args: never[]) => unknown }
    | Generator<unknown, unknown, never>
    | AsyncGenerator<unknown, unknown, never>;

  type SyncCallback<F extends () => unknown> =
    F & (Extract<ReturnType<NoInfer<F>>, UnsupportedCallbackResult> extends never ? unknown : never);

  /** Thrown when an assertion fails. */
  export class AssertionError extends Error {
    name: "AssertionError";
    /** Stable code; match on this, never on the message. */
    code: "ERR_ASSERTION";
    /** The failed assertion, such as "equal", "deepEqual", or "throws". */
    operator: string;
    /** The value supplied to the assertion, when it has one. */
    actual?: unknown;
    /** The value the assertion expected, when it has one. */
    expected?: unknown;
  }

  export type ErrorConstructor = new (...args: any[]) => Error;

  /**
   * Throws unless both values are the same under `Object.is`.
   * `NaN` equals `NaN`; `-0` and `0` differ; objects compare by identity.
   * Throws `TypeError` when `message` is not a string.
   */
  export function equal(actual: unknown, expected: unknown, message?: string): void;

  /** Throws when both values are the same under `Object.is`. */
  export function notEqual(actual: unknown, expected: unknown, message?: string): void;

  /**
   * Throws unless the values are structurally equal.
   *
   * Prototypes must match. Own enumerable string and symbol properties are
   * compared, with getters invoked. Arrays also compare length. Maps and Sets
   * ignore iteration order. Dates, regular expressions (including `lastIndex`),
   * boxed primitives, errors, buffers, and views compare their internal state
   * as well as own properties. Functions, Promises, WeakMaps, WeakSets, and
   * WeakRefs compare by identity. Cycles are handled. The failure message names
   * the first differing path.
   */
  export function deepEqual(actual: unknown, expected: unknown, message?: string): void;

  /** Throws when the values are structurally equal. */
  export function notDeepEqual(actual: unknown, expected: unknown, message?: string): void;

  /** Throws when `value` is falsy. Narrows the value in TypeScript. */
  export function ok(value: unknown, message?: string): asserts value;

  /** Throws `AssertionError` unconditionally. */
  export function fail(message?: string): never;

  /**
   * Throws unless calling `fn` throws. Returns the thrown value.
   *
   * `expected` is an error constructor (checked with `instanceof`) or a RegExp
   * matched against the message, or is omitted to accept any throw. The message
   * is the third argument; a string second argument is a `TypeError`, not a
   * message. Rejects direct async, generator, and callable proxy callbacks
   * with a `TypeError`, as well as native Promise and generator returns,
   * callable `then` properties, and `then` lookups that reach an accessor
   * or proxy. The return check does not invoke accessors or proxy traps.
   */
  export function throws<F extends () => unknown>(
    fn: SyncCallback<F>,
    expected: RegExp,
    message?: string,
  ): unknown;
  export function throws<T extends ErrorConstructor, F extends () => unknown = () => unknown>(
    fn: SyncCallback<F>,
    expected: T,
    message?: string,
  ): InstanceType<T>;
  export function throws<F extends () => unknown>(
    fn: SyncCallback<F>,
    expected?: null,
    message?: string,
  ): unknown;

  /**
   * Throws when calling `fn` throws, and otherwise returns its return value.
   * Rejects the same callback kinds and return values as `throws`.
   */
  export function doesNotThrow<T = unknown, F extends () => T = () => T>(
    fn: SyncCallback<F>,
    message?: string,
  ): ReturnType<F>;
}

/**
 * A synchronous test registry. Requires no permissions and grants none.
 *
 * Tests run in registration order in the caller's single global context. The
 * CLI runs them after the entry module settles; call `run()` to run them
 * earlier. There is no test-file discovery, concurrency, timeout, retry, or
 * isolation, and no asynchronous form.
 */
declare module "neo:test" {
  /** Counts from one call to `run()`. */
  export interface TestSummary {
    /** Tests executed by this run. */
    readonly total: number;
    readonly passed: number;
    readonly failed: number;
    /** One `name: error` line per failed test in this run. */
    readonly failures: ReadonlyArray<string>;
  }

  /** Thrown by `run()` after one or more tests failed. */
  export class TestFailureError extends Error {
    name: "TestFailureError";
    /** Stable code; match on this, never on the message. */
    code: "NEO_TEST_FAILURES";
    /** The same summary lines as `TestSummary.failures`. */
    failures: ReadonlyArray<string>;
  }

  /** Maximum registrations in one runtime; further calls throw. */
  export const MAX_TESTS: number;

  type AsyncTestResult =
    | { then: (...args: never[]) => unknown }
    | Generator<unknown, unknown, never>
    | AsyncGenerator<unknown, unknown, never>;

  /**
   * Queue a test. The callback must be an ordinary synchronous function;
   * direct async functions, generator functions and callable proxies throw a
   * `TypeError` at registration. Registering during a run throws
   * `NEO_TEST_DURING_RUN`. Native Promises, generators, thenables, and returns
   * whose `then` lookup reaches an accessor or proxy fail the test with
   * `NEO_TEST_ASYNC`; the check calls neither getters nor proxy traps. Inference
   * rejects visible Promise/thenable and generator return types. It cannot
   * recover a return type erased by `() => void`, `unknown`, or a cast;
   * runtime checks still apply.
   */
  export function test<F extends () => unknown>(
    name: string,
    fn: F & (Extract<ReturnType<NoInfer<F>>, AsyncTestResult> extends never ? unknown : never),
  ): void;

  /**
   * Run every queued test in registration order and return this run's counts.
   *
   * A failing test does not stop later tests. Details use the current
   * `globalThis.console.log` captured when this module initializes (or a no-op
   * if it is not a function). Replacing it before a dynamic import can suppress
   * details, but does not change the host failure count or the CLI's non-zero
   * exit. The module retains failures for the process report even if the caller
   * catches the thrown `TestFailureError`. A callback returning a
   * Promise is a failure (`NEO_TEST_ASYNC`), not a skipped or pending test.
   * With an empty queue this is a no-op returning zero counts.
   */
  export function run(): TestSummary;

  export default test;
}

/**
 * Synchronous standard streams, a runtime-local working directory, and host
 * process elevation status.
 *
 * The module is available with `--allow-run` or `--allow-stdio`; without one of
 * those flags the import fails and no host callback is installed. If the
 * embedder uses a module allowlist, it must include `neo:process`. Reading the
 * host's standard input requires one of those flags too; `--allow-stdio` is the
 * narrower choice. Writing `stdout` and `stderr` needs no separate permission
 * because `console` already reaches those streams.
 *
 * Every operation is synchronous. `write` and `read` are synchronous aliases of
 * `writeSync` and `readSync`; there is no Promise-returning form. `close()` is a
 * no-op because the streams belong to the host process. `isTerm()` reports the
 * host descriptor's terminal status.
 *
 * The working directory starts as the host process working directory and is
 * changed only for this runtime: `chdir()` never calls the host's global
 * `chdir`. A missing or non-Unicode initial directory is treated as
 * unavailable. Relative paths used by `neo:fs`, `neo:hash`, `neo:fetch`,
 * relative module entry names, and `neo:exec` children use this value. A
 * module's relative imports use the absolute base captured when that module was
 * declared, so they do not move with a later `chdir`. Absolute paths are
 * unchanged. While the directory is unavailable (including a non-Unicode
 * initial directory), relative operations that need it and `neo:exec` fail
 * explicitly rather than using the host's mutable directory. A relative import
 * fails only when its declaring module has no captured base; an absolute module
 * identity captures an absolute base and its relative imports can still
 * succeed. Absolute paths continue where the operation is otherwise safe. An
 * absolute `chdir()` can establish the runtime directory.
 * `chdir()` canonicalizes the target, requires an existing directory, and
 * leaves the previous value untouched on failure. The check reads directory
 * metadata only, never file contents. Windows drive-relative paths such as
 * `C:child` are rejected with `PROCESS_PATH`; verbatim-path behavior is
 * unverified.
 */
declare module "neo:process" {
  /** A standard output or error writer. Writes are synchronous and binary-safe. */
  export interface StdWriter {
    /**
     * Write a string as UTF-8 or a `Uint8Array` as bytes and return the number
     * of bytes written. The host stream is flushed before returning.
     *
     * @throws TypeError when `chunk` is not a string or `Uint8Array`.
     * @throws Error with code `PROCESS_IO` when the write fails.
     */
    write(chunk: string | Uint8Array): number;
    /** A synchronous alias of `write`. */
    writeSync(chunk: string | Uint8Array): number;
    /** True when the host stream is a terminal. */
    isTerm(): boolean;
    /** A no-op; the host owns the stream. */
    close(): void;
  }

  /** A standard input reader. Reads are synchronous and can block. */
  export interface StdReader {
    /**
     * Read at most `data.length` bytes into `data`, returning the count, `0` for
     * a zero-length buffer, or `null` at end of file. Reading blocks until
     * input, end of file, or an error; there is no timeout or nonblocking mode.
     *
     * @throws TypeError when `data` is not a `Uint8Array`.
     * @throws Error with code `PROCESS_IO` when the read fails.
     */
    read(data: Uint8Array): number | null;
    /** A synchronous alias of `read`. */
    readSync(data: Uint8Array): number | null;
    /** True when the host standard input is a terminal. */
    isTerm(): boolean;
    /** A no-op; the host owns the stream. */
    close(): void;
  }

  /** Thrown when `chdir` fails. */
  export class ChangeDirectoryError extends Error {
    /** The underlying error, when one was supplied to the constructor. */
    readonly cause?: unknown;
    /** A stable `PROCESS_CHDIR`, `PROCESS_CWD`, or `PROCESS_PATH` code is set on errors thrown by `chdir`. */
    readonly code?: string;
    constructor(message: string, options?: { cause?: unknown });
  }

  /** Thrown when `isElevated` cannot determine the host process elevation. */
  export class ElevationError extends Error {
    /** Set by `isElevated` when it throws; absent on manually constructed errors. */
    readonly code?: "PROCESS_ELEVATION" | "PROCESS_ELEVATION_UNSUPPORTED";
    constructor(message: string);
  }

  /** The host standard output stream. */
  export const stdout: StdWriter;
  /** The host standard error stream. */
  export const stderr: StdWriter;
  /** The host standard input stream. Reads require `--allow-stdio` or `--allow-run`. */
  export const stdin: StdReader;

  /**
   * Return this runtime's current working directory as an absolute path.
   *
   * @throws Error with code `PROCESS_CWD` when the host directory is
   * unavailable or not valid Unicode.
   */
  export function cwd(): string;

  /**
   * Change this runtime's working directory without changing the host process.
   *
   * @throws TypeError when `directory` is not a string.
   * @throws ChangeDirectoryError with code `PROCESS_CHDIR` when the target is
    * missing, is not a directory, is empty, or has a non-Unicode path. A
    * relative change while the runtime directory is unavailable reports
    * `PROCESS_CWD` instead of falling back to the host directory. A
    * drive-relative Windows path such as `C:child` reports `PROCESS_PATH`.
   */
  export function chdir(directory: string): void;

  /**
   * Report whether the host process has effective UID 0 on Unix or an elevated
   * primary process token on Windows. Each call takes a fresh snapshot. It does
   * not check Windows Administrators membership, change privileges, or predict
   * whether an operation will succeed.
   *
   * @throws ElevationError with code `PROCESS_ELEVATION` when the token query
   * fails or `PROCESS_ELEVATION_UNSUPPORTED` on an unsupported platform.
   */
  export function isElevated(): boolean;

  const _default: {
    cwd: typeof cwd;
    chdir: typeof chdir;
    isElevated: typeof isElevated;
    stdin: StdReader;
    stdout: StdWriter;
    stderr: StdWriter;
    ChangeDirectoryError: typeof ChangeDirectoryError;
    ElevationError: typeof ElevationError;
  };
  export default _default;
}

/** Available only with --allow-run; the child inherits host-user privileges. */
declare module "neo:exec" {
  export interface ExecResult {
    /** Exit code, or null when the child was terminated by a signal. */
    code: number | null;
    /** True only when the child exited with code 0. */
    success: boolean;
    /** Captured stdout bytes. */
    stdout: Uint8Array;
    /** Captured stderr bytes. */
    stderr: Uint8Array;
  }

  /**
   * Run a program synchronously and wait for it to exit. This blocks the
   * JavaScript thread.
   *
   * The program is invoked directly, not through a shell. The child's working
   * directory is the runtime's current directory, not the host process
   * directory; a relative program path is resolved against it. On Windows only
   * an explicit `.exe` or `.com` program is accepted; extensionless names and
   * non-`.exe`/`.com` files are rejected. Naming `cmd.exe` or
   * `powershell.exe` explicitly is allowed and uses that shell's normal
   * semantics.
   *
   * Input is written to stdin then closed. Each output stream is buffered up
   * to 8 MiB; overflow throws EXEC_IO. Non-zero exit is returned as data.
   * Invalid arguments throw TypeError. Other errors have a stable `code`:
   * EXEC_SPAWN, EXEC_WAIT, EXEC_IO, EXEC_WORKER, EXEC_UNSUPPORTED, or
   * EXEC_CWD. EXEC_CWD means the runtime working directory is unavailable;
   * no slot is reserved and no child is spawned, so the child cannot inherit
   * the mutable host directory.
   *
   * At most eight calls or background cleanup supervisors hold process-wide
   * slots. Exhaustion throws EXEC_WORKER before spawning. Fatal errors notify
   * the caller before potentially blocking cleanup, whose state is pending or
   * unknown. The first coordinator-observed fatal error is primary, favoring
   * queued worker errors over a wait failure. Cleanup runs in the background,
   * attempting to kill and reap the direct child and join workers. A slot is
   * held until resources are released, potentially forever if cleanup blocks
   * or reaping cannot be confirmed. Children or descendants may survive, as
   * may their effects; retries can repeat effects. Threads, pipes and slots can
   * persist, causing capacity exhaustion. The slot bound does not bound memory
   * or processes created by child code. Notification is not hard real-time.
   * No normal-execution timeout exists; inherited pipes can block indefinitely.
   * Output is unavailable on failure.
   *
   * Requires `--allow-run`. The child runs with the host user's authority,
   * including filesystem, environment, and network access.
   */
  export function exec(
    program: string,
    args?: string[],
    input?: Uint8Array,
  ): ExecResult;

  /**
   * Result of the single terminal call on a spawned-process handle.
   *
   * `complete` is true when both captured streams reached end of file before
   * the result was produced. `wait()` and `communicate()` only return a
   * successful result with `complete: true`. `kill()` returns a snapshot and
   * reports `complete: false` when a stream had not reached EOF, including a
   * descendant that still holds an output pipe or an output-limit overflow.
   */
  export interface SpawnResult extends ExecResult {
    /** True when both captured streams reached EOF before this result. */
    complete: boolean;
  }

  /**
   * An owned child process returned by `spawn`.
   *
   * Every method is synchronous and blocks the JavaScript thread. A handle
   * answers exactly one terminal call; its result (or error) is cached by this
   * module so later calls return fresh copies with the same bytes, code, and
   * message without contacting the child again. `kill()` after a terminal call
   * therefore returns the cached result rather than reporting a second signal.
   *
   * The runtime is single-threaded: while `wait()` or `communicate()` blocks,
   * no other JavaScript runs, so `kill()` cannot be invoked on the same handle
   * to interrupt it. `kill()` requests termination but has no hard deadline.
   */
  export interface SpawnedProcess {
    /**
     * Close the child's stdin and block until the direct child exits and both
     * captured streams reach EOF. A non-zero exit is returned as data.
     *
     * @throws Error with a stable code; see `spawn`.
     */
    wait(): SpawnResult;

    /**
     * Write `input` to the child's stdin, close it, and block like `wait()`.
     *
     * @throws TypeError when `input` is not a `Uint8Array`.
     * @throws Error with a stable code; see `spawn`.
     */
    communicate(input?: Uint8Array): SpawnResult;

    /**
     * Ask the direct child to terminate, attempt to reap it, and close stdin.
     * Descendants are not killed. Does not wait for a descendant that holds an
     * output pipe: the returned snapshot then has `complete: false`. If the
     * child had already exited, its existing status is returned and no signal
     * is sent. A successful signal is followed by a blocking reap to obtain
     * the exit status; a failed signal never falls through to a blocking wait.
     *
     * @throws Error with code `EXEC_KILL` or `EXEC_WAIT` when the signal or
     * reap cannot be confirmed. The message says cleanup is pending or
     * unknown and that the direct child may survive; the supervisor keeps
     * ownership and the slot and polls nonblockingly for the child to exit.
     */
    kill(): SpawnResult;
  }

  /**
   * Launch a child process immediately and return an owned synchronous handle.
   * Available only with `--allow-run`; requires the same host authority as
   * `exec`.
   *
   * The program is invoked directly, not through a shell, and the child's
   * working directory is the runtime's current directory. A relative program
   * path is resolved against it. The same Windows program policy as `exec`
   * applies: only an explicit `.exe` or `.com` is accepted.
   *
   * The direct child is supervised by a host thread. In normal operation it
   * reaps the child as soon as it exits, so an idle handle does not leave a
   * zombie; cleanup that cannot be confirmed is reported as pending or unknown
   * rather than assumed complete. Stdout and stderr are drained concurrently
   * and buffered up to 8 MiB per stream; excess bytes are read and discarded
   * so the child cannot block on a full pipe. `wait()` and
   * `communicate()` report `EXEC_IO` when a stream exceeded the limit and
   * return no output. Before reporting an IO, worker, or wait failure, the
   * supervisor makes a nonblocking attempt to stop the direct child. If
   * termination is confirmed, the error is delivered after that attempt; if it
   * is not, the message says cleanup is pending or unknown and that the direct
   * child may survive, and a background supervisor retains ownership and the
   * slot until it can reap the child. A failure therefore does not claim that
   * the child, its descendants, or their effects ceased instantly. An
   * unexpected supervisor panic leaves cleanup unknown; the child may remain
   * unreaped and its slot may be retained permanently. `kill()`
   * instead returns a snapshot, marked `complete: false` when a stream had not
   * reached EOF or had overflowed.
   *
   * A handle reserves one process-wide slot. A fully confirmed terminal call
   * releases it; a terminal call whose cleanup is pending, or a kill that
   * leaves a descendant holding an output pipe, keeps it reserved until the
   * child is reaped or the pipe closes. The same eight-slot limit bounds
   * `exec` calls, outstanding handles, and background cleanup supervisors. A
   * spawn with no slot fails with `EXEC_WORKER` before any child is started. A
   * handle whose JavaScript object becomes unreachable without a terminal call
   * keeps its child and slot until the runtime is dropped. Dropping the
   * runtime aborts outstanding handles: abort
   * requests are sent to all handles first, then acknowledgements are
   * collected under one shared 10-second budget, not 10 seconds per handle.
   * An acknowledgement means the supervisor has either confirmed the direct
   * child is reaped or attempted a nonblocking stop and kept ownership for
   * cleanup; it does not roll back side effects. A child that cannot be killed may survive or remain unreaped,
   * and its slot is retained until it can be reaped.
   *
   * Normal execution has no timeout: a child or descendant that holds a pipe
   * can block `wait()` and `communicate()` indefinitely, and `communicate()`
   * can block on a child or descendant that never reads stdin. The runtime is
   * single-threaded, so `kill()` cannot interrupt a blocked `wait()` or
   * `communicate()` on the same handle. `kill()` has no hard deadline and only addresses the
   * direct child but cannot kill descendants, and neither `kill()` nor runtime
   * drop promises that they stop. Output is not streamed and is unavailable on
   * failure. A terminal result is held in JavaScript for the lifetime of the
   * handle object; errors carry a stable `code` and are rethrown with the same
   * code and message on later calls. Codes are `EXEC_SPAWN`, `EXEC_WAIT`,
   * `EXEC_IO`, `EXEC_WORKER`, `EXEC_KILL` (stop requested but termination
   * unconfirmed), `EXEC_UNSUPPORTED`, `EXEC_CWD`, `EXEC_STATE` (invalid
   * sequence or internal state), and `EXEC_HANDLE` (unknown or already
   * consumed handle; reachable only through the undocumented host callback).
   * Windows child termination and reaping behavior is unverified.
   */
  export function spawn(program: string, args?: string[]): SpawnedProcess;
}

/**
 * Synchronous HTTP and HTTPS downloads. Requires `--allow-net`; without it the
 * module is absent and an import fails. Every download also requires
 * `--allow-fs`, because it writes the destination. JavaScript validation
 * (including option getters or proxy traps) can run or throw before the
 * native permission check. The download itself does not parse the URL, call
 * progress, or perform file or network I/O until that check passes. A relative
 * destination path is resolved against the runtime's current working directory,
 * not the host process directory.
 *
 * The network permission is broad outbound authority under the current user's
 * network access, including loopback, link-local, private, and metadata
 * addresses. It is not a sandbox, allowlist, or SSRF defense; a redirect may
 * change the host, port, or scheme, including an HTTPS-to-HTTP downgrade.
 *
 * Only absolute `http` and `https` URLs with a host and no userinfo are
 * accepted. Userinfo is rejected on the initial URL and on every redirect
 * target before the next request, including empty `@` and empty-password
 * forms; otherwise the client could turn it into an `Authorization: Basic` header.
 * The request is a GET; besides transport defaults, no headers are sent, and
 * there is no authentication, cookie jar, proxy, or automatic decompression.
 * Environment proxy discovery is explicitly disabled. The URL is never logged
 * or included in an error message. neo-js follows at most five redirects; a
 * chain that reaches the limit throws `FETCH_REDIRECT`. A final status outside
 * 2xx throws `FETCH_STATUS` with `error.status` set.
 *
 * HTTPS uses rustls with the bundled Mozilla webpki roots; certificate-chain
 * and hostname verification are always enabled, and the host trust store is
 * not consulted. There is no switch to disable verification.
 *
 * The body is streamed into a new private temporary directory in the
 * destination's parent directory, into a file neo-js opens there. On Unix the
 * directory is mode `0700` and the file mode `0600` (both subject to umask).
 * The open file handle is retained so its identity is checked before
 * publication and cleanup. The temp directory is also identity-checked
 * before removal (device/inode on Unix, a same-file handle elsewhere; Windows
 * is unverified), and removal is non-recursive. The file is synchronized
 * before publication. Temporary cleanup is best-effort: it can leave a temp
 * directory and possibly its file behind, without replacing the download error.
 * The destination is never written directly, so failures leave an existing
 * destination untouched on normal error paths when paths and callbacks are
 * trusted. With `overwrite: false` (the
 * default) publication uses a no-clobber hard link; a destination that exists
 * or appears during the download is not replaced. A filesystem without
 * hard-link support throws `FETCH_IO`. With `overwrite: true`, publication
 * renames over the destination atomically on Unix; Windows behavior is
 * unverified. On Unix the destination receives the temporary file's `0600`
 * mode (before umask), so a replacement does not preserve the old
 * destination's permissions; Windows ACL inheritance is unverified. The
 * parent directory and its path must be trusted against modification by other
 * users and same-user processes, including callbacks. A writable parent lets
 * an attacker replace even a `0700` directory. On all platforms identity
 * checks race with hard-link, rename, and removal: substitution can change the
 * destination or cause cleanup to delete a replacement. An abrupt process
 * termination can leave a `.neo-download-*.tmp` directory behind; it never
 * replaces the destination.
 *
 * Wrong argument or option types and out-of-range options throw `TypeError`.
 * Other failures throw `Error` with a stable `code`: `FETCH_PERMISSION`,
 * `FETCH_INVALID_URL`, `FETCH_OPTION`, `FETCH_STATUS`, `FETCH_REDIRECT`,
 * `FETCH_TIMEOUT`, `FETCH_NETWORK`, `FETCH_IO`, `FETCH_LIMIT`,
 * `FETCH_TRUNCATED`, `FETCH_EXISTS`, `FETCH_PATH`, `FETCH_CWD`, `FETCH_TEMP`,
 * `FETCH_CALLBACK`, or `FETCH_CALLBACK_ASYNC`. Messages never include the
 * URL, a credential, a path, or server text.
 *
 * This function blocks the JavaScript thread; callbacks run on that thread.
 * Transport and file work use a joined worker and are safe to invoke from a
 * Tokio runtime. At most 32 downloads can be active process-wide; additional
 * calls, including nested calls, fail with `FETCH_NETWORK` rather than wait.
 * There is no asynchronous form.
 */
declare module "neo:fetch" {
  type ThenableResult = { then: (...args: never[]) => unknown };
  type Progress = (received: number, total: number | null) => unknown;

  /**
   * Options for `download`. Only own properties count; omitted, inherited,
   * or `undefined` options use their defaults. Own getters and proxy traps
   * may run before the native filesystem permission check.
   *
   * The default `void` callback type follows
   * TypeScript's rule that callers can ignore any return value, even a Promise.
   * Preserve the inferred return type when passing options to `download` for
   * a static check. Runtime checks also apply to callbacks whose actual
   * return type is hidden by a `void` or `unknown` annotation or a cast.
   */
  export interface DownloadOptions<R = void> {
    /**
     * Maximum response-body bytes to accept. Defaults to 64 MiB; the accepted
     * range is an integer from 0 through 512 MiB. A declared Content-Length
     * above this bound, or a streamed body that exceeds it, throws
     * `FETCH_LIMIT`.
     */
    maxBytes?: number;
    /**
     * Network-operation timeout in milliseconds, including connection,
     * redirects, header and body reads, and time spent in `progress`. The
     * final local sync and publication are outside this timeout. Defaults to
     * 30,000; the accepted range is an integer from 1 through 600,000.
     */
    timeoutMs?: number;
    /**
     * Replace an existing destination during publication. Defaults to
     * `false`, which rejects an existing destination with `FETCH_EXISTS`.
     * Unix publication is atomic; Windows behavior is unverified.
     */
    overwrite?: boolean;
    /**
     * Synchronous progress callback. It is called once with `received` 0
     * before the body is read, then after each chunk with the cumulative
     * bytes. `total` is the declared Content-Length, or `null` when unknown.
     * A throw aborts with `FETCH_CALLBACK`. A directly recognizable async
     * function is rejected before invocation or I/O with `FETCH_CALLBACK_ASYNC`;
     * the same code aborts on a returned Promise or thenable (including from
     * a wrapper). A Promise is marked handled without awaiting it. A `then`
     * accessor, or a proxy encountered while inspecting `then`, is rejected
     * without invoking its getter or traps. Other return values are ignored.
     * On callback failure the download does not publish the destination. It
     * attempts temporary cleanup and joins the worker before throwing the
     * original download error, even if cleanup fails. The deadline is checked
     * after each callback and before publication. A callback may start
     * another download; each call has independent state. The callback must
     * be trusted not to modify the destination, its parent path, or temporary
     * entries.
     */
    progress?: (received: number, total: number | null) => R;
  }

  /** Result of a completed download. */
  export interface DownloadResult {
    /** Number of response-body bytes written to the destination. */
    bytes: number;
    /** Final HTTP status, which is in the 2xx range. */
    status: number;
  }

  /**
   * Download `url` synchronously to `path` and return the byte count and
   * final status.
   *
   * Wrong argument or option types and out-of-range options throw `TypeError`.
   * Other failures throw `Error` with a stable `code`: `FETCH_PERMISSION`,
   * `FETCH_INVALID_URL`, `FETCH_OPTION`, `FETCH_STATUS`, `FETCH_REDIRECT`,
   * `FETCH_TIMEOUT`, `FETCH_NETWORK`, `FETCH_IO`, `FETCH_LIMIT`,
   * `FETCH_TRUNCATED`, `FETCH_EXISTS`, `FETCH_PATH`, `FETCH_CWD`, `FETCH_TEMP`,
   * `FETCH_CALLBACK`, or `FETCH_CALLBACK_ASYNC`. Messages never include the
   * URL, a credential, a path, or server text. There is no asynchronous form.
   */
  export function download<F extends Progress>(
    url: string,
    path: string,
    options?: DownloadOptions<unknown> & {
      progress?: F &
        (Extract<ReturnType<NoInfer<F>>, ThenableResult> extends never ? unknown : never);
    },
  ): DownloadResult;
}

/**
 * Synchronous Windows Registry access. Requires `--allow-registry` and is
 * absent without it. Migrate imports from `neo:winregistry` to
 * `neo:win-registry`; both names refer to the same module. Neither spelling
 * accepts nonempty import attributes.
 *
 * This API grants full read, write, and delete authority under the current
 * process token. It is not a sandbox: a script can change any key or value the
 * user can, including keys that run programs at sign-in. Grant the permission
 * only to trusted scripts.
 *
 * The standalone functions release any handles before returning. For
 * repeated operations on a single owned handle, use `Registry.openKey` or
 * `Registry.createKey` with a full path and dispose the returned key with
 * `using` or `close()`. The standalone `writeString` does not create a key;
 * call `createKey` first. `deleteKey` removes one key and its values, but not
 * its subkeys.
 *
 * Hive names are explicit (`HKCR`, `HKCU`, `HKLM`, `HKU`, `HKCC`, or their
 * full `HKEY_*` names), case-insensitive, and have no default. Paths are
 * relative to the hive and use backslash as the only separator; the empty path
 * names the hive root. A path may not contain an empty `.` or `..` component,
 * a NUL, more than 16,384 UTF-16 units in total, or more than 255 units in one
 * component. A value name may be empty, meaning the key's default value, and
 * otherwise may not contain a NUL or exceed 16,383 UTF-16 units. Every string
 * argument must be well-formed UTF-16; an unpaired surrogate throws
 * `WINREGISTRY_ENCODING` rather than being replaced.
 *
 * Typed helpers are strict. `readString` accepts only `REG_SZ` and
 * `REG_EXPAND_SZ`; `readUint32` only `REG_DWORD`; `readUint64` only
 * `REG_QWORD`; `readBinary` only `REG_BINARY`. A different type throws
 * `WINREGISTRY_TYPE_MISMATCH`, never a converted value. `readValue` returns
 * the raw numeric type and exact bytes, including a string's trailing UTF-16
 * NUL, so no bytes are lost, and it returns any numeric registry type
 * unchanged, including types this module has no helper for. Strings must be
 * valid, NUL-terminated UTF-16LE with no data after the terminator; an odd
 * length, a missing terminator, data after it, or an unpaired surrogate is
 * rejected instead of truncated or replaced. A value read asks for its
 * required size and exact type, rejects a size above 8 MiB before allocating,
 * then allocates exactly that size. If the value grows between calls, it grows
 * only on `ERROR_MORE_DATA`, up to the same bound and a bounded number of
 * retries.
 *
 * `writeString` stores `REG_SZ`; `writeExpandString` stores the unexpanded
 * `REG_EXPAND_SZ` text; `writeUint32` stores four little-endian bytes;
 * `writeUint64` stores eight little-endian bytes; `writeBinary` stores the
 * exact bytes. A string with an embedded NUL, a DWORD outside 0 through
 * 4,294,967,295, a QWORD outside 0 through 18,446,744,073,709,551,615, or a
 * value above 8 MiB is rejected. There is no raw write for other registry
 * types, and no `REG_MULTI_SZ`, `REG_NONE`, `REG_LINK`, or resource-list
 * helper.
 *
 * Enumeration is bounded. `listSubKeys` and `listValues` return at most 4,096
 * entries by default and accept an explicit integer limit from 1 through
 * 4,096. They ask the key for its entry counts and maximum name lengths first.
 * A count above the limit throws `WINREGISTRY_LIMIT` before any entry is
 * fetched. `listValues` queries only each value's name and numeric type; it
 * does not fetch value data. Name buffers are sized from the reported maximum
 * and grown only on `ERROR_MORE_DATA`, up to the documented caps and a bounded
 * number of retries. At the limit, one additional index is checked and an
 * entry there throws `WINREGISTRY_LIMIT`. Enumeration is not an atomic
 * snapshot: concurrent changes may cause missed or duplicated entries,
 * errors, or changes after the lookahead. Returned names and types are the registry's, except value
 * names are limited to 16,383 UTF-16 units. A returned registry name that is
 * not valid UTF-16 throws `WINREGISTRY_ENCODING`.
 *
 * Before a host call, the JavaScript wrapper checks the UTF-16 length of each
 * primitive string against these caps, so an oversized path, value name, or
 * string value is not converted to a native Rust string. It rejects proxies
 * and boxed or other non-primitive objects without reading their properties.
 * Native validation runs again after conversion.
 *
 * On a non-Windows host, `isAvailable()` returns `false` and every operation
 * throws `WINREGISTRY_UNSUPPORTED`; it never returns a fabricated success,
 * `false`, or empty list. Argument validation still runs first, so an invalid
 * hive or path reports its own invalid-argument code.
 *
 * Errors: a wrong JavaScript argument type throws `TypeError`, and an
 * out-of-range limit or DWORD throws `TypeError`; nothing is coerced. Other
 * failures throw an `Error` with a stable `code`:
 * `WINREGISTRY_INVALID_HIVE`, `WINREGISTRY_INVALID_PATH`,
 * `WINREGISTRY_INVALID_NAME`, `WINREGISTRY_INVALID_VALUE`,
 * `WINREGISTRY_ENCODING`, `WINREGISTRY_TYPE_MISMATCH`,
 * `WINREGISTRY_NOT_FOUND`, `WINREGISTRY_ACCESS_DENIED`, `WINREGISTRY_CLOSED`,
 * `WINREGISTRY_LIMIT`, `WINREGISTRY_UNSUPPORTED`, or `WINREGISTRY_IO`.
 * Messages never include a path, value name, or value data.
 *
 * Limitations: the process-default WOW64 view is used, so a 32-bit process can
 * see different or unavailable values; WOW64 views cannot be selected. There
 * is no ACL or security-descriptor access, no
 * transactional update, no recursive delete, no key rename or copy, no change
 * notification, no native key metadata or last-write time, and no remote registry
 * support. A non-Windows host cannot read or write a Windows registry.
 */
declare module "neo:win-registry" {
  /** A predefined registry root, by short or full name. */
  export type Hive =
    | "HKCR" | "HKCU" | "HKLM" | "HKU" | "HKCC"
    | "HKEY_CLASSES_ROOT" | "HKEY_CURRENT_USER" | "HKEY_LOCAL_MACHINE"
    | "HKEY_USERS" | "HKEY_CURRENT_CONFIG";

  /** Windows access masks; combine with `|`. No WOW64 view flags are accepted. */
  export const Rights: Readonly<{
    QUERY_VALUE: 0x00001;
    SET_VALUE: 0x00002;
    CREATE_SUB_KEY: 0x00004;
    ENUMERATE_SUB_KEYS: 0x00008;
    NOTIFY: 0x00010;
    CREATE_LINK: 0x00020;
    READ: 0x20019;
    WRITE: 0x20006;
    ALL_ACCESS: 0xf003f;
  }>;

  /** Maximum concurrently open owned keys per runtime, across both import spellings. */
  export const MAX_OPEN_KEYS: 64;

  /**
   * An owned Windows registry key. Only `Registry` can construct keys; the
   * native handle is never exposed. Use `using` or `try`/`finally` to close
   * reliably. The runtime closes any remaining owned handles on teardown.
   */
  export class RegistryKey {
    #private;
    private constructor();
    /** Full path as supplied when the key was acquired; readable after close. */
    get path(): string;
    /** True only if this call created the final key; readable after close. */
    get created(): boolean;
    /** Release this key's handle. Repeated calls have no effect. */
    close(): void;
    /** Same operation as `close()`, also called at `using` scope exit. */
    [Symbol.dispose](): void;
    /**
     * Read `REG_SZ` or unexpanded `REG_EXPAND_SZ` on this handle.
     * Requires `Rights.QUERY_VALUE`; a closed key throws `WINREGISTRY_CLOSED`.
     */
    getString(name: string): string;
    /**
     * Write `REG_SZ` on this handle. Requires `Rights.SET_VALUE`; a closed
     * key throws `WINREGISTRY_CLOSED`. The empty name is the default value.
     */
    setString(name: string, value: string): void;
  }

  /**
   * Synchronous owned-key API. Paths include a hive and a nonempty subkey,
   * for example `"HKCU\\Software\\MyApp"`; bare hives are not owned keys.
   * Access is a nonzero combination of bits in `Rights.ALL_ACCESS`, using
   * the process-default registry view. Windows still checks permissions.
   * The mask limits this handle's operations, not the script's overall
   * `--allow-registry` authority.
   */
  export const Registry: Readonly<{
    /** Open an existing key with the requested access (default `Rights.READ`). */
    openKey(path: string, access?: number): RegistryKey;
    /** Create or open a key with the requested access (default `Rights.ALL_ACCESS`). */
    createKey(path: string, access?: number): RegistryKey;
  }>;

  /** Numeric registry value types, as stored by Windows. */
  export const ValueType: Readonly<{
    NONE: 0;
    SZ: 1;
    EXPAND_SZ: 2;
    BINARY: 3;
    DWORD: 4;
    DWORD_BIG_ENDIAN: 5;
    LINK: 6;
    MULTI_SZ: 7;
    RESOURCE_LIST: 8;
    FULL_RESOURCE_DESCRIPTOR: 9;
    RESOURCE_REQUIREMENTS_LIST: 10;
    QWORD: 11;
  }>;

  /** The supported short hive names, frozen. */
  export const HIVES: Readonly<Array<"HKCR" | "HKCU" | "HKLM" | "HKU" | "HKCC">>;

  /** A raw registry value: its numeric type and exact stored bytes. */
  export interface RawValue {
    /** One of the `ValueType` numbers. */
    readonly type: number;
    /** Exact bytes, including a string's trailing UTF-16 NUL. */
    readonly data: Uint8Array;
  }

  /** A value name and its numeric registry type. */
  export interface ValueEntry {
    /** The value name; `""` is the default value. */
    readonly name: string;
    /** One of the `ValueType` numbers. */
    readonly type: number;
  }

  /** True only on a Windows host; permission alone does not make it true. */
  export function isAvailable(): boolean;

  /**
   * Create a key, or open it when it already exists.
   *
   * @returns `true` when a new key was created, `false` when it already
   * existed.
   */
  export function createKey(hive: Hive | string, path: string): boolean;

  /**
   * True when the key exists and can be opened for reading.
   *
   * A key that exists but cannot be opened throws
   * `WINREGISTRY_ACCESS_DENIED`; it is not reported as `false`.
   */
  export function keyExists(hive: Hive | string, path: string): boolean;

  /**
   * Delete one key and its values. Subkeys are not removed.
   *
   * @returns `true` when the key was deleted, `false` when it did not exist.
   */
  export function deleteKey(hive: Hive | string, path: string): boolean;

  /**
   * Delete one value.
   *
   * @returns `true` when the value was deleted, `false` when it did not exist.
   * A missing key throws `WINREGISTRY_NOT_FOUND`.
   */
  export function deleteValue(
    hive: Hive | string,
    path: string,
    name: string,
  ): boolean;

  /**
   * Enumerate direct subkey names.
   *
   * @throws Error with `code` `WINREGISTRY_LIMIT` when more than `limit`
   * entries are reported initially or found by the bounded lookahead.
   */
  export function listSubKeys(
    hive: Hive | string,
    path: string,
    limit?: number,
  ): string[];

  /**
   * Enumerate value names and numeric types. This does not fetch value data.
   *
   * @throws Error with `code` `WINREGISTRY_LIMIT` when more than `limit`
   * entries are reported initially or found by the bounded lookahead.
   */
  export function listValues(
    hive: Hive | string,
    path: string,
    limit?: number,
  ): ValueEntry[];

  /** Read one value without converting it. */
  export function readValue(
    hive: Hive | string,
    path: string,
    name: string,
  ): RawValue;

  /**
   * Read a `REG_SZ` or `REG_EXPAND_SZ` value. Does not expand environment
   * references.
   *
   * @throws Error with `code` `WINREGISTRY_TYPE_MISMATCH` for another type.
   * @throws Error with `code` `WINREGISTRY_ENCODING` for invalid UTF-16.
   */
  export function readString(
    hive: Hive | string,
    path: string,
    name: string,
  ): string;

  /** Read a `REG_DWORD` value as a JavaScript number. */
  export function readUint32(
    hive: Hive | string,
    path: string,
    name: string,
  ): number;

  /** Read a `REG_QWORD` value as a JavaScript bigint. */
  export function readUint64(
    hive: Hive | string,
    path: string,
    name: string,
  ): bigint;

  /** Read a `REG_BINARY` value as exact bytes. */
  export function readBinary(
    hive: Hive | string,
    path: string,
    name: string,
  ): Uint8Array;

  /** Write a `REG_SZ` value. The key must already exist. */
  export function writeString(
    hive: Hive | string,
    path: string,
    name: string,
    value: string,
  ): void;

  /** Write an unexpanded `REG_EXPAND_SZ` value. The key must already exist. */
  export function writeExpandString(
    hive: Hive | string,
    path: string,
    name: string,
    value: string,
  ): void;

  /**
   * Write a `REG_DWORD` value.
   *
   * @throws TypeError unless `value` is an integer from 0 through
   * 4,294,967,295.
   */
  export function writeUint32(
    hive: Hive | string,
    path: string,
    name: string,
    value: number,
  ): void;

  /**
   * Write a `REG_QWORD` value.
   *
   * @throws TypeError unless `value` is a bigint.
   * @throws Error with `code` `WINREGISTRY_INVALID_VALUE` when it is outside
   * 0 through 18,446,744,073,709,551,615.
   */
  export function writeUint64(
    hive: Hive | string,
    path: string,
    name: string,
    value: bigint,
  ): void;

  /** Write a `REG_BINARY` value with exact bytes, up to 8 MiB. */
  export function writeBinary(
    hive: Hive | string,
    path: string,
    name: string,
    data: Uint8Array,
  ): void;
}

/** @deprecated Import from `neo:win-registry` instead. No release is scheduled to remove this alias. */
declare module "neo:winregistry" {
  export * from "neo:win-registry";
}

declare module "neo:hash" {
  export type Algorithm =
    | "sha224" | "sha256" | "sha384" | "sha512"
    | "sha3-224" | "sha3-256" | "sha3-384" | "sha3-512"
    | "blake2b-512" | "blake2s-256" | "blake3";
  /** Hash bytes and return a lowercase hexadecimal digest. */
  export function digest(algorithm: Algorithm, data: Uint8Array): string;
  /** Available only with --allow-fs; hashes a file without loading it all at once. */
  export function digestFile(algorithm: Algorithm, path: string): string;
}

/**
 * Synchronous AES-256-GCM authenticated encryption. Requires no permission
 * flag and grants no host capability.
 *
 * The native wrapper and callbacks are private to the module. There are no
 * `__neoCrypto*` globals. This does not isolate values from other code in the
 * same realm.
 *
 * The key is exactly 32 bytes. The plaintext, associated data, and returned
 * envelope are `Uint8Array`s; the API is binary-safe and never interprets
 * bytes as text. This runtime does not provide `TextEncoder`/`TextDecoder`, so
 * callers encode text themselves.
 *
 * Every `encrypt` call draws a fresh 96-bit nonce from the operating system
 * CSPRNG. The envelope is:
 *
 * | offset | bytes | field                    |
 * | ------ | ----- | ------------------------ |
 * | 0      | 4     | magic, ASCII `NEO1`      |
 * | 4      | 1     | version, `1`             |
 * | 5      | 1     | algorithm, `1` (AES-256-GCM) |
 * | 6      | 12    | nonce                    |
 * | 18     | n     | ciphertext               |
 * | 18+n   | 16    | authentication tag       |
 *
 * The header and nonce plus `associatedData` are authenticated. `decrypt`
 * throws unless the same key and associated data are supplied and every
 * envelope byte is unchanged; plaintext is never returned on failure.
 *
 * Nonce randomness: a random 96-bit nonce gives a collision probability near
 * 2^-32 after about 2^32 encryptions under one key. A key should therefore be
 * retired well before that, and a nonce must never be chosen by the caller.
 *
 * Key management: `generateKey` uses the OS CSPRNG. Keep the key secret, use a
 * distinct key per purpose, and protect every place it is stored. The module
 * does not store, derive, wrap, rotate, or persist keys. It does not promise
 * zeroization: key, plaintext, and ciphertext bytes may remain in Rust buffers
 * and the JavaScript heap until reclaimed. It is not a sandbox.
 *
 * neo-js limits: at most 8 MiB of plaintext, 8 MiB of caller associated data,
 * and an envelope of 8 MiB + 34 bytes. These are not intrinsic AES-GCM limits.
 * Native code checks these sizes before copying inputs or allocating input
 * buffers. A failed
 * reservation is reported as `NEO_CRYPTO_LIMIT`, but an allocation failure
 * inside QuickJS, the Rust allocator, or the JavaScript heap can still abort
 * the process or raise an uncatchable error; out-of-memory recovery is not
 * guaranteed.
 *
 * Errors: an argument that is not a `Uint8Array` throws `TypeError`. Other
 * failures throw an `Error` with a stable `code`: `NEO_CRYPTO_KEY` (key length),
 * `NEO_CRYPTO_FORMAT` (truncated or unknown envelope), `NEO_CRYPTO_AUTH`
 * (wrong key, wrong associated data, or tampering), `NEO_CRYPTO_RANDOM` (OS
 * CSPRNG failure), `NEO_CRYPTO_LIMIT` (size bound or refused allocation), or
 * `NEO_CRYPTO_DETACHED` (detached or out-of-bounds view). Messages never
 * include key, plaintext, or ciphertext bytes.
 *
 * The envelope is specific to neo-js. The underlying AES-256-GCM primitive is
 * standard and is tested against published NIST vectors; the envelope is also
 * tested against a fixture produced independently with Node/OpenSSL. Another
 * tool must reproduce the envelope and associated-data construction to
 * interoperate. age file encryption is not provided by this module.
 */
declare module "neo:crypto" {
  /** Generate a 32-byte AES-256 key from the operating system CSPRNG. */
  export function generateKey(): Uint8Array;

  /**
   * Encrypt `plaintext` into a fresh versioned envelope.
   *
   * @throws TypeError when an argument is not a `Uint8Array`.
   * @throws Error with `code` `NEO_CRYPTO_KEY` when the key is not 32 bytes.
   * @throws Error with `code` `NEO_CRYPTO_RANDOM` when the CSPRNG fails.
   * @throws Error with `code` `NEO_CRYPTO_LIMIT` when the input exceeds the
   * neo-js size bound or an allocation is refused.
   * @throws Error with `code` `NEO_CRYPTO_DETACHED` when an input view is
   * detached or out of bounds.
   */
  export function encrypt(
    key: Uint8Array,
    plaintext: Uint8Array,
    associatedData?: Uint8Array,
  ): Uint8Array;

  /**
   * Verify and decrypt one envelope.
   *
   * @throws TypeError when an argument is not a `Uint8Array`.
   * @throws Error with `code` `NEO_CRYPTO_AUTH` for a wrong key, different
   * associated data, or tampering that leaves a structurally valid envelope.
   * @throws Error with `code` `NEO_CRYPTO_FORMAT` for a truncated envelope or
   * an unknown magic, version, or algorithm.
   * @throws Error with `code` `NEO_CRYPTO_KEY` when the key is not 32 bytes.
   * @throws Error with `code` `NEO_CRYPTO_LIMIT` when the envelope or
   * associated data exceeds its documented bound or an allocation is refused.
   * @throws Error with `code` `NEO_CRYPTO_DETACHED` when an input view is
   * detached or out of bounds.
   */
  export function decrypt(
    key: Uint8Array,
    envelope: Uint8Array,
    associatedData?: Uint8Array,
  ): Uint8Array;
}

/**
 * Synchronous age file encryption for X25519 recipients. Requires no
 * permission flag and grants no host capability.
 *
 * The native wrapper and callbacks are private to the module. There are no
 * `__neoAge*` globals, and the module checks typed-array and string arguments
 * without consulting caller-replaceable prototypes. This does not isolate
 * values from other code in the same realm.
 *
 * `encrypt` returns a standard binary age v1 file encrypted to one X25519
 * recipient. `decrypt` accepts standard binary age v1 files encrypted to an
 * X25519 recipient. ASCII armor, passphrase encryption, SSH keys, plugins, and
 * post-quantum recipients are not exposed. The `age` command-line tool can
 * decrypt this module's output, and this module decrypts its binary X25519
 * output.
 *
 * Keys are standard Bech32 text. `generateIdentity` and `encrypt` ask the
 * vetted age crate for randomness, which draws from the operating system
 * CSPRNG. age 0.11 exposes no fallible or injectable RNG path, so the module
 * contains panics from the age call and reports them as `NEO_AGE_RANDOM`. The
 * known panic cause is OS RNG failure; a `panic = "abort"` build cannot contain
 * it and aborts instead. Keep the identity secret, use a distinct identity per
 * purpose, and protect where it is stored. The module does not store, wrap,
 * rotate, or persist keys. It does not promise zeroization: identity text,
 * plaintext, ciphertext, and intermediate key material may remain in Rust
 * buffers and the JavaScript heap until reclaimed. It is not a sandbox.
 *
 * neo-js limits: at most 8 MiB of plaintext and a 9 MiB complete age file.
 * Decryption also stops if the recovered plaintext would exceed 8 MiB. Identity
 * and recipient text longer than 512 bytes is rejected as malformed; the
 * original UTF-8 byte length is checked before trimming, and a native UTF-16
 * length check keeps any conversion small, so huge padded keys are rejected
 * without being copied. These are neo-js limits, not intrinsic age limits; a
 * valid external age file with an unusually large header can be refused. Native
 * code bounds inputs before copying them. A refused allocation is reported as
 * `NEO_AGE_LIMIT`, but an allocation failure inside QuickJS, the Rust
 * allocator, or the JavaScript heap can still abort the process or raise an
 * uncatchable error; out-of-memory recovery is not guaranteed.
 *
 * Errors: a non-string key or non-`Uint8Array` data throws `TypeError`. Other
 * failures throw an `Error` with a stable `code`: `NEO_AGE_IDENTITY` (malformed
 * identity text), `NEO_AGE_RECIPIENT` (malformed recipient text),
 * `NEO_AGE_FORMAT` (truncated, unknown, or unsupported age file),
 * `NEO_AGE_IDENTITY_MISMATCH` (no supplied identity matched), `NEO_AGE_AUTH`
 * (a header MAC or payload tag did not verify), `NEO_AGE_LIMIT` (size bound or
 * refused allocation), `NEO_AGE_RANDOM` (OS CSPRNG failure or a contained
 * panic in the age encryption path), or `NEO_AGE_INTERNAL` (an unexpected
 * failure from the age crate). Messages never include identity, recipient,
 * plaintext, or ciphertext bytes.
 *
 * A damaged recipient stanza is indistinguishable from a file encrypted to a
 * different identity, so both are reported as `NEO_AGE_IDENTITY_MISMATCH`.
 * Payload tampering and header-MAC failures are `NEO_AGE_AUTH`.
 */
declare module "neo:age" {
  /** A new X25519 identity and the matching recipient, as Bech32 text. */
  export interface IdentityPair {
    /** `AGE-SECRET-KEY-1...`. Keep secret. */
    identity: string;
    /** `age1...`, the public recipient for `identity`. */
    recipient: string;
  }

  /**
   * Generate a new X25519 identity and its recipient.
   *
   * @throws Error with `code` `NEO_AGE_RANDOM` when the OS CSPRNG fails or the
   * age call panics in a build that unwinds. A `panic = "abort"` build cannot
   * contain this failure.
   */
  export function generateIdentity(): IdentityPair;

  /**
   * Derive the recipient for an X25519 identity.
   *
   * @throws TypeError when `identity` is not a string.
   * @throws Error with `code` `NEO_AGE_IDENTITY` for malformed identity text.
   */
  export function identityToRecipient(identity: string): string;

  /**
   * Encrypt `plaintext` to one X25519 recipient as a binary age v1 file.
   *
   * @throws TypeError when `recipient` is not a string or `plaintext` is not a
   * `Uint8Array`.
   * @throws Error with `code` `NEO_AGE_RECIPIENT` for malformed recipient text.
   * @throws Error with `code` `NEO_AGE_LIMIT` when the plaintext exceeds 8 MiB,
   * the output would exceed 9 MiB, or an allocation is refused.
   * @throws Error with `code` `NEO_AGE_DETACHED` when `plaintext` is a detached
   * or out-of-bounds view.
   * @throws Error with `code` `NEO_AGE_RANDOM` when the OS CSPRNG fails or the
   * age call panics in a build that unwinds. A `panic = "abort"` build cannot
   * contain this failure.
   */
  export function encrypt(recipient: string, plaintext: Uint8Array): Uint8Array;

  /**
   * Decrypt a binary age v1 file with one X25519 identity.
   *
   * @throws TypeError when `identity` is not a string or `ciphertext` is not a
   * `Uint8Array`.
   * @throws Error with `code` `NEO_AGE_IDENTITY` for malformed identity text.
   * @throws Error with `code` `NEO_AGE_FORMAT` for a truncated, unknown, or
   * unsupported age file, such as a passphrase-encrypted file.
   * @throws Error with `code` `NEO_AGE_IDENTITY_MISMATCH` when no supplied
   * identity matches, including a damaged recipient stanza.
   * @throws Error with `code` `NEO_AGE_AUTH` when a header MAC or payload tag
   * fails to verify.
   * @throws Error with `code` `NEO_AGE_LIMIT` when the ciphertext exceeds 9 MiB,
   * the recovered plaintext would exceed 8 MiB, or an allocation is refused.
   * @throws Error with `code` `NEO_AGE_DETACHED` when `ciphertext` is a detached
   * or out-of-bounds view.
   */
  export function decrypt(identity: string, ciphertext: Uint8Array): Uint8Array;
}
