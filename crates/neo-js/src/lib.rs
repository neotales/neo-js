//! A small, configurable QuickJS host for automation.
//!
//! Permissions control host functions provided by this crate. They do not limit
//! CPU or memory consumption and must not be treated as an OS sandbox.
//!
//! # Capabilities and the module allowlist
//!
//! Two independent axes decide what a script can reach:
//!
//! * [`Permissions`] (capabilities) decide what a granted host function may do.
//!   `neo:fs` needs [`Permissions::filesystem`], `neo:env` needs
//!   [`Permissions::environment`], and so on. These are the historical flags.
//! * [`RuntimeOptions`] can carry an explicit [`ModuleAllowlist`] of the host
//!   `neo:` modules this runtime constructs. It is checked before permissions.
//!
//! [`NeoJs::new`] uses unrestricted options, exactly as before: every module
//! this build provides is available when its permission is granted. Use
//! [`NeoJs::new_with_options`] with [`RuntimeOptions::modules`] (or
//! [`RuntimeOptions::with_allowlist`]) to opt into a fail-closed set. An
//! explicit allowlist denies every `neo:` import it omits, including dynamic
//! imports and imports of an already-cached module, and withholds the globals
//! and native callbacks that only those modules use. A module that is allowed
//! still needs its permission; permission alone never overrides the allowlist.
//!
//! Embedders can add static optional modules through
//! [`ExtensionDescriptor`] and [`RuntimeOptions::with_extensions`]. An
//! extension declares exact `neo:` names and selects one of the existing
//! [`Permissions`] gates; core still owns resolution, the module cache,
//! initialization, and teardown. See the
//! [extension module documentation](extension) for the hook contract and a
//! complete native-module example.
//!
//! Optional modules are not compiled into this crate. SQLite is supplied by
//! the separate `neojs-sqlite` package, which registers the exact
//! `neo:sqlite` name and keeps `rusqlite` out of this crate's dependency graph.
//! [`Permissions::sqlite`] remains the gate, but it grants nothing unless that
//! extension is registered. Operating-system metadata is supplied by
//! `neojs-os`, which registers `neo:os`, installs the `os` global, and reads
//! the bounded host metadata. The pure scheduler text generators are supplied
//! by `neojs-schedulers`, which registers the exact
//! `neo:os/schedulers/systemd`, `neo:os/schedulers/windows`, and deprecated
//! `neo:scheduler` names; none needs a permission flag or a global. Core itself
//! no longer adds any of them, so a core-only runtime has no `neo:os` import,
//! no `os` global, and no scheduler import.
//!
//! The allowlist is a host-capability boundary, not an OS sandbox. It does not
//! limit CPU, memory, or the JavaScript language, and a script still shares one
//! realm with everything the host left in it. `console` is present in every
//! runtime: it is the existing output channel rather than an importable module,
//! so `console.log` and `console.error` always reach the host's standard output
//! and standard error. Entry scripts still execute under an empty allowlist;
//! only access to the omitted host modules changes.

/// Third-party copyright and permission notices embedded in this crate.
///
/// This is the complete text of `crates/neo-js/src/js/path.LICENSE.md` at build
/// time: provenance for the path algorithms adapted from `@neotales/path`, the
/// adapted-file list, and the upstream MIT copyright and permission notices. It
/// covers adapted source code, not every dependency of neo-js, and is not a
/// complete dependency-license inventory.
///
/// Reading the text performs no filesystem access and needs no permission. The
/// CLI prints the same text for `neo-js --third-party-notices`.
pub fn third_party_notices() -> &'static str {
    include_str!("js/path.LICENSE.md")
}

mod age;
mod crypto;
mod cwd;
mod dotenv;
pub mod extension;
mod fetch;
mod fs;
mod hash;
mod js_text;
mod loader;
mod module_turn;
mod modules;
mod path;
mod process;
mod semver;
mod stdio;
mod sync_callback;
mod toml;
mod typescript;
mod unwind;
mod winregistry;
mod yaml;

pub use extension::{
    ExtensionDescriptor, ExtensionInit, ExtensionLoad, ExtensionModule, ExtensionPermission,
    ExtensionRegistrationError, ExtensionRegistry, ExtensionTeardown,
};
pub use modules::{ModuleAllowlist, RuntimeModule, RuntimeOptions, UnknownModuleNames};

/// The QuickJS binding used by extension hooks.
///
/// Native extensions should use these types, not a separately resolved
/// `rquickjs`, so `Ctx`, `Module`, and `Persistent` are the same types as the
/// runtime's. This re-export follows neo-js's own rquickjs dependency.
pub use rquickjs;

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
};

use rquickjs::{Context, Function, Module, Object, Persistent, Runtime, function::Rest};

/// Host capabilities made available to JavaScript.
///
/// A permission is necessary but not sufficient: a host module must also be
/// admitted by the runtime's [`ModuleAllowlist`] when one is configured.
/// [`NeoJs::new`] imposes no allowlist, so these flags alone preserve the
/// historical behavior. See the crate documentation, "Capabilities and the
/// module allowlist".
///
/// This type is not `#[non_exhaustive]`. Before 1.0, adding a field such as
/// `Permissions::process`, `Permissions::stdio`, `Permissions::registry`,
/// `Permissions::network`, or `Permissions::sqlite` is a source-breaking
/// change for complete struct literals. Construct values with
/// `..Permissions::default()` or name every field explicitly.
#[derive(Clone, Copy, Debug, Default)]
pub struct Permissions {
    /// Allow filesystem access through `neo:fs`, plus file reads
    /// by `neo:hash.digestFile` and file writes by `neo:fetch.download`.
    ///
    /// `neo:fs` provides bounded binary and UTF-8 reads and writes, metadata,
    /// directory listing, directory creation, copy, rename, and deletion. It is not a
    /// sandbox: a script gets the host user's filesystem authority. Reads,
    /// writes, paths, and listings are bounded, and failures carry stable
    /// `FsError` codes. The module and its `fs` global are also gated by the
    /// runtime's [`ModuleAllowlist`]. When `neo:fs` is admitted, this also
    /// lets `neo:path` use and disclose the runtime working-directory text.
    pub filesystem: bool,
    /// Allow reading environment variables.
    pub environment: bool,
    /// Allow reading the host process's standard input through `neo:process`.
    ///
    /// A script that reads stdin consumes input the embedder or an interactive
    /// user may be using, so this is an explicit opt-in. It also makes
    /// `neo:process` importable for its stdout/stderr writers and the runtime
    /// working directory. When that module is admitted, `neo:path` can also
    /// use and disclose the runtime working-directory text.
    /// [`Self::process`] grants the same stdin access
    /// because it already permits arbitrary child programs with the host
    /// user's privileges; `stdio` is the narrower way to allow stdin alone.
    /// Standard output and standard error writes are already reachable through
    /// `console`, but either flag lets a script write arbitrary bytes without
    /// adding a new tier for those streams.
    ///
    /// This is not a sandbox. Reads are synchronous and can block until input
    /// or end of file; there is no timeout or nonblocking mode.
    pub stdio: bool,
    /// Allow synchronous execution of arbitrary programs with the host user's
    /// privileges.
    ///
    /// This grants the child the same filesystem, network, and environment
    /// access as the host process. It is not constrained by the other
    /// permission fields here. It also makes `neo:process` importable and
    /// permits reading host stdin through that module. When `neo:process` is
    /// admitted, `neo:path` can also use and disclose the runtime cwd text.
    /// `neo:exec` exposes both
    /// the one-shot `exec` call and owned `spawn` handles; both are synchronous
    /// and both use the same process-wide slot limit.
    pub process: bool,
    /// Allow synchronous read, write, and delete access to the Windows
    /// Registry.
    ///
    /// This is full authority under the current process token, not a sandbox:
    /// a script can read, create, change, and delete any key or value the
    /// current user can, including keys that run programs at sign-in. Grant it
    /// only to scripts you trust. On a non-Windows host the module is present
    /// with this permission but every operation reports that it is unsupported.
    pub registry: bool,
    /// Allow synchronous HTTP and HTTPS downloads through `neo:fetch`.
    ///
    /// This grants broad outbound network authority under the current user's
    /// network access, including loopback, link-local, private, and metadata
    /// addresses. It is not a sandbox, allowlist, or SSRF protection, and a
    /// redirect may change the host. Downloading a file also requires
    /// [`Self::filesystem`]; without it `neo:fetch.download` reports
    /// `FETCH_PERMISSION` before any file or network activity.
    pub network: bool,
    /// Allow synchronous, private in-memory databases through `neo:sqlite`.
    ///
    /// This gates the optional `neo:sqlite` module; core itself does not link
    /// SQLite. The module is present only when a host registers the
    /// `neojs-sqlite` extension, and importing it still requires admission by
    /// an explicit [`ModuleAllowlist`] when one is configured. It opts into the
    /// host's native SQLite engine, not a sandbox. Use only trusted SQL: there
    /// is no cap on SQLite's intermediate heap. It grants no file-backed
    /// database, filesystem, shell or network API.
    pub sqlite: bool,
}

