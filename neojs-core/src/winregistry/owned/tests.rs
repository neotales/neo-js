use std::collections::BTreeMap;

use super::*;
use crate::{NeoJs, Permissions, RuntimeModule, RuntimeOptions};

#[derive(Default)]
struct Audit {
    acquired: Vec<(Hive, String, u32, bool)>,
    closed: Vec<usize>,
    reads: Vec<usize>,
    writes: Vec<usize>,
}

struct TestHandle {
    audit: Rc<RefCell<Audit>>,
    id: usize,
    values: RefCell<BTreeMap<String, Vec<u8>>>,
}

impl Drop for TestHandle {
    fn drop(&mut self) {
        self.audit.borrow_mut().closed.push(self.id);
    }
}

impl Handle for TestHandle {
    fn read(&self, name: &str) -> Result<(u32, Vec<u8>), RegistryError> {
        self.audit.borrow_mut().reads.push(self.id);
        self.values
            .borrow()
            .get(name)
            .cloned()
            .map(|bytes| (TYPE_SZ, bytes))
            .ok_or_else(|| RegistryError::new(ErrorCode::NotFound))
    }

    fn write(&self, name: &str, bytes: &[u8]) -> Result<(), RegistryError> {
        self.audit.borrow_mut().writes.push(self.id);
        self.values
            .borrow_mut()
            .insert(name.to_owned(), bytes.to_vec());
        Ok(())
    }
}

fn runtime() -> (NeoJs, Rc<RefCell<Audit>>) {
    let js = NeoJs::new_with_options(
        Permissions {
            registry: true,
            ..Permissions::default()
        },
        RuntimeOptions::new().modules([RuntimeModule::WinRegistry]),
    )
    .unwrap();
    let audit = Rc::new(RefCell::new(Audit::default()));
    let backend_audit = Rc::clone(&audit);
    js.winregistry.borrow_mut().keys.backend = Some(Box::new(move |hive, path, access, create| {
        let id = {
            let mut audit = backend_audit.borrow_mut();
            let id = audit.acquired.len();
            audit.acquired.push((hive, path.to_owned(), access, create));
            id
        };
        if path == "Missing" {
            return Err(RegistryError::new(ErrorCode::NotFound));
        }
        Ok((
            Box::new(TestHandle {
                audit: Rc::clone(&backend_audit),
                id,
                values: RefCell::default(),
            }),
            create && path != "Existing",
        ))
    }));
    (js, audit)
}

#[test]
fn owned_keys_run_using_in_js_and_ts_and_close_once_in_reverse_order() {
    for (entry, annotation) in [("owned.js", ""), ("owned.ts", ": RegistryKey")] {
        for fail in [false, true] {
            let (js, audit) = runtime();
            let exit = if fail {
                "throw Error('body failed');"
            } else {
                ""
            };
            let source = format!(
                r#"import {{ Registry, RegistryKey }} from 'neo:win-registry';
                 using first{annotation} = Registry.createKey('hkcu\\Software\\Example');
                 using second{annotation} = Registry.openKey('HKEY_CURRENT_USER\\Software\\Example');
                 if (!(first instanceof RegistryKey) || first.path !== 'hkcu\\Software\\Example' ||
                     !first.created || second.created || first.close !== first[Symbol.dispose])
                     throw Error('key metadata');
                 globalThis.first = first;
                 globalThis.second = second;
                 first.setString('', 'tekst 🦀');
                 if (first.getString('') !== 'tekst 🦀') throw Error('value did not round-trip');
                 {exit}"#
            );
            let result = js.run_module(entry, &source);
            if fail {
                assert!(result.unwrap_err().contains("body failed"));
            } else {
                result.unwrap();
            }
            assert_eq!(audit.borrow().closed, [1, 0]);
            js.run_module(
                "check-closed.js",
                "for (const key of [globalThis.first, globalThis.second]) {
                    for (const op of [() => key.getString(''), () => key.setString('', '')]) {
                        let code;
                        try { op(); } catch (error) { code = error.code; }
                        if (code !== 'WINREGISTRY_CLOSED') throw Error('key was not closed');
                    }
                    key[Symbol.dispose]();
                    key.close();
                 }",
            )
            .unwrap();
            drop(js);
            let audit = audit.borrow();
            assert_eq!(
                audit.acquired,
                [
                    (
                        Hive::CurrentUser,
                        "Software\\Example".to_owned(),
                        ALL_ACCESS,
                        true
                    ),
                    (
                        Hive::CurrentUser,
                        "Software\\Example".to_owned(),
                        0x20019,
                        false
                    ),
                ]
            );
            assert_eq!(audit.reads, [0]);
            assert_eq!(audit.writes, [0]);
            assert_eq!(audit.closed, [1, 0]);
        }
    }
}

