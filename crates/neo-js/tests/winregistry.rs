//! Integration tests for `neo:win-registry` that do not touch a real registry.
//!
//! Permission gating, argument validation, and the non-Windows unsupported
//! behavior are shared by every host. Windows-only success paths live in
//! `winregistry_windows.rs` and use temporary HKCU keys only.

use neo_js::{NeoJs, Permissions};

fn runtime(registry: bool) -> NeoJs {
    NeoJs::new(Permissions {
        registry,
        ..Permissions::default()
    })
    .unwrap()
}

#[test]
fn registry_is_absent_without_permission() {
    let js = runtime(false);
    assert!(
        js.run_module("denied.js", "import 'neo:winregistry';")
            .is_err()
    );
    assert!(
        js.run_module(
            "denied-named.js",
            "import {isAvailable} from 'neo:winregistry'; isAvailable();",
        )
        .is_err()
    );
}

#[test]
fn prototype_setters_before_dynamic_import_cannot_capture_host_or_limits() {
    let js = runtime(true);
    js.run_module(
        "registry-private-objects.js",
        r#"
        let captured = 0;
        const names = [
            'isAvailable', 'createKey', 'keyExists', 'deleteKey', 'deleteValue',
            'listSubKeys', 'listValues', 'readValue', 'readString', 'readUint32',
            'readUint64', 'readBinary', 'writeString', 'writeExpandString',
            'writeUint32', 'writeUint64', 'writeBinary',
            'acquireOwned', 'created', 'getString', 'setString', 'close', 'open',
            'hive', 'path', 'valueName', 'valueBytes', 'valueUnits', 'enum'
        ];
        for (const name of names) {
            Object.defineProperty(Object.prototype, name, {
                configurable: true,
                set(value) { captured++; },
                get() { return Infinity; }
            });
        }
        const registry = await import('neo:winregistry');
        if (captured !== 0) throw Error('captured private properties: ' + captured);
        function expectCode(fn, code) {
            let seen;
            try { fn(); } catch (error) { seen = error.code; }
            if (seen !== code) throw Error('expected ' + code + ', got ' + seen);
        }
        expectCode(() => registry.createKey('x'.repeat(33), ''), 'WINREGISTRY_INVALID_HIVE');
        expectCode(() => registry.createKey('HKCU', 'x'.repeat(16385)), 'WINREGISTRY_INVALID_PATH');
        expectCode(() => registry.deleteValue('HKCU', '', 'x'.repeat(16384)), 'WINREGISTRY_INVALID_NAME');
        expectCode(() => registry.writeBinary('HKCU', '', 'x', new Uint8Array(8388609)), 'WINREGISTRY_LIMIT');
        expectCode(() => registry.writeString('HKCU', '', 'x', 'x'.repeat(4194304)), 'WINREGISTRY_LIMIT');
        let typeError = false;
        try { registry.listValues('HKCU', '', 4097); } catch (error) { typeError = error instanceof TypeError; }
        if (!typeError || captured !== 0) throw Error('limit bypass or private capture');
        "#,
    ).unwrap();
}

#[test]
fn inherited_descriptor_accessors_cannot_replace_registry_errors() {
    for property in ["get", "set"] {
        let js = runtime(true);
        let source = r#"
            let reads = 0;
            let validation, native;
            Object.defineProperty(Object.prototype, '__PROPERTY__', {
                __proto__: null,
                configurable: true,
                get() { reads++; throw Error('inherited descriptor accessor'); }
            });
            try {
                const registry = await import('neo:winregistry');
                try { registry.createKey('HKCU', '\ud800'); } catch (error) { validation = error; }
                try { registry.createKey('BAD', 'valid'); } catch (error) { native = error; }
            } finally {
                delete Object.prototype.__PROPERTY__;
            }
            if (reads !== 0) throw Error('descriptor accessed inherited property');
            if (!validation || validation.code !== 'WINREGISTRY_ENCODING')
                throw Error('validation error lost');
            if (!native || native.code !== 'WINREGISTRY_INVALID_HIVE')
                throw Error('native error lost');
            for (const error of [validation, native]) {
                const d = Object.getOwnPropertyDescriptor(error, 'code');
                if (!d || !d.writable || !d.enumerable || !d.configurable)
                    throw Error('error code descriptor changed');
            }
        "#
        .replace("__PROPERTY__", property);
        js.run_module("registry-error-descriptors.js", &source)
            .unwrap();
    }
}