/// An isolated QuickJS runtime and its single context.
pub struct NeoJs {
    context: Context,
    filesystem: bool,
    formats: Rc<RefCell<loader::ModuleFormats>>,
    // Test bookkeeping is host-owned. The runner is a persistent reference
    // installed by `neo:test`; the host never discovers a script global.
    tests: Rc<RefCell<TestState>>,
    // `neo:crypto` keeps its wrapper factory and callbacks private to the
    // module. Nothing is installed on the global object.
    crypto: Rc<RefCell<crypto::CryptoState>>,
    // `neo:age` uses the same private-factory pattern.
    age: Rc<RefCell<age::AgeState>>,
    // `neo:dotenv` is also private: no parse callback is placed on the global
    // object.
    dotenv: Rc<RefCell<dotenv::DotenvState>>,
    // `neo:toml` and `neo:yaml` follow the same private-factory pattern. Their
    // callbacks parse text only and need no permission.
    toml: Rc<RefCell<toml::TomlState>>,
    yaml: Rc<RefCell<yaml::YamlState>>,
    // `neo:semver` keeps every native callback private to its module. It needs
    // no permission and installs nothing on the global object.
    semver: Rc<RefCell<semver::SemverState>>,
    // `neo:winregistry` also keeps its wrapper private. It has no live native
    // state because each call opens and closes its own key handle.
    winregistry: Rc<RefCell<winregistry::WinRegistryState>>,
    // `neo:fetch` keeps its host callback private. The state records whether
    // filesystem permission was granted so the callback can reject before I/O.
    fetch: Rc<RefCell<fetch::FetchState>>,
    // `neo:fs` keeps its native callbacks private. The state holds the pristine
    // export object built in the fresh realm before user code runs.
    fs: Rc<RefCell<fs::FsState>>,
    reads: fs::ReadDriver,
    module_turns: module_turn::ModuleTurns,
    // Static descriptors and per-runtime extension state. The registry holds no
    // JS values; the state is created and dropped on this runtime's thread.
    extension_registry: Rc<extension::ExtensionRegistry>,
    extensions: Rc<RefCell<extension::ExtensionRuntime>>,
    #[cfg(test)]
    before_drive: RefCell<Option<Box<dyn FnOnce()>>>,
    path: Rc<RefCell<path::PathState>>,
    // The runtime-local working directory. It is never installed on the host
    // process and is shared only with this runtime's host callbacks.
    cwd: Rc<RefCell<cwd::WorkingDirectory>>,
    // `neo:process` keeps its stream and directory callbacks private.
    stdio: Rc<RefCell<stdio::ProcessState>>,
    // `neo:exec` spawned-child supervisors, owned by this runtime. Dropping
    // this state requests abort of outstanding handles; cleanup may remain pending.
    exec: Rc<RefCell<process::ExecState>>,
    // The context must be dropped before its runtime.
    _runtime: Runtime,
}

#[derive(Default)]
struct TestState {
    assert_api: Option<Persistent<Object<'static>>>,
    factory: Option<Persistent<Function<'static>>>,
    runner: Option<Persistent<Function<'static>>>,
    total: usize,
    passed: usize,
    failed: usize,
    failures: Vec<String>,
}

impl Drop for NeoJs {
    fn drop(&mut self) {
        self.module_turns.clear(&self._runtime);
        // Extension teardown runs before core persistent values are released,
        // and in reverse initialization order. Teardown receives no context and
        // cannot execute JavaScript from Drop. Each hook and state drop is
        // unwind-contained.
        let teardown_panic = self
            .extensions
            .try_borrow_mut()
            .ok()
            .and_then(|mut extensions| extensions.teardown_all(&self.extension_registry));
        // Release persistent values while the runtime is still alive.
        // Host closures may keep the surrounding `TestState` alive, but it no
        // longer holds a JS value by the time the runtime is dropped.
        if let Ok(mut state) = self.tests.try_borrow_mut() {
            state.assert_api = None;
            state.runner = None;
            state.factory = None;
        }
        if let Ok(mut state) = self.crypto.try_borrow_mut() {
            state.factory = None;
        }
        if let Ok(mut state) = self.age.try_borrow_mut() {
            state.factory = None;
        }
        if let Ok(mut state) = self.dotenv.try_borrow_mut() {
            state.factory = None;
        }
        if let Ok(mut state) = self.toml.try_borrow_mut() {
            state.factory = None;
        }
        if let Ok(mut state) = self.yaml.try_borrow_mut() {
            state.factory = None;
        }
        if let Ok(mut state) = self.semver.try_borrow_mut() {
            state.factory = None;
        }
        if let Ok(mut state) = self.winregistry.try_borrow_mut() {
            state.factory = None;
        }
        if let Ok(mut state) = self.fetch.try_borrow_mut() {
            state.factory = None;
        }
        if let Ok(mut state) = self.fs.try_borrow_mut() {
            state.factory = None;
            state.pristine = None;
        }
        if let Ok(mut state) = self.path.try_borrow_mut() {
            state.api = None;
        }
        if let Ok(mut state) = self.stdio.try_borrow_mut() {
            state.factory = None;
        }
        // Request termination of direct children still owned by outstanding handles
        // before the context and runtime disappear.
        process::abort_all(&self.exec);
        // Every remaining teardown and core cleanup has run. A normal drop
        // resumes the first teardown panic; a drop already unwinding another
        // panic contains it so the original unwind is not turned into an abort.
        if let Some(payload) = teardown_panic {
            if std::thread::panicking() {
                payload.discard();
            } else {
                payload.resume();
            }
        }
    }
}

/// Counts from tests registered through `neo:test`.
///
/// The counts are cumulative for the runtime: a test executed once by an
/// explicit `run()` is not executed again by the host, but its failure still
/// appears here. `failures` contains one `name: error` line per failed test, in
/// execution order.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TestReport {
    /// Tests executed so far.
    pub total: usize,
    /// Tests that completed without throwing or returning a Promise or
    /// generator object.
    pub passed: usize,
    /// Tests that threw, or returned a Promise instead of running synchronously.
    pub failed: usize,
    /// One summary line per failed test.
    pub failures: Vec<String>,
}

impl TestReport {
    /// True when no executed test has failed.
    pub fn all_passed(&self) -> bool {
        self.failed == 0
    }
}

// TestState contains no borrowed JS values; its persistent handle is explicitly
// released before the context and runtime are dropped.
unsafe impl<'js> rquickjs::JsLifetime<'js> for TestState {
    type Changed<'to> = TestState;
}

