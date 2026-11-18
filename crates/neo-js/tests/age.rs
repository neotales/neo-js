//! Integration tests for `neo:age` through the public `NeoJs` API.
//!
//! age file encryption is synchronous and needs no host capability, so every
//! test uses the default (no) permissions. The Rust unit tests cover the core,
//! the size bounds, and an independently generated age fixture; these tests
//! exercise the JavaScript surface and its error codes.

use neo_js::{NeoJs, Permissions};

fn runtime() -> NeoJs {
    NeoJs::new(Permissions::default()).unwrap()
}

#[test]
fn age_is_available_without_permissions_and_roundtrips_bytes() {
    let js = runtime();
    js.run_module(
        "age-available.js",
        r#"
        import { decrypt, encrypt, generateIdentity, identityToRecipient } from 'neo:age';
        if (typeof encrypt !== 'function' || typeof decrypt !== 'function' ||
            typeof generateIdentity !== 'function' || typeof identityToRecipient !== 'function') {
            throw Error('missing exports');
        }
        if ('__neoAgeGenerate' in globalThis || '__neoAgeEncrypt' in globalThis ||
            '__neoAgeDecrypt' in globalThis || '__neoAgeDerive' in globalThis) {
            throw Error('global callback exposed');
        }
        if (typeof fs !== 'undefined' || typeof env !== 'undefined' ||
            typeof __neoExec !== 'undefined') throw Error('capability leaked');

        const pair = generateIdentity();
        if (typeof pair.identity !== 'string' || typeof pair.recipient !== 'string') {
            throw Error('identity pair types');
        }
        if (!pair.identity.startsWith('AGE-SECRET-KEY-1')) throw Error('identity prefix');
        if (!pair.recipient.startsWith('age1')) throw Error('recipient prefix');
        if (identityToRecipient(pair.identity) !== pair.recipient) throw Error('derived recipient');

        const plaintext = Uint8Array.from([0, 1, 2, 0, 255, 128, 7]);
        const ciphertext = encrypt(pair.recipient, plaintext);
        if (!(ciphertext instanceof Uint8Array)) throw Error('ciphertext is not a Uint8Array');
        if (ciphertext.length <= plaintext.length) throw Error('ciphertext length');
        const prefix = 'age-encryption.org/v1\n';
        for (let i = 0; i < prefix.length; i++) {
            if (ciphertext[i] !== prefix.charCodeAt(i)) throw Error('age header');
        }

        const returned = decrypt(pair.identity, ciphertext);
        if (!(returned instanceof Uint8Array)) throw Error('plaintext is not a Uint8Array');
        if (returned.length !== plaintext.length) throw Error('plaintext length');
        for (let i = 0; i < plaintext.length; i++) {
            if (returned[i] !== plaintext[i]) throw Error('plaintext byte ' + i);
        }
        "#,
    )
    .unwrap();
}

#[test]
fn age_handles_empty_unicode_binary_and_large_payloads() {
    let js = runtime();
    js.run_module(
        "age-data.js",
        r#"
        import { decrypt, encrypt, generateIdentity } from 'neo:age';
        const { identity, recipient } = generateIdentity();

        // UTF-8 for 'Grüße, 世界, 🦀'. The runtime has no TextEncoder, so the
        // bytes are supplied explicitly.
        const unicode = Uint8Array.from([
            0x47, 0x72, 0xc3, 0xbc, 0xc3, 0x9f, 0x65, 0x2c, 0x20,
            0xe4, 0xb8, 0x96, 0xe7, 0x95, 0x8c, 0x2c, 0x20,
            0xf0, 0x9f, 0xa6, 0x80,
        ]);
        const unicodeOut = decrypt(identity, encrypt(recipient, unicode));
        if (unicodeOut.length !== unicode.length) throw Error('unicode length');
        for (let i = 0; i < unicode.length; i++) {
            if (unicodeOut[i] !== unicode[i]) throw Error('unicode byte ' + i);
        }

        const empty = encrypt(recipient, new Uint8Array());
        if (decrypt(identity, empty).length !== 0) throw Error('empty plaintext');

        const all = new Uint8Array(256);
        for (let i = 0; i < 256; i++) all[i] = i;
        const allOut = decrypt(identity, encrypt(recipient, all));
        if (allOut.length !== 256) throw Error('binary length');
        for (let i = 0; i < 256; i++) {
            if (allOut[i] !== i) throw Error('binary byte ' + i);
        }

        const large = new Uint8Array(1024 * 1024 + 3);
        for (let i = 0; i < large.length; i++) large[i] = (i * 31 + 7) & 255;
        const largeOut = decrypt(identity, encrypt(recipient, large));
        if (largeOut.length !== large.length) throw Error('large length');
        for (let i = 0; i < large.length; i++) {
            if (largeOut[i] !== large[i]) throw Error('large byte ' + i);
        }
        "#,
    )
    .unwrap();
}

