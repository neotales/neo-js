//! Static host-extension registration.
//!
//! A host extension adds one or more exact `neo:` module names to a runtime.
//! The extension does not own authority: [`crate::Permissions`] still decides
//! what a granted host function may touch, and the runtime's
//! [`crate::ModuleAllowlist`] still decides which modules exist at all. Core
//! also owns module resolution, the QuickJS module cache, initialization order,
//! and teardown; an extension is a descriptor plus one native loader.
//!
//! Registration is static and explicit. Each extension provides an identifier
//! and a list of exact module names. [`ExtensionRegistry::register`] rejects:
//!
//! * an empty or duplicate extension identifier;
//! * an invalid module name, or a name that is not exactly `neo:` plus a
//!   non-empty suffix without a leading, trailing, empty, or relative path
//!   segment, whitespace, or control characters;
//! * a module name that collides with a built-in module;
//! * the same module name twice, in one descriptor or across descriptors.
//!
//! Validation is all-or-nothing: a failed registration leaves the registry
//! unchanged.
//!
//! An extension is trusted host code, not a sandbox. Registration prevents
//! accidental duplicate and built-in collisions; it does not constrain what a
//! native loader can do with the `Ctx` it receives.
//!
//! # Lifecycle and failure semantics
//!
//! For every runtime, an extension is initialized once, in registration order,
//! before user code runs, but only when at least one of its modules is both
//! admitted by the runtime allowlist and permitted by [`crate::Permissions`].
//! The init hook receives the exact names that passed both gates, so it can
//! avoid installing state for a module that the runtime withdrew. A module that
//! is not initialized cannot be loaded.
//!
//! [`ExtensionDescriptor::with_teardown`] runs in reverse registration order
//! when the [`crate::NeoJs`] runtime is dropped. It receives no `Ctx` and must
//! release native state only; it must not execute JavaScript. Teardown hooks
//! and state drops should not panic, but if they do, each panic is contained.
//!
//! If an init hook returns an error, extensions initialized earlier in the
//! same runtime are torn down in reverse order and runtime construction fails.
//! A JavaScript exception left by the hook is captured from the context before
//! it is dropped and reported as an owned diagnostic message; only primitive
//! strings and an Error object's own data `message` are read, so no getter or
//! `toString` is invoked. If an init hook panics, the panic is caught, earlier
//! extensions are still torn down by an unwinding guard, and construction
//! returns an owned error naming the extension and the panic payload. The
//! process panic hook still runs for a caught init panic; the library does not
//! replace global panic reporting. The
//! failed extension is responsible for releasing anything it created before
//! returning an error. A panicking init hook has no state to return, so any
//! partial native state it created must be held by local destructors.
//!
//! Caught payloads are dropped under a catch boundary. A finite chain of
//! panicking payload destructors is consumed without leaking either payload; an
//! intentionally endless chain cannot be both dropped and bounded, so it hangs
//! rather than leaking or aborting. The same limitation applies to the
//! filesystem worker's panic disposal.
//!
//! A failed module load does not tear down the runtime; the import fails and
//! later entry scripts still run. On a normal drop, the first teardown panic is
//! resumed only after every remaining teardown and core persistent cleanup have
//! completed. If the drop already happens during another unwind, teardown
//! panics are swallowed so the original unwind continues instead of aborting
//! the process with a double panic. Teardown panics during init-failure cleanup
//! are also contained and do not replace the init error.
//!
//! # Built-in names
//!
//! A built-in name is reserved for the runtime. Registration reports
//! [`ExtensionRegistrationError::CoreModuleCollision`] for a name such as
//! `neo:fs` that this build still provides. `neo:sqlite`, `neo:os`, and the
//! scheduler specifiers are no longer built in: the separate `neojs-sqlite`,
//! `neojs-os`, and `neojs-schedulers` packages register those exact names
//! through this API with [`ExtensionPermission::Sqlite`] and
//! [`ExtensionPermission::None`], and own their native dependencies. Core
//! still owns the permission gate, the allowlist, resolution, the module
//! cache, and teardown; an extension cannot define new authority.
//!
//! # Thread ownership
//!
//! [`ExtensionDescriptor`] and [`ExtensionRegistry`] are `Send + Sync` and hold
//! no QuickJS values: they contain only the identifier, exact names,
//! permissions, and function pointers. Per-runtime state is created by an init
//! hook with the runtime's `Ctx` and stays on that runtime's thread. No JS
//! value, `Persistent` handle, or extension state crosses threads through this
//! API.
//!
//! Hooks must not re-enter host module loading. An extension's state is already
//! borrowed while its loader runs; re-entering reports an error rather than
//! panicking.
//!
//! # Example
//!
//! ```rust
//! use std::any::Any;
//!
//! use neojs_core::{
//!     ExtensionDescriptor, ExtensionPermission, NeoJs, Permissions, RuntimeOptions,
//!     rquickjs::{
//!         Ctx, Error as JsError, Function, Module, Result as JsResult,
//!         module::{Declarations, Declared, Exports, ModuleDef},
//!     },
//! };
//!
//! struct Greeting;
//!
//! impl ModuleDef for Greeting {
//!     fn declare<'js>(decl: &Declarations<'js>) -> JsResult<()> {
//!         decl.declare("greet")?;
//!         Ok(())
//!     }
//!
//!     fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> JsResult<()> {
//!         exports.export("greet", Function::new(ctx.clone(), || "hello")?)?;
//!         Ok(())
//!     }
//! }
//!
//! struct GreetingState {
//!     live: bool,
//! }
//!
//! fn init<'js>(_ctx: &Ctx<'js>, _allowed: &[&str]) -> JsResult<Box<dyn Any>> {
//!     Ok(Box::new(GreetingState { live: true }))
//! }
//!
//! fn teardown(state: &mut dyn Any) {
//!     if let Some(state) = state.downcast_mut::<GreetingState>() {
//!         state.live = false;
//!     }
//! }
//!
//! fn load<'js>(
//!     ctx: &Ctx<'js>,
//!     state: &mut dyn Any,
//!     name: &str,
//! ) -> JsResult<Module<'js, Declared>> {
//!     let state = state
//!         .downcast_mut::<GreetingState>()
//!         .ok_or_else(|| JsError::new_loading_message(name, "greeting state is unavailable"))?;
//!     if !state.live {
//!         return Err(JsError::new_loading_message(name, "greeting state is torn down"));
//!     }
//!     Module::declare_def::<Greeting, _>(ctx.clone(), name)
//! }
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let extension = ExtensionDescriptor::new("greeting", load)
//!         .with_module("neo:greeting", ExtensionPermission::None)
//!         .with_init(init)
//!         .with_teardown(teardown);
//!     let options = RuntimeOptions::new().with_extensions([extension])?;
//!     let runtime = NeoJs::new_with_options(Permissions::default(), options)?;
//!     runtime.run_module(
//!         "main.js",
//!         "import {greet} from 'neo:greeting';
//!          if (greet() !== 'hello') throw Error('bad greeting');",
//!     )?;
//!     Ok(())
//! }
//! ```
//!
//! `neo:greeting` is importable because the default runtime has no allowlist.
//! With an explicit allowlist, the same import fails unless the exact name is
//! present, and with a non-`None` [`ExtensionPermission`] it also needs that
//! permission. In both cases the extension's init hook is skipped and no
//! module is loaded.

