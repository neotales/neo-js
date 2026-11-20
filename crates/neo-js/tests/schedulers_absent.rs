//! Core must not provide or recognize the optional scheduler generators.
//!
//! `neojs-schedulers` registers them as extension modules. A runtime that does
//! not register that extension has no built-in variant, no allowlist name, and
//! no importable module.

use neojs_core::{ModuleAllowlist, NeoJs, Permissions, RuntimeModule, RuntimeOptions};

const MODULE_NAMES: [&str; 3] = [
    "neo:os/schedulers/systemd",
    "neo:os/schedulers/windows",
    "neo:scheduler",
];

#[test]
fn core_has_no_builtin_scheduler_modules() {
    for name in MODULE_NAMES {
        assert_eq!(RuntimeModule::from_name(name), None, "{name}");
        assert!(
            !RuntimeModule::ALL
                .iter()
                .any(|module| module.name() == name),
            "core still lists {name} as a built-in module"
        );
    }
    assert!(!RuntimeOptions::default().is_module_name_allowed(MODULE_NAMES[0]));
}

#[test]
fn scheduler_names_are_unknown_without_a_registered_extension() {
    let error = ModuleAllowlist::from_names(MODULE_NAMES).unwrap_err();
    assert_eq!(error.names(), MODULE_NAMES);

    let error = RuntimeOptions::new()
        .module_names(MODULE_NAMES)
        .unwrap_err();
    assert_eq!(error.names(), MODULE_NAMES);

    let js = NeoJs::new(Permissions::default()).unwrap();
    for (index, name) in MODULE_NAMES.into_iter().enumerate() {
        let error = js
            .run_module(&format!("absent-{index}.js"), &format!("import '{name}';"))
            .unwrap_err();
        assert!(error.contains(name), "{error}");
    }
}
