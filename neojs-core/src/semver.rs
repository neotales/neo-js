//! Semantic-version parsing, formatting, incrementing, and range matching.
//!
//! The module is synchronous, needs no permission, and grants no host
//! capability. It reads no file, environment variable, process argument, or
//! network resource. Parsing and range evaluation use the `semver` crate; this
//! module never uses a regular expression.
//!
//! # Grammar
//!
//! A version is exactly
//! `MAJOR.MINOR.PATCH[-PRERELEASE][+BUILD]` with no surrounding whitespace:
//!
//! * `MAJOR`, `MINOR`, and `PATCH` are ASCII decimal integers without a sign
//!   and without leading zeros (`0` itself is allowed). Each is limited to
//!   `u64::MAX`, the range of the Rust `semver` crate. A larger number is
//!   rejected rather than truncated or rounded.
//! * `PRERELEASE` is one or more dot-separated identifiers made of ASCII
//!   letters, digits, and `-`, and none may be empty. A purely numeric
//!   identifier has no leading zero unless it is exactly `0`.
//! * `BUILD` is one or more dot-separated identifiers made of ASCII letters,
//!   digits, and `-`, and none may be empty. Leading zeros are allowed
//!   because build metadata never participates in precedence.
//!
//! The English SemVer 2.0.0 specification defines no upper bound on a numeric
//! identifier. This module follows the maintained Rust `semver` crate and
//! therefore rejects a core component above `u64::MAX`; the error message
//! names the overflowing component. Numeric identifiers in `PRERELEASE` have
//! no such bound. Every numeric component and numeric prerelease identifier is
//! returned as a decimal string, so a value above `Number.MAX_SAFE_INTEGER`
//! keeps its exact digits.
//!
//! # Precedence
//!
//! `compare` implements SemVer precedence: `MAJOR`, `MINOR`, and `PATCH` are
//! compared numerically; a prerelease is lower than the release with the same
//! core; prerelease identifiers are compared left to right, numeric
//! identifiers numerically and non-numeric identifiers in ASCII order, with a
//! numeric identifier lower than a non-numeric one. A shorter identifier list
//! is lower when every shared identifier is equal. Build metadata is ignored,
//! so `1.0.0+one` and `1.0.0+two` have equal precedence. The crate compares
//! arbitrarily long numeric prerelease identifiers numerically, not as
//! JavaScript doubles.
//!
//! # Ranges
//!
//! `satisfies` and `validRange` use the Cargo interpretation implemented by
//! the `semver` crate, which differs from npm in these ways:
//!
//! * A range is either `*` alone or one or more comma-separated comparators.
//!   There is no `||` union, no hyphen range (`1.2.3 - 2.0.0`), and no
//!   whitespace-separated implicit AND (`>=1.2.3 <2.0.0` is invalid).
//! * A bare version uses Cargo's default caret operator. `1.2` is `^1.2` (so
//!   `1.9.0` matches, `1.1.9` does not), and `1.2.3` is `^1.2.3`, not an exact
//!   match; write `=1.2.3` for exactness. This is unlike npm's `1.2.x`
//!   x-range. `0.2` is `^0.2` and matches only `0.2.x`; `0` is `^0` and
//!   matches any `0.x.y`. Explicit wildcards (`1.2.*`, `1.*`, `*`) are also
//!   accepted.
//! * `~` and `^` follow Cargo's documented equivalences, including `^0.0.3`
//!   matching only `0.0.3` and `^0.0` matching only `0.0.x`.
//! * A prerelease version satisfies a range only when at least one comparator
//!   names the same `MAJOR.MINOR.PATCH` and has a non-empty prerelease. `*`
//!   consequently matches no prerelease. This is the same inclusion rule npm
//!   applies unless `includePrerelease` is requested; this module has no such
//!   option.
//! * At most 32 comparators are accepted, and build metadata on a comparator
//!   is accepted but ignored.
//!
//! # Incrementing
//!
//! `increment` removes build metadata in every case and never wraps:
//! `major`, `minor`, and `patch` bump their component, reset lower components
//! to zero, and drop the prerelease. `premajor`, `preminor`, and `prepatch`
//! do the same and start a prerelease at `0` or at `<identifier>.0`.
//! `prerelease` leaves a version without a prerelease at
//! `PATCH + 1-0` (or `PATCH + 1-<identifier>.0`); with a prerelease, it
//! increments the last numeric identifier, appends `.0` when there is none,
//! or restarts at `<identifier>.0` when the first identifier differs from the
//! supplied identifier. Unlike npm, `major` never treats a prerelease as
//! already promoted: `1.0.0-alpha` increments to `2.0.0`, not `1.0.0`.
//! Overflow of a core component or of a numeric prerelease identifier throws
//! `SEMVER_OVERFLOW`.
//!
//! # Errors
//!
//! A non-string argument throws `TypeError`; nothing is coerced. A malformed
//! version throws `SemVerError` with code `SEMVER_INVALID`, a malformed range
//! with `SEMVER_RANGE`, an unknown release name or invalid identifier with
//! `SEMVER_INCREMENT`, and arithmetic overflow with `SEMVER_OVERFLOW`. The
//! `code` is stable; the `message` may add detail and is not stable. `valid`
//! and `validRange` return `false` for malformed input instead of throwing.
//!
//! # Limits
//!
//! Input is bounded before it crosses into native code. A version string may
//! be at most `MAX_VERSION_BYTES` (1024) UTF-8 bytes, a range at most
//! `MAX_RANGE_BYTES` (4096), a core component at most `MAX_CORE_BYTES` (20), a
//! version may have at most `MAX_IDENTIFIERS` (32) prerelease identifiers and
//! at most that many build identifiers, and one identifier may be at most
//! `MAX_IDENTIFIER_BYTES` (256) bytes. `format` applies the same per-field
//! limits, at most 32 entries per array, and `MAX_VERSION_BYTES` to the exact
//! rendered version. The range comparator limit remains 32, as documented
//! above.
//!
//! The JavaScript wrapper checks primitive string lengths, scans a
//! length-bounded version for identifier count and size, and checks array
//! counts before copying or calling native code; the native functions recheck
//! the same bounds, using checked addition for the rendered-size total. The
//! wrapper's length precheck counts UTF-16 code units while native code
//! enforces UTF-8 bytes; the accepted grammar is ASCII, so the limits agree
//! for every accepted value and a non-ASCII value is rejected by at least one
//! check.
//! Exceeding a bound throws `SemVerError` with code `SEMVER_LIMIT`, except
//! that `valid` and `validRange` return `false`. Limits take precedence over
//! malformed input. These bounds are new behavior for the preserved
//! `valid`/`compare` exports: a version longer than the limit now makes
//! `valid` return `false` and `compare` throw `SEMVER_LIMIT` instead of
//! parsing without a bound.
//!
//! The size checks use the primitive string's own length and `Array.isArray`.
//! A `format` argument object can be a Proxy, have getters, or be an array
//! subclass: those property reads are ordinary JavaScript and may run user
//! code before a value is bounded. The module does not claim such an object is
//! side-effect free; it only bounds the primitive strings and array lengths it
//! actually receives.
//!
//! # Output shape
//!
//! `parse` returns a null-prototype object whose `version`, `major`, `minor`,
//! `patch`, `prerelease` identifiers, and `build` identifiers are own data
//! properties. `prerelease` and `build` are arrays of strings with an own
//! property per index. A polluted `Object.prototype` or `Array.prototype`
//! setter cannot receive or replace a value. The object is a snapshot: mutate
//! it freely without affecting later calls.
//!
//! The native callbacks receive their arguments only through the private
//! factory built before user code runs. They are never installed on the global
//! object, so no `__neoSemver*` name exists to replace.

