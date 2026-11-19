# TypeScript declarations

The declaration files are split by the crate that supplies each module. A host
that compiles only `neojs-core` uses the core file; a host that also links an
optional crate uses a composition of the core file and that crate's fragment.

| File | Role |
| --- | --- |
| `neo-js.core.d.ts` | Hand-written declarations for `neojs-core`. It omits every optional module, including `neo:sqlite`. |
| `neo-js.d.ts` | Generated entry for the CLI build. It is core plus the optional modules the CLI links, currently `neo:sqlite`. Do not edit it by hand. |
| `crates/neojs-sqlite/types/neo-sqlite.d.ts` | Hand-written `neo:sqlite` fragment owned by the `neojs-sqlite` crate. |

The declarations describe **compiled availability**, not permission. Declaring
`neo:sqlite` means the host links `neojs-sqlite` and can register the module; it
does not grant `Permissions::sqlite`, and a runtime allowlist can still deny the
import. The reverse is also true: a script can type-check against the core file,
where the module is absent, even on a CLI that would admit it at run time with
`--allow-sqlite`. The checker models the build, while the runtime enforces
permissions.

## Select

- Core-only embedder, or a script checked against the core API only:
  reference `types/neo-js.core.d.ts`.
- CLI build, which links `neojs-core` and `neojs-sqlite`: reference
  `types/neo-js.d.ts`. This is the existing public entry and keeps the previous
  module set. Consumers that relied on `neo:sqlite` through this file need no
  change.

`examples/typed-script/tsconfig.json` lists the composed entry, so the example
and its editor completion continue to describe the CLI.

## Generate and verify

The composer is the offline `compose-types` example. Selection is explicit:
core is always included, and each `--with NAME` adds a registered optional
fragment. It writes to stdout by default, or to one explicit `--output` path.
It never selects a fragment from Cargo metadata.

```sh
# Core-only declarations, printed to stdout.
cargo run --offline -p neojs --example compose-types

# CLI selection, printed to stdout.
cargo run --offline -p neojs --example compose-types -- \
  --with sqlite --check-cli-manifest

# Regenerate the checked-in CLI entry.
cargo run --offline -p neojs --example compose-types -- \
  --with sqlite --check-cli-manifest --output types/neo-js.d.ts

# Verify it without writing.
cargo run --offline -p neojs --example compose-types -- \
  --with sqlite --check-cli-manifest --check types/neo-js.d.ts
```

`--with` is repeatable. Unknown or duplicate selections are errors. `--output -`
is stdout; omitting `--output` also uses stdout. `--check PATH` compares and
never writes. Only the requested output path is written: the tool refuses to
write over the core fragment, an optional fragment, or the CLI manifest.
`--root PATH` selects a different checkout; the default is this one.

`--check-cli-manifest` parses `crates/neo-js-cli/Cargo.toml` as TOML and checks
both directions for every registered optional fragment: a linked normal
dependency must be selected, and a selected fragment must be a linked normal
dependency. It only verifies the explicit selection; it never chooses fragments
or claims that the manifest authorizes the output.

The check reads `[dependencies]` for ordinary resolution.
`[dev-dependencies]` and `[build-dependencies]` do not count as runtime links.
An `optional = true` dependency counts only when the `default` feature closure
enables it, through `dep:name`, the implicit `name` feature, or `name/feature`.
A dependency key whose `package` field names another package is not treated as
the registered module. A renamed key with `package = "<registered>"` is
reported unresolved.

Target-specific `dependencies`, `dev-dependencies`, or `build-dependencies`
entries that refer to a registered module are detected before ordinary
resolution and reported unresolved. There is no target-specific fallback, even
when an ordinary entry also exists. The check does not resolve non-default
feature sets, feature unification across workspace members,
workspace-inherited dependency entries, or renamed dependency keys. This is
deliberately not a full Cargo resolver.

Output is written through a same-directory temporary file and an atomic
replacement. Before writing, and again immediately before replacement, the tool
compares file identity rather than path text, so hard-link and symlink aliases
of the core file, a fragment, or the manifest are rejected. An existing
output's permissions are preserved; a new output keeps the temporary file's
restrictive mode. A failed write or replacement leaves the existing output
untouched and removes the temporary file. A concurrent directory-entry change
after the final check can still redirect the replacement, but the atomic
replacement never truncates an input inode, so its other links remain intact.

`cargo test -p neojs --test declarations` covers the composition tests:
enabled and disabled selections, stdout and file output, check mode, refusal to
overwrite inputs including hard-link aliases, atomic replacement and failure
preservation, unknown and duplicate selections, determinism, duplicate module
declarations, both directions of the manifest check, dev-dependency exclusion,
default optional-feature activation, wrong-package-override rejection,
renamed-dependency and renamed-target rejection, target detection before
ordinary resolution, and unresolved workspace dependencies.

## Check the declarations

With a local `tsc` (TypeScript 5.8 or newer), check the composed entry, the
core-only rejection, and the DOM and WebWorker libs:

```sh
tsc -p examples/typed-script/tsconfig.json
tsc -p examples/typed-script/tsconfig.json --lib ES2022,ESNext.Disposable,DOM,DOM.Iterable
tsc -p examples/typed-script/tsconfig.json --lib ES2022,ESNext.Disposable,WebWorker,WebWorker.Iterable
tsc --noEmit --strict --target ES2022 --module ESNext --moduleResolution Bundler \
  --lib ES2022,ESNext.Disposable --skipLibCheck false \
  types/neo-js.d.ts types/sqlite.test.ts
tsc --noEmit --strict --target ES2022 --module ESNext --moduleResolution Bundler \
  --skipLibCheck false types/neo-js.core.d.ts types/sqlite-absent.test.ts
```

The last command must fail with exactly one
`TS2307: Cannot find module 'neo:sqlite' or its corresponding type declarations.`
`examples/typed-script/smoke.sh` rejects any other diagnostic and reports when
`tsc` is unavailable.

The positive `types/sqlite.test.ts` fixture contains negative type assertions
with `@ts-expect-error`. If a checker treats a missing `neo:sqlite` as `any`,
those directives become unused and the fixture fails. The smoke test proves
this by pointing the fixture at a core-only composition and requiring an unused
directive.

Deno checks the composed entry with its defaults:

```sh
deno check --cached-only --no-config types/neo-js.d.ts types/path.test.ts \
  types/schedulers.test.ts types/sqlite.test.ts types/synchronous-callbacks.test.ts \
  examples/typed-script/main.mts examples/typed-script/types.test.mts
```

Deno resolves an unknown `scheme:` specifier as external and types it as `any`,
so it cannot show that core-only declarations reject `neo:sqlite`. That absence
check is `tsc`-only. Deno does prove the positive fixture is not accepted as
`any`, through the same mutation the smoke test runs: the fixture fails without
its fragment because its negative assertions become unused directives.

## Add another optional module

Put the fragment next to the crate that supplies the module, for example
`crates/neojs-<name>/types/neo-<name>.d.ts`. Add it to `OPTIONAL` in
`crates/neo-js-cli/src/declarations.rs`; the composer then accepts
`--with <name>`. Select it for the CLI entry only when `crates/neo-js-cli`
depends on that crate, and add a fixture under `types/`. Do not copy the
declarations into the core file.
