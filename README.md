# neo-js

neo-js is an early Rust library and command-line host for QuickJS. It runs
JavaScript modules and exposes optional host functions for automation.

## Run a module

Install Rust 1.87 or later, then run:

```sh
cargo run -p neo-js-cli -- script.js
cargo run -p neo-js-cli -- script.ts
cargo run -p neo-js-cli -- --allow-env --allow-fs script.js
cargo run -p neo-js-cli -- --allow-run script.js
cargo run -p neo-js-cli -- --allow-stdio script.js
cargo run -p neo-js-cli -- --allow-registry script.js
cargo run -p neo-js-cli -- --allow-net --allow-fs script.js
cargo run -p neo-js-cli -- --third-party-notices
```

The CLI reads the entry script regardless of `--allow-fs`. That flag grants
scripts the host user's filesystem authority, including reading, writing,
deleting, renaming, and replacing files or directories as permitted by the
OS. It is not a sandbox; grant it only to scripts you trust. `--allow-env`
permits the script to read any environment variable. Neither capability is
enabled by default.

`--allow-registry` permits the synchronous `neo:winregistry` module. On
Windows this is full read, write, and delete authority under the current
process token, not a sandbox; a script can change any key or value the user can,
including keys that run programs at sign-in. Grant it only to scripts you
trust. On a non-Windows host the module can be imported with this permission,
but `isAvailable()` is `false` and every operation reports that registry access
is unsupported. Without the flag the module and all of its functions are
absent.

`--allow-run` permits the synchronous `neo:exec` module, including its
owned-child `spawn` handles. The child process runs with the current user's
full host authority: it can read files, read the environment, and use the
network even when `--allow-fs` and `--allow-env` are absent. Enable this flag
only for scripts you trust.

`--allow-stdio` permits scripts to read the host's standard input through
`neo:process`. That consumes input the embedder or an interactive user may be
using, so it is an explicit opt-in; reads are synchronous and can block until
input or end of file. `--allow-run` also grants this access, because it already
permits arbitrary child programs under the host user's authority. `--allow-stdio`
is the narrower choice when only standard I/O is needed. `--allow-run` or
`--allow-stdio` makes `neo:process` importable for its stdout and stderr writers
and its runtime-local working directory; without one of them the module is
absent.

`--allow-net` permits the synchronous `neo:fetch` module. It grants broad
outbound HTTP and HTTPS authority under the current user's network access,
including loopback, link-local, private, and cloud-metadata addresses. It is
not a sandbox, allowlist, or SSRF defense; a redirect may change the host,
port, or scheme, including an HTTPS-to-HTTP downgrade. Downloading a file also
requires `--allow-fs`, because the module writes the destination; without it
`download` throws `FETCH_PERMISSION` before any file or network activity. Enable `--allow-net` only for scripts you trust with the
user's network reachability.

## Embed neo-js

`Permissions` controls host operations, while `RuntimeOptions` can restrict
which `neo:` modules the runtime constructs. These controls are independent.
An explicit module allowlist does not remove authority granted by a permission
through a different path:

- `neo:hash.digestFile` reads files with filesystem permission even if `neo:fs`
  is not allowed.
- `neo:fetch.download` writes files when filesystem and network permissions
  are granted even if `neo:fs` is not allowed.
- Filesystem permission enables relative module imports independently of the
  `neo:fs` module.
- Process permission lets `neo:exec` launch programs with the host user's
  authority, regardless of other permission flags or module selections.
- `console` is present in every runtime.

neo-js is not a sandbox. For untrusted scripts, isolate the process with
operating-system controls.

```rust
use neo_js::{NeoJs, Permissions, RuntimeModule, RuntimeOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = RuntimeOptions::new().modules([
        RuntimeModule::Hash,
        RuntimeModule::Fetch,
    ]);
    let runtime = NeoJs::new_with_options(
        Permissions {
            filesystem: true,
            network: true,
            ..Permissions::default()
        },
        options,
    )?;
    runtime.run("console.log('host runtime ready')")?;
    Ok(())
}
```