use rquickjs::{
    Ctx, Function, Object, Persistent, Result as JsResult, Value,
    function::{Flat, Opt},
    module::{Declarations, Exports, ModuleDef},
};
use semver::{BuildMetadata, Prerelease, Version, VersionReq};
use serde_json::{Value as Json, json};

/// Stable code for a malformed version or format component.
const INVALID: &str = "SEMVER_INVALID";
/// Stable code for a malformed range.
const RANGE: &str = "SEMVER_RANGE";
/// Stable code for an unsupported release name or increment identifier.
const INCREMENT: &str = "SEMVER_INCREMENT";
/// Stable code for arithmetic that would exceed the supported integer range.
const OVERFLOW: &str = "SEMVER_OVERFLOW";
/// Stable code for input that exceeds a documented size or count limit.
const LIMIT: &str = "SEMVER_LIMIT";

/// Largest accepted version string, in UTF-8 bytes.
pub(crate) const MAX_VERSION_BYTES: usize = 1024;
/// Largest accepted range string, in UTF-8 bytes.
pub(crate) const MAX_RANGE_BYTES: usize = 4096;
/// Largest accepted core component, in bytes (the widest `u64` is 20 digits).
pub(crate) const MAX_CORE_BYTES: usize = 20;
/// Maximum prerelease identifiers and maximum build identifiers.
pub(crate) const MAX_IDENTIFIERS: usize = 32;
/// Largest accepted single prerelease or build identifier, in bytes.
pub(crate) const MAX_IDENTIFIER_BYTES: usize = 256;

