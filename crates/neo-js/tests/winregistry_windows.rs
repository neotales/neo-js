//! Windows-only tests for `neo:winregistry`.
//!
//! Every test creates a uniquely named key under
//! `HKEY_CURRENT_USER\Software\neo-js-tests-*` and removes it with a drop
//! guard. No existing key is opened for writing, no machine-wide hive is
//! touched, and no value outside that temporary subtree is modified.

#![cfg(windows)]

use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use neo_js::{NeoJs, Permissions};
use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS};
use windows_sys::Win32::Storage::FileSystem::DELETE;
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, RegCloseKey, RegDeleteKeyExW, RegDeleteTreeW,
    RegOpenKeyExW, RegSetValueExW,
};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

struct TempKey {
    path: String,
}

impl TempKey {
    fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        Self {
            path: format!(
                "Software\\neo-js-tests-{}-{}-{}",
                std::process::id(),
                nanos,
                id
            ),
        }
    }

    fn js_path(&self) -> String {
        self.path.replace('\\', "\\\\")
    }

    fn wide_path(&self) -> Vec<u16> {
        self.path.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

impl Drop for TempKey {
    fn drop(&mut self) {
        let wide = self.wide_path();
        let mut key: HKEY = ptr::null_mut();
        // SAFETY: `wide` is a NUL-terminated path under the unique
        // HKCU\Software\neo-js-tests-* subtree. The handle is closed once
        // after the tree is emptied, and only this exact path is removed.
        let result = unsafe {
            let open = RegOpenKeyExW(
                HKEY_CURRENT_USER,
                wide.as_ptr(),
                0,
                KEY_READ | KEY_SET_VALUE | DELETE,
                &mut key,
            );
            if open == ERROR_FILE_NOT_FOUND || open == ERROR_PATH_NOT_FOUND {
                Ok(())
            } else if open != ERROR_SUCCESS {
                Err(("RegOpenKeyExW", open))
            } else {
                let tree = RegDeleteTreeW(key, ptr::null());
                let close = RegCloseKey(key);
                if tree != ERROR_SUCCESS {
                    Err(("RegDeleteTreeW", tree))
                } else if close != ERROR_SUCCESS {
                    Err(("RegCloseKey", close))
                } else {
                    let delete = RegDeleteKeyExW(HKEY_CURRENT_USER, wide.as_ptr(), 0, 0);
                    if delete == ERROR_SUCCESS || delete == ERROR_FILE_NOT_FOUND {
                        Ok(())
                    } else {
                        Err(("RegDeleteKeyExW", delete))
                    }
                }
            }
        };
        if let Err((operation, status)) = result {
            if std::thread::panicking() {
                eprintln!("temporary registry cleanup failed: {operation}: {status}");
            } else {
                panic!("temporary registry cleanup failed: {operation}: {status}");
            }
        }
    }
}

/// Set a raw value with an arbitrary numeric type for tests that need a type
/// this module has no writer for.
fn set_raw_value(path: &str, name: &str, kind: u32, data: &[u8]) {
    let path: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let name: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let mut key: HKEY = ptr::null_mut();
    // SAFETY: both wide strings are NUL-terminated and live for the calls; the
    // handle is checked before use and closed unconditionally afterwards.
    unsafe {
        let status = RegOpenKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, KEY_SET_VALUE, &mut key);
        assert_eq!(status, 0, "open temporary key");
        let status = RegSetValueExW(
            key,
            name.as_ptr(),
            0,
            kind,
            if data.is_empty() {
                ptr::null()
            } else {
                data.as_ptr()
            },
            data.len() as u32,
        );
        RegCloseKey(key);
        assert_eq!(status, 0, "set raw temporary value");
    }
}

fn runtime() -> NeoJs {
    NeoJs::new(Permissions {
        registry: true,
        ..Permissions::default()
    })
    .unwrap()
}

fn source(template: &str, key: &TempKey) -> String {
    template.replace("__PATH__", &key.js_path())
}

#[test]
fn cleanup_guard_removes_values_and_children() {
    let key = TempKey::new();
    let path = key.js_path();
    let js = runtime();
    js.run_module(
        "cleanup-create.js",
        &source(
            r#"import {createKey, writeString} from 'neo:winregistry';
            createKey('HKCU', '__PATH__');
            createKey('HKCU', '__PATH__\\child');
            writeString('HKCU', '__PATH__', 'value', 'data');
            writeString('HKCU', '__PATH__\\child', 'value', 'data');"#,
            &key,
        ),
    )
    .unwrap();
    drop(key);
    js.run_module(
        "cleanup-verify.js",
        &format!(
            "import {{keyExists}} from 'neo:winregistry'; if (keyExists('HKCU', '{path}')) throw Error('cleanup left the key');"
        ),
    )
    .unwrap();
}

