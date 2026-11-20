# neo-js

neo-js is a Rust library and command-line host for QuickJS. It runs JavaScript
modules and strips types from TypeScript scripts, without type checking.
Built-in `neo:` modules provide utilities and optional access to host resources.

With Rust 1.87 or later, run a script from the checkout root:

```sh
cargo run -p neojs -- script.js
```

The executable is `neojs`. Earlier 0.1 checkouts named it `neo-js`; that name is
no longer installed, so update scripts and wrappers to run `neojs`. Rust
embedders use `neojs-core`.

The [documentation site](docs/site/) contains the usage guides, embedding
instructions, and generated API reference. Review
[host permissions](docs/site/src/content/docs/runtime/permissions.md) before
granting access. neo-js is not a sandbox.

[`neo:os/user`](docs/site/src/content/docs/guides/modules/os-user.md) is
an optional identity module. The CLI registers it; scripts need `--allow-user`.

[`neo:win-registry`](docs/site/src/content/docs/guides/modules/winregistry.md)
is built into `neojs-core`. It requires `--allow-registry` and supports owned
keys with `using` on Windows. `neo:winregistry` is a deprecated alias.

Run `cargo test --workspace` to test the Rust workspace.