/// A deterministic failure that the JavaScript wrapper turns into a
/// `SemVerError`. The message comes from the parser or from this module; it
/// never repeats the caller's full input.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct Failure {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

impl Failure {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Serialize a successful native result. The wrapper parses this and rebuilds
/// every object with a null prototype.
fn ok(value: Json) -> String {
    json!({"ok": true, "value": value}).to_string()
}

/// Serialize a native failure. The wrapper throws `SemVerError`.
fn failed(failure: Failure) -> String {
    json!({
        "ok": false,
        "error": {"code": failure.code, "message": failure.message},
    })
    .to_string()
}

fn outcome<T: Into<Json>>(result: Result<T, Failure>) -> String {
    match result {
        Ok(value) => ok(value.into()),
        Err(failure) => failed(failure),
    }
}

fn parse_version(text: &str) -> Result<Version, Failure> {
    Version::parse(text).map_err(|error| Failure::new(INVALID, error.to_string()))
}

fn limit() -> Failure {
    Failure::new(LIMIT, "semantic version input exceeds a documented limit")
}

/// Check the identifier count and per-identifier size of a parsed version.
fn check_identifier_bounds(version: &Version) -> Result<(), Failure> {
    for text in [version.pre.as_str(), version.build.as_str()] {
        if text.is_empty() {
            continue;
        }
        let mut count = 0usize;
        for identifier in text.split('.') {
            count = count.checked_add(1).ok_or_else(limit)?;
            if count > MAX_IDENTIFIERS || identifier.len() > MAX_IDENTIFIER_BYTES {
                return Err(limit());
            }
        }
    }
    Ok(())
}

/// Parse a version with the documented input limits. The JavaScript wrapper
/// applies a length precheck before the string crosses into native code; this
/// function is the authoritative recheck.
fn bounded_version(text: &str) -> Result<Version, Failure> {
    if text.len() > MAX_VERSION_BYTES {
        return Err(limit());
    }
    let version = parse_version(text)?;
    check_identifier_bounds(&version)?;
    Ok(version)
}

fn bounded_range(text: &str) -> Result<VersionReq, Failure> {
    if text.len() > MAX_RANGE_BYTES {
        return Err(limit());
    }
    VersionReq::parse(text).map_err(|error| Failure::new(RANGE, error.to_string()))
}

/// The native Boolean behind `valid`. It never throws for a string.
pub(crate) fn valid_js(version: String) -> bool {
    bounded_version(&version).is_ok()
}

/// The native Boolean behind `validRange`. It never throws for a string.
pub(crate) fn valid_range_js(range: String) -> bool {
    bounded_range(&range).is_ok()
}

/// The native JSON response behind `parse`.
pub(crate) fn parse_js(version: String) -> String {
    outcome(bounded_version(&version).map(|version| encode_version(&version)))
}

/// The native JSON response behind `compare`.
pub(crate) fn compare_js(a: String, b: String) -> String {
    let result = (|| {
        let a = bounded_version(&a)?;
        let b = bounded_version(&b)?;
        let ordering = match a.cmp_precedence(&b) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        };
        Ok::<i32, Failure>(ordering)
    })();
    outcome(result)
}

/// The native JSON response behind `satisfies`.
pub(crate) fn satisfies_js(version: String, range: String) -> String {
    let result = (|| {
        let version = bounded_version(&version)?;
        let range = bounded_range(&range)?;
        Ok::<bool, Failure>(range.matches(&version))
    })();
    outcome(result)
}