#[test]
fn overridden_intrinsics_do_not_corrupt_registry_values() {
    let key = TempKey::new();
    runtime().run_module("registry-overrides.js", &source(r#"
        import {createKey, writeUint64, readUint64, writeBinary, readBinary, listValues, listSubKeys} from 'neo:winregistry';
        const path = '__PATH__';
        createKey('HKCU', path);
        BigInt.prototype.toString = () => '7';
        RegExp.prototype.exec = () => [''];
        writeUint64('HKCU', path, 'qword', 1n);
        if (readUint64('HKCU', path, 'qword') !== 1n) throw Error('qword corrupted');
        let code;
        try { createKey('HKCU', '\ud800'); } catch (e) { code = e.code; }
        if (code !== 'WINREGISTRY_ENCODING') throw Error('surrogate accepted');
        const bytes = new Uint8Array([1, 2, 3]);
        Object.defineProperty(bytes, 'length', { get() { throw Error('length read'); } });
        writeBinary('HKCU', path, 'bytes', bytes);
        const result = readBinary('HKCU', path, 'bytes');
        if (result.length !== 3 || result[0] !== 1 || result[2] !== 3) throw Error('bytes corrupted');
        createKey('HKCU', path + '\\child');
        let hits = 0;
        Object.defineProperty(Array.prototype, '0', {set() { hits++; }, configurable: true});
        const values = listValues('HKCU', path);
        const keys = listSubKeys('HKCU', path);
        delete Array.prototype[0];
        if (hits || values.length !== 2 || !Object.hasOwn(values, 0) || keys[0] !== 'child') {
            throw Error('enumeration intercepted');
        }
    "#, &key)).unwrap();
}

#[test]
fn writes_reads_enumerates_and_deletes_a_temporary_key() {
    let key = TempKey::new();
    let js = runtime();
    js.run_module(
        "registry-roundtrip.js",
        &source(
            r#"import {createKey, keyExists, deleteKey, deleteValue, listSubKeys,
                     listValues, readValue, readString, readUint32, readUint64,
                     readBinary, writeString, writeExpandString, writeUint32,
                     writeUint64, writeBinary, ValueType}
                     from 'neo:winregistry';

             const hive = 'HKCU';
             const path = '__PATH__';
             function expectCode(fn, code) {
                 let seen = null;
                 try { fn(); } catch (error) { seen = error.code; }
                 if (seen !== code) throw Error('expected ' + code + ', got ' + seen);
             }

             if (keyExists(hive, path)) throw Error('temporary key already exists');
             if (createKey(hive, path) !== true) throw Error('create did not report a new key');
             if (createKey(hive, path) !== false) throw Error('create did not report an existing key');
             if (!keyExists(hive, path)) throw Error('created key is missing');

             const text = 'Grüße 世界 🦀';
             writeString(hive, path, 'Text', text);
             if (readString(hive, path, 'Text') !== text) throw Error('string roundtrip');
             writeExpandString(hive, path, 'Expand', '%USERPROFILE%\\AppData');
             if (readString(hive, path, 'Expand') !== '%USERPROFILE%\\AppData') {
                 throw Error('expand string roundtrip');
             }

             writeUint32(hive, path, 'Count', 4294967295);
             if (readUint32(hive, path, 'Count') !== 4294967295) throw Error('dword');
             if (typeof readUint32(hive, path, 'Count') !== 'number') throw Error('dword type');

             writeUint64(hive, path, 'Big', 18446744073709551615n);
             const big = readUint64(hive, path, 'Big');
             if (typeof big !== 'bigint' || big !== 18446744073709551615n) throw Error('qword');
             writeUint64(hive, path, 'SmallBig', 9007199254740993n);
             if (readUint64(hive, path, 'SmallBig') !== 9007199254740993n) {
                 throw Error('qword above safe integer');
             }

             const bytes = new Uint8Array(257);
             for (let i = 0; i < 256; i++) bytes[i] = i;
             bytes[256] = 0xff;
             writeBinary(hive, path, 'Bytes', bytes);
             const returned = readBinary(hive, path, 'Bytes');
             if (!(returned instanceof Uint8Array) || returned.length !== bytes.length) {
                 throw Error('binary length');
             }
             for (let i = 0; i < bytes.length; i++) {
                 if (returned[i] !== bytes[i]) throw Error('binary byte ' + i);
             }

             const rawText = readValue(hive, path, 'Text');
             if (rawText.type !== ValueType.SZ) throw Error('raw string type');
             if (!(rawText.data instanceof Uint8Array) || rawText.data.length < 2) {
                 throw Error('raw string data');
             }
             const end = rawText.data.length;
             if (rawText.data[end - 1] !== 0 || rawText.data[end - 2] !== 0) {
                 throw Error('raw string terminator');
             }
             const rawDword = readValue(hive, path, 'Count');
             if (rawDword.type !== ValueType.DWORD || rawDword.data.length !== 4) {
                 throw Error('raw dword');
             }
             const rawQword = readValue(hive, path, 'Big');
             if (rawQword.type !== ValueType.QWORD || rawQword.data.length !== 8) {
                 throw Error('raw qword');
             }
             const rawBinary = readValue(hive, path, 'Bytes');
             if (rawBinary.type !== ValueType.BINARY || rawBinary.data.length !== bytes.length) {
                 throw Error('raw binary');
             }

             const values = listValues(hive, path);
             const types = new Map();
             for (const entry of values) types.set(entry.name, entry.type);
             if (types.get('Text') !== ValueType.SZ) throw Error('value type Text');
             if (types.get('Expand') !== ValueType.EXPAND_SZ) throw Error('value type Expand');
             if (types.get('Count') !== ValueType.DWORD) throw Error('value type Count');
             if (types.get('Big') !== ValueType.QWORD) throw Error('value type Big');
             if (types.get('Bytes') !== ValueType.BINARY) throw Error('value type Bytes');

             createKey(hive, path + '\\Child');
             const children = listSubKeys(hive, path);
             if (!children.includes('Child')) throw Error('missing child key');
             if (deleteKey(hive, path + '\\Child') !== true) throw Error('delete child');
             if (deleteKey(hive, path + '\\Child') !== false) throw Error('delete child twice');

             createKey(hive, path + '\\A');
             createKey(hive, path + '\\B');
             createKey(hive, path + '\\C');
             expectCode(() => listSubKeys(hive, path, 2), 'WINREGISTRY_LIMIT');
             if (listSubKeys(hive, path, 3).length !== 3) throw Error('subkey limit');
             expectCode(() => listValues(hive, path, 4), 'WINREGISTRY_LIMIT');
             if (listValues(hive, path, 6).length !== 6) throw Error('value limit');

             writeString(hive, path, '', 'default value');
             if (readString(hive, path, '') !== 'default value') throw Error('default value');
             if (deleteValue(hive, path, '') !== true) throw Error('delete default value');
             if (deleteValue(hive, path, '') !== false) throw Error('delete default value twice');

             expectCode(() => readUint32(hive, path, 'Text'), 'WINREGISTRY_TYPE_MISMATCH');
             expectCode(() => readUint64(hive, path, 'Count'), 'WINREGISTRY_TYPE_MISMATCH');
             expectCode(() => readString(hive, path, 'Count'), 'WINREGISTRY_TYPE_MISMATCH');
             expectCode(() => readBinary(hive, path, 'Big'), 'WINREGISTRY_TYPE_MISMATCH');
             expectCode(() => readString(hive, path, 'Missing'), 'WINREGISTRY_NOT_FOUND');
             expectCode(
                 () => writeString(hive, path + '\\Missing', 'Value', 'text'),
                 'WINREGISTRY_NOT_FOUND'
             );

             if (deleteValue(hive, path, 'Text') !== true) throw Error('delete value');
             if (deleteValue(hive, path, 'Text') !== false) throw Error('delete value twice');
             if (deleteKey(hive, path + '\\A') !== true) throw Error('delete A');
             if (deleteKey(hive, path + '\\B') !== true) throw Error('delete B');
             if (deleteKey(hive, path + '\\C') !== true) throw Error('delete C');
             if (deleteKey(hive, path) !== true) throw Error('delete key');
             if (deleteKey(hive, path) !== false) throw Error('delete key twice');
             if (keyExists(hive, path)) throw Error('key still exists');"#,
            &key,
        ),
    )
    .unwrap();
}

#[test]
fn missing_keys_and_values_are_reported_without_creating_anything() {
    let key = TempKey::new();
    let js = runtime();
    js.run_module(
        "registry-missing.js",
        &source(
            r#"import {keyExists, deleteKey, deleteValue, readString, writeString}
                     from 'neo:winregistry';

             const hive = 'HKCU';
             const path = '__PATH__';
             function expectCode(fn, code) {
                 let seen = null;
                 try { fn(); } catch (error) { seen = error.code; }
                 if (seen !== code) throw Error('expected ' + code + ', got ' + seen);
             }

             if (keyExists(hive, path)) throw Error('missing key exists');
             if (deleteKey(hive, path) !== false) throw Error('missing delete key');
             expectCode(() => readString(hive, path, 'Value'), 'WINREGISTRY_NOT_FOUND');
             expectCode(() => deleteValue(hive, path, 'Value'), 'WINREGISTRY_NOT_FOUND');
             expectCode(
                 () => writeString(hive, path, 'Value', 'text'),
                 'WINREGISTRY_NOT_FOUND'
             );
             if (keyExists(hive, path)) throw Error('failed operations created the key');"#,
            &key,
        ),
    )
    .unwrap();
}

#[test]
fn bounds_and_invalid_values_do_not_reach_the_registry() {
    let key = TempKey::new();
    let js = runtime();
    js.run_module(
        "registry-bounds.js",
        &source(
            r#"import {createKey, deleteKey, writeBinary, writeString, writeUint64}
                     from 'neo:winregistry';

             const hive = 'HKCU';
             const path = '__PATH__';
             function expectCode(fn, code) {
                 let seen = null;
                 try { fn(); } catch (error) { seen = error.code; }
                 if (seen !== code) throw Error('expected ' + code + ', got ' + seen);
             }

             if (createKey(hive, path) !== true) throw Error('create');
             expectCode(() => writeBinary(hive, path, 'Big', new Uint8Array(8388609)),
                        'WINREGISTRY_LIMIT');
             expectCode(() => writeString(hive, path, 'Value', 'A' + String.fromCharCode(0) + 'B'),
                        'WINREGISTRY_INVALID_VALUE');
             expectCode(() => writeUint64(hive, path, 'Value', -1n),
                        'WINREGISTRY_INVALID_VALUE');
             expectCode(() => writeUint64(hive, path, 'Value', 18446744073709551616n),
                        'WINREGISTRY_INVALID_VALUE');

             const longComponent = 'a'.repeat(256);
             const seen = {};
             try { createKey(hive, longComponent); } catch (error) { seen.code = error.code; }
             if (seen.code !== 'WINREGISTRY_INVALID_PATH') throw Error('long component');

             if (deleteKey(hive, path) !== true) throw Error('cleanup');"#,
            &key,
        ),
    )
    .unwrap();
}

#[test]
fn arbitrary_types_long_names_and_large_values_roundtrip() {
    let key = TempKey::new();
    let js = runtime();
    js.run_module(
        "registry-setup.js",
        &source(
            r#"import {createKey, writeString} from 'neo:winregistry';
             const path = '__PATH__';
             if (createKey('HKCU', path) !== true) throw Error('create');
             writeString('HKCU', path, 'a'.repeat(16000), 'long value name');
             "#,
            &key,
        ),
    )
    .unwrap();

    // Types and data this module has no writer for. The module must return the
    // numeric type and exact bytes rather than rejecting the type.
    set_raw_value(&key.path, "Custom", 0xFFFF, &[0xde, 0xad, 0xbe, 0xef, 0x00]);
    set_raw_value(&key.path, "None", 0, &[]);
    let large = vec![0x5a; 64 * 1024];
    set_raw_value(&key.path, "Large", 3, &large);

    js.run_module(
        "registry-types.js",
        &source(
            r#"import {readValue, readBinary, listValues, readString, deleteValue,
                     deleteKey} from 'neo:winregistry';
             const path = '__PATH__';
             const longName = 'a'.repeat(16000);
             function expectCode(fn, code) {
                 let seen = null;
                 try { fn(); } catch (error) { seen = error.code; }
                 if (seen !== code) throw Error('expected ' + code + ', got ' + seen);
             }

             const custom = readValue('HKCU', path, 'Custom');
             if (custom.type !== 0xFFFF) throw Error('custom type');
             if (custom.data.length !== 5) throw Error('custom length');
             const expected = [0xde, 0xad, 0xbe, 0xef, 0x00];
             for (let i = 0; i < expected.length; i++) {
                 if (custom.data[i] !== expected[i]) throw Error('custom byte ' + i);
             }

             const none = readValue('HKCU', path, 'None');
             if (none.type !== 0 || none.data.length !== 0) throw Error('reg none');

             const big = readBinary('HKCU', path, 'Large');
             if (big.length !== 65536) throw Error('large length');
             if (big[0] !== 0x5a || big[65535] !== 0x5a) throw Error('large bytes');

             if (readString('HKCU', path, longName) !== 'long value name') {
                 throw Error('long value name');
             }
             expectCode(() => readString('HKCU', path, 'Custom'), 'WINREGISTRY_TYPE_MISMATCH');

             const values = listValues('HKCU', path);
             const byName = new Map();
             for (const entry of values) byName.set(entry.name, entry.type);
             if (byName.get('Custom') !== 0xFFFF) throw Error('listed custom type');
             if (byName.get('None') !== 0) throw Error('listed none type');
             if (byName.get(longName) !== 1) throw Error('listed long name');

             if (deleteValue('HKCU', path, longName) !== true) throw Error('delete long name');
             expectCode(() => listValues('HKCU', path, 2), 'WINREGISTRY_LIMIT');
             if (listValues('HKCU', path, 3).length !== 3) throw Error('value limit');
             if (deleteKey('HKCU', path) !== true) throw Error('delete key');"#,
            &key,
        ),
    )
    .unwrap();
}
