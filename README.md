# neo-js

neo-js is a Rust library and command-line host for QuickJS. It runs JavaScript
modules and strips types from TypeScript scripts, without type checking.
Built-in `neo:` modules provide utilities and optional access to host resources.

With Rust 1.87 or later, run a script from the checkout root:

```sh
cargo run -p neojs -- script.js
```

The executable is `neo-js`. Rust embedders use `neojs-core`.

The [documentation site](docs/site/) contains the usage guides, embedding
instructions, and generated API reference. Review
[host permissions](docs/site/src/content/docs/runtime/permissions.md) before
granting access. neo-js is not a sandbox.

Run `cargo test --workspace` to test the Rust workspace.