/// Split a prerelease or build string into identifiers. The crate rejects an
/// empty identifier, so an empty string means "none".
fn identifiers(text: &str) -> Vec<Json> {
    if text.is_empty() {
        Vec::new()
    } else {
        text.split('.')
            .map(|part| Json::String(part.to_owned()))
            .collect()
    }
}

fn encode_version(version: &Version) -> Json {
    json!({
        "version": version.to_string(),
        "major": version.major.to_string(),
        "minor": version.minor.to_string(),
        "patch": version.patch.to_string(),
        "prerelease": identifiers(version.pre.as_str()),
        "build": identifiers(version.build.as_str()),
    })
}

/// True for a non-empty ASCII decimal string without a leading zero.
fn is_canonical_numeric(identifier: &str) -> bool {
    let mut characters = identifier.chars();
    match characters.next() {
        None => false,
        Some('0') => characters.next().is_none(),
        Some(first) if first.is_ascii_digit() => {
            characters.all(|character| character.is_ascii_digit())
        }
        Some(_) => false,
    }
}

/// True for a prerelease identifier: ASCII alphanumerics and `-`, with no
/// leading zero on a purely numeric identifier.
fn is_prerelease_identifier(identifier: &str) -> bool {
    if identifier.is_empty() {
        return false;
    }
    if identifier
        .chars()
        .all(|character| character.is_ascii_digit())
    {
        return is_canonical_numeric(identifier);
    }
    identifier
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '-')
}

/// True for a build identifier: ASCII alphanumerics and `-`, non-empty.
/// Leading zeros are permitted.
fn is_build_identifier(identifier: &str) -> bool {
    !identifier.is_empty()
        && identifier
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
}

/// Validate a core component supplied to `format`.
fn core_component(text: &str, name: &str) -> Result<u64, Failure> {
    if !is_canonical_numeric(text) {
        return Err(Failure::new(
            INVALID,
            format!("{name} must be a canonical decimal integer"),
        ));
    }
    text.parse::<u64>().map_err(|_| {
        Failure::new(
            OVERFLOW,
            format!("{name} exceeds the supported 64-bit range"),
        )
    })
}

/// The native JSON response behind `format`.
///
/// The wrapper has already checked that every element is a string and copied
/// it into a fresh array. Components are validated here with the same grammar
/// the parser uses, then the canonical `Version` display form is returned.
pub(crate) fn format_js(
    major: String,
    minor: String,
    patch: String,
    prerelease: Vec<String>,
    build: Vec<String>,
) -> String {
    let result = (|| {
        for component in [&major, &minor, &patch] {
            if component.len() > MAX_CORE_BYTES {
                return Err(limit());
            }
        }
        if prerelease.len() > MAX_IDENTIFIERS || build.len() > MAX_IDENTIFIERS {
            return Err(limit());
        }
        for identifier in prerelease.iter().chain(build.iter()) {
            if identifier.len() > MAX_IDENTIFIER_BYTES {
                return Err(limit());
            }
        }
        let major_value = core_component(&major, "major")?;
        let minor_value = core_component(&minor, "minor")?;
        let patch_value = core_component(&patch, "patch")?;
        for identifier in &prerelease {
            if !is_prerelease_identifier(identifier) {
                return Err(Failure::new(
                    INVALID,
                    "prerelease contains an invalid identifier",
                ));
            }
        }
        for identifier in &build {
            if !is_build_identifier(identifier) {
                return Err(Failure::new(
                    INVALID,
                    "build contains an invalid identifier",
                ));
            }
        }
        // Check the exact rendered size before allocating it. Every step is
        // checked so a huge (but already count-bounded) input cannot wrap the
        // arithmetic into a small total.
        let mut total = major
            .len()
            .checked_add(minor.len())
            .and_then(|sum| sum.checked_add(patch.len()))
            .and_then(|sum| sum.checked_add(2))
            .ok_or_else(limit)?;
        for (prefix, identifiers) in [(1usize, &prerelease), (1usize, &build)] {
            if identifiers.is_empty() {
                continue;
            }
            total = total.checked_add(prefix).ok_or_else(limit)?;
            for (index, identifier) in identifiers.iter().enumerate() {
                total = total.checked_add(identifier.len()).ok_or_else(limit)?;
                if index > 0 {
                    total = total.checked_add(1).ok_or_else(limit)?;
                }
            }
        }
        if total > MAX_VERSION_BYTES {
            return Err(limit());
        }
        let mut text = format!("{major_value}.{minor_value}.{patch_value}");
        if !prerelease.is_empty() {
            text.push('-');
            text.push_str(&prerelease.join("."));
        }
        if !build.is_empty() {
            text.push('+');
            text.push_str(&build.join("."));
        }
        Version::parse(&text)
            .map(|version| version.to_string())
            .map_err(|error| Failure::new(INVALID, error.to_string()))
    })();
    outcome(result)
}