use std::{
    any::Any,
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    fmt,
    mem::MaybeUninit,
    panic::{self, AssertUnwindSafe},
};

use rquickjs::{Ctx, Error, Module, Object, Result as JsResult, Value, module::Declared, qjs};

use crate::{
    Permissions,
    modules::{AllowedModules, RuntimeModule},
    unwind::discard_panic,
};

/// The core permission that gates one extension module.
///
/// This enum only names existing [`Permissions`] fields. An extension cannot
/// define new authority or replace a permission check; it selects one of the
/// runtime's existing gates. [`Self::None`] means the module needs no
/// permission, not that it bypasses the runtime allowlist.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum ExtensionPermission {
    /// The module needs no permission flag.
    #[default]
    None,
    /// Requires [`Permissions::filesystem`].
    Filesystem,
    /// Requires [`Permissions::environment`].
    Environment,
    /// Requires [`Permissions::stdio`].
    Stdio,
    /// Requires [`Permissions::process`].
    Process,
    /// Requires [`Permissions::registry`].
    Registry,
    /// Requires [`Permissions::network`].
    Network,
    /// Requires [`Permissions::sqlite`].
    Sqlite,
}

impl ExtensionPermission {
    /// The stable diagnostic name of this permission, such as `filesystem`.
    pub const fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Filesystem => "filesystem",
            Self::Environment => "environment",
            Self::Stdio => "stdio",
            Self::Process => "process",
            Self::Registry => "registry",
            Self::Network => "network",
            Self::Sqlite => "sqlite",
        }
    }

    /// True when the runtime's permissions satisfy this gate.
    pub(crate) const fn granted(self, permissions: &Permissions) -> bool {
        match self {
            Self::None => true,
            Self::Filesystem => permissions.filesystem,
            Self::Environment => permissions.environment,
            Self::Stdio => permissions.stdio,
            Self::Process => permissions.process,
            Self::Registry => permissions.registry,
            Self::Network => permissions.network,
            Self::Sqlite => permissions.sqlite,
        }
    }
}

