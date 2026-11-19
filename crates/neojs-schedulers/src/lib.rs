//! Optional, pure scheduler configuration text generators for `neojs-core`.
//!
//! This crate supplies the exact `neo:os/schedulers/systemd`,
//! `neo:os/schedulers/windows`, and deprecated `neo:scheduler` modules through
//! [`neojs_core::extension`]. `neojs-core` itself does not compile these
//! generators, does not recognize their names as built-in modules, and has no
//! scheduler dependency. Registering this extension restores the previous
//! import behavior; the CLI does so explicitly.
//!
//! The generators are pure, synchronous JavaScript text generation. No
//! function reads or writes a file, creates a directory, changes permissions,
//! starts a process, runs a shell, calls a service manager, registers a task,
//! or enables anything. The module needs no permission flag and installs
//! nothing on the global object. The returned strings are inert until a caller
//! with separate authority writes and installs them.
//!
//! # Registration
//!
//! Register [`schedulers_extension`] on the runtime options and name the exact
//! modules in any explicit allowlist. Every module uses
//! [`neojs_core::ExtensionPermission::None`]; the runtime allowlist still
//! decides whether that exact name exists. The three specifiers are admitted
//! independently: naming [`SYSTEMD_MODULE_NAME`] does not admit
//! [`WINDOWS_MODULE_NAME`] or the legacy alias, and naming
//! [`LEGACY_MODULE_NAME`] admits only the legacy name.
//!
//! ```rust
//! use neojs_core::{NeoJs, Permissions, RuntimeOptions};
//! use neojs_schedulers::{
//!     MODULE_NAMES, SYSTEMD_MODULE_NAME, WINDOWS_MODULE_NAME, with_schedulers,
//! };
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let options = with_schedulers(RuntimeOptions::new())?
//!     .module_names(MODULE_NAMES)?;
//! let runtime = NeoJs::new_with_options(Permissions::default(), options)?;
//! runtime.run_module(
//!     "main.js",
//!     "import {systemdUnits} from 'neo:os/schedulers/systemd';
//!      import {windowsTask} from 'neo:os/schedulers/windows';
//!      import {systemdUnits as legacyUnits} from 'neo:scheduler';
//!      systemdUnits({name: 'a', program: '/bin/true', schedule: {kind: 'daily', at: '09:30'}});
//!      windowsTask({name: 'A', startDate: '2026-11-18', program: 'C:\\\\a.exe',
//!                   schedule: {kind: 'daily', at: '09:30'}});
//!      legacyUnits({name: 'a', program: '/bin/true', schedule: {kind: 'daily', at: '09:30'}});",
//! )?;
//! # Ok(())
//! # }
//! ```
//!
//! The canonical specifiers export one target each:
//!
//! * [`SYSTEMD_MODULE_NAME`] exports `systemdService`, `systemdTimer`,
//!   `systemdUnits`, `MAX_SYSTEMD_LINE_BYTES`, and `SchedulerError`.
//! * [`WINDOWS_MODULE_NAME`] exports `windowsTask`, `WINDOWS_XML_ENCODING`,
//!   `MAX_WINDOWS_COMMAND_LINE`, and `SchedulerError`.
//! * [`LEGACY_MODULE_NAME`] is deprecated and exports both surfaces through
//!   that name only.
//!
//! Each specifier has its own module instance and its own `SchedulerError`
//! constructor. Two specifiers never share that constructor, and a static and
//! dynamic import of the same specifier share one instance.
//!
//! # Migration from the built-in modules
//!
//! `neo:scheduler`, `neo:os/schedulers/systemd`, and
//! `neo:os/schedulers/windows` were previously [`neojs_core::RuntimeModule`]
//! variants compiled into `neojs-core`. Those variants no longer exist, so
//! code that named `RuntimeModule::Scheduler`, `RuntimeModule::SchedulerSystemd`,
//! or `RuntimeModule::SchedulerWindows` does not compile. A core-only runtime
//! no longer provides any of the three names. Replace the built-in selections
//! with exact extension names:
//!
//! ```rust,compile_fail
//! # use neojs_core::{NeoJs, Permissions, RuntimeModule, RuntimeOptions};
//! # fn before() -> Result<NeoJs, Box<dyn std::error::Error>> {
//! // Before: this no longer compiles, because the variants are gone.
//! let js = NeoJs::new_with_options(
//!     Permissions::default(),
//!     RuntimeOptions::new().modules([RuntimeModule::SchedulerSystemd]),
//! )?;
//! # Ok(js)
//! # }
//! ```
//!
//! ```rust
//! # use neojs_core::{NeoJs, Permissions, RuntimeOptions};
//! # use neojs_schedulers::{SYSTEMD_MODULE_NAME, with_schedulers};
//! # fn after() -> Result<NeoJs, Box<dyn std::error::Error>> {
//! let options = with_schedulers(RuntimeOptions::new())?
//!     .module_names([SYSTEMD_MODULE_NAME])?;
//! let js = NeoJs::new_with_options(Permissions::default(), options)?;
//! # Ok(js)
//! # }
//! ```
//!
//! [`neojs_core::ModuleAllowlist::all`] contains built-in modules only, so it
//! no longer admits any scheduler specifier. Use
//! [`neojs_core::ModuleAllowlist::all_with_extensions`] or add the exact
//! constants from this crate. [`RuntimeOptions::default`] remains unrestricted
//! and admits a registered extension. A dependency on the `neojs` CLI is
//! unchanged: it registers this extension itself.

