---
title: "Filesystem"
description: "Filesystem calls, symlinks, permissions, and limits."
---

`neo:fs` is a bounded subset of the usual filesystem API. It is
available only with `--allow-fs` and only when the runtime's module allowlist
admits `neo:fs`. Both gates are checked before the `fs` global or any native
callback is built. The global `fs` object and the module expose the same
functions and `FsError` class.

| Function                              | Description                                                             |
| ------------------------------------- | ----------------------------------------------------------------------- |
| `readText(path)`                      | Read a regular file as UTF-8 text.                                      |
| `writeText(path, data)`               | Write text, creating or truncating the file.                            |
| `readFile(path)`                      | Read a regular file on a worker. Returns `Promise<Uint8Array>`.         |
| `readFileSync(path)`                  | Read a regular file as a `Uint8Array`, blocking the JavaScript thread.  |
| `writeFile(path, data, options?)`     | Write a `Uint8Array` on a worker. Returns `Promise<void>`.              |
| `writeFileSync(path, data, options?)` | Write a `Uint8Array`, blocking the JavaScript thread.                   |
| `copyFile(source, destination)`       | Stream a regular file into a new file; never overwrite.                 |
| `stat(path)`                          | Metadata on a worker, following a final symlink. `Promise<FileInfo>`.   |
| `statSync(path)`                      | Metadata, following a final symlink, blocking the JavaScript thread.    |
| `lstat(path)`                         | Metadata on a worker without following a final symlink.                 |
| `lstatSync(path)`                     | Metadata without following a final symlink, blocking the thread.        |
| `exists(path)`                        | Existence check, following a final symlink.                             |
| `readDir(path)`                       | List a directory on a worker. Returns `Promise<DirEntry[]>`.            |
| `readDirSync(path)`                   | Non-recursive, sorted listing, blocking the JavaScript thread.          |
| `mkdir(path, options?)`               | Create a directory on a worker. Returns `Promise<void>`.                |
| `mkdirSync(path, options?)`           | Create a directory; `{recursive: true}` creates parents.                |
| `remove(path, options?)`              | Remove a file, link, or directory on a worker. Returns `Promise<void>`. |
| `removeSync(path, options?)`          | Remove a file, link, or directory; `{recursive: true}` removes a tree.  |
| `rename(source, destination)`         | Rename a file, directory, or symlink entry using the host OS.           |

```js
import { readDir, stat } from "neo:fs";

// Read-only example: it lists the current directory and inspects a known
// entry only if present. It does not create, overwrite, or delete user files.
const entries = await readDir(".");
if (entries.some((entry) => entry.name === "README.md")) {
	console.log((await stat("./README.md")).size);
}
```

## Working directory

Relative paths use the runtime's working directory, not the host process's
mutable directory. `neo:process.chdir` changes only that runtime. If the
runtime directory is unavailable, `readFile`, `writeFile`, `stat`, `lstat`,
`readDir`, `mkdir`, and `remove` reject with `FS_CWD` during a managed turn,
and synchronous operations such as `readFileSync`, `writeFileSync`, `statSync`,
`readDirSync`, `mkdirSync`, and `removeSync` throw `FS_CWD`. None falls back to
the host directory. Absolute paths and `..` components still reach everything
the host user can reach; the working directory is coordination, not
confinement.

## Migrate asynchronous operations

`readFile`, `writeFile`, `stat`, `lstat`, `readDir`, `mkdir`, and `remove`
return Promises on both `fs` and `neo:fs`. Rename an existing blocking call to
its `Sync` counterpart, or await the asynchronous form in a module. `readText`,
`writeText`, and the remaining filesystem methods are still synchronous.
During managed module evaluation, invalid arguments and filesystem errors
reject the Promise. The `Sync` forms still throw. Outside a managed turn, the
asynchronous forms throw `FS_ASYNC_OUTSIDE_TURN` before any I/O. This includes
calls from `NeoJs::eval`, `NeoJs::run`, and the CLI's automatic `neo:test`
runner after the module returns.

### Breaking change: readDir

