use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
    rc::Rc,
};

use rquickjs::{
    Ctx, Error, Function, Module, Object, Persistent, Result,
    loader::{ImportAttributes, Loader, Resolver},
    module::Declared,
    module::{Declarations, Exports, ModuleDef},
};

use crate::{
    Permissions,
    extension::{ExtensionRegistry, ExtensionRuntime},
    modules::{AllowedModules, RuntimeModule},
};

const ASSERT_MODULE: &str = include_str!("js/assert.js");
const TEST_MODULE: &str = include_str!("js/test.js");
const ANSI_MODULE: &str = include_str!("js/ansi.js");
const CHARS_MODULE: &str = include_str!("js/chars.js");
const STRING_BUILDER_MODULE: &str = include_str!("js/string_builder.js");
const EXEC_MODULE: &str = include_str!("js/exec.js");

// Give each scheduler specifier its own export surface, without importing
// another public specifier or introducing an importable internal module.
// The legacy surface intentionally exports both sets of helpers.
macro_rules! scheduler_module {
    ($($export:ident),+ $(,)?) => {
        concat!(
            include_str!("js/scheduler.js"),
            "\nexport { SchedulerError, ",
            $(stringify!($export), ", ",)+
            "};\n",
        )
    };
}

const SCHEDULER_SYSTEMD_MODULE: &str = scheduler_module!(
    systemdService,
    systemdTimer,
    systemdUnits,
    MAX_SYSTEMD_LINE_BYTES
);
const SCHEDULER_WINDOWS_MODULE: &str =
    scheduler_module!(windowsTask, WINDOWS_XML_ENCODING, MAX_WINDOWS_COMMAND_LINE);
const LEGACY_SCHEDULER_MODULE: &str = scheduler_module!(
    systemdService,
    systemdTimer,
    systemdUnits,
    MAX_SYSTEMD_LINE_BYTES,
    windowsTask,
    WINDOWS_XML_ENCODING,
    MAX_WINDOWS_COMMAND_LINE,
);

/// Resolves only the modules provided by this host and relative JavaScript or
/// TypeScript files. A TypeScript file is stripped in [`Loader::load`].
pub(crate) struct ModuleLoader {
    pub permissions: Permissions,
    pub formats: Rc<RefCell<ModuleFormats>>,
    /// Optional fail-closed module allowlist. `None` bits are unrestricted.
    pub allowed: AllowedModules,
    /// Registered extension descriptors, used only for exact name lookup.
    pub extensions: Rc<ExtensionRegistry>,
    /// Per-runtime extension state shared with `NeoJs` for teardown.
    pub extension_runtime: Rc<RefCell<ExtensionRuntime>>,
}

#[derive(Default)]
pub(crate) struct ModuleFormats {
    entries: HashMap<String, FormatEntry>,
    next_token: u64,
    cycle_seen: bool,
    poisoned: bool,
}

struct FormatEntry {
    requested: String,
    token: u64,
    ready: bool,
    /// Absolute directory captured when the module was declared. Relative
    /// imports use this, never the runtime's current directory at import time.
    base_dir: Option<PathBuf>,
}

impl ModuleFormats {
    pub fn register(
        &mut self,
        identity: &str,
        requested: &str,
        base_dir: Option<PathBuf>,
    ) -> std::result::Result<u64, String> {
        let format = |name: &str| match Path::new(name).extension().and_then(|s| s.to_str()) {
            Some("ts") => "ts",
            Some("mts") => "mts",
            Some("mjs") => "mjs",
            _ => "js",
        };
        if let Some(previous) = self.entries.get(identity) {
            if format(&previous.requested) != format(requested) {
                return Err(format!(
                    "module format conflict for {identity}: {} and {requested}",
                    previous.requested
                ));
            }
            if !previous.ready {
                self.cycle_seen = true;
            }
            Ok(previous.token)
        } else {
            self.next_token += 1;
            let token = self.next_token;
            self.entries.insert(
                identity.to_owned(),
                FormatEntry {
                    requested: requested.to_owned(),
                    token,
                    ready: false,
                    base_dir,
                },
            );
            Ok(token)
        }
    }