/// One exact `neo:` module provided by an extension.
///
/// The name is matched exactly, like a built-in module name. The constructor
/// does not validate; [`ExtensionRegistry::register`] performs all name
/// validation and rejects duplicates before any module becomes reachable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtensionModule {
    name: String,
    permission: ExtensionPermission,
}

impl ExtensionModule {
    /// Describe a module by exact name and the core permission that gates it.
    pub fn new(name: impl Into<String>, permission: ExtensionPermission) -> Self {
        Self {
            name: name.into(),
            permission,
        }
    }

    /// The exact module name, including the `neo:` prefix.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The core permission that gates this module.
    pub fn permission(&self) -> ExtensionPermission {
        self.permission
    }
}

/// Builds per-runtime extension state during runtime construction.
///
/// `allowed` contains the exact module names from this extension that passed
/// both the runtime allowlist and this module's permission. The returned state
/// stays on the runtime thread; it is passed back verbatim to the loader and
/// to [`ExtensionTeardown`]. Store one concrete type in the returned box and
/// downcast it in those hooks; core treats the value as opaque. Returning an
/// error makes runtime construction fail and tears down extensions already
/// initialized in that runtime.
pub type ExtensionInit = for<'js, 'a, 'b> fn(&Ctx<'js>, &'a [&'b str]) -> JsResult<Box<dyn Any>>;

/// Loads one extension module into the runtime's fresh realm.
///
/// `name` is one of the exact names registered by this extension. `state` is
/// the value returned by [`ExtensionInit`] for the same runtime and extension.
/// The loader must declare the module with the supplied name and must not
/// re-enter host module loading.
pub type ExtensionLoad =
    for<'js, 'a, 'b> fn(&Ctx<'js>, &'a mut dyn Any, &'b str) -> JsResult<Module<'js, Declared>>;

/// Releases per-runtime extension state before the QuickJS context is dropped.
///
/// Teardown receives the same state value returned by [`ExtensionInit`] but no
/// `Ctx`, so it cannot execute JavaScript from [`crate::NeoJs`]'s `Drop`. Drop
/// native handles and `Persistent` values instead. Teardown should not panic;
/// panics are contained per hook as described in the [module
/// documentation](self).
pub type ExtensionTeardown = for<'a> fn(&'a mut dyn Any);

fn noop_init<'js>(_ctx: &Ctx<'js>, _allowed: &[&str]) -> JsResult<Box<dyn Any>> {
    Ok(Box::new(()))
}

fn noop_teardown(_state: &mut dyn Any) {}

/// A static description of one host extension.
///
/// A descriptor contains no QuickJS value and is safe to share across threads.
/// Construct one with [`Self::new`], add exact module names and lifecycle
/// hooks, then register it on [`crate::RuntimeOptions`] or an
/// [`ExtensionRegistry`]. The same descriptor can be used for many runtimes;
/// each runtime receives independent state from [`ExtensionInit`].
///
/// Registration and import failures are described in the [module
/// documentation](self).
#[derive(Clone, Debug)]
pub struct ExtensionDescriptor {
    id: String,
    modules: Vec<ExtensionModule>,
    init: ExtensionInit,
    load: ExtensionLoad,
    teardown: ExtensionTeardown,
}

