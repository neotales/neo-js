#!/bin/sh
set -eu

cd "$(dirname "$0")/../.."
example=examples/typed-script

if command -v tsc >/dev/null 2>&1; then
  tsc -p "$example/tsconfig.json"
  tsc -p "$example/tsconfig.json" --lib ES2022,DOM,DOM.Iterable
  tsc -p "$example/tsconfig.json" --lib ES2022,WebWorker,WebWorker.Iterable
  printf '%s\n' 'tsc: ES2022, DOM, and WebWorker checks passed'
elif command -v deno >/dev/null 2>&1; then
  printf '%s\n' 'tsc unavailable: using Deno; the tsc project and optional lib checks are unverified.'
  deno check --cached-only --no-config types/neo-js.d.ts types/schedulers.test.ts \
    "$example/main.mts" "$example/types.test.mts"
else
  printf '%s\n' 'Smoke test needs a local tsc or Deno executable.' >&2
  exit 1
fi

output=$(cargo run --offline --quiet -p neo-js-cli -- --allow-stdio "$example/main.mts")
case "$output" in
  'neo-js: '?*/?*) ;;
  *) printf 'Unexpected example output: %s\n' "$output" >&2; exit 1 ;;
esac

if denied=$(cargo run --offline --quiet -p neo-js-cli -- "$example/main.mts" 2>&1); then
  printf '%s\n' 'Example unexpectedly ran without --allow-stdio.' >&2
  exit 1
fi
case "$denied" in
  *"Error resolving module 'neo:process'"*) ;;
  *) printf '%s\n' 'Expected a module resolution error for neo:process.' >&2; exit 1 ;;
esac
printf '%s\n' 'neo-js: example ran with --allow-stdio and was denied without it'
