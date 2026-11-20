# neo-js documentation

This directory contains the Starlight site. To read the Markdown in the repository, start with the
[script guide](src/content/docs/guides/run.md), [module guides](src/content/docs/guides/modules/),
[runtime guides](src/content/docs/runtime/), [module declarations](src/content/docs/api/modules/),
or [public host globals](src/content/docs/api/globals.md). Links inside site pages use site URLs,
not repository paths.

## Build the site

From the checkout root, with Node.js 22.23.4 or later, pnpm 12.18.0, and Rust 1.87 or later:

```sh
pnpm --dir docs/site install --frozen-lockfile --offline
ASTRO_TELEMETRY_DISABLED=1 pnpm --dir docs/site check
```

The reference check verifies declaration composition and runs
`cargo build --locked --offline -p neojs --bin neo-js` from this checkout.
It uses the CLI executable Cargo reports. Cargo reuses unchanged outputs in
the checkout's `target/docs-reference/` directory. If Cargo cannot build the
CLI, the check fails without running a cached executable. Populate the Cargo
and pnpm dependency caches before running the offline checks.

`pnpm --dir docs/site check` checks formatting and the API reference, then
builds the static site into `docs/site/dist/`. To preview edits, run
`pnpm --dir docs/site dev`.

Serve `dist/` at the root of a site. Links such as `/api/` do not support
deployment under a path prefix without changes. No public site URL is
configured, so Astro skips sitemap generation. Set `site` in `astro.config.mjs`
once a deployment URL exists.

## Update the API reference

Edit [`types/neo-js.core.d.ts`](../../types/neo-js.core.d.ts) for built-ins
or the fragment owned by the optional crate:
[`neojs-os`](../../crates/neojs-os/types/neo-os.d.ts) or
[`neojs-schedulers`](../../crates/neojs-schedulers/types/neo-schedulers.d.ts) or
[`neojs-sqlite`](../../crates/neojs-sqlite/types/neo-sqlite.d.ts).
Do not hand-edit the composed CLI entry. See the
[composition instructions](../../types/README.md). From the checkout root:

```sh
cargo run --locked --offline -p neojs --target-dir target/docs-reference --example compose-types -- \
  --with os --with schedulers --with sqlite --check-cli-manifest --output types/neo-js.d.ts
pnpm --dir docs/site reference:generate
pnpm --dir docs/site reference:check
```

Commit the changes under [the generated API directory](src/content/docs/api/).
Do not put other files in that directory. Add examples or longer explanations
under [the guides directory](src/content/docs/guides/).

The generator checks core module names against `RuntimeModule::name()` in
Rust and publishes the CLI composition, including optional modules linked by
the CLI. `pnpm reference:compose:check` checks that composition without
rewriting it. The inventory check requires the permission model to match the
`--allow-*` options listed in the current CLI's `--help`. It then imports each
module through the CLI with its unrestricted allowlist: once with every flag,
once with none, and once per flag. The probe runs in a temporary directory with
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

The CLI probe checks all three scheduler specifiers independently. The CLI
registers `neojs-schedulers`; none of its names requires a permission flag.
The `neojs-schedulers` crate's `allowlist` tests verify independent admission.

Run `pnpm --dir docs/site reference:test` to exercise the checks against a
temporary copy of the Rust sources. The copy has its own Cargo target
directory and builds offline.
