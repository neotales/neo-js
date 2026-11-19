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

use std::{collections::BTreeSet, fmt};

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
    /// `neo:os` — target platform, architecture, and optional OS-release
    /// metadata. It needs no permission flag, but discloses host OS metadata.
    Os,
    /// `neo:path` — lexical path helpers and a global file-only `URL` class.
    /// Reading the runtime working directory also requires admitted
    /// filesystem or process/stdio authority.
    Path,
    /// `neo:process` — runtime streams and working directory; also requires
    /// [`crate::Permissions::process`] or [`crate::Permissions::stdio`].
    Process,
    /// `neo:scheduler` — deprecated combined scheduler compatibility module.
    ///
    /// Selecting this entry allows both generators through the legacy name
    /// only. It does not allow imports of the two target-specific modules.
    /// For new allowlists, select [`Self::SchedulerSystemd`] and/or
    /// [`Self::SchedulerWindows`] instead.
    Scheduler,
    /// `neo:semver` — semantic-version parsing, formatting, ranges, and
    /// comparison.
    Semver,
    /// `neo:string-builder` — a small string builder.
    StringBuilder,
    /// `neo:test` — the synchronous test registry.
    Test,
    /// `neo:toml` — TOML text parsing.
    Toml,
    /// `neo:winregistry` — Windows Registry access; also requires
    /// [`crate::Permissions::registry`].
    WinRegistry,
    /// `neo:yaml` — YAML text parsing.
    Yaml,
    /// `neo:os/schedulers/systemd` — systemd unit configuration text generation.
    SchedulerSystemd,
    /// `neo:os/schedulers/windows` — Windows Task Scheduler XML text generation.
    SchedulerWindows,
    /// `neo:sqlite` — in-memory databases; also requires [`crate::Permissions::sqlite`].
    Sqlite,
}

impl RuntimeModule {
    /// Every host module, in declaration order.
    ///
    /// The array grows when host modules are added. Iterate with
    /// `RuntimeModule::ALL.into_iter()` rather than naming its length in a
    /// type. To build an allowlist of all modules, use [`ModuleAllowlist::all`].
    pub const ALL: [Self; 24] = [
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
        Self::Os,
        Self::Path,
        Self::Process,
        Self::Scheduler,
        Self::Semver,
        Self::StringBuilder,
        Self::Test,
        Self::Toml,
        Self::WinRegistry,
        Self::Yaml,
        Self::SchedulerSystemd,
        Self::SchedulerWindows,
        Self::Sqlite,
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
            Self::Os => "neo:os",
            Self::Path => "neo:path",
            Self::Process => "neo:process",
            Self::Scheduler => "neo:scheduler",
            Self::SchedulerSystemd => "neo:os/schedulers/systemd",
            Self::SchedulerWindows => "neo:os/schedulers/windows",
            Self::Semver => "neo:semver",
            Self::StringBuilder => "neo:string-builder",
            Self::Test => "neo:test",
            Self::Toml => "neo:toml",
            Self::WinRegistry => "neo:winregistry",
            Self::Yaml => "neo:yaml",
            Self::Sqlite => "neo:sqlite",
        }
    }

    /// Parse a canonical `neo:` module name.
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
            "neo:os" => Self::Os,
            "neo:path" => Self::Path,
            "neo:process" => Self::Process,
            "neo:scheduler" => Self::Scheduler,
            "neo:os/schedulers/systemd" => Self::SchedulerSystemd,
            "neo:os/schedulers/windows" => Self::SchedulerWindows,
            "neo:semver" => Self::Semver,
            "neo:string-builder" => Self::StringBuilder,
            "neo:test" => Self::Test,
            "neo:toml" => Self::Toml,
            "neo:winregistry" => Self::WinRegistry,
            "neo:yaml" => Self::Yaml,
            "neo:sqlite" => Self::Sqlite,
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
/// exact names ([`Self::from_names`], which rejects unknown names). A value is
/// cheap to clone and contains no global mutable state.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ModuleAllowlist {
    modules: BTreeSet<RuntimeModule>,
}

impl ModuleAllowlist {
    /// An allowlist that denies every host module.
    pub fn none() -> Self {
        Self::default()
    }

    /// An allowlist that accepts every host module this build provides.
    ///
    /// [`RuntimeOptions::default`] is unrestricted and equivalent for module
    /// access, but it is not the same configuration: an explicit
    /// [`Self::all`] allowlist remains a fixed set, while the default also
    /// admits host modules added by a later neo-js version.
    pub fn all() -> Self {
        Self::from_modules(Self::every())
    }