impl ExtensionDescriptor {
    /// Create a descriptor with a required native loader and no modules.
    ///
    /// The loader is called only for names added with [`Self::with_module`].
    /// Adding no modules is valid but makes the extension inert.
    pub fn new(id: impl Into<String>, load: ExtensionLoad) -> Self {
        Self {
            id: id.into(),
            modules: Vec::new(),
            init: noop_init,
            load,
            teardown: noop_teardown,
        }
    }

    /// Add one exact `neo:` module and the core permission that gates it.
    pub fn with_module(mut self, name: impl Into<String>, permission: ExtensionPermission) -> Self {
        self.modules.push(ExtensionModule::new(name, permission));
        self
    }

    /// Add several exact modules.
    pub fn with_modules<I>(mut self, modules: I) -> Self
    where
        I: IntoIterator<Item = ExtensionModule>,
    {
        self.modules.extend(modules);
        self
    }

    /// Set the per-runtime initialization hook.
    pub fn with_init(mut self, init: ExtensionInit) -> Self {
        self.init = init;
        self
    }

    /// Set the per-runtime teardown hook.
    ///
    /// It receives no `Ctx`, so it must release native state only; a `Drop`
    /// teardown cannot execute JavaScript.
    pub fn with_teardown(mut self, teardown: ExtensionTeardown) -> Self {
        self.teardown = teardown;
        self
    }

    /// The extension identifier used in registration diagnostics.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The exact modules described by this extension.
    pub fn modules(&self) -> &[ExtensionModule] {
        &self.modules
    }

    pub(crate) const fn init_fn(&self) -> ExtensionInit {
        self.init
    }

    pub(crate) const fn load_fn(&self) -> ExtensionLoad {
        self.load
    }

    pub(crate) const fn teardown_fn(&self) -> ExtensionTeardown {
        self.teardown
    }
}

/// A validated, immutable set of extension descriptors.
///
/// Registries are cloned into a runtime's options and shared by both the
/// resolver and the loader. Cloning copies descriptors but no per-runtime
/// state. Registration is all-or-nothing and rejects duplicate identifiers,
/// duplicate module names, built-in collisions, and invalid names with
/// [`ExtensionRegistrationError`].
#[derive(Clone, Debug, Default)]
pub struct ExtensionRegistry {
    descriptors: Vec<ExtensionDescriptor>,
    modules: BTreeMap<String, ExtensionRegistration>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ExtensionRegistration {
    descriptor: usize,
    permission: ExtensionPermission,
}

impl ExtensionRegistration {
    pub(crate) const fn permission(self) -> ExtensionPermission {
        self.permission
    }

    pub(crate) const fn descriptor(self) -> usize {
        self.descriptor
    }
}

impl ExtensionRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register one descriptor.
    ///
    /// On error the registry is unchanged. The checks run before anything is
    /// inserted, so a failed multi-module descriptor cannot leave some of its
    /// modules reachable.
    pub fn register(
        &mut self,
        descriptor: ExtensionDescriptor,
    ) -> Result<(), ExtensionRegistrationError> {
        if descriptor.id.trim().is_empty() {
            return Err(ExtensionRegistrationError::EmptyExtensionId);
        }
        if self
            .descriptors
            .iter()
            .any(|existing| existing.id == descriptor.id)
        {
            return Err(ExtensionRegistrationError::DuplicateExtension {
                id: descriptor.id.clone(),
            });
        }

        let mut local = BTreeSet::new();
        for module in &descriptor.modules {
            let name = module.name();
            if let Err(reason) = validate_module_name(name) {
                return Err(ExtensionRegistrationError::InvalidModuleName {
                    name: name.to_owned(),
                    reason,
                });
            }
            if RuntimeModule::from_name(name).is_some() {
                return Err(ExtensionRegistrationError::CoreModuleCollision {
                    name: name.to_owned(),
                });
            }
            if !local.insert(name) || self.modules.contains_key(name) {
                return Err(ExtensionRegistrationError::DuplicateModule {
                    name: name.to_owned(),
                });
            }
        }

        let index = self.descriptors.len();
        for module in &descriptor.modules {
            self.modules.insert(
                module.name.clone(),
                ExtensionRegistration {
                    descriptor: index,
                    permission: module.permission,
                },
            );
        }
        self.descriptors.push(descriptor);
        Ok(())
    }

