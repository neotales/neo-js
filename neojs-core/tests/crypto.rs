//! Integration tests for `neo:crypto` through the public `NeoJs` API.
//!
//! The module is pure synchronous cryptography, so every test uses the default
//! (no) permissions. The tests exercise the JavaScript surface and its error
//! codes; the Rust unit tests cover the primitive and the envelope internals.

use neojs_core::{NeoJs, Permissions};

fn runtime() -> NeoJs {
    NeoJs::new(Permissions::default()).unwrap()
}

#[test]
fn crypto_is_available_without_permissions_and_roundtrips_bytes() {
    let js = runtime();
    js.run_module(
        "crypto-available.js",
        "import { decrypt, encrypt, generateKey } from 'neo:crypto';
         if (typeof encrypt !== 'function' || typeof decrypt !== 'function' ||
             typeof generateKey !== 'function') throw Error('missing exports');
         if ('__neoCryptoEncrypt' in globalThis || '__neoCryptoDecrypt' in globalThis ||
             '__neoCryptoGenerateKey' in globalThis) throw Error('global callback exposed');
         if (typeof fs !== 'undefined' || typeof env !== 'undefined') throw Error('capability leaked');

         const key = generateKey();
         if (!(key instanceof Uint8Array)) throw Error('key is not a Uint8Array');
         if (key.length !== 32) throw Error('key length');

         const plaintext = Uint8Array.from([0, 1, 2, 0, 255, 128, 7]);
         const envelope = encrypt(key, plaintext);
         if (!(envelope instanceof Uint8Array)) throw Error('envelope is not a Uint8Array');
         if (envelope.length !== plaintext.length + 34) throw Error('envelope length');

         const returned = decrypt(key, envelope);
         if (returned.length !== plaintext.length) throw Error('plaintext length');
         for (let i = 0; i < plaintext.length; i++) {
             if (returned[i] !== plaintext[i]) throw Error('plaintext byte ' + i);
         }
         if (key.length !== 32) throw Error('key was mutated');",
    )
    .unwrap();
}

#[test]
fn crypto_handles_empty_unicode_binary_and_large_payloads() {
    let js = runtime();
    js.run_module(
        "crypto-data.js",
        "import { decrypt, encrypt, generateKey } from 'neo:crypto';
         const key = generateKey();
         const aad = Uint8Array.from([0x63, 0x6f, 0x6e, 0x74, 0x65, 0x78, 0x74]);

         // UTF-8 for 'Grüße, 世界, 🦀', including two-, three-, and
         // four-byte sequences. The runtime has no TextEncoder, so the
         // bytes are supplied explicitly.
         const unicode = Uint8Array.from([
             0x47, 0x72, 0xc3, 0xbc, 0xc3, 0x9f, 0x65, 0x2c, 0x20,
             0xe4, 0xb8, 0x96, 0xe7, 0x95, 0x8c, 0x2c, 0x20,
             0xf0, 0x9f, 0xa6, 0x80,
         ]);
         const unicodeOut = decrypt(key, encrypt(key, unicode, aad), aad);
         if (unicodeOut.length !== unicode.length) throw Error('unicode length');
         for (let i = 0; i < unicode.length; i++) {
             if (unicodeOut[i] !== unicode[i]) throw Error('unicode byte ' + i);
         }

         // Empty plaintext still has a full header, nonce, and tag.
         const empty = encrypt(key, new Uint8Array(), aad);
         if (empty.length !== 34) throw Error('empty envelope length');
         if (decrypt(key, empty, aad).length !== 0) throw Error('empty plaintext');

         // Every byte value survives, including NUL.
         const all = new Uint8Array(256);
         for (let i = 0; i < 256; i++) all[i] = i;
         const allOut = decrypt(key, encrypt(key, all, aad), aad);
         if (allOut.length !== 256) throw Error('binary length');
         for (let i = 0; i < 256; i++) {
             if (allOut[i] !== i) throw Error('binary byte ' + i);
         }

         // 1 MiB plus a few bytes.
         const large = new Uint8Array(1024 * 1024 + 3);
         for (let i = 0; i < large.length; i++) large[i] = (i * 31 + 7) & 255;
         const largeOut = decrypt(key, encrypt(key, large), new Uint8Array());
         if (largeOut.length !== large.length) throw Error('large length');
         for (let i = 0; i < large.length; i++) {
             if (largeOut[i] !== large[i]) throw Error('large byte ' + i);
         }",
    )
    .unwrap();
}