    /// Build an allowlist from module values.
    pub fn from_modules<I>(modules: I) -> Self
    where
        I: IntoIterator<Item = RuntimeModule>,
    {
        Self {
            modules: modules.into_iter().collect(),
        }
    }

    /// Build an allowlist from exact canonical `neo:` names.
    ///
    /// Every name is checked. If any name is unknown, the returned error lists
    /// all of them and no allowlist is produced, so a caller can report a typo
    /// instead of running with a silently smaller set. An empty iterator is
    /// valid and produces an empty allowlist.
    pub fn from_names<I, S>(names: I) -> Result<Self, UnknownModuleNames>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut modules = BTreeSet::new();
        let mut unknown = Vec::new();
        for name in names {
            let name = name.as_ref();
            match RuntimeModule::from_name(name) {
                Some(module) => {
                    modules.insert(module);
                }
                None => unknown.push(name.to_owned()),
            }
        }
        if unknown.is_empty() {
            Ok(Self { modules })
        } else {
            Err(UnknownModuleNames { names: unknown })
        }
    }

    /// Add one module and return the updated allowlist.
    pub fn with(mut self, module: RuntimeModule) -> Self {
        self.modules.insert(module);
        self
    }

    /// True when this allowlist contains `module`.
    pub fn contains(&self, module: RuntimeModule) -> bool {
        self.modules.contains(&module)
    }

    /// The allowed modules in a deterministic order.
    pub fn iter(&self) -> impl Iterator<Item = RuntimeModule> + '_ {
        self.modules.iter().copied()
    }

    /// Number of allowed modules.
    pub fn len(&self) -> usize {
        self.modules.len()
    }

    /// True when no module is allowed.
    pub fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }

    /// Every module this build provides, in declaration order.
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
/// it uses: `neo:os` owns `os`, `neo:env` owns `env`, `neo:fs` owns `fs`,
/// `neo:hash` owns `__neoHash` (and `__neoHashFile` with filesystem
/// permission), and `neo:exec` owns the `__neoExec*` callbacks. `neo:hash`'s
/// `digestFile` remains part of `neo:hash`; it additionally needs
/// [`crate::Permissions::filesystem`] but not the `neo:fs` module.
///
/// This is not a sandbox. It does not limit CPU or memory, and `console`
/// remains available in every runtime because it is the existing output
/// channel rather than an importable module.
#[derive(Clone, Debug, Default)]
pub struct RuntimeOptions {
    // `None` means unrestricted, the backward-compatible default. `Some` means
    // fail closed: a module absent from the set is denied.
    allowlist: Option<ModuleAllowlist>,
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
        }
    }

    /// Builder form of [`Self::with_allowlist`].
    ///
    /// ```
    /// use neo_js::{RuntimeModule, RuntimeOptions};
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

    /// Builder form of [`ModuleAllowlist::from_names`].
    ///
    /// Every name must be a canonical `neo:` name. If any name is unknown, the
    /// call returns an error and no options; nothing is silently dropped.
    pub fn module_names<I, S>(self, names: I) -> Result<Self, UnknownModuleNames>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Ok(Self::with_allowlist(ModuleAllowlist::from_names(names)?))
    }

    /// The explicit allowlist, or `None` when unrestricted.
    pub fn allowlist(&self) -> Option<&ModuleAllowlist> {
        self.allowlist.as_ref()
    }

    /// True when `module` may be constructed under these options.
    ///
    /// This answers only the allowlist question. The module's permission is a
    /// separate condition and is checked when it is imported.
    pub fn is_module_allowed(&self, module: RuntimeModule) -> bool {
        self.allowlist
            .as_ref()
            .is_none_or(|allowlist| allowlist.contains(module))
    }

    /// The copyable bit set used by the loader.
    pub(crate) fn allowed_modules(&self) -> AllowedModules {
        AllowedModules {
            bits: self.allowlist.as_ref().map(|allowlist| {
                allowlist
                    .iter()
                    .fold(0, |bits, module| bits | module.mask())
            }),
        }
    }
}

/// Copyable, allocation-free form of an optional allowlist.
///
/// `None` is unrestricted; `Some` is explicit and fail closed. Kept crate
/// private so callers cannot set a raw bit and bypass name validation.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AllowedModules {
    bits: Option<u64>,
}

impl AllowedModules {
    /// True when `module` may be constructed.
    pub(crate) fn allows(self, module: RuntimeModule) -> bool {
        self.bits.is_none_or(|bits| bits & module.mask() != 0)
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
