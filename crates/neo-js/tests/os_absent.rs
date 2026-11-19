//! Core-only checks that operating-system metadata is not built in.
//!
//! The optional `neojs-os` crate registers `neo:os` and installs the `os`
//! global through the public extension API. These tests use only `neojs-core`,
//! so they also show that neither exists unless a host registers the crate.

use neojs_core::{ModuleAllowlist, NeoJs, Permissions, RuntimeModule, RuntimeOptions};

#[test]
fn os_is_not_a_builtin_module_name() {
    assert_eq!(RuntimeModule::from_name("neo:os"), None);
    assert_eq!(RuntimeModule::from_name("neo:OS"), None);
    assert!(
        !RuntimeModule::ALL
            .iter()
            .any(|module| module.name() == "neo:os")
    );
    assert!(!ModuleAllowlist::all().contains_extension_name("neo:os"));
}

#[test]
fn a_core_runtime_has_neither_the_os_module_nor_its_global() {
    // Neither restricted nor unrestricted core options know the module, and
    // the global is not installed. No permission flag is involved.
    for options in [
        RuntimeOptions::default(),
        RuntimeOptions::with_allowlist(ModuleAllowlist::none()),
    ] {
        let js = NeoJs::new_with_options(Permissions::default(), options).unwrap();
        assert_eq!(js.eval("typeof os").unwrap(), "undefined");
        let static_error = js
            .run_module("absent-static.js", "import 'neo:os';")
            .unwrap_err();
        assert!(static_error.contains("neo:os"), "{static_error}");
        let dynamic_error = js
            .run_module("absent-dynamic.js", "await import('neo:os');")
            .unwrap_err();
        assert!(dynamic_error.contains("neo:os"), "{dynamic_error}");
    }
}

#[test]
fn an_unregistered_os_name_is_rejected_by_allowlist_builders() {
    let error = ModuleAllowlist::from_names(["neo:os"]).unwrap_err();
    assert_eq!(error.names(), ["neo:os"]);

    let error = RuntimeOptions::new().module_names(["neo:os"]).unwrap_err();
    assert_eq!(error.names(), ["neo:os"]);
}