    /// Register several descriptors in order.
    ///
    /// If a descriptor is rejected, no registry is returned; the partially
    /// built registry is dropped. Use [`Self::register`] directly when earlier
    /// successful registrations must be retained.
    pub fn from_descriptors<I>(descriptors: I) -> Result<Self, ExtensionRegistrationError>
    where
        I: IntoIterator<Item = ExtensionDescriptor>,
    {
        let mut registry = Self::new();
        for descriptor in descriptors {
            registry.register(descriptor)?;
        }
        Ok(registry)
    }

    /// The registered descriptors, in registration order.
    pub fn descriptors(&self) -> &[ExtensionDescriptor] {
        &self.descriptors
    }

    /// Number of registered extensions.
    pub fn len(&self) -> usize {
        self.descriptors.len()
    }

    /// True when no extension is registered.
    pub fn is_empty(&self) -> bool {
        self.descriptors.is_empty()
    }

    /// True when `name` is an exact module registered by an extension.
    pub fn contains_module(&self, name: &str) -> bool {
        self.modules.contains_key(name)
    }

    /// Every exact extension module name, in deterministic order.
    pub fn module_names(&self) -> impl Iterator<Item = &str> {
        self.modules.keys().map(String::as_str)
    }

    /// The permission gate and owning descriptor for an exact module name.
    pub(crate) fn registration(&self, name: &str) -> Option<ExtensionRegistration> {
        self.modules.get(name).copied()
    }
}

/// Why an extension descriptor was rejected.
///
/// The variant identifies the failure programmatically; [`Self::code`] returns
/// a stable string for logs, and the [`Display`](fmt::Display) message names
/// the offending identifier, module, or validation rule.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ExtensionRegistrationError {
    /// The descriptor's identifier is empty or only whitespace.
    EmptyExtensionId,
    /// Another descriptor in the registry already uses this identifier.
    DuplicateExtension {
        /// The repeated extension identifier.
        id: String,
    },
    /// A module name does not follow the exact `neo:` name rules.
    InvalidModuleName {
        /// The rejected name.
        name: String,
        /// The rule that rejected it.
        reason: &'static str,
    },
    /// The name is already provided by a built-in module.
    CoreModuleCollision {
        /// The colliding name.
        name: String,
    },
    /// The name is registered more than once.
    DuplicateModule {
        /// The repeated name.
        name: String,
    },
}

impl ExtensionRegistrationError {
    /// A stable, greppable code for this failure.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::EmptyExtensionId => "EXTENSION_EMPTY_ID",
            Self::DuplicateExtension { .. } => "EXTENSION_DUPLICATE_EXTENSION",
            Self::InvalidModuleName { .. } => "EXTENSION_INVALID_MODULE_NAME",
            Self::CoreModuleCollision { .. } => "EXTENSION_CORE_MODULE_COLLISION",
            Self::DuplicateModule { .. } => "EXTENSION_DUPLICATE_MODULE",
        }
    }
}

impl fmt::Display for ExtensionRegistrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyExtensionId => formatter.write_str("extension id is empty"),
            Self::DuplicateExtension { id } => {
                write!(formatter, "extension {id:?} is already registered")
            }
            Self::InvalidModuleName { name, reason } => {
                write!(
                    formatter,
                    "invalid extension module name {name:?}: {reason}"
                )
            }
            Self::CoreModuleCollision { name } => {
                write!(
                    formatter,
                    "extension module {name:?} collides with a built-in module"
                )
            }
            Self::DuplicateModule { name } => {
                write!(
                    formatter,
                    "extension module {name:?} is registered more than once"
                )
            }
        }
    }
}

impl std::error::Error for ExtensionRegistrationError {}

