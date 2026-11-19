use neojs_core::{NeoJs, Permissions};

#[test]
fn typescript_module_names_are_stripped() {
    let js = NeoJs::new(Permissions::default()).unwrap();
    js.run_module(
        "main.ts",
        "type Value = number;
export const doubled: Value = 21 * 2;
if (doubled !== 42) throw Error('bad');
",
    )
    .unwrap();
}

#[test]
fn non_erasable_typescript_is_rejected_with_a_diagnostic() {
    let js = NeoJs::new(Permissions::default()).unwrap();
    let error = js
        .run_module("main.mts", "export enum Colour { Red }\n")
        .unwrap_err();
    assert!(error.contains("main.mts:1:8"), "{error}");
    assert!(error.contains("[TS_UNSUPPORTED]"), "{error}");
    assert!(!error.contains("Colour"), "{error}");
}

#[test]
fn javascript_module_names_are_not_stripped() {
    let js = NeoJs::new(Permissions::default()).unwrap();
    assert!(
        js.run_module("main.js", "const value: number = 1;")
            .is_err()
    );
}

#[test]
fn an_erased_type_import_conflicts_are_reported_before_evaluation() {
    let js = NeoJs::new(Permissions::default()).unwrap();
    let error = js
        .run_module(
            "main.ts",
            "import { type T } from './none.ts';\ntype T = number;\nthrow Error('executed');\n",
        )
        .unwrap_err();
    assert!(error.contains("main.ts:1:15"), "{error}");
    assert!(error.contains("[TS_SEMANTIC]"), "{error}");
    assert!(!error.contains("executed"), "{error}");
}

#[test]
fn an_entry_can_import_itself_under_an_alias_once() {
    let dir = tempfile::tempdir().unwrap();
    let entry = dir.path().join("self.ts");
    std::fs::write(
        &entry,
        "import '././self.ts';\nglobalThis.runs = (globalThis.runs ?? 0) + 1;\n",
    )
    .unwrap();
    let js = NeoJs::new(Permissions {
        filesystem: true,
        ..Permissions::default()
    })
    .unwrap();
    let name = entry.to_str().unwrap();
    js.run_module(name, &std::fs::read_to_string(&entry).unwrap())
        .unwrap();
    assert_eq!(js.eval("String(globalThis.runs)").unwrap(), "1");
}

#[test]
fn virtual_entry_requires_permission_for_relative_imports() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("dep.js"), "globalThis.loaded = true;").unwrap();
    let entry = dir.path().join("virtual.js");
    let name = entry.to_str().unwrap();
    let denied = NeoJs::new(Permissions::default()).unwrap();
    assert!(denied.run_module(name, "import './dep.js';").is_err());
    assert_eq!(
        denied.eval("typeof globalThis.loaded").unwrap(),
        "undefined"
    );
    let allowed = NeoJs::new(Permissions {
        filesystem: true,
        ..Permissions::default()
    })
    .unwrap();
    allowed.run_module(name, "import './dep.js';").unwrap();
    assert_eq!(allowed.eval("String(globalThis.loaded)").unwrap(), "true");
    assert!(allowed.run_module(name, "import './missing.js';").is_err());
}

#[cfg(unix)]
#[test]
fn canonical_entry_with_non_utf8_target_is_an_error() {
    use std::{
        ffi::OsString,
        os::unix::{ffi::OsStringExt, fs::symlink},
    };
    let dir = tempfile::tempdir().unwrap();
    let target = dir
        .path()
        .join(OsString::from_vec(b"non-utf8-\xff.js".to_vec()));
    std::fs::write(&target, "").unwrap();
    let alias = dir.path().join("alias.js");
    symlink(&target, &alias).unwrap();
    let allowed = NeoJs::new(Permissions {
        filesystem: true,
        ..Permissions::default()
    })
    .unwrap();
    let error = allowed.run_module(alias.to_str().unwrap(), "").unwrap_err();
    assert!(error.contains("not valid UTF-8"), "{error}");
    let denied = NeoJs::new(Permissions::default()).unwrap();
    denied.run_module(alias.to_str().unwrap(), "").unwrap();
}

#[cfg(unix)]
#[test]
fn library_symlink_entry_uses_requested_extension_with_or_without_permission() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target.js");
    std::fs::write(&target, "").unwrap();
    let alias = dir.path().join("alias.ts");
    symlink(&target, &alias).unwrap();
    for filesystem in [false, true] {
        let js = NeoJs::new(Permissions {
            filesystem,
            ..Permissions::default()
        })
        .unwrap();
        js.run_module(alias.to_str().unwrap(), "const x: number = 1;")
            .unwrap();
    }
}

#[test]
fn repeated_entry_identity_is_rejected_and_original_exports_survive() {
    let dir = tempfile::tempdir().unwrap();
    let entry = dir.path().join("entry.js");
    std::fs::write(&entry, "export const value = 99;").unwrap();
    let name = entry.to_str().unwrap();
    let js = NeoJs::new(Permissions {
        filesystem: true,
        ..Permissions::default()
    })
    .unwrap();
    js.run_module(name, "export const value = 1;").unwrap();
    assert!(js.run_module(name, "export const value = 2;").is_err());
    js.run_module(
        dir.path().join("importer.js").to_str().unwrap(),
        "import { value } from './entry.js'; if (value !== 1) throw Error('wrong export');",
    )
    .unwrap();
}

#[test]
fn failed_entry_parse_releases_identity_but_evaluation_failure_keeps_it() {
    let js = NeoJs::new(Permissions::default()).unwrap();
    assert!(
        js.run_module_with_identity("bad.js", "id", "export const x: number = 1;")
            .is_err()
    );
    js.run_module_with_identity("good.ts", "id", "export const x: number = 1;")
        .unwrap();
    assert!(
        js.run_module_with_identity("throw.js", "failed", "throw Error('failure');")
            .is_err()
    );
    let error = js
        .run_module_with_identity("retry.ts", "failed", "export const x = 1;")
        .unwrap_err();
    assert!(error.contains("already registered"), "{error}");
}

#[test]
fn builtin_names_cannot_be_used_as_entry_identities() {
    let js = NeoJs::new(Permissions::default()).unwrap();
    for name in ["neo:os", "neo:test", "neo:anything"] {
        assert!(js.run_module(name, "export const forged = true;").is_err());
        assert!(js.run_module_with_identity("ok.js", name, "").is_err());
    }
    js.run_module(
        "valid.js",
        "import {platform} from 'neo:os'; if (!platform) throw Error('missing builtin');",
    )
    .unwrap();
}