#[test]
fn crypto_respects_typed_array_views() {
    let js = runtime();
    js.run_module(
        "crypto-views.js",
        "import { decrypt, encrypt } from 'neo:crypto';
         // A key view at a non-zero byte offset in a larger buffer.
         const keyBytes = new Uint8Array(34);
         keyBytes[0] = 0xff;
         for (let i = 1; i <= 32; i++) keyBytes[i] = i;
         keyBytes[33] = 0xff;
         const key = keyBytes.subarray(1, 33);
         if (key.length !== 32) throw Error('key view length');

         // A plaintext view that starts after a sentinel byte and excludes
         // the trailing byte.
         const backing = Uint8Array.from([0xaa, 0xbb, 0x00, 0x01, 0xfe, 0xcc]);
         const plaintext = backing.subarray(2, 5);
         const envelope = encrypt(key, plaintext);
         const out = decrypt(key, envelope);
         if (out.length !== 3) throw Error('view length');
         if (out[0] !== 0x00 || out[1] !== 0x01 || out[2] !== 0xfe) {
             throw Error('view bytes');
         }

         // The same for associated data.
         const aadBacking = Uint8Array.from([0x11, 0x61, 0x62, 0x63, 0x22]);
         const aad = aadBacking.subarray(1, 4);
         const withAad = encrypt(key, plaintext, aad);
         if (decrypt(key, withAad, aad).length !== 3) throw Error('aad view');",
    )
    .unwrap();
}

#[test]
fn crypto_uses_a_fresh_nonce_per_encryption() {
    let js = runtime();
    js.run_module(
        "crypto-nonces.js",
        "import { encrypt, generateKey } from 'neo:crypto';
         const key = generateKey();
         const plaintext = Uint8Array.from([1, 2, 3]);
         const seen = new Set();
         for (let i = 0; i < 500; i++) {
             const envelope = encrypt(key, plaintext);
             if (envelope.length !== 37) throw Error('length');
             let nonce = '';
             let zero = true;
             for (let j = 6; j < 18; j++) {
                 nonce += String.fromCharCode(envelope[j]);
                 if (envelope[j] !== 0) zero = false;
             }
             if (zero) throw Error('nonce was all zero');
             if (seen.has(nonce)) throw Error('nonce repeated at ' + i);
             seen.add(nonce);
         }",
    )
    .unwrap();
}