/// Validate an optional `increment` identifier.
fn increment_identifier(identifier: Option<&str>) -> Result<Option<&str>, Failure> {
    match identifier {
        None => Ok(None),
        Some(value) if value.len() > MAX_IDENTIFIER_BYTES => Err(limit()),
        Some(value) if is_prerelease_identifier(value) => Ok(Some(value)),
        Some(_) => Err(Failure::new(
            INCREMENT,
            "identifier must be a single valid prerelease identifier",
        )),
    }
}

fn bumped(component: u64, name: &str) -> Result<u64, Failure> {
    component.checked_add(1).ok_or_else(|| {
        Failure::new(
            OVERFLOW,
            format!("{name} cannot be incremented beyond the supported 64-bit range"),
        )
    })
}

fn start_prerelease(identifier: Option<&str>) -> Result<Prerelease, Failure> {
    let text = match identifier {
        Some(identifier) => format!("{identifier}.0"),
        None => "0".to_owned(),
    };
    Prerelease::new(&text).map_err(|error| Failure::new(INVALID, error.to_string()))
}

/// The native JSON response behind `increment`.
pub(crate) fn increment_js(version: String, release: String, identifier: Opt<String>) -> String {
    let result = (|| {
        let version = bounded_version(&version)?;
        let identifier = increment_identifier(identifier.0.as_deref())?;
        let mut next = version.clone();
        next.build = BuildMetadata::EMPTY;
        match release.as_str() {
            "major" => {
                next.major = bumped(next.major, "major")?;
                next.minor = 0;
                next.patch = 0;
                next.pre = Prerelease::EMPTY;
            }
            "minor" => {
                next.minor = bumped(next.minor, "minor")?;
                next.patch = 0;
                next.pre = Prerelease::EMPTY;
            }
            "patch" => {
                next.patch = bumped(next.patch, "patch")?;
                next.pre = Prerelease::EMPTY;
            }
            "premajor" => {
                next.major = bumped(next.major, "major")?;
                next.minor = 0;
                next.patch = 0;
                next.pre = start_prerelease(identifier)?;
            }
            "preminor" => {
                next.minor = bumped(next.minor, "minor")?;
                next.patch = 0;
                next.pre = start_prerelease(identifier)?;
            }
            "prepatch" => {
                next.patch = bumped(next.patch, "patch")?;
                next.pre = start_prerelease(identifier)?;
            }
            "prerelease" => {
                if next.pre.is_empty() {
                    next.patch = bumped(next.patch, "patch")?;
                    next.pre = start_prerelease(identifier)?;
                } else {
                    let parts: Vec<&str> = next.pre.as_str().split('.').collect();
                    let same_identifier = match identifier {
                        Some(identifier) => parts.first() == Some(&identifier),
                        None => true,
                    };
                    if !same_identifier {
                        next.pre = start_prerelease(identifier)?;
                    } else {
                        let mut parts: Vec<String> =
                            parts.iter().map(|part| (*part).to_owned()).collect();
                        let mut incremented = false;
                        for part in parts.iter_mut().rev() {
                            if part.chars().all(|character| character.is_ascii_digit()) {
                                let value = part.parse::<u64>().map_err(|_| {
                                    Failure::new(
                                        OVERFLOW,
                                        "prerelease identifier exceeds the supported 64-bit range",
                                    )
                                })?;
                                *part = bumped(value, "prerelease identifier")?.to_string();
                                incremented = true;
                                break;
                            }
                        }
                        if !incremented {
                            parts.push("0".to_owned());
                        }
                        next.pre = Prerelease::new(&parts.join("."))
                            .map_err(|error| Failure::new(INVALID, error.to_string()))?;
                    }
                }
            }
            _ => {
                return Err(Failure::new(INCREMENT, "unknown release name"));
            }
        }
        // Incrementing can add an identifier or grow the rendered string.
        // Reapply the same bounds as public parsing before returning it.
        let rendered = next.to_string();
        bounded_version(&rendered)?;
        Ok::<String, Failure>(rendered)
    })();
    outcome(result)
}

