---
title: "TypeScript support"
description: "Supported type syntax, loader diagnostics, and editor type checking."
---

For editor completion and offline type checking, open the
typed script example at `examples/typed-script/` in VS Code or another
TypeScript language server. Its `tsconfig.json` loads the checked-in
`types/neo-js.d.ts` declarations without installing a package.

## Select declarations

| Declaration input                                   | Use                                                                                      |
| --------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| `types/neo-js.core.d.ts`                            | A core-only embedder. It omits `os` and every optional module.                           |
| `types/neo-js.d.ts`                                 | The CLI build. This generated file combines core and all four extension fragments.       |
| `crates/neojs-os/types/neo-os.d.ts`                 | The `neo:os` and `os` fragment owned by `neojs-os`.                                      |
| `crates/neojs-os-user/types/neo-os-user.d.ts`       | The independently selectable account and process identity fragment.                      |
| `crates/neojs-schedulers/types/neo-schedulers.d.ts` | The two canonical scheduler names and the deprecated `neo:scheduler` alias.              |
| `crates/neojs-sqlite/types/neo-sqlite.d.ts`         | The SQLite fragment owned by the optional crate, not a standalone core declaration file. |

Choose declarations for the extensions the host links and registers, not for
the permissions a script receives:

- No extensions: use `types/neo-js.core.d.ts`.
- A custom selection: pass `--with os`, `--with os-user`, `--with schedulers`, or `--with sqlite`
  for each extension your host registers. For example, SQLite alone uses
  `--with sqlite`, and OS metadata alone uses `--with os`.
- All four, as in the CLI: use `types/neo-js.d.ts` or compose with
  `--with os --with os-user --with schedulers --with sqlite`.

The declaration files record module availability in the host build, not permission.
An import can type-check yet fail at runtime if an extension is not registered,
the allowlist omits it, or a required permission is missing. `neo:os` needs
registration and allowlist admission but no permission flag. The scheduler
fragment declares all three scheduler names, but a runtime allowlist admits
each exact name independently. Scheduler imports need no permission flag
either. `neo:sqlite` needs `Permissions::sqlite`, and `neo:os/user` needs
`Permissions::user`. Core-only declarations omit all six optional module
names and the `os` global.

## Compose and check declarations offline

Run these commands from the checkout root with the Cargo dependencies already
cached. Select optional fragments explicitly. Use an isolated target directory
to keep the build outputs separate.

```sh
# Print the core-only composition.
cargo run --locked --offline -p neojs --target-dir target/docs-reference --example compose-types

# Print a composition for a host that registers only neojs-os.
cargo run --locked --offline -p neojs --target-dir target/docs-reference --example compose-types -- --with os

# Print a composition for a host that registers only neojs-schedulers.
cargo run --locked --offline -p neojs --target-dir target/docs-reference --example compose-types -- --with schedulers

# Print a composition for a host that registers only neojs-sqlite.
cargo run --locked --offline -p neojs --target-dir target/docs-reference --example compose-types -- --with sqlite

# Print the CLI composition and verify its selection against the manifest.
cargo run --locked --offline -p neojs --target-dir target/docs-reference --example compose-types -- \
  --with os --with os-user --with schedulers --with sqlite --check-cli-manifest

# Regenerate the checked-in CLI entry.
cargo run --locked --offline -p neojs --target-dir target/docs-reference --example compose-types -- \
  --with os --with os-user --with schedulers --with sqlite --check-cli-manifest --output types/neo-js.d.ts

# Verify the CLI entry without writing.
cargo run --locked --offline -p neojs --target-dir target/docs-reference --example compose-types -- \
  --with os --with os-user --with schedulers --with sqlite --check-cli-manifest --check types/neo-js.d.ts
```

Core is always included. Repeat `--with` to select more fragments. Unknown or
duplicate selections fail. `--output -` or no output option prints to stdout;
`--root PATH` selects another checkout. `--check-cli-manifest` verifies the
explicit selection in both directions. It does not infer selections. It counts
ordinary runtime dependencies and default-enabled optional dependencies, not
dev or build dependencies. Target-specific, workspace-inherited, and renamed
registered dependencies are not resolved. Neither are non-default feature
sets or workspace feature unification.