    pub fn reserve_entry(
        &mut self,
        identity: &str,
        requested: &str,
        base_dir: Option<PathBuf>,
    ) -> std::result::Result<u64, String> {
        if self.entries.contains_key(identity) {
            return Err(format!("module identity already registered: {identity}"));
        }
        self.register(identity, requested, base_dir)
    }

    pub fn complete(&mut self, identity: &str, token: u64, success: bool) {
        if let Some(entry) = self.entries.get_mut(identity) {
            if entry.token != token || entry.ready {
                return;
            }
            if success {
                entry.ready = true;
            } else {
                self.entries.remove(identity);
                if self.cycle_seen {
                    // QuickJS can retain a resolved child that points at a
                    // freed module after a cyclic graph fails to resolve.
                    // Do not let subsequent imports reach that dangling graph.
                    self.poisoned = true;
                }
            }
        }
        if !self.entries.values().any(|entry| !entry.ready) {
            self.cycle_seen = false;
        }
    }

    pub fn ensure_usable(&self) -> std::result::Result<(), String> {
        if self.poisoned {
            Err("module loading disabled after failed cyclic resolution".to_owned())
        } else {
            Ok(())
        }
    }
}

impl ModuleLoader {
    fn builtin(&self, name: &str) -> Option<&'static str> {
        match name {
            "neo:ansi" => Some(ANSI_MODULE),
            "neo:chars" => Some(CHARS_MODULE),
            "neo:scheduler" => Some(LEGACY_SCHEDULER_MODULE),
            "neo:os/schedulers/systemd" => Some(SCHEDULER_SYSTEMD_MODULE),
            "neo:os/schedulers/windows" => Some(SCHEDULER_WINDOWS_MODULE),
            "neo:string-builder" => Some(STRING_BUILDER_MODULE),
            "neo:hash" if self.permissions.filesystem => Some(
                "export function digest(algorithm, bytes) { \
                     if (!(bytes instanceof Uint8Array)) throw new TypeError('expected Uint8Array'); \
                     return globalThis.__neoHash(algorithm, Array.from(bytes)); \
                 } \
                 export const digestFile = globalThis.__neoHashFile;",
            ),
            "neo:hash" => Some(
                "export function digest(algorithm, bytes) { \
                     if (!(bytes instanceof Uint8Array)) throw new TypeError('expected Uint8Array'); \
                     return globalThis.__neoHash(algorithm, Array.from(bytes)); \
                 }",
            ),
            "neo:env" if self.permissions.environment => {
                Some("export const get = env.get, has = env.has;")
            }
            "neo:exec" if self.permissions.process => Some(EXEC_MODULE),
            _ => None,
        }
    }

    /// Whether the module's own permission is granted.
    ///
    /// The allowlist is checked separately and first. This reproduces the
    /// pre-allowlist rules: a module can be known to the host yet still denied
    /// because its capability was not granted.
    fn module_permitted(&self, module: RuntimeModule) -> bool {
        match module {
            RuntimeModule::Age
            | RuntimeModule::Ansi
            | RuntimeModule::Assert
            | RuntimeModule::Chars
            | RuntimeModule::Crypto
            | RuntimeModule::Dotenv
            | RuntimeModule::Hash
            | RuntimeModule::Path
            | RuntimeModule::Scheduler
            | RuntimeModule::SchedulerSystemd
            | RuntimeModule::SchedulerWindows
            | RuntimeModule::Semver
            | RuntimeModule::StringBuilder
            | RuntimeModule::Test
            | RuntimeModule::Toml
            | RuntimeModule::Yaml => true,
            RuntimeModule::Env => self.permissions.environment,
            RuntimeModule::Exec => self.permissions.process,
            RuntimeModule::Fetch => self.permissions.network,
            RuntimeModule::Fs => self.permissions.filesystem,
            RuntimeModule::Process => self.permissions.process || self.permissions.stdio,
            RuntimeModule::WinRegistry => self.permissions.registry,
        }
    }
}