#[test]
fn crypto_rejects_wrong_keys_tampering_and_bad_arguments() {
    let js = runtime();
    js.run_module(
        "crypto-errors.js",
        "import { decrypt, encrypt, generateKey } from 'neo:crypto';
         const key = generateKey();
         const other = generateKey();
         const plaintext = Uint8Array.from([1, 2, 3]);
         const aad = Uint8Array.from([9, 8, 7]);
         const envelope = encrypt(key, plaintext, aad);

         function expectCode(fn, code) {
             try {
                 fn();
             } catch (error) {
                 if (error.code !== code) {
                     throw Error('expected ' + code + ', got ' + error.code + ': ' + error.message);
                 }
                 return;
             }
             throw Error('expected ' + code + ' but no error was thrown');
         }

         function expectTypeError(fn) {
             try {
                 fn();
             } catch (error) {
                 if (!(error instanceof TypeError)) throw Error('expected TypeError: ' + error);
                 return;
             }
             throw Error('expected TypeError but no error was thrown');
         }

         // A wrong key of the same length and different AAD both look like
         // authentication failures; no plaintext is returned.
         expectCode(() => decrypt(other, envelope, aad), 'NEO_CRYPTO_AUTH');
         expectCode(() => decrypt(key, envelope, Uint8Array.from([9, 8, 6])), 'NEO_CRYPTO_AUTH');
         expectCode(() => decrypt(key, envelope, new Uint8Array()), 'NEO_CRYPTO_AUTH');
         expectCode(() => decrypt(key, envelope), 'NEO_CRYPTO_AUTH');

         // Every single-bit change in the envelope is rejected.
         for (let i = 0; i < envelope.length; i++) {
             for (let bit = 0; bit < 8; bit++) {
                 const copy = envelope.slice();
                 copy[i] ^= 1 << bit;
                 try {
                     decrypt(key, copy, aad);
                     throw Error('tamper at ' + i + '/' + bit + ' was accepted');
                 } catch (error) {
                     if (error.code !== 'NEO_CRYPTO_AUTH' && error.code !== 'NEO_CRYPTO_FORMAT') {
                         throw error;
                     }
                 }
             }
         }

         // Key length is enforced before any cryptography.
         expectCode(() => encrypt(new Uint8Array(31), plaintext), 'NEO_CRYPTO_KEY');
         expectCode(() => encrypt(new Uint8Array(33), plaintext), 'NEO_CRYPTO_KEY');
         expectCode(() => decrypt(new Uint8Array(16), envelope, aad), 'NEO_CRYPTO_KEY');

         // Truncated and unknown-format envelopes are malformed, not
         // authentication failures.
         expectCode(() => decrypt(key, new Uint8Array()), 'NEO_CRYPTO_FORMAT');
         expectCode(() => decrypt(key, new Uint8Array(33)), 'NEO_CRYPTO_FORMAT');
         const badMagic = envelope.slice();
         badMagic[0] = 0;
         expectCode(() => decrypt(key, badMagic, aad), 'NEO_CRYPTO_FORMAT');
         const badVersion = envelope.slice();
         badVersion[4] = 2;
         expectCode(() => decrypt(key, badVersion, aad), 'NEO_CRYPTO_FORMAT');
         const badAlgorithm = envelope.slice();
         badAlgorithm[5] = 2;
         expectCode(() => decrypt(key, badAlgorithm, aad), 'NEO_CRYPTO_FORMAT');

         // The JavaScript wrapper rejects non-Uint8Array arguments.
         expectTypeError(() => encrypt('key', plaintext));
         expectTypeError(() => encrypt(key, [1, 2, 3]));
         expectTypeError(() => encrypt(key, plaintext, 'aad'));
         expectTypeError(() => encrypt(key, plaintext, null));
         expectTypeError(() => decrypt(key, 'envelope'));
         expectTypeError(() => decrypt(key, envelope, 'aad'));",
    )
    .unwrap();
}

#[test]
fn crypto_errors_do_not_echo_inputs() {
    let js = runtime();
    js.run_module(
        "crypto-error-text.js",
        "import { decrypt, encrypt } from 'neo:crypto';
         const key = new Uint8Array(32).fill(0x41);
         const secret = Uint8Array.from([0xde, 0xad, 0xbe, 0xef, 0x00, 0x99]);
         const envelope = encrypt(key, secret);
         let failure;
         try {
             decrypt(new Uint8Array(32).fill(0x42), envelope);
         } catch (error) {
             failure = error;
         }
         if (!failure) throw Error('expected failure');
         if (failure.code !== 'NEO_CRYPTO_AUTH') throw Error('wrong code');
         const text = String(failure.message);
         if (text.includes('AAAA') || text.includes('dead') ||
             text.includes('\\u0000') || text.includes('99')) {
             throw Error('error message echoed input');
         }",
    )
    .unwrap();
}

#[test]
fn crypto_error_codes_survive_inherited_code_setters() {
    let js = runtime();
    js.run_module(
        "crypto-code-setters.js",
        r#"
        import { decrypt, encrypt, generateKey } from 'neo:crypto';
        const RealError = Error;

        function expectStableKeyError(phase) {
            let failure;
            try {
                decrypt(new Uint8Array(16), new Uint8Array(34));
            } catch (error) {
                failure = error;
            }
            if (!(failure instanceof RealError)) {
                throw new RealError(phase + ': wrong error type: ' + failure);
            }
            if (failure.code !== 'NEO_CRYPTO_KEY') {
                throw new RealError(phase + ': stable code lost: ' + failure.code);
            }
            if (String(failure.message).includes('inherited code setter')) {
                throw new RealError(phase + ': inherited setter ran');
            }
        }

        expectStableKeyError('no mutation');
        Object.defineProperty(Object.prototype, 'code', {
            __proto__: null,
            set() { throw new RealError('inherited code setter ran'); },
            configurable: true,
        });
        expectStableKeyError('throwing setter');
        Object.defineProperty(Object.prototype, 'code', {
            __proto__: null,
            set() {},
            configurable: true,
        });
        expectStableKeyError('no-op setter');
        delete Object.prototype.code;

        const key = generateKey();
        const plaintext = Uint8Array.from([9, 8, 7]);
        const out = decrypt(key, encrypt(key, plaintext));
        if (out.length !== 3 || out[2] !== 7) throw new RealError('roundtrip');
        "#,
    )
    .unwrap();
}

