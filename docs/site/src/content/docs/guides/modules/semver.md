---
title: "Semantic versions"
description: "SemVer parsing, comparison, Cargo-style ranges, and input limits."
---

`neo:semver` parses, formats, increments, compares, and matches SemVer 2.0.0
versions. It is synchronous, needs no permission flag, and grants no host
capability: it reads no file, environment variable, process argument, terminal
state, or network resource. Parsing and range evaluation are delegated to the
maintained Rust `semver` crate, which parses without a regular expression; the
module itself contains none. Its native callbacks are private to the module and
are never installed on the global object.

```js
import { parse, format, increment, satisfies, compare } from "neo:semver";

const version = parse("1.2.3-alpha.1+sha.001");
// { version: "1.2.3-alpha.1+sha.001", major: "1", minor: "2", patch: "3",
//   prerelease: ["alpha", "1"], build: ["sha", "001"] }

format(version); // "1.2.3-alpha.1+sha.001"
increment("1.2.3-alpha.1", "prerelease"); // "1.2.3-alpha.2"
satisfies("1.3.0-beta.2", ">=1.3.0-beta.1, <1.4.0"); // true
compare("1.0.0+one", "1.0.0+two"); // 0
```

`valid(version)` and `compare(a, b)` are the original exports and keep their
signatures and ordinary-input behavior. `valid` returns `false` for a malformed
string; `compare` returns `-1`, `0`, or `1` by precedence and throws for a
malformed version. The new size bound applies to both; see Limits below.

`parse(version)` returns a null-prototype object whose `version`, `major`,
`minor`, `patch`, `prerelease`, and `build` are own data properties.
`prerelease` and `build` are arrays of complete identifier strings. Numeric
components are decimal strings, not JavaScript numbers, so a value above
`Number.MAX_SAFE_INTEGER` keeps every digit:

```js
parse("1.0.0-9007199254740993").prerelease[0]; // "9007199254740993"
parse("18446744073709551615.0.0").major; // "18446744073709551615"
```

`format({ major, minor, patch, prerelease, build })` is the inverse. Core
components may be canonical decimal strings or non-negative safe integers; use
a string for a larger value. `prerelease` and `build` are arrays of single
identifiers, so a dot belongs between elements, not inside one.
`increment(version, release, identifier?)` returns a canonical string;
`release` is `major`, `minor`, `patch`, `premajor`, `preminor`, `prepatch`, or
`prerelease`. The optional `identifier` is one complete prerelease identifier,
not a dotted list. `increment` always drops build metadata and never wraps. `major`, `minor`,
and `patch` bump their component, reset lower components, and drop the
prerelease. The `pre*` releases do the same and start at `0` or
`<identifier>.0`. `prerelease` increments the last numeric prerelease
identifier, appends `.0` when none exists, or restarts at `<identifier>.0` when
the first identifier differs from the supplied one. Unlike npm,
`increment("1.0.0-alpha", "major")` is `"2.0.0"`, not `"1.0.0"`.

## Grammar and precision

A version is exactly `MAJOR.MINOR.PATCH[-PRERELEASE][+BUILD]` with no
surrounding whitespace:

- `MAJOR`, `MINOR`, and `PATCH` are unsigned ASCII decimal integers without
  leading zeros. `0` is allowed; `01` is not. Each is limited to `u64::MAX`.
- `PRERELEASE` is one or more non-empty dot-separated identifiers of ASCII
  letters, digits, and `-`. A purely numeric identifier has no leading zero
  unless it is exactly `0`. Numeric prerelease identifiers have no `u64` bound.
- `BUILD` uses the same character set, but leading zeros are allowed because
  build metadata never participates in precedence.

The English SemVer 2.0.0 specification sets no upper bound on a numeric
identifier. This module follows the Rust crate and rejects a core component
above `u64::MAX` with `SEMVER_INVALID` and a message naming the overflowing
component; it never truncates or rounds. A partial version such as `1.2` or
`1`, a leading `v`, whitespace, an empty identifier, and any character outside
the grammar are invalid.