impl Resolver for ModuleLoader {
    fn resolve<'js>(
        &mut self,
        _ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> Result<String> {
        self.formats
            .borrow()
            .ensure_usable()
            .map_err(|error| Error::new_resolving_message(base, name, error))?;
        // Every host module is checked here, before QuickJS looks in its
        // module cache. `resolve` is called on each import request, including
        // dynamic imports and repeats, so a cached identity cannot bypass the
        // allowlist. Checking in `load` as well covers a direct loader call.
        if let Some(module) = RuntimeModule::from_name(name) {
            if !self.allowed.allows(module) {
                return Err(Error::new_resolving_message(
                    base,
                    name,
                    "module is not enabled by this runtime's allowlist",
                ));
            }
            // A module that is admitted by the allowlist still needs its own
            // permission. This preserves the rule that a permission flag is
            // necessary, never merely a way to bypass the allowlist.
            if !self.module_permitted(module) {
                return Err(Error::new_resolving(base, name));
            }
            return Ok(name.to_owned());
        }
        // Extension module names are exact and registered. The allowlist check
        // runs here, before QuickJS consults its module cache, so neither a
        // static nor a dynamic import can reach a withdrawn extension module.
        if let Some(registration) = self.extensions.registration(name) {
            if !self.allowed.allows_name(name) {
                return Err(Error::new_resolving_message(
                    base,
                    name,
                    "module is not enabled by this runtime's allowlist",
                ));
            }
            if !registration.permission().granted(&self.permissions) {
                return Err(Error::new_resolving(base, name));
            }
            return Ok(name.to_owned());
        }
        if self.permissions.filesystem && (name.starts_with("./") || name.starts_with("../")) {
            crate::typescript::validate_extension(name)
                .map_err(|error| Error::new_resolving_message(base, name, error))?;
            let base_dir = {
                let formats = self.formats.borrow();
                let entry = formats.entries.get(base).ok_or_else(|| {
                    Error::new_resolving_message(base, name, "module base is unavailable")
                })?;
                entry.base_dir.clone().ok_or_else(|| {
                    Error::new_resolving_message(
                        base,
                        name,
                        "the runtime working directory is unavailable",
                    )
                })?
            };
            let path = base_dir.join(name);
            if path
                .extension()
                .is_some_and(|ext| ext == "js" || ext == "mjs" || ext == "ts" || ext == "mts")
            {
                let canonical = std::fs::canonicalize(&path)
                    .map_err(|error| Error::new_resolving_message(base, name, error.to_string()))?;
                let identity = canonical.to_str().ok_or_else(|| {
                    Error::new_resolving_message(base, name, "resolved path is not valid UTF-8")
                })?;
                let dependency_base = canonical.parent().unwrap_or(Path::new(".")).to_path_buf();
                self.formats
                    .borrow_mut()
                    .register(identity, name, Some(dependency_base))
                    .map_err(|error| Error::new_resolving_message(base, name, error))?;
                return Ok(identity.to_owned());
            }
        }
        Err(Error::new_resolving(base, name))
    }
}

