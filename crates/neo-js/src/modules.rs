//! Which host-provided `neo:` modules a runtime may load.
//!
//! A module allowlist is a second, independent axis beside [`crate::Permissions`].
//! Permissions describe what a granted host function may touch; the allowlist
//! describes which host modules are constructed at all. A module that is not on
//! the allowlist is not importable, and the globals or native callbacks that
//! only it uses are not installed. A module that *is* on the allowlist still
//! needs its permission: an allowlisted `neo:fs` with
//! [`crate::Permissions::filesystem`] denied remains unavailable.
//!
//! This is not an OS sandbox. It does not limit CPU, memory, or the JavaScript
//! language itself, and it cannot isolate a script from the rest of the realm.

use std::{collections::BTreeSet, fmt, rc::Rc};

use crate::extension::{ExtensionDescriptor, ExtensionRegistrationError, ExtensionRegistry};

/// One host-provided `neo:` module.
///
/// The canonical name is available from [`RuntimeModule::name`] and parsed by
/// [`RuntimeModule::from_name`]. Names are exact: `neo:hash` is accepted and
/// `hash` or `neo:Hash` are not.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
#[repr(u8)]
pub enum RuntimeModule {
    /// `neo:age` — synchronous age file encryption.
    Age,
    /// `neo:ansi` — ANSI style helpers.
    Ansi,
    /// `neo:assert` — synchronous assertions.
    Assert,
    /// `neo:chars` — Unicode character helpers.
    Chars,
    /// `neo:crypto` — AES-256-GCM encryption helpers.
    Crypto,
    /// `neo:dotenv` — `.env` text parsing.
    Dotenv,
    /// `neo:env` — environment reads; also requires
    /// [`crate::Permissions::environment`].
    Env,
    /// `neo:exec` — synchronous child programs; also requires
    /// [`crate::Permissions::process`].
    Exec,
    /// `neo:fetch` — synchronous downloads; also requires
    /// [`crate::Permissions::network`].
    Fetch,
    /// `neo:fs` — bounded file reads, writes, metadata, listing, directory
    /// creation, copy, rename, and deletion; also requires
    /// [`crate::Permissions::filesystem`].
    Fs,
    /// `neo:hash` — digests. `digestFile` additionally requires
    /// [`crate::Permissions::filesystem`].
    Hash,
    /// `neo:path` — lexical path helpers and a global file-only `URL` class.
    /// Reading the runtime working directory also requires admitted
    /// filesystem or process/stdio authority.
    Path,
    /// `neo:process` — streams, runtime working directory, and host process
    /// elevation status; also requires [`crate::Permissions::process`] or
    /// [`crate::Permissions::stdio`].
    Process,
    /// `neo:semver` — semantic-version parsing, formatting, ranges, and
    /// comparison.
    Semver,
    /// `neo:string-builder` — a small string builder.
    StringBuilder,
    /// `neo:test` — the synchronous test registry.
    Test,
    /// `neo:toml` — TOML text parsing.
    Toml,
    /// `neo:win-registry` — Windows Registry access; also requires
    /// [`crate::Permissions::registry`].
    ///
    /// Migrate imports and name-based allowlists from the deprecated
    /// `neo:winregistry` spelling to `neo:win-registry`. Both names identify
    /// the same module and use this same allowlist entry. No release is
    /// scheduled to remove the alias. Neither spelling accepts nonempty
    /// import attributes. The standalone functions close their handles before
    /// returning; `Registry.openKey` and `Registry.createKey` instead return
    /// owned keys usable with `using` or `close()`.
    WinRegistry,
    /// `neo:yaml` — YAML text parsing.
    Yaml,
}