#[test]
fn age_respects_typed_array_views_and_empty_views() {
    let js = runtime();
    js.run_module(
        "age-views.js",
        r#"
        import { decrypt, encrypt, generateIdentity } from 'neo:age';
        const { identity, recipient } = generateIdentity();

        const backing = Uint8Array.from([0xaa, 0xbb, 0x00, 0x01, 0xfe, 0xcc]);
        const plaintext = backing.subarray(2, 5);
        const ciphertext = encrypt(recipient, plaintext);
        const out = decrypt(identity, ciphertext);
        if (out.length !== 3) throw Error('view length');
        if (out[0] !== 0x00 || out[1] !== 0x01 || out[2] !== 0xfe) {
            throw Error('view bytes');
        }

        // A ciphertext view at a non-zero offset in a larger buffer.
        const padded = new Uint8Array(ciphertext.length + 2);
        padded[0] = 0xff;
        padded.set(ciphertext, 1);
        padded[padded.length - 1] = 0xff;
        const view = padded.subarray(1, padded.length - 1);
        const viewed = decrypt(identity, view);
        if (viewed.length !== 3 || viewed[0] !== 0x00) throw Error('ciphertext view');

        // A zero-length attached view is empty data, not detached.
        if (decrypt(identity, encrypt(recipient, new Uint8Array(0))).length !== 0) {
            throw Error('empty view');
        }
        "#,
    )
    .unwrap();
}

#[test]
fn age_rejects_wrong_identity_tampering_and_bad_arguments() {
    let js = runtime();
    js.run_module(
        "age-errors.js",
        r#"
        import { decrypt, encrypt, generateIdentity, identityToRecipient } from 'neo:age';
        const first = generateIdentity();
        const second = generateIdentity();
        const plaintext = Uint8Array.from([1, 2, 3]);
        const ciphertext = encrypt(first.recipient, plaintext);

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

        // A different valid identity is explicitly reported as no match.
        expectCode(() => decrypt(second.identity, ciphertext), 'NEO_AGE_IDENTITY_MISMATCH');

        // Every single-bit change is rejected, and no plaintext is returned.
        for (let i = 0; i < ciphertext.length; i++) {
            for (let bit = 0; bit < 8; bit++) {
                const copy = ciphertext.slice();
                copy[i] ^= 1 << bit;
                let failed = false;
                try {
                    decrypt(first.identity, copy);
                } catch (error) {
                    failed = true;
                    if (error.code !== 'NEO_AGE_AUTH' &&
                        error.code !== 'NEO_AGE_FORMAT' &&
                        error.code !== 'NEO_AGE_IDENTITY_MISMATCH') {
                        throw error;
                    }
                }
                if (!failed) throw Error('tamper at ' + i + '/' + bit + ' was accepted');
            }
        }

        // Payload tampering is an authentication failure, not a format error.
        const payload = ciphertext.slice();
        payload[payload.length - 1] ^= 1;
        expectCode(() => decrypt(first.identity, payload), 'NEO_AGE_AUTH');

        // Truncated and unknown files are malformed.
        expectCode(() => decrypt(first.identity, new Uint8Array()), 'NEO_AGE_FORMAT');
        expectCode(() => decrypt(first.identity, Uint8Array.from([1, 2, 3])), 'NEO_AGE_FORMAT');

        // Malformed keys are explicit and distinct from file errors.
        expectCode(() => identityToRecipient('not-a-key'), 'NEO_AGE_IDENTITY');
        expectCode(() => identityToRecipient(second.recipient), 'NEO_AGE_IDENTITY');
        expectCode(() => encrypt(first.identity, plaintext), 'NEO_AGE_RECIPIENT');
        expectCode(() => decrypt(first.recipient, ciphertext), 'NEO_AGE_IDENTITY');

        // Non-string keys and non-Uint8Array data are TypeErrors.
        expectTypeError(() => encrypt(123, plaintext));
        expectTypeError(() => encrypt(null, plaintext));
        expectTypeError(() => encrypt(first.recipient, [1, 2, 3]));
        expectTypeError(() => encrypt(first.recipient, 'plaintext'));
        expectTypeError(() => decrypt(first.identity, [1, 2, 3]));
        expectTypeError(() => decrypt(first.identity, 'ciphertext'));
        expectTypeError(() => identityToRecipient(null));
        "#,
    )
    .unwrap();
}

