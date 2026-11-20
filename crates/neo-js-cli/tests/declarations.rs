//! Tests for the selectable, deterministic declaration composition.
//!
//! The example command is:
//!
//! ```sh
//! cargo run --offline -p neojs --example compose-types -- --with os --with os-user --with schedulers \
//!   --with sqlite --check-cli-manifest --check types/neo-js.d.ts
//! ```

#[path = "../src/declarations.rs"]
mod declarations;

use std::{
    fs,
    path::{Path, PathBuf},
};

fn root() -> PathBuf {
    declarations::default_root()
}

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

fn read(relative: &str) -> String {
    fs::read_to_string(root().join(relative))
        .unwrap_or_else(|error| panic!("cannot read {relative}: {error}"))
}

/// A temporary directory on the checkout's filesystem, so hard links to the
/// declaration inputs can be created.
fn workdir() -> tempfile::TempDir {
    let base = root().join("target/tmp");
    fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new()
        .prefix("neo-js-declarations-")
        .tempdir_in(&base)
        .unwrap()
}

fn declared_modules(source: &str) -> Vec<&str> {
    source
        .lines()
        .filter_map(|line| {
            line.strip_prefix("declare module \"")
                .and_then(|rest| rest.strip_suffix("\" {"))
        })
        .collect()
}

fn assert_no_duplicate_modules(source: &str) {
    let modules = declared_modules(source);
    let mut unique = modules.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(
        modules.len(),
        unique.len(),
        "composition contains duplicate module declarations"
    );
}

#[test]
fn checked_in_cli_composition_is_current() {
    let expected =
        declarations::compose(&root(), &["os", "os-user", "schedulers", "sqlite"]).unwrap();
    let actual = read("types/neo-js.d.ts");
    assert_eq!(
        actual, expected,
        "types/neo-js.d.ts is stale; regenerate with \
         cargo run --offline -p neojs --example compose-types -- \
         --with os --with os-user --with schedulers --with sqlite --check-cli-manifest -- \
         --output types/neo-js.d.ts"
    );
}

#[test]
fn core_declarations_omit_optional_modules() {
    let core = read(declarations::CORE_PATH);
    for name in [
        "neo:os",
        "neo:os/user",
        "neo:os/schedulers/systemd",
        "neo:os/schedulers/windows",
        "neo:scheduler",
        "neo:sqlite",
    ] {
        assert!(
            !declared_modules(&core).contains(&name),
            "core declarations must not declare the optional {name} module"
        );
    }

    let core_only = declarations::compose(&root(), &[]).unwrap();
    for name in [
        "neo:os",
        "neo:os/user",
        "neo:os/schedulers/systemd",
        "neo:os/schedulers/windows",
        "neo:scheduler",
        "neo:sqlite",
    ] {
        assert!(
            !declared_modules(&core_only).contains(&name),
            "a core-only composition must not declare {name}"
        );
    }
    assert!(
        !core_only.contains("neo-os.d.ts"),
        "a core-only composition must not name the optional OS fragment"
    );
    assert!(
        !core_only.contains("neo-schedulers.d.ts"),
        "a core-only composition must not name the optional scheduler fragment"
    );
    assert!(
        !core_only.contains("neo-sqlite.d.ts"),
        "a core-only composition must not name the optional fragment"
    );
    assert!(
        core_only.contains("// Selection: core\n"),
        "a core-only composition must name only the core selection"
    );
}

#[test]
fn enabling_os_user_does_not_enable_os_metadata() {
    let core = declarations::compose(&root(), &[]).unwrap();
    let composed = declarations::compose(&root(), &["os-user"]).unwrap();
    let mut expected = declared_modules(&core);
    expected.push("neo:os/user");
    assert_eq!(declared_modules(&composed), expected);
    assert!(!composed.contains("declare const os:"));
    assert!(composed.contains("// Selection: core, os-user\n"));
}

