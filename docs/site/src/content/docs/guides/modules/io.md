---
title: Synchronous in-memory byte I/O
description: Read and write bounded bytes with the optional neo:io extension.
---

`neo:io` provides a synchronous byte reader and writer over in-memory storage.
It is an optional crate, `neojs-io`; `neojs-core` does not compile it and the
CLI does not link it. Register the extension and name the exact module in an
explicit allowlist:

```rust
use neojs_core::{NeoJs, Permissions, RuntimeOptions};
use neojs_io::{MODULE_NAME, with_io};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = with_io(RuntimeOptions::new())?.module_names([MODULE_NAME])?;
    let _runtime = NeoJs::new_with_options(Permissions::default(), options)?;
    Ok(())
}
```

The module needs no permission flag. It does not touch files, directories,
sockets, processes, environment variables, or the registry. It is not a
Node.js stream or an `EventEmitter`: there are no events, listeners, promises,
`pipe`, `on`, encodings, backpressure, or `Buffer` values.

## Read a bounded snapshot

`byteReader` copies the current view of its `Uint8Array`. Later mutation,
detaching, or resizing of the caller's buffer does not change the reader.

```js
import { byteReader } from "neo:io";

const reader = byteReader(Uint8Array.from([1, 2, 3]));
const target = new Uint8Array(2);
try {
	for (;;) {
		const count = reader.read(target);
		if (count === 0) break; // end of file, not an exception
		console.log(target.subarray(0, count));
	}
} finally {
	reader.close();
}
```

`read` copies at most `target.byteLength` bytes and returns the number copied.
It returns exactly `0` at end of file. An empty target throws `IO_ARGUMENT`
rather than returning an ambiguous zero. A closed reader throws `IO_CLOSED`
before it validates the target.

## Write owned bytes

`byteWriter` owns a growable buffer. `toUint8Array` returns a fresh copy that
the caller owns; it never exposes the internal buffer.

```js
import { byteWriter } from "neo:io";

const writer = byteWriter();
try {
	writer.write(Uint8Array.from([0x68, 0x69]));
	writer.write(Uint8Array.from([0, 255]));
	const bytes = writer.toUint8Array(); // caller-owned copy
	console.log(bytes.join(","));
} finally {
	writer.close();
}
```

`write` appends the whole input or throws `IO_LIMIT` without changing the
writer. A closed writer throws `IO_CLOSED` before it validates the input.
The returned `Uint8Array` is JavaScript-owned and is outside the module's
native byte budget after return.

## Limits, errors, and cleanup

| Limit               | Value  | Scope                                                            |
| ------------------- | ------ | ---------------------------------------------------------------- |
| `MAX_BYTES`         | 8 MiB  | One reader or writer, and one `write` call.                      |
| `MAX_RUNTIME_BYTES` | 16 MiB | Native bytes across every live reader and writer in one runtime. |
| `MAX_OBJECTS`       | 32     | Live reader and writer objects in one runtime.                   |

Reader input and writer content count against both the per-object and runtime
limits. `toUint8Array` also momentarily charges its copy; it can report
`IO_LIMIT` when that copy would exceed the runtime aggregate.

`close()` releases the stored bytes and is idempotent. A closed object keeps
its object slot until it is collected or the runtime is dropped, so `close()`
returns the bytes to the budget immediately while the object slot becomes
available only after collection or teardown. Ordinary unreachable objects
release their slot immediately; cycles need collection; runtime teardown
releases everything.

Failures throw a native `Error` with `name = "IoError"` and a stable `code`:
`IO_ARGUMENT`, `IO_LIMIT`, `IO_CLOSED`, `IO_DETACHED`, `IO_SHARED`, or
`IO_RESIZABLE`. Messages are static and never include the rejected bytes.
Match the code, not the message. A method or getter called with a receiver
that is not an object from this module throws the engine's native `TypeError`,
not an `IoError`.

Inputs must be genuine `Uint8Array` views over attached, fixed-length,
non-shared `ArrayBuffer` storage. Detached, shared, resizable, forged, and
proxied values are rejected. Inputs are checked through the engine's internal
typed-array brand and captured accessors, not through mutable globals or
prototype properties. This is not a sandbox or a general memory bound for the
rest of the runtime; it bounds only the native storage this module owns.

For TypeScript, compose the `neojs-io` fragment with `--with io`. See
[declaration selection](/runtime/typescript/#select-declarations).