#[test]
fn age_errors_do_not_echo_inputs() {
    let js = runtime();
    js.run_module(
        "age-error-text.js",
        r#"
        import { decrypt, encrypt, generateIdentity } from 'neo:age';
        const first = generateIdentity();
        const second = generateIdentity();
        const secret = Uint8Array.from([0xde, 0xad, 0xbe, 0xef, 0x00, 0x99]);
        const ciphertext = encrypt(first.recipient, secret);
        let failure;
        try {
            decrypt(second.identity, ciphertext);
        } catch (error) {
            failure = error;
        }
        if (!failure) throw Error('expected failure');
        if (failure.code !== 'NEO_AGE_IDENTITY_MISMATCH') throw Error('wrong code');
        const text = String(failure.message);
        if (text.includes(second.identity) || text.includes(first.recipient) ||
            text.includes('dead') || text.includes('\\u0000') || text.includes('99')) {
            throw Error('error message echoed input');
        }
        "#,
    )
    .unwrap();
}

#[test]
fn age_globals_cannot_intercept_before_import() {
    let js = runtime();
    js.run_module(
        "age-before-import.js",
        r#"
        const RealUint8Array = Uint8Array;
        const RealError = Error;
        const RealTypeError = TypeError;
        let generateCalls = 0;
        let deriveCalls = 0;
        let encryptCalls = 0;
        let decryptCalls = 0;
        globalThis.__neoAgeGenerate = function () {
            generateCalls++;
            return { ok: true, identity: 'x', recipient: 'y' };
        };
        globalThis.__neoAgeDerive = function () {
            deriveCalls++;
            return { ok: true, value: new RealUint8Array(1) };
        };
        globalThis.__neoAgeEncrypt = function () {
            encryptCalls++;
            return { ok: true, value: new RealUint8Array(1) };
        };
        globalThis.__neoAgeDecrypt = function () {
            decryptCalls++;
            return { ok: true, value: new RealUint8Array(1) };
        };
        let fakeErrorCalled = false;
        let fakeTypeErrorCalled = false;
        globalThis.Uint8Array = function FakeUint8Array() {};
        globalThis.Error = function FakeError() { fakeErrorCalled = true; };
        globalThis.TypeError = function FakeTypeError() { fakeTypeErrorCalled = true; };

        const { decrypt, encrypt, generateIdentity, identityToRecipient } = await import('neo:age');
        const pair = generateIdentity();
        if (typeof pair.identity !== 'string' || typeof pair.recipient !== 'string') {
            throw new RealError('identity pair');
        }
        if (identityToRecipient(pair.identity) !== pair.recipient) {
            throw new RealError('derive');
        }
        const plaintext = RealUint8Array.from([0, 1, 2, 255]);
        const ciphertext = encrypt(pair.recipient, plaintext);
        const out = decrypt(pair.identity, ciphertext);
        if (out.length !== 4) throw new RealError('length');
        for (let i = 0; i < 4; i++) {
            if (out[i] !== plaintext[i]) throw new RealError('bytes');
        }
        if (generateCalls || deriveCalls || encryptCalls || decryptCalls) {
            throw new RealError('global interceptor was called');
        }
        try {
            decrypt(pair.identity, new RealUint8Array(3));
            throw new RealError('unexpected success');
        } catch (error) {
            if (fakeErrorCalled) throw new RealError('fake Error used');
            if (!(error instanceof RealError)) throw new RealError('wrong error type');
            if (error.code !== 'NEO_AGE_FORMAT') throw new RealError('wrong code');
        }
        try {
            encrypt(pair.recipient, [1, 2, 3]);
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
fn age_globals_cannot_intercept_after_import() {
    let js = runtime();
    js.run_module(
        "age-after-import.js",
        r#"
        import { decrypt, encrypt, generateIdentity, identityToRecipient } from 'neo:age';
        if ('__neoAgeGenerate' in globalThis || '__neoAgeDerive' in globalThis ||
            '__neoAgeEncrypt' in globalThis || '__neoAgeDecrypt' in globalThis) {
            throw Error('global callback exposed');
        }
        const RealUint8Array = Uint8Array;
        const RealError = Error;
        const RealTypeError = TypeError;
        const pair = generateIdentity();
        const plaintext = RealUint8Array.from([9, 8, 7, 0]);
        const ciphertext = encrypt(pair.recipient, plaintext);

        let generateCalls = 0;
        let deriveCalls = 0;
        let encryptCalls = 0;
        let decryptCalls = 0;
        globalThis.__neoAgeGenerate = function () {
            generateCalls++;
            return { ok: true, identity: 'x', recipient: 'y' };
        };
        globalThis.__neoAgeDerive = function () {
            deriveCalls++;
            return { ok: true, value: new RealUint8Array(1) };
        };
        globalThis.__neoAgeEncrypt = function () {
            encryptCalls++;
            return { ok: true, value: new RealUint8Array(1) };
        };
        globalThis.__neoAgeDecrypt = function () {
            decryptCalls++;
            return { ok: true, value: new RealUint8Array(1) };
        };
        let fakeErrorCalled = false;
        let fakeTypeErrorCalled = false;
        globalThis.Uint8Array = function FakeUint8Array() {};
        globalThis.Error = function FakeError() { fakeErrorCalled = true; };
        globalThis.TypeError = function FakeTypeError() { fakeTypeErrorCalled = true; };

        const out = decrypt(pair.identity, ciphertext);
        if (out.length !== 4) throw new RealError('length');
        for (let i = 0; i < 4; i++) {
            if (out[i] !== plaintext[i]) throw new RealError('bytes');
        }
        if (identityToRecipient(pair.identity) !== pair.recipient) {
            throw new RealError('derive');
        }
        const second = encrypt(pair.recipient, plaintext);
        if (decrypt(pair.identity, second).length !== 4) throw new RealError('second roundtrip');
        const third = generateIdentity();
        if (typeof third.identity !== 'string') throw new RealError('generate');
        if (generateCalls || deriveCalls || encryptCalls || decryptCalls) {
            throw new RealError('global interceptor was called');
        }
        try {
            decrypt(pair.identity, new RealUint8Array(3));
            throw new RealError('unexpected success');
        } catch (error) {
            if (fakeErrorCalled) throw new RealError('fake Error used');
            if (!(error instanceof RealError)) throw new RealError('wrong error type');
        }
        try {
            encrypt(pair.recipient, [1]);
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
fn age_host_rejects_non_typed_arrays_even_if_instanceof_lies() {
    let js = runtime();
    js.run_module(
        "age-hasinstance.js",
        r#"
        import { encrypt, generateIdentity } from 'neo:age';
        const RealUint8Array = Uint8Array;
        const RealError = Error;
        const pair = generateIdentity();
        Object.defineProperty(RealUint8Array, Symbol.hasInstance, {
            value: () => true,
        });
        let rejected = false;
        try {
            encrypt(pair.recipient, [1, 2, 3]);
        } catch (_) {
            rejected = true;
        }
        if (!rejected) throw new RealError('host accepted a non-typed-array');
        "#,
    )
    .unwrap();
}

#[test]
fn age_rejects_inputs_above_the_documented_limits() {
    let js = runtime();
    js.run_module(
        "age-limits.js",
        r#"
        import { decrypt, encrypt, generateIdentity } from 'neo:age';
        const MAX_PLAINTEXT = 8 * 1024 * 1024;
        const MAX_CIPHERTEXT = MAX_PLAINTEXT + 1024 * 1024;
        const { identity, recipient } = generateIdentity();
        function expectLimit(fn) {
            try {
                fn();
            } catch (error) {
                if (error.code !== 'NEO_AGE_LIMIT') throw error;
                return;
            }
            throw Error('expected NEO_AGE_LIMIT');
        }
        expectLimit(() => encrypt(recipient, new Uint8Array(MAX_PLAINTEXT + 1)));
        expectLimit(() => decrypt(identity, new Uint8Array(MAX_CIPHERTEXT + 1)));
        "#,
    )
    .unwrap();
}

#[test]
fn age_decrypts_an_independently_generated_age_file() {
    let js = runtime();
    js.run_module(
        "age-interop.js",
        r#"
        import { decrypt, encrypt, identityToRecipient } from 'neo:age';
        function fromHex(value) {
            const out = new Uint8Array(value.length / 2);
            for (let i = 0; i < out.length; i++) {
                out[i] = parseInt(value.substring(i * 2, i * 2 + 2), 16);
            }
            return out;
        }
        // age 1.3.2 command-line output for "neo:age interop fixture", using the
        // identity from the Rust fixture. The key is a public upstream test key.
        const identity = 'AGE-SECRET-KEY-1GQ9778VQXMMJVE8SK7J6VT8UJ4HDQAJUVSFCWCM02D8GEWQ72PVQ2Y5J33';
        const ciphertext = fromHex('6167652d656e6372797074696f6e2e6f72672f76310a2d3e20583235353139203865565a39572b2b7346766b5a2b4f6f52764c3930367346785737726d2f57762b4a79762f4147377067410a30486632774b554d717734646b38466f3479773747354c4b6d67666c336f315a444145426b4a68764c50380a2d2d2d2066343976564d30694d696942456a454972686b6b5237394a72504d656f47744e7730725a305267782b6a730a299f3a639e16f5d4605db5bc12063db20d7dc0b76fce5a856ac3dbfe20e767ae66c874752e9da729e2e610bfd65ef16edcd38ce8f45dfa');
        const expected = 'neo:age interop fixture';
        const out = decrypt(identity, ciphertext);
        if (out.length !== expected.length) throw Error('length');
        for (let i = 0; i < out.length; i++) {
            if (out[i] !== expected.charCodeAt(i)) throw Error('byte ' + i);
        }
        if (identityToRecipient(identity) !== 'age1t7rxyev2z3rw82stdlrrepyc39nvn86l5078zqkf5uasdy86jp6svpy7pa') {
            throw Error('recipient');
        }
        // Output starts as a binary age v1 file, not armor.
        const fresh = encrypt(identityToRecipient(identity), Uint8Array.from([1, 2, 3]));
        if (String.fromCharCode(fresh[0], fresh[1], fresh[2]) !== 'age') throw Error('not binary age');
        "#,
    )
    .unwrap();
}

#[test]
fn age_does_not_invoke_has_instance_or_byte_length_getters() {
    let js = runtime();
    js.run_module(
        "age-hooks.js",
        r#"
        const U = Uint8Array;
        let calls = 0;
        Object.defineProperty(U, Symbol.hasInstance, {value(value) { calls++; return true; }});
        const {encrypt, decrypt, generateIdentity} = await import('neo:age');
        const {identity, recipient} = generateIdentity();
        const data = new U(2).fill(1);
        Object.defineProperty(data, 'byteLength', {get() { calls++; return 0; }});
        const result = decrypt(identity, encrypt(recipient, data));
        if (calls || result.length !== 2 || result[0] !== 1) throw Error('observable validation hook');
        "#,
    )
    .unwrap();
}

#[test]
fn age_does_not_read_intercepted_result_properties() {
    let js = runtime();
    js.run_module(
        "age-prototype-hooks.js",
        r#"
        import {decrypt, encrypt, generateIdentity, identityToRecipient} from 'neo:age';
        const RealError = Error;
        const {identity, recipient} = generateIdentity();
        const plaintext = Uint8Array.from([42]);
        const ciphertext = encrypt(recipient, plaintext);
        let hits = 0;
        for (const name of ['ok', 'value', 'code', 'message', 'identity', 'recipient']) {
            // Use a null-prototype descriptor: QuickJS reads descriptor fields
            // through the prototype chain, and an inherited `value` accessor
            // would otherwise make the next defineProperty call fail.
            const descriptor = Object.create(null);
            descriptor.configurable = true;
            descriptor.get = function () { hits++; return undefined; };
            Object.defineProperty(Object.prototype, name, descriptor);
        }
        const out = decrypt(identity, ciphertext);
        if (out.length !== 1 || out[0] !== 42) throw new RealError('roundtrip');
        if (identityToRecipient(identity) !== recipient) throw new RealError('derive');
        const pair = generateIdentity();
        if (typeof pair.identity !== 'string') throw new RealError('generate');
        if (hits) throw new RealError('result property came from Object.prototype');
        "#,
    )
    .unwrap();
}

#[test]
fn age_rejects_detached_and_forged_views() {
    let js = runtime();
    js.run_module(
        "age-detached.js",
        r#"
        import {encrypt, decrypt, generateIdentity} from 'neo:age';
        const RealError = Error;
        const {identity, recipient} = generateIdentity();
        const empty = new Uint8Array(0);
        const ciphertext = encrypt(recipient, empty);
        if (decrypt(identity, ciphertext).length !== 0) throw new RealError('empty');
        function detached(n) { const a = new Uint8Array(n); a.buffer.transfer(); return a; }
        for (const call of [
            () => encrypt(recipient, detached(0)),
            () => encrypt(recipient, detached(1)),
            () => decrypt(identity, detached(0)),
            () => decrypt(identity, detached(ciphertext.length)),
        ]) {
            try {
                call();
                throw new RealError('accepted detached');
            } catch (e) {
                if (e.code !== 'NEO_AGE_DETACHED') throw e;
            }
        }
        for (const fake of [
            Object.create(Uint8Array.prototype),
            new Proxy(new Uint8Array(1), {}),
            new Uint8ClampedArray(1),
            new Int8Array(1),
        ]) {
            try { encrypt(recipient, fake); throw new RealError('accepted fake'); }
            catch (e) { if (!(e instanceof TypeError)) throw e; }
        }
        "#,
    )
    .unwrap();
}

#[test]
fn age_validation_ignores_post_import_hooks_and_detaching_getters() {
    let js = runtime();
    js.run_module(
        "age-hooks-after.js",
        r#"
        import {encrypt, decrypt, generateIdentity} from 'neo:age';
        const RealError = Error;
        const {identity, recipient} = generateIdentity();
        const plain = new Uint8Array([42]);
        let hits = 0;
        Object.defineProperty(Uint8Array, Symbol.hasInstance, {
            value(v) { hits++; return true; }
        });
        Object.defineProperty(plain, 'byteLength', {
            get() { hits++; plain.buffer.transfer(); return 1; }
        });
        const ciphertext = encrypt(recipient, plain);
        Object.defineProperty(ciphertext, 'byteLength', {
            get() { hits++; plain.buffer.transfer(); return 34; }
        });
        const result = decrypt(identity, ciphertext);
        if (hits || result.length !== 1 || result[0] !== 42) throw new RealError('hook invoked');
        "#,
    )
    .unwrap();
}

#[test]
fn age_and_crypto_can_be_imported_together() {
    let js = runtime();
    js.run_module(
        "age-and-crypto.js",
        r#"
        import { decrypt as ageDecrypt, encrypt as ageEncrypt, generateIdentity } from 'neo:age';
        import { decrypt as aesDecrypt, encrypt as aesEncrypt, generateKey } from 'neo:crypto';
        const pair = generateIdentity();
        const plaintext = Uint8Array.from([0, 1, 2, 0xff]);
        const ageOut = ageDecrypt(pair.identity, ageEncrypt(pair.recipient, plaintext));
        const key = generateKey();
        const aesOut = aesDecrypt(key, aesEncrypt(key, plaintext));
        for (let i = 0; i < plaintext.length; i++) {
            if (ageOut[i] !== plaintext[i]) throw Error('age byte ' + i);
            if (aesOut[i] !== plaintext[i]) throw Error('aes byte ' + i);
        }
        "#,
    )
    .unwrap();
}

#[test]
fn age_result_properties_cannot_be_intercepted_before_import() {
    let js = runtime();
    js.run_module(
        "age-prototype-hooks-before.js",
        r#"
        const RealError = Error;
        let hits = 0;
        for (const name of ['ok', 'value', 'code', 'message', 'identity', 'recipient']) {
            const descriptor = Object.create(null);
            descriptor.configurable = true;
            descriptor.get = function () { hits++; return undefined; };
            Object.defineProperty(Object.prototype, name, descriptor);
        }
        const { decrypt, encrypt, generateIdentity, identityToRecipient } = await import('neo:age');
        const pair = generateIdentity();
        if (typeof pair.identity !== 'string' || typeof pair.recipient !== 'string') {
            throw new RealError('identity pair');
        }
        if (identityToRecipient(pair.identity) !== pair.recipient) throw new RealError('derive');
        const plaintext = Uint8Array.from([42]);
        const out = decrypt(pair.identity, encrypt(pair.recipient, plaintext));
        if (out.length !== 1 || out[0] !== 42) throw new RealError('roundtrip');
        let failure;
        try {
            decrypt(pair.identity, new Uint8Array(3));
        } catch (error) {
            failure = error;
        }
        if (!failure || failure.code !== 'NEO_AGE_FORMAT') throw new RealError('format error');
        if (hits) throw new RealError('result property came from Object.prototype');
        "#,
    )
    .unwrap();
}

#[test]
fn age_error_codes_survive_inherited_descriptor_fields_before_import() {
    let js = runtime();
    js.run_module(
        "age-descriptor-before.js",
        r#"
        const RealError = Error;
        Object.defineProperty(Object.prototype, 'get', {
            __proto__: null, value: () => {}, configurable: true, writable: true,
        });
        Object.defineProperty(Object.prototype, 'set', {
            __proto__: null, value: () => {}, configurable: true, writable: true,
        });
        const { decrypt, encrypt, generateIdentity, identityToRecipient } = await import('neo:age');
        const pair = generateIdentity();
        if (typeof pair.identity !== 'string' || typeof pair.recipient !== 'string') {
            throw new RealError('identity pair');
        }
        if (identityToRecipient(pair.identity) !== pair.recipient) throw new RealError('derive');
        const plaintext = Uint8Array.from([7, 0, 255]);
        const out = decrypt(pair.identity, encrypt(pair.recipient, plaintext));
        if (out.length !== 3 || out[1] !== 0) throw new RealError('roundtrip');
        for (const check of [
            [() => decrypt(pair.identity, new Uint8Array(3)), 'NEO_AGE_FORMAT'],
            [() => identityToRecipient('not-a-key'), 'NEO_AGE_IDENTITY'],
            [() => encrypt(pair.identity, plaintext), 'NEO_AGE_RECIPIENT'],
        ]) {
            let failure;
            try {
                check[0]();
            } catch (error) {
                failure = error;
            }
            if (!(failure instanceof RealError)) {
                throw new RealError('wrong error type: ' + failure);
            }
            if (failure.code !== check[1]) {
                throw new RealError('expected ' + check[1] + ', got ' + failure.code);
            }
            if (String(failure.message).includes('property descriptor')) {
                throw new RealError('descriptor failure leaked: ' + failure.message);
            }
        }
        "#,
    )
    .unwrap();
}

#[test]
fn age_error_codes_survive_inherited_descriptor_fields_after_import() {
    let js = runtime();
    js.run_module(
        "age-descriptor-after.js",
        r#"
        import { decrypt, encrypt, generateIdentity } from 'neo:age';
        const RealError = Error;
        const pair = generateIdentity();
        const plaintext = Uint8Array.from([1, 2, 3]);
        const ciphertext = encrypt(pair.recipient, plaintext);
        Object.defineProperty(Object.prototype, 'get', {
            __proto__: null, value: () => {}, configurable: true, writable: true,
        });
        Object.defineProperty(Object.prototype, 'set', {
            __proto__: null, value: () => {}, configurable: true, writable: true,
        });
        if (decrypt(pair.identity, ciphertext).length !== 3) throw new RealError('roundtrip');
        let failure;
        try {
            decrypt(pair.identity, new Uint8Array(3));
        } catch (error) {
            failure = error;
        }
        if (!(failure instanceof RealError)) throw new RealError('wrong error type: ' + failure);
        if (failure.code !== 'NEO_AGE_FORMAT') {
            throw new RealError('expected NEO_AGE_FORMAT, got ' + failure.code);
        }
        "#,
    )
    .unwrap();
}

#[test]
fn age_does_not_call_a_recipient_objects_to_string() {
    let js = runtime();
    js.run_module(
        "age-tostring.js",
        r#"
        import {encrypt, generateIdentity} from 'neo:age';
        const RealError = Error;
        const RealTypeError = TypeError;
        const {recipient} = generateIdentity();
        let calls = 0;
        const trick = {
            toString() { calls++; return recipient; },
            valueOf() { calls++; return recipient; },
        };
        try {
            encrypt(trick, new Uint8Array(1));
            throw new RealError('unexpected success');
        } catch (error) {
            if (!(error instanceof RealTypeError)) throw error;
        }
        if (calls) throw new RealError('toString/valueOf was called');
        "#,
    )
    .unwrap();
}

#[test]
fn age_rejects_oversized_padded_keys_before_trimming() {
    let js = runtime();
    js.run_module(
        "age-padded-keys.js",
        r#"
        import { decrypt, encrypt, generateIdentity, identityToRecipient } from 'neo:age';
        const pair = generateIdentity();
        const plaintext = Uint8Array.from([1, 2, 3]);
        const ciphertext = encrypt(pair.recipient, plaintext);
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
        const pad = ' '.repeat(10000);
        expectCode(() => identityToRecipient(pad + pair.identity + pad), 'NEO_AGE_IDENTITY');
        expectCode(() => decrypt(pad + pair.identity, ciphertext), 'NEO_AGE_IDENTITY');
        expectCode(() => encrypt(pad + pair.recipient, plaintext), 'NEO_AGE_RECIPIENT');
        expectCode(() => identityToRecipient(pad + 'not-a-key' + pad), 'NEO_AGE_IDENTITY');

        // The original UTF-8 byte length is checked before trimming, so a
        // padded valid key exactly at the limit works and one byte over fails.
        const identityRoom = 512 - pair.identity.length;
        const paddedIdentity = ' '.repeat(identityRoom) + pair.identity;
        if (identityToRecipient(paddedIdentity) !== pair.recipient) {
            throw Error('padded identity at the boundary');
        }
        expectCode(() => identityToRecipient(' ' + paddedIdentity), 'NEO_AGE_IDENTITY');

        const recipientRoom = 512 - pair.recipient.length;
        const paddedRecipient = ' '.repeat(recipientRoom) + pair.recipient;
        const out = decrypt(pair.identity, encrypt(paddedRecipient, plaintext));
        if (out.length !== plaintext.length) throw Error('padded recipient at the boundary');
        expectCode(() => encrypt(' ' + paddedRecipient, plaintext), 'NEO_AGE_RECIPIENT');
        "#,
    )
    .unwrap();
}

#[test]
fn age_enforces_key_length_at_the_utf8_byte_boundary() {
    let js = runtime();
    js.run_module(
        "age-key-boundary.js",
        r#"
        import { identityToRecipient, generateIdentity } from 'neo:age';
        const pair = generateIdentity();
        function expectCode(fn, code) {
            try {
                fn();
            } catch (error) {
                if (error.code !== code) throw error;
                return;
            }
            throw Error('expected ' + code);
        }

        // U+00A0 is Unicode whitespace and two UTF-8 bytes. 219 of them plus
        // the 74-byte identity is exactly 512 bytes.
        const nbsp = '\u00a0'.repeat(219);
        if (identityToRecipient(nbsp + pair.identity) !== pair.recipient) {
            throw Error('unicode padding at the boundary');
        }
        expectCode(
            () => identityToRecipient('\u00a0' + nbsp + pair.identity),
            'NEO_AGE_IDENTITY',
        );

        // 110 four-byte emoji are 440 bytes. Their UTF-16 length is only 220,
        // so the exact UTF-8 check is what rejects the 514-byte result.
        const emoji = '\u{1F600}'.repeat(110);
        expectCode(() => identityToRecipient(emoji + pair.identity), 'NEO_AGE_IDENTITY');

        // A 512-byte string that is not a key passes the length bound and
        // fails parsing.
        const notAKey = '\u00e9'.repeat(256);
        expectCode(() => identityToRecipient(notAKey), 'NEO_AGE_IDENTITY');
        "#,
    )
    .unwrap();
}

#[test]
fn age_key_length_check_ignores_prototype_and_proxy_hooks() {
    let js = runtime();
    js.run_module(
        "age-key-hooks.js",
        r#"
        import { encrypt, identityToRecipient, generateIdentity } from 'neo:age';
        const pair = generateIdentity();
        let hits = 0;

        const descriptor = Object.create(null);
        descriptor.configurable = true;
        descriptor.get = function () { hits++; return 0; };
        try {
            Object.defineProperty(String.prototype, 'length', descriptor);
        } catch (_) {
            // String.prototype.length is normally non-configurable.
        }

        if (identityToRecipient(pair.identity) !== pair.recipient) {
            throw Error('valid key');
        }
        try {
            identityToRecipient(' '.repeat(10000) + pair.identity);
        } catch (error) {
            if (error.code !== 'NEO_AGE_IDENTITY') throw error;
        }

        const proxy = new Proxy(new String(pair.identity), {
            get(target, key) { hits++; return target[key]; },
        });
        try {
            identityToRecipient(proxy);
            throw Error('proxy was accepted');
        } catch (error) {
            if (!(error instanceof TypeError)) throw error;
        }

        const object = new String(pair.identity);
        object.toString = function () { hits++; return pair.identity; };
        object.valueOf = function () { hits++; return pair.identity; };
        object[Symbol.toPrimitive] = function () { hits++; return pair.identity; };
        try {
            identityToRecipient(object);
            throw Error('String object was accepted');
        } catch (error) {
            if (!(error instanceof TypeError)) throw error;
        }
        try {
            encrypt(object, new Uint8Array(1));
            throw Error('String recipient object was accepted');
        } catch (error) {
            if (!(error instanceof TypeError)) throw error;
        }

        if (hits) throw Error('key length hook was invoked');
        "#,
    )
    .unwrap();
}