#[test]
fn enabling_sqlite_adds_exactly_one_module_declaration() {
    let composed = declarations::compose(&root(), &["sqlite"]).unwrap();
    assert_eq!(
        declared_modules(&composed)
            .iter()
            .filter(|module| **module == "neo:sqlite")
            .count(),
        1,
        "the SQLite selection must declare neo:sqlite exactly once"
    );
    assert!(composed.contains("// Selection: core, sqlite\n"));
    assert_no_duplicate_modules(&composed);
    assert_no_duplicate_modules(&read(declarations::CORE_PATH));
}

#[test]
fn enabling_os_adds_exactly_one_module_declaration_and_the_global() {
    let composed = declarations::compose(&root(), &["os"]).unwrap();
    assert_eq!(
        declared_modules(&composed)
            .iter()
            .filter(|module| **module == "neo:os")
            .count(),
        1,
        "the OS selection must declare neo:os exactly once"
    );
    assert!(composed.contains("// Selection: core, os\n"));
    assert!(composed.contains("declare const os:"));
    assert_no_duplicate_modules(&composed);
    assert!(
        !declared_modules(&read(declarations::CORE_PATH)).contains(&"neo:os"),
        "the OS fragment must own neo:os"
    );
}

#[test]
fn enabling_schedulers_adds_the_three_specifiers_once() {
    let composed = declarations::compose(&root(), &["schedulers"]).unwrap();
    let modules = declared_modules(&composed);
    for name in [
        "neo:os/schedulers/systemd",
        "neo:os/schedulers/windows",
        "neo:scheduler",
    ] {
        assert_eq!(
            modules.iter().filter(|module| **module == name).count(),
            1,
            "the scheduler selection must declare {name} exactly once"
        );
    }
    assert!(composed.contains("// Selection: core, schedulers\n"));
    assert_no_duplicate_modules(&composed);
}

#[test]
fn composition_is_deterministic_and_offline() {
    let first = declarations::compose(&root(), &["os", "os-user", "schedulers", "sqlite"]).unwrap();
    let second =
        declarations::compose(&root(), &["os", "os-user", "schedulers", "sqlite"]).unwrap();
    assert_eq!(first, second, "composition must not depend on run order");
    assert!(
        !first.contains(env!("CARGO_MANIFEST_DIR")),
        "composition must not embed the build-time checkout path"
    );
    assert!(
        !first.contains("Generated at"),
        "composition must not embed a timestamp"
    );
}

#[test]
fn run_supports_stdout_and_an_explicit_output_path() {
    let expected = declarations::compose(&root(), &["sqlite"]).unwrap();
    assert_eq!(
        declarations::run(args(&["--with", "sqlite"])).unwrap(),
        Some(expected.clone())
    );
    assert_eq!(
        declarations::run(args(&["--with", "sqlite", "--output", "-"])).unwrap(),
        Some(expected.clone())
    );

    let dir = workdir();
    let sentinel = dir.path().join("keep.txt");
    fs::write(&sentinel, "keep").unwrap();
    let output = dir.path().join("neo-js.d.ts");
    let result = declarations::run(args(&[
        "--with",
        "sqlite",
        "--output",
        output.to_str().unwrap(),
    ]))
    .unwrap();
    assert_eq!(result, None);
    assert_eq!(fs::read_to_string(&output).unwrap(), expected);
    assert_eq!(fs::read_to_string(&sentinel).unwrap(), "keep");
    assert_eq!(
        fs::read_dir(dir.path()).unwrap().count(),
        2,
        "composition must write only the requested output path"
    );
}

#[test]
fn output_never_overwrites_an_input() {
    let core_path = root().join(declarations::CORE_PATH);
    let before = fs::read(&core_path).unwrap();
    let error = declarations::run(args(&["--output", core_path.to_str().unwrap()])).unwrap_err();
    assert!(
        error.contains("refusing to write the composition over its input"),
        "{error}"
    );
    assert_eq!(fs::read(&core_path).unwrap(), before);
}

fn hard_link_or_report(source: &Path, alias: &Path) -> bool {
    match fs::hard_link(source, alias) {
        Ok(()) => true,
        Err(error) if hard_link_unavailable(&error) => {
            eprintln!("hard-link identity test not run on this filesystem: {error}");
            false
        }
        Err(error) => panic!("cannot create hard link for test: {error}"),
    }
}