impl Loader for ModuleLoader {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> Result<Module<'js, Declared>> {
        // Defense in depth: `resolve` already enforced both gates, but a load
        // call must never reach a module the runtime did not admit.
        if let Some(module) = RuntimeModule::from_name(name) {
            if !self.allowed.allows(module) {
                return Err(Error::new_loading_message(
                    name,
                    "module is not enabled by this runtime's allowlist",
                ));
            }
            if !self.module_permitted(module) {
                return Err(Error::new_loading(name));
            }
        }
        // Defense in depth: repeat both gates before dispatching to an
        // extension. `try_borrow_mut` makes a re-entrant load fail closed
        // instead of panicking while the same extension state is borrowed.
        if let Some(registration) = self.extensions.registration(name) {
            if !self.allowed.allows_name(name) {
                return Err(Error::new_loading_message(
                    name,
                    "module is not enabled by this runtime's allowlist",
                ));
            }
            if !registration.permission().granted(&self.permissions) {
                return Err(Error::new_loading(name));
            }
            let extension_runtime = Rc::clone(&self.extension_runtime);
            let mut runtime = extension_runtime.try_borrow_mut().map_err(|_| {
                Error::new_loading_message(name, "extension state is already in use")
            })?;
            return runtime.load(&self.extensions, registration.descriptor(), ctx, name);
        }
        if name == "neo:crypto" {
            return Module::declare_def::<crate::crypto::CryptoModule, _>(ctx.clone(), name);
        }
        if name == "neo:age" {
            return Module::declare_def::<crate::age::AgeModule, _>(ctx.clone(), name);
        }
        if name == "neo:dotenv" {
            return Module::declare_def::<crate::dotenv::DotenvModule, _>(ctx.clone(), name);
        }
        if name == "neo:toml" {
            return Module::declare_def::<crate::toml::TomlModule, _>(ctx.clone(), name);
        }
        if name == "neo:yaml" {
            return Module::declare_def::<crate::yaml::YamlModule, _>(ctx.clone(), name);
        }
        if name == "neo:semver" {
            return Module::declare_def::<crate::semver::SemverModule, _>(ctx.clone(), name);
        }
        if name == "neo:process" {
            if !(self.permissions.process || self.permissions.stdio) {
                return Err(Error::new_loading(name));
            }
            return Module::declare_def::<crate::stdio::ProcessModule, _>(ctx.clone(), name);
        }
        if name == "neo:fs" {
            if !self.permissions.filesystem {
                return Err(Error::new_loading(name));
            }
            return Module::declare_def::<crate::fs::FsModule, _>(ctx.clone(), name);
        }
        if name == "neo:path" {
            return Module::declare_def::<crate::path::PathModule, _>(ctx.clone(), name);
        }
        if name == "neo:winregistry" {
            if !self.permissions.registry {
                return Err(Error::new_loading(name));
            }
            return Module::declare_def::<crate::winregistry::WinRegistryModule, _>(
                ctx.clone(),
                name,
            );
        }
        if name == "neo:fetch" {
            if !self.permissions.network {
                return Err(Error::new_loading(name));
            }
            return Module::declare_def::<crate::fetch::FetchModule, _>(ctx.clone(), name);
        }
        if name == "neo:test" {
            return Module::declare_def::<TestModule, _>(ctx.clone(), name);
        }
        if name == "neo:assert" {
            return Module::declare_def::<AssertModule, _>(ctx.clone(), name);
        }
        if let Some(source) = self.builtin(name) {
            return Module::declare(ctx.clone(), name, source);
        }
        if !self.permissions.filesystem {
            return Err(Error::new_loading(name));
        }
        let (requested, token) = {
            let formats = self.formats.borrow();
            let entry = formats
                .entries
                .get(name)
                .ok_or_else(|| Error::new_loading_message(name, "missing module format"))?;
            (entry.requested.clone(), entry.token)
        };
        let result = (|| {
            let source = std::fs::read(name)?;
            if !crate::typescript::is_typescript(&requested) {
                return Module::declare(ctx.clone(), name, source);
            }
            let source = String::from_utf8(source).map_err(|_| {
                Error::new_loading_message(
                    name,
                    format!("{name}:1:1: TypeScript source is not valid UTF-8 [TS_PARSE]"),
                )
            })?;
            match crate::typescript::strip_if_typescript(&requested, &source) {
                Ok(Some(stripped)) => Module::declare(ctx.clone(), name, stripped),
                Ok(None) => Module::declare(ctx.clone(), name, source),
                Err(error) => Err(Error::new_loading_message(name, error)),
            }
        })();
        self.formats
            .borrow_mut()
            .complete(name, token, result.is_ok());
        result
    }
}

// Only the module's factory receives these functions; they are never exported
// or stored on the global object. Loading a similarly named user module cannot
// acquire the callbacks.
struct TestModule;

