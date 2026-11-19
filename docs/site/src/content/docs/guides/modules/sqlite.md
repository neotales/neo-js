---
title: In-memory SQLite
description: Use private, synchronous SQLite databases through the optional neo:sqlite extension.
---

`neo:sqlite` runs SQL in the host process. **Use only trusted SQL.** This
module is not a sandbox and has no hard deadline or intermediate-memory cap.
The CLI links `neojs-sqlite`, but denies imports by default. For a trusted
script, run:

```sh
cargo run -p neojs -- --allow-sqlite script.js
```

`--allow-sqlite` is independent of filesystem, process, and network
permissions. The module exposes no API for files, the network, shells,
child processes, or loading SQLite extensions. No combination of flags
enables file-backed databases.

## Open and close a database

```js
import { openMemory } from "neo:sqlite";

using db = openMemory();
db.execute("CREATE TABLE notes(id INTEGER PRIMARY KEY, body TEXT NOT NULL)");
db.execute("INSERT INTO notes VALUES (?, ?)", [42n, "hello"]);
const [[id, body]] = db.query("SELECT id, body FROM notes WHERE id = ?", [42n]);
console.log(`${id}: ${body}`);
```

`openMemory()` takes no arguments and opens a new private database.
`close()` and `[Symbol.dispose]()` roll back any open transaction and discard
the database. Both are idempotent. Use `using` or `try`/`finally` for prompt
cleanup. Runtime teardown releases remaining handles.

`execute(sql, parameters?)` accepts one statement with no result columns
and returns the number of changed rows, including foreign-key actions, as a
bigint. `query(sql, parameters?)` accepts one read-only statement with result
columns and returns owned positional rows. Use `ORDER BY` when order matters.
Only whitespace may follow the terminating semicolon. Both methods use
anonymous `?` placeholders and require exactly that many own data elements
in a dense parameter array.

Bind `null` as NULL, finite numbers as REAL, signed 64-bit bigints as INTEGER,
and well-formed strings as TEXT. BLOB bindings copy genuine `Uint8Array`
views over attached, fixed-length, non-shared buffers. Returned INTEGERs are
bigints and REALs are numbers. Returned TEXT must be valid UTF-8. Each
returned BLOB is a fresh `Uint8Array`. There are no named bindings, reusable
statement handles, multi-statement batches, or writes with `RETURNING`.

## SQL restrictions and limits

Both the database and temporary storage are in memory. The SQLite authorizer
permits a subset of the main schema: tables, indexes, queries, mutations,
transactions, and selected built-in functions. Foreign keys are enforced,
with at most eight levels of cascading actions. The authorizer rejects
attachments, writable schema, pragmas, user-defined triggers, views, virtual
tables, extension loading, and savepoints, including `ROLLBACK TO`.
Expensive string functions, `LIKE`, `GLOB`, `VACUUM`, and `VACUUM INTO` are
not supported. SQLite does not wait for locks.

| Resource                       | Limit                                                 |
| ------------------------------ | ----------------------------------------------------- |
| Live databases                 | 8 per runtime                                         |
| Database pages                 | 8 MiB per database                                    |
| SQL input                      | 64 KiB of UTF-8                                       |
| Parameters or columns          | 64 each                                               |
| Text, BLOB, or SQLite row      | 1 MiB each                                            |
| Total bound text and BLOB data | 4 MiB                                                 |
| Results                        | 1,024 rows or 4 MiB of payload, whichever comes first |

The progress handler interrupts cooperatively at a checkpoint after roughly
200,000 VM operations or 250 ms. Native work can continue between checkpoints.
Database and result limits do not cap the SQLite heap. Sorts, grouping, and
subqueries can allocate substantial workspace before returning a row.
Arbitrary SQL can exhaust host memory.

Script-visible errors use stable `SQLITE_*` codes and omit SQL, paths, and
bound values. The database borrow ends before an error is thrown. QuickJS can
still call stack-trace hooks during propagation. A failed query returns no
partial result. Failed mutations follow SQLite's normal conflict and rollback
semantics. If you install a process-global SQLite log callback, keep its
output private: engine diagnostics can include SQL.

SQLite has no async API. The filesystem read worker does not run SQL.
Native Windows and macOS behavior was not verified for this increment.

## Embed SQLite

`neojs-core` does not register `neo:sqlite` or depend on `rusqlite` or
`libsqlite3-sys`. `neojs-sqlite` owns the registration, the per-runtime
native state, and `rusqlite` with `bundled`, `hooks`, and `limits`.
Registration, exact allowlist admission, and `Permissions::sqlite` are
separate requirements. Missing permission or admission denies import before
module load. The default, unrestricted `RuntimeOptions` admits registered
modules if their permissions are granted.

```toml
[dependencies]
neojs-core = { path = "../neo-js/crates/neo-js" }
neojs-sqlite = { path = "../neo-js/crates/neojs-sqlite" }
```

```rust
use neojs_core::{NeoJs, Permissions, RuntimeOptions};
use neojs_sqlite::{MODULE_NAME, with_sqlite};

fn main() -> Result<(), Box<dyn std::error::Error>> {
	let options = with_sqlite(RuntimeOptions::new())?.module_names([MODULE_NAME])?;
	let runtime = NeoJs::new_with_options(
		Permissions { sqlite: true, ..Permissions::default() },
		options,
	)?;
	runtime.run_module(
		"main.js",
		"import {openMemory} from 'neo:sqlite'; openMemory().close();",
	)?;
	Ok(())
}
```

Replace the removed `RuntimeModule::Sqlite` variant with the registered
exact name `neo:sqlite` (`neojs_sqlite::MODULE_NAME`). Use
`neojs_sqlite::sqlite_extension()` if you assemble the registry yourself.
It declares `ExtensionPermission::Sqlite`. `ModuleAllowlist::all()` admits
built-ins only. To include registered extensions, use
`ModuleAllowlist::all_with_extensions(options.extensions())`, or add the
exact name to your allowlist. This migration keeps the JavaScript API,
permission, default-deny behavior, and SQL restrictions.

For an embedder that registers only `neojs-sqlite`, compose declarations
with `--with sqlite`. The CLI composition also declares `neo:os` and `os`;
use it only when the host registers both extensions. The core-only file
omits `neo:sqlite`. See [Select declarations](/runtime/typescript/#select-declarations).
Declarations never grant permission or override an allowlist.

Declarations: [`neo:sqlite`](/api/modules/sqlite/).