fn hard_link_unavailable(error: &std::io::Error) -> bool {
    if error.kind() == std::io::ErrorKind::CrossesDevices {
        return true;
    }
    if !cfg!(windows) {
        return false;
    }
    matches!(
        error.kind(),
        std::io::ErrorKind::Unsupported | std::io::ErrorKind::PermissionDenied
    ) || matches!(
        error.raw_os_error(),
        Some(1) | Some(5) | Some(17) | Some(50) | Some(1314)
    )
}

#[test]
fn rejects_hard_link_aliases_of_inputs() {
    let dir = workdir();
    let inputs = [
        declarations::CORE_PATH,
        "crates/neojs-os/types/neo-os.d.ts",
        "crates/neojs-schedulers/types/neo-schedulers.d.ts",
        "crates/neojs-sqlite/types/neo-sqlite.d.ts",
        declarations::CLI_MANIFEST_PATH,
    ];
    for input in inputs {
        let alias = dir.path().join(input.rsplit('/').next().unwrap());
        if !hard_link_or_report(&root().join(input), &alias) {
            continue;
        }
        let before = fs::read(root().join(input)).unwrap();
        let error = declarations::run(args(&["--output", alias.to_str().unwrap()])).unwrap_err();
        assert!(
            error.contains("same file identity"),
            "hard-link alias `{input}` was not rejected: {error}"
        );
        assert_eq!(fs::read(root().join(input)).unwrap(), before);
        assert_eq!(fs::read(&alias).unwrap(), before);
    }
}

#[test]
fn output_replacement_preserves_inputs_and_old_output_on_failure() {
    let dir = workdir();
    let output = dir.path().join("out.d.ts");
    fs::write(&output, "old\n").unwrap();

    let core_before = fs::read(root().join(declarations::CORE_PATH)).unwrap();
    let fragment_before =
        fs::read(root().join("crates/neojs-sqlite/types/neo-sqlite.d.ts")).unwrap();
    let manifest_before = fs::read(root().join(declarations::CLI_MANIFEST_PATH)).unwrap();
    let expected = declarations::compose(&root(), &["sqlite"]).unwrap();

    assert_eq!(
        declarations::run(args(&[
            "--with",
            "sqlite",
            "--output",
            output.to_str().unwrap()
        ]))
        .unwrap(),
        None
    );
    assert_eq!(fs::read_to_string(&output).unwrap(), expected);
    assert_eq!(
        fs::read(root().join(declarations::CORE_PATH)).unwrap(),
        core_before
    );
    assert_eq!(
        fs::read(root().join("crates/neojs-sqlite/types/neo-sqlite.d.ts")).unwrap(),
        fragment_before
    );
    assert_eq!(
        fs::read(root().join(declarations::CLI_MANIFEST_PATH)).unwrap(),
        manifest_before
    );

    // A failed replacement leaves the target untouched and removes the
    // temporary file created next to it.
    let target_dir = dir.path().join("target-dir");
    fs::create_dir(&target_dir).unwrap();
    let error = declarations::run(args(&[
        "--with",
        "sqlite",
        "--output",
        target_dir.to_str().unwrap(),
    ]))
    .unwrap_err();
    assert!(error.contains("cannot replace"), "{error}");
    assert!(target_dir.is_dir());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
}

#[cfg(unix)]
#[test]
fn replacement_preserves_existing_output_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let dir = workdir();
    let output = dir.path().join("mode.d.ts");
    fs::write(&output, "old\n").unwrap();
    fs::set_permissions(&output, fs::Permissions::from_mode(0o640)).unwrap();

    declarations::run(args(&[
        "--with",
        "sqlite",
        "--output",
        output.to_str().unwrap(),
    ]))
    .unwrap();

    let mode = fs::metadata(&output).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o640);
}

