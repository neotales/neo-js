//! Public API and behavior checks for the optional operating-system module.

use neojs_core::{
    ExtensionPermission, ModuleAllowlist, NeoJs, Permissions, RuntimeModule, RuntimeOptions,
};
use neojs_os::{MODULE_NAME, os_extension, with_os};

fn os_options() -> RuntimeOptions {
    with_os(RuntimeOptions::new()).unwrap()
}

#[test]
fn extension_declares_the_exact_module() {
    let extension = os_extension();
    assert_eq!(extension.id(), "neojs-os");
    assert_eq!(extension.modules().len(), 1);
    assert_eq!(extension.modules()[0].name(), MODULE_NAME);
    assert_eq!(
        extension.modules()[0].permission(),
        ExtensionPermission::None
    );
}

#[test]
fn registration_is_required_for_the_module_and_global() {
    let js = NeoJs::new(Permissions::default()).unwrap();
    assert_eq!(js.eval("typeof os").unwrap(), "undefined");
    let error = js
        .run_module("unregistered.js", "import 'neo:os';")
        .unwrap_err();
    assert!(error.contains("neo:os"), "{error}");
}

#[test]
fn os_extension_installs_no_other_authority() {
    let js = NeoJs::new_with_options(Permissions::default(), os_options()).unwrap();
    assert_eq!(
        js.eval(
            "typeof fs + ',' + typeof env + ',' + typeof process + ',' + \
             typeof __neoExec + ',' + typeof __neoExecSpawn + ',' + typeof __neoExecCall"
        )
        .unwrap(),
        "undefined,undefined,undefined,undefined,undefined,undefined"
    );
    for module in ["neo:fs", "neo:env", "neo:exec", "neo:winregistry"] {
        let error = js
            .run_module(
                &format!("denied-{module}.js"),
                &format!("import '{module}';"),
            )
            .unwrap_err();
        assert!(error.contains(module), "{error}");
    }
}

#[test]
fn module_and_global_share_the_private_snapshot() {
    let js = NeoJs::new_with_options(Permissions::default(), os_options()).unwrap();
    js.run_module(
        "os.js",
        "import {platform, arch, version, codename, variant} from 'neo:os';
         if (platform !== os.platform || arch !== os.arch) throw Error('platform mismatch');
         if (version !== os.version || codename !== os.codename || variant !== os.variant)
             throw Error('metadata mismatch');
         if (typeof platform !== 'string' || typeof arch !== 'string')
             throw Error('platform type');
         for (const value of [version, codename, variant]) {
             if (value !== undefined && typeof value !== 'string')
                 throw Error('metadata type');
         }",
    )
    .unwrap();
    assert_eq!(js.eval("typeof os").unwrap(), "object");
    assert_eq!(js.eval("os.platform").unwrap(), std::env::consts::OS);
    assert_eq!(js.eval("os.arch").unwrap(), std::env::consts::ARCH);
}

#[test]
fn explicit_allowlist_controls_both_the_module_and_the_global() {
    let options = with_os(RuntimeOptions::new())
        .unwrap()
        .modules([RuntimeModule::Path]);
    let js = NeoJs::new_with_options(Permissions::default(), options).unwrap();
    assert_eq!(js.eval("typeof os").unwrap(), "undefined");
    let static_error = js
        .run_module("denied-static.js", "import 'neo:os';")
        .unwrap_err();
    assert!(static_error.contains("allowlist"), "{static_error}");
    let dynamic_error = js
        .run_module("denied-dynamic.js", "await import('neo:os');")
        .unwrap_err();
    assert!(dynamic_error.contains("allowlist"), "{dynamic_error}");

    let options = with_os(RuntimeOptions::new())
        .unwrap()
        .module_names([MODULE_NAME])
        .unwrap();
    let js = NeoJs::new_with_options(Permissions::default(), options).unwrap();
    assert_eq!(js.eval("typeof os").unwrap(), "object");
    js.run_module(
        "allowed.js",
        "import {platform} from 'neo:os';
         if (platform !== os.platform) throw Error('global/module mismatch');",
    )
    .unwrap();
}

#[test]
fn module_exports_do_not_follow_mutable_global_reads() {
    let js = NeoJs::new_with_options(Permissions::default(), os_options()).unwrap();
    js.eval(
        "globalThis.originalOs = {platform: os.platform, arch: os.arch,
         version: os.version, codename: os.codename, variant: os.variant};
         os.platform = 'changed-platform'; os.arch = 'changed-arch';
         os.version = 'changed-version'; os.codename = 'changed-codename';
         os.variant = 'changed-variant';",
    )
    .unwrap();
    js.run_module(
        "snapshot.js",
        "import {platform, arch, version, codename, variant} from 'neo:os';
         const matchesSnapshot = () => platform === originalOs.platform &&
             arch === originalOs.arch && version === originalOs.version &&
             codename === originalOs.codename && variant === originalOs.variant;
         if (!matchesSnapshot()) throw Error('module read mutable global');
         os.platform = 'changed-again'; os.arch = 'changed-again';
         os.version = 'changed-again'; os.codename = 'changed-again';
         os.variant = 'changed-again';
         if (!matchesSnapshot()) throw Error('module exports changed with global');",
    )
    .unwrap();
    assert_eq!(
        js.eval(
            "String(os.platform === 'changed-again' && \
             originalOs.platform !== 'changed-platform')"
        )
        .unwrap(),
        "true"
    );
}

#[test]
fn registration_helpers_reject_duplicates_and_recognize_the_exact_name() {
    let duplicate = with_os(os_options()).unwrap_err();
    assert_eq!(duplicate.code(), "EXTENSION_DUPLICATE_EXTENSION");

    let error = os_options().module_names(["neo:os "]).unwrap_err();
    assert_eq!(error.names(), ["neo:os "]);

    let options = os_options();
    let named = ModuleAllowlist::from_names_with([MODULE_NAME], options.extensions()).unwrap();
    assert!(named.contains_extension_name(MODULE_NAME));
    let all = ModuleAllowlist::all_with_extensions(options.extensions());
    assert!(all.contains_extension_name(MODULE_NAME));
    assert!(all.contains(RuntimeModule::Fs));
}
