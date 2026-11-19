//! Integration tests for static host-extension registration.
//!
//! These exercise the public API rather than the registry internals: exact
//! names, duplicate and core collisions, the independent permission and
//! allowlist gates for static and dynamic imports, cache reuse, initialization
//! and teardown order, and partial-failure behavior.

use std::{
    any::Any,
    cell::RefCell,
    rc::Rc,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use neojs_core::{
    ExtensionDescriptor, ExtensionModule, ExtensionPermission, ExtensionRegistry, ModuleAllowlist,
    NeoJs, Permissions, RuntimeModule, RuntimeOptions,
    rquickjs::{
        Ctx, Error, Function, Module, Persistent, Result as JsResult,
        module::{Declarations, Declared, Exports, ModuleDef},
    },
};

static LOCK: Mutex<()> = Mutex::new(());
static INIT_CALLS: AtomicUsize = AtomicUsize::new(0);
static LOAD_CALLS: AtomicUsize = AtomicUsize::new(0);
static TEARDOWN_CALLS: AtomicUsize = AtomicUsize::new(0);
static GETTER_CALLS: AtomicUsize = AtomicUsize::new(0);
static HOSTILE_DROPS: AtomicUsize = AtomicUsize::new(0);
static CHAIN_DROPS: AtomicUsize = AtomicUsize::new(0);
static MARKED_DROP_ORDER: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
static INIT_NAMES: Mutex<Vec<String>> = Mutex::new(Vec::new());
static ORDER: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct ProbeModule;

impl ModuleDef for ProbeModule {
    fn declare<'js>(decl: &Declarations<'js>) -> JsResult<()> {
        decl.declare("value")?;
        Ok(())
    }

    fn evaluate<'js>(_ctx: &Ctx<'js>, exports: &Exports<'js>) -> JsResult<()> {
        exports.export("value", 7i32)?;
        Ok(())
    }
}

struct ProbeState {
    names: Vec<String>,
    // A real JS value, held only on the runtime thread and released before the
    // runtime/context are dropped.
    persistent: Option<Persistent<Function<'static>>>,
}

/// Native module state stored in context userdata, matching the shape a native
/// extension such as a SQLite crate uses.
struct NativeProbeState {
    value: u32,
}

// The state contains no borrowed JS value; it is only native data.
unsafe impl<'js> neojs_core::rquickjs::JsLifetime<'js> for NativeProbeState {
    type Changed<'to> = NativeProbeState;
}

struct NativeProbeModule;

impl ModuleDef for NativeProbeModule {
    fn declare<'js>(decl: &Declarations<'js>) -> JsResult<()> {
        decl.declare("nativeValue")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> JsResult<()> {
        let state = ctx
            .userdata::<Rc<RefCell<NativeProbeState>>>()
            .ok_or(Error::Unknown)?;
        let value = state.borrow().value;
        exports.export("nativeValue", value)?;
        Ok(())
    }
}

struct DropPanicState;

impl Drop for DropPanicState {
    fn drop(&mut self) {
        panic!("probe state drop panic");
    }
}

/// A `panic_any` payload whose own destructor panics.
struct HostilePayload;

impl Drop for HostilePayload {
    fn drop(&mut self) {
        HOSTILE_DROPS.fetch_add(1, Ordering::SeqCst);
        panic!("hostile payload destructor panic");
    }
}

/// A hostile payload carrying a marker so a resumed payload can be identified.
struct MarkedHostilePayload(&'static str);

impl Drop for MarkedHostilePayload {
    fn drop(&mut self) {
        MARKED_DROP_ORDER
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(self.0);
        panic!("marked hostile payload destructor panic");
    }
}

/// A finite chain of payload destructor panics.
struct ChainPayload(u8);

impl Drop for ChainPayload {
    fn drop(&mut self) {
        CHAIN_DROPS.fetch_add(1, Ordering::SeqCst);
        if self.0 > 0 {
            std::panic::panic_any(ChainPayload(self.0 - 1));
        }
        panic!("end of hostile payload chain");
    }
}

/// Extension state whose Drop raises a marked hostile payload.
struct HostileState;

impl Drop for HostileState {
    fn drop(&mut self) {
        std::panic::panic_any(MarkedHostilePayload("later-state"));
    }
}

fn init<'js>(ctx: &Ctx<'js>, names: &[&str]) -> JsResult<Box<dyn Any>> {
    INIT_CALLS.fetch_add(1, Ordering::SeqCst);
    let names: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
    *INIT_NAMES.lock().unwrap() = names.clone();
    ORDER
        .lock()
        .unwrap()
        .push(format!("init:{}", names.join(",")));
    let function: Function = ctx.eval("() => 1")?;
    Ok(Box::new(ProbeState {
        names,
        persistent: Some(Persistent::save(ctx, function)),
    }))
}

fn teardown(state: &mut dyn Any) {
    TEARDOWN_CALLS.fetch_add(1, Ordering::SeqCst);
    let state = state.downcast_mut::<ProbeState>().expect("probe state");
    ORDER
        .lock()
        .unwrap()
        .push(format!("teardown:{}", state.names.join(",")));
    state.persistent = None;
}