#[test]
fn check_mode_never_writes() {
    let expected = declarations::compose(&root(), &["sqlite"]).unwrap();
    let dir = workdir();
    let current = dir.path().join("current.d.ts");
    fs::write(&current, &expected).unwrap();

    assert_eq!(
        declarations::run(args(&[
            "--with",
            "sqlite",
            "--check",
            current.to_str().unwrap()
        ]))
        .unwrap(),
        None
    );
    assert_eq!(fs::read_to_string(&current).unwrap(), expected);

    let stale = dir.path().join("stale.d.ts");
    fs::write(&stale, "stale\n").unwrap();
    let error = declarations::run(args(&[
        "--with",
        "sqlite",
        "--check",
        stale.to_str().unwrap(),
    ]))
    .unwrap_err();
    assert!(error.contains("is stale"), "{error}");
    assert_eq!(fs::read_to_string(&stale).unwrap(), "stale\n");
}

#[test]
fn rejects_unknown_and_duplicate_selections() {
    let unknown = declarations::run(args(&["--with", "nope"])).unwrap_err();
    assert!(
        unknown.contains("unknown optional fragment `nope`"),
        "{unknown}"
    );
    assert!(
        unknown.contains("available: os, os-user, schedulers, sqlite"),
        "{unknown}"
    );

    let duplicate = declarations::run(args(&["--with", "sqlite", "--with", "sqlite"])).unwrap_err();
    assert!(
        duplicate.contains("duplicate --with `sqlite`"),
        "{duplicate}"
    );

    let core = declarations::run(args(&["--with", "core"])).unwrap_err();
    assert!(core.contains("unknown optional fragment `core`"), "{core}");
}

#[test]
fn rejects_conflicting_and_incomplete_arguments() {
    let conflicting = declarations::run(args(&[
        "--with", "sqlite", "--output", "out.d.ts", "--check", "out.d.ts",
    ]))
    .unwrap_err();
    assert!(conflicting.contains("mutually exclusive"), "{conflicting}");

    let missing = declarations::run(args(&["--with"])).unwrap_err();
    assert!(
        missing.contains("--with needs a non-empty value"),
        "{missing}"
    );

    let unknown_flag = declarations::run(args(&["--wat"])).unwrap_err();
    assert!(
        unknown_flag.contains("unknown argument `--wat`"),
        "{unknown_flag}"
    );
}

fn manifest_fixture() -> tempfile::TempDir {
    let dir = workdir();
    fs::create_dir_all(dir.path().join("types")).unwrap();
    fs::create_dir_all(dir.path().join("crates/neojs-os/types")).unwrap();
    fs::create_dir_all(dir.path().join("crates/neojs-schedulers/types")).unwrap();
    fs::create_dir_all(dir.path().join("crates/neojs-sqlite/types")).unwrap();
    fs::create_dir_all(dir.path().join("crates/neo-js-cli")).unwrap();
    fs::write(
        dir.path().join(declarations::CORE_PATH),
        "declare module \"neo:core\" {}\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("crates/neojs-os/types/neo-os.d.ts"),
        "declare module \"neo:os\" {}\ndeclare const os: { platform: string };\n",
    )
    .unwrap();
    fs::write(
        dir.path()
            .join("crates/neojs-schedulers/types/neo-schedulers.d.ts"),
        "declare module \"neo:os/schedulers/systemd\" {}\n\
         declare module \"neo:os/schedulers/windows\" {}\n\
         declare module \"neo:scheduler\" {}\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("crates/neojs-sqlite/types/neo-sqlite.d.ts"),
        "declare module \"neo:sqlite\" {}\n",
    )
    .unwrap();
    dir
}

fn write_manifest(dir: &Path, body: &str) {
    fs::write(dir.join(declarations::CLI_MANIFEST_PATH), body).unwrap();
}

fn run_with_root(root: &Path, extra: &[&str]) -> Result<Option<String>, String> {
    let mut values = vec!["--root".to_string(), root.to_str().unwrap().to_string()];
    values.extend(extra.iter().map(|value| value.to_string()));
    declarations::run(values)
}