impl NeoJs {
    /// Create a runtime with the given host permissions and no module
    /// allowlist.
    ///
    /// This is the backward-compatible constructor: every module this build
    /// provides remains importable when its permission is granted. Use
    /// [`Self::new_with_options`] to restrict the host modules available.
    pub fn new(permissions: Permissions) -> rquickjs::Result<Self> {
        Self::new_with_options(permissions, RuntimeOptions::default())
    }

    /// Create a runtime with the given permissions and module options.
    ///
    /// An explicit allowlist in `options` denies every `neo:` module it omits,
    /// including dynamic imports and cache hits, and does not install the
    /// globals or native callbacks that only those modules use. A module that
    /// the allowlist admits still needs its [`Permissions`] flag. Entry-script
    /// evaluation and ordinary relative JavaScript imports are unaffected, and
    /// `console` remains available.
    ///
    /// The module allowlist does not confine permission effects to the same-named
    /// module. `neo:hash.digestFile` reads files with filesystem permission even
    /// when `neo:fs` is omitted. `neo:fetch.download` writes files when filesystem
    /// and network permissions are granted, also without `neo:fs`. Filesystem
    /// permission enables relative module imports independently of the `neo:fs`
    /// selection. Process permission lets `neo:exec` launch programs with the
    /// host user's authority, regardless of the other permission flags and module
    /// selections. This runtime is not an OS sandbox.
    ///
    /// Registered extensions are initialized once, in registration order,
    /// before this method returns. A module is initialized only when it is
    /// admitted by the allowlist and its permission is granted. An init error
    /// or panic tears down extensions initialized earlier and returns an owned
    /// error; a pending JavaScript exception is captured without invoking
    /// getters. Teardown runs on drop without a `Ctx` and its panics are
    /// contained. See [`crate::extension`] for the full contract.
    pub fn new_with_options(
        permissions: Permissions,
        options: RuntimeOptions,
    ) -> rquickjs::Result<Self> {
        // An allowlist may name an extension module only when the same options
        // registered that extension. Check before constructing the runtime so a
        // mismatched configuration fails closed with a clear error.
        if let Some(allowlist) = options.allowlist() {
            for name in allowlist.extension_names() {
                if !options.extensions().contains_module(name) {
                    return Err(rquickjs::Error::new_loading_message(
                        name,
                        "allowlisted extension module is not registered",
                    ));
                }
            }
        }
        let allowed = options.allowed_modules();
        let extension_registry = Rc::new(options.extensions().clone());
        let extension_runtime = Rc::new(RefCell::new(extension::ExtensionRuntime::new()));
        let runtime = Runtime::new()?;
        let module_turns = module_turn::ModuleTurns::new(&runtime)?;
        let reads = fs::ReadDriver::default();
        let formats = Rc::new(RefCell::new(loader::ModuleFormats::default()));
        let cwd = Rc::new(RefCell::new(cwd::WorkingDirectory::new()));
        runtime.set_loader(
            loader::ModuleLoader {
                permissions,
                formats: Rc::clone(&formats),
                allowed: allowed.clone(),
                extensions: Rc::clone(&extension_registry),
                extension_runtime: Rc::clone(&extension_runtime),
            },
            loader::ModuleLoader {
                permissions,
                formats: Rc::clone(&formats),
                allowed: allowed.clone(),
                extensions: Rc::clone(&extension_registry),
                extension_runtime: Rc::clone(&extension_runtime),
            },
        );
        let context = Context::full(&runtime)?;
        let tests = Rc::new(RefCell::new(TestState::default()));
        let crypto = Rc::new(RefCell::new(crypto::CryptoState::default()));
        let age = Rc::new(RefCell::new(age::AgeState::default()));
        let dotenv = Rc::new(RefCell::new(dotenv::DotenvState::default()));
        let toml = Rc::new(RefCell::new(toml::TomlState::default()));
        let yaml = Rc::new(RefCell::new(yaml::YamlState::default()));
        let semver = Rc::new(RefCell::new(semver::SemverState::default()));
        let winregistry = Rc::new(RefCell::new(winregistry::WinRegistryState::default()));
        let fetch = Rc::new(RefCell::new(fetch::FetchState::new(
            permissions.filesystem,
            Rc::clone(&cwd),
        )));
        let fs = Rc::new(RefCell::new(fs::FsState::new(Rc::clone(&cwd))));
        let path = Rc::new(RefCell::new(path::PathState::default()));
        let stdio = Rc::new(RefCell::new(stdio::ProcessState::new(Rc::clone(&cwd))));
        let exec = Rc::new(RefCell::new(process::ExecState::new()));
        context.with(|ctx| -> rquickjs::Result<()> {
            let globals = ctx.globals();
            let console = Object::new(ctx.clone())?;
            console.set(
                "log",
                Function::new(ctx.clone(), |args: Rest<String>| {
                    println!("{}", args.0.join(" "));
                })?,
            )?;
            console.set(
                "error",
                Function::new(ctx.clone(), |args: Rest<String>| {
                    eprintln!("{}", args.0.join(" "));
                })?,
            )?;
            globals.set("console", console)?;

            if permissions.process && allowed.allows(RuntimeModule::Exec) {
                let exec_cwd = Rc::clone(&cwd);
                globals.set(
                    "__neoExec",
                    Function::new(
                        ctx.clone(),
                        move |program: String, args: Vec<String>, input: Vec<u8>| {
                            let directory = exec_cwd.borrow().current_path();
                            process::exec(&program, &args, input, directory.as_deref())
                        },
                    )?,
                )?;
                let spawn_cwd = Rc::clone(&cwd);
                let spawn_state = Rc::clone(&exec);
                globals.set(
                    "__neoExecSpawn",
                    Function::new(ctx.clone(), move |program: String, args: Vec<String>| {
                        let directory = spawn_cwd.borrow().current_path();
                        process::spawn(&spawn_state, &program, &args, directory.as_deref())
                    })?,
                )?;
                let call_state = Rc::clone(&exec);
                globals.set(
                    "__neoExecCall",
                    Function::new(
                        ctx.clone(),
                        move |id: u64, action: String, input: Vec<u8>| {
                            process::call(&call_state, id, &action, input)
                        },
                    )?,
                )?;
            }

            if allowed.allows(RuntimeModule::Hash) {
                globals.set(
                    "__neoHash",
                    Function::new(ctx.clone(), |algorithm: String, data: Vec<u8>| {
                        hash::hash_bytes(&algorithm, &data)
                            .map_err(|e| rquickjs::Error::new_from_js_message("input", "hash", e))
                    })?,
                )?;
            }

            if permissions.environment && allowed.allows(RuntimeModule::Env) {
                let env = Object::new(ctx.clone())?;
                // Absence and non-Unicode values return `undefined`, matching
                // the existing runtime contract documented in the types.
                env.set(
                    "get",
                    Function::new(ctx.clone(), |key: String| std::env::var(key).ok())?,
                )?;
                env.set(
                    "has",
                    Function::new(ctx.clone(), |key: String| std::env::var_os(key).is_some())?,
                )?;
                globals.set("env", env)?;
            }
            // `__neoHashFile` belongs to `neo:hash`: `digestFile` is part of
            // that module and requires filesystem permission. It is not part
            // of `neo:fs`, so admitting `neo:hash` without `neo:fs` still
            // allows the file digest while leaving the `fs` global absent.
            if permissions.filesystem && allowed.allows(RuntimeModule::Hash) {
                let hash_cwd = Rc::clone(&cwd);
                globals.set(
                    "__neoHashFile",
                    Function::new(ctx.clone(), move |algorithm: String, path: String| {
                        let path = hash_cwd.borrow().resolve(&path).map_err(|error| {
                            rquickjs::Error::new_from_js_message("file", "hash", error.message)
                        })?;
                        let path = path.to_str().ok_or_else(|| {
                            rquickjs::Error::new_from_js_message(
                                "file",
                                "hash",
                                "path is not valid UTF-8",
                            )
                        })?;
                        hash::hash_file(&algorithm, path)
                            .map_err(|e| rquickjs::Error::new_from_js_message("file", "hash", e))
                    })?,
                )?;
            }
            if permissions.filesystem && allowed.allows(RuntimeModule::Fs) {
                ctx.store_userdata(Rc::clone(&fs))
                    .map_err(|_| rquickjs::Error::Unknown)?;
                let fs_factory = fs::factory(&ctx)?;
                fs.borrow_mut().factory = Some(Persistent::save(&ctx, fs_factory));
                let fs_api = fs::install(&ctx, &fs, reads.clone())?;
                globals.set("fs", fs_api)?;
            }
            // Both admitted modules share a private callback guard created
            // before user code can change the realm. Only `neo:test` can
            // install the test runner.
            if allowed.allows(RuntimeModule::Test) || allowed.allows(RuntimeModule::Assert) {
                ctx.store_userdata(Rc::clone(&tests))
                    .map_err(|_| rquickjs::Error::Unknown)?;
                let guard = sync_callback::guard(&ctx)?;
                if allowed.allows(RuntimeModule::Test) {
                    let factory = loader::test_factory(&ctx, &guard)?;
                    tests.borrow_mut().factory = Some(Persistent::save(&ctx, factory));
                }
                if allowed.allows(RuntimeModule::Assert) {
                    let api = loader::assert_api(&ctx, &guard)?;
                    tests.borrow_mut().assert_api = Some(Persistent::save(&ctx, api));
                }
            }

            // Build the `neo:crypto` wrapper in the fresh realm, before any
            // user code. Its host callbacks are passed privately when the
            // module is evaluated and are never placed on the global object.
            if allowed.allows(RuntimeModule::Crypto) {
                ctx.store_userdata(Rc::clone(&crypto))
                    .map_err(|_| rquickjs::Error::Unknown)?;
                let crypto_factory = crypto::factory(&ctx)?;
                crypto.borrow_mut().factory = Some(Persistent::save(&ctx, crypto_factory));
            }

            // `neo:age` follows the same private-callback pattern.
            if allowed.allows(RuntimeModule::Age) {
                ctx.store_userdata(Rc::clone(&age))
                    .map_err(|_| rquickjs::Error::Unknown)?;
                let age_factory = age::factory(&ctx)?;
                age.borrow_mut().factory = Some(Persistent::save(&ctx, age_factory));
            }

            // `neo:dotenv` follows the private-callback pattern too.
            if allowed.allows(RuntimeModule::Dotenv) {
                ctx.store_userdata(Rc::clone(&dotenv))
                    .map_err(|_| rquickjs::Error::Unknown)?;
                let dotenv_factory = dotenv::factory(&ctx)?;
                dotenv.borrow_mut().factory = Some(Persistent::save(&ctx, dotenv_factory));
            }

            // `neo:toml` and `neo:yaml` are pure text parsers. Their callbacks
            // stay private to their modules and need no permission.
            if allowed.allows(RuntimeModule::Toml) {
                ctx.store_userdata(Rc::clone(&toml))
                    .map_err(|_| rquickjs::Error::Unknown)?;
                let toml_factory = toml::factory(&ctx)?;
                toml.borrow_mut().factory = Some(Persistent::save(&ctx, toml_factory));
            }

            if allowed.allows(RuntimeModule::Yaml) {
                ctx.store_userdata(Rc::clone(&yaml))
                    .map_err(|_| rquickjs::Error::Unknown)?;
                let yaml_factory = yaml::factory(&ctx)?;
                yaml.borrow_mut().factory = Some(Persistent::save(&ctx, yaml_factory));
            }

            // `neo:semver` is a pure text and arithmetic module. Its native
            // callbacks are passed privately to the factory and never become
            // globals, so no `__neoSemver*` name can be replaced or called.
            if allowed.allows(RuntimeModule::Semver) {
                ctx.store_userdata(Rc::clone(&semver))
                    .map_err(|_| rquickjs::Error::Unknown)?;
                let semver_factory = semver::factory(&ctx)?;
                semver.borrow_mut().factory = Some(Persistent::save(&ctx, semver_factory));
            }

            // `neo:winregistry` is built the same way, before user code can
            // replace JavaScript constructors. The factory is only reachable
            // through the allowlisted and permission-gated module.
            if permissions.registry && allowed.allows(RuntimeModule::WinRegistry) {
                ctx.store_userdata(Rc::clone(&winregistry))
                    .map_err(|_| rquickjs::Error::Unknown)?;
                let winregistry_factory = winregistry::factory(&ctx)?;
                winregistry.borrow_mut().factory =
                    Some(Persistent::save(&ctx, winregistry_factory));
            }

            // `neo:fetch` uses the same private-factory pattern. It is only
            // importable with network permission, and its callback checks
            // filesystem permission before any I/O.
            if permissions.network && allowed.allows(RuntimeModule::Fetch) {
                ctx.store_userdata(Rc::clone(&fetch))
                    .map_err(|_| rquickjs::Error::Unknown)?;
                let fetch_factory = fetch::factory(&ctx)?;
                fetch.borrow_mut().factory = Some(Persistent::save(&ctx, fetch_factory));
            }

            // `neo:process` keeps every callback private to the module. It is
            // built only when both admitted and the runtime has process or
            // stdio authority; reading host stdin requires one of them as
            // well.
            if (permissions.process || permissions.stdio) && allowed.allows(RuntimeModule::Process)
            {
                ctx.store_userdata(Rc::clone(&stdio))
                    .map_err(|_| rquickjs::Error::Unknown)?;
                let stdio_factory = stdio::factory(&ctx)?;
                stdio.borrow_mut().factory = Some(Persistent::save(&ctx, stdio_factory));
            }
            if allowed.allows(RuntimeModule::Path) {
                let api = path::api(&ctx, permissions, &allowed, Rc::clone(&cwd))?;
                ctx.store_userdata(Rc::clone(&path))
                    .map_err(|_| rquickjs::Error::Unknown)?;
                path.borrow_mut().api = Some(Persistent::save(&ctx, api));
            }
            // Extension init runs after core state exists and before user code.
            // Keep this last in the fallible construction path: a successful
            // init is only assigned to the shared runtime below, and any later
            // error would leave that state without a teardown call.
            let initialized = extension::ExtensionRuntime::initialize(
                &ctx,
                &extension_registry,
                permissions,
                &allowed,
            )?;
            *extension_runtime.borrow_mut() = initialized;
            Ok(())
        })?;
        module_turns.install(&runtime);
        Ok(Self {
            context,
            filesystem: permissions.filesystem,
            formats,
            tests,
            crypto,
            age,
            dotenv,
            toml,
            yaml,
            semver,
            winregistry,
            fetch,
            fs,
            reads,
            module_turns,
            extension_registry,
            extensions: extension_runtime,
            #[cfg(test)]
            before_drive: RefCell::default(),
            path,
            cwd,
            stdio,
            exec,
            _runtime: runtime,
        })
    }

