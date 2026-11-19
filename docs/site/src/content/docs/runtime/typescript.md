---
title: "TypeScript support"
description: "Supported type syntax, loader diagnostics, and editor type checking."
---

For editor completion and offline type checking, open the
typed script example at `examples/typed-script/` in VS Code or another
TypeScript language server. Its `tsconfig.json` loads the checked-in
`types/neo-js.d.ts` declarations without installing a package.

A `.ts` or `.mts` entry, and any relative `.ts` or `.mts` import, is stripped
to JavaScript before QuickJS sees it. Stripping erases type syntax; it does
not check types and does not compile anything. A `.js` or `.mjs` file is
evaluated byte for byte, so type syntax in it is a syntax error, as before.

Erased: annotations, `interface`, `type`, generic parameters and arguments,
`as`, `as const`, `satisfies`, `<T>expr`, non-null assertions, `implements`,
`abstract` members, `declare` fields, access modifiers, overload signatures,
`import type`, `export type`, inline `type` specifiers, and ambient
declarations such as `declare namespace` and `declare enum`. An import whose
specifiers are all `type` is erased whole, so a module that only holds types
does not have to exist at run time. Value imports, side-effect imports, and
their specifiers are kept, so a `.ts` specifier reaches the loader unchanged.
A type-only import whose name is also declared in the same module is a
`TS_SEMANTIC` error, as in TypeScript, and it is reported before anything runs
rather than hidden by the erasure.

Rejected with a `TS_UNSUPPORTED` diagnostic instead of being transformed:
`enum` and `const enum`, `namespace` and `module` declarations with a body,
constructor parameter properties, decorators, `import x = ...`, `export = ...`,
and `export as namespace`. Precompile those with `tsc` (targeting ES2022 or
later) and run the output as JavaScript.

Diagnostics are one line each, `<path>:<line>:<column>: <message> [<code>]`,
with code `TS_PARSE`, `TS_SEMANTIC`, `TS_UNSUPPORTED`, `TS_TRANSFORM`, or
`TS_EXTENSION`, and are reported before any of the module runs. Only the first
is reported. Early semantic errors, such as a duplicate lexical declaration, are
reported before any module code runs.

Output is regenerated from the parsed module. Formatting, blank lines, and a
comment that sat only on an erased node are not preserved, and no source map is
produced: a stack trace names the generated JavaScript position, not the
original TypeScript. `.cts` and `.tsx` modules and `.d.ts` declaration files
are rejected; JSX is not supported.

neo-js pins Oxc 0.81.0, the newest Oxc release whose full dependency graph
compiles on Rust 1.87. `Cargo.lock` holds two transitive dependencies back for
the same reason: `textwrap` 0.16.2 (0.16.4 needs Rust 1.88) and
`oxc_sourcemap` 4.1.0 (4.2 needs Rust 1.89). Keep both pins when updating.
