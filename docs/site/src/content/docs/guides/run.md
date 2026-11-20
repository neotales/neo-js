---
title: Run a script
description: Run a JavaScript or TypeScript entry file with the neo-js CLI.
---

1. Install Rust 1.87 or later.
2. In the checkout root, save the following as `script.js`:

   ```js
   import { compare } from "neo:semver";

   console.log(compare("1.2.0", "1.1.9").toString());
   ```

3. Run the script:

   ```sh
   cargo run -p neojs -- script.js
   ```

   The script prints `1`.

The Cargo package is `neojs`; the executable is `neo-js`. The CLI registers
[`neojs-os`](/guides/modules/os/), so `neo:os` and the `os` global are available
without a flag. The CLI also registers [`neojs-schedulers`](/guides/modules/scheduler/).
Its three scheduler imports need no flag. Importing
[`neo:os/user`](/guides/modules/os-user/) requires `--allow-user`. For a `.ts` or
`.mts` entry file, pass that filename instead. The CLI strips
types but does not check them. See [TypeScript support](/runtime/typescript/).

## Grant host access

The CLI reads the entry file without `--allow-fs`. Other access is opt-in.
For example, to run a script that imports `neo:fs` and `neo:env`:

```sh
cargo run -p neojs -- --allow-env --allow-fs script.js
```

Grant host access only to scripts you trust. See [host permissions](/runtime/permissions/)
for the full list of flags and what each flag allows.

## Read third-party notices

```sh
cargo run -p neojs -- --third-party-notices
```

This standalone mode reads no script and needs no permission flags. The text
covers source adapted for `neo:path`. It is not a complete dependency-license
inventory. The Rust library returns the same text through
`neojs_core::third_party_notices()` without constructing a runtime.