#[test]
fn crypto_globals_cannot_intercept_before_import() {
    let js = runtime();
    js.run_module(
        "crypto-before-import.js",
        r#"
        const RealUint8Array = Uint8Array;
        const RealError = Error;
        const RealTypeError = TypeError;
        let encryptCalls = 0;
        let decryptCalls = 0;
        let keyCalls = 0;
        globalThis.__neoCryptoEncrypt = function () {
            encryptCalls++;
            return { ok: true, value: new RealUint8Array(34) };
        };
        globalThis.__neoCryptoDecrypt = function () {
            decryptCalls++;
            return { ok: true, value: new RealUint8Array(0) };
        };
        globalThis.__neoCryptoGenerateKey = function () {
            keyCalls++;
            return { ok: true, value: new RealUint8Array(32) };
        };
        let fakeErrorCalled = false;
        let fakeTypeErrorCalled = false;
        globalThis.Uint8Array = function FakeUint8Array() {};
        globalThis.Error = function FakeError() { fakeErrorCalled = true; };
        globalThis.TypeError = function FakeTypeError() { fakeTypeErrorCalled = true; };

        const { decrypt, encrypt, generateKey } = await import('neo:crypto');
        const key = new RealUint8Array(32);
        for (let i = 0; i < 32; i++) key[i] = i;
        const plaintext = RealUint8Array.from([0, 1, 2, 255]);
        const envelope = encrypt(key, plaintext);
        const out = decrypt(key, envelope);
        if (out.length !== 4) throw new RealError('length');
        for (let i = 0; i < 4; i++) {
            if (out[i] !== plaintext[i]) throw new RealError('bytes');
        }
        const generated = generateKey();
        if (!(generated instanceof RealUint8Array) || generated.length !== 32) {
            throw new RealError('generated key');
        }
        if (encryptCalls || decryptCalls || keyCalls) {
            throw new RealError('global interceptor was called');
        }
        try {
            decrypt(key, new RealUint8Array(34));
            throw new RealError('unexpected success');
        } catch (error) {
            if (fakeErrorCalled) throw new RealError('fake Error used');
            if (!(error instanceof RealError)) throw new RealError('wrong error type');
            if (error.code !== 'NEO_CRYPTO_FORMAT' && error.code !== 'NEO_CRYPTO_AUTH') {
                throw new RealError('wrong code');
            }
        }
        try {
            encrypt(key, [1, 2, 3]);
            throw new RealError('unexpected success');
        } catch (error) {
            if (fakeTypeErrorCalled) throw new RealError('fake TypeError used');
            if (!(error instanceof RealTypeError)) throw new RealError('wrong type error');
        }
        "#,
    )
    .unwrap();
}

