---
title: "Host OS metadata"
description: "How neo-js obtains target and operating-system metadata."
---

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
console.log(platform, arch, version ?? "", codename ?? "", variant ?? "");
```

Declarations: [`neo:os`](/api/modules/os/).
