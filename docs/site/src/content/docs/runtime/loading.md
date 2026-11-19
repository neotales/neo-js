---
title: "Module loading"
description: "Entry files, relative imports, module identity, and format conflicts."
---

The CLI evaluates its entry file as an ECMAScript module. Relative `.js`, `.mjs`, `.ts`, and `.mts` imports require `--allow-fs`. Bare imports are not supported. The [embedding guide](/runtime/embedding/) describes the identity rules for source supplied by a Rust host.

The CLI canonicalizes the entry path and reads that file even without
`--allow-fs`. With permission, relative dependencies are canonicalized before
loading: aliases and cycles use the canonical file identity, and symlinks
and `..` follow filesystem semantics. Missing or inaccessible dependencies are
errors. Canonical paths must be valid UTF-8. Windows behavior is unverified.
The requested spelling, not the canonical target's extension, selects the
format: `.ts` and `.mts` are stripped, `.js` and `.mjs` are JavaScript, and
`.tsx`, `.cts`, `.d.ts`, and `.d.mts` are rejected even through symlinks.
Within one runtime, requesting the same canonical file using different
extensions among `.js`, `.mjs`, `.ts`, and `.mts` is a format conflict,
including after it has been loaded or when it aliases the entry. Same-format
aliases share one module. A later dynamic-import conflict cannot undo earlier
side effects. Canonical paths do not unify hard links.
Load, parse, and resolution failures release the failing module's format
reservation; successfully declared dependencies remain reserved. Once
declared, a module's format stays reserved even if linking or evaluation
fails. If cyclic resolution fails, the runtime disables further module loads
and entry evaluations: QuickJS can otherwise retain a dangling reference to a
freed module. Create a new runtime to recover; already executed side effects
are not undone.