fn load<'js>(ctx: &Ctx<'js>, _state: &mut dyn Any, name: &str) -> JsResult<Module<'js, Declared>> {
    LOAD_CALLS.fetch_add(1, Ordering::SeqCst);
    Module::declare_def::<ProbeModule, _>(ctx.clone(), name)
}

fn failing_load<'js>(
    _ctx: &Ctx<'js>,
    _state: &mut dyn Any,
    name: &str,
) -> JsResult<Module<'js, Declared>> {
    LOAD_CALLS.fetch_add(1, Ordering::SeqCst);
    Err(Error::new_loading_message(name, "probe load failed"))
}

fn native_probe_init<'js>(ctx: &Ctx<'js>, _names: &[&str]) -> JsResult<Box<dyn Any>> {
    ctx.store_userdata(Rc::new(RefCell::new(NativeProbeState { value: 23 })))
        .map_err(|_| Error::Unknown)?;
    Ok(Box::new(()))
}

fn native_probe_load<'js>(
    ctx: &Ctx<'js>,
    _state: &mut dyn Any,
    name: &str,
) -> JsResult<Module<'js, Declared>> {
    Module::declare_def::<NativeProbeModule, _>(ctx.clone(), name)
}

fn state_drop_init<'js>(_ctx: &Ctx<'js>, _names: &[&str]) -> JsResult<Box<dyn Any>> {
    Ok(Box::new(DropPanicState))
}

fn normal_teardown(state: &mut dyn Any) {
    TEARDOWN_CALLS.fetch_add(1, Ordering::SeqCst);
    let label = state
        .downcast_mut::<DropPanicState>()
        .map(|_| "drop-panic-state")
        .unwrap_or("unknown");
    ORDER
        .lock()
        .unwrap()
        .push(format!("teardown-normal:{label}"));
}

fn fail_init<'js>(_ctx: &Ctx<'js>, names: &[&str]) -> JsResult<Box<dyn Any>> {
    INIT_CALLS.fetch_add(1, Ordering::SeqCst);
    ORDER
        .lock()
        .unwrap()
        .push(format!("init-failed:{}", names.join(",")));
    Err(Error::new_from_js_message(
        "extension",
        "init",
        "probe init failed",
    ))
}

fn panic_init<'js>(_ctx: &Ctx<'js>, names: &[&str]) -> JsResult<Box<dyn Any>> {
    INIT_CALLS.fetch_add(1, Ordering::SeqCst);
    ORDER
        .lock()
        .unwrap()
        .push(format!("init-panic:{}", names.join(",")));
    panic!("probe init panic")
}

fn js_throwing_init<'js>(ctx: &Ctx<'js>, names: &[&str]) -> JsResult<Box<dyn Any>> {
    INIT_CALLS.fetch_add(1, Ordering::SeqCst);
    ORDER
        .lock()
        .unwrap()
        .push(format!("init-js-error:{}", names.join(",")));
    let _: () = ctx.eval("throw new Error('probe js init failure')")?;
    Ok(Box::new(()))
}

fn js_getter_init<'js>(ctx: &Ctx<'js>, names: &[&str]) -> JsResult<Box<dyn Any>> {
    INIT_CALLS.fetch_add(1, Ordering::SeqCst);
    ORDER
        .lock()
        .unwrap()
        .push(format!("init-js-getter:{}", names.join(",")));
    let hit = Function::new(ctx.clone(), || {
        GETTER_CALLS.fetch_add(1, Ordering::SeqCst);
    })?;
    ctx.globals().set("__neoExtensionGetterHit", hit)?;
    let _: () = ctx.eval(
        r#"(() => {
            const error = new Error('probe getter message');
            Object.defineProperty(error, 'message', {
                configurable: true,
                get() {
                    __neoExtensionGetterHit();
                    throw new Error('getter invoked');
                },
            });
            throw error;
        })()"#,
    )?;
    Ok(Box::new(()))
}

fn panic_teardown(state: &mut dyn Any) {
    TEARDOWN_CALLS.fetch_add(1, Ordering::SeqCst);
    let label = state
        .downcast_mut::<ProbeState>()
        .map(|state| state.names.join(","))
        .unwrap_or_else(|| "unknown".to_owned());
    ORDER
        .lock()
        .unwrap()
        .push(format!("teardown-panic:{label}"));
    panic!("probe teardown panic")
}

fn hostile_teardown(_state: &mut dyn Any) {
    TEARDOWN_CALLS.fetch_add(1, Ordering::SeqCst);
    ORDER.lock().unwrap().push("teardown-hostile".to_owned());
    std::panic::panic_any(HostilePayload);
}

fn marked_hook_teardown(_state: &mut dyn Any) {
    TEARDOWN_CALLS.fetch_add(1, Ordering::SeqCst);
    ORDER
        .lock()
        .unwrap()
        .push("teardown-marked-hook".to_owned());
    std::panic::panic_any(MarkedHostilePayload("first-hook"));
}

fn hostile_init<'js>(_ctx: &Ctx<'js>, _names: &[&str]) -> JsResult<Box<dyn Any>> {
    std::panic::panic_any(HostilePayload);
}

fn chain_init<'js>(_ctx: &Ctx<'js>, _names: &[&str]) -> JsResult<Box<dyn Any>> {
    std::panic::panic_any(ChainPayload(3));
}