#[test]
fn owned_keys_ignore_prototype_hooks_and_reject_unbranded_receivers() {
    let (js, audit) = runtime();
    js.run_module(
        "owned-prototype-hooks.js",
        r#"let hooks = 0;
           for (const name of [
               'acquireOwned', 'created', 'getString', 'setString', 'close',
               'open', 'path', 'ok', 'value', 'get', 'set'
           ]) {
               Object.defineProperty(Object.prototype, name, {
                   __proto__: null, configurable: true,
                   get() { hooks++; throw Error('inherited getter ran'); },
                   set() { hooks++; throw Error('inherited setter ran'); }
               });
           }
           const { Registry, RegistryKey } = await import('neo:win-registry');
           using key = Registry.createKey('HKCU\\Synthetic');
           if (!key.created || key.path !== 'HKCU\\Synthetic') throw Error('invalid metadata');
           const proto = RegistryKey.prototype;
           const pathGetter = Object.getOwnPropertyDescriptor(proto, 'path').get;
           const createdGetter = Object.getOwnPropertyDescriptor(proto, 'created').get;
           for (const receiver of [
               Object.create(proto),
               new Proxy(key, { get() { hooks++; throw Error('proxy trap ran'); } })
           ]) {
               for (const method of [
                   proto.getString, proto.setString, proto.close, proto[Symbol.dispose],
                   pathGetter, createdGetter
               ]) {
                   let branded = false;
                   try { Reflect.apply(method, receiver, ['', '']); }
                   catch (error) { branded = error instanceof TypeError; }
                   if (!branded) throw Error('unbranded receiver was accepted');
               }
           }
           key.setString('', 'synthetic');
           if (key.getString('') !== 'synthetic' || hooks !== 0)
               throw Error('owned key operations were intercepted');"#,
    )
    .unwrap();
    let audit = audit.borrow();
    assert_eq!(audit.acquired.len(), 1);
    assert_eq!(audit.reads, [0]);
    assert_eq!(audit.writes, [0]);
    assert_eq!(audit.closed, [0]);
}

