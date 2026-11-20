//! Deterministic, offline composition of the TypeScript declarations.
//!
//! This module is compiled by the `compose-types` example and by the
//! `declarations` integration test. It is not part of the `neojs` binary.
//!
//! Selection is explicit. `--check-cli-manifest` verifies that the explicit
//! selection agrees with the normal dependencies and default optional features
//! in `neojs/Cargo.toml`; it never selects a fragment from the
//! manifest on its own, and it is not a full Cargo resolver.

use std::{
    collections::BTreeSet,
    fs, io,
    path::{Path, PathBuf},
};

pub const CORE_PATH: &str = "types/neo-js.core.d.ts";
pub const CLI_MANIFEST_PATH: &str = "neojs/Cargo.toml";

pub const USAGE: &str = "\
Usage: compose-types [OPTIONS]

Compose the TypeScript declarations for an explicit set of compiled modules.
Core declarations are always included; optional fragments are selected with
--with.

Options:
  --with <NAME>          Include an optional fragment (repeatable).
                         Available: os, os-user, schedulers, sqlite.
  --output <PATH>        Write the composition to PATH. Use - for stdout.
  --check <PATH>         Fail if PATH differs from the composition.
  --check-cli-manifest   Verify the selection against neojs/Cargo.toml.
  --root <PATH>          Repository root (default: this checkout).
  -h, --help             Print this help.

Selection is explicit. The manifest check only verifies consistency; it never
selects fragments by itself. It resolves direct normal dependencies and default
optional features only; dev/build entries are ignored, and target-specific,
workspace, or renamed entries are reported unresolved. It is not a full Cargo
resolver.
";

#[derive(Clone, Copy)]
struct Fragment {
    name: &'static str,
    path: &'static str,
    dependency: &'static str,
}

const CORE: Fragment = Fragment {
    name: "core",
    path: CORE_PATH,
    dependency: "neojs-core",
};

const OPTIONAL: &[Fragment] = &[
    Fragment {
        name: "os",
        path: "neojs-os/types/neo-os.d.ts",
        dependency: "neojs-os",
    },
    Fragment {
        name: "os-user",
        path: "neojs-os-user/types/neo-os-user.d.ts",
        dependency: "neojs-os-user",
    },
    Fragment {
        name: "schedulers",
        path: "neojs-schedulers/types/neo-schedulers.d.ts",
        dependency: "neojs-schedulers",
    },
    Fragment {
        name: "sqlite",
        path: "neojs-sqlite/types/neo-sqlite.d.ts",
        dependency: "neojs-sqlite",
    },
];

enum Output {
    Stdout,
    File(PathBuf),
    Check(PathBuf),
}

struct Options {
    root: PathBuf,
    with: Vec<String>,
    output: Output,
    verify_manifest: bool,
}

/// The checkout containing `types/` and the crate directories.
pub fn default_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Compose the core declarations plus the named optional fragments.
pub fn compose(root: &Path, optional: &[&str]) -> Result<String, String> {
    let selected = resolve(optional)?;
    render(root, &selected)
}

/// Run the `compose-types` command. Return `Some(text)` for stdout, or `None`
/// when the composition was written or checked.
pub fn run<I>(args: I) -> Result<Option<String>, String>
where
    I: IntoIterator<Item = String>,
{
    let args: Vec<String> = args.into_iter().collect();
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        return Ok(Some(USAGE.to_string()));
    }

    let options = Options::parse(args)?;
    let with = options.with.iter().map(String::as_str).collect::<Vec<_>>();
    let selected = resolve(&with)?;
    if options.verify_manifest {
        check_cli_manifest(&options.root, &selected)?;
    }
    let text = render(&options.root, &selected)?;

    match options.output {
        Output::Stdout => Ok(Some(text)),
        Output::File(path) => {
            write_atomically(&options.root, &path, &text)?;
            Ok(None)
        }
        Output::Check(path) => {
            let actual = fs::read_to_string(&path)
                .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
            if actual == text {
                Ok(None)
            } else {
                Err(format!(
                    "{} is stale; regenerate with the compose-types command in types/README.md",
                    path.display()
                ))
            }
        }
    }
}

