---
title: "AES-256-GCM encryption"
description: "The neo-js envelope format, limits, and encryption errors."
---

`neo:crypto` provides synchronous authenticated encryption with AES-256-GCM.
It needs no permission flag and grants no host capability. Its native wrapper
and callbacks are private to the module: there are no `__neoCrypto*` globals,
but this is not isolation from other code in the same realm that can access
the caller's values.

```js
import { decrypt, encrypt, generateKey } from "neo:crypto";

const key = generateKey();
const plaintext = Uint8Array.from([0x68, 0x65, 0x6c, 0x6c, 0x6f]);
const context = Uint8Array.from([0x61, 0x2d, 0x64]);
const envelope = encrypt(key, plaintext, context);
const returned = decrypt(key, envelope, context);
```

The key is exactly 32 bytes. Plaintext, associated data, and the returned
envelope are `Uint8Array`s; bytes are never interpreted as text. The runtime
has no `TextEncoder` or `TextDecoder`, so callers encode text themselves.
`associatedData` is optional, authenticated but not encrypted, and must be
supplied unchanged to `decrypt`.

Every `encrypt` call draws a fresh 96-bit nonce from the operating system
CSPRNG through `getrandom`. The envelope is:

| offset | bytes | field                        |
| ------ | ----- | ---------------------------- |
| 0      | 4     | magic, ASCII `NEO1`          |
| 4      | 1     | version, `1`                 |
| 5      | 1     | algorithm, `1` (AES-256-GCM) |
| 6      | 12    | nonce                        |
| 18     | n     | ciphertext                   |
| 18+n   | 16    | authentication tag           |

The first 18 bytes and the caller's associated data are authenticated as
AES-GCM associated data. Changing any envelope byte, using another key, or
supplying different associated data makes `decrypt` throw; plaintext is not
returned from a failed check. An unknown magic, version, or algorithm is
reported as malformed.

Nonce collision risk grows with the number of messages under one key: a random
96-bit nonce reaches about 2^-32 collision probability at roughly 2^32
encryptions. Retire a key well before that. The caller cannot choose a nonce,
so the module removes the common reuse mistake, but key lifetime remains the
caller's responsibility.

`generateKey()` returns 32 CSPRNG bytes. Keep the key secret, use a distinct
key per purpose, and protect where it is stored. The module does not store,
derive, wrap, rotate, or persist keys. It does not promise zeroization: key and
plaintext bytes may remain in Rust buffers and the JavaScript heap until
reclaimed. It is not a sandbox.

The neo-js module limits plaintext and caller associated data to 8 MiB each,
and envelopes to 8 MiB + 34 bytes. These are neo-js limits, not intrinsic
AES-GCM limits. Native code checks these sizes before copying inputs or
allocating input buffers. The core uses checked arithmetic and fallible
reservations; a refused reservation is reported as `NEO_CRYPTO_LIMIT`. That
mapping is best effort: an allocation failure inside QuickJS, the Rust
allocator, or the JavaScript heap can still abort the process or raise an
error the module cannot catch, so recovery from out-of-memory is not
guaranteed.

An argument that is not a `Uint8Array` throws `TypeError`. Other failures are
`Error` objects with a stable `code`: `NEO_CRYPTO_KEY` (key length),
`NEO_CRYPTO_FORMAT` (truncated or unknown envelope), `NEO_CRYPTO_AUTH` (wrong
key, wrong associated data, or tampering), `NEO_CRYPTO_RANDOM` (OS CSPRNG
failure), `NEO_CRYPTO_LIMIT` (an input above a documented size bound, or a
refused allocation), or `NEO_CRYPTO_DETACHED` (detached or out-of-bounds
view). Messages never include key, plaintext, or ciphertext bytes.

The envelope is specific to neo-js. The AES-256-GCM primitive is standard and
is checked against published NIST vectors; the envelope is also checked
against a fixture produced independently with Node/OpenSSL. Another tool must
reproduce the envelope and associated-data construction to interoperate. For
standard age file encryption, see [`neo:age`](/guides/modules/age/).

Declarations: [`neo:crypto`](/api/modules/crypto/).