impl RuntimeModule {
    /// Every host module, in declaration order.
    ///
    /// The array grows when host modules are added. Iterate with
    /// `RuntimeModule::ALL.into_iter()` rather than naming its length in a
    /// type. To build an allowlist of all modules, use [`ModuleAllowlist::all`].
    pub const ALL: [Self; 19] = [
        Self::Age,
        Self::Ansi,
        Self::Assert,
        Self::Chars,
        Self::Crypto,
        Self::Dotenv,
        Self::Env,
        Self::Exec,
        Self::Fetch,
        Self::Fs,
        Self::Hash,
        Self::Path,
        Self::Process,
        Self::Semver,
        Self::StringBuilder,
        Self::Test,
        Self::Toml,
        Self::WinRegistry,
        Self::Yaml,
    ];

    /// The canonical `neo:` name of this module.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Age => "neo:age",
            Self::Ansi => "neo:ansi",
            Self::Assert => "neo:assert",
            Self::Chars => "neo:chars",
            Self::Crypto => "neo:crypto",
            Self::Dotenv => "neo:dotenv",
            Self::Env => "neo:env",
            Self::Exec => "neo:exec",
            Self::Fetch => "neo:fetch",
            Self::Fs => "neo:fs",
            Self::Hash => "neo:hash",
            Self::Path => "neo:path",
            Self::Process => "neo:process",
            Self::Semver => "neo:semver",
            Self::StringBuilder => "neo:string-builder",
            Self::Test => "neo:test",
            Self::Toml => "neo:toml",
            Self::WinRegistry => "neo:win-registry",
            Self::Yaml => "neo:yaml",
        }
    }

    /// Parse an exact `neo:` module name, including the deprecated
    /// `neo:winregistry` alias for [`Self::WinRegistry`].
    ///
    /// Matching is exact. A bare name such as `"fs"`, a different case, or any
    /// module this host does not implement returns `None`.
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "neo:age" => Self::Age,
            "neo:ansi" => Self::Ansi,
            "neo:assert" => Self::Assert,
            "neo:chars" => Self::Chars,
            "neo:crypto" => Self::Crypto,
            "neo:dotenv" => Self::Dotenv,
            "neo:env" => Self::Env,
            "neo:exec" => Self::Exec,
            "neo:fetch" => Self::Fetch,
            "neo:fs" => Self::Fs,
            "neo:hash" => Self::Hash,
            "neo:path" => Self::Path,
            "neo:process" => Self::Process,
            "neo:semver" => Self::Semver,
            "neo:string-builder" => Self::StringBuilder,
            "neo:test" => Self::Test,
            "neo:toml" => Self::Toml,
            "neo:win-registry" | "neo:winregistry" => Self::WinRegistry,
            "neo:yaml" => Self::Yaml,
            _ => return None,
        })
    }

    /// The internal bit for this module.
    ///
    /// Kept private: callers identify a module by value, never by a raw bit.
    const fn mask(self) -> u64 {
        1u64 << (self as u8)
    }
}

/// The exact `neo:` names supplied to [`ModuleAllowlist::from_names`] that this
/// host does not implement.
///
/// The list is returned instead of silently ignoring a typo, so an allowlist
/// cannot quietly enable nothing or the wrong module. Order follows the input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnknownModuleNames {
    names: Vec<String>,
}

impl UnknownModuleNames {
    /// The unknown names, in the order they were supplied.
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// True when no names were rejected, which cannot happen for a value
    /// produced by [`ModuleAllowlist::from_names`].
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

impl fmt::Display for UnknownModuleNames {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("unknown neo module name")?;
        if self.names.len() != 1 {
            formatter.write_str("s")?;
        }
        formatter.write_str(": ")?;
        for (index, name) in self.names.iter().enumerate() {
            if index != 0 {
                formatter.write_str(", ")?;
            }
            write!(formatter, "{name:?}")?;
        }
        Ok(())
    }
}

impl std::error::Error for UnknownModuleNames {}