fn hostile_state_init<'js>(_ctx: &Ctx<'js>, _names: &[&str]) -> JsResult<Box<dyn Any>> {
    Ok(Box::new(HostileState))
}

fn descriptor(id: &str, module: &str, permission: ExtensionPermission) -> ExtensionDescriptor {
    ExtensionDescriptor::new(id, load)
        .with_module(module, permission)
        .with_init(init)
        .with_teardown(teardown)
}

fn lock() -> std::sync::MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Integration tests cannot use the crate-private disposal helper, so mirror
/// the same finite-chain disposal for a payload the test itself caught.
fn dispose_payload(mut payload: Box<dyn Any + Send>) {
    while let Err(next) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(payload))) {
        payload = next;
    }
}

fn reset() {
    INIT_CALLS.store(0, Ordering::SeqCst);
    LOAD_CALLS.store(0, Ordering::SeqCst);
    TEARDOWN_CALLS.store(0, Ordering::SeqCst);
    GETTER_CALLS.store(0, Ordering::SeqCst);
    HOSTILE_DROPS.store(0, Ordering::SeqCst);
    CHAIN_DROPS.store(0, Ordering::SeqCst);
    INIT_NAMES.lock().unwrap().clear();
    ORDER.lock().unwrap().clear();
}

fn construct(options: RuntimeOptions) -> std::result::Result<NeoJs, Error> {
    NeoJs::new_with_options(Permissions::default(), options)
}

#[test]
fn registration_rejects_duplicates_core_collisions_and_invalid_names() {
    let _guard = lock();
    let mut registry = ExtensionRegistry::new();
    registry
        .register(descriptor(
            "one",
            "neo:extension-one",
            ExtensionPermission::None,
        ))
        .unwrap();
    assert_eq!(registry.len(), 1);

    let duplicate_extension = registry
        .register(descriptor(
            "one",
            "neo:extension-two",
            ExtensionPermission::None,
        ))
        .unwrap_err();
    assert_eq!(duplicate_extension.code(), "EXTENSION_DUPLICATE_EXTENSION");
    assert!(!registry.contains_module("neo:extension-two"));

    let duplicate_module = registry
        .register(descriptor(
            "two",
            "neo:extension-one",
            ExtensionPermission::None,
        ))
        .unwrap_err();
    assert_eq!(duplicate_module.code(), "EXTENSION_DUPLICATE_MODULE");

    let core_collision = registry
        .register(descriptor("two", "neo:fs", ExtensionPermission::Filesystem))
        .unwrap_err();
    assert_eq!(core_collision.code(), "EXTENSION_CORE_MODULE_COLLISION");

    // The built-in SQLite name is reserved exactly like any other core module.
    let sqlite_collision = registry
        .register(descriptor(
            "sqlite",
            "neo:sqlite",
            ExtensionPermission::Sqlite,
        ))
        .unwrap_err();
    assert_eq!(sqlite_collision.code(), "EXTENSION_CORE_MODULE_COLLISION");

    let within_descriptor = registry
        .register(
            ExtensionDescriptor::new("three", load)
                .with_module("neo:extension-three", ExtensionPermission::None)
                .with_module("neo:extension-three", ExtensionPermission::None),
        )
        .unwrap_err();
    assert_eq!(within_descriptor.code(), "EXTENSION_DUPLICATE_MODULE");

    for invalid in ["fs", "neo:", "neo:/leading", "neo:trailing/", "neo:a//b"] {
        let error = registry
            .register(descriptor("invalid", invalid, ExtensionPermission::None))
            .unwrap_err();
        assert_eq!(
            error.code(),
            "EXTENSION_INVALID_MODULE_NAME",
            "{invalid} was accepted"
        );
    }

    let empty_id = ExtensionRegistry::new()
        .register(ExtensionDescriptor::new(" ", load))
        .unwrap_err();
    assert_eq!(empty_id.code(), "EXTENSION_EMPTY_ID");

    let mut multi = ExtensionRegistry::new();
    multi
        .register(ExtensionDescriptor::new("multi", load).with_modules([
            ExtensionModule::new("neo:extension-multi-a", ExtensionPermission::None),
            ExtensionModule::new("neo:extension-multi-b", ExtensionPermission::Network),
        ]))
        .unwrap();
    assert!(multi.contains_module("neo:extension-multi-a"));
    assert!(multi.contains_module("neo:extension-multi-b"));
}