#[test]
fn crypto_globals_cannot_intercept_after_import() {
    let js = runtime();
    js.run_module(
        "crypto-after-import.js",
        r#"
        import { decrypt, encrypt, generateKey } from 'neo:crypto';
        if ('__neoCryptoEncrypt' in globalThis || '__neoCryptoDecrypt' in globalThis ||
            '__neoCryptoGenerateKey' in globalThis) {
            throw Error('global callback exposed');
        }
        const RealUint8Array = Uint8Array;
        const RealError = Error;
        const RealTypeError = TypeError;
        const key = new RealUint8Array(32);
        for (let i = 0; i < 32; i++) key[i] = i + 1;
        const plaintext = RealUint8Array.from([9, 8, 7, 0]);
        const envelope = encrypt(key, plaintext);

        let encryptCalls = 0;
        let decryptCalls = 0;
        let keyCalls = 0;
        globalThis.__neoCryptoEncrypt = function () {
            encryptCalls++;
            return { ok: true, value: new RealUint8Array(0) };
        };
        globalThis.__neoCryptoDecrypt = function () {
            decryptCalls++;
            return { ok: true, value: new RealUint8Array(0) };
        };
        globalThis.__neoCryptoGenerateKey = function () {
            keyCalls++;
            return { ok: true, value: new RealUint8Array(0) };
        };
        let fakeErrorCalled = false;
        let fakeTypeErrorCalled = false;
        globalThis.Uint8Array = function FakeUint8Array() {};
        globalThis.Error = function FakeError() { fakeErrorCalled = true; };
        globalThis.TypeError = function FakeTypeError() { fakeTypeErrorCalled = true; };

        const out = decrypt(key, envelope);
        if (out.length !== 4) throw new RealError('length');
        for (let i = 0; i < 4; i++) {
            if (out[i] !== plaintext[i]) throw new RealError('bytes');
        }
        const second = encrypt(key, plaintext);
        if (decrypt(key, second).length !== 4) throw new RealError('second roundtrip');
        const generated = generateKey();
        if (!(generated instanceof RealUint8Array) || generated.length !== 32) {
            throw new RealError('generated key');
        }
        if (encryptCalls || decryptCalls || keyCalls) {
            throw new RealError('global interceptor was called');
        }
        try {
            decrypt(key, new RealUint8Array(34));
            throw new RealError('unexpected success');
        } catch (error) {
            if (fakeErrorCalled) throw new RealError('fake Error used');
            if (!(error instanceof RealError)) throw new RealError('wrong error type');
        }
        try {
            encrypt(key, [1]);
            throw new RealError('unexpected success');
        } catch (error) {
            if (fakeTypeErrorCalled) throw new RealError('fake TypeError used');
            if (!(error instanceof RealTypeError)) throw new RealError('wrong type error');
        }
        "#,
    )
    .unwrap();
}

#[test]
fn crypto_host_rejects_non_typed_arrays_even_if_instanceof_lies() {
    let js = runtime();
    js.run_module(
        "crypto-hasinstance.js",
        r#"
        import { encrypt } from 'neo:crypto';
        const RealUint8Array = Uint8Array;
        const RealError = Error;
        Object.defineProperty(RealUint8Array, Symbol.hasInstance, {
            value: () => true,
        });
        const key = new RealUint8Array(32);
        let rejected = false;
        try {
            encrypt(key, [1, 2, 3]);
        } catch (_) {
            rejected = true;
        }
        if (!rejected) throw new RealError('host accepted a non-typed-array');
        "#,
    )
    .unwrap();
}

#[test]
fn crypto_rejects_inputs_above_the_documented_limits() {
    let js = runtime();
    js.run_module(
        "crypto-limits.js",
        r#"
        import { decrypt, encrypt } from 'neo:crypto';
        const MAX_PLAINTEXT = 8 * 1024 * 1024;
        const MAX_AAD = 8 * 1024 * 1024;
        const MAX_ENVELOPE = MAX_PLAINTEXT + 34;
        const key = new Uint8Array(32);
        const small = new Uint8Array(1);
        // One allocation, viewed at the three public bounds.
        const big = new Uint8Array(MAX_ENVELOPE + 1);
        const overPlaintext = big.subarray(0, MAX_PLAINTEXT + 1);
        const overAad = big.subarray(0, MAX_AAD + 1);
        const boundaryEnvelope = big.subarray(0, MAX_ENVELOPE);
        function expectLimit(fn) {
            try {
                fn();
            } catch (error) {
                if (error.code !== 'NEO_CRYPTO_LIMIT') throw error;
                return;
            }
            throw Error('expected NEO_CRYPTO_LIMIT');
        }
        expectLimit(() => encrypt(key, overPlaintext));
        expectLimit(() => encrypt(key, small, overAad));
        expectLimit(() => decrypt(key, big));
        // Exactly at the envelope bound is not a limit error; the zero-filled
        // header is a format error.
        let format = false;
        try {
            decrypt(key, boundaryEnvelope);
        } catch (error) {
            format = error.code === 'NEO_CRYPTO_FORMAT';
        }
        if (!format) throw Error('boundary envelope not format-checked');
        "#,
    )
    .unwrap();
}