/// An explicit set of host modules.
///
/// An allowlist denies every module not in it. An empty allowlist therefore
/// denies every `neo:` import while leaving entry-script evaluation, ordinary
/// relative JavaScript imports, and the always-present `console` untouched.
///
/// Build one from values ([`Self::from_modules`], [`Self::with`]) or from
/// exact names ([`Self::from_names`] for built-in names, or
/// [`Self::from_names_with`] when host extensions are registered). A value is
/// cheap to clone and contains no global mutable state.
///
/// [`Self::iter`] yields built-in modules only, because its item type cannot
/// represent an extension's name. Use [`Self::extension_names`] for registered
/// extension names. [`Self::len`] counts both kinds; [`Self::builtin_len`]
/// counts the built-in subset that [`Self::iter`] yields.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ModuleAllowlist {
    modules: BTreeSet<RuntimeModule>,
    extension_modules: BTreeSet<String>,
}

impl ModuleAllowlist {
    /// An allowlist that denies every host module.
    pub fn none() -> Self {
        Self::default()
    }

    /// An allowlist that accepts every built-in host module this build
    /// provides.
    ///
    /// [`RuntimeOptions::default`] is unrestricted and equivalent for built-in
    /// module access, but it is not the same configuration: an explicit
    /// [`Self::all`] allowlist remains a fixed set, while the default also
    /// admits host modules added by a later neo-js version. Registered
    /// extensions are not included; use [`Self::all_with_extensions`] for a
    /// fixed set that names them too.
    pub fn all() -> Self {
        Self::from_modules(Self::every())
    }

    /// An allowlist that accepts every built-in module and every exact module
    /// registered in `extensions`.
    pub fn all_with_extensions(extensions: &ExtensionRegistry) -> Self {
        let mut allowlist = Self::all();
        allowlist
            .extension_modules
            .extend(extensions.module_names().map(str::to_owned));
        allowlist
    }

    /// Build an allowlist from built-in module values.
    pub fn from_modules<I>(modules: I) -> Self
    where
        I: IntoIterator<Item = RuntimeModule>,
    {
        Self {
            modules: modules.into_iter().collect(),
            extension_modules: BTreeSet::new(),
        }
    }

    /// Build an allowlist from names accepted by [`RuntimeModule::from_name`].
    ///
    /// Every name is checked. If any name is unknown, the returned error lists
    /// all of them and no allowlist is produced, so a caller can report a typo
    /// instead of running with a silently smaller set. An empty iterator is
    /// valid and produces an empty allowlist.
    ///
    /// This constructor knows only built-in modules. An extension module name
    /// is reported as unknown here even when the name is registered; use
    /// [`Self::from_names_with`] to accept registered extension names.
    pub fn from_names<I, S>(names: I) -> Result<Self, UnknownModuleNames>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self::from_names_known(names, None)
    }

    /// Build an allowlist from built-in names (including aliases) and names
    /// registered in `extensions`.
    ///
    /// Built-in names must still be exact. An extension name is accepted only
    /// when `extensions` registered it. If any name is unknown, the returned
    /// error lists all of them and no allowlist is produced.
    pub fn from_names_with<I, S>(
        names: I,
        extensions: &ExtensionRegistry,
    ) -> Result<Self, UnknownModuleNames>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self::from_names_known(names, Some(extensions))
    }

    fn from_names_known<I, S>(
        names: I,
        extensions: Option<&ExtensionRegistry>,
    ) -> Result<Self, UnknownModuleNames>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut modules = BTreeSet::new();
        let mut extension_modules = BTreeSet::new();
        let mut unknown = Vec::new();
        for name in names {
            let name = name.as_ref();
            if let Some(module) = RuntimeModule::from_name(name) {
                modules.insert(module);
            } else if extensions.is_some_and(|extensions| extensions.contains_module(name)) {
                extension_modules.insert(name.to_owned());
            } else {
                unknown.push(name.to_owned());
            }
        }
        if unknown.is_empty() {
            Ok(Self {
                modules,
                extension_modules,
            })
        } else {
            Err(UnknownModuleNames { names: unknown })
        }
    }

    /// Add one built-in module and return the updated allowlist.
    pub fn with(mut self, module: RuntimeModule) -> Self {
        self.modules.insert(module);
        self
    }

    /// True when this allowlist contains the built-in `module`.
    pub fn contains(&self, module: RuntimeModule) -> bool {
        self.modules.contains(&module)
    }

    /// True when this allowlist names the exact extension module `name`.
    pub fn contains_extension_name(&self, name: &str) -> bool {
        self.extension_modules.contains(name)
    }

    /// The allowed built-in modules in a deterministic order.
    ///
    /// Extension module names are available from [`Self::extension_names`].
    pub fn iter(&self) -> impl Iterator<Item = RuntimeModule> + '_ {
        self.modules.iter().copied()
    }

    /// The allowed extension module names in a deterministic order.
    pub fn extension_names(&self) -> impl Iterator<Item = &str> + '_ {
        self.extension_modules.iter().map(String::as_str)
    }

    /// Number of allowed built-in and extension modules.
    pub fn len(&self) -> usize {
        self.modules.len() + self.extension_modules.len()
    }

    /// Number of allowed built-in modules, matching [`Self::iter`].
    pub fn builtin_len(&self) -> usize {
        self.modules.len()
    }

    /// True when no built-in or extension module is allowed.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(crate) fn extension_name_set(&self) -> &BTreeSet<String> {
        &self.extension_modules
    }

    /// Every built-in module this build provides, in declaration order.
    fn every() -> impl Iterator<Item = RuntimeModule> {
        RuntimeModule::ALL.into_iter()
    }
}