`readDir(path)` used to return `DirEntry[]` synchronously. It now runs on a
worker and returns `Promise<DirEntry[]>`. In a managed module, write
`await readDir(path)`; in synchronous code use `readDirSync(path)`. Calling
`readDir` outside a managed turn throws `FS_ASYNC_OUTSIDE_TURN` before any
I/O. This is the same migration shape as `readFile` and `writeFile`.

### Breaking change: mkdir

`mkdir(path, options?)` used to return `void` synchronously. It now runs on a
worker and returns `Promise<void>`. In a managed module, write
`await mkdir(path, options)`; in synchronous code use
`mkdirSync(path, options)`. Calling `mkdir` outside a managed turn throws
`FS_ASYNC_OUTSIDE_TURN` before any I/O.

### Breaking change: remove

`remove(path, options?)` used to return `void` synchronously. It now runs on a
worker and returns `Promise<void>`. In a managed module, write
`await remove(path, options)`; in synchronous code use
`removeSync(path, options)`. Calling `remove` outside a managed turn throws
`FS_ASYNC_OUTSIDE_TURN` before any I/O.

```js
import { mkdir, readDir, readFile, remove, stat, writeFile } from "neo:fs";

const bytes = await readFile("input.bin");
await writeFile("output.bin", bytes);
const info = await stat("output.bin");
console.log(info.size);
for (const entry of await readDir(".")) console.log(entry.name);
await mkdir("cache", { recursive: true });
await remove("cache", { recursive: true });
```

Each managed turn accepts at most four outstanding filesystem operations,
including undelivered results. At most two run on workers. Each outstanding
read charges `MAX_READ_BYTES` plus the encoded byte length of its resolved
path; each write charges its copied byte length plus that path length; each
metadata read charges a small result bound plus its resolved path length; a
directory read charges a bounded listing result plus that path length; a
directory creation or deletion charges its resolved path length. The shared
budget is `4 * (max(MAX_READ_BYTES, MAX_WRITE_BYTES) + MAX_PATH_BYTES)`.
Exceeding the count or byte budget rejects the operation with `FS_LIMIT`.
This accounts for outstanding worker work, not total memory use. A large
asynchronous write can briefly hold the caller's input, the JavaScript
snapshot, and the worker-owned copy. The budget does not cover buffers that
JavaScript retains after delivery.