#[test]
fn acquisition_is_bounded_and_rights_and_runtime_teardown_are_enforced() {
    let (js, audit) = runtime();
    js.run_module(
        "owned-limits.js",
        r#"import { Registry, RegistryKey, Rights, MAX_OPEN_KEYS } from 'neo:winregistry';
           function expect(operation, expected) {
               let code;
               try { operation(); } catch (error) { code = error.code ?? error.name; }
               if (code !== expected) throw Error('unexpected failure category: ' + code);
           }
           expect(() => new RegistryKey(1n, 'anything'), 'TypeError');
           expect(() => Registry.openKey('HKCU'), 'WINREGISTRY_INVALID_PATH');
           expect(() => Registry.createKey('HKCU\\', 1), 'WINREGISTRY_INVALID_PATH');
           expect(() => Registry.openKey('HKCU\\Existing', 0), 'WINREGISTRY_INVALID_VALUE');
           expect(() => Registry.openKey('HKCU\\Existing', 0x100), 'WINREGISTRY_INVALID_VALUE');
           expect(() => Registry.openKey('HKCU\\Existing', 0x100000000), 'TypeError');
           expect(() => Registry.createKey('HKCU\\Missing'), 'WINREGISTRY_NOT_FOUND');
           globalThis.keys = [];
           for (let i = 0; i < MAX_OPEN_KEYS; i++) {
               keys.push(Registry.openKey('HKCU\\Existing', Rights.QUERY_VALUE));
           }
           expect(() => keys[0].setString('', ''), 'WINREGISTRY_ACCESS_DENIED');
           expect(() => Registry.createKey('HKCU\\Existing'), 'WINREGISTRY_LIMIT');
           keys[0].close();
           keys[0][Symbol.dispose]();
           globalThis.writer = Registry.createKey('HKCU\\Existing', Rights.SET_VALUE);
           if (writer.created || writer.path !== 'HKCU\\Existing' || 'unwrap' in writer)
               throw Error('key metadata or raw handle exposed');
           expect(() => writer.getString(''), 'WINREGISTRY_ACCESS_DENIED');
           writer.setString('', 'new value');"#,
    )
    .unwrap();
    assert_eq!(audit.borrow().acquired.len(), MAX_OPEN_KEYS + 2);
    assert_eq!(audit.borrow().closed, [1]);
    assert!(audit.borrow().reads.is_empty());
    assert_eq!(audit.borrow().writes, [MAX_OPEN_KEYS + 1]);
    drop(js);
    let mut closed = audit.borrow().closed.clone();
    closed.sort_unstable();
    assert_eq!(closed, (1..=MAX_OPEN_KEYS + 1).collect::<Vec<_>>());
}

#[test]
fn failed_response_construction_closes_a_partially_wrapped_handle() {
    let (js, audit) = runtime();
    let (slot, created) = js
        .winregistry
        .borrow_mut()
        .keys
        .acquire("HKCU\\New", ALL_ACCESS, true)
        .unwrap();
    assert!(created);
    js.context.with(|ctx| {
        let error = finish_acquisition(&ctx, &slot, |object| {
            install_callbacks(&ctx, object, &slot)?;
            Err(rquickjs::Error::Unknown)
        })
        .unwrap_err();
        assert!(matches!(error, rquickjs::Error::Unknown));
    });
    assert_eq!(audit.borrow().closed, [0]);
    drop(js);
    drop(slot);
    assert_eq!(audit.borrow().closed, [0]);
}

#[test]
fn collecting_an_unreachable_key_releases_its_handle_and_slot() {
    let (js, audit) = runtime();
    js.run_module(
        "owned-retained.js",
        r#"import { Registry } from 'neo:win-registry';
           const cycle = { key: Registry.openKey('HKCU\\Existing') };
           cycle.self = cycle;
           globalThis.cycle = cycle;"#,
    )
    .unwrap();
    // A top-level binding keeps the cycle alive as long as the module lives.
    // Put another cycle entirely inside a function so it can be collected.
    js.run_module(
        "owned-collectable.js",
        r#"import { Registry } from 'neo:win-registry';
           (function () {
               const cycle = { key: Registry.openKey('HKCU\\Existing') };
               cycle.self = cycle;
           })();"#,
    )
    .unwrap();
    js._runtime.run_gc();
    assert_eq!(audit.borrow().closed, [1]);
    js.run_module(
        "owned-refill.js",
        r#"import { Registry, MAX_OPEN_KEYS } from 'neo:win-registry';
           globalThis.keys = [];
           for (let i = 1; i < MAX_OPEN_KEYS; i++) {
               keys.push(Registry.openKey('HKCU\\Existing'));
           }"#,
    )
    .unwrap();
    assert_eq!(audit.borrow().acquired.len(), MAX_OPEN_KEYS + 1);
    drop(js);
    let mut closed = audit.borrow().closed.clone();
    closed.sort_unstable();
    assert_eq!(closed, (0..=MAX_OPEN_KEYS).collect::<Vec<_>>());
}