    /// Evaluate JavaScript as a script and return its string result.
    pub fn eval(&self, source: &str) -> Result<String, String> {
        self.module_turns.ensure_usable()?;
        self.context.with(|ctx| {
            ctx.eval::<String, _>(source).map_err(|error| {
                use rquickjs::CatchResultExt;
                Err::<(), _>(error).catch(&ctx).unwrap_err().to_string()
            })
        })
    }

    /// Run JavaScript as a script, discarding its result.
    pub fn run(&self, source: &str) -> Result<(), String> {
        self.module_turns.ensure_usable()?;
        self.context.with(|ctx| {
            ctx.eval::<(), _>(source).map_err(|error| {
                use rquickjs::CatchResultExt;
                Err::<(), _>(error).catch(&ctx).unwrap_err().to_string()
            })
        })
    }

    /// Capture the absolute directory used to resolve a module's relative
    /// imports. This happens when the module is declared, so a later runtime
    /// `chdir` cannot move an existing virtual module's base.
    fn module_base_dir(&self, identity: &str) -> Option<PathBuf> {
        let path = if Path::new(identity).is_absolute() {
            PathBuf::from(identity)
        } else {
            self.cwd.borrow().resolve(identity).ok()?
        };
        Some(path.parent().unwrap_or(Path::new(".")).to_path_buf())
    }

