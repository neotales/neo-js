//! Core-only checks that SQLite is not part of the built-in runtime.
//!
//! The optional `neojs-sqlite` crate registers `neo:sqlite` through the public
//! extension API. These tests use only `neojs-core`, so they also show that the
//! SQLite name is absent unless a host registers it.

use neojs_core::{ModuleAllowlist, NeoJs, Permissions, RuntimeModule, RuntimeOptions};

#[test]
fn sqlite_is_not_a_builtin_module_name() {
    assert_eq!(RuntimeModule::from_name("neo:sqlite"), None);
    assert_eq!(RuntimeModule::from_name("neo:SQLite"), None);
    assert!(!ModuleAllowlist::all().contains_extension_name("neo:sqlite"));
}

#[test]
fn granting_sqlite_permission_alone_installs_nothing() {
    let permissions = Permissions {
        sqlite: true,
        ..Permissions::default()
    };
    for options in [
        RuntimeOptions::default(),
        RuntimeOptions::with_allowlist(ModuleAllowlist::none()),
    ] {
        let js = NeoJs::new_with_options(permissions, options).unwrap();
        let static_error = js
            .run_module("absent-static.js", "import 'neo:sqlite';")
            .unwrap_err();
        assert!(static_error.contains("neo:sqlite"), "{static_error}");
        let dynamic_error = js
            .run_module("absent-dynamic.js", "await import('neo:sqlite');")
            .unwrap_err();
        assert!(dynamic_error.contains("neo:sqlite"), "{dynamic_error}");
    }
}

#[test]
fn an_unregistered_sqlite_name_is_rejected_by_allowlist_builders() {
    let error = ModuleAllowlist::from_names(["neo:sqlite"]).unwrap_err();
    assert_eq!(error.names(), ["neo:sqlite"]);

    let error = RuntimeOptions::new()
        .module_names(["neo:sqlite"])
        .unwrap_err();
    assert_eq!(error.names(), ["neo:sqlite"]);
}
