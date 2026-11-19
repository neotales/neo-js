#!/bin/sh
set -eu

cd "$(dirname "$0")/../.."
example=examples/typed-script
mutation=target/tmp/compose-types-smoke.$$
rm -rf "$mutation"
mkdir -p "$mutation"
trap 'rm -rf "$mutation"' 0

# The checked-in CLI composition must match the explicit selection and the
# CLI manifest. The tool never selects fragments from the manifest itself.
cargo run --offline --quiet -p neojs --example compose-types -- \
  --with sqlite --check-cli-manifest --check types/neo-js.d.ts

# Prove the positive SQLite fixture is load-bearing. Deno types an unknown
# `scheme:` specifier as `any`, and a checker would otherwise accept the
# fixture without the fragment. Point the fixture at a core-only composition:
# its @ts-expect-error directives must become unused and fail.
cargo run --offline --quiet -p neojs --example compose-types > "$mutation/core-only.d.ts"
sed 's#\./neo-js\.d\.ts#./core-only.d.ts#' types/sqlite.test.ts > "$mutation/sqlite.test.ts"

if command -v tsc >/dev/null 2>&1; then
  tsc -p "$example/tsconfig.json"
  tsc -p "$example/tsconfig.json" --lib ES2022,ESNext.Disposable,DOM,DOM.Iterable
  tsc -p "$example/tsconfig.json" --lib ES2022,ESNext.Disposable,WebWorker,WebWorker.Iterable
  # The CLI links neojs-sqlite, so the composed declarations admit the module.
  tsc --noEmit --strict --target ES2022 --module ESNext --moduleResolution Bundler \
    --lib ES2022,ESNext.Disposable --skipLibCheck false \
    types/neo-js.d.ts types/sqlite.test.ts

  # Core declarations must reject the same import with exactly TS2307.
  if core_errors=$(tsc --noEmit --strict --target ES2022 --module ESNext \
    --moduleResolution Bundler --skipLibCheck false --pretty false \
    types/neo-js.core.d.ts types/sqlite-absent.test.ts 2>&1); then
    printf '%s\n' 'Core declarations unexpectedly admitted neo:sqlite.' >&2
    exit 1
  fi
  core_errors=$(printf '%s\n' "$core_errors" | tr -d '\r')
  error_count=$(printf '%s\n' "$core_errors" | grep -c 'error TS' || true)
  if [ "$error_count" -ne 1 ]; then
    printf 'Core-only check produced %s diagnostics, expected exactly one TS2307:\n%s\n' \
      "$error_count" "$core_errors" >&2
    exit 1
  fi
  if ! printf '%s\n' "$core_errors" | grep -Eq \
    "error TS2307: Cannot find module 'neo:sqlite' or its corresponding type declarations\.$"; then
    printf 'Core-only check did not report the expected TS2307 for neo:sqlite:\n%s\n' \
      "$core_errors" >&2
    exit 1
  fi

  # The fixture's negative assertions must fail without the fragment.
  if mutation_errors=$(tsc --noEmit --strict --target ES2022 --module ESNext \
    --moduleResolution Bundler --lib ES2022,ESNext.Disposable --skipLibCheck false \
    --pretty false "$mutation/core-only.d.ts" "$mutation/sqlite.test.ts" 2>&1); then
    printf '%s\n' 'The positive SQLite fixture passed without its fragment.' >&2
    exit 1
  fi
  case "$mutation_errors" in
    *"Unused '@ts-expect-error' directive."*) ;;
    *)
      printf 'The core-only fixture did not fail through an unused @ts-expect-error:\n%s\n' \
        "$mutation_errors" >&2
      exit 1
      ;;
  esac
  printf '%s\n' 'tsc: ES2022, DOM, WebWorker, and composed SQLite checks passed; core rejects neo:sqlite with exactly TS2307; the fixture fails without its fragment'
elif command -v deno >/dev/null 2>&1; then
  printf '%s\n' 'tsc unavailable: using Deno; the tsc project and optional lib checks are unverified.'
  deno check --cached-only --no-config types/neo-js.d.ts types/path.test.ts \
    types/schedulers.test.ts types/sqlite.test.ts types/synchronous-callbacks.test.ts \
    "$example/main.mts" "$example/types.test.mts"

  # Deno cannot prove the core-only absence message: it types an unknown
  # `scheme:` specifier as `any`. It can prove the positive fixture rejects
  # that fallback, because the negative assertions become unused directives.
  if mutation_errors=$(deno check --cached-only --no-config \
    "$mutation/core-only.d.ts" "$mutation/sqlite.test.ts" 2>&1); then
    printf '%s\n' 'The positive SQLite fixture passed without its fragment.' >&2
    exit 1
  fi
  case "$mutation_errors" in
    *"Unused '@ts-expect-error'"*) ;;
    *)
      printf 'The core-only fixture did not fail through an unused @ts-expect-error:\n%s\n' \
        "$mutation_errors" >&2
      exit 1
      ;;
  esac
  printf '%s\n' 'Deno: composed declarations passed; core-only absence is not checkable by Deno (unknown schemes are any); the fixture fails without its fragment'
else
  printf '%s\n' 'Smoke test needs a local tsc or Deno executable.' >&2
  exit 1
fi

output=$(cargo run --offline --quiet -p neojs -- --allow-stdio "$example/main.mts")
case "$output" in
  'neo-js: '?*/?*) ;;
  *) printf 'Unexpected example output: %s\n' "$output" >&2; exit 1 ;;
esac

if denied=$(cargo run --offline --quiet -p neojs -- "$example/main.mts" 2>&1); then
  printf '%s\n' 'Example unexpectedly ran without --allow-stdio.' >&2
  exit 1
fi
case "$denied" in
  *"Error resolving module 'neo:process'"*) ;;
  *) printf '%s\n' 'Expected a module resolution error for neo:process.' >&2; exit 1 ;;
esac
printf '%s\n' 'neo-js: example ran with --allow-stdio and was denied without it'