#[test]
fn permission_exposes_only_the_documented_surface() {
    let js = runtime(true);
    js.run_module(
        "surface.js",
        "import {isAvailable, createKey, keyExists, deleteKey, deleteValue,
                 listSubKeys, listValues, readValue, readString, readUint32,
                 readUint64, readBinary, writeString, writeExpandString,
                 writeUint32, writeUint64, writeBinary, ValueType, HIVES}
                 from 'neo:winregistry';
         const names = [isAvailable, createKey, keyExists, deleteKey, deleteValue,
                        listSubKeys, listValues, readValue, readString, readUint32,
                        readUint64, readBinary, writeString, writeExpandString,
                        writeUint32, writeUint64, writeBinary];
         for (const name of names) if (typeof name !== 'function') throw Error('missing export');
         if (typeof isAvailable() !== 'boolean') throw Error('isAvailable');
         if (Object.isFrozen(ValueType) !== true) throw Error('ValueType not frozen');
         if (Object.isFrozen(HIVES) !== true) throw Error('HIVES not frozen');
         if (ValueType.SZ !== 1 || ValueType.EXPAND_SZ !== 2 ||
             ValueType.BINARY !== 3 || ValueType.DWORD !== 4 ||
             ValueType.MULTI_SZ !== 7 || ValueType.QWORD !== 11) throw Error('types');
         if (HIVES.join(',') !== 'HKCR,HKCU,HKLM,HKU,HKCC') throw Error('hives');
         if ('__neoWinRegistry' in globalThis) throw Error('global callback exposed');",
    )
    .unwrap();
}

#[cfg(not(windows))]
#[test]
fn every_operation_reports_unsupported_without_faking_success() {
    let source = r#"import {isAvailable, createKey, keyExists, deleteKey, deleteValue,
                 listSubKeys, listValues, readValue, readString, readUint32,
                 readUint64, readBinary, writeString, writeExpandString,
                 writeUint32, writeUint64, writeBinary}
                 from '__REGISTRY__';
         if (isAvailable() !== false) throw Error('isAvailable lied');

         const calls = [
             () => createKey('HKCU', 'Software\\neo-js-test'),
             () => keyExists('HKCU', ''),
             () => deleteKey('HKCU', 'Software\\neo-js-test'),
             () => deleteValue('HKCU', '', 'Value'),
             () => listSubKeys('HKCU', ''),
             () => listValues('HKCU', ''),
             () => readValue('HKCU', '', 'Value'),
             () => readString('HKCU', '', 'Value'),
             () => readUint32('HKCU', '', 'Value'),
             () => readUint64('HKCU', '', 'Value'),
             () => readBinary('HKCU', '', 'Value'),
             () => writeString('HKCU', '', 'Value', 'text'),
             () => writeString('HKCU', '', 'Value', 'Grüße 世界 🦀'),
             () => writeExpandString('HKCU', '', 'Value', '%TEMP%'),
             () => writeUint32('HKCU', '', 'Value', 1),
             () => writeUint64('HKCU', '', 'Value', 1n),
             () => writeBinary('HKCU', '', 'Value', Uint8Array.from([1, 2])),
         ];
         for (const call of calls) {
             let code = null;
             try {
                 call();
             } catch (error) {
                 code = error.code;
             }
             if (code !== 'WINREGISTRY_UNSUPPORTED') {
                 throw Error('expected unsupported, got ' + code);
             }
         }"#;
    for name in ["neo:win-registry", "neo:winregistry"] {
        runtime(true)
            .run_module("unsupported.js", &source.replace("__REGISTRY__", name))
            .unwrap();
    }
}

#[cfg(not(windows))]
#[test]
fn owned_registry_keys_do_not_fake_success_on_non_windows() {
    for specifier in ["neo:win-registry", "neo:winregistry"] {
        let js = runtime(true);
        js.run_module(
            "owned-unsupported.js",
            &format!(
                r#"import {{Registry, Rights, isAvailable}} from '{specifier}';
                   if (isAvailable()) throw Error('unexpected Windows support');
                   for (const operation of [Registry.openKey, Registry.createKey]) {{
                       let code;
                       try {{ operation('HKCU\\Software\\Example', Rights.QUERY_VALUE); }}
                       catch (error) {{ code = error.code; }}
                       if (code !== 'WINREGISTRY_UNSUPPORTED') throw Error('expected unsupported');
                   }}"#
            ),
        )
        .unwrap();
    }
}

