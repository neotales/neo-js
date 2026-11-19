//! Optional, bounded, read-only operating-system metadata for `neojs-core`.
//!
//! This crate supplies the exact `neo:os` module and the `os` global through
//! [`neojs_core::extension`]. `neojs-core` itself does not compile this module,
//! does not install the `os` global, and contains no built-in `neo:os`
//! implementation. Registering this extension restores the previous behavior;
//! the CLI does so explicitly.
//!
//! Linux reads os-release, Windows reads fixed OS identity values from the
//! registry, and macOS queries its product version. No probe runs a command or
//! takes a script-provided path or key.
//!
//! # Registration
//!
//! Register [`os_extension`] on the runtime options and name the exact module
//! in any explicit allowlist. `neo:os` needs no permission flag:
//! [`ExtensionPermission::None`] still means the runtime allowlist must admit
//! the name.
//!
//! ```rust
//! use neojs_core::{NeoJs, Permissions, RuntimeOptions};
//! use neojs_os::{MODULE_NAME, with_os};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let options = with_os(RuntimeOptions::new())?.module_names([MODULE_NAME])?;
//! let runtime = NeoJs::new_with_options(Permissions::default(), options)?;
//! runtime.run_module(
//!     "main.js",
//!     "import {platform} from 'neo:os'; if (platform !== os.platform) throw Error('os');",
//! )?;
//! # Ok(())
//! # }
//! ```
//!
//! The `os` global is installed by the extension's initialization hook, which
//! core runs before user code only when the allowlist admits at least one of
//! the extension's modules. With an explicit allowlist that omits `neo:os`,
//! neither the module nor the global exists. [`RuntimeOptions::default`] stays
//! unrestricted and admits a registered extension.
//!
//! # Migration from the built-in module
//!
//! `neo:os` was previously a [`neojs_core::RuntimeModule`] variant compiled
//! into `neojs-core`, and core installed the `os` global itself. That variant
//! no longer exists, so code that named `RuntimeModule::Os` does not compile,
//! and a core-only runtime no longer has `neo:os` or the global. Migrate as
//! follows:
//!
//! ```rust,compile_fail
//! # use neojs_core::{NeoJs, Permissions, RuntimeModule, RuntimeOptions};
//! # fn before() -> Result<NeoJs, Box<dyn std::error::Error>> {
//! // Before: this no longer compiles, because RuntimeModule::Os is gone.
//! let js = NeoJs::new_with_options(
//!     Permissions::default(),
//!     RuntimeOptions::new().modules([RuntimeModule::Os]),
//! )?;
//! # Ok(js)
//! # }
//! ```
//!
//! Replace the built-in selection with an exact extension name:
//!
//! ```rust
//! # use neojs_core::{NeoJs, Permissions, RuntimeOptions};
//! # use neojs_os::{MODULE_NAME, with_os};
//! # fn after() -> Result<NeoJs, Box<dyn std::error::Error>> {
//! let options = with_os(RuntimeOptions::new())?.module_names([MODULE_NAME])?;
//! let js = NeoJs::new_with_options(Permissions::default(), options)?;
//! # Ok(js)
//! # }
//! ```
//!
//! [`neojs_core::ModuleAllowlist::all`] contains built-in modules only, so it
//! no longer admits `neo:os`. Use
//! [`neojs_core::ModuleAllowlist::all_with_extensions`] or add
//! [`MODULE_NAME`] explicitly. A dependency on the `neojs` CLI is unchanged:
//! it registers this extension itself, so `--allow-*` flags and the `os`
//! global behave as before. The `neo:os/schedulers/*` modules remain built
//! into core and are unaffected.

use std::any::Any;

use neojs_core::rquickjs::{Ctx, Error, Module, Object, Result as JsResult, module::Declared};
use neojs_core::{
    ExtensionDescriptor, ExtensionPermission, ExtensionRegistrationError, RuntimeOptions,
};

#[cfg(any(target_os = "macos", test))]
mod macos;
#[cfg(any(windows, test))]
mod windows;

#[cfg(any(target_os = "linux", test))]
use std::{fs::OpenOptions, io::Read, path::Path};