Precedence compares the core numerically, then treats a prerelease as lower
than the same core release. Prerelease identifiers are compared left to right;
numeric identifiers compare numerically, non-numeric identifiers compare in
ASCII order, and a numeric identifier is lower than a non-numeric one. A
shorter identifier list is lower when every shared identifier is equal. Build
metadata is ignored: `1.0.0+one` and `1.0.0+two` have equal precedence. The
official ascending chain
`1.0.0-alpha < 1.0.0-alpha.1 < 1.0.0-alpha.beta < 1.0.0-beta < 1.0.0-beta.2 <
1.0.0-beta.11 < 1.0.0-rc.1 < 1.0.0` is tested.

## Ranges

`satisfies(version, range)` and `validRange(range)` use Cargo's interpretation
of ranges, not npm's. The differences matter:

- A range is `*` alone or one or more comma-separated comparators. `||` unions,
  hyphen ranges (`1.2.3 - 2.0.0`), and whitespace-separated implicit AND
  (`>=1.2.3 <2.0.0`) are invalid; write `>=1.2.3, <2.0.0`.
- A bare version uses Cargo's default caret operator: `1.2` means `^1.2`, so
  `1.9.0` matches and `1.1.9` does not. `1.2.3` means `^1.2.3`, not an exact
  match; write `=1.2.3` for exactness. It is not npm's `1.2.x` x-range. `0.2`
  matches only `0.2.x`, and `0` matches any `0.x.y`. Explicit wildcards
  `1.2.*`, `1.*`, and `*` are accepted.
- `~` and `^` follow Cargo's equivalences. In particular `^0.0.3` matches only
  `0.0.3`, and `^0.0` matches only the `0.0.x` line.
- A prerelease version matches only when at least one comparator names the
  same `MAJOR.MINOR.PATCH` and has a non-empty prerelease. Thus `*` matches no
  prerelease, and `^1.2.0` does not match `1.3.0-alpha`; use
  `>=1.3.0-alpha, <1.4.0`. npm applies the same rule unless
  `includePrerelease` is requested, which this module does not offer.
- At most 32 comparators are accepted. Build metadata on a comparator is
  allowed but ignored.

`satisfies` throws for a malformed version or range instead of returning
`false`; `validRange` returns `false` for a malformed range string.

## Errors

A non-string argument throws `TypeError` and is never coerced. A malformed
version throws `SemVerError` with code `SEMVER_INVALID`, a malformed range with
`SEMVER_RANGE`, an unknown release name or invalid increment identifier with
`SEMVER_INCREMENT`, and arithmetic overflow with `SEMVER_OVERFLOW`. The `code`
is stable; the `message` may add detail and is not stable. `valid` and
`validRange` are the non-throwing checks. Error messages describe the failing
component and never repeat the caller's full input. `neo:semver` has no
filesystem, environment, network, process, or registry failure.

## Limits

The module bounds input before it is copied into native code and rechecks the
same bounds in Rust:

- a version string: 1024 UTF-8 bytes (`MAX_VERSION_BYTES`);
- a range string: 4096 UTF-8 bytes (`MAX_RANGE_BYTES`);
- one core component: 20 bytes (`MAX_CORE_BYTES`);
- prerelease identifiers: at most 32 per version (`MAX_IDENTIFIERS`);
- build identifiers: at most 32 per version (`MAX_IDENTIFIERS`);
- one identifier: at most 256 bytes (`MAX_IDENTIFIER_BYTES`);
- `format` arrays: at most 32 entries each, the same per-identifier bound, and
  the rendered version must be at most `MAX_VERSION_BYTES`;
- a range: at most 32 comparators, in addition to the range byte limit.

The wrapper's precheck counts UTF-16 code units; native code enforces UTF-8
bytes. The accepted grammar is ASCII, so the two agree for every accepted
version or identifier; a non-ASCII string is rejected by at least one check.
The wrapper checks a primitive string's own length, scans a length-bounded
version for identifier count and size, and checks an array's length before
copying; native code repeats the checks and uses checked addition for the
rendered-size total. Exceeding a bound throws `SemVerError` with
`SEMVER_LIMIT`, except that `valid` and `validRange` return `false`. Limits
take precedence over malformed input. This is new behavior for the preserved
`valid`/`compare` functions: an over-limit version now makes `valid` return
`false` and `compare` throw `SEMVER_LIMIT` rather than parsing without a bound.

`format` reads properties from its argument. A getter, Proxy trap, or array
access can run ordinary user code before the returned value is checked; the
module does not claim otherwise. The bounds apply to the primitive strings and
array lengths those reads produce.

Declarations: [`neo:semver`](/api/modules/semver/).