#[test]
fn allowlist_builders_distinguish_registered_extension_names() {
    let registry = ExtensionRegistry::from_descriptors([descriptor(
        "probe",
        "neo:extension-probe",
        ExtensionPermission::None,
    )])
    .unwrap();

    let unknown = ModuleAllowlist::from_names_with(
        ["neo:extension-probe", "neo:missing", "neo:Hash"],
        &registry,
    )
    .unwrap_err();
    assert_eq!(unknown.names(), ["neo:missing", "neo:Hash"]);

    let allowlist =
        ModuleAllowlist::from_names_with(["neo:extension-probe", "neo:fs"], &registry).unwrap();
    assert!(allowlist.contains(RuntimeModule::Fs));
    assert!(allowlist.contains_extension_name("neo:extension-probe"));
    assert_eq!(
        allowlist.extension_names().collect::<Vec<_>>(),
        ["neo:extension-probe"]
    );
    assert_eq!(allowlist.len(), 2);
    assert_eq!(allowlist.builtin_len(), 1);

    // Built-in-only construction still rejects an extension name, so a typo
    // cannot silently become a different configuration.
    let builtin_only = ModuleAllowlist::from_names(["neo:extension-probe"]).unwrap_err();
    assert_eq!(builtin_only.names(), ["neo:extension-probe"]);

    let all = ModuleAllowlist::all_with_extensions(&registry);
    assert!(all.contains(RuntimeModule::Fs));
    assert!(all.contains_extension_name("neo:extension-probe"));

    let options = RuntimeOptions::new()
        .with_extensions([descriptor(
            "probe",
            "neo:extension-probe",
            ExtensionPermission::None,
        )])
        .unwrap();
    let options = options
        .module_names(["neo:extension-probe"])
        .expect("registered extension name is known to these options");
    assert!(options.is_module_name_allowed("neo:extension-probe"));
    assert!(!options.is_module_name_allowed("neo:fs"));

    let without_extension = RuntimeOptions::new()
        .module_names(["neo:extension-probe"])
        .unwrap_err();
    assert_eq!(without_extension.names(), ["neo:extension-probe"]);
}

#[test]
fn unregistered_extension_is_not_importable() {
    let _guard = lock();
    reset();
    let runtime = NeoJs::new(Permissions::default()).unwrap();
    assert!(
        runtime
            .run_module("unregistered.js", "import 'neo:extension-probe';")
            .is_err()
    );
    assert_eq!(INIT_CALLS.load(Ordering::SeqCst), 0);
    assert_eq!(LOAD_CALLS.load(Ordering::SeqCst), 0);
}

