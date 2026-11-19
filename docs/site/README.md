# neo-js documentation

This directory contains the Starlight site. To read the Markdown in the repository, start with the
[script guide](src/content/docs/guides/run.md), [module guides](src/content/docs/guides/modules/),
[runtime guides](src/content/docs/runtime/), [module declarations](src/content/docs/api/modules/),
or [public host globals](src/content/docs/api/globals.md). Links inside site pages use site URLs,
not repository paths.

## Build the site

From the checkout root, with Node.js 22.23.4 or later, pnpm 12.18.0, and Rust 1.87 or later:

```sh
cargo build --locked --offline -p neo-js-cli --target-dir target/docs-inventory --bin neo-js
export NEO_JS_DOCS_BIN="$PWD/target/docs-inventory/debug/neo-js"
pnpm --dir docs/site install --frozen-lockfile --offline
ASTRO_TELEMETRY_DISABLED=1 pnpm --dir docs/site check
```

Use `neo-js.exe` in the binary path on Windows. Use only a trusted binary.
Rebuild it from this checkout when Rust or built-in module source changes.
The check requires an absolute binary path and does not build or download the
CLI. If the offline install cannot find dependencies, populate the Cargo and
pnpm caches before running the checks.

`pnpm --dir docs/site check` checks formatting and the API reference, then
builds the static site into `docs/site/dist/`. To preview edits, run
`pnpm --dir docs/site dev`.

Serve `dist/` at the root of a site. Links such as `/api/` do not support
deployment under a path prefix without changes. No public site URL is
configured, so Astro skips sitemap generation. Set `site` in `astro.config.mjs`
once a deployment URL exists.

## Update the API reference

Edit [`types/neo-js.d.ts`](../../types/neo-js.d.ts), then run from the checkout
root with `NEO_JS_DOCS_BIN` set:

```sh
pnpm --dir docs/site reference:generate
pnpm --dir docs/site reference:check
```

Commit the changes under [the generated API directory](src/content/docs/api/).
Do not put other files in that directory. Add examples or longer explanations
under [the guides directory](src/content/docs/guides/).

The generator checks declaration module names against `RuntimeModule::name()`
in Rust. The inventory check imports each module through the CLI with its
unrestricted allowlist, first with all permission flags, then without any
flags, then with each flag alone. The probe runs in a temporary directory with
an empty environment. It enumerates export names without reading export values
or calling module APIs. The CLI still makes its normal bounded OS metadata
probe during initialization. The inventory neither reads nor prints its result.

The check compares runtime value names, including default exports, with
TypeScript declarations. It excludes type-only exports and accounts for
`neo:hash.digestFile` being absent without `--allow-fs`. It does not check
runtime function signatures or custom embedder allowlists. Update the
declarations when changing exports, and update the probe and
[host permissions](src/content/docs/runtime/permissions.md) when changing access.
The runtime supplies `console`, but the declaration file does not declare it.

To run the regression tests with the same binary, use
`pnpm --dir docs/site reference:test`.
