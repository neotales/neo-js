---
title: "Embed the runtime"
description: "Construct a NeoJs instance, select modules, and evaluate source from Rust."
---

`NeoJs::new_with_options` takes both [permissions and a module selection](/runtime/permissions/).
This example admits `neo:hash` and `neo:fetch` without admitting `neo:fs`:

```rust
use neojs_core::{NeoJs, Permissions, RuntimeModule, RuntimeOptions};

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
[RuntimeOptions reference](https://docs.rs/neojs-core/latest/neojs_core/struct.RuntimeOptions.html)
and [Permissions reference](https://docs.rs/neojs-core/latest/neojs_core/struct.Permissions.html)
for the Rust API.

To load optional native modules, see [Register a host extension](/runtime/extensions/).

## Evaluate a module

```rust
use neojs_core::{NeoJs, Permissions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
	let runtime = NeoJs::new(Permissions::default())?;
	runtime.run("console.log('hello')")?;
	Ok(())
}
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

Both module entry APIs block until the managed turn settles or fails. See
[Managed module evaluation](/runtime/loading/#managed-module-evaluation) for
the asynchronous read and Promise limits.

## Compatibility

neo-js is pre-1.0. See [Rust package migration](#rust-package-migration)
for the breaking Cargo package and Rust import rename.

Adding the `process` field to `Permissions` was a
source-breaking change for code that constructed the struct with a complete
literal. Adding `registry` is the same kind of change, and so is adding
`network`, `stdio`, or `sqlite`. Migrate by setting the
fields you need and adding `..Permissions::default()`, or by naming every field
explicitly, including `process`, `stdio`, `registry`, `network`, and `sqlite`.
CLI users opt in with `--allow-net`, `--allow-stdio`, and `--allow-sqlite`.
`RuntimeModule::Sqlite` was removed. To admit `neo:sqlite`, register the
[optional SQLite extension](/guides/modules/sqlite/#embed-sqlite) and grant
`Permissions::sqlite`.

Removing `RuntimeModule::Os` also breaks Rust callers that used the variant.
Register [`neojs-os`](/guides/modules/os/#embed-os-metadata) and use its exact
module name in the allowlist. Core alone provides neither `neo:os` nor `os`.
The CLI registers the extension, so its module, writable global, and private
snapshot exports are unchanged. No permission flag is required.

`RuntimeModule::Scheduler`, `RuntimeModule::SchedulerSystemd`, and
`RuntimeModule::SchedulerWindows` were removed. Register
[`neojs-schedulers`](/guides/modules/scheduler/#embed-scheduler-generators)
and replace the variants with its exact module name constants. The CLI
registers the extension without a permission flag.

Adding `neo:exec.spawn` is
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

The scheduler split grew `RuntimeModule::ALL` from 21 to 23 entries.
Moving `neo:os` out of core left 22 entries. Moving the three scheduler
specifiers to `neojs-schedulers` leaves 19 built-in entries.
Iterate with `RuntimeModule::ALL.into_iter()` rather than declaring an array
with a fixed length. `ModuleAllowlist::all()` admits built-ins only. Use
`ModuleAllowlist::all_with_extensions(options.extensions())` if you also
intend to admit registered extensions. For an explicit scheduler allowlist, see
[the scheduler migration guide](/guides/modules/scheduler/#migrate-from-neoscheduler).

Pre-1.0 releases may make further
source-breaking changes; no compatibility guarantee is implied.

## Rust package migration

| Before                         | After                                  |
| ------------------------------ | -------------------------------------- |
| `neo-js`, imported as `neo_js` | `neojs-core`, imported as `neojs_core` |
| `neo-js-cli`                   | `neojs`                                |

Change workspace package selectors, dependency keys, and Rust imports. There
is no old package alias or `neo_js` re-export. The rename alone did not
change the JavaScript API or the executable name. The CLI's `--help` and
`--version` still identify `neo-js`.

For a checkout next to your crate:

```toml
[dependencies]
neojs-core = { path = "../neo-js/crates/neo-js" }
```

Use `use neojs_core::{NeoJs, Permissions};` in Rust. The extension packages
are optional dependencies, not renames. Add `neojs-sqlite` for `neo:sqlite`,
`neojs-os` for `neo:os` and `os`, or `neojs-schedulers` for the scheduler
generators. Their import roots are `neojs_sqlite`, `neojs_os`, and
`neojs_schedulers`. Core does not link these extensions.

```sh
cargo run -p neojs -- script.js
cargo test -p neojs-core
```