#[test]
fn extension_import_is_cached_and_initialized_once() {
    let _guard = lock();
    reset();
    let options = RuntimeOptions::new()
        .with_extensions([descriptor(
            "probe",
            "neo:extension-probe",
            ExtensionPermission::None,
        )])
        .unwrap();
    let runtime = NeoJs::new_with_options(Permissions::default(), options).unwrap();
    runtime
        .run_module(
            "first.js",
            "import {value} from 'neo:extension-probe';
             if (value !== 7) throw Error('bad value');
             globalThis.firstValue = value;",
        )
        .unwrap();
    runtime
        .run_module(
            "second.js",
            "const first = await import('neo:extension-probe');
             const second = await import('neo:extension-probe');
             if (first.value !== 7 || first !== second || globalThis.firstValue !== 7)
                 throw Error('cache identity');",
        )
        .unwrap();
    assert_eq!(INIT_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(LOAD_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(
        *INIT_NAMES.lock().unwrap(),
        ["neo:extension-probe".to_owned()]
    );
    drop(runtime);
    assert_eq!(TEARDOWN_CALLS.load(Ordering::SeqCst), 1);
}

#[test]
fn explicit_allowlist_denies_static_dynamic_and_cached_imports_without_init() {
    let _guard = lock();
    reset();
    let options = RuntimeOptions::new()
        .with_extensions([descriptor(
            "probe",
            "neo:extension-probe",
            ExtensionPermission::None,
        )])
        .unwrap()
        .modules([RuntimeModule::Path]);
    let runtime = NeoJs::new_with_options(Permissions::default(), options).unwrap();

    let static_error = runtime
        .run_module("deny-static.js", "import 'neo:extension-probe';")
        .unwrap_err();
    assert!(static_error.contains("allowlist"), "{static_error}");

    runtime
        .run_module(
            "deny-dynamic.js",
            "let leaked = false;
             let message = '';
             try { await import('neo:extension-probe'); leaked = true; }
             catch (error) { message = String(error); }
             if (!message.includes('allowlist')) throw Error('wrong error: ' + message);
             if (leaked) throw Error('dynamic import leaked');",
        )
        .unwrap();

    assert_eq!(INIT_CALLS.load(Ordering::SeqCst), 0);
    assert_eq!(LOAD_CALLS.load(Ordering::SeqCst), 0);
    drop(runtime);
    assert_eq!(TEARDOWN_CALLS.load(Ordering::SeqCst), 0);
}

#[test]
fn permission_gates_the_module_and_skips_init_when_denied() {
    let _guard = lock();
    reset();
    let options = RuntimeOptions::new()
        .with_extensions([descriptor(
            "probe",
            "neo:extension-probe",
            ExtensionPermission::Environment,
        )])
        .unwrap();

    let denied = NeoJs::new_with_options(Permissions::default(), options.clone()).unwrap();
    assert!(
        denied
            .run_module("denied.js", "import 'neo:extension-probe';")
            .is_err()
    );
    assert_eq!(INIT_CALLS.load(Ordering::SeqCst), 0);
    drop(denied);
    assert_eq!(TEARDOWN_CALLS.load(Ordering::SeqCst), 0);

    let allowed = NeoJs::new_with_options(
        Permissions {
            environment: true,
            ..Permissions::default()
        },
        options,
    )
    .unwrap();
    allowed
        .run_module(
            "allowed.js",
            "import {value} from 'neo:extension-probe';
             if (value !== 7) throw Error('bad value');",
        )
        .unwrap();
    assert_eq!(INIT_CALLS.load(Ordering::SeqCst), 1);
    drop(allowed);
    assert_eq!(TEARDOWN_CALLS.load(Ordering::SeqCst), 1);
}

#[test]
fn init_receives_only_names_admitted_by_both_gates() {
    let _guard = lock();
    reset();
    let options = RuntimeOptions::new()
        .with_extensions([ExtensionDescriptor::new("probe", load)
            .with_module("neo:extension-a", ExtensionPermission::None)
            .with_module("neo:extension-b", ExtensionPermission::None)
            .with_init(init)
            .with_teardown(teardown)])
        .unwrap()
        .module_names(["neo:extension-a"])
        .unwrap();
    let runtime = NeoJs::new_with_options(Permissions::default(), options).unwrap();
    assert_eq!(*INIT_NAMES.lock().unwrap(), ["neo:extension-a".to_owned()]);
    runtime
        .run_module(
            "admitted.js",
            "import {value} from 'neo:extension-a';
             if (value !== 7) throw Error('bad value');",
        )
        .unwrap();
    let denied = runtime
        .run_module("withdrawn.js", "import 'neo:extension-b';")
        .unwrap_err();
    assert!(denied.contains("allowlist"), "{denied}");
    drop(runtime);
    assert_eq!(TEARDOWN_CALLS.load(Ordering::SeqCst), 1);
}

#[test]
fn lifecycle_runs_init_in_registration_order_and_teardown_in_reverse() {
    let _guard = lock();
    reset();
    let options = RuntimeOptions::new()
        .with_extensions([
            descriptor("a", "neo:extension-a", ExtensionPermission::None),
            descriptor("b", "neo:extension-b", ExtensionPermission::None),
        ])
        .unwrap();
    let runtime = NeoJs::new_with_options(Permissions::default(), options).unwrap();
    runtime
        .run_module(
            "both.js",
            "import 'neo:extension-a';
             import 'neo:extension-b';",
        )
        .unwrap();
    drop(runtime);
    assert_eq!(
        *ORDER.lock().unwrap(),
        [
            "init:neo:extension-a",
            "init:neo:extension-b",
            "teardown:neo:extension-b",
            "teardown:neo:extension-a",
        ]
    );
}

#[test]
fn failed_init_tears_down_earlier_extensions_and_returns_no_runtime() {
    let _guard = lock();
    reset();
    let bad = ExtensionDescriptor::new("bad", load)
        .with_module("neo:extension-bad", ExtensionPermission::None)
        .with_init(fail_init)
        .with_teardown(teardown);
    let options = RuntimeOptions::new()
        .with_extensions([
            descriptor("good", "neo:extension-good", ExtensionPermission::None),
            bad,
        ])
        .unwrap();
    let error = match construct(options) {
        Ok(_) => panic!("runtime was constructed after a failed init"),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains("probe init failed"),
        "unexpected error: {error}"
    );
    assert_eq!(
        *ORDER.lock().unwrap(),
        [
            "init:neo:extension-good",
            "init-failed:neo:extension-bad",
            "teardown:neo:extension-good",
        ]
    );
    assert_eq!(TEARDOWN_CALLS.load(Ordering::SeqCst), 1);
}

#[test]
fn failed_module_load_does_not_poison_the_runtime_or_skip_teardown() {
    let _guard = lock();
    reset();
    let extension = ExtensionDescriptor::new("bad-load", failing_load)
        .with_module("neo:extension-bad-load", ExtensionPermission::None)
        .with_init(init)
        .with_teardown(teardown);
    let options = RuntimeOptions::new().with_extensions([extension]).unwrap();
    let runtime = NeoJs::new_with_options(Permissions::default(), options).unwrap();
    let error = runtime
        .run_module("bad-load.js", "import 'neo:extension-bad-load';")
        .unwrap_err();
    assert!(error.contains("probe load failed"), "{error}");

    runtime
        .run_module("after-load.js", "globalThis.afterLoad = true;")
        .unwrap();
    assert_eq!(
        runtime.eval("String(globalThis.afterLoad)").unwrap(),
        "true"
    );
    drop(runtime);
    assert_eq!(TEARDOWN_CALLS.load(Ordering::SeqCst), 1);
}

#[test]
fn extension_init_can_store_native_userdata_for_its_module() {
    let _guard = lock();
    let extension = ExtensionDescriptor::new("native-probe", native_probe_load)
        .with_module("neo:native-probe", ExtensionPermission::None)
        .with_init(native_probe_init);
    let options = RuntimeOptions::new().with_extensions([extension]).unwrap();
    let runtime = NeoJs::new_with_options(Permissions::default(), options).unwrap();
    runtime
        .run_module(
            "native-probe.js",
            "import {nativeValue} from 'neo:native-probe';
             if (nativeValue !== 23) throw Error('bad native value');",
        )
        .unwrap();
}

#[test]
fn sqlite_permission_can_gate_an_external_module_name() {
    let _guard = lock();
    reset();
    let options = RuntimeOptions::new()
        .with_extensions([descriptor(
            "sqlite-candidate",
            "neo:sqlite-candidate",
            ExtensionPermission::Sqlite,
        )])
        .unwrap();

    let denied = NeoJs::new_with_options(Permissions::default(), options.clone()).unwrap();
    assert!(
        denied
            .run_module("sqlite-denied.js", "import 'neo:sqlite-candidate';")
            .is_err()
    );
    assert_eq!(INIT_CALLS.load(Ordering::SeqCst), 0);
    drop(denied);

    let allowed = NeoJs::new_with_options(
        Permissions {
            sqlite: true,
            ..Permissions::default()
        },
        options,
    )
    .unwrap();
    allowed
        .run_module(
            "sqlite-allowed.js",
            "import {value} from 'neo:sqlite-candidate';
             if (value !== 7) throw Error('bad value');",
        )
        .unwrap();
    assert_eq!(INIT_CALLS.load(Ordering::SeqCst), 1);
}

#[test]
fn init_panic_tears_down_earlier_extensions_and_reports_payload() {
    let _guard = lock();
    reset();
    let bad = ExtensionDescriptor::new("bad-panic", load)
        .with_module("neo:extension-bad-panic", ExtensionPermission::None)
        .with_init(panic_init)
        .with_teardown(teardown);
    let options = RuntimeOptions::new()
        .with_extensions([
            descriptor("good", "neo:extension-good", ExtensionPermission::None),
            bad,
        ])
        .unwrap();
    let error = match construct(options) {
        Ok(_) => panic!("runtime was constructed after a panicking init"),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains("probe init panic"),
        "unexpected error: {error}"
    );
    assert_eq!(
        *ORDER.lock().unwrap(),
        [
            "init:neo:extension-good",
            "init-panic:neo:extension-bad-panic",
            "teardown:neo:extension-good",
        ]
    );
    assert_eq!(TEARDOWN_CALLS.load(Ordering::SeqCst), 1);
}

#[test]
fn hostile_init_payload_is_disposed_and_earlier_cleanup_runs() {
    let _guard = lock();
    reset();
    let bad = ExtensionDescriptor::new("bad-hostile-init", load)
        .with_module("neo:extension-bad-hostile-init", ExtensionPermission::None)
        .with_init(hostile_init);
    let options = RuntimeOptions::new()
        .with_extensions([
            descriptor("good", "neo:extension-good", ExtensionPermission::None),
            bad,
        ])
        .unwrap();
    let error = match construct(options) {
        Ok(_) => panic!("runtime was constructed after a hostile init panic"),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains("non-string panic payload"),
        "unexpected error: {error}"
    );
    assert_eq!(HOSTILE_DROPS.load(Ordering::SeqCst), 1);
    assert!(
        ORDER
            .lock()
            .unwrap()
            .iter()
            .any(|entry| entry == "teardown:neo:extension-good")
    );
}

#[test]
fn finite_hostile_init_payload_chain_is_disposed() {
    let _guard = lock();
    reset();
    let bad = ExtensionDescriptor::new("chain-init", load)
        .with_module("neo:extension-chain-init", ExtensionPermission::None)
        .with_init(chain_init);
    let error = match construct(RuntimeOptions::new().with_extensions([bad]).unwrap()) {
        Ok(_) => panic!("runtime was constructed after a hostile init panic"),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains("non-string panic payload"),
        "unexpected error: {error}"
    );
    assert_eq!(CHAIN_DROPS.load(Ordering::SeqCst), 4);
}

#[test]
fn simultaneous_hook_state_payload_panics_are_contained_and_first_resumed() {
    let _guard = lock();
    reset();
    MARKED_DROP_ORDER.lock().unwrap().clear();
    let bad = ExtensionDescriptor::new("bad-both", load)
        .with_module("neo:extension-bad-both", ExtensionPermission::None)
        .with_init(hostile_state_init)
        .with_teardown(marked_hook_teardown);
    let options = RuntimeOptions::new()
        .with_extensions([
            descriptor("good", "neo:extension-good", ExtensionPermission::None),
            bad,
        ])
        .unwrap();
    let runtime = NeoJs::new_with_options(Permissions::default(), options).unwrap();
    let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(runtime)))
        .expect_err("the first hostile teardown payload was not resumed");
    // Capture owned data before disposing the payload. Asserting while the
    // hostile payload is still alive would abort on a regression because the
    // assertion panic would drop a payload whose destructor also panics.
    let marker = payload
        .downcast_ref::<MarkedHostilePayload>()
        .map(|payload| payload.0);
    dispose_payload(payload);
    assert_eq!(
        marker,
        Some("first-hook"),
        "the resumed payload was not the first hook panic"
    );
    // Drop order proves the later state payload was disposed before the first
    // hook payload was resumed to the caller.
    assert_eq!(
        *MARKED_DROP_ORDER.lock().unwrap(),
        ["later-state", "first-hook"]
    );
    let order = ORDER.lock().unwrap();
    assert!(
        order.iter().any(|entry| entry == "teardown-marked-hook"),
        "{order:?}"
    );
    assert!(
        order
            .iter()
            .any(|entry| entry == "teardown:neo:extension-good"),
        "later teardown did not run: {order:?}"
    );
    assert_eq!(TEARDOWN_CALLS.load(Ordering::SeqCst), 2);
}

