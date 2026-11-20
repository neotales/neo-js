---
title: "TOML and YAML"
description: "TOML and YAML parsing, limits, and errors."
---

`neo:toml` and `neo:yaml` parse configuration text synchronously. They need no
permission flag and grant no host capability: they read no file, consult no
environment, access no network, and execute nothing. Reading a file is a
separate, explicit decision:

```js
import { parse as parseToml } from "neo:toml";
import { parse as parseYaml } from "neo:yaml";
import { readText } from "neo:fs"; // requires --allow-fs

const config = parseToml(readText("config.toml")); // only with --allow-fs
const workflow = parseYaml(readText("workflow.yaml"));
```

The native parse callbacks are private to their modules. They are not installed
on the global object, so replacing a similarly named global cannot forge a
result or call the native function directly.

Both modules return the same JavaScript shapes: objects are null-prototype
records, arrays are arrays, and leaves are strings, booleans, finite numbers,
and null. YAML can produce null; TOML has no null value. Keys such as
`__proto__`, `constructor`, and `prototype` are ordinary own enumerable data
properties, so parsing untrusted content cannot change a result's prototype or
pollute `Object.prototype`. Object keys are not in source order: both parsers
encode objects with sorted keys, and JavaScript then applies its own
enumeration order, with integer-like keys first. Use bracket access or
`Object.keys`, not source order.

TOML values follow these rules:

- TOML date and time values, including offset date-times, local date-times,
  local dates, and local times, are rejected as `TOML_DATETIME`. They are not
  converted to strings or `Date` objects, because JavaScript has no built-in
  exact type for all four TOML forms and a string conversion would silently
  lose the distinction. Handle dates in the caller if they are needed.
- Integers outside ±(2^53 - 1) are rejected as `TOML_INTEGER`; they cannot
  round-trip through a JavaScript number.
- Non-finite floats (`inf`, `-inf`, and `nan`) are rejected as `TOML_FLOAT`.
- Duplicate keys and redefined tables are syntax errors, `TOML_SYNTAX`.
- Empty input is an empty record.
- Depth counts collections, with the root table at depth one. A scalar leaf
  inside the deepest permitted collection does not add another level.

YAML values are deliberately a smaller language than general YAML:

- Exactly one document is accepted. A second document is `YAML_MULTIDOC`.
- Aliases are rejected as `YAML_ALIAS`. Anchors without aliases are ignored.
  Rejecting an alias at its event, before `saphyr-parser` can resolve it, is
  what prevents an alias bomb from expanding: a small document cannot turn into
  a large tree. A recursive or cyclic alias graph is impossible for the same
  reason. Merge keys written with an alias are therefore rejected too. A
  `<<` key without an alias is an ordinary string key; merge semantics are not
  applied.
- Explicit tags, including core-schema tags such as `!!str`, are rejected as
  `YAML_TAG`. Tagged input is never interpreted as an object constructor.
- Mapping keys must resolve to strings. Numeric, boolean, null, sequence, and
  mapping keys are `YAML_KEY`; empty keys are null and are rejected for the
  same reason.
- Duplicate keys are `YAML_DUPLICATE`, not last-value-wins.
- Plain scalars follow the YAML 1.2 core schema: null (`~`, `null`, `Null`,
  `NULL`, or empty), `true`/`false` in the three core spellings, integers,
  finite floats, and strings. `yes`, `no`, `on`, and `off` remain strings, as
  do date-like scalars: the core schema has no timestamp type, so
  `2020-01-01` is a string, not a `Date`. Quoted scalars are always strings.
- YAML integers outside ±(2^53 - 1) are `YAML_INTEGER`; `.inf`, `-.inf`,
  `.nan`, and a finite-looking literal that overflows to infinity are
  `YAML_FLOAT`.
- Empty input, or a stream containing only `---`, is null.

## Limits and errors

Both modules limit source to 1 MiB of UTF-8 bytes, nesting to 64 collection
levels (counting the root collection as one), total nodes to 100,000, entries
in one collection to 100,000, and the JSON response built for JavaScript to
16 MiB. The named constants are exported. The source size is checked before
parsing. Exceeding a limit is `TOML_LIMIT` or `YAML_LIMIT`; exceeding the depth
is `TOML_DEPTH` or `YAML_DEPTH`.

TOML budget enforcement happens in a streaming event pass over the bounded
source before the `toml` crate builds a value tree. That pass lexes the source
into a token vector proportional to the 1 MiB input and consumes parser events
without materialising a tree; only after it accepts does the value tree get
built. A separate preflight-work limit of 100,000 accounting units counts
repeated table prefixes and repeated array-of-tables containers again. Thus,
for example, 50,000 dotted keys sharing one prefix can exceed this work limit
even though the resulting tree contains only 50,002 nodes. Header depth is
also conservative after an array-of-tables header: prior array headers may be
counted as ancestors even when unrelated, causing an early depth rejection.
Preflight depth and work errors take precedence over semantic errors such as
duplicate keys, even when the semantic error appears earlier. The parser still
allocates: the token vector and the accepted tree are
bounded by the source and node limits, but this is not a zero-allocation
parser. The preflight uses the
maintained toml-rs lexer/parser. If that parser rejects text the TOML 1.0 value
parser would accept, the module returns `TOML_SYNTAX` rather than building an
unvalidated tree. Post-parse conversion also checks node and item limits.

YAML consumes `saphyr-parser` events directly and checks budgets before
inserting into the result; aliases are rejected at their event, before any
expansion. Exceeding a budget is reported at the event that crossed it.

Error positions follow that model. A TOML or YAML source-size limit has no
event to point at and reports `line` 1, `column` 1. TOML and YAML preflight or
event budget errors report the event position. A TOML conversion rejection with
no source span (datetime, integer, float, or a post-parse conversion fallback)
reports `line` 0, `column` 0.

A non-string argument throws `TypeError`. Unpaired UTF-16 surrogates are
rejected as `TOML_ENCODING` or `YAML_ENCODING`, and NUL is rejected as
`TOML_NUL` or `YAML_NUL`. The native callback checks the primitive string's
UTF-16 length and exact UTF-8 byte count before conversion. A leading BOM is
ignored. CRLF and lone CR are normalized to LF before parsing, and line and
column positions refer to the normalized text.

Malformed input throws `TomlParseError` or `YamlParseError` with a stable
`code`, a one-based `line`, and a one-based `column` in Unicode scalar values.
Match on `code`, never on the message. Messages are static and never include
source text or parsed values. TOML syntax errors carry the parser's source
span.

Declarations: [`neo:toml`](/api/modules/toml/), [`neo:yaml`](/api/modules/yaml/).