The composer refuses to overwrite inputs, including hard-link and symlink
aliases. It checks file identity before writing and again before atomically
replacing the output with a same-directory temporary file. It preserves
existing output permissions and leaves the old output untouched on failure.
A concurrent directory change after the last check can still redirect the
replacement. The composer does not truncate an input inode.

With a local TypeScript 5.8 or later:

```sh
tsc -p examples/typed-script/tsconfig.json
tsc -p examples/typed-script/tsconfig.json --lib ES2022,ESNext.Disposable,DOM,DOM.Iterable
tsc -p examples/typed-script/tsconfig.json --lib ES2022,ESNext.Disposable,WebWorker,WebWorker.Iterable
tsc --noEmit --strict --target ES2022 --module ESNext --moduleResolution Bundler \
  --lib ES2022,ESNext.Disposable --skipLibCheck false types/neo-js.d.ts types/sqlite.test.ts
tsc --noEmit --strict --target ES2022 --module ESNext --moduleResolution Bundler \
  --skipLibCheck false types/neo-js.core.d.ts types/sqlite-absent.test.ts
tsc --noEmit --strict --target ES2022 --module ESNext --moduleResolution Bundler \
  --skipLibCheck false types/neo-js.core.d.ts types/os-absent.test.ts
```

The last two commands must each fail with exactly one `TS2307`, for
`neo:sqlite` and `neo:os` respectively.
`examples/typed-script/smoke.sh` checks for those failures and checks that the
positive fixture does not silently treat `neo:sqlite` as `any`. Deno's
cached-only check can exercise the composed declarations, but Deno treats
unknown `scheme:` imports as external. Use `tsc` to verify core-only absence.
The repository's `types/README.md` has the full smoke and composition checks.

## Syntax the loader accepts

A `.ts` or `.mts` entry, and any relative `.ts` or `.mts` import, is stripped
to JavaScript before QuickJS sees it. Stripping erases type syntax; it does
not check types and does not compile anything. A `.js` or `.mjs` file is
evaluated byte for byte, so type syntax in it is a syntax error, as before.

Erased: annotations, `interface`, `type`, generic parameters and arguments,
`as`, `as const`, `satisfies`, `<T>expr`, non-null assertions, `implements`,
`abstract` members, `declare` fields, access modifiers, overload signatures,
`import type`, `export type`, inline `type` specifiers, and ambient
declarations such as `declare namespace` and `declare enum`. An import whose
specifiers are all `type` is erased whole, so a module that only holds types
does not have to exist at run time. Value imports, side-effect imports, and
their specifiers are kept, so a `.ts` specifier reaches the loader unchanged.
A type-only import whose name is also declared in the same module is a
`TS_SEMANTIC` error, as in TypeScript, and it is reported before anything runs
rather than hidden by the erasure.

Rejected with a `TS_UNSUPPORTED` diagnostic instead of being transformed:
`enum` and `const enum`, `namespace` and `module` declarations with a body,
constructor parameter properties, decorators, `import x = ...`, `export = ...`,
and `export as namespace`. Precompile those with `tsc` (targeting ES2022 or
later) and run the output as JavaScript.

Diagnostics are one line each, `<path>:<line>:<column>: <message> [<code>]`,
with code `TS_PARSE`, `TS_SEMANTIC`, `TS_UNSUPPORTED`, `TS_TRANSFORM`, or
`TS_EXTENSION`, and are reported before any of the module runs. Only the first
is reported. Early semantic errors, such as a duplicate lexical declaration, are
reported before any module code runs.

Output is regenerated from the parsed module. Formatting, blank lines, and a
comment that sat only on an erased node are not preserved, and no source map is
produced: a stack trace names the generated JavaScript position, not the
original TypeScript. `.cts` and `.tsx` modules and `.d.ts` declaration files
are rejected; JSX is not supported.

neo-js pins Oxc 0.81.0, the newest Oxc release whose full dependency graph
compiles on Rust 1.87. `Cargo.lock` holds two transitive dependencies back for
the same reason: `textwrap` 0.16.2 (0.16.4 needs Rust 1.88) and
`oxc_sourcemap` 4.1.0 (4.2 needs Rust 1.89). Keep both pins when updating.