#[test]
fn invalid_arguments_are_rejected_before_platform_work() {
    let js = runtime(true);
    js.run_module(
        "invalid-args.js",
        r#"import {createKey, keyExists, deleteKey, deleteValue, listSubKeys,
                 readString, writeString, writeUint64}
                 from 'neo:winregistry';

         function expectCode(fn, code) {
             let seen = null;
             try { fn(); } catch (error) { seen = error.code; }
             if (seen !== code) throw Error('expected ' + code + ', got ' + seen);
         }

         const NUL = String.fromCharCode(0);
         const SEP = '\\';
         expectCode(() => createKey('NOPE', 'Software'), 'WINREGISTRY_INVALID_HIVE');
         expectCode(() => createKey('HKPD', 'Software'), 'WINREGISTRY_INVALID_HIVE');
         expectCode(() => createKey('HKCU', SEP + 'Software'), 'WINREGISTRY_INVALID_PATH');
         expectCode(() => createKey('HKCU', 'Software' + SEP), 'WINREGISTRY_INVALID_PATH');
         expectCode(
             () => createKey('HKCU', 'Software' + SEP + SEP + 'App'),
             'WINREGISTRY_INVALID_PATH'
         );
         expectCode(
             () => createKey('HKCU', 'Software' + SEP + '..' + SEP + 'App'),
             'WINREGISTRY_INVALID_PATH'
         );
         expectCode(
             () => createKey('HKCU', 'Software' + NUL + 'App'),
             'WINREGISTRY_INVALID_PATH'
         );
         expectCode(
             () => keyExists('HKCU', 'A' + SEP + SEP + 'B'),
             'WINREGISTRY_INVALID_PATH'
         );
         expectCode(() => deleteKey('HKCU', ''), 'WINREGISTRY_INVALID_PATH');
         expectCode(
             () => deleteValue('HKCU', '', 'A' + NUL + 'B'),
             'WINREGISTRY_INVALID_NAME'
         );
         expectCode(
             () => readString('HKCU', '', 'A' + NUL + 'B'),
             'WINREGISTRY_INVALID_NAME'
         );
         expectCode(
             () => writeString('HKCU', '', 'Value', 'A' + NUL + 'B'),
             'WINREGISTRY_INVALID_VALUE'
         );
         const secret = 'neo-secret-input-123';
         try {
             createKey('HKCU', secret + SEP + '..');
         } catch (error) {
             if (error.message.includes(secret)) throw Error('path echoed in error');
         }
         try {
             writeString('HKCU', '', 'Value', 'A' + NUL + secret);
         } catch (error) {
             if (error.message.includes(secret)) throw Error('value echoed in error');
         }
         const lone = 'A' + String.fromCharCode(0xD800) + 'B';
         expectCode(() => createKey('HKCU', lone), 'WINREGISTRY_ENCODING');
         expectCode(() => writeString('HKCU', '', 'Value', lone), 'WINREGISTRY_ENCODING');
         expectCode(() => readString('HKCU', lone, 'Value'), 'WINREGISTRY_ENCODING');
         expectCode(() => writeUint64('HKCU', '', 'Value', -1n), 'WINREGISTRY_INVALID_VALUE');
         expectCode(
             () => writeUint64('HKCU', '', 'Value', 18446744073709551616n),
             'WINREGISTRY_INVALID_VALUE'
         );
         "#,
    )
    .unwrap();
}

#[test]
fn javascript_argument_types_are_not_coerced() {
    let js = runtime(true);
    js.run_module(
        "argument-types.js",
        "import {createKey, listSubKeys, readString, writeBinary, writeString,
                 writeUint32, writeUint64}
                 from 'neo:winregistry';

         function expectTypeError(fn) {
             let seen = false;
             try { fn(); } catch (error) { seen = error instanceof TypeError; }
             if (!seen) throw Error('expected TypeError');
         }

         expectTypeError(() => createKey(1, 'Software'));
         expectTypeError(() => createKey('HKCU', 2));
         expectTypeError(() => readString('HKCU', '', 3));
         expectTypeError(() => writeString('HKCU', '', 'Value', 4));
         expectTypeError(() => writeUint32('HKCU', '', 'Value', '1'));
         expectTypeError(() => writeUint32('HKCU', '', 'Value', 1.5));
         expectTypeError(() => writeUint32('HKCU', '', 'Value', -1));
         expectTypeError(() => writeUint32('HKCU', '', 'Value', 4294967296));
         expectTypeError(() => writeUint32('HKCU', '', 'Value', NaN));
         expectTypeError(() => writeUint64('HKCU', '', 'Value', 1));
         expectTypeError(() => writeBinary('HKCU', '', 'Value', [1, 2]));
         expectTypeError(() => listSubKeys('HKCU', '', '10'));
         expectTypeError(() => listSubKeys('HKCU', '', 1.5));
         expectTypeError(() => listSubKeys('HKCU', '', 0));
         expectTypeError(() => listSubKeys('HKCU', '', 4097));",
    )
    .unwrap();
}