    /// Evaluate an ECMAScript module. The name is the base path for relative imports.
    ///
    /// Relative imports require filesystem permission. This blocking facade
    /// drives QuickJS jobs and filesystem read completions to quiescence.
    /// Earlier script or test execution must not leave pending Promises or
    /// jobs (`NEO_MODULE_DIRTY`). A live unresolved Promise with no remaining
    /// source of progress reports `NEO_MODULE_STALLED`, even after the entry
    /// itself has fulfilled. Stalls, driver step exhaustion, and errors with
    /// unfinished work abandon the runtime; create a new one to continue.
    ///
    /// With filesystem permission, an existing entry name and each relative
    /// dependency are canonicalized using the filesystem, including symlinks.
    /// Relative entry names are resolved against this runtime's working
    /// directory, not the host process working directory. If the runtime
    /// working directory is unavailable, a relative entry name is an error
    /// rather than a fallback to the host directory. If the entry name cannot
    /// be canonicalized it remains a virtual name; relative dependencies still
    /// require permission and must exist. The absolute directory used for those
    /// dependencies is captured when the entry is declared, so a later
    /// `neo:process.chdir` cannot move an existing module's base. An absolute
    /// entry name captures an absolute base and can import relative files even
    /// when the runtime working directory is unavailable. Without
    /// permission the entry name is used verbatim, no filesystem lookup is
    /// performed, and a virtual entry still runs as long as it imports no
    /// relative path. A successful canonicalization to a non-UTF-8 path is an
    /// error. The caller supplies the entry source in either case.
    ///
    /// The requested name selects the format independently of the canonical
    /// identity. Different requested extensions among `.js`, `.mjs`, `.ts`,
    /// and `.mts` for the same identity conflict for the runtime's lifetime,
    /// including on cache hits. Same-format aliases share one module.
    /// Dynamic import conflicts do not undo earlier side effects. Canonical
    /// paths do not unify hard links; Windows behavior is unverified.
    /// A failed load, parse, or resolution releases that module's reservation;
    /// a declared module retains it even if evaluation fails. A failed cyclic
    /// resolution disables further module loading and entry evaluation in this
    /// runtime; construct a new runtime to recover. Earlier effects remain.
    /// Entries whose identity is already declared or loading are rejected,
    /// and `neo:` names and identities are reserved for host modules. A host
    /// module import is checked against this runtime's [`RuntimeOptions`]
    /// allowlist as well as its permission; an omitted module cannot be
    /// reached by a static or dynamic import, including a cache hit.
    ///
    /// A `.ts` or `.mts` name is stripped before evaluation. Stripping erases
    /// type syntax and nothing else: there is no type checking, so an
    /// annotation never changes behaviour, and constructs whose JavaScript form
    /// is not an erasure are rejected with a `TS_UNSUPPORTED` error rather than
    /// compiled. That covers `enum` and `const enum`, `namespace` and `module`
    /// declarations with a body, constructor parameter properties, decorators,
    /// `import x = ...`, `export = ...`, and `export as namespace`. Type-only
    /// imports and specifiers are removed; an import with any value specifier
    /// still needs its target and preserves its side effects. Parser and
    /// early semantic errors are reported before any of the module runs. Output
    /// is regenerated from the parsed module, so formatting is not preserved and
    /// no source map is produced.
    ///
    /// `.cts`, `.tsx`, `.d.ts`, and `.d.mts` names are rejected with a
    /// `TS_EXTENSION` error. Any other name is evaluated as JavaScript, byte for
    /// byte, so a `.js` file containing type syntax still fails.
    pub fn run_module(&self, name: &str, source: &str) -> Result<(), String> {
        self.module_turns.ensure_usable()?;
        if name.starts_with("neo:") {
            return Err("neo: module identities are reserved".to_owned());
        }
        let identity = if self.filesystem {
            let candidate = self.cwd.borrow().resolve(name).map_err(|e| e.to_string())?;
            match std::fs::canonicalize(candidate) {
                Ok(path) => path
                    .to_str()
                    .ok_or("canonical entry path is not valid UTF-8")?
                    .to_owned(),
                Err(_) => name.to_owned(),
            }
        } else {
            name.to_owned()
        };
        self.run_module_with_identity(name, &identity, source)
    }

    /// Evaluate supplied source with a separate requested format name and
    /// host-provided identity/base. The caller must ensure the identity names
    /// the intended file and is consistently canonicalized; this method
    /// performs no filesystem lookup and does not grant permission. An absolute
    /// identity supplies its own base, so its relative imports can resolve even
    /// when the runtime working directory is unavailable. A relative identity
    /// uses this runtime's working directory at declaration time; if that
    /// directory is unavailable, the entry still runs but relative imports
    /// report the unavailable directory, including after a later `chdir`. (There is no declaration
    /// directory to capture, so the module has no base. Recover with a new
    /// runtime, or declare the source under a distinct identity after
    /// establishing a directory; an identity already declared as an entry
    /// cannot be reused.) A later `chdir` never moves a captured base.
    /// Requested extensions and identity conflicts follow [`Self::run_module`].
    pub fn run_module_with_identity(
        &self,
        name: &str,
        identity: &str,
        source: &str,
    ) -> Result<(), String> {
        self.module_turns.ensure_usable()?;
        if name.starts_with("neo:") || identity.starts_with("neo:") {
            return Err("neo: module identities are reserved".to_owned());
        }
        self.formats.borrow().ensure_usable()?;
        self.module_turns
            .require_idle(&self.context, &self._runtime)?;
        let stripped = typescript::strip_if_typescript(name, source)?;
        let source = stripped.as_deref().unwrap_or(source);
        let base = self.module_base_dir(identity);
        let token = self
            .formats
            .borrow_mut()
            .reserve_entry(identity, name, base)?;
        let turn = self.module_turns.begin();
        let reads = self.reads.begin();
        let entry = self.context.with(|ctx| {
            let declared = Module::declare(ctx.clone(), identity, source);
            self.formats
                .borrow_mut()
                .complete(identity, token, declared.is_ok());
            declared
                .and_then(|module| module.eval())
                .map(|(_, promise)| {
                    module_turn::observe_entry(&promise);
                    Persistent::save(&ctx, promise)
                })
                .map_err(|error| module_turn::js_error(&ctx, error))
        });
        let result = entry.and_then(|root| {
            #[cfg(test)]
            if let Some(before_drive) = self.before_drive.borrow_mut().take() {
                before_drive();
            }
            turn.drive(&self.context, &self._runtime, &reads, root)
        });
        turn.finish(&self.context, &self._runtime, &reads, result.is_err());
        result
    }