#[test]
fn simultaneous_hook_state_payload_panics_during_outer_unwind_are_contained() {
    let _guard = lock();
    reset();
    MARKED_DROP_ORDER.lock().unwrap().clear();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let bad = ExtensionDescriptor::new("bad-both", load)
            .with_module("neo:extension-bad-both", ExtensionPermission::None)
            .with_init(hostile_state_init)
            .with_teardown(marked_hook_teardown);
        let options = RuntimeOptions::new()
            .with_extensions([
                descriptor("good", "neo:extension-good", ExtensionPermission::None),
                bad,
            ])
            .unwrap();
        let _runtime = NeoJs::new_with_options(Permissions::default(), options).unwrap();
        panic!("outer panic");
    }));
    let payload = result.expect_err("the outer panic was not caught");
    // Own the message before disposal: a regression that resumes the hostile
    // payload must not be dropped by a failing assertion.
    let message = payload
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned());
    dispose_payload(payload);
    assert_eq!(message.as_deref(), Some("outer panic"));
    // Both hostile payloads were disposed; the first was not resumed during
    // the existing unwind.
    assert_eq!(
        *MARKED_DROP_ORDER.lock().unwrap(),
        ["later-state", "first-hook"]
    );
    let order = ORDER.lock().unwrap();
    assert!(
        order.iter().any(|entry| entry == "teardown-marked-hook"),
        "{order:?}"
    );
    assert!(
        order
            .iter()
            .any(|entry| entry == "teardown:neo:extension-good"),
        "later teardown did not run during unwind: {order:?}"
    );
    assert_eq!(TEARDOWN_CALLS.load(Ordering::SeqCst), 2);
}