use neojs_core::rquickjs::{Ctx, Error, Module, Result as JsResult, module::Declared};
use neojs_core::{ExtensionDescriptor, ExtensionPermission, RuntimeOptions};

/// The exact canonical `neo:` name for the systemd generator module.
pub const SYSTEMD_MODULE_NAME: &str = "neo:os/schedulers/systemd";
/// The exact canonical `neo:` name for the Windows Task Scheduler generator
/// module.
pub const WINDOWS_MODULE_NAME: &str = "neo:os/schedulers/windows";
/// The deprecated combined module name.
///
/// Selecting this name admits both generators through this name only; it does
/// not admit either canonical name.
pub const LEGACY_MODULE_NAME: &str = "neo:scheduler";

/// Every exact module name supplied by this crate.
pub const MODULE_NAMES: [&str; 3] = [SYSTEMD_MODULE_NAME, WINDOWS_MODULE_NAME, LEGACY_MODULE_NAME];

// Give each specifier its own export surface, without importing another public
// specifier or introducing an importable internal module. The legacy surface
// intentionally exports both sets of helpers.
macro_rules! scheduler_module {
    ($($export:ident),+ $(,)?) => {
        concat!(
            include_str!("scheduler.js"),
            "\nexport { SchedulerError, ",
            $(stringify!($export), ", ",)+
            "};\n",
        )
    };
}

const SYSTEMD_MODULE: &str = scheduler_module!(
    systemdService,
    systemdTimer,
    systemdUnits,
    MAX_SYSTEMD_LINE_BYTES
);
const WINDOWS_MODULE: &str =
    scheduler_module!(windowsTask, WINDOWS_XML_ENCODING, MAX_WINDOWS_COMMAND_LINE);
const LEGACY_MODULE: &str = scheduler_module!(
    systemdService,
    systemdTimer,
    systemdUnits,
    MAX_SYSTEMD_LINE_BYTES,
    windowsTask,
    WINDOWS_XML_ENCODING,
    MAX_WINDOWS_COMMAND_LINE,
);

/// Build the static extension descriptor that supplies all three specifiers.
///
/// The descriptor gates every module on [`ExtensionPermission::None`] and adds
/// no initialization hook, so it installs no global and keeps no per-runtime
/// state. Register it with [`RuntimeOptions::with_extension`],
/// [`RuntimeOptions::with_extensions`], or [`with_schedulers`].
///
/// Registering the same extension twice is rejected by core like any duplicate
/// extension or module name.
pub fn schedulers_extension() -> ExtensionDescriptor {
    ExtensionDescriptor::new("neojs-schedulers", load)
        .with_module(SYSTEMD_MODULE_NAME, ExtensionPermission::None)
        .with_module(WINDOWS_MODULE_NAME, ExtensionPermission::None)
        .with_module(LEGACY_MODULE_NAME, ExtensionPermission::None)
}

/// Register the scheduler extension on `options` and return the updated
/// options.
///
/// This is [`RuntimeOptions::with_extension`] with [`schedulers_extension`]. It
/// grants no permission, because these modules need none, and does not change
/// an explicit allowlist; call [`RuntimeOptions::module_names`] with the exact
/// constants from this crate when the options carry one. A duplicate
/// registration is an error and produces no options value.
pub fn with_schedulers(
    options: RuntimeOptions,
) -> Result<RuntimeOptions, neojs_core::ExtensionRegistrationError> {
    options.with_extension(schedulers_extension())
}

/// Declare exactly the module requested by the loader.
fn load<'js>(
    ctx: &Ctx<'js>,
    _state: &mut dyn std::any::Any,
    name: &str,
) -> JsResult<Module<'js, Declared>> {
    let source = match name {
        SYSTEMD_MODULE_NAME => SYSTEMD_MODULE,
        WINDOWS_MODULE_NAME => WINDOWS_MODULE,
        LEGACY_MODULE_NAME => LEGACY_MODULE,
        _ => {
            return Err(Error::new_loading_message(
                name,
                "unknown scheduler module name",
            ));
        }
    };
    Module::declare(ctx.clone(), name, source)
}