impl Options {
    fn parse(args: Vec<String>) -> Result<Self, String> {
        let mut root = default_root();
        let mut root_set = false;
        let mut with = Vec::new();
        let mut output = None;
        let mut check = None;
        let mut verify_manifest = false;
        let mut args = args.into_iter();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--root" => {
                    if root_set {
                        return Err("--root given twice".to_string());
                    }
                    root = PathBuf::from(value("--root", &mut args)?);
                    root_set = true;
                }
                "--with" => with.push(value("--with", &mut args)?),
                "--output" => {
                    if output.is_some() {
                        return Err("--output given twice".to_string());
                    }
                    if check.is_some() {
                        return Err("--output and --check are mutually exclusive".to_string());
                    }
                    let value = value("--output", &mut args)?;
                    output = Some(if value == "-" {
                        Output::Stdout
                    } else {
                        Output::File(PathBuf::from(value))
                    });
                }
                "--check" => {
                    if check.is_some() {
                        return Err("--check given twice".to_string());
                    }
                    if output.is_some() {
                        return Err("--output and --check are mutually exclusive".to_string());
                    }
                    check = Some(Output::Check(PathBuf::from(value("--check", &mut args)?)));
                }
                "--check-cli-manifest" => verify_manifest = true,
                _ => return Err(format!("unknown argument `{arg}`")),
            }
        }

        Ok(Self {
            root,
            with,
            output: check.or(output).unwrap_or(Output::Stdout),
            verify_manifest,
        })
    }
}

fn value(flag: &str, args: &mut impl Iterator<Item = String>) -> Result<String, String> {
    args.next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{flag} needs a non-empty value"))
}

fn resolve(optional: &[&str]) -> Result<Vec<&'static Fragment>, String> {
    let mut selected = BTreeSet::new();
    for name in optional {
        if !selected.insert(*name) {
            return Err(format!("duplicate --with `{name}`"));
        }
        if !OPTIONAL.iter().any(|fragment| fragment.name == *name) {
            return Err(format!(
                "unknown optional fragment `{name}`; available: {}",
                available()
            ));
        }
    }
    let mut fragments = vec![&CORE];
    fragments.extend(
        OPTIONAL
            .iter()
            .filter(|fragment| selected.contains(fragment.name)),
    );
    Ok(fragments)
}

fn available() -> String {
    OPTIONAL
        .iter()
        .map(|fragment| fragment.name)
        .collect::<Vec<_>>()
        .join(", ")
}

fn render(root: &Path, selected: &[&Fragment]) -> Result<String, String> {
    let inputs = selected
        .iter()
        .map(|fragment| fragment.path)
        .collect::<Vec<_>>()
        .join(", ");
    let names = selected
        .iter()
        .map(|fragment| fragment.name)
        .collect::<Vec<_>>()
        .join(", ");
    let mut out = format!(
        "// Generated declarations for the selected compiled modules. Do not edit.\n\
         // Inputs: {inputs}\n\
         // Selection: {names}\n\
         // See types/README.md for the compose-types command.\n\n"
    );

    for (index, fragment) in selected.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        let path = root.join(fragment.path);
        let text = fs::read_to_string(&path)
            .map_err(|error| format!("cannot read {}: {error}", fragment.path))?;
        out.push_str(&text);
        if !out.ends_with('\n') {
            out.push('\n');
        }
    }
    Ok(out)
}

struct DependencyStatus {
    linked: bool,
    detail: &'static str,
}