#[cfg(any(target_os = "linux", test))]
const MAX_OS_RELEASE_BYTES: u64 = 16 * 1024;
#[cfg(any(target_os = "linux", test))]
const MAX_VALUE_BYTES: usize = 4096;

/// Optional descriptive host metadata. `version` identifies the OS release
/// or Windows build, never a Unix kernel release.
#[derive(Clone, Default)]
struct Metadata {
    version: Option<String>,
    codename: Option<String>,
    variant: Option<String>,
}

/// The exact `neo:` module name supplied by this crate.
///
/// Use this constant when constructing an explicit
/// [`neojs_core::ModuleAllowlist`] or calling
/// [`RuntimeOptions::module_names`], so the name is not retyped.
pub const MODULE_NAME: &str = "neo:os";

/// Build the static extension descriptor that supplies `neo:os` and `os`.
///
/// The descriptor gates the module on [`ExtensionPermission::None`], takes one
/// private metadata snapshot and installs the `os` global during runtime
/// construction, and declares `neo:os` from that snapshot when it is imported.
/// Register it with [`RuntimeOptions::with_extension`],
/// [`RuntimeOptions::with_extensions`], or [`with_os`].
///
/// Registering the same extension twice is rejected by core like any duplicate
/// extension or module name.
pub fn os_extension() -> ExtensionDescriptor {
    ExtensionDescriptor::new("neojs-os", load)
        .with_module(MODULE_NAME, ExtensionPermission::None)
        .with_init(init)
}

/// Register `neo:os` on `options` and return the updated options.
///
/// This is [`RuntimeOptions::with_extension`] with [`os_extension`]. It grants
/// no permission, because `neo:os` needs none, and does not change an explicit
/// allowlist; call [`RuntimeOptions::module_names`] with [`MODULE_NAME`] when
/// the options carry one. A duplicate registration is an error and produces no
/// options value.
pub fn with_os(options: RuntimeOptions) -> Result<RuntimeOptions, ExtensionRegistrationError> {
    options.with_extension(os_extension())
}

/// Per-runtime metadata captured once, before user code can run.
struct OsExtensionState {
    metadata: Metadata,
}

/// Take the private snapshot and install the writable `os` global.
fn init(ctx: &Ctx<'_>, _allowed: &[&str]) -> JsResult<Box<dyn Any>> {
    let metadata = metadata();
    let os = Object::new(ctx.clone())?;
    os.set("platform", std::env::consts::OS)?;
    os.set("arch", std::env::consts::ARCH)?;
    os.set("version", metadata.version.clone())?;
    os.set("codename", metadata.codename.clone())?;
    os.set("variant", metadata.variant.clone())?;
    ctx.globals().set("os", os)?;
    Ok(Box::new(OsExtensionState { metadata }))
}

/// Declare the module from the snapshot installed by [`init`].
fn load<'js>(ctx: &Ctx<'js>, state: &mut dyn Any, name: &str) -> JsResult<Module<'js, Declared>> {
    let state = state
        .downcast_ref::<OsExtensionState>()
        .ok_or_else(|| Error::new_loading_message(name, "operating-system state is unavailable"))?;
    Module::declare(ctx.clone(), name, module_source(&state.metadata))
}

/// Collects optional, bounded host metadata without invoking external tools.
fn metadata() -> Metadata {
    #[cfg(target_os = "linux")]
    {
        metadata_from_paths(
            Path::new("/etc/os-release"),
            Path::new("/usr/lib/os-release"),
        )
    }
    #[cfg(windows)]
    {
        windows::metadata()
    }
    #[cfg(target_os = "macos")]
    {
        macos::metadata()
    }
    #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
    {
        Metadata::default()
    }
}

/// Reads the primary file and only consults the fallback when the primary is
/// absent. A present but unreadable, malformed, or oversized primary is not
/// merged with or replaced by the fallback.
#[cfg(any(target_os = "linux", test))]
fn metadata_from_paths(primary: &Path, fallback: &Path) -> Metadata {
    match read_release_file(primary) {
        Ok(input) => metadata_from_text(&input),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => read_release_file(fallback)
            .map(|input| metadata_from_text(&input))
            .unwrap_or_default(),
        Err(_) => Metadata::default(),
    }
}