// Construct the private closure in the fresh realm, before user code runs.
// Invoking the returned factory, not constructing it, installs the runner.
pub(crate) fn test_factory<'js>(ctx: &Ctx<'js>, guard: &Object<'js>) -> Result<Function<'js>> {
    let factory: Function = ctx.eval(format!(
        r#"(function(guard) {{
            'use strict';
            const {{invalidCallback, unsupportedReturn}} = guard;
            const nativeString = String;
            const charCodeAt = Function.prototype.call.bind(String.prototype.charCodeAt);
            const toWellFormed = Function.prototype.call.bind(String.prototype.toWellFormed);
            const defineProperty = Object.defineProperty;
            const NativeError = Error;
            const NativeTypeError = TypeError;
            const queue = Object.create(null);
            return function(installRunner, setCounts, recordFailure) {{
                {}
            }};
        }})"#,
        TEST_MODULE
    ))?;
    factory.call((guard.clone(),))
}

// Keep the guard out of the module namespace and globals, even when only
// `neo:assert` is allowed.
struct AssertModule;

const ASSERT_EXPORTS: &[&str] = &[
    "AssertionError",
    "equal",
    "notEqual",
    "deepEqual",
    "notDeepEqual",
    "ok",
    "fail",
    "throws",
    "doesNotThrow",
];

/// Construct the assertion API with pristine intrinsics and a private guard.
pub(crate) fn assert_api<'js>(ctx: &Ctx<'js>, guard: &Object<'js>) -> Result<Object<'js>> {
    let factory: Function = ctx.eval(format!(
        "(function (guard) {{ 'use strict'; {ASSERT_MODULE} }})"
    ))?;
    factory.call((guard.clone(),))
}

impl ModuleDef for AssertModule {
    fn declare<'js>(decl: &Declarations<'js>) -> Result<()> {
        for &name in ASSERT_EXPORTS {
            decl.declare(name)?;
        }
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        let state = ctx
            .userdata::<Rc<RefCell<crate::TestState>>>()
            .ok_or(Error::Unknown)?;
        let api = state
            .borrow()
            .assert_api
            .as_ref()
            .cloned()
            .ok_or(Error::Unknown)?
            .restore(ctx)?;
        for &name in ASSERT_EXPORTS {
            exports.export(name, api.get::<_, rquickjs::Value>(name)?)?;
        }
        Ok(())
    }
}

impl ModuleDef for TestModule {
    fn declare<'js>(decl: &Declarations<'js>) -> Result<()> {
        for name in ["default", "test", "run", "MAX_TESTS", "TestFailureError"] {
            decl.declare(name)?;
        }
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        use std::{cell::RefCell, rc::Rc};
        let tests = Rc::clone(
            &*ctx
                .userdata::<Rc<RefCell<crate::TestState>>>()
                .ok_or(Error::Unknown)?,
        );
        let install_state = Rc::clone(&tests);
        let install = Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, runner: Function<'js>| -> Result<()> {
                let mut state = install_state.borrow_mut();
                if state.runner.is_some() {
                    return Err(Error::new_from_js_message(
                        "function",
                        "test runner",
                        "a test runner is already installed",
                    ));
                }
                state.runner = Some(Persistent::save(&ctx, runner));
                Ok(())
            },
        )?;
        let counts_state = Rc::clone(&tests);
        let counts = Function::new(
            ctx.clone(),
            move |total: usize, passed: usize, failed: usize| -> Result<()> {
                if passed.checked_add(failed) != Some(total) {
                    return Err(Error::new_from_js_message(
                        "number",
                        "test report",
                        "counts do not add up",
                    ));
                }
                let mut state = counts_state.borrow_mut();
                state.total = total;
                state.passed = passed;
                state.failed = failed;
                Ok(())
            },
        )?;
        let failure_state = Rc::clone(&tests);
        let failure = Function::new(ctx.clone(), move |summary: String| {
            failure_state.borrow_mut().failures.push(summary);
        })?;
        let factory = tests
            .borrow()
            .factory
            .as_ref()
            .cloned()
            .ok_or(Error::Unknown)?
            .restore(ctx)?;
        let api: Object = factory.call((install, counts, failure))?;
        for name in ["default", "test", "run", "MAX_TESTS", "TestFailureError"] {
            exports.export(name, api.get::<_, rquickjs::Value>(name)?)?;
        }
        Ok(())
    }
}
