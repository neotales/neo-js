# TypeScript declarations

The declaration files are split by the crate that supplies each module. A host
that compiles only `neojs-core` uses the core file; a host that also links an
optional crate uses a composition of the core file and that crate's fragment.

| File                                         | Role                                                                                                                          |
| -------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| `neo-js.core.d.ts`                           | Hand-written declarations for `neojs-core`. It omits every optional module and the `os` global.                               |
| `neo-js.d.ts`                                | Generated entry for the CLI build. It is core plus the four extensions the CLI links. Do not edit it by hand.                 |
| `neojs-io/types/neo-io.d.ts`                 | Hand-written synchronous byte reader and writer fragment owned by `neojs-io`.                                                |
| `neojs-os/types/neo-os.d.ts`                 | Hand-written `neo:os` fragment, including the `os` global, owned by the `neojs-os` crate.                                     |
| `neojs-os-user/types/neo-os-user.d.ts`       | Hand-written `neo:os/user` fragment owned by `neojs-os-user`. It does not include `neo:os` or its global.                     |
| `neojs-schedulers/types/neo-schedulers.d.ts` | Hand-written systemd, Windows Task Scheduler, and deprecated `neo:scheduler` fragments owned by the `neojs-schedulers` crate. |
| `neojs-sqlite/types/neo-sqlite.d.ts`         | Hand-written `neo:sqlite` fragment owned by the `neojs-sqlite` crate.                                                         |

The declarations describe **compiled availability**, not permission. Declaring
the built-in `neo:win-registry` (or its deprecated `neo:winregistry` alias)
does not grant registry permission or allowlist admission. The core
declarations reference `ESNext.Disposable` for owned registry keys. Declaring
`neo:os` means the host links `neojs-os` and can register the module; it grants
no permission because none is needed, and a runtime allowlist can still deny
the import. Declaring the scheduler specifiers means the host links
`neojs-schedulers` and can register them; they grant no permission, and a
runtime allowlist still decides each exact name independently. Declaring
`neo:io` means the host links `neojs-io` and can register the module; it neither
needs nor grants a permission, and a runtime allowlist still decides the exact
name. Declaring `neo:sqlite` means the host links `neojs-sqlite` and can
register the module; it does not grant `Permissions::sqlite`, and a runtime
allowlist can still deny the import. Declaring `neo:os/user` likewise grants
neither `Permissions::user` nor explicit allowlist admission. The reverse is
also true: a script can type-check against the core
file, where all optional modules are absent, even on a CLI that would admit
them at run time. The checker models the build, while the
runtime enforces permissions and allowlists.

## Select

- Match the optional fragments to the extensions your host links and
  registers, not the permissions or allowlist of a particular runtime.
- Core-only embedder, or a script checked against the core API only:
  reference `types/neo-js.core.d.ts`.
- For a custom selection, pass one flag per registered extension: `--with io`
  for `neojs-io`, `--with os` for `neojs-os`, `--with os-user` for
  `neojs-os-user`, `--with schedulers` for `neojs-schedulers`, and
  `--with sqlite` for `neojs-sqlite`. For example, SQLite alone uses
  `--with sqlite`; registering byte I/O and OS uses `--with io --with os`.
- The CLI registers four extensions: OS, OS user, schedulers, and SQLite. Use
  `types/neo-js.d.ts` or compose with
  `--with os --with os-user --with schedulers --with sqlite`. The CLI does not
  link `neojs-io`; add `--with io` only for a host that does.

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
  --with os --with os-user --with schedulers --with sqlite --check-cli-manifest

# Regenerate the checked-in CLI entry.
cargo run --offline -p neojs --example compose-types -- \
  --with os --with os-user --with schedulers --with sqlite --check-cli-manifest --output types/neo-js.d.ts

# Verify it without writing.
cargo run --offline -p neojs --example compose-types -- \
  --with os --with os-user --with schedulers --with sqlite --check-cli-manifest --check types/neo-js.d.ts

# Compose core plus the optional byte I/O fragment for a host that links it.
cargo run --offline -p neojs --example compose-types -- --with io --output target/neo-js-io.d.ts
```

`--with` is repeatable. Unknown or duplicate selections are errors. `--output -`
is stdout; omitting `--output` also uses stdout. `--check PATH` compares and
never writes. Only the requested output path is written: the tool refuses to
write over the core fragment, an optional fragment, or the CLI manifest.
`--root PATH` selects a different checkout; the default is this one.

`--check-cli-manifest` parses `neojs/Cargo.toml` as TOML and checks
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
tsc --noEmit --strict --target ES2022 --module ESNext --moduleResolution Bundler \
  --skipLibCheck false types/neo-js.core.d.ts types/os-absent.test.ts
```

The last two commands must each fail with exactly one `TS2307` for `neo:sqlite`
and `neo:os` respectively.
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
  types/os-user.test.ts types/schedulers.test.ts types/sqlite.test.ts types/synchronous-callbacks.test.ts \
  examples/typed-script/main.mts examples/typed-script/types.test.mts

# The io fragment is not part of the CLI entry; compose it explicitly.
cargo run --offline -p neojs --example compose-types -- --with io --output target/neo-js-io.d.ts
deno check --cached-only --no-config target/neo-js-io.d.ts types/io.test.ts
```

Deno resolves an unknown `scheme:` specifier as external and types it as `any`,
so it cannot show that core-only declarations reject `neo:os` or `neo:sqlite`.
Those absence checks are `tsc`-only. Deno does prove the positive fixture is not
accepted as `any`, through the same mutation the smoke test runs: the fixture
fails without its fragment because its negative assertions become unused
directives.

## Add another optional module

Put the fragment next to the crate that supplies the module, for example
`neojs-<name>/types/neo-<name>.d.ts`. Add it to `OPTIONAL` in
`neojs/src/declarations.rs`; the composer then accepts
`--with <name>`. Select it for the CLI entry only when `neojs`
depends on that crate, and add a fixture under `types/`. Do not copy the
declarations into the core file.