fn check_cli_manifest(root: &Path, selected: &[&Fragment]) -> Result<(), String> {
    let source = fs::read_to_string(root.join(CLI_MANIFEST_PATH))
        .map_err(|error| format!("cannot read {CLI_MANIFEST_PATH}: {error}"))?;
    let manifest: toml::Table = toml::from_str(&source)
        .map_err(|error| format!("cannot parse {CLI_MANIFEST_PATH}: {error}"))?;
    let activated = default_activated_optional_dependencies(&manifest)?;

    let core = dependency_status(&manifest, CORE.dependency, &activated)?;
    if !core.linked {
        return Err(format!(
            "{CLI_MANIFEST_PATH} does not link `{}` ({})",
            CORE.dependency, core.detail
        ));
    }

    for fragment in OPTIONAL {
        let status = dependency_status(&manifest, fragment.dependency, &activated)?;
        let selected = selected
            .iter()
            .any(|selected| selected.name == fragment.name);
        match (status.linked, selected) {
            (true, false) => {
                return Err(format!(
                    "{CLI_MANIFEST_PATH} links `{}` ({}) but the selection omits `{}`; \
                     add `--with {}` or remove the dependency",
                    fragment.dependency, status.detail, fragment.name, fragment.name
                ));
            }
            (false, true) => {
                return Err(format!(
                    "the selection includes `{}` but {CLI_MANIFEST_PATH} does not link `{}` ({})",
                    fragment.name, fragment.dependency, status.detail
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

struct TargetEntry {
    target: String,
    section: &'static str,
    key: String,
}

fn dependency_status(
    manifest: &toml::Table,
    name: &str,
    activated: &BTreeSet<String>,
) -> Result<DependencyStatus, String> {
    // Target-specific tables are not resolved. Detect any relevant entry
    // before considering a normal dependency, so a target-only or
    // target-and-normal declaration never silently falls back.
    if let Some(entry) = target_dependency(manifest, name)? {
        return Err(format!(
            "{CLI_MANIFEST_PATH} declares `{name}` for target `{}` under `{}` in [{}]; compose-types does not resolve target-specific dependency tables",
            entry.target, entry.key, entry.section
        ));
    }

    let mut wrong_override = false;
    if let Some(dependencies) = manifest.get("dependencies").and_then(toml::Value::as_table) {
        let mut keys = dependencies.keys().collect::<Vec<_>>();
        keys.sort();
        let mut canonical = None;
        for key in keys {
            let entry = dependencies
                .get(key)
                .expect("key came from the dependencies table");
            let package = package_override(key, entry)?;
            if key == name {
                match package {
                    Some(package) if package != name => wrong_override = true,
                    _ => canonical = Some(entry),
                }
            } else if package == Some(name) {
                return Err(format!(
                    "{CLI_MANIFEST_PATH} declares `{name}` under the renamed dependency key `{key}`; compose-types does not resolve renamed dependencies"
                ));
            }
        }

        if let Some(entry) = canonical {
            if let Some(table) = entry.as_table() {
                if table.get("workspace").and_then(toml::Value::as_bool) == Some(true) {
                    return Err(format!(
                        "{CLI_MANIFEST_PATH} inherits `{name}` from the workspace; compose-types does not resolve workspace dependencies"
                    ));
                }
                if table.get("optional").and_then(toml::Value::as_bool) == Some(true) {
                    let linked = activated.contains(name);
                    return Ok(DependencyStatus {
                        linked,
                        detail: if linked {
                            "optional dependency activated by the default features"
                        } else {
                            "optional dependency not activated by the default feature set"
                        },
                    });
                }
            }
            return Ok(DependencyStatus {
                linked: true,
                detail: "normal dependency",
            });
        }
    }

    if wrong_override {
        return Ok(DependencyStatus {
            linked: false,
            detail: "its dependency key overrides `package` to another package",
        });
    }

    if table_has(manifest, "dev-dependencies", name) {
        return Ok(DependencyStatus {
            linked: false,
            detail: "only a dev-dependency",
        });
    }
    if table_has(manifest, "build-dependencies", name) {
        return Ok(DependencyStatus {
            linked: false,
            detail: "only a build-dependency",
        });
    }
    Ok(DependencyStatus {
        linked: false,
        detail: "not a normal runtime dependency",
    })
}

fn target_dependency(manifest: &toml::Table, name: &str) -> Result<Option<TargetEntry>, String> {
    let Some(targets) = manifest.get("target").and_then(toml::Value::as_table) else {
        return Ok(None);
    };
    let mut target_names = targets.keys().collect::<Vec<_>>();
    target_names.sort();
    for target in target_names {
        let Some(table) = targets.get(target).and_then(toml::Value::as_table) else {
            continue;
        };
        for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
            let Some(entries) = table.get(section).and_then(toml::Value::as_table) else {
                continue;
            };
            let mut keys = entries.keys().collect::<Vec<_>>();
            keys.sort();
            for key in keys {
                let entry = entries.get(key).expect("key came from the target table");
                let package = package_override(key, entry)?;
                let relevant = match package {
                    Some(package) if package != name => false,
                    Some(_) => true,
                    None => key == name,
                };
                if relevant {
                    return Ok(Some(TargetEntry {
                        target: target.clone(),
                        section,
                        key: key.clone(),
                    }));
                }
            }
        }
    }
    Ok(None)
}

fn package_override<'a>(key: &str, entry: &'a toml::Value) -> Result<Option<&'a str>, String> {
    let Some(table) = entry.as_table() else {
        return Ok(None);
    };
    match table.get("package") {
        None => Ok(None),
        Some(package) => package.as_str().map(Some).ok_or_else(|| {
            format!("{CLI_MANIFEST_PATH} dependency `{key}` has a non-string `package` field")
        }),
    }
}

fn table_has(manifest: &toml::Table, table: &str, name: &str) -> bool {
    manifest
        .get(table)
        .and_then(toml::Value::as_table)
        .is_some_and(|entries| entries.contains_key(name))
}

fn default_activated_optional_dependencies(
    manifest: &toml::Table,
) -> Result<BTreeSet<String>, String> {
    let Some(features) = manifest.get("features").and_then(toml::Value::as_table) else {
        return Ok(BTreeSet::new());
    };
    let Some(default) = features.get("default") else {
        return Ok(BTreeSet::new());
    };
    let default = default
        .as_array()
        .ok_or_else(|| "features.default must be an array".to_string())?;
    let mut activated = BTreeSet::new();
    let mut visited = BTreeSet::new();
    for entry in default {
        let entry = entry
            .as_str()
            .ok_or_else(|| "feature entries must be strings".to_string())?;
        visit_feature(entry, features, &mut activated, &mut visited)?;
    }
    Ok(activated)
}

fn visit_feature(
    entry: &str,
    features: &toml::Table,
    activated: &mut BTreeSet<String>,
    visited: &mut BTreeSet<String>,
) -> Result<(), String> {
    if entry.is_empty() {
        return Err("feature entries must not be empty".to_string());
    }
    if let Some(dependency) = entry.strip_prefix("dep:") {
        activated.insert(dependency.to_string());
        return Ok(());
    }
    if entry.contains('?') {
        // `dep?/feature` does not enable `dep`.
        return Ok(());
    }
    if let Some((dependency, _feature)) = entry.split_once('/') {
        // `dep/feature` enables the optional dependency `dep`.
        activated.insert(dependency.to_string());
        return Ok(());
    }
    if !visited.insert(entry.to_string()) {
        return Ok(());
    }
    match features.get(entry) {
        None => {
            // An implicit optional-dependency feature, or a feature of a
            // non-optional dependency. Only the former is relevant here.
            activated.insert(entry.to_string());
            Ok(())
        }
        Some(value) => {
            let list = value
                .as_array()
                .ok_or_else(|| format!("feature `{entry}` must be an array"))?;
            for item in list {
                let item = item
                    .as_str()
                    .ok_or_else(|| format!("feature `{entry}` entries must be strings"))?;
                visit_feature(item, features, activated, visited)?;
            }
            Ok(())
        }
    }
}

/// Write `text` through a same-directory temporary file and an atomic replace.
///
/// Inputs are checked by file identity, not path spelling, so hard links and
/// symlinks to core, a fragment, or the manifest are rejected. The check is
/// repeated immediately before replacement to narrow the time-of-check window.
/// A failed write or replace leaves the existing output untouched and removes
/// the temporary file.
fn write_atomically(root: &Path, path: &Path, text: &str) -> Result<(), String> {
    ensure_output_is_not_input(root, path)?;

    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    // Preserve an existing output's permissions; a new output keeps the
    // temporary file's restrictive mode.
    let permissions = fs::metadata(path)
        .ok()
        .map(|metadata| metadata.permissions());
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|error| {
        format!(
            "cannot create a temporary file next to {}: {error}",
            path.display()
        )
    })?;
    if let Some(permissions) = permissions {
        temporary
            .as_file()
            .set_permissions(permissions)
            .map_err(|error| format!("cannot set permissions on {}: {error}", path.display()))?;
    }
    std::io::Write::write_all(&mut temporary, text.as_bytes())
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;

    // Recheck after the temporary file exists. Another process can still
    // change the directory entry after this point; the atomic replacement
    // never truncates an input inode, so its other links remain intact.
    ensure_output_is_not_input(root, path)?;
    temporary
        .persist(path)
        .map_err(|error| format!("cannot replace {}: {}", path.display(), error.error))?;
    Ok(())
}

fn ensure_output_is_not_input(root: &Path, output: &Path) -> Result<(), String> {
    let inputs = std::iter::once(CORE.path)
        .chain(OPTIONAL.iter().map(|fragment| fragment.path))
        .chain(std::iter::once(CLI_MANIFEST_PATH));
    for input in inputs {
        if same_file_identity(&root.join(input), output)? {
            return Err(format!(
                "refusing to write the composition over its input `{input}` (same file identity)"
            ));
        }
    }
    Ok(())
}

fn same_file_identity(a: &Path, b: &Path) -> Result<bool, String> {
    match same_file::is_same_file(a, b) {
        Ok(same) => Ok(same),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!(
            "cannot compare the file identity of `{}` with the output path: {error}",
            a.display()
        )),
    }
}