const CORE_DEPENDENCY: &str = "[dependencies]\nneojs-core = { path = \"../neo-js\" }\n";
const OS_DEPENDENCY: &str = "neojs-os = { path = \"../neojs-os\" }\n";
const SCHEDULERS_DEPENDENCY: &str = "neojs-schedulers = { path = \"../neojs-schedulers\" }\n";
const SQLITE_DEPENDENCY: &str = "neojs-sqlite = { path = \"../neojs-sqlite\" }\n";

#[test]
fn manifest_check_requires_both_directions() {
    let dir = manifest_fixture();
    let root = dir.path();

    write_manifest(root, CORE_DEPENDENCY);
    assert!(
        run_with_root(root, &["--check-cli-manifest"]).is_ok(),
        "core-only selection matches a manifest without the optional dependency"
    );
    let error = run_with_root(root, &["--with", "sqlite", "--check-cli-manifest"]).unwrap_err();
    assert!(error.contains("does not link `neojs-sqlite`"), "{error}");

    write_manifest(root, &format!("{CORE_DEPENDENCY}{SQLITE_DEPENDENCY}"));
    let error = run_with_root(root, &["--check-cli-manifest"]).unwrap_err();
    assert!(
        error.contains("links `neojs-sqlite`")
            && error.contains("omits `sqlite`")
            && error.contains("add `--with sqlite`"),
        "{error}"
    );
    assert!(
        run_with_root(root, &["--with", "sqlite", "--check-cli-manifest"]).is_ok(),
        "selecting the dependency's fragment matches the manifest"
    );
}

#[test]
fn manifest_check_covers_the_os_fragment() {
    let dir = manifest_fixture();
    let root = dir.path();

    write_manifest(root, &format!("{CORE_DEPENDENCY}{OS_DEPENDENCY}"));
    let error = run_with_root(root, &["--check-cli-manifest"]).unwrap_err();
    assert!(
        error.contains("links `neojs-os`")
            && error.contains("omits `os`")
            && error.contains("add `--with os`"),
        "{error}"
    );
    assert!(
        run_with_root(root, &["--with", "os", "--check-cli-manifest"]).is_ok(),
        "selecting the OS fragment matches a manifest that links neojs-os"
    );

    write_manifest(root, CORE_DEPENDENCY);
    let error = run_with_root(root, &["--with", "os", "--check-cli-manifest"]).unwrap_err();
    assert!(error.contains("does not link `neojs-os`"), "{error}");
}

#[test]
fn manifest_check_covers_the_schedulers_fragment() {
    let dir = manifest_fixture();
    let root = dir.path();

    write_manifest(root, &format!("{CORE_DEPENDENCY}{SCHEDULERS_DEPENDENCY}"));
    let error = run_with_root(root, &["--check-cli-manifest"]).unwrap_err();
    assert!(
        error.contains("links `neojs-schedulers`")
            && error.contains("omits `schedulers`")
            && error.contains("add `--with schedulers`"),
        "{error}"
    );
    assert!(
        run_with_root(root, &["--with", "schedulers", "--check-cli-manifest"]).is_ok(),
        "selecting the scheduler fragment matches a manifest that links neojs-schedulers"
    );

    write_manifest(root, CORE_DEPENDENCY);
    let error = run_with_root(root, &["--with", "schedulers", "--check-cli-manifest"]).unwrap_err();
    assert!(
        error.contains("does not link `neojs-schedulers`"),
        "{error}"
    );
}

#[test]
fn manifest_check_ignores_dev_dependencies() {
    let dir = manifest_fixture();
    let root = dir.path();
    write_manifest(
        root,
        &format!("{CORE_DEPENDENCY}\n[dev-dependencies]\n{SQLITE_DEPENDENCY}"),
    );

    assert!(
        run_with_root(root, &["--check-cli-manifest"]).is_ok(),
        "a dev-dependency must not make the core-only selection inconsistent"
    );
    let error = run_with_root(root, &["--with", "sqlite", "--check-cli-manifest"]).unwrap_err();
    assert!(
        error.contains("does not link `neojs-sqlite`") && error.contains("only a dev-dependency"),
        "{error}"
    );
}

