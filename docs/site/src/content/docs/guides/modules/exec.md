---
title: "Child processes"
description: "Child process execution, buffered output, and handle cleanup."
---

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

## Spawned child handles

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

Declarations: [`neo:exec`](/api/modules/exec/).