    /// Run every test registered through `neo:test` and return cumulative counts.
    ///
    /// This is synchronous: it runs the queued tests in registration order and
    /// returns after all of them have settled. A Promise or generator object
    /// returned by a test is recorded as a failure rather than awaited or
    /// iterated. The `neo:test` and `neo:assert` modules need no permissions and
    /// grant none.
    ///
    /// Returns `Ok(None)` when the script never imported `neo:test`, so a host
    /// can distinguish "no tests" from a reported run. The runner is a
    /// persistent reference installed by that module; unrelated script globals
    /// are not probed. Counts and failure summaries are recorded through
    /// host-owned callbacks, avoiding the tested prototype and `toJSON`
    /// reporting attacks. This is not protection against arbitrary shared-realm
    /// mutation. Test progress and failure text use `console.log` captured
    /// when the module initializes; an earlier replacement can suppress those
    /// details without changing the host report. This method itself does not
    /// print. Because the report is cumulative, a failure from an earlier
    /// explicit `run()` still appears even when no tests remain.
    ///
    /// This does not decide process status. The CLI exits non-zero when
    /// [`TestReport::all_passed`] is false. Returns an error when the runner
    /// raises an exception or its counts are inconsistent; inconsistent state
    /// is never treated as success. If module evaluation abandoned the
    /// runtime, returns `NEO_MODULE_ABORTED` without running registered tests
    /// or any other script code.
    pub fn run_pending_tests(&self) -> Result<Option<TestReport>, String> {
        self.module_turns.ensure_usable()?;
        let runner = self.tests.borrow().runner.as_ref().cloned();
        let Some(runner) = runner else {
            return Ok(None);
        };

        self.context.with(|ctx| {
            use rquickjs::CatchResultExt;
            runner
                .restore(&ctx)
                .and_then(|runner| runner.call::<_, ()>(()))
                .catch(&ctx)
                .map_err(|error| error.to_string())
        })?;

        let state = self.tests.borrow();
        let report = TestReport {
            total: state.total,
            passed: state.passed,
            failed: state.failed,
            failures: state.failures.clone(),
        };
        if report.passed.checked_add(report.failed) != Some(report.total) {
            return Err("neo:test reported counts that do not add up".to_owned());
        }
        if report.failures.len() != report.failed {
            return Err(format!(
                "neo:test recorded {} failure(s) but reports {} failed test(s)",
                report.failures.len(),
                report.failed
            ));
        }
        Ok(Some(report))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returned_promises_notify_the_rejection_tracker_without_draining_jobs() {
        // Only the engine exposes the rejection tracker, so use the runtime
        // directly to observe both already-rejected and pending cases.
        use std::cell::Cell;

        let js = NeoJs::new(Permissions::default()).unwrap();
        let events = Rc::new(Cell::new((0, 0)));
        let tracked = Rc::clone(&events);
        js._runtime
            .set_host_promise_rejection_tracker(Some(Box::new(move |_, _, _, handled| {
                let (mut rejected, mut resolved) = tracked.get();
                if handled {
                    resolved += 1;
                } else {
                    rejected += 1;
                }
                tracked.set((rejected, resolved));
            })));
        js.run_module(
            "test-rejections.js",
            r#"import test from 'neo:test';
               import {doesNotThrow, throws} from 'neo:assert';
               throws(() => doesNotThrow(() => Promise.reject('assert rejection')), TypeError);
               test('already rejected', () => {
                   const promise = Promise.reject('test rejection');
                   return Object.setPrototypeOf(promise, null);
               });
               test('rejects later', () => Promise.resolve().then(() => {
                   globalThis.testJobRan = true;
                   throw Error('later rejection');
               }));"#,
        )
        .unwrap();
        assert_eq!(events.get(), (1, 1));
        let report = js.run_pending_tests().unwrap().unwrap();
        assert_eq!((report.total, report.passed, report.failed), (2, 0, 2));
        assert_eq!(events.get(), (2, 2));
        js.context.with(|ctx| {
            let job_ran = "globalThis.testJobRan === true";
            assert!(!ctx.eval::<bool, _>(job_ran).unwrap());
            assert!(ctx.execute_pending_job());
            assert!(ctx.eval::<bool, _>(job_ran).unwrap());
        });
        assert_eq!(events.get(), (2, 2));
    }

    #[test]
    fn capabilities_are_disabled_by_default() {
        let js = NeoJs::new(Permissions::default()).unwrap();
        assert_eq!(
            js.eval("typeof fs + ',' + typeof env + ',' + typeof __neoExec")
                .unwrap(),
            "undefined,undefined,undefined"
        );
        // Core no longer installs the optional `os` global. A host that
        // registers `neojs_os::os_extension` gets it back.
        assert_eq!(js.eval("typeof os").unwrap(), "undefined");
    }

    #[test]
    fn permissions_enable_only_requested_capabilities() {
        let js = NeoJs::new(Permissions {
            filesystem: false,
            environment: true,
            ..Permissions::default()
        })
        .unwrap();
        assert_eq!(
            js.eval("typeof fs + ',' + typeof env.get").unwrap(),
            "undefined,function"
        );
    }

    #[test]
    fn process_permission_exposes_the_exec_module() {
        let denied = NeoJs::new(Permissions::default()).unwrap();
        assert!(
            denied
                .run_module("denied-exec.js", "import {exec} from 'neo:exec';")
                .is_err()
        );

        let allowed = NeoJs::new(Permissions {
            process: true,
            ..Permissions::default()
        })
        .unwrap();
        allowed
            .run_module(
                "allowed-exec.js",
                "import {exec, spawn} from 'neo:exec'; \
                 if (typeof exec !== 'function') throw Error('missing exec'); \
                 if (typeof spawn !== 'function') throw Error('missing spawn');",
            )
            .unwrap();
    }

    #[test]
    fn registry_permission_is_required_for_the_module() {
        let denied = NeoJs::new(Permissions::default()).unwrap();
        assert!(
            denied
                .run_module("denied-registry.js", "import 'neo:winregistry';")
                .is_err()
        );
        // The module has no public global, so denying permission denies every
        // entry point, not just the import.
        assert_eq!(
            denied.eval("typeof globalThis.__neoWinRegistry").unwrap(),
            "undefined"
        );

        let allowed = NeoJs::new(Permissions {
            registry: true,
            ..Permissions::default()
        })
        .unwrap();
        allowed
            .run_module(
                "allowed-registry.js",
                "import {isAvailable, ValueType, HIVES} from 'neo:winregistry';
                 if (typeof isAvailable !== 'function') throw Error('missing isAvailable');
                 if (ValueType.DWORD !== 4 || ValueType.QWORD !== 11) throw Error('types');
                 if (HIVES.length !== 5 || HIVES[0] !== 'HKCR') throw Error('hives');",
            )
            .unwrap();
    }

    #[test]
    fn exceptions_reach_the_host() {
        let js = NeoJs::new(Permissions::default()).unwrap();
        assert!(
            js.run("throw new Error('test failure')")
                .unwrap_err()
                .contains("test failure")
        );
    }

    #[test]
    fn filesystem_permission_allows_reading_and_propagates_errors() {
        let js = NeoJs::new(Permissions {
            filesystem: true,
            environment: false,
            ..Permissions::default()
        })
        .unwrap();
        assert_eq!(
            js.eval("fs.readText('Cargo.toml').includes('neojs-core') ? 'yes' : 'no'")
                .unwrap(),
            "yes"
        );
        assert!(
            js.run("fs.readText('nonexistent-neo-js-test-file')")
                .is_err()
        );
    }

    #[test]
    fn builtin_modules_obey_permissions_and_semver_precedence() {
        let js = NeoJs::new(Permissions::default()).unwrap();
        js.run_module(
            "main.js",
            "import {valid, compare} from 'neo:semver';
             if (!valid('1.2.3') || valid('01.2.3') ||
                 compare('1.0.0-alpha', '1.0.0') !== -1 ||
                 compare('1.0.0+one', '1.0.0+two') !== 0) throw Error('semver');",
        )
        .unwrap();
        assert!(js.run_module("denied.js", "import 'neo:fs'").is_err());
        assert!(js.run_module("denied-env.js", "import 'neo:env'").is_err());
        assert!(js.run_module("unknown.js", "import 'neo:unknown'").is_err());
    }

    #[test]
    fn relative_imports_require_filesystem_permission() {
        let dir = tempfile::tempdir().unwrap();
        let entry = dir.path().join("main.js");
        std::fs::write(dir.path().join("dep.js"), "export const value = 42;").unwrap();
        let source = "import {value} from './dep.js'; if (value !== 42) throw Error('bad');";
        let name = entry.to_str().unwrap();
        let denied = NeoJs::new(Permissions::default()).unwrap();
        assert!(denied.run_module(name, source).is_err());
        let allowed = NeoJs::new(Permissions {
            filesystem: true,
            environment: false,
            ..Permissions::default()
        })
        .unwrap();
        allowed.run_module(name, source).unwrap();
        allowed
            .run_module(
                "builtin-test.js",
                "import {readText} from 'neo:fs'; if (typeof readText !== 'function') throw Error();",
            )
            .unwrap();
    }

    #[test]
    fn module_exceptions_reach_the_host() {
        let js = NeoJs::new(Permissions::default()).unwrap();
        assert!(
            js.run_module("main.js", "throw new Error('module failure')")
                .unwrap_err()
                .contains("module failure")
        );
    }

    #[test]
    fn hash_module_handles_byte_views_and_rejects_bad_inputs() {
        let js = NeoJs::new(Permissions::default()).unwrap();
        js.run_module(
            "hash-test.js",
            "import {digest} from 'neo:hash';
             const bytes = new Uint8Array([0, 97, 98, 99, 0]).subarray(1, 4);
             if (digest('sha256', bytes) !== 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad') throw Error('digest');
             let rejected = false;
             try { digest('sha256', 'abc'); } catch (_) { rejected = true; }
             if (!rejected) throw Error('expected type error');",
        )
        .unwrap();
        assert!(
            js.run_module(
                "unsupported.js",
                "import {digest} from 'neo:hash'; digest('md5', new Uint8Array());"
            )
            .is_err()
        );
        assert!(
            js.run_module("no-file.js", "import {digestFile} from 'neo:hash';")
                .is_err()
        );
    }

    #[test]
    fn hash_file_export_requires_filesystem_permission() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("data");
        std::fs::write(&file, b"abc").unwrap();
        let js = NeoJs::new(Permissions {
            filesystem: true,
            environment: false,
            ..Permissions::default()
        })
        .unwrap();
        // Escape the path for a single-quoted JavaScript string.
        let path = file
            .to_str()
            .unwrap()
            .replace('\\', "\\\\")
            .replace('\'', "\\'");
        js.run_module(
            "hash-file-test.js",
            &format!(
                "import {{digestFile}} from 'neo:hash'; if (digestFile('sha256', '{path}') !== \
                 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad') throw Error('digest');"
            ),
        )
        .unwrap();
    }
    #[test]
    fn unavailable_runtime_directory_never_falls_back_to_the_host() {
        let host_before = std::env::current_dir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("recovered.txt"), "yes").unwrap();
        std::fs::write(dir.path().join("absolute.txt"), "absolute").unwrap();
        let absolute =
            serde_json::to_string(dir.path().join("absolute.txt").to_str().unwrap()).unwrap();

        let js = NeoJs::new(Permissions {
            filesystem: true,
            stdio: true,
            ..Permissions::default()
        })
        .unwrap();
        *js.cwd.borrow_mut() = crate::cwd::WorkingDirectory::unavailable();

        let error = js
            .eval("fs.readText('neo-js-no-such-relative-file')")
            .unwrap_err();
        assert!(
            error.contains("runtime working directory is unavailable"),
            "{error}"
        );
        assert_eq!(
            js.eval(&format!("fs.readText({absolute})")).unwrap(),
            "absolute"
        );
        let absolute_hash =
            serde_json::to_string(dir.path().join("absolute.txt").to_str().unwrap()).unwrap();
        js.run_module_with_identity(
            "hash-absolute.js",
            "hash-absolute",
            &format!(
                "import {{digestFile}} from 'neo:hash';
                 if (digestFile('sha256', {absolute_hash}).length !== 64) throw Error('hash');"
            ),
        )
        .unwrap();

        std::fs::write(
            dir.path().join("absolute-dependency.js"),
            "globalThis.absoluteDependency = true;",
        )
        .unwrap();
        let absolute_entry = dir.path().join("absolute-entry.js");
        std::fs::write(&absolute_entry, "").unwrap();
        js.run_module(
            absolute_entry.to_str().unwrap(),
            "import './absolute-dependency.js';
             globalThis.absoluteEntry = true;",
        )
        .unwrap();
        assert_eq!(js.eval("String(globalThis.absoluteEntry)").unwrap(), "true");
        assert_eq!(
            js.eval("String(globalThis.absoluteDependency)").unwrap(),
            "true"
        );

        let error = js
            .run_module("virtual.js", "globalThis.ran = true;")
            .unwrap_err();
        assert!(
            error.contains("runtime working directory is unavailable"),
            "{error}"
        );
        js.run_module_with_identity("virtual.js", "virtual", "globalThis.ran = true;")
            .unwrap();
        assert_eq!(js.eval("String(globalThis.ran)").unwrap(), "true");

        let error = js
            .run_module_with_identity("import.js", "import", "import './dependency.js';")
            .unwrap_err();
        assert!(
            error.contains("runtime working directory is unavailable"),
            "{error}"
        );

        let hash = NeoJs::new(Permissions {
            filesystem: true,
            ..Permissions::default()
        })
        .unwrap();
        *hash.cwd.borrow_mut() = crate::cwd::WorkingDirectory::unavailable();
        hash.run_module_with_identity(
            "hash.js",
            "hash",
            "import {digestFile} from 'neo:hash';
             let denied = false;
             try { digestFile('sha256', 'relative'); }
             catch (error) {
                 if (!String(error).includes('runtime working directory is unavailable')) throw error;
                 denied = true;
             }
             if (!denied) throw Error('relative hash used the host directory');",
        )
        .unwrap();

        let fetch = NeoJs::new(Permissions {
            network: true,
            filesystem: true,
            ..Permissions::default()
        })
        .unwrap();
        *fetch.cwd.borrow_mut() = crate::cwd::WorkingDirectory::unavailable();
        fetch
            .run_module_with_identity(
                "fetch.js",
                "fetch",
                "import {download} from 'neo:fetch';
                 let denied = false;
                 try { download('http://127.0.0.1:1/unused', 'relative.bin'); }
                 catch (error) {
                     if (error.code !== 'FETCH_CWD') throw error;
                     denied = true;
                 }
                 if (!denied) throw Error('relative fetch used the host directory');",
            )
            .unwrap();
        let absolute_destination =
            serde_json::to_string(dir.path().join("download.bin").to_str().unwrap()).unwrap();
        fetch
            .run_module_with_identity(
                "fetch-absolute.js",
                "fetch-absolute",
                &format!(
                    "import {{download}} from 'neo:fetch';
                     let reached = false;
                     try {{ download('not-a-url', {absolute_destination}); }}
                     catch (error) {{
                         if (error.code !== 'FETCH_INVALID_URL') throw error;
                         reached = true;
                     }}
                     if (!reached) throw Error('absolute destination was blocked by cwd');"
                ),
            )
            .unwrap();

        js.run_module_with_identity(
            "relative-chdir.js",
            "relative-chdir",
            "import {chdir, cwd} from 'neo:process';
             let denied = false;
             try { cwd(); }
             catch (error) {
                 if (error.code !== 'PROCESS_CWD') throw error;
                 denied = true;
             }
             if (!denied) throw Error('cwd invented a directory');
             denied = false;
             try { chdir('relative'); }
             catch (error) {
                 if (error.code !== 'PROCESS_CWD') throw error;
                 denied = true;
             }
             if (!denied) throw Error('relative chdir fell back to the host');",
        )
        .unwrap();

        let directory = serde_json::to_string(dir.path().to_str().unwrap()).unwrap();
        js.run_module_with_identity(
            "recover.js",
            "recover",
            &format!("import {{chdir}} from 'neo:process';\nchdir({directory});"),
        )
        .unwrap();
        assert_eq!(js.eval("fs.readText('recovered.txt')").unwrap(), "yes");
        assert_eq!(std::env::current_dir().unwrap(), host_before);
    }