fn validate_module_name(name: &str) -> Result<(), &'static str> {
    let Some(rest) = name.strip_prefix("neo:") else {
        return Err("names must start with \"neo:\"");
    };
    if rest.is_empty() {
        return Err("the name after \"neo:\" is empty");
    }
    if rest.starts_with('/') || rest.ends_with('/') {
        return Err("the name cannot start or end with a slash");
    }
    if rest.split('/').any(|segment| segment.is_empty()) {
        return Err("the name cannot contain an empty path segment");
    }
    if rest
        .split('/')
        .any(|segment| segment == "." || segment == "..")
    {
        return Err("the name cannot contain a relative path segment");
    }
    if name
        .chars()
        .any(|character| character.is_whitespace() || character.is_control())
    {
        return Err("the name cannot contain whitespace or control characters");
    }
    Ok(())
}

/// Per-runtime extension state. Kept crate-private: callers never handle the
/// state directly, so no JS value can be extracted into a thread-crossing type.
pub(crate) struct ExtensionRuntime {
    descriptors: Vec<Option<ExtensionSession>>,
}

struct ExtensionSession {
    state: Option<RefCell<Box<dyn Any>>>,
}

impl ExtensionRuntime {
    pub(crate) fn new() -> Self {
        Self {
            descriptors: Vec::new(),
        }
    }

    /// Initialize each extension that has at least one module admitted by the
    /// allowlist and permitted by the runtime.
    ///
    /// The returned runtime has one slot per registered descriptor. `InitCleanup`
    /// tears down every extension initialized already if this function returns
    /// early, including when an init hook panics. Init panics are converted to
    /// owned errors instead of escaping.
    pub(crate) fn initialize(
        ctx: &Ctx<'_>,
        registry: &ExtensionRegistry,
        permissions: Permissions,
        allowed: &AllowedModules,
    ) -> JsResult<Self> {
        let mut runtime = Self {
            descriptors: Vec::with_capacity(registry.len()),
        };
        let mut cleanup = InitCleanup {
            sessions: &mut runtime.descriptors,
            registry,
            armed: true,
        };
        for descriptor in registry.descriptors() {
            let names: Vec<&str> = descriptor
                .modules()
                .iter()
                .filter(|module| {
                    allowed.allows_name(module.name()) && module.permission().granted(&permissions)
                })
                .map(ExtensionModule::name)
                .collect();
            if names.is_empty() {
                cleanup.sessions().push(None);
                continue;
            }
            let result =
                panic::catch_unwind(AssertUnwindSafe(|| (descriptor.init_fn())(ctx, &names)));
            let state = match result {
                Ok(Ok(state)) => state,
                Ok(Err(error)) => return Err(init_error(descriptor, ctx, error)),
                Err(payload) => {
                    // Keep the payload behind a disposal guard: formatting the
                    // diagnostic must not drop an arbitrary payload, and the
                    // diagnostic path itself must not leave it to unwinding.
                    let payload = PanicPayload::new(payload);
                    let error = init_panic_error(descriptor, ctx, payload.as_ref());
                    payload.discard();
                    return Err(error);
                }
            };
            cleanup.sessions().push(Some(ExtensionSession::new(state)));
        }
        cleanup.disarm();
        Ok(runtime)
    }

    /// Load one module from the extension that owns `descriptor`.
    pub(crate) fn load<'js>(
        &mut self,
        registry: &ExtensionRegistry,
        descriptor: usize,
        ctx: &Ctx<'js>,
        name: &str,
    ) -> JsResult<Module<'js, Declared>> {
        let session = self
            .descriptors
            .get_mut(descriptor)
            .and_then(Option::as_mut)
            .ok_or_else(|| {
                Error::new_loading_message(name, "extension module was not initialized")
            })?;
        let state = session.state.as_ref().ok_or_else(|| {
            Error::new_loading_message(name, "extension module was not initialized")
        })?;
        let descriptor = registry.descriptors().get(descriptor).ok_or_else(|| {
            Error::new_loading_message(name, "extension descriptor is unavailable")
        })?;
        let mut state = state
            .try_borrow_mut()
            .map_err(|_| Error::new_loading_message(name, "extension state is already in use"))?;
        (descriptor.load_fn())(ctx, &mut **state, name)
    }

    /// Release every extension state in reverse initialization order.
    ///
    /// Each hook and each state drop is run under `catch_unwind`; the first
    /// panic payload, if any, is returned after every remaining hook has run.
    /// Repeated calls do nothing, and a panic cannot skip a later hook or core
    /// cleanup.
    pub(crate) fn teardown_all(&mut self, registry: &ExtensionRegistry) -> Option<PanicPayload> {
        teardown_sessions(&mut self.descriptors, registry)
    }
}

