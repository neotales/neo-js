---
title: "Host OS metadata"
description: "How neo-js obtains target and operating-system metadata."
---

The optional `neojs-os` crate supplies `neo:os` and the `os` global. The CLI
links and registers it, so both are available without a permission flag.
`neojs-core` alone provides neither. A custom embedder must register the
extension and admit the exact name `neo:os` in any explicit allowlist.
Registration alone does not override an allowlist. The module and global
disclose basic host operating-system metadata.

`platform` and `arch` are Rust's compilation
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
console.log(platform, arch, version ?? "", codename ?? "", variant ?? "");
```

## Embed OS metadata

For a checkout next to your crate, add both dependencies:

```toml
[dependencies]
neojs-core = { path = "../neo-js/crates/neojs-core" }
neojs-os = { path = "../neo-js/crates/neojs-os" }
```

```rust
use neojs_core::{NeoJs, Permissions, RuntimeOptions};
use neojs_os::{MODULE_NAME, with_os};

fn main() -> Result<(), Box<dyn std::error::Error>> {
	let options = with_os(RuntimeOptions::new())?.module_names([MODULE_NAME])?;
	let runtime = NeoJs::new_with_options(Permissions::default(), options)?;
	runtime.run_module(
		"main.js",
		"import {platform} from 'neo:os'; if (platform !== os.platform) throw Error('os');",
	)?;
	Ok(())
}
```

`neojs_os::with_os` registers the exact name with `ExtensionPermission::None`.
Use `neojs_os::os_extension()` when assembling a registry yourself. If the
runtime admits `neo:os`, the init hook takes one private snapshot and installs
the writable global before user code. If the runtime omits the name, it does
neither. The named exports always use the snapshot, not the mutable global.

Replace the removed `RuntimeModule::Os` in explicit allowlists with the
registered name `neojs_os::MODULE_NAME`. `ModuleAllowlist::all()` covers
built-ins only. To admit all registered extensions as well, use
`ModuleAllowlist::all_with_extensions(options.extensions())`. Unrestricted
`RuntimeOptions::default()` admits registered modules, subject to their
permissions. It does not register modules on its own.

The [scheduler modules](/guides/modules/scheduler/) come from the separate
optional `neojs-schedulers` crate. They do not depend on `neojs-os`, and
their exact allowlist entries are separate from `neo:os`.

Use [the matching declaration composition](/runtime/typescript/#select-declarations)
for your host. For a host that registers only `neojs-os`, compose with
`--with os`. Core-only declarations omit `neo:os` and `os`. The CLI
composition includes both from `crates/neojs-os/types/neo-os.d.ts`.

Declarations: [`neo:os`](/api/modules/os/).
