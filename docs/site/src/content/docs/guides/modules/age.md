---
title: "age file encryption"
description: "X25519 identities, binary age files, and encryption errors."
---

`neo:age` provides synchronous age file encryption for X25519 recipients. It
needs no permission flag and grants no host capability. Its native wrapper and
callbacks are private to the module: there are no `__neoAge*` globals, but this
is not isolation from other code in the same realm that can access the caller's
values.

```js
import { decrypt, encrypt, generateIdentity } from "neo:age";

const { identity, recipient } = generateIdentity();
const plaintext = Uint8Array.from([0x68, 0x65, 0x6c, 0x6c, 0x6f]);
const ciphertext = encrypt(recipient, plaintext);
const returned = decrypt(identity, ciphertext);
```

`generateIdentity()` returns `{ identity, recipient }` as standard Bech32 text:
`AGE-SECRET-KEY-1...` and `age1...`. `identityToRecipient(identity)` derives the
recipient from an existing identity. Keep the identity secret; the module does
not store, wrap, rotate, or persist it. `generateIdentity` and `encrypt` ask the
vetted Rust `age` crate for randomness, which draws from the operating system
CSPRNG. age 0.11 exposes no fallible or injectable RNG path, so the module
contains panics from the age call and reports them as `NEO_AGE_RANDOM`. The
known panic cause is OS RNG failure; a `panic = "abort"` build cannot contain
it and aborts instead.

`encrypt(recipient, plaintext)` returns a standard binary age v1 file encrypted
to one X25519 recipient, and `decrypt(identity, ciphertext)` accepts standard
binary age v1 files encrypted to an X25519 recipient. Plaintext and ciphertext
are `Uint8Array`s; bytes are never interpreted as text. The runtime has no
`TextEncoder` or `TextDecoder`, so callers encode text themselves. ASCII armor,
passphrase encryption, SSH keys, plugins, and post-quantum recipients are not
exposed. The `age` command-line tool can decrypt this module's output, and this
module decrypts its binary X25519 output.

The test suite decrypts a fixture produced independently with the `age` 1.3.2
command-line tool and checks the module's output is a binary age v1 file. The
reverse direction was also verified during development: `age` 1.3.2 decrypted
this module's output byte for byte. The module does not promise zeroization.
Identity text, plaintext, ciphertext, and intermediate key material may remain
in Rust buffers and the JavaScript heap until reclaimed. No secret is logged or
included in an error message.

The neo-js module limits plaintext to 8 MiB and a complete age file to 9 MiB.
Decryption also stops if the recovered plaintext would exceed 8 MiB. Identity
and recipient text longer than 512 bytes is rejected as malformed; the original
UTF-8 byte length is checked before trimming, and a native UTF-16 length check
keeps any conversion small, so huge padded keys are rejected without being
copied. These are neo-js limits, not intrinsic age limits: a valid external age
file with an unusually large header can be refused. Native code bounds inputs
before copying them, and decryption reads in bounded chunks. Core functions
use checked arithmetic, a bounded writer, and fallible reservations; a refused
reservation is reported as `NEO_AGE_LIMIT`. That mapping is best effort: an
allocation failure inside QuickJS, the Rust allocator, or the JavaScript heap can
still abort the process or raise an error the module cannot catch, so recovery
from out-of-memory is not guaranteed.

An argument that is not a string or `Uint8Array` throws `TypeError`. Other
failures are `Error` objects with a stable `code`: `NEO_AGE_IDENTITY`
(malformed identity text), `NEO_AGE_RECIPIENT` (malformed recipient text),
`NEO_AGE_FORMAT` (truncated, unknown, or unsupported age file),
`NEO_AGE_IDENTITY_MISMATCH` (no supplied identity matched), `NEO_AGE_AUTH`
(a header MAC or payload tag did not verify), `NEO_AGE_LIMIT` (a documented
size bound or a refused allocation), `NEO_AGE_RANDOM` (OS CSPRNG failure or a
contained panic in the age encryption path), `NEO_AGE_DETACHED` (detached or
out-of-bounds view), or `NEO_AGE_INTERNAL` (an unexpected failure from the age
crate). A damaged recipient stanza is indistinguishable from a file encrypted
to a different identity, so both are reported as `NEO_AGE_IDENTITY_MISMATCH`.
Payload tampering and header-MAC failures are `NEO_AGE_AUTH`. The age
implementation is pinned to 0.11; `Cargo.toml` also pins `rust-embed` 8.11.0
and its helpers because later 8.x releases need Rust 1.88 through `sha2`
0.11.

Declarations: [`neo:age`](/api/modules/age/).