#[test]
fn oversized_and_exotic_inputs_are_rejected_before_conversion() {
    let js = runtime(true);
    js.run_module(
        "precopy-limits.js",
        r#"import {createKey, deleteValue, writeBinary, writeString} from 'neo:winregistry';

         function expectCode(fn, code) {
             let seen = null;
             try { fn(); } catch (error) { seen = error.code; }
             if (seen !== code) throw Error('expected ' + code + ', got ' + seen);
         }

         function expectTypeError(fn) {
             let seen = false;
             try { fn(); } catch (error) { seen = error instanceof TypeError; }
             if (!seen) throw Error('expected TypeError');
         }

         const huge = 'a'.repeat(1024 * 1024);
         expectCode(() => createKey('HKCU', huge), 'WINREGISTRY_INVALID_PATH');
         expectCode(() => deleteValue('HKCU', '', huge), 'WINREGISTRY_INVALID_NAME');
         expectCode(
             () => writeString('HKCU', '', 'Value', 'a'.repeat(4194304)),
             'WINREGISTRY_LIMIT'
         );
         expectCode(
             () => deleteValue('HKCU', '', 'a'.repeat(16384)),
             'WINREGISTRY_INVALID_NAME'
         );
         expectCode(
             () => writeBinary('HKCU', '', 'Value', new Uint8Array(8388609)),
             'WINREGISTRY_LIMIT'
         );

         const throwing = {
             get() { throw Error('property read'); },
             has() { throw Error('property read'); }
         };
         const proxy = new Proxy({}, throwing);
         expectTypeError(() => createKey(proxy, 'Software'));
         expectTypeError(() => writeString('HKCU', '', 'Value', proxy));
         expectTypeError(() => writeBinary('HKCU', '', 'Value', proxy));
         expectTypeError(() => createKey(new String('HKCU'), 'Software'));
         expectTypeError(() => writeBinary('HKCU', '', 'Value', new DataView(new ArrayBuffer(1))));
         expectTypeError(
             () => writeBinary('HKCU', '', 'Value', new Proxy(new Uint8Array(1), throwing))
         );"#,
    )
    .unwrap();
}

#[cfg(not(windows))]
#[test]
fn mutated_intrinsics_and_own_length_do_not_bypass_validation() {
    let js = runtime(true);
    js.run_module(
        "intrinsic-overrides.js",
        r#"import {createKey, writeBinary, writeUint64} from 'neo:winregistry';
        function expectCode(fn, code) {
            let seen;
            try { fn(); } catch (error) { seen = error.code; }
            if (seen !== code) throw Error('expected ' + code + ', got ' + seen);
        }
        Object.defineProperty(Error.prototype, 'code', {set() { throw Error('code setter called'); }});
        RegExp.prototype.exec = () => [''];
        expectCode(() => createKey('HKCU', '\ud800'), 'WINREGISTRY_ENCODING');
        BigInt.prototype.toString = () => '7'.repeat(1000);
        expectCode(() => writeUint64('HKCU', '', 'v', 1n), 'WINREGISTRY_UNSUPPORTED');
        Object.defineProperty(Uint8Array, Symbol.hasInstance, {value() { throw Error('hasInstance called'); }});
        expectCode(() => writeBinary('HKCU', '', 'v', new Uint8Array(1)), 'WINREGISTRY_UNSUPPORTED');
        const bytes = new Uint8Array(8388609);
        Object.defineProperty(bytes, 'length', {get() { return 0; }});
        expectCode(() => writeBinary('HKCU', '', 'v', bytes), 'WINREGISTRY_LIMIT');
        "#,
    )
    .unwrap();
}

#[cfg(not(windows))]
#[test]
fn maximum_accepted_inputs_reach_the_unsupported_host() {
    let js = runtime(true);
    js.run_module(
        "precopy-boundaries.js",
        r#"import {deleteValue, writeBinary, writeString} from 'neo:winregistry';

         function expectCode(fn, code) {
             let seen = null;
             try { fn(); } catch (error) { seen = error.code; }
             if (seen !== code) throw Error('expected ' + code + ', got ' + seen);
         }

         // These are exactly at the documented caps. They pass the wrapper and
         // native validation, then report that this host has no registry.
         expectCode(
             () => writeString('HKCU', '', 'Value', 'a'.repeat(4194303)),
             'WINREGISTRY_UNSUPPORTED'
         );
         expectCode(() => deleteValue('HKCU', '', 'a'.repeat(16383)), 'WINREGISTRY_UNSUPPORTED');
         expectCode(
             () => writeBinary('HKCU', '', 'Value', new Uint8Array(8388608)),
             'WINREGISTRY_UNSUPPORTED'
         );"#,
    )
    .unwrap();
}
