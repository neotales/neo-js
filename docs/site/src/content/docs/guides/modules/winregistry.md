---
title: "Windows registry"
description: "Registry permissions, disposable keys, supported values, and platform limits."
---

`neo:win-registry` provides synchronous Windows Registry access in `neojs-core`.
The deprecated `neo:winregistry` spelling is an alias for the same module and
allowlist entry. No release is scheduled to remove it. Neither spelling
accepts nonempty import attributes. Imports require `--allow-registry` (and
`RuntimeModule::WinRegistry` for embedders with an explicit allowlist). On
Windows the permission grants full read, write, and delete
authority under the current process token. It is not a sandbox: a script can
change any key or value the user can, including keys that run programs at
sign-in. Grant it only to scripts you trust.

## Owned keys

Save this example as `registry.js` and run
`cargo run -p neojs -- --allow-registry registry.js` on Windows. It creates
or opens `HKCU\Software\NeoJsRegistryExample` and writes to it.

```js
import { Registry, Rights } from "neo:win-registry";

const path = "HKCU\\Software\\NeoJsRegistryExample";
{
	using key = Registry.createKey(path, Rights.QUERY_VALUE | Rights.SET_VALUE);
	key.setString("Theme", "dark");
}
using reader = Registry.openKey(path, Rights.QUERY_VALUE);
if (reader.getString("Theme") !== "dark") throw new Error("unexpected theme");
```

`Registry.openKey(path, access?)` and `Registry.createKey(path, access?)`
return a `RegistryKey` owning one native handle. Pass a full path with a
hive and a nonempty subkey. `path` records the supplied full path and
`created` tells whether the call created the final key.

`getString` requires `Rights.QUERY_VALUE` and reads `REG_SZ` or unexpanded
`REG_EXPAND_SZ`; `setString` requires `Rights.SET_VALUE` and writes `REG_SZ`.
The mask must be a nonzero combination of bits in `Rights.ALL_ACCESS`. Open
defaults to `Rights.READ`, and create defaults to `Rights.ALL_ACCESS`. The
access mask limits that handle, not the script's overall registry authority.
Windows still checks permissions and uses the process's default registry view.

`using` calls `[Symbol.dispose]()` when leaving scope, even on an exception.
It is the same idempotent operation as `close()`. Metadata stays readable,
but further I/O throws `WINREGISTRY_CLOSED`. At most 64 owned keys can be
open per runtime across the two spellings; another open throws
`WINREGISTRY_LIMIT` until a slot is freed. The runtime closes any remaining
handles at teardown. No raw handles are exposed. **Closing a key does not
undo writes or remove keys.** The example leaves its key and value in the
registry. `using` also works in TypeScript entries; the shipped declarations
reference `ESNext.Disposable` for type checking.

## Standalone functions

```js
import { createKey, readString, writeString, writeUint32 } from "neo:win-registry";

createKey("HKCU", "Software\\MyApp");
writeString("HKCU", "Software\\MyApp", "Theme", "dark");
writeUint32("HKCU", "Software\\MyApp", "LaunchCount", 3);
console.log(readString("HKCU", "Software\\MyApp", "Theme"));
```

The standalone functions release any handles before returning. `createKey` reports whether it
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
`WINREGISTRY_NOT_FOUND`, `WINREGISTRY_ACCESS_DENIED`, `WINREGISTRY_CLOSED`,
`WINREGISTRY_LIMIT`, `WINREGISTRY_UNSUPPORTED`, or `WINREGISTRY_IO`. Error messages are fixed text
and never include a path, value name, or value data.

The process's default registry view is used; WOW64 32-bit and 64-bit views
cannot be selected. There is no ACL or security-descriptor access, no
transactional update, no recursive delete, no key rename or copy, no change
notification, no native key metadata or last-write time, and no remote registry
support. Windows-specific tests, which run only on Windows hosts, create
uniquely named temporary keys under `HKCU\Software\neo-js-tests-*` and delete
them with a drop guard.

Declarations: [`neo:win-registry`](/api/modules/win-registry/).