    #[test]
    fn unavailable_base_recovery_needs_a_new_runtime_or_distinct_identity() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("dependency.js"),
            "globalThis.dependencyLoaded = true;",
        )
        .unwrap();

        let js = NeoJs::new(Permissions {
            filesystem: true,
            stdio: true,
            ..Permissions::default()
        })
        .unwrap();
        *js.cwd.borrow_mut() = crate::cwd::WorkingDirectory::unavailable();
        js.run_module_with_identity("entry.js", "entry", "globalThis.first = true;")
            .unwrap();

        let directory = serde_json::to_string(dir.path().to_str().unwrap()).unwrap();
        js.run_module_with_identity(
            "setup.js",
            "setup",
            &format!(
                "import {{chdir}} from 'neo:process';
chdir({directory});"
            ),
        )
        .unwrap();

        let error = js
            .run_module_with_identity("entry.js", "entry", "globalThis.second = true;")
            .unwrap_err();
        assert!(error.contains("already registered"), "{error}");

        let recovered_identity =
            serde_json::to_string(dir.path().join("recovered-entry.js").to_str().unwrap()).unwrap();
        js.run_module_with_identity(
            "entry.js",
            recovered_identity.trim_matches('"'),
            "import './dependency.js';",
        )
        .unwrap();
        assert_eq!(
            js.eval("String(globalThis.dependencyLoaded)").unwrap(),
            "true"
        );
    }

    #[cfg(unix)]
    #[test]
    fn injected_non_unicode_cwd_makes_relative_ops_and_exec_unavailable() {
        use std::os::unix::ffi::OsStringExt;

        let host_before = std::env::current_dir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("absolute.txt"), "absolute").unwrap();
        let absolute =
            serde_json::to_string(dir.path().join("absolute.txt").to_str().unwrap()).unwrap();

        let js = NeoJs::new(Permissions {
            filesystem: true,
            process: true,
            ..Permissions::default()
        })
        .unwrap();
        let non_unicode = std::path::PathBuf::from(std::ffi::OsString::from_vec(
            b"/tmp/neo-js-non-unicode-\xff".to_vec(),
        ));
        *js.cwd.borrow_mut() = crate::cwd::WorkingDirectory::injected(Some(non_unicode));

        let error = js.eval("fs.readText('relative.txt')").unwrap_err();
        assert!(
            error.contains("runtime working directory is unavailable"),
            "{error}"
        );
        assert_eq!(
            js.eval(&format!("fs.readText({absolute})")).unwrap(),
            "absolute"
        );

        js.run_module_with_identity(
            "exec.js",
            "exec",
            "import {exec} from 'neo:exec';
             let denied = false;
             try { exec('neo-js-no-such-program'); }
             catch (error) {
                 if (error.code !== 'EXEC_CWD') throw error;
                 denied = true;
             }
             if (!denied) throw Error('exec used a non-Unicode cwd');",
        )
        .unwrap();

        let recovered = tempfile::tempdir().unwrap();
        std::fs::write(recovered.path().join("recovered.txt"), "yes").unwrap();
        let directory = serde_json::to_string(recovered.path().to_str().unwrap()).unwrap();
        js.run_module_with_identity(
            "recover.js",
            "recover",
            &format!("import {{chdir}} from 'neo:process';\nchdir({directory});"),
        )
        .unwrap();
        assert_eq!(js.eval("fs.readText('recovered.txt')").unwrap(), "yes");
        assert_eq!(std::env::current_dir().unwrap(), host_before);
    }

    #[test]
    fn absolute_entry_identity_imports_relative_file_with_unavailable_cwd() {
        let host_before = std::env::current_dir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("dependency.js"),
            "globalThis.absoluteIdentityDependency = true;",
        )
        .unwrap();

        let js = NeoJs::new(Permissions {
            filesystem: true,
            ..Permissions::default()
        })
        .unwrap();
        *js.cwd.borrow_mut() = crate::cwd::WorkingDirectory::unavailable();
        let identity = dir.path().join("entry.js");
        js.run_module_with_identity(
            "entry.js",
            identity.to_str().unwrap(),
            "import './dependency.js';",
        )
        .unwrap();
        assert_eq!(
            js.eval("String(globalThis.absoluteIdentityDependency)")
                .unwrap(),
            "true"
        );
        assert_eq!(std::env::current_dir().unwrap(), host_before);
    }

    #[test]
    // Inject unavailable state: public APIs cannot clear a valid cwd, and
    // changing the process cwd would affect unrelated tests and embedders.
    fn authorized_path_resolution_fails_only_when_it_needs_the_unavailable_cwd() {
        let host_before = std::env::current_dir().unwrap();
        let js = NeoJs::new_with_options(
            Permissions {
                stdio: true,
                ..Permissions::default()
            },
            RuntimeOptions::new().modules([RuntimeModule::Path, RuntimeModule::Process]),
        )
        .unwrap();
        *js.cwd.borrow_mut() = crate::cwd::WorkingDirectory::unavailable();
        js.run_module(
            "unavailable-path.js",
            r#"import { resolve, relative, SEPARATOR } from 'neo:path';
               function expectUnavailable(operation) {
                 try { operation(); }
                 catch (error) {
                   if (error.code !== 'PATH_CWD') throw error;
                   return;
                 }
                 throw Error('operation used an unavailable cwd');
               }
               const windows = SEPARATOR === '\\';
               const base = windows ? 'C:\\project' : '/project';
               const child = windows ? 'C:\\project\\file' : '/project/file';
               expectUnavailable(() => resolve('file'));
               expectUnavailable(() => relative(base, 'file'));
               if (resolve(base, 'file') !== child) throw Error('explicit base failed');
               if (relative(base, child) !== 'file') throw Error('absolute operands failed');"#,
        )
        .unwrap();
        assert_eq!(std::env::current_dir().unwrap(), host_before);
    }

    #[test]
    fn no_filesystem_virtual_entry_still_runs_when_cwd_is_unavailable() {
        let js = NeoJs::new(Permissions::default()).unwrap();
        *js.cwd.borrow_mut() = crate::cwd::WorkingDirectory::unavailable();
        js.run_module("virtual.js", "globalThis.ran = true;")
            .unwrap();
        assert_eq!(js.eval("String(globalThis.ran)").unwrap(), "true");
        assert!(
            js.run_module("virtual-import.js", "import './dependency.js';")
                .is_err()
        );
    }

    #[test]
    fn unavailable_runtime_directory_makes_exec_fail_without_spawning() {
        let js = NeoJs::new(Permissions {
            process: true,
            ..Permissions::default()
        })
        .unwrap();
        *js.cwd.borrow_mut() = crate::cwd::WorkingDirectory::unavailable();
        js.run_module_with_identity(
            "exec.js",
            "exec",
            "import {exec, spawn} from 'neo:exec';
             let denied = false;
             try { exec('neo-js-no-such-program'); }
             catch (error) {
                 if (error.code !== 'EXEC_CWD') throw error;
                 denied = true;
             }
             if (!denied) throw Error('exec ran with an unavailable cwd');
             let deniedSpawn = false;
             try { spawn('neo-js-no-such-program'); }
             catch (error) {
                 if (error.code !== 'EXEC_CWD') throw error;
                 deniedSpawn = true;
             }
             if (!deniedSpawn) throw Error('spawn ran with an unavailable cwd');",
        )
        .unwrap();
    }
}
