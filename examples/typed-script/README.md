# Typed neo-js script

Open this folder (or the whole checkout) in VS Code with its built-in TypeScript
language support, or use an editor whose TypeScript language server reads
`tsconfig.json`. Select TypeScript 5.8 or newer for `erasableSyntaxOnly`. In VS
Code you can check the active version with **TypeScript: Select TypeScript
Version** while `main.mts` is open. Try completion on `os.` or hover over
`stdout.write`. The config lists `../../types/neo-js.d.ts` as an input so the
editor sees both the ambient globals and the `neo:` modules. That file is the
generated CLI composition: core declarations plus the optional modules the CLI
links and registers: `neojs-os`, `neojs-os-user`, `neojs-schedulers`, and `neojs-sqlite`.
For an embedder, match the declaration selection to the extensions the host
registers. With none, use `../../types/neo-js.core.d.ts`. Otherwise, pass
`--with os`, `--with os-user`, `--with schedulers`, or `--with sqlite` for each registered
extension. For example, a host with only SQLite uses `--with sqlite`.
Use the CLI composition only for a host that registers all four extensions,
or generate it with `--with os --with os-user --with schedulers --with sqlite`.

Do not select declarations based on which runtime permissions a script has.
The declaration files grant no permissions and do not override an allowlist.
See [`types/README.md`](../../types/README.md) for the composer command. Keep
the checkout layout, or copy the chosen declaration into your own project
and update `files` to match.

The config checks ES2022 plus `ESNext.Disposable` without loading DOM or
automatic `@types` packages. `ESNext.Disposable` supplies `Symbol.dispose`,
which the `neo:sqlite` declarations use.
Keep scripts in `.ts` or `.mts`, use `import type` for type-only imports, and
keep explicit file extensions in relative imports. The config is not a runtime
compatibility check: neo-js strips types, does not support every TypeScript
syntax form, and does not resolve bare imports. Adding browser, Node, or Deno
types does not make those host APIs available in neo-js. The runtime's `console`
exists, but it is not declared in the current ES-only setup.

`types/neo-js.d.ts` references `ESNext.Disposable` for `RegistryKey` and
`Symbol.dispose`; use TypeScript 5.8+ as above. Neo-js runs `using` directly in
JavaScript and in type-stripped TypeScript, without a polyfill. For example,
`using key = Registry.openKey(path)` closes the handle at scope exit. Import
`Registry` from `neo:win-registry`; opening a real key needs Windows and
`--allow-registry`. If you maintain separate declarations for disposable
types, include `ESNext.Disposable` in their TypeScript `lib` list.

## Run and check offline

From the checkout root, with the Rust dependencies already cached:

```sh
cargo run --offline -p neojs -- --allow-stdio examples/typed-script/main.mts
```

The script prints the target platform and architecture from `neo:os` and, when
available, the operating-system release from the `os` global. The CLI registers
the optional `neojs-os` crate, so OS metadata needs no permission. `neo:process`
needs `--allow-stdio` for this
example; no filesystem permission is needed. The runtime enforces permissions,
not TypeScript. A script can type-check and still fail to import a module when
its permission is absent or an embedder excludes it.

Use an already installed `tsc`, or use Deno's bundled checker as a fallback:

```sh
tsc -p examples/typed-script/tsconfig.json
# Fallback if tsc is not available:
deno check --cached-only --no-config types/neo-js.d.ts types/path.test.ts \
  types/os-user.test.ts types/schedulers.test.ts types/sqlite.test.ts types/synchronous-callbacks.test.ts \
  types/win-registry.test.ts examples/typed-script/main.mts examples/typed-script/types.test.mts
```

Deno checks the listed inputs with its own defaults, not the `tsconfig.json`
project. Use it only for checking; run the example with neo-js. The
`types.test.mts`, `types/path.test.ts`, and `types/schedulers.test.ts` fixtures
are checked, not run: they require the checker to reject invalid types. The
path fixture covers the file-only `FileURL` type without requiring a DOM
global declaration. Deno cannot check the core-only negative fixture: it
resolves an unknown `scheme:` specifier as external and types it as `any`. The
smoke test proves the composed SQLite fixture is not accepted that way by
pointing it at a core-only composition; its `@ts-expect-error` assertions then
become unused and fail.

Run `./examples/typed-script/smoke.sh` from a POSIX shell to verify the
checked-in composition, check the types, run the example, and verify the
missing permission causes a failure. With `tsc` on `PATH`, the smoke test also
checks for declaration collisions with the DOM and WebWorker libs separately,
checks that the composed entry admits `neo:os` and `neo:sqlite`, checks that
the core entry rejects each with exactly `TS2307`, and checks that the positive
SQLite fixture fails when the fragment is removed. Without `tsc`, it reports that the tsc-only
absence check was not run, uses cached-only Deno, and still runs the
fragment-removal mutation. No command installs dependencies.