#[cfg(any(target_os = "linux", test))]
fn metadata_from_text(input: &str) -> Metadata {
    let values = parse_os_release(input);
    Metadata {
        version: nonempty(&values, "VERSION_ID"),
        codename: nonempty(&values, "VERSION_CODENAME")
            .or_else(|| nonempty(&values, "UBUNTU_CODENAME")),
        variant: nonempty(&values, "VARIANT_ID").or_else(|| nonempty(&values, "VARIANT")),
    }
}

#[cfg(any(target_os = "linux", test))]
fn nonempty(values: &std::collections::HashMap<String, String>, key: &str) -> Option<String> {
    values.get(key).filter(|value| !value.is_empty()).cloned()
}

#[cfg(any(target_os = "linux", test))]
fn read_release_file(path: &Path) -> std::io::Result<String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    let mut file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "OS release source is not a regular file",
        ));
    }
    // Read one byte beyond the cap to distinguish an exactly capped file from
    // a truncated oversized source. Never expose partial metadata.
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_OS_RELEASE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_OS_RELEASE_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "OS release source exceeds the size limit",
        ));
    }
    String::from_utf8(bytes).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "OS release source is not valid UTF-8",
        )
    })
}

/// Parses the supported os-release assignments. Values are data, never shell
/// expansions. Invalid assignments do not replace an earlier valid value.
#[cfg(any(target_os = "linux", test))]
fn parse_os_release(input: &str) -> std::collections::HashMap<String, String> {
    let mut values = std::collections::HashMap::new();
    for line in input.lines() {
        let Some((key, raw)) = line.split_once('=') else {
            continue;
        };
        if line.starts_with('#') {
            continue;
        }
        if !matches!(
            key,
            "VERSION_ID" | "VERSION_CODENAME" | "UBUNTU_CODENAME" | "VARIANT_ID" | "VARIANT"
        ) {
            continue;
        }
        if let Some(value) = parse_value(raw) {
            if value.len() <= MAX_VALUE_BYTES && (key != "VARIANT_ID" || valid_variant_id(&value)) {
                values.insert(key.to_owned(), value);
            }
        }
    }
    values
}

/// Unlike the version fields, VARIANT_ID permits only lowercase ASCII
/// letters, digits, dots, underscores, and hyphens, even when quoted.
#[cfg(any(target_os = "linux", test))]
fn valid_variant_id(value: &str) -> bool {
    value.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
    })
}

#[cfg(any(target_os = "linux", test))]
fn parse_value(raw: &str) -> Option<String> {
    if raw.is_empty() {
        return Some(String::new());
    }
    let mut chars = raw.chars().peekable();
    let first = *chars.peek()?;
    if first == '\'' || first == '"' {
        chars.next();
        let mut value = String::new();
        loop {
            let ch = chars.next()?;
            if ch == first {
                return chars.next().is_none().then_some(value);
            }
            if ch == '\\' && first == '"' {
                let escaped = chars.next()?;
                if matches!(escaped, '\\' | '"' | '$' | '`') {
                    value.push(escaped);
                } else {
                    // Shell double quotes preserve a backslash before
                    // characters for which backslash has no special meaning.
                    value.push('\\');
                    value.push(escaped);
                }
            } else {
                value.push(ch);
            }
        }
    }

    let mut value = String::new();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            value.push(chars.next()?);
        } else if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '+' | '~' | '^') {
            value.push(ch);
        } else {
            return None;
        }
    }
    Some(value)
}