impl ExtensionSession {
    fn new(state: Box<dyn Any>) -> Self {
        Self {
            state: Some(RefCell::new(state)),
        }
    }
}

/// Tears down initialized sessions even when an init hook panics or an earlier
/// teardown unwinds. Teardown panics are contained per hook.
struct InitCleanup<'a> {
    sessions: &'a mut Vec<Option<ExtensionSession>>,
    registry: &'a ExtensionRegistry,
    armed: bool,
}

impl InitCleanup<'_> {
    fn sessions(&mut self) -> &mut Vec<Option<ExtensionSession>> {
        self.sessions
    }

    fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for InitCleanup<'_> {
    fn drop(&mut self) {
        if self.armed {
            if let Some(payload) = teardown_sessions(self.sessions, self.registry) {
                payload.discard();
            }
        }
    }
}

fn teardown_sessions(
    sessions: &mut [Option<ExtensionSession>],
    registry: &ExtensionRegistry,
) -> Option<PanicPayload> {
    let mut first_panic = None;
    for (index, session) in sessions.iter_mut().enumerate().rev() {
        let Some(session) = session else {
            continue;
        };
        let Some(descriptor) = registry.descriptors().get(index) else {
            continue;
        };
        let Some(cell) = session.state.take() else {
            continue;
        };
        let mut state = cell.into_inner();
        // Keep the state outside the hook unwind so a panicking hook cannot
        // turn a panicking state Drop into a double panic. Both are contained.
        let hook_panic = panic::catch_unwind(AssertUnwindSafe(|| {
            (descriptor.teardown_fn())(&mut *state);
        }))
        .err();
        let drop_panic = panic::catch_unwind(AssertUnwindSafe(|| drop(state))).err();
        if let Some(payload) = hook_panic {
            record_panic(&mut first_panic, payload);
        }
        if let Some(payload) = drop_panic {
            record_panic(&mut first_panic, payload);
        }
    }
    first_panic
}

/// Owns a caught panic payload until it is resumed or safely discarded.
///
/// `Drop` uses `discard_panic`, so an unrelated unwind through this value
/// cannot drop an arbitrary payload destructor outside a catch boundary.
pub(crate) struct PanicPayload(Option<Box<dyn Any + Send>>);

impl PanicPayload {
    fn new(payload: Box<dyn Any + Send>) -> Self {
        Self(Some(payload))
    }

    fn as_ref(&self) -> &(dyn Any + Send) {
        self.0
            .as_ref()
            .map(Box::as_ref)
            .expect("panic payload was already taken")
    }

    pub(crate) fn discard(mut self) {
        if let Some(payload) = self.0.take() {
            discard_panic(payload);
        }
    }

    pub(crate) fn resume(mut self) -> ! {
        let payload = self.0.take().expect("panic payload was already taken");
        std::panic::resume_unwind(payload)
    }
}

impl Drop for PanicPayload {
    fn drop(&mut self) {
        if let Some(payload) = self.0.take() {
            discard_panic(payload);
        }
    }
}

/// Preserve the first payload for the caller to resume or discard. Later
/// payloads are discarded behind `discard_panic`, never by the ordinary
/// `Option` combinators that would drop them outside a catch boundary.
fn record_panic(first: &mut Option<PanicPayload>, payload: Box<dyn Any + Send>) {
    if first.is_none() {
        *first = Some(PanicPayload::new(payload));
    } else {
        discard_panic(payload);
    }
}

/// Convert an init hook error into an owned diagnostic before the context goes
/// away. A JS exception is read without invoking getters or `toString`.
fn init_error(descriptor: &ExtensionDescriptor, ctx: &Ctx<'_>, error: Error) -> Error {
    if error.is_exception() {
        let detail = capture_pending_exception(ctx)
            .unwrap_or_else(|| "JavaScript exception without a readable message".to_owned());
        return extension_init_error(descriptor, &format!("failed: {detail}"));
    }
    // Clear any unexpected pending exception so teardown starts clean.
    let _ = capture_pending_exception(ctx);
    error
}