/// Runtime construction options.
///
/// [`RuntimeOptions::default`] (and [`Self::new`]) imposes no module allowlist,
/// which preserves the historical [`crate::NeoJs::new`] behavior exactly: every
/// module this build provides is importable when its permission is granted.
///
/// Use [`Self::with_allowlist`] or [`Self::modules`] to opt into an explicit,
/// fail-closed allowlist. With one, every `neo:` import is checked against it,
/// including dynamic imports and imports of a module that is already cached.
/// The check runs in the module resolver before QuickJS consults its cache, so
/// an identity cannot be reused to reach a disabled module.
///
/// The allowlist does not replace permissions. An allowlisted `neo:fs` still
/// requires [`crate::Permissions::filesystem`], and an allowlisted
/// `neo:process` still requires `process` or `stdio`. Conversely, a permission
/// alone never exposes a module that the allowlist omits.
/// Permissions can also authorize work outside the similarly named module:
/// `neo:hash.digestFile` reads files with filesystem permission even when
/// `neo:fs` is omitted, and `neo:fetch.download` writes files when filesystem
/// and network permissions are granted even when `neo:fs` is omitted.
/// Filesystem permission also enables relative module imports independently
/// of whether `neo:fs` is allowed. `neo:exec` can launch programs with the
/// host user's authority when process permission is granted, regardless of
/// the other permission flags or module selections.
///
/// Withdrawing a module also withdraws the global or native callback that only
/// it uses: the optional `neo:os` extension installs `os`, `neo:env` owns
/// `env`, `neo:fs` owns `fs`, `neo:hash` owns `__neoHash` (and
/// `__neoHashFile` with filesystem permission), and `neo:exec` owns the
/// `__neoExec*` callbacks. `neo:hash`'s `digestFile` remains part of
/// `neo:hash`; it additionally needs
/// [`crate::Permissions::filesystem`] but not the `neo:fs` module.
///
/// This is not a sandbox. It does not limit CPU or memory, and `console`
/// remains available in every runtime because it is the existing output
/// channel rather than an importable module.
///
/// Static host extensions are registered with [`Self::with_extension`] or
/// [`Self::with_extensions`]. Their exact names are checked by the same
/// allowlist and permission gates as built-in modules, and extension state is
/// per runtime. See [`crate::extension`].
#[derive(Clone, Debug, Default)]
pub struct RuntimeOptions {
    // `None` means unrestricted, the backward-compatible default. `Some` means
    // fail closed: a module absent from the set is denied.
    allowlist: Option<ModuleAllowlist>,
    // Static extension descriptors. Registration validates exact names and
    // rejects duplicates and built-in collisions before a runtime exists.
    extensions: ExtensionRegistry,
}