The asynchronous forms capture the working directory at the call. A read
Promise resolves to a new `Uint8Array`; a write Promise resolves to
`undefined` after the file operation completes; a metadata Promise resolves
to a `FileInfo` snapshot; a directory listing Promise resolves to a sorted
`DirEntry[]`; a directory creation or deletion Promise resolves to
`undefined`. Concurrent operations have no guaranteed completion order. There
is no caller cancellation API. On a failed
turn with outstanding work, the driver cancels queued work and stops running
reads between chunks and directory listings between entries where possible.
A started listing checks cancellation between entries, before its explicit
no-follow metadata fallback, and before sorting; it rejects with
`FS_CANCELLED` rather than resolving a partial array. Pending operations
reject with `FS_CANCELLED`. Teardown cannot interrupt an OS call that has
already started: a write may still create, truncate, or fully write the file
after a failed turn or abandoned Promise; a started `mkdir` may still create
some or all directories, including a partial recursive tree, and cancellation
never removes a directory; a started `remove` may still delete some or all
entries, including part of a recursive tree, and cancellation restores
nothing. Cancellation does not roll back those side effects. An OS call can
delay teardown. See
[Managed module evaluation](/runtime/loading/#managed-module-evaluation) for
runtime failure and quiescence rules.

## Files and sizes

`readFile`, `readFileSync`, `readText`, `writeFile`, `writeFileSync`, and `writeText` require a regular file.
An open may occur before a non-regular object is rejected. An identified
directory reports `FS_IS_DIRECTORY`; other identified non-regular objects
report `FS_TYPE`. If opening fails before the type can be identified, the
underlying open error determines the code. `stat`, `statSync`, `lstat`, and
`lstatSync` inspect any path the operating system exposes. `readDir` and
`readDirSync` require a directory.

`writeFile`, `writeFileSync`, and `writeText` do not create parent
directories. They verify the opened descriptor is a regular file before
truncating that same descriptor; by default an existing file is truncated
before the new bytes are written. On Unix, opens use `O_NONBLOCK` to avoid
waiting for a FIFO peer and `O_NOCTTY` to avoid acquiring a controlling
terminal. Opening a device can still have other side effects, and there is no
latency guarantee. Writes are not atomic: an I/O failure can leave the
destination empty or partially written. There is no temporary file, rename
step, rollback, cleanup, or lock. No write is durable on return: success does
not mean the data reached stable storage. Concurrent writes have no ordering
guarantee; final contents depend on their interleaving at the OS and
filesystem level.
Writing follows a symlink at the final path component. A dangling symlink is
followed and its target is created by default; `{overwrite: false}` refuses the
path if the link itself exists.

`FileInfo.size` is a decimal string of the exact `u64` byte size, never a
JavaScript number. A value above `Number.MAX_SAFE_INTEGER` therefore keeps
every digit. `mtimeMs` is whole milliseconds since the Unix epoch as a decimal
string rounded toward zero (so a pre-epoch timestamp less than one millisecond
from 1970 is `"0"`), or `null` when the host cannot report it. For a symlink
reported by `lstat` or `lstatSync`, `size` is the host's symlink size, not the
target size.

## Symlinks

The rules are explicit:

- `stat`, `statSync`, `readFile`, `readFileSync`, `readText`, `writeFile`,
  `writeFileSync`, `writeText`, `exists`, `readDir`, and `readDirSync` follow a
  symlink at the final path component.
- `lstat` and `lstatSync` do not follow it and report `isSymlink: true`.
- `readDir` and `readDirSync` follow a symlink to a directory but do not follow
  entries; an entry that is a link is reported with `isSymlink: true`.
- `remove` and `removeSync` do not follow a symlink at the target; they remove
  the link except for Windows directory symlinks (see below). Recursive
  deletion also does not follow symlinks inside the tree. An intermediate
  directory symlink in the path is resolved by the OS, so deletion can land
  inside the link target, outside the apparent tree.
- `mkdir` follows the operating system's rules for existing components.
- `rename` acts on the final entries without following either final symlink.
- `copyFile` follows a final source symlink to a regular file. A final
  destination symlink, even a dangling one, is an existing entry and is not
  followed.

A dangling symlink is reported by `lstat` and `lstatSync` but `exists` follows
it and returns `false`. On Windows, `remove` rejects directory symlinks named directly as
targets with `FS_UNSUPPORTED`, even if dangling. Windows reparse-point and
junction behavior has not been verified by running on Windows.

## Listing

`readDir` lists on a worker; `readDirSync` blocks. Both are non-recursive and
return entries sorted by name in UTF-8 byte order, which is also Unicode
code-point order. A name that is not valid UTF-8 rejects `readDir` and throws
`readDirSync` with `FS_ENCODING`; the module never substitutes a lossy name.
The listing is bounded before it can grow without limit; exceeding a bound
likewise reports `FS_LIMIT`.
The listing is not an atomic snapshot: entries can change while the host reads
the directory, and type flags may reflect a different instant from the names.
An asynchronous listing that hits a cancellation check rejects with
`FS_CANCELLED` instead of returning the entries gathered so far.

## Directories

`mkdir` creates on a worker; `mkdirSync` blocks. Without `recursive`, an
existing path reports `FS_ALREADY_EXISTS` and a missing parent reports
`FS_NOT_FOUND`. With `recursive: true`, missing parents are created and an
existing directory is accepted. No `mode` option is provided. An OS or
network failure can leave a partially created tree; there is no rollback,
cleanup, or removal. A started asynchronous creation cannot be interrupted by
teardown and may still create some or all directories, including a partial
recursive tree, after a failed turn.

On Unix, a non-recursive `mkdir` refuses a final symlink, including a dangling
one, with `FS_ALREADY_EXISTS` and never follows it. Recursive `mkdir` accepts a
final symlink to an existing directory but reports `FS_ALREADY_EXISTS` for a
dangling final symlink. Both modes follow intermediate symlinks, so creation
can land outside the apparent parent tree. A non-recursive path below a
dangling intermediate symlink reports `FS_NOT_FOUND`; the recursive form
reports `FS_ALREADY_EXISTS`. An intermediate component that resolves to a
non-directory reports `FS_NOT_DIRECTORY`. A final path occupied by an existing
regular file reports `FS_ALREADY_EXISTS` in both modes. These rules apply to
`mkdir` and `mkdirSync`.

These rules are path-based and are not containment. A component can be
replaced between path resolution and the OS call, redirecting creation
elsewhere. Windows reparse-point, junction, and symbolic-link semantics are
platform-specific; this build does not assert them.

## Deletion

`remove` deletes on a worker; `removeSync` blocks. Both remove a file,
symlink, or empty directory. A non-empty directory reports `FS_NOT_EMPTY`.
`remove(path, {recursive: true})` removes a tree. Recursive deletion has no
count limit and no undo; choose the path carefully. It may remove some entries
before an OS error stops the traversal; there is no rollback. A started
asynchronous deletion cannot be interrupted by teardown and may still remove
some or all entries after a failed turn; cancellation restores nothing.
Parsed roots (including Windows UNC and verbatim UNC share roots without a
trailing separator), paths ending in a separator, and paths ending in a `.` or
`..` component are rejected with `FS_PATH` before cwd resolution, without
canonicalizing. Remove a specific child instead.

A final symlink is removed as a link and is never followed. An intermediate
directory symlink in the path is resolved by the OS, so both forms can delete
inside the link target, outside the apparent parent tree. The path checks are
not containment: an intermediate component can be replaced between validation
and the OS call (time-of-check/time-of-use), and the runtime neither
canonicalizes the target nor reverses a deletion.

Options are strict: only `overwrite` or `recursive` (as applicable) is
accepted, and each option must be an own, enumerable data property. Accessors,
non-enumerable properties, and unknown keys throw `TypeError`; inherited
properties are ignored. A Proxy may run traps during reflective validation.
The runtime-local absolute path is snapshotted before this reflection, so a
trap that changes `neo:process.chdir()` cannot redirect that call. Proxy traps
can still run arbitrary script code and reenter the filesystem API.

## Rename

`rename(source, destination)` validates both paths before I/O and resolves
relative paths against the same runtime-local working directory. It takes no
options and does not create destination parents. Parsed roots (including
Windows UNC share roots), trailing separators, final `.` or `..` components,
and Windows drive-relative paths throw `FS_PATH`. The runtime does not
canonicalize either operand on disk. Joining a relative path to a Windows
verbatim working directory can normalize intermediate components lexically;
the OS resolves the resulting paths.

An existing destination may be replaced under the host OS rules; replacement
is not guaranteed to work the same way on different platforms or filesystems.
A rejected replacement of a non-empty directory can report
`FS_ALREADY_EXISTS` or `FS_NOT_EMPTY`, depending on the OS error.
The runtime does not pre-delete the destination or fall back to copy/delete.
An OS cross-device error throws `FS_CROSS_DEVICE`. There are no multi-step
changes by the runtime to undo, and there is no rollback or guarantee of the
outcome after an OS or network filesystem failure.

## Copy

`copyFile(source, destination)` is synchronous and takes two primitive path
strings and no options. It checks both paths before I/O and resolves them
against the same runtime working directory. As for `rename`, roots, trailing
separators, final `.` or `..` components, and Windows drive-relative paths
throw `FS_PATH`. On Windows, copy accepts only ordinary local-drive path
syntax (including verbatim drive paths); it rejects UNC and device
namespaces, reserved device names, alternate data streams, drive-less rooted
paths, and ambiguous trailing dots or spaces in components. These syntax
checks do not guarantee that the underlying storage is local.

Unlike the reference copy API, `copyFile` is **no-clobber** and copies only
bytes, not permissions or other metadata. Once the source has been opened
and checked, the OS creates the destination exclusively. An existing
destination (including the source itself, a hardlink, or a dangling
symlink) is not modified; an OS existing-entry refusal is `FS_ALREADY_EXISTS`.
The function does not create parents. New files use mode `0600` subject to
umask on Unix and the OS default security on Windows. Copy can span
filesystems; it does not use a cross-device rename or a copy/delete fallback.

```js
import { copyFile, FsError } from "neo:fs";

try {
	copyFile("README.md", "README-copy.md");
} catch (error) {
	if (!(error instanceof FsError) || error.code !== "FS_ALREADY_EXISTS") throw error;
	console.log("The backup already exists; nothing was overwritten.");
}
```

The opened source must be a regular file and is checked against
`MAX_READ_BYTES` before destination creation. Copy uses an 8 KiB buffer
and reads at most one byte beyond the limit to detect growth; that byte is
never written. The destination is checked through its opened descriptor too.
On Unix, opens use `O_NONBLOCK` to avoid waiting for FIFO peers and `O_NOCTTY`
to avoid acquiring a controlling terminal. Other device-open side effects
are possible before rejection. Disk and network-filesystem I/O have no
deadline.

The new destination is visible while it is being written. An error after
creation, including source growth past the limit, can leave an empty or
partial file. There is no lock, temporary path, rename, automatic unlink,
or durability guarantee. Inspect the destination before retrying. A parent
directory or symlink can change before either open; once opened, I/O uses
the handles, but their paths may no longer name the same files. Concurrent
source writes can produce a mixed copy. No-clobber relies on the filesystem
honoring exclusive creation; network or unusual filesystems can weaken
that guarantee or make the result of an error uncertain.

## Limits

| Constant               |   Value | Applies to                                              |
| ---------------------- | ------: | ------------------------------------------------------- |
| `MAX_PATH_BYTES`       |  32 KiB | Every path argument.                                    |
| `MAX_READ_BYTES`       |  64 MiB | `readText`, `readFile`, `readFileSync`, and `copyFile`. |
| `MAX_WRITE_BYTES`      |  64 MiB | `writeText`, `writeFile`, and `writeFileSync`.          |
| `MAX_LIST_ENTRIES`     | 100,000 | Entries returned by `readDir` or `readDirSync`.         |
| `MAX_LIST_NAME_BYTES`  |   1 KiB | One directory-entry name.                               |
| `MAX_LIST_TOTAL_BYTES` |  16 MiB | Sum of all names a listing returns.                     |

Reads inspect the opened descriptor and its metadata before allocating, then
read in bounded chunks with fallible reservation. A separate one-byte probe
detects growth past the limit. A binary write copies the `Uint8Array` view's
current bytes (including for a resizable `ArrayBuffer`) before reflecting on
options. The asynchronous form copies that snapshot into worker-owned memory
before dispatching it. `SharedArrayBuffer` views are unsupported. `writeText` checks the
primitive string's code-unit length before exporting it from QuickJS as UTF-16.
The UTF-16 export itself can allocate up to 128 MiB of string data. If QuickJS
flattens a narrow string rope, the flattened string can use another 64 MiB
and coexist with the export: up to 192 MiB of additional engine string data
in that case. These figures exclude the original input, overhead, and the
separate Rust UTF-8 snapshot; they are not a total memory bound. The native
scan rejects unpaired surrogates before applying the exact UTF-8 byte limit;
only then does it reserve space for the UTF-8 snapshot. The limits are
exported as constants on both the global `fs` object and the module.

## Errors

Filesystem failures are `FsError` values with a stable `code` and a fixed
message that never contains the supplied path, an OS error string, or file
contents. Argument type errors are `TypeError` (thrown by synchronous
operations, used to reject asynchronous ones), and exceptions thrown by
option proxy traps can propagate or reject likewise.

| Code                    | Meaning                                                                                                                                                                                                              |
| ----------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `FS_PATH`               | Invalid or unsupported path syntax, including empty paths, embedded NULs, and on Windows, drive-relative paths; also a parsed root, trailing separator, or final `.` or `..` for `remove`, `rename`, and `copyFile`. |
| `FS_NOT_FOUND`          | The path does not exist (or a parent does not).                                                                                                                                                                      |
| `FS_NOT_DIRECTORY`      | An intermediate path component, or a listed path, is not a directory.                                                                                                                                                |
| `FS_IS_DIRECTORY`       | A file operation was given a directory.                                                                                                                                                                              |
| `FS_ALREADY_EXISTS`     | The OS reported an existing path, e.g. for `copyFile`, non-recursive `mkdir`, a write with `{overwrite: false}`, or a `rename` rejected because the destination directory is non-empty.                              |
| `FS_NOT_EMPTY`          | The OS required an empty directory, e.g. for non-recursive `remove` or some `rename` replacements.                                                                                                                   |
| `FS_CROSS_DEVICE`       | The OS reported that the rename crossed filesystems.                                                                                                                                                                 |
| `FS_DENIED`             | The operating system denied access.                                                                                                                                                                                  |
| `FS_LIMIT`              | A path, read, write, or listing exceeded a documented bound, or a filesystem quota, storage, or fallible allocation resource was unavailable.                                                                        |
| `FS_ENCODING`           | Text or a directory-entry name is not valid UTF-8.                                                                                                                                                                   |
| `FS_DETACHED`           | A `Uint8Array` buffer was detached.                                                                                                                                                                                  |
| `FS_OUT_OF_BOUNDS`      | A `Uint8Array` view is out of bounds after its resizable buffer shrank.                                                                                                                                              |
| `FS_CWD`                | A relative operation needs an unavailable runtime working directory.                                                                                                                                                 |
| `FS_TYPE`               | The path is not a regular file.                                                                                                                                                                                      |
| `FS_UNSUPPORTED`        | The operation or input is unsupported, e.g. direct removal of a Windows directory symlink or writing from a `SharedArrayBuffer` view.                                                                                |
| `FS_ASYNC_OUTSIDE_TURN` | An asynchronous filesystem operation was called outside managed module evaluation.                                                                                                                                   |
| `FS_CANCELLED`          | An outstanding asynchronous filesystem operation was cancelled after module evaluation failed.                                                                                                                       |
| `FS_IO`                 | Any other filesystem failure.                                                                                                                                                                                        |

## Authority and races

This is not a sandbox. A script with `neo:fs` has the host user's filesystem
authority. Path confinement, a virtual root, and copy-on-write are not
provided. The module does not start a shell, a child process, or a network
request. Operating-system filesystem activity can still produce network
traffic when a path refers to a network mount or a special filesystem.

`exists`, `stat`, `statSync`, and `lstatSync` are snapshots and are
time-of-check/time-of-use race prone.
A path can be replaced between resolution and open, delete, or rename, so no operation
is atomic with a prior check. Reads and writes validate the opened descriptor.
After some failed opens, a separate metadata lookup may refine the error code;
this lookup is not used to authorize reading, writing, or truncating. Deletion
remains pathname based and is not race-free. Do not use `exists` to decide whether a later
operation is safe; perform the operation and handle `FsError` instead.
`readDir`, `readDirSync`, and recursive deletion are non-atomic; recursive
deletion may be partial on failure. Writes can leave partial files, and no file locking is
performed.

## Migration from the original `neo:fs`

The original module exported only `readText` and `writeText` and read or wrote
without a bound. Both signatures are unchanged, and ordinary successful calls
behave the same. The differences are deliberate:

- `readText` now rejects a file above `MAX_READ_BYTES` with `FS_LIMIT` and
  checks that the opened descriptor is a regular file instead of attempting
  an unbounded read.
- `writeText` now rejects text above `MAX_WRITE_BYTES` with `FS_LIMIT` and
  checks that the opened descriptor is a regular file before truncating.
- Failures are now `FsError` with a stable `code`; messages no longer include
  OS error text. Invalid UTF-8 text reads report `FS_ENCODING`.
- The `fs` object and the `neo:fs` API object have a null prototype and own
  data properties. They are still mutable, but replacement of a global
  property does not change an already-imported function.

Declarations: [`neo:fs`](/api/modules/fs/).