/// Generates the module from the runtime's private metadata snapshot. JSON
/// string literals safely encode host text and `null ?? undefined` preserves
/// the public undefined-for-unavailable contract.
fn module_source(metadata: &Metadata) -> String {
    fn literal(value: &Option<String>) -> String {
        serde_json::to_string(value).expect("serializing OS metadata cannot fail")
    }

    format!(
        "export const platform = {}, arch = {}, version = {} ?? undefined, \
         codename = {} ?? undefined, variant = {} ?? undefined;",
        serde_json::to_string(std::env::consts::OS)
            .expect("serializing a platform constant cannot fail"),
        serde_json::to_string(std::env::consts::ARCH)
            .expect("serializing an architecture constant cannot fail"),
        literal(&metadata.version),
        literal(&metadata.codename),
        literal(&metadata.variant)
    )
}

#[cfg(test)]
mod tests {
    use super::{metadata_from_paths, metadata_from_text, parse_os_release};
    use tempfile::tempdir;

    #[test]
    fn parses_quoted_linux_release_fields_without_shell_expansion() {
        let values = parse_os_release(
            "NAME=ignored\nVERSION_ID=\"24.04\"\nVERSION_CODENAME=noble\nVARIANT='Server Edition'\nOTHER=ignored\n",
        );
        assert_eq!(values.get("VERSION_ID").map(String::as_str), Some("24.04"));
        assert_eq!(
            values.get("VERSION_CODENAME").map(String::as_str),
            Some("noble")
        );
        assert_eq!(
            values.get("VARIANT").map(String::as_str),
            Some("Server Edition")
        );
        assert!(!values.contains_key("NAME"));
    }

    #[test]
    fn unquoted_os_release_values_accept_documented_punctuation() {
        let values =
            parse_os_release("VERSION_ID=1.2+build~rc^x\nVERSION_CODENAME=alpha+beta~rc^x\n");
        assert_eq!(
            values.get("VERSION_ID").map(String::as_str),
            Some("1.2+build~rc^x")
        );
        assert_eq!(
            values.get("VERSION_CODENAME").map(String::as_str),
            Some("alpha+beta~rc^x")
        );
    }

    #[test]
    fn variant_id_must_be_valid_before_it_takes_precedence_over_variant() {
        for (raw, expected) in [
            ("server.2_edge-node", "server.2_edge-node"),
            ("\"server.2_edge-node\"", "server.2_edge-node"),
            ("'server.2_edge-node'", "server.2_edge-node"),
            ("server+edge", "Édition serveur + Edge"),
            ("\"server~edge\"", "Édition serveur + Edge"),
            ("'server^edge'", "Édition serveur + Edge"),
            ("Server", "Édition serveur + Edge"),
            ("\"serveur-é\"", "Édition serveur + Edge"),
            (r"server\ edition", "Édition serveur + Edge"),
            ("\"server edition\"", "Édition serveur + Edge"),
            ("", "Édition serveur + Edge"),
            ("''", "Édition serveur + Edge"),
        ] {
            let metadata = metadata_from_text(&format!(
                "VARIANT_ID={raw}\nVARIANT='Édition serveur + Edge'\n"
            ));
            assert_eq!(metadata.variant.as_deref(), Some(expected), "{raw:?}");
        }
        let metadata = metadata_from_text("VARIANT_ID='Server Edition'\nVARIANT=\"\"\n");
        assert_eq!(metadata.variant, None);
        let metadata =
            metadata_from_text("VARIANT_ID=server\nVARIANT_ID=Server\nVARIANT=display\n");
        assert_eq!(metadata.variant.as_deref(), Some("server"));
    }

    #[test]
    fn missing_and_malformed_release_fields_are_ignored() {
        let values =
            parse_os_release("VERSION_ID=\"unfinished\nVERSION_CODENAME=\nVARIANT_ID=stable\n");
        assert!(!values.contains_key("VERSION_ID"));
        assert_eq!(values.get("VERSION_CODENAME").map(String::as_str), Some(""));
        assert_eq!(values.get("VARIANT_ID").map(String::as_str), Some("stable"));
    }