/// Persistent factory for the private `neo:semver` wrapper.
#[derive(Default)]
pub(crate) struct SemverState {
    pub(crate) factory: Option<Persistent<Function<'static>>>,
}

// SemverState contains no borrowed JS value; its persistent handle is
// explicitly released before the context and runtime are dropped.
unsafe impl<'js> rquickjs::JsLifetime<'js> for SemverState {
    type Changed<'to> = SemverState;
}

const EXPORTS: [&str; 13] = [
    "valid",
    "compare",
    "parse",
    "format",
    "increment",
    "satisfies",
    "validRange",
    "SemVerError",
    "MAX_VERSION_BYTES",
    "MAX_RANGE_BYTES",
    "MAX_CORE_BYTES",
    "MAX_IDENTIFIERS",
    "MAX_IDENTIFIER_BYTES",
];

/// The native `neo:semver` module.
pub(crate) struct SemverModule;

impl ModuleDef for SemverModule {
    fn declare<'js>(decl: &Declarations<'js>) -> JsResult<()> {
        for name in EXPORTS {
            decl.declare(name)?;
        }
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> JsResult<()> {
        use std::{cell::RefCell, rc::Rc};
        let state = Rc::clone(
            &*ctx
                .userdata::<Rc<RefCell<SemverState>>>()
                .ok_or(rquickjs::Error::Unknown)?,
        );
        let factory = state
            .borrow()
            .factory
            .as_ref()
            .cloned()
            .ok_or(rquickjs::Error::Unknown)?
            .restore(ctx)?;
        let valid = Function::new(ctx.clone(), valid_js)?;
        let compare = Function::new(ctx.clone(), compare_js)?;
        let parse = Function::new(ctx.clone(), parse_js)?;
        let format = Function::new(ctx.clone(), format_js)?;
        let increment = Function::new(ctx.clone(), increment_js)?;
        let satisfies = Function::new(ctx.clone(), satisfies_js)?;
        let valid_range = Function::new(ctx.clone(), valid_range_js)?;
        let limits = format!(
            r#"{{"MAX_VERSION_BYTES":{MAX_VERSION_BYTES},"MAX_RANGE_BYTES":{MAX_RANGE_BYTES},"MAX_CORE_BYTES":{MAX_CORE_BYTES},"MAX_IDENTIFIERS":{MAX_IDENTIFIERS},"MAX_IDENTIFIER_BYTES":{MAX_IDENTIFIER_BYTES}}}"#
        );
        let api: Object = factory.call((
            valid,
            Flat((
                compare,
                parse,
                format,
                increment,
                satisfies,
                valid_range,
                limits,
            )),
        ))?;
        for name in EXPORTS {
            exports.export(name, api.get::<_, Value>(name)?)?;
        }
        Ok(())
    }
}

