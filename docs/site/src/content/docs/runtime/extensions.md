---
title: Register a host extension
description: Add static native modules, choose which names a runtime may import, and manage per-runtime state.
---

An extension is trusted native host code, not a sandbox or a plug-in search
mechanism. Register it explicitly before constructing the runtime. Each
descriptor declares exact `neo:` module names, the existing permission that
gates each name, and optional init and teardown hooks.

```rust
use std::any::Any;

use neojs_core::{
	ExtensionDescriptor, ExtensionPermission, NeoJs, Permissions, RuntimeOptions,
	rquickjs::{Ctx, Module, Result as JsResult, module::Declared},
};

fn load<'js>(
	ctx: &Ctx<'js>,
	_state: &mut dyn Any,
	name: &str,
) -> JsResult<Module<'js, Declared>> {
	Module::declare(ctx.clone(), name, "export const answer = 42;")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
	let extension = ExtensionDescriptor::new("demo", load)
		.with_module("neo:demo", ExtensionPermission::None);
	let options = RuntimeOptions::new().with_extensions([extension])?;
	let runtime = NeoJs::new_with_options(Permissions::default(), options)?;
	runtime.run_module(
		"main.js",
		"import {answer} from 'neo:demo'; if (answer !== 42) throw Error('bad answer');",
	)?;
	Ok(())
}
```

## Admission and authority

Registration rejects empty or duplicate extension IDs, malformed names,
duplicate module names, and collisions with names the core still provides.
The registry does not change on failure. Core owns permissions, exact
allowlists, module resolution, and the QuickJS module cache. The resolver
checks both permissions and the allowlist before consulting the cache, for
static imports, dynamic imports, and cache hits.

Registering a loader does not constrain what that Rust code can do with its
`Ctx` or the process's authority. Extensions cannot define new runtime
permission categories. Use `neojs_core::rquickjs` so the native `Ctx`,
`Module`, and `Persistent` types match the runtime's. Hooks must not re-enter
host module loading while the extension's state is borrowed.

## Lifecycle and errors

Init runs once per runtime before user code, in registration order, and only
if at least one declared module passes both gates. The hook receives the
names that passed. Descriptors and registries are `Send + Sync` and hold no
QuickJS values. Init creates native state on the runtime's thread, and that
state stays there until teardown.

Teardown runs in reverse order on drop. It receives no `Ctx` and must not
execute JavaScript. If init fails, core tears down earlier extensions and
construction returns an owned error. A JavaScript exception is captured
without invoking getters or `toString`. Core catches an init panic and
returns a diagnostic naming the extension and panic payload. The process
panic hook still runs. The failing hook must release its own partial state.
A module load failure fails the import without tearing down the runtime.

If a teardown hook or state destructor panics, core still runs the remaining
teardowns and releases its persistent handles. On normal drop it then resumes
the first panic. If an unwind is already in progress, it preserves the
original panic. During init-failure cleanup, teardown panics do not replace
the init error. Core consumes a finite chain of panicking panic-payload
destructors. An intentionally endless chain hangs rather than leaking or
aborting, as in the filesystem worker.

[`neojs_core::extension`](https://docs.rs/neojs-core/latest/neojs_core/extension/index.html)
has the full hook contract and a native `ModuleDef` example. The
[SQLite guide](/guides/modules/sqlite/#embed-sqlite) shows registration of a
separately packaged extension.