fn init_panic_error(
    descriptor: &ExtensionDescriptor,
    ctx: &Ctx<'_>,
    payload: &(dyn Any + Send),
) -> Error {
    let mut detail = format!("panicked: {}", panic_message(payload));
    if let Some(exception) = capture_pending_exception(ctx) {
        detail.push_str(&format!("; JavaScript exception: {exception}"));
    }
    extension_init_error(descriptor, &detail)
}

fn extension_init_error(descriptor: &ExtensionDescriptor, detail: &str) -> Error {
    Error::new_loading_message(
        "extension-init",
        sanitize_diagnostic(&format!("extension {:?} {detail}", descriptor.id())),
    )
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_owned()
    }
}

/// Consume a pending JavaScript exception into an owned, bounded message.
///
/// Only primitive strings and an Error object's own data `message` property are
/// read. Accessors and `toString` are never invoked, so capture cannot re-enter
/// script or run extension code.
fn capture_pending_exception(ctx: &Ctx<'_>) -> Option<String> {
    if !ctx.has_exception() {
        return None;
    }
    let value = ctx.catch();
    let captured = if value.is_string() {
        value.get::<String>().ok()
    } else if value.is_error() {
        value
            .as_object()
            .and_then(|object| own_data_string(ctx, object, b"message\0"))
    } else {
        None
    };
    // A failed conversion or property lookup can leave a new exception behind.
    if ctx.has_exception() {
        drop(ctx.catch());
    }
    captured.map(|message| sanitize_diagnostic(&message))
}

fn own_data_string(ctx: &Ctx<'_>, object: &Object<'_>, name: &'static [u8]) -> Option<String> {
    let raw = ctx.as_raw().as_ptr();
    // SAFETY: `name` is a static NUL-terminated byte string. QuickJS returns an
    // atom that is freed immediately after the property lookup.
    let atom = unsafe { qjs::JS_NewAtom(raw, name.as_ptr().cast()) };
    if atom == 0 {
        return None;
    }
    let mut descriptor = MaybeUninit::<qjs::JSPropertyDescriptor>::uninit();
    // SAFETY: `object` is an object in this context and `descriptor` is a
    // writable out-parameter. `JS_GetOwnProperty` does not invoke getters.
    let status =
        unsafe { qjs::JS_GetOwnProperty(raw, descriptor.as_mut_ptr(), object.as_raw(), atom) };
    unsafe { qjs::JS_FreeAtom(raw, atom) };
    if status < 0 {
        if ctx.has_exception() {
            drop(ctx.catch());
        }
        return None;
    }
    if status == 0 {
        return None;
    }
    // SAFETY: a successful lookup initializes the descriptor and transfers its
    // three rooted values, including undefined placeholders.
    let descriptor = unsafe { descriptor.assume_init() };
    let accessor = descriptor.flags & qjs::JS_PROP_TMASK as i32 == qjs::JS_PROP_GETSET as i32;
    // SAFETY: the descriptor owns getter/setter/value references from the
    // successful lookup. Free the slots this function does not consume.
    unsafe {
        qjs::JS_FreeValue(raw, descriptor.getter);
        qjs::JS_FreeValue(raw, descriptor.setter);
    }
    if accessor {
        // An accessor could run script (or leak through its return value).
        unsafe { qjs::JS_FreeValue(raw, descriptor.value) };
        return None;
    }
    // SAFETY: a data property's value is owned by this descriptor and is now
    // owned by `Value`, which releases it on drop.
    let value = unsafe { Value::from_raw(ctx.clone(), descriptor.value) };
    if !value.is_string() {
        return None;
    }
    value.get::<String>().ok()
}

fn sanitize_diagnostic(text: &str) -> String {
    const LIMIT: usize = 512;
    let mut result = String::new();
    let mut truncated = false;
    for (index, character) in text.chars().enumerate() {
        if index == LIMIT {
            truncated = true;
            break;
        }
        result.push(if character.is_control() {
            ' '
        } else {
            character
        });
    }
    if truncated {
        result.push_str("...");
    }
    result
}
