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

## Managed module evaluation

The CLI and the blocking Rust module entry APIs drive QuickJS jobs and
filesystem read workers until the turn settles. This supports top-level
`await` and dynamic imports, not a general external event loop.

| Error                     | Meaning                                                                                                                                                       |
| ------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `NEO_MODULE_DIRTY`        | The runtime already has pending Promises or queued jobs from `eval`, `run`, or the host test runner. The driver does not execute them in a later module turn. |
| `NEO_MODULE_STALLED`      | A live Promise is unresolved and nothing can advance it. This includes a detached dynamic import waiting on top-level `await`.                                |
| `NEO_UNHANDLED_REJECTION` | A rejection remains unhandled when the turn settles. A collected, unhandled rejection still counts.                                                           |
| `NEO_MODULE_LIMIT`        | The driver exceeded 100,000 steps, 4,096 tracked pending Promise identities, or 128 tracked unhandled rejections. These are bounds, not a timeout.            |

The tracker does not keep otherwise collectible Promises or their results
alive. It checks for unreachable cycles before refusing pending work and
checks for any cleanup jobs that collection queued. A live, never-settled
Promise prevents quiescence even if it cannot resume.

A dirty entry, stall, exhausted step or pending-Promise budget, failed turn
with unfinished work, or Rust panic during a turn abandons the runtime.
Subsequent entry points report `NEO_MODULE_ABORTED`. The driver does not run
remaining reactions. On Rust unwind the owner drops settlement handles
without calling JavaScript. A read already in an OS call can delay teardown.
See [Migrate binary reads](/guides/modules/fs/#migrate-binary-reads).