#[test]
fn init_failure_cleanup_disposes_hostile_payload_and_runs_later_hooks() {
    let _guard = lock();
    reset();
    let first = descriptor("first", "neo:extension-first", ExtensionPermission::None);
    let hostile = ExtensionDescriptor::new("hostile", load)
        .with_module("neo:extension-hostile", ExtensionPermission::None)
        .with_init(init)
        .with_teardown(hostile_teardown);
    let failing = ExtensionDescriptor::new("failing", load)
        .with_module("neo:extension-failing", ExtensionPermission::None)
        .with_init(hostile_init);
    let options = RuntimeOptions::new()
        .with_extensions([first, hostile, failing])
        .unwrap();
    let error = match construct(options) {
        Ok(_) => panic!("runtime was constructed after a hostile init panic"),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains("non-string panic payload"),
        "unexpected error: {error}"
    );
    // One hostile payload from the failing init, one from the earlier
    // extension's teardown.
    assert_eq!(HOSTILE_DROPS.load(Ordering::SeqCst), 2);
    let order = ORDER.lock().unwrap();
    assert!(
        order.iter().any(|entry| entry == "teardown-hostile"),
        "{order:?}"
    );
    assert!(
        order
            .iter()
            .any(|entry| entry == "teardown:neo:extension-first"),
        "later teardown did not run after hostile cleanup: {order:?}"
    );
    assert_eq!(TEARDOWN_CALLS.load(Ordering::SeqCst), 2);
}

#[test]
fn teardown_panic_continues_remaining_teardowns_and_resumes_after_cleanup() {
    let _guard = lock();
    reset();
    let bad = ExtensionDescriptor::new("bad-teardown", load)
        .with_module("neo:extension-bad-teardown", ExtensionPermission::None)
        .with_init(init)
        .with_teardown(panic_teardown);
    let options = RuntimeOptions::new()
        .with_extensions([
            descriptor("good", "neo:extension-good", ExtensionPermission::None),
            bad,
        ])
        .unwrap();
    let runtime = NeoJs::new_with_options(Permissions::default(), options).unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(runtime)));
    assert!(result.is_err(), "teardown panic was not resumed");
    let order = ORDER.lock().unwrap();
    assert!(
        order
            .iter()
            .any(|entry| entry == "teardown-panic:neo:extension-bad-teardown"),
        "{order:?}"
    );
    assert!(
        order
            .iter()
            .any(|entry| entry == "teardown:neo:extension-good"),
        "later teardown did not run: {order:?}"
    );
    assert_eq!(TEARDOWN_CALLS.load(Ordering::SeqCst), 2);
}

#[test]
fn teardown_panic_during_prior_unwind_does_not_replace_the_original() {
    let _guard = lock();
    reset();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let bad = ExtensionDescriptor::new("bad-teardown", load)
            .with_module("neo:extension-bad-teardown", ExtensionPermission::None)
            .with_init(init)
            .with_teardown(panic_teardown);
        let options = RuntimeOptions::new()
            .with_extensions([
                descriptor("good", "neo:extension-good", ExtensionPermission::None),
                bad,
            ])
            .unwrap();
        let _runtime = NeoJs::new_with_options(Permissions::default(), options).unwrap();
        panic!("outer panic");
    }));
    let payload = result.expect_err("the outer panic was not caught");
    let message = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str));
    assert_eq!(message, Some("outer panic"));

    let order = ORDER.lock().unwrap();
    assert!(
        order
            .iter()
            .any(|entry| entry == "teardown-panic:neo:extension-bad-teardown"),
        "{order:?}"
    );
    assert!(
        order
            .iter()
            .any(|entry| entry == "teardown:neo:extension-good"),
        "later teardown did not run during unwind: {order:?}"
    );
    assert_eq!(TEARDOWN_CALLS.load(Ordering::SeqCst), 2);
}

