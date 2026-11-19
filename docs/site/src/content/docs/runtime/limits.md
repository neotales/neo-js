---
title: "Runtime limits"
description: "What neo-js does not provide or isolate."
---

This is a first implementation, not an isolation boundary. It does not limit
CPU or memory use. Do not run untrusted scripts without OS-level isolation.
The runtime has no external event loop. TypeScript support erases types
without checking them or producing source maps. The loader rejects constructs
that need a transform. Host APIs are synchronous. The callback guards in
[`neo:test` and `neo:assert`](/guides/modules/testing/) reject Promise returns.
`neo:toml` and `neo:yaml` parse text only: neither has a file loader, expands
no alias or variable, and reads no environment.
Test support is registration-based: it does not discover test files, isolate
tests, enforce a timeout, or run tests in parallel. Process support is limited
to the synchronous, buffered `exec` and `spawn` calls of `neo:exec` and the
synchronous `neo:process` stream and working-directory API; there are no
asynchronous or streaming child process APIs. Spawned-child handles hold one
slot each until their terminal call or runtime drop, and their results are
buffered rather than streamed. Reading stdin blocks with no timeout, and
`neo:process` stream `close()` is a no-op. The runtime working directory is local to a `NeoJs`
instance and is not a sandbox. The [Windows registry module](/guides/modules/winregistry/)
operates only on Windows. [`neo:path`](/guides/modules/path/) is lexical.
It does not resolve symlinks, enforce containment, or provide file-URL helpers.
The [glob helpers](/guides/modules/path/#glob-matching) do not inspect files.
The raw `RegExp` has no evaluation budget. Path resolution's runtime cwd
fallback requires a matching permission and allowlist entry.
`neo:dotenv` parses text only: it has no file loader, no expansion,
and no environment mutation. Network support is limited to the synchronous
`neo:fetch.download` call: there is no asynchronous or streaming API, no
authentication, cookie, proxy, decompression, resume, or content-type support,
and no SSRF filtering. The scheduler modules generate text only. They do not
write, install, enable, or run units or tasks, and they do not provide a
scheduler daemon. They support only [daily and weekly schedules](/guides/modules/scheduler/).
`neo:semver` follows the Rust `semver` crate: a core component is limited to
`u64::MAX`, and ranges are Cargo-style intersections without `||`, hyphen
ranges, or npm x-range defaults. Numeric prerelease identifiers remain
arbitrary-precision strings; there is no npm `includePrerelease` option.
`neo:crypto` provides only symmetric authenticated encryption; it has no key
store, key derivation, password-based encryption, or public-key operation.
`neo:age` provides X25519 age file encryption but no key store, passphrase,
SSH-key, plugin, or post-quantum recipient support.
