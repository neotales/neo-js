# neo-js documentation

This directory contains the Starlight site. Start with the [site home page](src/content/docs/index.md)
or the [script guide](src/content/docs/guides/run.md) when reading in the repository.

## Build the site

From this directory, with Node.js 22.23.4 or later and pnpm 12.18.0:

```sh
pnpm install --frozen-lockfile
pnpm check
```

`pnpm check` checks formatting and builds the static site into `dist/`. The
build checks the API reference before rendering. For a local preview while
editing, run `pnpm dev`.

No public site URL is configured, so Astro skips sitemap generation. Set
`site` in `astro.config.mjs` once a deployment URL exists.

## Update the API reference

Edit `types/neo-js.d.ts` at the repository root, then run:

```sh
pnpm reference:generate
pnpm reference:check
```

Commit the generated changes under `src/content/docs/api/`. The generator owns
that directory. Add examples or longer explanations under `src/content/docs/guides/`.

The generator reads the declarations with the TypeScript parser and compares
their module names with `RuntimeModule::name()` in Rust. The reference tracks
the declared signatures, not every runtime export directly. Update the
declaration file when changing a runtime export and
[host permissions](src/content/docs/runtime/permissions.md) when changing access.
`console` is documented separately because the runtime supplies it but the
declaration file does not.