#[test]
fn panicking_state_drop_is_contained_and_does_not_skip_later_teardowns() {
    let _guard = lock();
    reset();
    let bad = ExtensionDescriptor::new("bad-state", load)
        .with_module("neo:extension-bad-state", ExtensionPermission::None)
        .with_init(state_drop_init)
        .with_teardown(normal_teardown);
    let options = RuntimeOptions::new()
        .with_extensions([
            descriptor("good", "neo:extension-good", ExtensionPermission::None),
            bad,
        ])
        .unwrap();
    let runtime = NeoJs::new_with_options(Permissions::default(), options).unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(runtime)));
    assert!(result.is_err(), "state Drop panic was not resumed");
    let order = ORDER.lock().unwrap();
    assert!(
        order
            .iter()
            .any(|entry| entry == "teardown-normal:drop-panic-state"),
        "{order:?}"
    );
    assert!(
        order
            .iter()
            .any(|entry| entry == "teardown:neo:extension-good"),
        "later teardown did not run: {order:?}"
    );
    assert_eq!(TEARDOWN_CALLS.load(Ordering::SeqCst), 2);
}

#[test]
fn teardown_panic_during_init_failure_does_not_replace_the_init_error() {
    let _guard = lock();
    reset();
    let good = ExtensionDescriptor::new("good", load)
        .with_module("neo:extension-good", ExtensionPermission::None)
        .with_init(init)
        .with_teardown(panic_teardown);
    let bad = ExtensionDescriptor::new("bad-panic", load)
        .with_module("neo:extension-bad-panic", ExtensionPermission::None)
        .with_init(panic_init)
        .with_teardown(teardown);
    let options = RuntimeOptions::new().with_extensions([good, bad]).unwrap();
    let error = match construct(options) {
        Ok(_) => panic!("runtime was constructed after a panicking init"),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains("probe init panic"),
        "unexpected error: {error}"
    );
    assert_eq!(
        *ORDER.lock().unwrap(),
        [
            "init:neo:extension-good",
            "init-panic:neo:extension-bad-panic",
            "teardown-panic:neo:extension-good",
        ]
    );
    assert_eq!(TEARDOWN_CALLS.load(Ordering::SeqCst), 1);
}

#[test]
fn init_error_captures_the_js_exception_message() {
    let _guard = lock();
    reset();
    let bad = ExtensionDescriptor::new("bad-js", load)
        .with_module("neo:extension-bad-js", ExtensionPermission::None)
        .with_init(js_throwing_init)
        .with_teardown(teardown);
    let error = match construct(RuntimeOptions::new().with_extensions([bad]).unwrap()) {
        Ok(_) => panic!("runtime was constructed after a throwing init"),
        Err(error) => error,
    };
    let message = error.to_string();
    assert!(
        message.contains("probe js init failure"),
        "message was not captured: {message}"
    );
    assert!(
        message.contains("bad-js"),
        "message did not identify the extension: {message}"
    );
}

#[test]
fn init_exception_capture_does_not_invoke_message_getter() {
    let _guard = lock();
    reset();
    let bad = ExtensionDescriptor::new("bad-getter", load)
        .with_module("neo:extension-bad-getter", ExtensionPermission::None)
        .with_init(js_getter_init)
        .with_teardown(teardown);
    let error = match construct(RuntimeOptions::new().with_extensions([bad]).unwrap()) {
        Ok(_) => panic!("runtime was constructed after a throwing init"),
        Err(error) => error,
    };
    assert_eq!(
        GETTER_CALLS.load(Ordering::SeqCst),
        0,
        "capture invoked the message getter"
    );
    let message = error.to_string();
    assert!(
        !message.contains("getter invoked"),
        "getter output leaked into the diagnostic: {message}"
    );
    assert!(
        message.contains("JavaScript exception"),
        "generic diagnostic missing: {message}"
    );
}

#[test]
fn allowlist_naming_an_unregistered_extension_is_rejected_at_construction() {
    let _guard = lock();
    let registry = ExtensionRegistry::from_descriptors([descriptor(
        "probe",
        "neo:extension-probe",
        ExtensionPermission::None,
    )])
    .unwrap();
    let allowlist = ModuleAllowlist::from_names_with(["neo:extension-probe"], &registry).unwrap();
    let options = RuntimeOptions::with_allowlist(allowlist);
    let error = match construct(options) {
        Ok(_) => panic!("runtime accepted an unregistered allowlisted extension"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("allowlisted extension module is not registered"),
        "unexpected error: {error}"
    );
}

#[test]
fn descriptors_can_cross_threads_and_carry_no_js_values() {
    // Compile-time proof: the only public registration types are Send + Sync.
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<ExtensionDescriptor>();
    assert_send_sync::<ExtensionRegistry>();
    assert_send_sync::<ExtensionModule>();
    assert_send_sync::<ModuleAllowlist>();
    assert_send_sync::<RuntimeOptions>();

    let descriptor = descriptor("probe", "neo:extension-probe", ExtensionPermission::None);
    let id = std::thread::spawn(move || descriptor.id().to_owned())
        .join()
        .unwrap();
    assert_eq!(id, "probe");
}
