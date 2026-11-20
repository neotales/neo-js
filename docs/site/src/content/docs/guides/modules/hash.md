---
title: "File and byte digests"
description: "Supported hash algorithms and file access requirements."
---

`neo:hash` provides `digest(algorithm, bytes)` for a `Uint8Array` and, with
`--allow-fs`, `digestFile(algorithm, path)` for a file. Both return lowercase
hexadecimal strings. Supported algorithms are `sha224`, `sha256`, `sha384`,
`sha512`, `sha3-224`, `sha3-256`, `sha3-384`, `sha3-512`, `blake2b-512`,
`blake2s-256`, and `blake3`. File hashing reads in bounded chunks.

Without `--allow-fs`, the module does not export `digestFile`. Import `digest`
alone when the script only hashes bytes.

Declarations: [`neo:hash`](/api/modules/hash/).