#[test]
fn crypto_decrypts_an_independently_generated_envelope() {
    let js = runtime();
    js.run_module(
        "crypto-interop.js",
        r#"
        import { decrypt } from 'neo:crypto';
        function fromHex(value) {
            const out = new Uint8Array(value.length / 2);
            for (let i = 0; i < out.length; i++) {
                out[i] = parseInt(value.substring(i * 2, i * 2 + 2), 16);
            }
            return out;
        }
        // Produced with Node's OpenSSL-backed aes-256-gcm using this module's
        // documented header/nonce/AAD layout. The key is a public test vector.
        const key = fromHex('000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f');
        const aad = fromHex('636f6e746578742d6279746573');
        const envelope = fromHex('4e454f310101000102030405060708090a0b2967b921a697bb6bf92eb7e2df9d1d1feca6a75299032b094a0230ed39f87f14b520fd7a392dc47ab999');
        const expected = 'neo:crypto interop fixture';
        const out = decrypt(key, envelope, aad);
        if (out.length !== expected.length) throw Error('length');
        for (let i = 0; i < out.length; i++) {
            if (out[i] !== expected.charCodeAt(i)) throw Error('byte ' + i);
        }
        "#,
    )
    .unwrap();
}

#[test]
fn crypto_does_not_invoke_has_instance_or_byte_length_getters() {
    let js = runtime();
    js.run_module("crypto-hooks.js", r#"
        const U = Uint8Array;
        let calls = 0;
        Object.defineProperty(U, Symbol.hasInstance, {value(value) { calls++; return true; }});
        const {encrypt, decrypt} = await import('neo:crypto');
        const key = new U(32), data = new U([1, 2]);
        Object.defineProperty(data, 'byteLength', {get() { calls++; return 0; }});
        const result = decrypt(key, encrypt(key, data));
        if (calls || result.length !== 2 || result[0] !== 1) throw Error('observable validation hook');
    "#).unwrap();
}

#[test]
fn crypto_rejects_detached_and_forged_views() {
    let js = runtime();
    js.run_module("crypto-detached.js", r#"
        import {encrypt, decrypt} from 'neo:crypto';
        const key = new Uint8Array(32), empty = new Uint8Array(0);
        const envelope = encrypt(key, empty, empty);
        if (decrypt(key, envelope, empty).length !== 0) throw Error('empty');
        function detached(n) { const a = new Uint8Array(n); a.buffer.transfer(); return a; }
        for (const call of [
            () => encrypt(detached(32), empty),
            () => encrypt(key, detached(0)),
            () => encrypt(key, empty, detached(1)),
            () => decrypt(key, detached(34)),
            () => decrypt(key, envelope, detached(0)),
        ]) {
            try { call(); throw Error('accepted detached'); }
            catch (e) { if (e.code !== 'NEO_CRYPTO_DETACHED') throw e; }
        }
        for (const fake of [Object.create(Uint8Array.prototype), new Proxy(new Uint8Array(1), {}), new Uint8ClampedArray(1), new Int8Array(1)]) {
            try { encrypt(key, fake); throw Error('accepted fake'); }
            catch (e) { if (!(e instanceof TypeError)) throw e; }
        }
    "#).unwrap();
}

#[test]
fn crypto_validation_ignores_post_import_hooks_and_detaching_getters() {
    let js = runtime();
    js.run_module(
        "crypto-hooks-after.js",
        r#"
        import {encrypt, decrypt} from 'neo:crypto';
        const key = new Uint8Array(32), plain = new Uint8Array([42]);
        const aad = new Uint8Array([7]);
        let hits = 0;
        Object.defineProperty(Uint8Array, Symbol.hasInstance, {
            value(v) { hits++; return true; }
        });
        Object.defineProperty(key, 'byteLength', {get() { hits++; plain.buffer.transfer(); return 32; }});
        Object.defineProperty(plain, 'byteLength', {get() { hits++; aad.buffer.transfer(); return 1; }});
        Object.defineProperty(aad, 'byteLength', {get() { hits++; key.buffer.transfer(); return 1; }});
        const envelope = encrypt(key, plain, aad);
        Object.defineProperty(envelope, 'byteLength', {get() { hits++; key.buffer.transfer(); return 34; }});
        const result = decrypt(key, envelope, aad);
        if (hits || result.length !== 1 || result[0] !== 42) throw Error('hook invoked');
        "#,
    )
    .unwrap();
}