#[test]
fn manifest_check_honors_default_optional_features() {
    let dir = manifest_fixture();
    let root = dir.path();
    write_manifest(
        root,
        &format!(
            "{CORE_DEPENDENCY}neojs-sqlite = {{ path = \"../neojs-sqlite\", optional = true }}\n"
        ),
    );

    assert!(
        run_with_root(root, &["--check-cli-manifest"]).is_ok(),
        "an optional dependency outside the default features is not linked"
    );
    let error = run_with_root(root, &["--with", "sqlite", "--check-cli-manifest"]).unwrap_err();
    assert!(
        error.contains("optional dependency not activated by the default feature set"),
        "{error}"
    );

    write_manifest(
        root,
        &format!(
            "{CORE_DEPENDENCY}neojs-sqlite = {{ path = \"../neojs-sqlite\", optional = true }}\n\
             \n[features]\ndefault = [\"dep:neojs-sqlite\"]\n"
        ),
    );
    assert!(
        run_with_root(root, &["--with", "sqlite", "--check-cli-manifest"]).is_ok(),
        "a default feature activating the optional dependency links it"
    );
    let error = run_with_root(root, &["--check-cli-manifest"]).unwrap_err();
    assert!(
        error.contains("links `neojs-sqlite`")
            && error.contains("optional dependency activated by the default features"),
        "{error}"
    );

    write_manifest(
        root,
        &format!(
            "{CORE_DEPENDENCY}neojs-sqlite = {{ path = \"../neojs-sqlite\", optional = true }}\n\
             \n[features]\ndefault = [\"neojs-sqlite\"]\n"
        ),
    );
    assert!(
        run_with_root(root, &["--with", "sqlite", "--check-cli-manifest"]).is_ok(),
        "the implicit optional-dependency feature links the dependency"
    );

    write_manifest(
        root,
        &format!(
            "{CORE_DEPENDENCY}neojs-sqlite = {{ path = \"../neojs-sqlite\", optional = true }}\n\
             \n[features]\ndefault = [\"sqlite-support\"]\n\
             sqlite-support = [\"dep:neojs-sqlite\"]\n"
        ),
    );
    assert!(
        run_with_root(root, &["--with", "sqlite", "--check-cli-manifest"]).is_ok(),
        "a default feature must be followed through nested feature entries"
    );
}

#[test]
fn manifest_check_rejects_target_specific_dependencies() {
    let dir = manifest_fixture();
    let root = dir.path();
    write_manifest(
        root,
        &format!("{CORE_DEPENDENCY}\n[target.'cfg(windows)'.dependencies]\n{SQLITE_DEPENDENCY}"),
    );

    let error = run_with_root(root, &["--check-cli-manifest"]).unwrap_err();
    assert!(
        error.contains("for target `cfg(windows)`")
            && error.contains("does not resolve target-specific dependency tables"),
        "{error}"
    );
}

#[test]
fn manifest_check_rejects_target_entries_before_normal_resolution() {
    let dir = manifest_fixture();
    let root = dir.path();
    let selections: [&[&str]; 2] = [
        &["--check-cli-manifest"],
        &["--with", "sqlite", "--check-cli-manifest"],
    ];

    // An inactive optional normal dependency must not hide a non-optional
    // target-specific entry.
    write_manifest(
        root,
        &format!(
            "{CORE_DEPENDENCY}neojs-sqlite = {{ path = \"../neojs-sqlite\", optional = true }}\n\n[target.'cfg(windows)'.dependencies]\n{SQLITE_DEPENDENCY}"
        ),
    );
    for selection in selections {
        let error = run_with_root(root, selection).unwrap_err();
        assert!(error.contains("for target `cfg(windows)`"), "{error}");
        assert!(
            error.contains("does not resolve target-specific dependency tables"),
            "{error}"
        );
    }

    // A normal dependency must not hide a target-specific entry either.
    write_manifest(
        root,
        &format!(
            "{CORE_DEPENDENCY}{SQLITE_DEPENDENCY}\n[target.'cfg(windows)'.dependencies]\n{SQLITE_DEPENDENCY}"
        ),
    );
    for selection in selections {
        let error = run_with_root(root, selection).unwrap_err();
        assert!(error.contains("for target `cfg(windows)`"), "{error}");
        assert!(
            error.contains("does not resolve target-specific dependency tables"),
            "{error}"
        );
    }
}