impl RuntimeOptions {
    /// Unrestricted options: every host module this build provides is subject
    /// only to its permission.
    pub fn new() -> Self {
        Self::default()
    }

    /// Options with an explicit allowlist.
    pub fn with_allowlist(allowlist: ModuleAllowlist) -> Self {
        Self {
            allowlist: Some(allowlist),
            extensions: ExtensionRegistry::new(),
        }
    }

    /// Register one host extension.
    ///
    /// The descriptor is validated as described by [`crate::extension`]. A
    /// rejected descriptor returns an error and no options value. Register
    /// extensions before calling [`Self::module_names`] for their exact names.
    pub fn with_extension(
        mut self,
        descriptor: ExtensionDescriptor,
    ) -> Result<Self, ExtensionRegistrationError> {
        self.extensions.register(descriptor)?;
        Ok(self)
    }

    /// Register several host extensions in order.
    ///
    /// A rejected descriptor returns an error and no options value; the
    /// partially built options are dropped. Use [`ExtensionRegistry::register`]
    /// directly when earlier successful registrations must be retained.
    pub fn with_extensions<I>(mut self, descriptors: I) -> Result<Self, ExtensionRegistrationError>
    where
        I: IntoIterator<Item = ExtensionDescriptor>,
    {
        for descriptor in descriptors {
            self.extensions.register(descriptor)?;
        }
        Ok(self)
    }

    /// The registered extension descriptors and exact module lookups.
    pub fn extensions(&self) -> &ExtensionRegistry {
        &self.extensions
    }

    /// Builder form of [`Self::with_allowlist`].
    ///
    /// ```
    /// use neojs_core::{RuntimeModule, RuntimeOptions};
    ///
    /// let options = RuntimeOptions::new().modules([RuntimeModule::Path, RuntimeModule::Hash]);
    /// assert!(options.is_module_allowed(RuntimeModule::Hash));
    /// assert!(!options.is_module_allowed(RuntimeModule::Fs));
    /// ```
    pub fn modules<I>(mut self, modules: I) -> Self
    where
        I: IntoIterator<Item = RuntimeModule>,
    {
        self.allowlist = Some(ModuleAllowlist::from_modules(modules));
        self
    }

    /// Builder form of [`ModuleAllowlist::from_names_with`].
    ///
    /// Every name must be accepted by [`RuntimeModule::from_name`] or
    /// registered by one of these options' extensions. If any name is unknown,
    /// the call returns an error and no options; nothing is silently dropped.
    pub fn module_names<I, S>(self, names: I) -> Result<Self, UnknownModuleNames>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let allowlist = ModuleAllowlist::from_names_with(names, &self.extensions)?;
        Ok(self.set_allowlist(allowlist))
    }

    fn set_allowlist(mut self, allowlist: ModuleAllowlist) -> Self {
        self.allowlist = Some(allowlist);
        self
    }

    /// The explicit allowlist, or `None` when unrestricted.
    pub fn allowlist(&self) -> Option<&ModuleAllowlist> {
        self.allowlist.as_ref()
    }

    /// True when built-in `module` may be constructed under these options.
    ///
    /// This answers only the allowlist question. The module's permission is a
    /// separate condition and is checked when it is imported. Extension
    /// modules are not represented by [`RuntimeModule`]; use
    /// [`Self::is_module_name_allowed`] for an exact name.
    pub fn is_module_allowed(&self, module: RuntimeModule) -> bool {
        self.allowlist
            .as_ref()
            .is_none_or(|allowlist| allowlist.contains(module))
    }

    /// True when the exact `neo:` name may be constructed under these options.
    ///
    /// Built-in names use [`Self::is_module_allowed`]. A registered extension
    /// name is allowed only when this options value has no explicit allowlist,
    /// or when the allowlist names it exactly. An unregistered name is false.
    pub fn is_module_name_allowed(&self, name: &str) -> bool {
        if let Some(module) = RuntimeModule::from_name(name) {
            return self.is_module_allowed(module);
        }
        self.extensions.contains_module(name)
            && self
                .allowlist
                .as_ref()
                .is_none_or(|allowlist| allowlist.contains_extension_name(name))
    }

    /// The copyable built-in bit set plus the exact extension names used by the
    /// loader and by extension initialization.
    pub(crate) fn allowed_modules(&self) -> AllowedModules {
        let (bits, extension_names) = match self.allowlist.as_ref() {
            None => (None, None),
            Some(allowlist) => (
                Some(
                    allowlist
                        .iter()
                        .fold(0, |bits, module| bits | module.mask()),
                ),
                Some(Rc::new(allowlist.extension_name_set().clone())),
            ),
        };
        AllowedModules {
            bits,
            extension_names,
        }
    }
}