    #[test]
    fn shell_compatible_escapes_are_decoded_but_concatenation_is_rejected() {
        let values = parse_os_release(
            "VERSION_ID=1.2\nVERSION_CODENAME=\"a\\\"b\\\\c\\$d\\`e\"\n\
             VARIANT=space\\ escaped\n",
        );
        assert_eq!(values.get("VERSION_ID").map(String::as_str), Some("1.2"));
        assert_eq!(
            values.get("VERSION_CODENAME").map(String::as_str),
            Some("a\"b\\c$d`e")
        );
        assert_eq!(
            values.get("VARIANT").map(String::as_str),
            Some("space escaped")
        );
        assert!(!parse_os_release("VARIANT='valid'junk\n").contains_key("VARIANT"));
        assert!(!parse_os_release("VARIANT=bad\"value\"\n").contains_key("VARIANT"));
        let unknown_escape = parse_os_release("VARIANT=\"keep\\q\"\n");
        assert_eq!(
            unknown_escape.get("VARIANT").map(String::as_str),
            Some("keep\\q")
        );
    }

    #[test]
    fn last_valid_assignment_including_empty_value_wins() {
        let values = parse_os_release(
            "VERSION_ID=first\nVERSION_ID=invalid value\nVERSION_ID=\n\
             VERSION_CODENAME=old\nVERSION_CODENAME=\nUBUNTU_CODENAME=fallback\n",
        );
        assert_eq!(values.get("VERSION_ID").map(String::as_str), Some(""));
        assert_eq!(values.get("VERSION_CODENAME").map(String::as_str), Some(""));
        let metadata = super::metadata_from_text(
            "VERSION_CODENAME=old\nVERSION_CODENAME=\nUBUNTU_CODENAME=fallback\n",
        );
        assert_eq!(metadata.codename.as_deref(), Some("fallback"));
    }

    #[test]
    fn missing_primary_falls_back_without_merging_and_oversized_is_rejected() {
        let dir = tempdir().unwrap();
        let primary = dir.path().join("etc-os-release");
        let fallback = dir.path().join("usr-lib-os-release");
        std::fs::write(
            &fallback,
            "VERSION_ID=from-fallback\nVARIANT_ID=\nVARIANT='fallback variant'\n",
        )
        .unwrap();
        let metadata = metadata_from_paths(&primary, &fallback);
        assert_eq!(metadata.version.as_deref(), Some("from-fallback"));
        assert_eq!(metadata.variant.as_deref(), Some("fallback variant"));

        std::fs::write(&primary, "VERSION_ID=primary\n").unwrap();
        let metadata = metadata_from_paths(&primary, &fallback);
        assert_eq!(metadata.version.as_deref(), Some("primary"));
        assert_eq!(metadata.variant, None);

        let oversized = format!(
            "VERSION_ID=partial\n{}",
            "x".repeat(super::MAX_OS_RELEASE_BYTES as usize)
        );
        std::fs::write(&primary, oversized).unwrap();
        let metadata = metadata_from_paths(&primary, &fallback);
        assert_eq!(metadata.version, None);
        assert_eq!(metadata.variant, None);
    }

    #[test]
    fn invalid_utf8_primary_is_rejected_without_falling_back() {
        let dir = tempdir().unwrap();
        let primary = dir.path().join("primary");
        let fallback = dir.path().join("fallback");
        std::fs::write(
            &primary,
            b"VERSION_ID=valid-version\nVERSION_CODENAME=valid-codename\nVARIANT=\"bad\xff\"\n",
        )
        .unwrap();
        std::fs::write(&fallback, "VERSION_ID=must-not-be-used\n").unwrap();

        let metadata = metadata_from_paths(&primary, &fallback);
        assert_eq!(metadata.version, None);
        assert_eq!(metadata.codename, None);
        assert_eq!(metadata.variant, None);
    }

    #[test]
    fn unavailable_primary_uses_fallback_and_present_bad_primary_does_not() {
        let dir = tempdir().unwrap();
        let primary = dir.path().join("missing-primary");
        let fallback = dir.path().join("fallback");
        std::fs::write(&fallback, "VERSION_ID=available\n").unwrap();
        assert_eq!(
            metadata_from_paths(&primary, &fallback).version.as_deref(),
            Some("available")
        );
        std::fs::create_dir(&primary).unwrap();
        assert_eq!(metadata_from_paths(&primary, &fallback).version, None);
        assert!(primary.is_dir());
    }
}
