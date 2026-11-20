---
title: "Dotenv"
description: "Dotenv syntax, limits, and parse errors."
---

`neo:dotenv` parses `.env` content strings synchronously. It needs no
permission flag and grants no host capability: it reads no file, consults or
mutates no process environment, expands no variables, and runs no shell
command. Reading a `.env` file is a separate, explicit decision:

```js
import { parse } from "neo:dotenv";
import { readText } from "neo:fs"; // requires --allow-fs

const values = readText(".env"); // only with --allow-fs
const config = parse(values);
```

The native parse callback is private to the module. It is not installed on the
global object, so replacing a similarly named global cannot forge a result or
call the native function directly.

`parse(source)` returns a null-prototype record. Keys such as `__proto__`,
`constructor`, and `prototype` are ordinary own enumerable data properties, so
parsing untrusted content cannot change the result's prototype or pollute
`Object.prototype`. Duplicate keys are last-value-wins. Property enumeration is
JavaScript's own order, not source order: array-index keys (canonical numeric
strings) come first in ascending numeric order, then other string keys in
first-insertion order. Source order `10, 2, b, a` enumerates as `2, 10, b, a`.
Use `Object.keys`, bracket access, or `{ ...config }` when a normal object is
wanted.

## Syntax

- A leading UTF-8 BOM is ignored. CRLF and lone CR are normalized to LF before
  parsing, including inside quoted values. NUL is rejected anywhere, including
  a `\u0000` or `\U00000000` escape.
- Only ASCII space and tab are whitespace. Blank lines and lines whose first
  non-space character is `#` are ignored.
- `export` is a prefix only when the exact, case-sensitive token is followed by
  a space or tab, so `export A=1` defines `A` while `export=1` defines `export`.
- A key is one or more ASCII letters, digits, or `_`. It must not be empty and
  is limited to 1,024 UTF-8 bytes. Spaces between a key and `=` are allowed.
- An unquoted value runs to the line end, with trailing spaces and tabs
  trimmed. `#` starts a comment only at the start of the value or after a space
  or tab: `PASSWORD=abc#123` keeps `abc#123`, while `VALUE=abc # note` is
  `abc`. A line without `=` is an error.
- A single-quoted value removes the outer quotes; only `\'` is an escape, and
  every other backslash is literal.
- A double-quoted value removes the outer quotes and decodes `\n`, `\r`, `\t`,
  `\b`, `\\`, `\"`, `\'`, a backslash before a backtick, `\uXXXX`, and
  `\UXXXXXXXX`. An unknown escape such as `\q` stays literal; a malformed or
  surrogate `\u`/`\U` escape is an error.
- A backtick-delimited value is raw: the outer backticks are removed, no escape
  is decoded, and the contents are never executed.

  ```dotenv
  KEY=`echo hi`
  ```

  The value of `KEY` above is the literal text `echo hi`.

- Unquoted values have no escape processing; a backslash is an ordinary
  character.
- Quotes may span lines. After a closing quote, only spaces, tabs, and an
  optional `#` comment may follow the line. An unterminated quote is an error.

Values are never expanded. `$HOME`, `${HOME:-/tmp}`, and `$(command)` are
returned exactly as written. With `--allow-env`, expansion is something the
caller opts into separately. Because command substitution is not interpreted,
a double-quoted value ends at the first unescaped `"` even inside `$(...)`; use
`\"` there, or a single-quoted or backtick-delimited value, if the text contains
an unescaped quote.

## Errors and limits

The first error in source order throws a `DotenvParseError` with a stable
`code`, one-based `line`, and one-based `column` in Unicode scalar values.
Match on `code`, never on the message; messages never include source text or
values. Codes are `DOTENV_SYNTAX`, `DOTENV_KEY`, `DOTENV_QUOTE`,
`DOTENV_ESCAPE`, `DOTENV_LIMIT`, `DOTENV_ENCODING`, and `DOTENV_NUL`. A
non-string argument throws `TypeError`, and an unpaired UTF-16 surrogate is
rejected as `DOTENV_ENCODING`. Source is limited to 1 MiB of UTF-8 bytes before
line-ending normalization, 16,384 assignments, and 1,024-byte keys; exceeding a
limit is `DOTENV_LIMIT`. An over-limit source reports `DOTENV_LIMIT` before
parsing, so the size check takes precedence over syntax. The native callback
checks the primitive string's UTF-16 length before conversion and the exact
UTF-8 byte count before parsing.

Deliberate differences from `@neotales/dotenv`: this module has no
`stringify`, document token stream, `load`, or implicit expansion. It rejects
unterminated quotes, bare lines without `=`, and malformed `\u`/`\U` escapes
that the reference parser would keep as text. It also keeps `__proto__` as a
key instead of assigning it through `Object.prototype` and losing it. The
reference treats any `#` in an unquoted value as a comment; this parser
requires whitespace before `#` so a value that contains `#` is not silently
truncated.

Declarations: [`neo:dotenv`](/api/modules/dotenv/).