/// Build the private wrapper in the fresh realm, before user code runs.
pub(crate) fn factory<'js>(ctx: &Ctx<'js>) -> JsResult<Function<'js>> {
    ctx.eval(include_str!("js/semver.js"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(value: &str) -> Json {
        let response: Json = serde_json::from_str(&parse_js(value.to_owned())).unwrap();
        assert_eq!(response["ok"], true, "{response}");
        response["value"].clone()
    }

    fn failure(code: &str, response: String) -> Json {
        let response: Json = serde_json::from_str(&response).unwrap();
        assert_eq!(response["ok"], false, "{response}");
        assert_eq!(response["error"]["code"], code, "{response}");
        response
    }

    fn increment(value: &str, release: &str, identifier: Option<&str>) -> String {
        let response: Json = serde_json::from_str(&increment_js(
            value.to_owned(),
            release.to_owned(),
            Opt(identifier.map(str::to_owned)),
        ))
        .unwrap();
        assert_eq!(response["ok"], true, "{response}");
        response["value"].as_str().unwrap().to_owned()
    }

    #[test]
    fn parses_documented_components_without_losing_precision() {
        let value = parsed("18446744073709551615.0.1-alpha.9007199254740993+sha.001");
        assert_eq!(
            value["version"],
            "18446744073709551615.0.1-alpha.9007199254740993+sha.001"
        );
        assert_eq!(value["major"], "18446744073709551615");
        assert_eq!(value["minor"], "0");
        assert_eq!(value["patch"], "1");
        assert_eq!(value["prerelease"][0], "alpha");
        assert_eq!(value["prerelease"][1], "9007199254740993");
        assert_eq!(value["build"][0], "sha");
        assert_eq!(value["build"][1], "001");
    }

    #[test]
    fn core_overflow_is_rejected_clearly() {
        let response = failure(INVALID, parse_js("18446744073709551616.0.0".to_owned()));
        assert!(
            response["error"]["message"]
                .as_str()
                .unwrap()
                .contains("u64::MAX"),
            "{response}"
        );
    }

    #[test]
    fn leading_zeros_and_partial_versions_are_invalid() {
        for value in [
            "01.2.3", "1.02.3", "1.2.03", "1.2", "1", "1.2.3-01", "1.2.3+", "1.2.3-",
        ] {
            let response = failure(INVALID, parse_js(value.to_owned()));
            assert!(!response["ok"].as_bool().unwrap());
        }
        assert!(valid_js("0.0.0".to_owned()));
        assert!(!valid_js("0.0.0-01".to_owned()));
    }

    #[test]
    fn build_metadata_never_affects_precedence() {
        let response: Json =
            serde_json::from_str(&compare_js("1.0.0+a".to_owned(), "1.0.0+b".to_owned())).unwrap();
        assert_eq!(response["value"], 0);
        assert_eq!(increment("1.2.3+build.9", "patch", None), "1.2.4");
    }

    #[test]
    fn increments_follow_the_documented_rules() {
        assert_eq!(increment("1.2.3", "major", None), "2.0.0");
        assert_eq!(increment("1.2.3", "minor", None), "1.3.0");
        assert_eq!(increment("1.2.3", "patch", None), "1.2.4");
        assert_eq!(increment("1.2.3", "premajor", None), "2.0.0-0");
        assert_eq!(increment("1.2.3", "premajor", Some("beta")), "2.0.0-beta.0");
        assert_eq!(increment("1.2.3", "preminor", Some("rc")), "1.3.0-rc.0");
        assert_eq!(increment("1.2.3", "prepatch", None), "1.2.4-0");
        assert_eq!(increment("1.2.3", "prerelease", None), "1.2.4-0");
        assert_eq!(
            increment("1.2.3-alpha", "prerelease", None),
            "1.2.3-alpha.0"
        );
        assert_eq!(
            increment("1.2.3-alpha.9", "prerelease", None),
            "1.2.3-alpha.10"
        );
        assert_eq!(
            increment("1.2.3-alpha.9", "prerelease", Some("beta")),
            "1.2.3-beta.0"
        );
        assert_eq!(increment("1.0.0-alpha", "major", None), "2.0.0");
    }

    #[test]
    fn increment_overflow_is_a_stable_error() {
        failure(
            OVERFLOW,
            increment_js(
                "18446744073709551615.0.0".to_owned(),
                "major".to_owned(),
                Opt(None),
            ),
        );
        failure(
            OVERFLOW,
            increment_js(
                "1.0.0-18446744073709551615".to_owned(),
                "prerelease".to_owned(),
                Opt(None),
            ),
        );
    }

    #[test]
    fn format_round_trips_and_validates_components() {
        let response: Json = serde_json::from_str(&format_js(
            "1".to_owned(),
            "2".to_owned(),
            "3".to_owned(),
            vec!["alpha".to_owned(), "1".to_owned()],
            vec!["sha".to_owned(), "001".to_owned()],
        ))
        .unwrap();
        assert_eq!(response["value"], "1.2.3-alpha.1+sha.001");
        failure(
            INVALID,
            format_js(
                "01".to_owned(),
                "2".to_owned(),
                "3".to_owned(),
                vec![],
                vec![],
            ),
        );
        failure(
            INVALID,
            format_js(
                "1".to_owned(),
                "2".to_owned(),
                "3".to_owned(),
                vec!["01".to_owned()],
                vec![],
            ),
        );
        failure(
            INVALID,
            format_js(
                "1".to_owned(),
                "2".to_owned(),
                "3".to_owned(),
                vec![],
                vec!["a.b".to_owned()],
            ),
        );
    }

    #[test]
    fn range_failures_have_their_own_code() {
        failure(
            RANGE,
            satisfies_js("1.2.3".to_owned(), ">=1.2.3 <2.0.0".to_owned()),
        );
        assert!(valid_range_js("^1.2.3".to_owned()));
    }

    #[test]
    fn range_comparator_limit_is_documented() {
        let allowed = vec![">=0.0.0"; 32].join(", ");
        assert!(valid_range_js(allowed));
        let excessive = vec![">=0.0.0"; 33].join(", ");
        assert!(!valid_range_js(excessive));
    }

    fn at_version_limit() -> String {
        let mut text = String::from("1.0.0-");
        for index in 0..MAX_IDENTIFIERS {
            if index > 0 {
                text.push('.');
            }
            text.push_str(&"a".repeat(30));
        }
        text.push('+');
        text.push_str(&"b".repeat(26));
        assert_eq!(text.len(), MAX_VERSION_BYTES);
        text
    }

    #[test]
    fn version_length_limit_is_checked_at_and_above_the_bound() {
        let at_limit = at_version_limit();
        assert!(valid_js(at_limit.clone()));
        let above = format!("{at_limit}a");
        assert_eq!(above.len(), MAX_VERSION_BYTES + 1);
        assert!(!valid_js(above.clone()));
        failure(LIMIT, parse_js(above));
    }

    #[test]
    fn range_length_limit_is_checked_at_and_above_the_bound() {
        let at_limit = format!(">=1.0.0-{}", "a".repeat(MAX_RANGE_BYTES - 8));
        assert_eq!(at_limit.len(), MAX_RANGE_BYTES);
        assert!(valid_range_js(at_limit));
        let above = format!(">=1.0.0-{}", "a".repeat(MAX_RANGE_BYTES - 7));
        assert!(!valid_range_js(above.clone()));
        failure(LIMIT, satisfies_js("1.0.0".to_owned(), above));
    }

    #[test]
    fn identifier_count_and_size_limits_are_checked() {
        let allowed = format!("1.0.0-{}", vec!["a"; MAX_IDENTIFIERS].join("."));
        assert!(valid_js(allowed));
        let excessive = format!("1.0.0-{}", vec!["a"; MAX_IDENTIFIERS + 1].join("."));
        assert!(!valid_js(excessive.clone()));
        failure(LIMIT, parse_js(excessive));

        let largest = format!("1.0.0-{}", "a".repeat(MAX_IDENTIFIER_BYTES));
        assert!(valid_js(largest));
        let too_large = format!("1.0.0-{}", "a".repeat(MAX_IDENTIFIER_BYTES + 1));
        failure(LIMIT, parse_js(too_large));
    }

    #[test]
    fn format_bounds_are_enforced_natively() {
        failure(
            LIMIT,
            format_js(
                "1".to_owned(),
                "0".to_owned(),
                "0".to_owned(),
                vec!["a".to_owned(); MAX_IDENTIFIERS + 1],
                vec![],
            ),
        );
        failure(
            LIMIT,
            format_js(
                "1".to_owned(),
                "0".to_owned(),
                "0".to_owned(),
                vec!["a".repeat(MAX_IDENTIFIER_BYTES + 1)],
                vec![],
            ),
        );
        failure(
            LIMIT,
            format_js(
                "1".to_owned(),
                "0".to_owned(),
                "0".to_owned(),
                vec!["a".repeat(100); MAX_IDENTIFIERS],
                vec![],
            ),
        );
        assert!(
            serde_json::from_str::<Json>(&format_js(
                "1".to_owned(),
                "0".to_owned(),
                "0".to_owned(),
                vec!["a".to_owned(); MAX_IDENTIFIERS],
                vec![],
            ))
            .unwrap()["ok"]
                == true
        );
    }

    #[test]
    fn implementation_uses_no_regular_expressions() {
        let rust = include_str!("semver.rs");
        let end = rust
            .find("#[cfg(test)]")
            .expect("the production module precedes its tests");
        let javascript = include_str!("js/semver.js");
        for source in [&rust[..end], javascript] {
            for needle in ["regex", "Regex", "RegExp", ".match(", ".replace("] {
                assert!(
                    !source.contains(needle),
                    "semver implementation contains {needle:?}"
                );
            }
        }
    }
}