/// Allocate-once form of an optional allowlist.
///
/// `None` is unrestricted; `Some` is explicit and fail closed. Kept crate
/// private so callers cannot set a raw bit or name and bypass registration and
/// name validation.
#[derive(Clone, Debug)]
pub(crate) struct AllowedModules {
    bits: Option<u64>,
    extension_names: Option<Rc<BTreeSet<String>>>,
}

impl AllowedModules {
    /// True when built-in `module` may be constructed.
    pub(crate) fn allows(&self, module: RuntimeModule) -> bool {
        self.bits.is_none_or(|bits| bits & module.mask() != 0)
    }

    /// True when the exact registered extension `name` may be constructed.
    ///
    /// Callers check the registry first, so an unknown name never reaches this
    /// function. `None` names are unrestricted, matching the default runtime.
    pub(crate) fn allows_name(&self, name: &str) -> bool {
        if let Some(module) = RuntimeModule::from_name(name) {
            return self.allows(module);
        }
        self.extension_names
            .as_ref()
            .is_none_or(|names| names.contains(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_are_exact() {
        for module in RuntimeModule::ALL {
            assert_eq!(RuntimeModule::from_name(module.name()), Some(module));
        }
        assert_eq!(RuntimeModule::from_name("fs"), None);
        assert_eq!(RuntimeModule::from_name("neo:Hash"), None);
        assert_eq!(RuntimeModule::from_name("neo:unknown"), None);
    }

    #[test]
    fn unknown_names_are_all_reported_exactly() {
        let error =
            ModuleAllowlist::from_names(["neo:fs", "neo:nope", "fs", "neo:Hash"]).unwrap_err();
        assert_eq!(error.names(), ["neo:nope", "fs", "neo:Hash"]);
        assert_eq!(
            error.to_string(),
            "unknown neo module names: \"neo:nope\", \"fs\", \"neo:Hash\""
        );
        assert_eq!(
            ModuleAllowlist::from_names(["neo:nope"])
                .unwrap_err()
                .to_string(),
            "unknown neo module name: \"neo:nope\""
        );
    }

    #[test]
    fn empty_allowlist_is_valid_and_denies_everything() {
        let allowlist = ModuleAllowlist::from_names(std::iter::empty::<&str>()).unwrap();
        assert!(allowlist.is_empty());
        assert_eq!(allowlist.len(), 0);
        let allowed = RuntimeOptions::with_allowlist(allowlist).allowed_modules();
        for module in RuntimeModule::ALL {
            assert!(!allowed.allows(module));
        }
    }

    #[test]
    fn default_options_allow_everything() {
        let allowed = RuntimeOptions::default().allowed_modules();
        for module in RuntimeModule::ALL {
            assert!(allowed.allows(module));
        }
    }

    #[test]
    fn explicit_allowlist_is_fail_closed() {
        let options = RuntimeOptions::new().modules([RuntimeModule::Hash, RuntimeModule::Path]);
        let allowed = options.allowed_modules();
        assert!(allowed.allows(RuntimeModule::Hash));
        assert!(allowed.allows(RuntimeModule::Path));
        assert!(!allowed.allows(RuntimeModule::Fs));
        assert!(RuntimeOptions::default().is_module_allowed(RuntimeModule::Fs));
        assert!(!options.is_module_allowed(RuntimeModule::Fs));
    }
}
