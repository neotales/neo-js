# Typed neo-js script

Open this folder (or the whole checkout) in VS Code with its built-in TypeScript
language support, or use an editor whose TypeScript language server reads
`tsconfig.json`. Select TypeScript 5.8 or newer for `erasableSyntaxOnly`. In VS
Code you can check the active version with **TypeScript: Select TypeScript
Version** while `main.mts` is open. Try completion on `os.` or hover over
`stdout.write`. The config lists `../../types/neo-js.d.ts` as an input so the
editor sees both the ambient globals and the `neo:` modules. Keep the checkout
layout, or copy the declaration into your own project and update `files` to
match.

The config checks ES2022 without loading DOM or automatic `@types` packages.
Keep scripts in `.ts` or `.mts`, use `import type` for type-only imports, and
keep explicit file extensions in relative imports. The config is not a runtime
compatibility check: neo-js strips types, does not support every TypeScript
syntax form, and does not resolve bare imports. Adding browser, Node, or Deno
types does not make those host APIs available in neo-js. The runtime's `console`
exists, but it is not declared in the current ES-only setup.

## Run and check offline

From the checkout root, with the Rust dependencies already cached:

```sh
cargo run --offline -p neo-js-cli -- --allow-stdio examples/typed-script/main.mts
```

The script prints the target platform and architecture from `neo:os` and, when
available, the operating-system release from the `os` global. OS metadata
needs no permission in the CLI. `neo:process` needs `--allow-stdio` for this
example; no filesystem permission is needed. The runtime enforces permissions,
not TypeScript. A script can type-check and still fail to import a module when
its permission is absent or an embedder excludes it.

Use an already installed `tsc`, or use Deno's bundled checker as a fallback:

```sh
tsc -p examples/typed-script/tsconfig.json
# Fallback if tsc is not available:
deno check --cached-only --no-config types/neo-js.d.ts types/schedulers.test.ts \
  types/win-registry.test.ts examples/typed-script/main.mts examples/typed-script/types.test.mts
```

Deno checks the listed inputs with its own defaults, not the `tsconfig.json`
project. Use it only for checking; run the example with neo-js. The
`types.test.mts` fixture is not a runnable example: it requires the checker to
reject invalid types.

Run `./examples/typed-script/smoke.sh` from a POSIX shell to check the types,
run the example, and verify the missing permission causes a failure. With `tsc`
on `PATH`, the smoke test also checks for declaration collisions with the DOM
and WebWorker libs separately. Without `tsc`, it reports that those checks were
not run and uses cached-only Deno instead. No command installs dependencies.