#[test]
fn manifest_check_rejects_renamed_target_package() {
    let dir = manifest_fixture();
    let root = dir.path();
    write_manifest(
        root,
        &format!(
            "{CORE_DEPENDENCY}\n[target.'cfg(windows)'.dependencies]\n\
             sqlite-alias = {{ package = \"neojs-sqlite\", path = \"../neojs-sqlite\" }}\n"
        ),
    );

    for selection in [
        &["--check-cli-manifest"][..],
        &["--with", "sqlite", "--check-cli-manifest"][..],
    ] {
        let error = run_with_root(root, selection).unwrap_err();
        assert!(error.contains("sqlite-alias"), "{error}");
        assert!(error.contains("target `cfg(windows)`"), "{error}");
    }
}

#[test]
fn manifest_check_does_not_count_a_wrong_package_override() {
    let dir = manifest_fixture();
    let root = dir.path();

    write_manifest(
        root,
        &format!(
            "{CORE_DEPENDENCY}neojs-sqlite = {{ package = \"other-package\", path = \"../other\" }}\n"
        ),
    );
    assert!(
        run_with_root(root, &["--check-cli-manifest"]).is_ok(),
        "a wrong package override is not a SQLite dependency"
    );
    let error = run_with_root(root, &["--with", "sqlite", "--check-cli-manifest"]).unwrap_err();
    assert!(
        error.contains("does not link `neojs-sqlite`") && error.contains("overrides `package`"),
        "{error}"
    );

    // A wrong package override in a target table is not a SQLite target
    // entry, so it must not trigger the unresolved-target error.
    write_manifest(
        root,
        &format!(
            "{CORE_DEPENDENCY}\n[target.'cfg(windows)'.dependencies]\n\
             neojs-sqlite = {{ package = \"other-package\" }}\n"
        ),
    );
    assert!(
        run_with_root(root, &["--check-cli-manifest"]).is_ok(),
        "a wrong package override must not be treated as SQLite"
    );
    let error = run_with_root(root, &["--with", "sqlite", "--check-cli-manifest"]).unwrap_err();
    assert!(error.contains("does not link `neojs-sqlite`"), "{error}");
}

#[test]
fn manifest_check_rejects_renamed_dependencies() {
    let dir = manifest_fixture();
    let root = dir.path();
    write_manifest(
        root,
        &format!(
            "{CORE_DEPENDENCY}sqlite-alias = {{ package = \"neojs-sqlite\", path = \"../neojs-sqlite\" }}\n"
        ),
    );

    let error = run_with_root(root, &["--check-cli-manifest"]).unwrap_err();
    assert!(
        error.contains("renamed dependency key `sqlite-alias`"),
        "{error}"
    );
}

#[test]
fn manifest_check_rejects_unresolved_workspace_dependencies() {
    let dir = manifest_fixture();
    let root = dir.path();
    write_manifest(
        root,
        &format!("{CORE_DEPENDENCY}neojs-sqlite = {{ workspace = true }}\n"),
    );

    let error = run_with_root(root, &["--check-cli-manifest"]).unwrap_err();
    assert!(
        error.contains("does not resolve workspace dependencies"),
        "{error}"
    );
}

#[test]
fn manifest_check_reports_invalid_toml() {
    let dir = manifest_fixture();
    let root = dir.path();
    write_manifest(root, "[dependencies\n");

    let error = run_with_root(root, &["--check-cli-manifest"]).unwrap_err();
    assert!(error.contains("cannot parse"), "{error}");
}

#[test]
fn help_is_available_without_writing() {
    let help = declarations::run(args(&["--help"])).unwrap().unwrap();
    assert!(help.contains("Usage: compose-types"), "{help}");
    assert!(help.contains("--check-cli-manifest"), "{help}");
    assert!(help.contains("--output <PATH>"), "{help}");
}