The example omits `neo:fs` but grants filesystem permission for the listed
operations. `RuntimeOptions::default()` leaves the module set unrestricted,
preserving `NeoJs::new` behavior. See the
[RuntimeOptions reference](https://docs.rs/neo-js/latest/neo_js/struct.RuntimeOptions.html)
and [Permissions reference](https://docs.rs/neo-js/latest/neo_js/struct.Permissions.html)
for the Rust API.

`--third-party-notices` prints the third-party copyright and permission notices
embedded in neo-js and exits. It reads no script and needs no permission flag.
It is a standalone mode; combining it with a script is an error. The text
currently covers source adapted for `neo:path`; it is not a complete
dependency-license inventory. Embedders can retrieve the same text without
constructing a runtime by calling `neo_js::third_party_notices()`. The notice
file is part of the crate source and is included in the Cargo package.

The `os` global and `neo:os` need no permission flag, but they disclose basic
host operating-system metadata and are present only when the `neo:os` module is
admitted by the runtime allowlist. `platform` and `arch` are Rust's compilation
target constants, not a separate host-detection result. On Linux, `version` is
the distribution's `VERSION_ID` (not the kernel version), while `codename` is
`VERSION_CODENAME` with `UBUNTU_CODENAME` as a fallback and `variant` is
`VARIANT_ID` with `VARIANT` as a fallback. These values are read from
`/etc/os-release`; `/usr/lib/os-release` is consulted only if `/etc/os-release`
is absent, never merged with it. On Windows, the fixed `CurrentVersion`
registry key supplies best-effort `DisplayVersion` for `version` and
`EditionID` for `variant`. Without a usable display version, a decimal
`CurrentBuildNumber` is reported as `build <number>`. These registry probes
use the process-default WOW64 view (no view selector); a 32-bit process can
therefore see different values or find them unavailable compared with a
64-bit process. `codename` is
`undefined`. On macOS, `version` is the product version if the
`kern.osproductversion` query succeeds; `codename` and `variant` are
`undefined`. On other platforms all three optional fields are `undefined`.
Unavailable, malformed, or oversized values are not exposed. These bounded,
read-only probes do not require `--allow-fs` or `--allow-registry` and do not
grant arbitrary filesystem or registry access or launch subprocesses.

The global object's properties remain writable for compatibility. In contrast,
the named `neo:os` exports are generated from a private snapshot taken when the
runtime is constructed, so later global mutations do not alter them:

```js
import { platform, arch, version, codename, variant } from "neo:os";
console.log(platform, arch, version, codename, variant);
```

`console.log` and `console.error` accept strings. With the relevant permission, scripts can
use `env.get(name)`, `env.has(name)`, and the `fs` API described under
[Filesystem](#filesystem). `env.get` returns `undefined` for an absent or
non-Unicode value; `env.has` tests presence. The declarations in
`types/neo-js.d.ts` describe these globals.

The CLI evaluates its entry file as an ECMAScript module. `neo:os` exports
`platform`, `arch`, `version`, `codename`, and `variant`. Its three optional
metadata exports use the same meanings and availability rules as the global;
named exports use the runtime snapshot, independent of global object mutation.
`neo:semver` exports `valid`, `compare`, `parse`,
`format`, `increment`, `satisfies`, `validRange`, and `SemVerError`; it parses
exact decimal components and uses Cargo range semantics. `neo:env` exports the
corresponding global functions when `--allow-env` is granted. `neo:fs` exports
the same API as the global `fs` object when `--allow-fs` and the runtime
allowlist admit it. `neo:semver`, `neo:dotenv`, `neo:toml`,
`neo:yaml`, and `neo:scheduler` need no permission flag. Relative `.js`,
`.mjs`, `.ts`, and `.mts` imports also require `--allow-fs`.
Bare imports are not supported.

The CLI canonicalizes the entry path and reads that file even without
`--allow-fs`. With permission, relative dependencies are canonicalized before
loading: aliases and cycles use the canonical file identity, and symlinks
and `..` follow filesystem semantics. Missing or inaccessible dependencies are
errors. Canonical paths must be valid UTF-8. Windows behavior is unverified.
The requested spelling, not the canonical target's extension, selects the
format: `.ts` and `.mts` are stripped, `.js` and `.mjs` are JavaScript, and
`.tsx`, `.cts`, `.d.ts`, and `.d.mts` are rejected even through symlinks.
Within one runtime, requesting the same canonical file using different
extensions among `.js`, `.mjs`, `.ts`, and `.mts` is a format conflict,
including after it has been loaded or when it aliases the entry. Same-format
aliases share one module. A later dynamic-import conflict cannot undo earlier
side effects. Canonical paths do not unify hard links.
Load, parse, and resolution failures release the failing module's format
reservation; successfully declared dependencies remain reserved. Once
declared, a module's format stays reserved even if linking or evaluation
fails. If cyclic resolution fails, the runtime disables further module loads
and entry evaluations: QuickJS can otherwise retain a dangling reference to a
freed module. Create a new runtime to recover; already executed side effects
are not undone.

`neo:hash` provides `digest(algorithm, bytes)` for a `Uint8Array` and, with
`--allow-fs`, `digestFile(algorithm, path)` for a file. Both return lowercase
hexadecimal strings. Supported algorithms are `sha224`, `sha256`, `sha384`,
`sha512`, `sha3-224`, `sha3-256`, `sha3-384`, `sha3-512`, `blake2b-512`,
`blake2s-256`, and `blake3`. File hashing reads in bounded chunks.

`neo:crypto` provides synchronous authenticated encryption with AES-256-GCM.
It needs no permission flag and grants no host capability. Its native wrapper
and callbacks are private to the module: there are no `__neoCrypto*` globals,
but this is not isolation from other code in the same realm that can access
the caller’s values.

```js
import { decrypt, encrypt, generateKey } from "neo:crypto";

const key = generateKey();
const plaintext = Uint8Array.from([0x68, 0x65, 0x6c, 0x6c, 0x6f]);
const context = Uint8Array.from([0x61, 0x2d, 0x64]);
const envelope = encrypt(key, plaintext, context);
const returned = decrypt(key, envelope, context);
```

The key is exactly 32 bytes. Plaintext, associated data, and the returned
envelope are `Uint8Array`s; bytes are never interpreted as text. The runtime
has no `TextEncoder` or `TextDecoder`, so callers encode text themselves.
`associatedData` is optional, authenticated but not encrypted, and must be
supplied unchanged to `decrypt`.

Every `encrypt` call draws a fresh 96-bit nonce from the operating system
CSPRNG through `getrandom`. The envelope is:

| offset | bytes | field                        |
| ------ | ----- | ---------------------------- |
| 0      | 4     | magic, ASCII `NEO1`          |
| 4      | 1     | version, `1`                 |
| 5      | 1     | algorithm, `1` (AES-256-GCM) |
| 6      | 12    | nonce                        |
| 18     | n     | ciphertext                   |
| 18+n   | 16    | authentication tag           |

The first 18 bytes and the caller's associated data are authenticated as
AES-GCM associated data. Changing any envelope byte, using another key, or
supplying different associated data makes `decrypt` throw; plaintext is not
returned from a failed check. An unknown magic, version, or algorithm is
reported as malformed.

Nonce collision risk grows with the number of messages under one key: a random
96-bit nonce reaches about 2^-32 collision probability at roughly 2^32
encryptions. Retire a key well before that. The caller cannot choose a nonce,
so the module removes the common reuse mistake, but key lifetime remains the
caller's responsibility.

`generateKey()` returns 32 CSPRNG bytes. Keep the key secret, use a distinct
key per purpose, and protect where it is stored. The module does not store,
derive, wrap, rotate, or persist keys. It does not promise zeroization: key and
plaintext bytes may remain in Rust buffers and the JavaScript heap until
reclaimed. It is not a sandbox.

The neo-js module limits plaintext and caller associated data to 8 MiB each,
and envelopes to 8 MiB + 34 bytes. These are neo-js limits, not intrinsic
AES-GCM limits. Native code checks these sizes before copying inputs or
allocating input buffers. The core uses checked arithmetic and fallible
reservations; a refused reservation is reported as `NEO_CRYPTO_LIMIT`. That
mapping is best effort: an allocation failure inside QuickJS, the Rust
allocator, or the JavaScript heap can still abort the process or raise an
error the module cannot catch, so recovery from out-of-memory is not
guaranteed.

An argument that is not a `Uint8Array` throws `TypeError`. Other failures are
`Error` objects with a stable `code`: `NEO_CRYPTO_KEY` (key length),
`NEO_CRYPTO_FORMAT` (truncated or unknown envelope), `NEO_CRYPTO_AUTH` (wrong
key, wrong associated data, or tampering), `NEO_CRYPTO_RANDOM` (OS CSPRNG
failure), `NEO_CRYPTO_LIMIT` (an input above a documented size bound, or a
refused allocation), or `NEO_CRYPTO_DETACHED` (detached or out-of-bounds
view). Messages never include key, plaintext, or ciphertext bytes.

The envelope is specific to neo-js. The AES-256-GCM primitive is standard and
is checked against published NIST vectors; the envelope is also checked
against a fixture produced independently with Node/OpenSSL. Another tool must
reproduce the envelope and associated-data construction to interoperate. For
standard age file encryption, use `neo:age` below.

`neo:age` provides synchronous age file encryption for X25519 recipients. It
needs no permission flag and grants no host capability. Its native wrapper and
callbacks are private to the module: there are no `__neoAge*` globals, but this
is not isolation from other code in the same realm that can access the caller’s
values.

```js
import { decrypt, encrypt, generateIdentity } from "neo:age";

const { identity, recipient } = generateIdentity();
const plaintext = Uint8Array.from([0x68, 0x65, 0x6c, 0x6c, 0x6f]);
const ciphertext = encrypt(recipient, plaintext);
const returned = decrypt(identity, ciphertext);
```

`generateIdentity()` returns `{ identity, recipient }` as standard Bech32 text:
`AGE-SECRET-KEY-1...` and `age1...`. `identityToRecipient(identity)` derives the
recipient from an existing identity. Keep the identity secret; the module does
not store, wrap, rotate, or persist it. `generateIdentity` and `encrypt` ask the
vetted Rust `age` crate for randomness, which draws from the operating system
CSPRNG. age 0.11 exposes no fallible or injectable RNG path, so the module
contains panics from the age call and reports them as `NEO_AGE_RANDOM`. The
known panic cause is OS RNG failure; a `panic = "abort"` build cannot contain
it and aborts instead.

`encrypt(recipient, plaintext)` returns a standard binary age v1 file encrypted
to one X25519 recipient, and `decrypt(identity, ciphertext)` accepts standard
binary age v1 files encrypted to an X25519 recipient. Plaintext and ciphertext
are `Uint8Array`s; bytes are never interpreted as text. The runtime has no
`TextEncoder` or `TextDecoder`, so callers encode text themselves. ASCII armor,
passphrase encryption, SSH keys, plugins, and post-quantum recipients are not
exposed. The `age` command-line tool can decrypt this module's output, and this
module decrypts its binary X25519 output.

The test suite decrypts a fixture produced independently with the `age` 1.3.2
command-line tool and checks the module's output is a binary age v1 file. The
reverse direction was also verified during development: `age` 1.3.2 decrypted
this module's output byte for byte. The module does not promise zeroization.
Identity text, plaintext, ciphertext, and intermediate key material may remain
in Rust buffers and the JavaScript heap until reclaimed. No secret is logged or
included in an error message.

The neo-js module limits plaintext to 8 MiB and a complete age file to 9 MiB.
Decryption also stops if the recovered plaintext would exceed 8 MiB. Identity
and recipient text longer than 512 bytes is rejected as malformed; the original
UTF-8 byte length is checked before trimming, and a native UTF-16 length check
keeps any conversion small, so huge padded keys are rejected without being
copied. These are neo-js limits, not intrinsic age limits: a valid external age
file with an unusually large header can be refused. Native code bounds inputs
before copying them, and decryption reads in bounded chunks. Core functions
use checked arithmetic, a bounded writer, and fallible reservations; a refused
reservation is reported as `NEO_AGE_LIMIT`. That mapping is best effort: an
allocation failure inside QuickJS, the Rust allocator, or the JavaScript heap can
still abort the process or raise an error the module cannot catch, so recovery
from out-of-memory is not guaranteed.

An argument that is not a string or `Uint8Array` throws `TypeError`. Other
failures are `Error` objects with a stable `code`: `NEO_AGE_IDENTITY`
(malformed identity text), `NEO_AGE_RECIPIENT` (malformed recipient text),
`NEO_AGE_FORMAT` (truncated, unknown, or unsupported age file),
`NEO_AGE_IDENTITY_MISMATCH` (no supplied identity matched), `NEO_AGE_AUTH`
(a header MAC or payload tag did not verify), `NEO_AGE_LIMIT` (a documented
size bound or a refused allocation), `NEO_AGE_RANDOM` (OS CSPRNG failure or a
contained panic in the age encryption path), `NEO_AGE_DETACHED` (detached or
out-of-bounds view), or `NEO_AGE_INTERNAL` (an unexpected failure from the age
crate). A damaged recipient stanza is indistinguishable from a file encrypted
to a different identity, so both are reported as `NEO_AGE_IDENTITY_MISMATCH`.
Payload tampering and header-MAC failures are `NEO_AGE_AUTH`. The age
implementation is pinned to 0.11; `Cargo.toml` also pins `rust-embed` 8.11.0
and its helpers because later 8.x releases need Rust 1.88 through `sha2`
0.11.

With `--allow-run`, `neo:exec` exports `exec(program, args?, input?)` and
`spawn(program, args?)`. `exec` blocks until the child exits, writes the
optional `input` bytes to the child's stdin, and closes stdin. The result is
`{ code, success, stdout, stderr }`: `code` is the exit code, or `null` when a
signal terminated the child, and `stdout`/`stderr` are `Uint8Array` copies of
the captured bytes. A non-zero exit is returned as data, not thrown.

```js
import { exec } from "neo:exec";

const program = os.platform === "windows" ? "git.exe" : "git";
const result = exec(program, ["status", "--short"]);
if (!result.success) throw new Error(`git exited with ${result.code}`);
console.log(`git printed ${result.stdout.length} bytes`);
```

The program is looked up on the host's program search path and launched
directly, without a shell, so arguments are not expanded or split. The child's
working directory is the runtime's current directory, not the host process
directory; a relative program path is resolved against it. On Windows, the
program must name an explicit `.exe` or `.com` file; extensionless names and
files with other extensions, including `.bat`, `.cmd`, and `.ps1`, are rejected
so Windows cannot dispatch them through `cmd.exe`. Naming `cmd.exe` or
`powershell.exe` explicitly is allowed and uses that shell's normal semantics.
The extension check uses the supplied path and does not resolve reparse points.

`exec` writes input and concurrently captures each output stream, up to 8 MiB
per stream. Exceeding either limit throws `EXEC_IO`; captured output is not
returned on failure. Invalid arguments throw `TypeError`. Other errors have a
stable `code`: `EXEC_SPAWN` (start), `EXEC_WAIT` (wait), `EXEC_IO` (pipe or
output limit), `EXEC_WORKER` (worker setup, panic, or capacity),
`EXEC_UNSUPPORTED` (program policy), or `EXEC_CWD` (the runtime working
directory is unavailable; no slot is reserved and no child is spawned).

### Spawned child handles

`spawn(program, args?)` launches the same kind of direct, shell-free child and
returns an owned handle at once, without waiting for it to exit:

```js
import { spawn } from "neo:exec";

const child = spawn(program, ["--some-flag"]);
const input = new Uint8Array([105, 110, 112, 117, 116]); // "input"
const result = child.communicate(input);
if (!result.success) throw new Error(`child exited with ${result.code}`);
console.log(`child printed ${result.stdout.length} bytes`);
```

The handle has three synchronous methods, and each closes the child's stdin
and performs one terminal call:

- `wait()` closes stdin without writing, then blocks until the direct child
  exits and both captured streams reach end of file.
- `communicate(input?)` writes `input`, closes stdin, then blocks the same way.
- `kill()` asks the direct child to terminate, attempts to reap it, and closes
  stdin. It does not kill descendants and does not wait for a descendant that
  still holds an output pipe; in that case the snapshot has `complete: false`.
  If the child had already exited, its existing status is returned and no
  signal is sent. A successful signal is followed by a blocking reap to obtain
  the exit status; a failed signal never falls through to a blocking wait. If
  the signal or reap cannot be confirmed, `kill()` throws `EXEC_KILL` or
  `EXEC_WAIT` with a message saying cleanup is pending or unknown; the direct
  child may survive, and the supervisor keeps ownership and the slot while it
  polls nonblockingly for the child to exit.

All three return `{ code, success, stdout, stderr, complete }`. `code` is the
exit code, or `null` when a signal terminated the child, and non-zero exits are
data, not exceptions. `complete` is true only when both streams had reached
end of file before the result was produced. A handle answers exactly one
terminal call; the JavaScript wrapper caches the result or error and later
calls return fresh copies of the same bytes, status, and message. In
particular, `kill()` after `wait()` reports the cached exit status rather than
signalling a second time. An error from that terminal call is also cached and
rethrown with the same stable `code` and message.

Each spawned child is owned by a host supervisor thread. In normal operation
it reaps the direct child as soon as it exits, so an idle handle does not leave
a zombie; cleanup that cannot be confirmed is reported as pending or unknown
rather than assumed complete. Stdout and stderr are drained concurrently so a
child cannot block on a full pipe while the script waits on another handle. Each stream buffers at most 8 MiB;
bytes beyond that are read and discarded. `wait()` and `communicate()` report
`EXEC_IO` when a stream exceeded the limit and return no output. Before
reporting an IO, worker, or wait failure, the supervisor makes a nonblocking
attempt to stop the direct child. If termination is confirmed, the error is
delivered after that attempt. If it cannot be confirmed, the message says
cleanup is pending or unknown and that the direct child may survive; a
background supervisor then retains ownership and the slot until it can reap
the child. A failure therefore does not claim that the child, its descendants,
or their effects ceased instantly. `kill()` instead returns whatever was
captured with `complete: false`. A read error is reported as `EXEC_IO`. In
an unexpected supervisor panic, cleanup is unknown: the child may remain
unreaped and its slot may be retained permanently. Startup failures after a
child starts likewise report pending cleanup when termination is unconfirmed.
In addition to the `exec` codes, a handle can report `EXEC_KILL` (the stop request
failed and termination is unconfirmed), `EXEC_STATE` (invalid sequence or
internal state), and `EXEC_HANDLE` (an unknown or already consumed handle;
reachable only through the undocumented host callback).

A handle reserves one process-wide slot. A fully confirmed terminal call
releases it; a terminal call whose cleanup is pending, or a kill that leaves a
descendant holding an output pipe, keeps it reserved until the child is reaped
or the pipe closes. The same limit of eight slots bounds `exec` calls,
outstanding handles, their capture threads, and background cleanup
supervisors; a spawn with no available slot fails with `EXEC_WORKER` before any
child starts. A handle whose JavaScript object becomes unreachable without a
terminal call keeps its child and slot until the runtime is dropped. Dropping the
`NeoJs` runtime aborts every outstanding handle: abort requests are sent to all
handles first, then acknowledgements are collected under one shared 10-second
budget, not 10 seconds per handle. An acknowledgement means the supervisor has
either confirmed the direct child is reaped or attempted a nonblocking stop and
kept ownership for cleanup; it does not roll back side effects. A child that cannot be killed may survive or
remain unreaped, and its slot is retained until it can be reaped.

Normal execution has no timeout. A child or descendant that keeps an output
pipe open can block `wait()` and `communicate()` indefinitely, and
`communicate()` can also block when the child or a descendant never reads
stdin. The runtime is single-threaded: while `wait()` or `communicate()`
blocks, no other JavaScript runs, so `kill()` cannot be called on the same
handle to interrupt it. Use `kill()` to request a stop, but it has no hard deadline. It cannot kill
descendants, and neither `kill()` nor runtime drop promises that they stop.
Captured output is not streamed and is not returned on failure. Child effects
are not rolled back and retries can repeat them. Interactive programs and
streaming are not supported. The same Windows program policy as `exec` applies.
Without `--allow-run`, the module and all of its host callbacks are absent.

For `exec`, a call with no available slot fails with `EXEC_WORKER` before
starting a child. On a fatal error the caller is notified before cleanup
may block; the error reports cleanup as pending or unknown, not successful.
The first fatal event observed by the supervisor is primary; queued worker
errors are examined before polling the child and again after a wait failure.
The supervisor attempts to kill and reap the direct child and joins its workers
in the background. The slot remains reserved until resources are released; if
reaping fails or the supervisor fails unexpectedly, it may remain reserved
permanently. Surviving children or descendants, open pipes, threads, and other
resources may persist indefinitely, eventually exhausting capacity. This is a
bounded number of supervisors and workers, not a bound on memory used by the
caller or processes spawned by the child. OS scheduling means prompt failure
notification is not a hard real-time guarantee.

### Process streams and working directory

With `--allow-run` or `--allow-stdio`, `neo:process` exports `stdin`,
`stdout`, `stderr`, `cwd()`, `chdir()`, and `ChangeDirectoryError`; the same
values are also available on its default export. `stdout` and `stderr`
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
import {stdin, stdout, stderr, cwd, chdir} from "neo:process";

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

```js
import { compare } from "neo:semver";
console.log(compare("1.2.0", "1.1.9").toString());
```

## Semantic versions

`neo:semver` parses, formats, increments, compares, and matches SemVer 2.0.0
versions. It is synchronous, needs no permission flag, and grants no host
capability: it reads no file, environment variable, process argument, terminal
state, or network resource. Parsing and range evaluation are delegated to the
maintained Rust `semver` crate, which parses without a regular expression; the
module itself contains none. Its native callbacks are private to the module and
are never installed on the global object.

```js
import { parse, format, increment, satisfies, compare } from "neo:semver";

const version = parse("1.2.3-alpha.1+sha.001");
// { version: "1.2.3-alpha.1+sha.001", major: "1", minor: "2", patch: "3",
//   prerelease: ["alpha", "1"], build: ["sha", "001"] }

format(version);                                  // "1.2.3-alpha.1+sha.001"
increment("1.2.3-alpha.1", "prerelease");         // "1.2.3-alpha.2"
satisfies("1.3.0-beta.2", ">=1.3.0-beta.1, <1.4.0"); // true
compare("1.0.0+one", "1.0.0+two");                // 0
```

`valid(version)` and `compare(a, b)` are the original exports and keep their
signatures and ordinary-input behavior. `valid` returns `false` for a malformed
string; `compare` returns `-1`, `0`, or `1` by precedence and throws for a
malformed version. The new size bound applies to both; see Limits below.

`parse(version)` returns a null-prototype object whose `version`, `major`,
`minor`, `patch`, `prerelease`, and `build` are own data properties.
`prerelease` and `build` are arrays of complete identifier strings. Numeric
components are decimal strings, not JavaScript numbers, so a value above
`Number.MAX_SAFE_INTEGER` keeps every digit:

```js
parse("1.0.0-9007199254740993").prerelease[0]; // "9007199254740993"
parse("18446744073709551615.0.0").major;       // "18446744073709551615"
```

`format({ major, minor, patch, prerelease, build })` is the inverse. Core
components may be canonical decimal strings or non-negative safe integers; use
a string for a larger value. `prerelease` and `build` are arrays of single
identifiers, so a dot belongs between elements, not inside one.
`increment(version, release, identifier?)` returns a canonical string;
`release` is `major`, `minor`, `patch`, `premajor`, `preminor`, `prepatch`, or
`prerelease`. The optional `identifier` is one complete prerelease identifier,
not a dotted list. `increment` always drops build metadata and never wraps. `major`, `minor`,
and `patch` bump their component, reset lower components, and drop the
prerelease. The `pre*` releases do the same and start at `0` or
`<identifier>.0`. `prerelease` increments the last numeric prerelease
identifier, appends `.0` when none exists, or restarts at `<identifier>.0` when
the first identifier differs from the supplied one. Unlike npm,
`increment("1.0.0-alpha", "major")` is `"2.0.0"`, not `"1.0.0"`.

### Grammar and precision

A version is exactly `MAJOR.MINOR.PATCH[-PRERELEASE][+BUILD]` with no
surrounding whitespace:

- `MAJOR`, `MINOR`, and `PATCH` are unsigned ASCII decimal integers without
  leading zeros. `0` is allowed; `01` is not. Each is limited to `u64::MAX`.
- `PRERELEASE` is one or more non-empty dot-separated identifiers of ASCII
  letters, digits, and `-`. A purely numeric identifier has no leading zero
  unless it is exactly `0`. Numeric prerelease identifiers have no `u64` bound.
- `BUILD` uses the same character set, but leading zeros are allowed because
  build metadata never participates in precedence.

The English SemVer 2.0.0 specification sets no upper bound on a numeric
identifier. This module follows the Rust crate and rejects a core component
above `u64::MAX` with `SEMVER_INVALID` and a message naming the overflowing
component; it never truncates or rounds. A partial version such as `1.2` or
`1`, a leading `v`, whitespace, an empty identifier, and any character outside
the grammar are invalid.

Precedence compares the core numerically, then treats a prerelease as lower
than the same core release. Prerelease identifiers are compared left to right;
numeric identifiers compare numerically, non-numeric identifiers compare in
ASCII order, and a numeric identifier is lower than a non-numeric one. A
shorter identifier list is lower when every shared identifier is equal. Build
metadata is ignored: `1.0.0+one` and `1.0.0+two` have equal precedence. The
official ascending chain
`1.0.0-alpha < 1.0.0-alpha.1 < 1.0.0-alpha.beta < 1.0.0-beta < 1.0.0-beta.2 <
1.0.0-beta.11 < 1.0.0-rc.1 < 1.0.0` is tested.

### Ranges

`satisfies(version, range)` and `validRange(range)` use Cargo's interpretation
of ranges, not npm's. The differences matter:

- A range is `*` alone or one or more comma-separated comparators. `||` unions,
  hyphen ranges (`1.2.3 - 2.0.0`), and whitespace-separated implicit AND
  (`>=1.2.3 <2.0.0`) are invalid; write `>=1.2.3, <2.0.0`.
- A bare version uses Cargo's default caret operator: `1.2` means `^1.2`, so
  `1.9.0` matches and `1.1.9` does not. `1.2.3` means `^1.2.3`, not an exact
  match; write `=1.2.3` for exactness. It is not npm's `1.2.x` x-range. `0.2`
  matches only `0.2.x`, and `0` matches any `0.x.y`. Explicit wildcards
  `1.2.*`, `1.*`, and `*` are accepted.
- `~` and `^` follow Cargo's equivalences. In particular `^0.0.3` matches only
  `0.0.3`, and `^0.0` matches only the `0.0.x` line.
- A prerelease version matches only when at least one comparator names the
  same `MAJOR.MINOR.PATCH` and has a non-empty prerelease. Thus `*` matches no
  prerelease, and `^1.2.0` does not match `1.3.0-alpha`; use
  `>=1.3.0-alpha, <1.4.0`. npm applies the same rule unless
  `includePrerelease` is requested, which this module does not offer.
- At most 32 comparators are accepted. Build metadata on a comparator is
  allowed but ignored.

`satisfies` throws for a malformed version or range instead of returning
`false`; `validRange` returns `false` for a malformed range string.

### Errors

A non-string argument throws `TypeError` and is never coerced. A malformed
version throws `SemVerError` with code `SEMVER_INVALID`, a malformed range with
`SEMVER_RANGE`, an unknown release name or invalid increment identifier with
`SEMVER_INCREMENT`, and arithmetic overflow with `SEMVER_OVERFLOW`. The `code`
is stable; the `message` may add detail and is not stable. `valid` and
`validRange` are the non-throwing checks. Error messages describe the failing
component and never repeat the caller's full input. `neo:semver` has no
filesystem, environment, network, process, or registry failure.

### Limits

The module bounds input before it is copied into native code and rechecks the
same bounds in Rust:

- a version string: 1024 UTF-8 bytes (`MAX_VERSION_BYTES`);
- a range string: 4096 UTF-8 bytes (`MAX_RANGE_BYTES`);
- one core component: 20 bytes (`MAX_CORE_BYTES`);
- prerelease identifiers: at most 32 per version (`MAX_IDENTIFIERS`);
- build identifiers: at most 32 per version (`MAX_IDENTIFIERS`);
- one identifier: at most 256 bytes (`MAX_IDENTIFIER_BYTES`);
- `format` arrays: at most 32 entries each, the same per-identifier bound, and
  the rendered version must be at most `MAX_VERSION_BYTES`;
- a range: at most 32 comparators, in addition to the range byte limit.

The wrapper's precheck counts UTF-16 code units; native code enforces UTF-8
bytes. The accepted grammar is ASCII, so the two agree for every accepted
version or identifier; a non-ASCII string is rejected by at least one check.
The wrapper checks a primitive string's own length, scans a length-bounded
version for identifier count and size, and checks an array's length before
copying; native code repeats the checks and uses checked addition for the
rendered-size total. Exceeding a bound throws `SemVerError` with
`SEMVER_LIMIT`, except that `valid` and `validRange` return `false`. Limits
take precedence over malformed input. This is new behavior for the preserved
`valid`/`compare` functions: an over-limit version now makes `valid` return
`false` and `compare` throw `SEMVER_LIMIT` rather than parsing without a bound.

`format` reads properties from its argument. A getter, Proxy trap, or array
access can run ordinary user code before the returned value is checked; the
module does not claim otherwise. The bounds apply to the primitive strings and
array lengths those reads produce.

## Text utilities

`neo:chars`, `neo:string-builder`, and `neo:ansi` are synchronous, pure
JavaScript modules. They need no permission flag and grant no host capability:
they do not read files, environment variables, process arguments, or terminal
state, and they install nothing on the global object.

### Characters

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

isAscii("A".codePointAt(0));       // true
isAscii("é".codePointAt(0));       // false
isLetter("Ж".codePointAt(0));      // true
isDigit("٥".codePointAt(0));       // true (Arabic-Indic five)
isSpace(0x3000);                   // true (ideographic space)
isSymbol("😀".codePointAt(0));      // true
isLetter("😀".codePointAt(0));      // false
toLower(0x41);                     // 0x61, "A" to "a"
equalFold(0x3c2, 0x3c3);           // true (final sigma and sigma)
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

### StringBuilder

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
builder.toString();       // "Hello, 42!"
builder.length;           // 10, UTF-16 code units
builder.clear().append("again").toString(); // "again"
```

`length` is the number of UTF-16 code units, matching `String.prototype.length`;
`codePointLength` counts Unicode code points, so `appendCodePoint(0x1f600)`
adds two code units and one code point. `.string` is an alias for
`toString()`. The builder stores UTF-16 strings and joins them lazily. It is
not `@neotales/slices`' `CharArrayBuilder`: there is no code-point buffer, no
`appendSlice` or `appendCharArray`, no `Date` support, and no capacity
argument; the internal chunk array grows as text is appended.

### ANSI styles

`neo:ansi` exports style functions for text attributes, 16 foreground and 16
background colors, 256-color, and 24-bit color. Color is on by default and is
never detected: no environment variable, process argument, or terminal check
changes the output, and there is no module-global toggle. Pass
`{ color: false }` to return the input unchanged.

```js
import { apply, bgBlue, blue, bold, red, rgb24, stripAnsi } from "neo:ansi";

red("error");                       // "\x1b[31merror\x1b[39m"
red("error", { color: false });     // "error"
apply("notice", [bold, blue]);      // blue wraps bold; the last style is outermost
rgb24("warm", 0xff8c00);            // SGR 38;2;255;140;0
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

## Dotenv

`neo:dotenv` parses `.env` content strings synchronously. It needs no
permission flag and grants no host capability: it reads no file, consults or
mutates no process environment, expands no variables, and runs no shell
command. Reading a `.env` file is a separate, explicit decision:

```js
import { parse } from "neo:dotenv";
import { readText } from "neo:fs"; // requires --allow-fs

const values = readText(".env"); // only with --allow-fs
const config = parse(values);
```

The native parse callback is private to the module. It is not installed on the
global object, so replacing a similarly named global cannot forge a result or
call the native function directly.

`parse(source)` returns a null-prototype record. Keys such as `__proto__`,
`constructor`, and `prototype` are ordinary own enumerable data properties, so
parsing untrusted content cannot change the result's prototype or pollute
`Object.prototype`. Duplicate keys are last-value-wins. Property enumeration is
JavaScript's own order, not source order: array-index keys (canonical numeric
strings) come first in ascending numeric order, then other string keys in
first-insertion order. Source order `10, 2, b, a` enumerates as `2, 10, b, a`.
Use `Object.keys`, bracket access, or `{ ...config }` when a normal object is
wanted.

### Syntax

- A leading UTF-8 BOM is ignored. CRLF and lone CR are normalized to LF before
  parsing, including inside quoted values. NUL is rejected anywhere, including
  a `\u0000` or `\U00000000` escape.
- Only ASCII space and tab are whitespace. Blank lines and lines whose first
  non-space character is `#` are ignored.
- `export` is a prefix only when the exact, case-sensitive token is followed by
  a space or tab, so `export A=1` defines `A` while `export=1` defines `export`.
- A key is one or more ASCII letters, digits, or `_`. It must not be empty and
  is limited to 1,024 UTF-8 bytes. Spaces between a key and `=` are allowed.
- An unquoted value runs to the line end, with trailing spaces and tabs
  trimmed. `#` starts a comment only at the start of the value or after a space
  or tab: `PASSWORD=abc#123` keeps `abc#123`, while `VALUE=abc # note` is
  `abc`. A line without `=` is an error.
- A single-quoted value removes the outer quotes; only `\'` is an escape, and
  every other backslash is literal.
- A double-quoted value removes the outer quotes and decodes `\n`, `\r`, `\t`,
  `\b`, `\\`, `\"`, `\'`, a backslash before a backtick, `\uXXXX`, and
  `\UXXXXXXXX`. An unknown escape such as `\q` stays literal; a malformed or
  surrogate `\u`/`\U` escape is an error.
- A backtick-delimited value is raw: the outer backticks are removed, no escape
  is decoded, and the contents are never executed.

  ```dotenv
  KEY=`echo hi`
  ```

  The value of `KEY` above is the literal text `echo hi`.
- Unquoted values have no escape processing; a backslash is an ordinary
  character.
- Quotes may span lines. After a closing quote, only spaces, tabs, and an
  optional `#` comment may follow the line. An unterminated quote is an error.

Values are never expanded. `$HOME`, `${HOME:-/tmp}`, and `$(command)` are
returned exactly as written. With `--allow-env`, expansion is something the
caller opts into separately. Because command substitution is not interpreted,
a double-quoted value ends at the first unescaped `"` even inside `$(...)`; use
`\"` there, or a single-quoted or backtick-delimited value, if the text contains
an unescaped quote.

### Errors and limits

The first error in source order throws a `DotenvParseError` with a stable
`code`, one-based `line`, and one-based `column` in Unicode scalar values.
Match on `code`, never on the message; messages never include source text or
values. Codes are `DOTENV_SYNTAX`, `DOTENV_KEY`, `DOTENV_QUOTE`,
`DOTENV_ESCAPE`, `DOTENV_LIMIT`, `DOTENV_ENCODING`, and `DOTENV_NUL`. A
non-string argument throws `TypeError`, and an unpaired UTF-16 surrogate is
rejected as `DOTENV_ENCODING`. Source is limited to 1 MiB of UTF-8 bytes before
line-ending normalization, 16,384 assignments, and 1,024-byte keys; exceeding a
limit is `DOTENV_LIMIT`. An over-limit source reports `DOTENV_LIMIT` before
parsing, so the size check takes precedence over syntax. The native callback
checks the primitive string's UTF-16 length before conversion and the exact
UTF-8 byte count before parsing.

Deliberate differences from `@neotales/dotenv`: this module has no
`stringify`, document token stream, `load`, or implicit expansion. It rejects
unterminated quotes, bare lines without `=`, and malformed `\u`/`\U` escapes
that the reference parser would keep as text. It also keeps `__proto__` as a
key instead of assigning it through `Object.prototype` and losing it. The
reference treats any `#` in an unquoted value as a comment; this parser
requires whitespace before `#` so a value that contains `#` is not silently
truncated.

## TOML and YAML

`neo:toml` and `neo:yaml` parse configuration text synchronously. They need no
permission flag and grant no host capability: they read no file, consult no
environment, access no network, and execute nothing. Reading a file is a
separate, explicit decision:

```js
import { parse as parseToml } from "neo:toml";
import { parse as parseYaml } from "neo:yaml";
import { readText } from "neo:fs"; // requires --allow-fs

const config = parseToml(readText("config.toml")); // only with --allow-fs
const workflow = parseYaml(readText("workflow.yaml"));
```

The native parse callbacks are private to their modules. They are not installed
on the global object, so replacing a similarly named global cannot forge a
result or call the native function directly.

Both modules return the same JavaScript shapes: objects are null-prototype
records, arrays are arrays, and leaves are strings, booleans, finite numbers,
and null. YAML can produce null; TOML has no null value. Keys such as
`__proto__`, `constructor`, and `prototype` are ordinary own enumerable data
properties, so parsing untrusted content cannot change a result's prototype or
pollute `Object.prototype`. Object keys are not in source order: both parsers
encode objects with sorted keys, and JavaScript then applies its own
enumeration order, with integer-like keys first. Use bracket access or
`Object.keys`, not source order.

TOML values follow these rules:

- TOML date and time values, including offset date-times, local date-times,
  local dates, and local times, are rejected as `TOML_DATETIME`. They are not
  converted to strings or `Date` objects, because JavaScript has no built-in
  exact type for all four TOML forms and a string conversion would silently
  lose the distinction. Handle dates in the caller if they are needed.
- Integers outside ±(2^53 - 1) are rejected as `TOML_INTEGER`; they cannot
  round-trip through a JavaScript number.
- Non-finite floats (`inf`, `-inf`, and `nan`) are rejected as `TOML_FLOAT`.
- Duplicate keys and redefined tables are syntax errors, `TOML_SYNTAX`.
- Empty input is an empty record.
- Depth counts collections, with the root table at depth one. A scalar leaf
  inside the deepest permitted collection does not add another level.

YAML values are deliberately a smaller language than general YAML:

- Exactly one document is accepted. A second document is `YAML_MULTIDOC`.
- Aliases are rejected as `YAML_ALIAS`. Anchors without aliases are ignored.
  Rejecting an alias at its event, before `saphyr-parser` can resolve it, is
  what prevents an alias bomb from expanding: a small document cannot turn into
  a large tree. A recursive or cyclic alias graph is impossible for the same
  reason. Merge keys written with an alias are therefore rejected too. A
  `<<` key without an alias is an ordinary string key; merge semantics are not
  applied.
- Explicit tags, including core-schema tags such as `!!str`, are rejected as
  `YAML_TAG`. Tagged input is never interpreted as an object constructor.
- Mapping keys must resolve to strings. Numeric, boolean, null, sequence, and
  mapping keys are `YAML_KEY`; empty keys are null and are rejected for the
  same reason.
- Duplicate keys are `YAML_DUPLICATE`, not last-value-wins.
- Plain scalars follow the YAML 1.2 core schema: null (`~`, `null`, `Null`,
  `NULL`, or empty), `true`/`false` in the three core spellings, integers,
  finite floats, and strings. `yes`, `no`, `on`, and `off` remain strings, as
  do date-like scalars: the core schema has no timestamp type, so
  `2020-01-01` is a string, not a `Date`. Quoted scalars are always strings.
- YAML integers outside ±(2^53 - 1) are `YAML_INTEGER`; `.inf`, `-.inf`,
  `.nan`, and a finite-looking literal that overflows to infinity are
  `YAML_FLOAT`.
- Empty input, or a stream containing only `---`, is null.

### Limits and errors

Both modules limit source to 1 MiB of UTF-8 bytes, nesting to 64 collection
levels (counting the root collection as one), total nodes to 100,000, entries
in one collection to 100,000, and the JSON response built for JavaScript to
16 MiB. The named constants are exported. The source size is checked before
parsing. Exceeding a limit is `TOML_LIMIT` or `YAML_LIMIT`; exceeding the depth
is `TOML_DEPTH` or `YAML_DEPTH`.

TOML budget enforcement happens in a streaming event pass over the bounded
source before the `toml` crate builds a value tree. That pass lexes the source
into a token vector proportional to the 1 MiB input and consumes parser events
without materialising a tree; only after it accepts does the value tree get
built. A separate preflight-work limit of 100,000 accounting units counts
repeated table prefixes and repeated array-of-tables containers again. Thus,
for example, 50,000 dotted keys sharing one prefix can exceed this work limit
even though the resulting tree contains only 50,002 nodes. Header depth is
also conservative after an array-of-tables header: prior array headers may be
counted as ancestors even when unrelated, causing an early depth rejection.
Preflight depth and work errors take precedence over semantic errors such as
duplicate keys, even when the semantic error appears earlier. The parser still
allocates: the token vector and the accepted tree are
bounded by the source and node limits, but this is not a zero-allocation
parser. The preflight uses the
maintained toml-rs lexer/parser. If that parser rejects text the TOML 1.0 value
parser would accept, the module returns `TOML_SYNTAX` rather than building an
unvalidated tree. Post-parse conversion also checks node and item limits.

YAML consumes `saphyr-parser` events directly and checks budgets before
inserting into the result; aliases are rejected at their event, before any
expansion. Exceeding a budget is reported at the event that crossed it.

Error positions follow that model. A TOML or YAML source-size limit has no
event to point at and reports `line` 1, `column` 1. TOML and YAML preflight or
event budget errors report the event position. A TOML conversion rejection with
no source span (datetime, integer, float, or a post-parse conversion fallback)
reports `line` 0, `column` 0.

A non-string argument throws `TypeError`. Unpaired UTF-16 surrogates are
rejected as `TOML_ENCODING` or `YAML_ENCODING`, and NUL is rejected as
`TOML_NUL` or `YAML_NUL`. The native callback checks the primitive string's
UTF-16 length and exact UTF-8 byte count before conversion. A leading BOM is
ignored. CRLF and lone CR are normalized to LF before parsing, and line and
column positions refer to the normalized text.

Malformed input throws `TomlParseError` or `YamlParseError` with a stable
`code`, a one-based `line`, and a one-based `column` in Unicode scalar values.
Match on `code`, never on the message. Messages are static and never include
source text or parsed values. TOML syntax errors carry the parser's source
span.

## Scheduler configuration

`neo:scheduler` generates configuration text for a recurring job on systemd or
Windows Task Scheduler. It is a pure, synchronous JavaScript module: it needs
no permission flag and grants no host capability. It does not read or write a
file, create a directory, change a permission, start a process, run a shell,
call `systemctl`, register a task, or enable anything. The returned strings are
inert until a caller with separate authority writes and installs them.

```js
import { systemdUnits } from "neo:scheduler";

const { service, timer } = systemdUnits({
  name: "backup",
  description: "Nightly backup",
  program: "/usr/local/bin/backup",
  args: ["--full", "My Data"],
  workingDirectory: "/var/lib/backup",
  user: "backup",
  schedule: { kind: "daily", at: "02:30" },
  persistent: true,
});

// Writing these strings is a separate decision:
// import { writeText } from "neo:fs"; // requires --allow-fs
// writeText("/etc/systemd/system/backup.service", service);
// writeText("/etc/systemd/system/backup.timer", timer);
```

The service is a `Type=oneshot` unit. The timer activates `<name>.service` and
is wanted by `timers.target`; the generated service has no `[Install]` section
because the timer, not the service, is the unit to enable. A system unit
normally belongs in `/etc/systemd/system` and needs administrator authority; a
user unit normally belongs in `~/.config/systemd/user`. Installing or
reloading either is outside this module.

```js
import { windowsTask } from "neo:scheduler";

const xml = windowsTask({
  name: "Backup",
  description: "Nightly backup",
  startDate: "2026-11-18",
  program: "C:\\Program Files\\Neo\\backup.exe",
  args: ["--full", "C:\\Data Folder"],
  workingDirectory: "C:\\Program Files\\Neo",
  schedule: { kind: "daily", at: "02:30" },
  principal: {
    userId: "CONTOSO\\svc-backup",
    logonType: "Password",
    runLevel: "LeastPrivilege",
  },
});
```

`windowsTask` returns XML with the declaration
`<?xml version="1.0" encoding="UTF-16"?>`. Write the string as UTF-16LE with a
byte-order mark, the usual Task Scheduler export encoding, or replace the
declaration with one that matches the bytes you actually write.
`WINDOWS_XML_ENCODING` is `"UTF-16LE"`. The module returns text, not bytes, and
performs no encoding. It does not register the task; registration and the
`Password` credential are the caller's responsibility, and this module never
accepts or stores a password.

The supported schedule is deliberately small:

| Schedule | systemd | Task Scheduler |
| --- | --- | --- |
| `{kind: "daily", at: "HH:MM"}` | `OnCalendar=*-*-* HH:MM:00` | `ScheduleByDay` with `DaysInterval=1` |
| `{kind: "weekly", at: "HH:MM", weekdays: [...]}` | `OnCalendar=Mon,Wed *-*-* HH:MM:00` | `ScheduleByWeek` with `WeeksInterval=1` |

`HH` is 00 through 23 and `MM` is 00 through 59; seconds, one-shot schedules,
monthly schedules, and interval schedules are not generated. Weekdays are the
lowercase three-letter names `mon` through `sun`, must be distinct, and are
emitted in Monday-through-Sunday order. Unknown kinds, a daily schedule that
also carries `weekdays`, and a weekly schedule without `weekdays` are rejected
rather than approximated.

Time zones are not converted. An `OnCalendar` expression without a time-zone
suffix uses the local time of the machine running the timer. A Task Scheduler
`StartBoundary` is emitted without a time-zone suffix and Task Scheduler
applies its own local-time interpretation. The two targets can therefore mean
different instants around a daylight-saving transition, and neither target's
missed-run or wall-clock behaviour is modelled. `persistent: true` emits
systemd's `Persistent=true`, which catches up a missed OnCalendar run when the
timer is next activated. The Windows XML deliberately omits `<Settings>`, so
Task Scheduler's registration defaults apply; the module does not claim that
the two catch-up policies are equivalent.

`startDate` is required for `windowsTask` and is a Gregorian `YYYY-MM-DD` date
from 1601 through 9999. It is written verbatim as the trigger's
`StartBoundary` date and is not computed or adjusted by this module. The
systemd side has no corresponding date.

### Arguments and escaping

For systemd, `ExecStart=` is generated as one double-quoted item per argument,
and systemd's own command-line parser, not a shell, turns the line back into
an argv. Whitespace, quotes, backslashes, semicolons, percent signs, and
dollar signs inside an argument do not split it or run a command. `%` is
doubled because systemd expands specifiers; `$` is doubled because ExecStart
performs environment-variable substitution; `\` and `"` are backslash-escaped.
A literal argument consisting of one semicolon, a program containing `$`, and
control characters are rejected because their ExecStart interpretation is
ambiguous. No shell is used and no shell quoting is attempted. A caller that
wants a shell must name it explicitly as the program, and then the arguments
belong to that shell's language, which this module does not escape.

`Description=`, `User=`, and `WorkingDirectory=` are emitted without
surrounding quotes and accept a smaller character set than systemd itself:
`%`, backslashes, quotation marks, control characters, and leading or trailing
whitespace are rejected. This avoids specifier expansion, escape, and
line-continuation ambiguity. `WorkingDirectory` must be an absolute POSIX path.
`User` must be a simple name or a decimal numeric ID; it is not looked up, and
the module does not set `Group=`, supplementary groups, capabilities, or
`DynamicUser=`. With no `user`, the unit uses the service manager's default
(root for a system unit, the invoking user for a user unit).

For Windows, `<Command>` holds the program path by itself and the module does
not wrap it in quotation marks. `<Arguments>` holds the remaining arguments
joined by one space, with each argument quoted according to the Microsoft C/C++
runtime `CommandLineToArgvW` convention. That convention is used by many
programs but not by every program; in particular it is not `cmd.exe` or
PowerShell quoting. A program that parses its command line differently needs
its own argument representation, which this module does not invent.

Windows paths must be absolute drive paths (`C:\...`) or UNC paths
(`\\server\share\...`) with backslash separators. Relative paths, forward
slashes, `.` and `..` components, empty components, invalid Windows filename
characters, percent signs, and device paths such as `\\?\...` and `\\.\...`
are rejected. The Task Scheduler schema's 260-character `pathType` limit is
enforced for the program and working directory. No path is checked for
existence, and other Windows path-length and permissions policy is not
enforced.

`principal` is optional. When present, it requires `userId` and one of the
Task Scheduler logon types `InteractiveToken`, `Password`, `S4U`, or
`InteractiveTokenOrPassword`, and optionally `LeastPrivilege` or
`HighestAvailable` as `runLevel`. Those four names are the complete
`LogonType` enumeration in the authoritative XSD; `Group` and
`ServiceAccount` are rejected rather than emitted. `InteractiveToken` runs
only for an already logged-on user, `Password` needs credentials supplied
separately at registration, and `S4U` stores no password and has no network
access; those are Task Scheduler's semantics, not this module's.
`HighestAvailable` requests the highest privileges available to the account.
Omitting `principal` omits `<Principals>` so registration applies its own
default; that default is not asserted here. A systemd `User=` only changes the
service process identity; it does not grant or remove the authority of the
administrator who installs the unit.

### Errors and verification

Every rejected value throws `SchedulerError` with a stable `code` and the
`field` that failed. Messages never include the rejected value, so a secret in
an argument is not copied into an error. The codes are `SCHEDULER_TYPE` (wrong
JavaScript type), `SCHEDULER_VALUE` (invalid value), `SCHEDULER_SCHEDULE`
(invalid schedule), `SCHEDULER_UNSUPPORTED` (unsupported or not safely
representable), and `SCHEDULER_LIMIT` (a documented size bound).

Every options object must be a plain object (prototype `Object.prototype`) or
a null-prototype record whose own properties are enumerable data properties.
Accessors, own symbol properties, non-enumerable own properties, unknown keys,
arrays, class instances, and objects with another prototype are rejected.
Validation reads property descriptors rather than `object[key]`, so an
accessor is rejected without being invoked. Only own values are used: an
inherited enumerable property is rejected, an inherited value never satisfies
a known option, and accepted values are copied onto a null-prototype record
before use. A transparent Proxy over a conforming record is
indistinguishable and may be accepted; a Proxy can run its traps during
inspection. `args` and `weekdays` are read by own index; an accessor or
non-enumerable index is rejected. Other own properties on those arrays are
ignored and cannot alter the generated schedule or argv. A misspelled or
unsupported key such as `interval` or `groupId` is never silently ignored.
Shape violations report `SCHEDULER_TYPE` for a non-object, array, or non-plain
prototype, and `SCHEDULER_UNSUPPORTED` for accessors, own symbols,
non-enumerable own properties, inherited enumerable properties, or unknown
keys. The exported bounds are
`MAX_SYSTEMD_LINE_BYTES` and `MAX_WINDOWS_COMMAND_LINE`; there is also a
per-argument bound and an internal argument-count bound. A Proxy trap can
still throw its own error.

The generated Task Scheduler XML declares schema version 1.3, the fixed value
in the authoritative XSD. `CalendarTrigger` follows the XSD sequence
(`Enabled` before `StartBoundary`). Weekly output emits `DaysOfWeek` before
`WeeksInterval`; the XSD's `weeklyScheduleType` is `xs:all`, so either order is
schema-valid and this is a documented convention, not a validity requirement.
`Actions/@Context` is emitted only when a principal with `id="Author"` is
present, because the schema's `ContextKeyRef` requires the reference to
resolve.

The repository tests run generated units through `systemd-analyze verify` and
validate generated UTF-16LE XML against the pinned Task Scheduler 1.3 XSD with
`xmllint --schema` on temporary files, when those tools are installed. The
schema fixture is reproduced from Microsoft Learn with provenance and license
in `crates/neo-js/tests/fixtures/`. Schema validation covers document validity,
not installation, service execution, or Windows runtime behaviour. No Windows
runtime is claimed.

## Path utilities

`neo:path` provides synchronous, lexical operations for POSIX and Windows
paths. Importing it requires no permission flag. Choose `posix` or `win32`
for a fixed dialect, or use named imports for the host dialect (`win32` on
Windows, `posix` elsewhere). No operation performs filesystem I/O, checks
existence, resolves symlinks or junctions, or reads environment variables.

```js
import { posix, win32, SEPARATOR, SEPARATOR_PATTERN } from "neo:path";

["app", "data"].join(SEPARATOR);       // "app/data" or "app\\data"
"app///data".split(SEPARATOR_PATTERN); // ["app", "data"]

posix.join("/srv", "app", "..", "data"); // "/srv/data"
posix.dirname("/srv/data/file.txt");     // "/srv/data"
posix.basename("/srv/data/file.txt");    // "file.txt"
posix.extname("/srv/data/file.txt");     // ".txt"
posix.parse("/srv/data/file.txt");
// { root: "/", dir: "/srv/data", base: "file.txt", ext: ".txt", name: "file" }

win32.normalize("C:/work/out/../log");   // "C:\\work\\log"
win32.resolve("C:/work/out", "../log");  // "C:\\work\\log", on any host, no grant
win32.parse("\\\\server\\share\\file");
// { root: "\\\\server\\share\\", dir: "\\\\server\\share\\",
//   base: "file", ext: "", name: "file" }
```

The frozen namespaces both provide `join`, `normalize`, `basename`, `dirname`,
`extname`, `isAbsolute`, `parse`, `format`, `common`, `resolve`, `relative`,
and `toNamespacedPath`. Each function is also available as a host-selected
named export. The named constants `SEPARATOR` and `DELIMITER` correspond to
the selected namespace's `sep` and `delimiter`: `/` and `:` on POSIX, `\`
and `;` on Windows. `SEPARATOR_PATTERN` is a regular expression matching runs
of host path separators, including both `/` and `\` on Windows. `parse`
returns a fresh plain object on each call.

### Arguments and errors

Every path must be a primitive string. A boxed `String`, `URL`, buffer, number,
`null`, `undefined`, or other value throws `TypeError`; nothing is coerced.
Windows functions accept both `\` and `/` as separators on input, except
`common`, which compares spelling using `\` only. `common` takes an array of
paths. `basename`'s optional suffix must be a string.
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

### Common prefixes, resolution, and the working directory

- `common(paths)` compares components without normalizing or folding case.
  It returns the shared path prefix, including the separator when paths
  diverge after a shared component, or `""` when there is no common prefix
  or the array is empty.
- `resolve(...paths)` works from right to left. It validates every segment,
  ignores empty strings, eliminates `.` and `..`, and returns an absolute
  path with no trailing separator except at a root. On Windows, `\child`
  must inherit a drive or share even though `isAbsolute("\\child")` is
  true. A drive-relative path such as `C:child` needs a matching drive base.
  No per-drive environment or implicit drive-root fallback is used.
- `relative(from, to)` resolves both operands, then returns the path from
  `from` to `to`, or `""` if they resolve to the same path. Windows compares
  roots and components case-insensitively, preserves destination spelling,
  and returns the absolute destination when the drive, share, or namespace
  differs.
- POSIX `toNamespacedPath(path)` returns the validated string unchanged.
  Windows resolves ordinary drive and UNC paths into `\\?\` and
  `\\?\UNC\` forms, respectively. Empty and already namespaced/device
  paths are returned unchanged. This is a string conversion, not validation
  for Windows I/O.

Supplying a fully qualified base or fully qualified operands requires no
permission flag, even for the non-host dialect. When resolution needs an
implicit base (including `resolve()`), it can disclose the **runtime-local**
working directory. This requires either filesystem permission (`--allow-fs`)
with `neo:fs` allowed, or process/stdio permission (`--allow-run` or
`--allow-stdio`) with `neo:process` allowed. Allowing `neo:path` alone does
not give it access to the cwd. This fallback is available only in the host
dialect. It observes `neo:process.chdir()` without changing or rereading the
host process's working directory.

| Error code | Meaning |
| --- | --- |
| `PATH_PERMISSION` | The operation needs the runtime cwd, but neither qualifying grant and module pair is available. |
| `PATH_CWD` | Cwd access is granted, but the runtime cwd is unavailable. |
| `PATH_CWD_DIALECT` | Cwd fallback is not available for the non-host dialect; supply an explicit base. |
| `PATH_WINDOWS_ROOT` | The operation could not obtain a matching drive or share, or the Windows namespace root is incomplete. |

### POSIX rules

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

### Windows rules

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
- `parse` and `normalize` keep their original UNC-shaped treatment of
  `\\?\` and `\\.\`. For example, `parse` reports `\\?\UNC\` as the root of
  `\\?\UNC\server\share\foo`, while `resolve` treats the full namespaced
  server and share as one root. The module does not enumerate devices or
  validate reserved device names or alternate data streams.

The Windows rules are pure string logic and are exercised on Linux by the test
suite; no Windows host was used to run it.

### Round trips and deliberate differences

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
implemented. URL inputs, buffers, and type coercion are not accepted. There
is no POSIX `//` root preservation. For a drive root or a complete UNC root,
`basename` and the `parse` component fields are empty rather than a separator,
and Windows `dirname("C:/")` preserves the root as written instead of stripping
its separator. There is no asynchronous form.

The path algorithms are adapted from `@neotales/path` under the MIT license.
See `crates/neo-js/src/js/path.LICENSE.md` for the exact upstream revision, the
adapted-file list, and the complete upstream copyright and permission notices.
The same text is embedded in the library and binary: print it with
`neo-js --third-party-notices` or call `neo_js::third_party_notices()`. It is
not a complete inventory of every dependency's license.

### Security: not a safe jail

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

## Filesystem

`neo:fs` is a synchronous, bounded subset of the usual filesystem API. It is
available only with `--allow-fs` and only when the runtime's module allowlist
admits `neo:fs`. Both gates are checked before the `fs` global or any native
callback is built. The global `fs` object and the module expose the same
functions and `FsError` class.

| Function | Description |
| --- | --- |
| `readText(path)` | Read a regular file as UTF-8 text. |
| `writeText(path, data)` | Write text, creating or truncating the file. |
| `readFile(path)` | Read a regular file as a `Uint8Array`. |
| `writeFile(path, data, options?)` | Write a `Uint8Array`; `{overwrite: false}` is create-new. |
| `copyFile(source, destination)` | Stream a regular file into a new file; never overwrite. |
| `stat(path)` | Metadata, following a final symlink. |
| `lstat(path)` | Metadata without following a final symlink. |
| `exists(path)` | Existence check, following a final symlink. |
| `readDir(path)` | Non-recursive, sorted directory listing. |
| `mkdir(path, options?)` | Create a directory; `{recursive: true}` creates parents. |
| `remove(path, options?)` | Remove a file, link, or directory; `{recursive: true}` removes a tree. |
| `rename(source, destination)` | Rename a file, directory, or symlink entry using the host OS. |

```js
import { readDir, stat } from "neo:fs";

// Read-only example: it lists the current directory and inspects a known
// entry only if present. It does not create, overwrite, or delete user files.
const entries = readDir(".");
if (entries.some((entry) => entry.name === "README.md")) {
  console.log(stat("./README.md").size);
}
```

### Working directory

Relative paths use the runtime's working directory, not the host process's
mutable directory. `neo:process.chdir` changes only that runtime. If the
runtime directory is unavailable, a relative operation reports `FS_CWD` and
never falls back to the host directory. Absolute paths and `..` components
still reach everything the host user can reach; the working directory is
coordination, not confinement.

### Files and sizes

`readFile`, `readText`, `writeFile`, and `writeText` require a regular file.
An open may occur before a non-regular object is rejected. An identified
directory reports `FS_IS_DIRECTORY`; other identified non-regular objects
report `FS_TYPE`. If opening fails before the type can be identified, the
underlying open error determines the code. `stat` and `lstat` inspect any
path the operating system exposes.

`writeFile` and `writeText` do not create parent directories. They verify the
opened descriptor is a regular file before truncating that same descriptor;
on Unix, opens use `O_NONBLOCK` to avoid waiting for a FIFO peer and `O_NOCTTY`
to avoid acquiring a controlling terminal. Opening a device can still have
other side effects, and there is no latency guarantee. Writes are
not atomic: a failure can leave a truncated or partially written file. There
is no temporary file, rename step, or lock.
Writing follows a symlink at the final path component. A dangling symlink is
followed and its target is created by default; `{overwrite: false}` refuses the
path if the link itself exists.

`FileInfo.size` is a decimal string of the exact `u64` byte size, never a
JavaScript number. A value above `Number.MAX_SAFE_INTEGER` therefore keeps
every digit. `mtimeMs` is whole milliseconds since the Unix epoch as a decimal
string rounded toward zero (so a pre-epoch timestamp less than one millisecond
from 1970 is `"0"`), or `null` when the host cannot report it. For a symlink
reported by `lstat`, `size` is the host's symlink size, not the target size.

### Symlinks

The rules are explicit:

- `stat`, `readFile`, `readText`, `writeFile`, `writeText`, `exists`, and
  `readDir` follow a symlink at the final path component.
- `lstat` does not follow it and reports `isSymlink: true`.
- `readDir` follows a symlink to a directory but does not follow entries; an
  entry that is a link is reported with `isSymlink: true`.
- `remove` does not follow a symlink at the target; it removes the link
  except for Windows directory symlinks (see below). Recursive deletion also
  does not follow symlinks inside the tree.
- `mkdir` follows the operating system's rules for existing components.
- `rename` acts on the final entries without following either final symlink.
- `copyFile` follows a final source symlink to a regular file. A final
  destination symlink, even a dangling one, is an existing entry and is not
  followed.

A dangling symlink is reported by `lstat` but `exists` follows it and returns
`false`. On Windows, `remove` rejects directory symlinks named directly as
targets with `FS_UNSUPPORTED`, even if dangling. Windows reparse-point and
junction behavior has not been verified by running on Windows.

### Listing

`readDir` is non-recursive and returns entries sorted by name in UTF-8 byte
order, which is also Unicode code-point order. A name that is not valid UTF-8
throws `FS_ENCODING`; the module never substitutes a lossy name. The listing is
bounded before it can grow without limit; exceeding a bound throws `FS_LIMIT`.
The listing is not an atomic snapshot: entries can change while the host reads
the directory, and type flags may reflect a different instant from the names.

### Deletion

`remove(path)` removes a file, symlink, or empty directory. A non-empty
directory throws `FS_NOT_EMPTY`. `remove(path, {recursive: true})` removes a
tree. Recursive deletion has no count limit and no undo; choose the path
carefully. It may remove some entries before an OS error stops the traversal;
there is no rollback. Parsed roots (including Windows UNC and verbatim UNC
share roots without a trailing separator), paths ending in a separator, and
paths ending in a `.` or `..` component are rejected with `FS_PATH` before cwd
resolution, without canonicalizing. Remove a specific child instead.

Options are strict: only `overwrite` or `recursive` (as applicable) is
accepted, and each option must be an own, enumerable data property. Accessors,
non-enumerable properties, and unknown keys throw `TypeError`; inherited
properties are ignored. A Proxy may run traps during reflective validation.
The runtime-local absolute path is snapshotted before this reflection, so a
trap that changes `neo:process.chdir()` cannot redirect that call. Proxy traps
can still run arbitrary script code and reenter the filesystem API.

### Rename

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

### Copy

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

### Limits

| Constant | Value | Applies to |
| --- | ---: | --- |
| `MAX_PATH_BYTES` | 32 KiB | Every path argument. |
| `MAX_READ_BYTES` | 64 MiB | `readText`, `readFile`, and `copyFile`. |
| `MAX_WRITE_BYTES` | 64 MiB | `writeText` and `writeFile`. |
| `MAX_LIST_ENTRIES` | 100,000 | Entries returned by `readDir`. |
| `MAX_LIST_NAME_BYTES` | 1 KiB | One directory-entry name. |
| `MAX_LIST_TOTAL_BYTES` | 16 MiB | Sum of all names returned by `readDir`. |

Reads inspect the opened descriptor and its metadata before allocating, then
read in bounded chunks with fallible reservation. A separate one-byte probe
detects growth past the limit. A binary write copies the `Uint8Array` view's
current bytes (including for a resizable `ArrayBuffer`) before reflecting on
options. `SharedArrayBuffer` views are unsupported. `writeText` checks the
primitive string's code-unit length before exporting it from QuickJS as UTF-16.
The UTF-16 export itself can allocate up to 128 MiB of string data. If QuickJS
flattens a narrow string rope, the flattened string can use another 64 MiB
and coexist with the export: up to 192 MiB of additional engine string data
in that case. These figures exclude the original input, overhead, and the
separate Rust UTF-8 snapshot; they are not a total memory bound. The native
scan rejects unpaired surrogates before applying the exact UTF-8 byte limit;
only then does it reserve space for the UTF-8 snapshot. The limits are
exported as constants on both the global `fs` object and the module.

### Errors

Filesystem failures are `FsError` values with a stable `code` and a fixed
message that never contains the supplied path, an OS error string, or file
contents. Argument type errors are `TypeError`, and exceptions thrown by
option proxy traps can propagate.

| Code | Meaning |
| --- | --- |
| `FS_PATH` | Invalid or unsupported path syntax, including empty paths, embedded NULs, and on Windows, drive-relative paths; also a parsed root, trailing separator, or final `.` or `..` for `remove`, `rename`, and `copyFile`. |
| `FS_NOT_FOUND` | The path does not exist (or a parent does not). |
| `FS_NOT_DIRECTORY` | A path component, or a listed path, is not a directory. |
| `FS_IS_DIRECTORY` | A file operation was given a directory. |
| `FS_ALREADY_EXISTS` | The OS reported an existing path, e.g. for `copyFile`, non-recursive `mkdir`, a write with `{overwrite: false}`, or a `rename` rejected because the destination directory is non-empty. |
| `FS_NOT_EMPTY` | The OS required an empty directory, e.g. for non-recursive `remove` or some `rename` replacements. |
| `FS_CROSS_DEVICE` | The OS reported that the rename crossed filesystems. |
| `FS_DENIED` | The operating system denied access. |
| `FS_LIMIT` | A path, read, write, or listing exceeded a documented bound, or a filesystem quota, storage, or fallible allocation resource was unavailable. |
| `FS_ENCODING` | Text or a directory-entry name is not valid UTF-8. |
| `FS_DETACHED` | A `Uint8Array` buffer was detached. |
| `FS_OUT_OF_BOUNDS` | A `Uint8Array` view is out of bounds after its resizable buffer shrank. |
| `FS_CWD` | A relative operation needs an unavailable runtime working directory. |
| `FS_TYPE` | The path is not a regular file. |
| `FS_UNSUPPORTED` | The operation or input is unsupported, e.g. direct removal of a Windows directory symlink or writing from a `SharedArrayBuffer` view. |
| `FS_IO` | Any other filesystem failure. |

### Authority and races

This is not a sandbox. A script with `neo:fs` has the host user's filesystem
authority. Path confinement, a virtual root, and copy-on-write are not
provided. The module does not start a shell, a child process, or a network
request. Operating-system filesystem activity can still produce network
traffic when a path refers to a network mount or a special filesystem.

`exists` and `stat` are snapshots and are time-of-check/time-of-use race prone.
A path can be replaced between resolution and open, delete, or rename, so no operation
is atomic with a prior check. Reads and writes validate the opened descriptor.
After some failed opens, a separate metadata lookup may refine the error code;
this lookup is not used to authorize reading, writing, or truncating. Deletion
remains pathname based and is not race-free. Do not use `exists` to decide whether a later
operation is safe; perform the operation and handle `FsError` instead.
`readDir` and recursive deletion are non-atomic; recursive deletion may be
partial on failure. Writes can leave partial files, and no file locking is
performed.

### Migration from the original `neo:fs`

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

## Network downloads

`neo:fetch` provides one synchronous function, `download(url, path, options?)`.
It requires `--allow-net`; without that permission the module is absent and an
import fails. Every download also requires `--allow-fs`, because it writes the
destination. JavaScript argument validation, including any option getters or
proxy traps, can run or throw before the native filesystem permission check.
Once validation passes, missing `--allow-fs` produces `FETCH_PERMISSION`;
the download itself does not parse the URL, invoke progress, or perform file
or network I/O before this check. A relative destination path is resolved
against the runtime's current working directory, not the host process
directory. The native callback is private to the module; there is no
`__neoFetch*` global.

```js
import { download } from "neo:fetch";

const result = download("https://example.test/release.tar.gz", "release.tar.gz", {
  maxBytes: 32 * 1024 * 1024,
  timeoutMs: 60_000,
  overwrite: true,
  progress(received, total) {
    // total is a number when Content-Length is known, otherwise null.
  },
});
console.log(`saved ${result.bytes} bytes with status ${result.status}`);
```

`--allow-net` is broad outbound network authority under the current user's
network access, not a sandbox or an allowlist. Loopback, link-local, private,
and cloud-metadata addresses are reachable. A redirect may change the host,
port, or scheme, including an HTTPS-to-HTTP downgrade. There is no IP
filtering, DNS pinning, redirect-host restriction, or SSRF protection. If
untrusted input chooses the URL, enforce an allowlist or isolate the process at
the OS layer; do not rely on this module to do it.

`download` blocks the JavaScript thread until the body is received and
published. There is no asynchronous form and no streaming API. Transport and
file work run on a joined worker thread, so a host may call it from a Tokio
runtime; progress callbacks still run synchronously on the calling thread.
At most 32 download workers are active process-wide. Further calls fail with
`FETCH_NETWORK` rather than waiting, including recursively nested downloads;
the limit avoids deadlock and unbounded worker creation.

### URL, transport, and status

Only absolute `http` and `https` URLs with a host and no userinfo are accepted;
other schemes and any userinfo are rejected with `FETCH_INVALID_URL` before a
connection. URLs are checked again on every redirect, including empty `@` and
empty-password forms, before the next request. This is deliberate: a client could
otherwise turn URL userinfo into an `Authorization: Basic` header. The request
is a `GET`; besides transport defaults (`Host`, `User-Agent`, and `Accept`), no
headers are sent. There is no `Authorization` header, cookie jar, proxy, or
automatic decompression; environment proxy discovery is explicitly disabled.
The URL is never logged or included in an error message. neo-js follows at
most five redirects; a chain that reaches the limit reports `FETCH_REDIRECT`.
A final status outside 2xx reports `FETCH_STATUS` with the numeric status in
`error.status`; the response body is not consumed or saved. The result's
`status` is the final status.

HTTPS uses rustls with the bundled Mozilla webpki roots and certificate-chain
and hostname verification enabled; TLS 1.2 and TLS 1.3 are supported. There is
no switch to disable verification, so a TLS-intercepting proxy whose
certificate does not chain to a bundled root causes the request to fail; the
host's own trust store is not consulted. DNS resolution and connection time are
bounded by the same overall timeout on a best-effort basis; an OS resolver can
still block.

### Options and bounds

| option | default | accepted range |
| --- | --- | --- |
| `maxBytes` | 64 MiB | integer 0 through 512 MiB |
| `timeoutMs` | 30,000 | integer 1 through 600,000 |
| `overwrite` | `false` | boolean |
| `progress` | none | function |

Omitting `options` uses the defaults. For an explicit options object, only
own properties count; inherited or `undefined` options use their defaults.
Own getters can run during validation, as can traps on a proxy options object.

`timeoutMs` covers the network operation, including connection, redirects,
header reads, body reads, and time spent inside the progress callback; it does
not cover the final local sync or publication. `maxBytes` applies both to a
declared `Content-Length` (checked before the body is read)
and to the bytes actually streamed. A body that exceeds the bound reports
`FETCH_LIMIT`; an over-limit body is not written past the bound and temporary
cleanup is attempted. Because automatic decompression is disabled, the
limit and the saved bytes describe the response body as received; a server
that applies `Content-Encoding` without being asked yields the encoded bytes.
A response that ends before its declared `Content-Length` reports
`FETCH_TRUNCATED`. Non-integer, out-of-range, or wrongly typed options throw
`TypeError`.

### Destination and atomic publication

`path` must name a file with an existing parent directory; relative paths use
the runtime working directory, not the host process directory, and report
`FETCH_CWD` when that directory is unavailable. Missing parents are not created
and report `FETCH_PATH`. The body is streamed to a newly created private temp directory
(`.neo-download-*.tmp`, mode `0700` on Unix) in the destination's parent
directory, into a file opened with mode `0600` (both modes are subject to the
process umask). neo-js keeps the open file handle and checks that the pathname
still names that file before publication and cleanup. The temp directory
is also identity-checked before removal (device/inode on Unix, a same-file
handle on other platforms; Windows is unverified), and removal is
non-recursive. The file is synchronized with `sync_all` and then published. The destination is never
written directly, so a status error, size limit, truncated body, callback
failure, or local write error leaves an existing destination untouched on
normal error paths, assuming trusted paths and callbacks as described below.
Temporary cleanup is best-effort: if removal fails, a temporary directory
and possibly its file can remain; the original download error is preserved.
The destination directory
itself is not synchronized, so durability of the directory entry across a
power loss is filesystem-dependent.

With `overwrite: false`, an existing destination is rejected before the
network request and publication uses a no-clobber hard link. That is atomic on
Unix and is the intended Windows behavior (unverified); it also stays on the
same filesystem because the temporary directory is in the destination
directory. A filesystem without hard-link support reports `FETCH_IO` and
leaves the destination untouched. With `overwrite: true`, publication is a
rename that replaces the destination atomically on Unix; Windows behavior is
unverified. On Unix the destination receives the temporary file's mode, so a
new download is private (`0600` before umask) and replacing an existing
destination does not preserve its old permissions. Windows ACL inheritance is
unverified. The preflight existence check is advisory; the no-clobber publish
is authoritative, so a destination that appears during the download is not
replaced when `overwrite` is false. When `overwrite` is true, another writer
can be replaced.

The parent directory and its path must be trusted against modifications by
other users and same-user processes, including progress callbacks. A user
able to modify the parent can rename or replace even a `0700` directory.
Identity checks may detect substitution, but on all platforms they are
separate from hard-link, rename and removal operations. Substitution can race
with publication, changing the destination, or with cleanup, deleting a
replacement. Portable standard-library APIs do not exclude these races.
An abrupt process termination can leave a `.neo-download-*.tmp` directory; it
is never the destination and never replaces one.

### Progress

`progress(received, total)` is called once with `received` `0` before the body
is read and again after each chunk with the cumulative bytes received. `total`
is the declared `Content-Length` or `null` when it is unknown. The callback is
synchronous and runs on the JavaScript thread. If it throws, the download
aborts with `FETCH_CALLBACK` instead of propagating the original exception.
With permissions granted, a direct `async` function throws `FETCH_CALLBACK_ASYNC`
before invocation or I/O. Bound or wrapped callbacks can run before their
return value is checked. Returning a Promise or thenable also throws
`FETCH_CALLBACK_ASYNC` synchronously. A returned Promise is marked handled
without awaiting or cancelling its work. For other objects and functions, a
callable data property `then` (own or inherited) is rejected. A `then` accessor
or a proxy encountered during this lookup is also rejected without running a
getter or trap. Other return values are ignored.

On callback failure, `download` does not publish the destination. It attempts
temporary cleanup and joins the worker before throwing the original download
error, even if cleanup fails. This does not roll back side effects of the
callback itself.
A callback may start another `download`; each call has independent temporary
state. Callback time counts against `timeoutMs`; the deadline is checked after
every callback and before publication. Callbacks must be trusted not to modify
the destination, parent path, or temporary entries.

The TypeScript declaration rejects inferred Promise and thenable returns,
but TypeScript allows a value-returning function to be assigned to `() => void`.
For example, annotating options as plain `DownloadOptions` can hide a Promise
return from the checker. The runtime check still applies.

### Errors

Wrong JavaScript argument types throw `TypeError`. Other failures throw an
`Error` with a stable `code`:

| code | meaning |
| --- | --- |
| `FETCH_PERMISSION` | `--allow-fs` was not granted |
| `FETCH_INVALID_URL` | not an accepted `http`/`https` URL with a host and no userinfo |
| `FETCH_OPTION` | an option value failed native validation |
| `FETCH_STATUS` | the final status was outside 2xx; `error.status` is set |
| `FETCH_REDIRECT` | the redirect limit was reached |
| `FETCH_TIMEOUT` | the overall timeout expired |
| `FETCH_NETWORK` | DNS, connection, TLS, protocol, or body-read failure |
| `FETCH_IO` | local temporary-file, sync, link, or rename failure |
| `FETCH_LIMIT` | the body exceeded `maxBytes` |
| `FETCH_TRUNCATED` | the body ended before its declared length |
| `FETCH_EXISTS` | the destination exists and `overwrite` is false |
| `FETCH_PATH` | the destination path or parent is invalid |
| `FETCH_CWD` | the runtime working directory is unavailable for a relative destination |
| `FETCH_TEMP` | the temporary pathname no longer names the open file |
| `FETCH_CALLBACK` | the progress callback threw |
| `FETCH_CALLBACK_ASYNC` | the progress callback is async, returned a Promise or thenable, or returned a value whose `then` could not be safely inspected |

Messages are fixed and never include the URL, a credential, a path, or
server-provided text. The module adds no credentials and starts no shell.

## Windows registry

`neo:winregistry` provides synchronous Windows Registry access. It requires
`--allow-registry`; without that permission the module has no exports and an
import fails. On Windows the permission grants full read, write, and delete
authority under the current process token. It is not a sandbox: a script can
change any key or value the user can, including keys that run programs at
sign-in. Grant it only to scripts you trust.

```js
import {
  createKey,
  readString,
  writeString,
  writeUint32,
} from "neo:winregistry";

createKey("HKCU", "Software\\MyApp");
writeString("HKCU", "Software\\MyApp", "Theme", "dark");
writeUint32("HKCU", "Software\\MyApp", "LaunchCount", 3);
console.log(readString("HKCU", "Software\\MyApp", "Theme"));
```

The API is a set of free functions, not key objects. Every call opens the key
it needs and closes that handle before returning, so there is no handle to
close and no handle finalizer to rely on. `createKey` reports whether it
created a new key. Writes and value deletes require an existing key; they never
create one. `deleteKey` removes one key and its values but not its subkeys.
`keyExists` returns `false` only when opening the key reports that it does not
exist; a key that exists but cannot be opened raises `WINREGISTRY_ACCESS_DENIED`
instead of a misleading `false`.

Hive names are explicit: `HKCR`, `HKCU`, `HKLM`, `HKU`, `HKCC`, or the full
`HKEY_*` names, matched case-insensitively. There is no default hive. Paths are
relative to the hive and use backslash as the only separator. The empty path
names the hive root. A path may not contain an empty, `.` or `..` component, a
NUL, more than 16,384 UTF-16 units in total, or more than 255 units in one
component. A value name may be empty, which selects the key's default value; it
otherwise may not contain a NUL or exceed 16,383 UTF-16 units. Forward slash is
an ordinary character in a key name, not a separator. Every string argument
must be well-formed UTF-16; an unpaired surrogate throws
`WINREGISTRY_ENCODING` rather than being replaced.

`readString` accepts `REG_SZ` and `REG_EXPAND_SZ` and does not expand
environment references. `readUint32` accepts only `REG_DWORD`, `readUint64`
only `REG_QWORD`, and `readBinary` only `REG_BINARY`. A different type throws
`WINREGISTRY_TYPE_MISMATCH`; no helper converts between types.
`readValue` returns the raw numeric type and exact bytes, including a string's
trailing UTF-16 NUL, so no bytes are lost, and it returns any numeric registry
type unchanged, including types this module has no helper for. A typed string
must be valid, NUL-terminated UTF-16LE with no data after the terminator; an
odd length, a missing terminator, data after it, or an unpaired surrogate is
rejected instead of being truncated or replaced. A value read first asks for
its required size and exact type, rejects a size above 8 MiB before allocating,
then allocates exactly that size. If the value grows between calls, it grows
only on `ERROR_MORE_DATA`, up to the same bound and a bounded number of retries.

`writeString` stores `REG_SZ`, `writeExpandString` stores the unexpanded
`REG_EXPAND_SZ` text, `writeUint32` stores four little-endian bytes,
`writeUint64` stores eight little-endian bytes, and `writeBinary` stores the
exact bytes. An embedded NUL in a string, a DWORD outside 0 through
4,294,967,295, a QWORD outside 0 through 18,446,744,073,709,551,615, or a value
above 8 MiB is rejected. There is no raw write for other registry types and no
`REG_MULTI_SZ`, `REG_NONE`, `REG_LINK`, or resource-list helper.

Enumeration is bounded. `listSubKeys` and `listValues` return at most 4,096
entries by default and accept an explicit integer limit from 1 through 4,096.
They ask the key for its entry counts and maximum name lengths first. A count
above the limit throws `WINREGISTRY_LIMIT` before any entry is fetched.
`listValues` queries only each value's name and numeric type; it does not fetch
value data. Name buffers are sized from the reported maximum and grown only on
`ERROR_MORE_DATA`, up to the documented caps and a bounded number of retries.
If the limit is reached, one additional index is checked and
`WINREGISTRY_LIMIT` is thrown if an entry is found. Enumeration is not an
atomic snapshot: concurrent additions or deletions can cause missed or
duplicated entries, an error, or changes after the lookahead. A returned registry name that is not valid UTF-16 throws
`WINREGISTRY_ENCODING`.

Before a host call, the JavaScript wrapper checks the UTF-16 length of each
primitive string against these caps, so an oversized path, value name, or
string value is not converted to a native Rust string. It rejects proxies and
boxed or other non-primitive objects without reading their properties. Native
validation runs again after conversion.

On a non-Windows host, `isAvailable()` is `false` and every operation throws
`WINREGISTRY_UNSUPPORTED`; no operation fakes success, `false`, or an empty
list. Argument validation still runs first, so an invalid hive or path reports
its own invalid-argument code. Without `--allow-registry`, the module and all
of its functions are absent on every host.

A wrong JavaScript argument type throws `TypeError`. A limit outside 1 through
4,096 or a DWORD outside its range also throws `TypeError`; the APIs never
coerce a value. Other failures throw an `Error` with a stable `code`:
`WINREGISTRY_INVALID_HIVE`, `WINREGISTRY_INVALID_PATH`,
`WINREGISTRY_INVALID_NAME`, `WINREGISTRY_INVALID_VALUE`,
`WINREGISTRY_ENCODING`, `WINREGISTRY_TYPE_MISMATCH`,
`WINREGISTRY_NOT_FOUND`, `WINREGISTRY_ACCESS_DENIED`, `WINREGISTRY_LIMIT`,
`WINREGISTRY_UNSUPPORTED`, or `WINREGISTRY_IO`. Error messages are fixed text
and never include a path, value name, or value data.

The process's default registry view is used; WOW64 32-bit and 64-bit views
cannot be selected. There is no ACL or security-descriptor access, no
transactional update, no recursive delete, no key rename or copy, no change
notification, no key metadata or last-write time, and no remote registry
support. Windows-specific tests, which run only on Windows hosts, create
uniquely named temporary keys under `HKCU\Software\neo-js-tests-*` and delete
them with a drop guard.

## TypeScript

For editor completion and offline type checking, open the
[typed script example](examples/typed-script/) in VS Code or another
TypeScript language server. Its `tsconfig.json` loads the checked-in
`types/neo-js.d.ts` declarations without installing a package.

A `.ts` or `.mts` entry, and any relative `.ts` or `.mts` import, is stripped
to JavaScript before QuickJS sees it. Stripping erases type syntax; it does
not check types and does not compile anything. A `.js` or `.mjs` file is
evaluated byte for byte, so type syntax in it is a syntax error, as before.

Erased: annotations, `interface`, `type`, generic parameters and arguments,
`as`, `as const`, `satisfies`, `<T>expr`, non-null assertions, `implements`,
`abstract` members, `declare` fields, access modifiers, overload signatures,
`import type`, `export type`, inline `type` specifiers, and ambient
declarations such as `declare namespace` and `declare enum`. An import whose
specifiers are all `type` is erased whole, so a module that only holds types
does not have to exist at run time. Value imports, side-effect imports, and
their specifiers are kept, so a `.ts` specifier reaches the loader unchanged.
A type-only import whose name is also declared in the same module is a
`TS_SEMANTIC` error, as in TypeScript, and it is reported before anything runs
rather than hidden by the erasure.

Rejected with a `TS_UNSUPPORTED` diagnostic instead of being transformed:
`enum` and `const enum`, `namespace` and `module` declarations with a body,
constructor parameter properties, decorators, `import x = ...`, `export = ...`,
and `export as namespace`. Precompile those with `tsc` (targeting ES2022 or
later) and run the output as JavaScript.

Diagnostics are one line each, `<path>:<line>:<column>: <message> [<code>]`,
with code `TS_PARSE`, `TS_SEMANTIC`, `TS_UNSUPPORTED`, `TS_TRANSFORM`, or
`TS_EXTENSION`, and are reported before any of the module runs. Only the first
is reported. Early semantic errors, such as a duplicate lexical declaration, are
reported before any module code runs.

Output is regenerated from the parsed module. Formatting, blank lines, and a
comment that sat only on an erased node are not preserved, and no source map is
produced: a stack trace names the generated JavaScript position, not the
original TypeScript. `.cts` and `.tsx` modules and `.d.ts` declaration files
are rejected; JSX is not supported.

neo-js pins Oxc 0.81.0, the newest Oxc release whose full dependency graph
compiles on Rust 1.87. `Cargo.lock` holds two transitive dependencies back for
the same reason: `textwrap` 0.16.2 (0.16.4 needs Rust 1.88) and
`oxc_sourcemap` 4.1.0 (4.2 needs Rust 1.89). Keep both pins when updating.

## Test scripts

`neo:test` registers tests and `neo:assert` supplies assertions. Neither needs
a permission flag, and neither grants any host capability. The CLI runs every
registered test after the entry module settles; importing `neo:test` is enough,
and calling `run()` yourself is optional.

```js
import test from "neo:test";
import { deepEqual, equal, throws } from "neo:assert";

test("adds numbers", () => {
  equal(1 + 1, 2);
});

test("parses nested data", () => {
  deepEqual(JSON.parse('{"a":[1,2]}'), { a: [1, 2] });
});

test("rejects invalid JSON", () => {
  throws(() => JSON.parse("{"), SyntaxError);
});
```

With the default logger and no failures, output is one `ok` line per test and
the process exits 0. With the default logger, a failing test prints `not ok`
with the assertion message and, for `deepEqual`, the first differing path. If
the second test above compared
`[1, 3]` with `[1, 2]`, it would report:

```text
ok 1 - adds numbers
not ok 2 - parses nested data
  AssertionError [ERR_ASSERTION]: values are not deeply equal: values differ at $.a[1]
  actual: 3
  expected: 2
ok 3 - rejects invalid JSON
# 3 tests: 2 passed, 1 failed
neo-js: 1 of 3 test(s) failed
```

Failure semantics:

- Tests run synchronously, in registration order, in the entry module's single
  global context. A test that throws is recorded and the run continues, so one
  failure cannot hide the others.
- The CLI exits non-zero when any executed test failed. Tests registered before
  an uncaught module error still run, and both failures are reported. A runner
  error or inconsistent host counts is also a failure, never a silent pass.
- `run()` is available for a script that wants results immediately. It returns
  this run's counts and throws `TestFailureError` (`code: "NEO_TEST_FAILURES"`)
  after printing. Catching that error does not make the process succeed: the
  module retains every failure, and the CLI still exits non-zero.
- Async work is not supported. `neo:test` rejects a directly supplied async
  function, generator function, or callable proxy at registration with
  `TypeError` (`NEO_TEST_ASYNC`). `neo:assert` applies the same checks to
  callbacks passed to `throws` or `doesNotThrow`. A bound function or other
  wrapper may not expose the underlying function's kind; it is called normally
  and its return value is checked.
- Both modules reject native Promises and generator objects (including async
  generators) regardless of changes to `then`, prototypes, or
  `Symbol.toStringTag`. A directly returned Promise is marked handled by the
  engine; the synchronous error reports the unsupported return, not its
  rejection reason. The guard does not await, advance generators, or drain
  jobs. It does not track async work that the callback starts but does not
  return.
- A callable `then` data property also makes a return unsupported. The check
  walks the `then` lookup without invoking getters or proxy traps: it rejects
  a `then` accessor even if its getter would return a non-function, a returned
  proxy (including revoked proxies), and a proxy reached in the prototype
  chain before finding a `then` property. It does not unwrap proxies.
  Unsupported returns fail `neo:test` with `NEO_TEST_ASYNC` and throw a
  `TypeError` from `throws` or `doesNotThrow`. TypeScript can reject inferred
  Promise, thenable, and generator return types, but cannot identify proxies,
  distinguish accessors from data properties, or recover erased return types
  such as `void` or `unknown`.
- Registering a test from inside a running test throws `NEO_TEST_DURING_RUN`.
  At most 10,000 tests may be registered in one runtime; further registrations
  throw `NEO_TEST_LIMIT`. Names need not be unique.
- `neo:assert` provides `equal`/`notEqual` (strict `Object.is` comparison,
  where `NaN` equals `NaN` and `-0` differs from `0`), `deepEqual`/
  `notDeepEqual`, `ok`, `fail`, `throws`/`doesNotThrow`, and the
  `AssertionError` class (`code: "ERR_ASSERTION"`). `deepEqual` compares own
  enumerable properties plus array length and the internal state of dates,
  regular expressions (including `lastIndex`), boxed primitives, Maps, Sets,
  errors, buffers, and typed arrays; functions, Promises, WeakMaps, WeakSets,
  and WeakRefs compare by identity. Cyclic values are detected instead of
  recursing forever, but this is not a full graph-isomorphism comparison.
  Getter properties are read, and a getter that throws aborts that assertion
  with the getter's error. Recursion is limited by the JavaScript stack rather
  than a count.
- Runner output uses the current `globalThis.console.log` captured when
  `neo:test` initializes, or a no-op if it is not a function. Replacing it
  before a dynamic import can redirect or suppress test details; replacing it
  afterward does not change the captured logger. Counts and failure summaries
  are recorded through host-owned callbacks, so suppressing that output does
  not change the host failure count or the CLI's non-zero exit for a failed
  test. Replacing `JSON.stringify`, mutating `Array.prototype.push`/`splice`,
  installing an inherited array index setter, or adding an inherited
  `Object.prototype.toJSON` after import cannot skip tests or hide failures
  from the host report.
- This is registration-based, not test discovery: only modules evaluated in
  this runtime can register tests. Tests are not isolated from each other, have
  no timeout, retry, or parallelism, and share globals and mutable state. A
  callback that never returns hangs the process. A test can still corrupt
  intrinsics and break later tests or assertion output; the runner protects its
  own bookkeeping, not the whole realm. Do not treat tests as a sandbox for
  untrusted code.

When embedding the library, call `NeoJs::run_pending_tests()` after evaluating
the entry module. It returns `None` when `neo:test` was never imported, or a
cumulative `TestReport`; it does not print beyond what the module writes to
`console` and does not choose a process exit status.

## Embed the runtime

```rust
use neo_js::{NeoJs, Permissions};

let runtime = NeoJs::new(Permissions::default())?;
runtime.run("console.log('hello')")?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

`NeoJs::run_module(name, source)` evaluates the supplied source, stripping names
ending in `.ts` or `.mts`. Without filesystem permission, the name is used
verbatim and no filesystem lookup is made, so a virtual entry still runs if it
imports no relative path. With permission, an existing name is canonicalized
relative to the runtime working directory; if it cannot be canonicalized it
remains a virtual name, and a relative entry name is an error when that
directory is unavailable. The absolute directory for a virtual entry's relative
imports is captured when the entry is declared, so a later `chdir` does not move
its base. Relative dependencies resolve from that captured directory and must
exist; they require `Permissions::filesystem`, exactly like `.js`. Supplying
source that differs from an existing file still gives it that file's canonical
identity. If canonicalization succeeds but yields a non-UTF-8 path, it is an
error, not a virtual name. `run_module_with_identity` lets a host that already
resolved an entry provide its format name separately from its identity and
base; an absolute identity supplies its own base, while a relative one is
resolved against the runtime directory when declared. If the directory is
unavailable at declaration, the entry still runs but its relative imports have
no captured base and continue to report unavailable after a later `chdir`.
Recover with a new runtime, or declare the source under a distinct identity
after establishing a directory; an identity already declared as an entry cannot
be reused. It performs no filesystem lookup itself and does not grant
filesystem permission.
Both entry APIs reject `neo:` names and identities, which are reserved for
host modules. An identity already declared as an entry or dependency cannot
be supplied as a new entry, even if its evaluation failed; this avoids
conflicting module definitions.

`neo_js::third_party_notices()` returns the third-party notice text embedded in
the library; it performs no I/O and needs no permission. The CLI prints the
same text for `--third-party-notices`. It covers adapted source code (currently
the `neo:path` algorithms), not every dependency, and is not a complete
dependency-license inventory.

## Compatibility

neo-js is pre-1.0. Adding the `process` field to `Permissions` was a
source-breaking change for code that constructed the struct with a complete
literal. Adding `registry` is the same kind of change, and so is adding
`network`; adding `stdio` is the same kind of change. Migrate by setting the
fields you need and adding `..Permissions::default()`, or by naming every field
explicitly, including `process`, `stdio`, `registry`, and `network`. CLI users
opt in with `--allow-net` and `--allow-stdio`. Adding `neo:exec.spawn` is
additive: the module exports one more function, existing `exec` calls and
permissions are unchanged, and the host installs two additional host
callbacks only when `process` is granted. `neo:semver` is additive for its
documented exports: `valid` and `compare` keep their behavior, and `parse`,
`format`, `increment`, `satisfies`, `validRange`, and `SemVerError` are new.
Its former `__neoSemverValid` and `__neoSemverCompare` globals were private,
never documented, and are no longer installed; import the module instead. It
also imposes the documented input limits: an oversized version makes `valid`
return `false` and `compare` throw `SEMVER_LIMIT`, where the previous
implementation had no bound.
Pre-1.0 releases may make further
source-breaking changes; no compatibility guarantee is implied.

## Limits

This is a first implementation, not an isolation boundary. It does not limit
CPU or memory use. Do not run untrusted scripts without OS-level isolation.
The runtime has no external event loop. TypeScript support erases types only:
there is no type checking, constructs that need a transform are rejected, and
no source map is produced. Every host API, including `neo:test` and
`neo:assert`, is synchronous; there is
no asynchronous counterpart to await, and a Promise returned to a synchronous
API is rejected or treated as a failure rather than silently left pending.
`neo:toml` and `neo:yaml` parse text only: neither has a file loader, expands
no alias or variable, and reads no environment.
Test support is registration-based: it does not discover test files, isolate
tests, enforce a timeout, or run tests in parallel. Process support is limited
to the synchronous, buffered `exec` and `spawn` calls of `neo:exec` and the
synchronous `neo:process` stream and working-directory API; there are no
asynchronous or streaming child process APIs. Spawned-child handles hold one
slot each until their terminal call or runtime drop, and their results are
buffered rather than streamed. Reading stdin blocks with no timeout, and
`neo:process` stream `close()` is a no-op. The runtime working directory is local to a `NeoJs`
instance and is not a sandbox. Windows registry support is limited to the
synchronous `neo:winregistry` functions described above and exists only on a
Windows host. `neo:path` is lexical: runtime cwd fallback requires a
qualifying grant and module allowlist entry. It does not resolve symlinks
or enforce containment, and has no file-URL or glob helpers. `neo:dotenv`
parses text only: it has no file loader, no expansion,
and no environment mutation. Network support is limited to the synchronous
`neo:fetch.download` call: there is no asynchronous or streaming API, no
authentication, cookie, proxy, decompression, resume, or content-type support,
and no SSRF filtering. `neo:scheduler` generates text only: it does not write,
install, enable, or run a generated unit or task, does not provide a scheduler
daemon, and supports only the daily and weekly forms described above. Most
other requested `neo:` modules are not implemented yet.
`neo:semver` follows the Rust `semver` crate: a core component is limited to
`u64::MAX`, and ranges are Cargo-style intersections without `||`, hyphen
ranges, or npm x-range defaults. Numeric prerelease identifiers remain
arbitrary-precision strings; there is no npm `includePrerelease` option.
`neo:crypto` provides only symmetric authenticated encryption; it has no key
store, key derivation, password-based encryption, or public-key operation.
`neo:age` provides X25519 age file encryption but no key store, passphrase,
SSH-key, plugin, or post-quantum recipient support.

Run `cargo test --workspace` to test the workspace.
