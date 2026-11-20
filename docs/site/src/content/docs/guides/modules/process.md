---
title: "Streams, working directory, and elevation"
description: "Standard streams, the runtime-local working directory, and host process elevation status."
---

With `--allow-run` or `--allow-stdio`, `neo:process` exports `stdin`,
`stdout`, `stderr`, `cwd()`, `chdir()`, `isElevated()`, `ChangeDirectoryError`,
and `ElevationError`; the same values are also available on its default export.
An embedder's explicit module allowlist must also include `neo:process`.
`stdout` and `stderr`
accept a string (UTF-8) or a `Uint8Array` and write binary data synchronously
without an intermediate buffer. Each write is flushed and returns the number of bytes
written. `stdin.read` and `stdin.readSync` write into a caller-supplied
`Uint8Array`, return the byte count, and return `null` at end of file; a
zero-length buffer returns `0`. Reads are available with `--allow-stdio` or
`--allow-run` and can block until input or end of file; there is no timeout or
nonblocking mode.
The write, read, and `isTerm` aliases are all synchronous; `close()` is a no-op
because the streams belong to the host process.

```js
import { stdin, stdout, stderr, cwd, chdir } from "neo:process";

const buffer = new Uint8Array(4096);
let count;
while ((count = stdin.readSync(buffer)) !== null) {
	stdout.write(buffer.subarray(0, count));
}
stderr.write("done\n");
console.log(cwd());
chdir("subdirectory");
```

The working directory is owned by the runtime, not the host process. `chdir()`
never calls the host's global directory change, so two `NeoJs` instances in one
embedder keep independent values and unrelated threads are unaffected. It starts
as the host process working directory when the runtime is constructed.
The [host path dialect](/guides/modules/path/#runtime-working-directory) can
use this directory as an implicit resolution base when the matching
permission and module allowlist entry grant access.
Relative paths used by `neo:fs`, `neo:hash`'s file digest, `neo:fetch`'s
destination, relative module entry names, and `neo:exec` children are resolved
against it. A module's relative imports use the absolute base captured when
that module was declared, so they do not move with a later `chdir`; only a
module with no captured base reports the unavailable directory. Absolute paths
are used unchanged. On Windows, drive-relative paths such as `C:child` are
rejected because they depend on the host's per-drive cwd: `chdir()` reports
`PROCESS_PATH`, while `neo:fs` reports `FS_PATH`. Verbatim path behavior is
unverified. `chdir()`
canonicalizes the target, requires an existing directory with a Unicode path,
and leaves the previous value untouched on failure. The check reads directory
metadata only, never file contents. A failed change reports
`ChangeDirectoryError` with code `PROCESS_CHDIR`, or with `PROCESS_CWD` when a
relative change is attempted while the runtime directory is unavailable. A
stream write or read failure reports `PROCESS_IO`. `cwd()` reports `PROCESS_CWD`
if the host directory is unavailable or not valid Unicode; a non-Unicode initial
directory is treated as unavailable. While the directory is unavailable,
relative operations that need it (`neo:fs`, `neo:hash` file digest,
`neo:fetch` destination, a relative module entry name, a relative import from a
module with no captured base, or relative `chdir`) fail explicitly instead of
using the host's mutable working directory. An absolute module identity captures
an absolute base, so its relative imports can still succeed. Absolute paths
continue where the operation is otherwise safe. `neo:exec` fails with
`EXEC_CWD` until an absolute `chdir` establishes a directory. Error
messages never include the supplied path or input bytes. This is path
coordination, not a sandbox: relative paths still have all the authority the
granted permissions carry.

## Process elevation

`isElevated()` synchronously takes a fresh snapshot. It reports whether the
host process has effective UID 0 on Unix or an elevated primary process token
on Windows (`TokenElevation`). On Windows it checks the token the process is
running with, not whether the account belongs to Administrators or could approve
a UAC prompt. The check does not grant or change privileges, launch a child
process, or access the registry. It does not predict whether a particular
operation will succeed.

If the token query fails, `isElevated()` throws `ElevationError` with code
`PROCESS_ELEVATION`. On unsupported platforms it throws the same error type
with code `PROCESS_ELEVATION_UNSUPPORTED`.

```js
import { ElevationError, isElevated } from "neo:process";

try {
	if (isElevated()) {
		console.log("The host process reports elevated status.");
	} else {
		console.log("The host process reports non-elevated status.");
	}
} catch (error) {
	if (!(error instanceof ElevationError)) throw error;
	console.error(`Could not determine elevation: ${error.code}`);
}
```

Declarations: [`neo:process`](/api/modules/process/).
