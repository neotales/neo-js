use neojs_core::{
    ExtensionPermission, ExtensionRegistrationError, NeoJs, Permissions, RuntimeModule,
    RuntimeOptions,
};
use neojs_os_user::{MODULE_NAME, os_user_extension, with_os_user};

fn user_permission() -> Permissions {
    Permissions {
        user: true,
        ..Permissions::default()
    }
}

#[test]
fn registration_permission_and_exact_allowlist_are_independent() {
    let registered = with_os_user(RuntimeOptions::new()).unwrap();
    let permitted = registered.clone().module_names([MODULE_NAME]).unwrap();
    let denied_cases = [
        (user_permission(), RuntimeOptions::new()),
        (user_permission(), registered.modules([RuntimeModule::Path])),
        (
            Permissions {
                environment: true,
                filesystem: true,
                process: true,
                stdio: true,
                registry: true,
                network: true,
                sqlite: true,
                user: false,
            },
            permitted.clone(),
        ),
    ];

    for (permissions, options) in denied_cases {
        let runtime = NeoJs::new_with_options(permissions, options).unwrap();
        let error = runtime
            .run_module("denied.js", "import 'neo:os/user';")
            .unwrap_err();
        assert!(error.contains(MODULE_NAME));
        runtime
            .run_module(
                "denied-dynamic.js",
                "let available = true;
                 try { await import('neo:os/user'); } catch { available = false; }
                 if (available) throw Error('unexpected identity authority');",
            )
            .unwrap();
    }

    // Check the surface without consulting the real account.
    let baseline =
        NeoJs::new_with_options(Permissions::default(), RuntimeOptions::new().modules([])).unwrap();
    let runtime = NeoJs::new_with_options(user_permission(), permitted).unwrap();
    let globals = "JSON.stringify(Object.getOwnPropertyNames(globalThis).sort())";
    let before = baseline.eval(globals).unwrap();
    assert_eq!(before, runtime.eval(globals).unwrap());
    runtime
        .run_module(
            "allow-user.js",
            "import * as api from 'neo:os/user';
             if (Object.keys(api).sort().join() !==
                 'accountGroupIdsSync,currentUserSync,homeDirSync,processIdsSync') {
                 throw Error('unexpected identity exports');
             }
             if (Object.values(api).some(fn => typeof fn !== 'function')) {
                 throw Error('identity exports must be functions');
             }
             if (typeof globalThis.os !== 'undefined' || typeof globalThis.process !== 'undefined') {
                 throw Error('unexpected identity global');
             }",
        )
        .unwrap();
    assert_eq!(before, runtime.eval(globals).unwrap());
    assert!(runtime.run_module("os.js", "import 'neo:os';").is_err());
}

#[test]
fn appended_alias_never_exposes_identity_exports() {
    const ALIAS: &str = "neo:identity-alias";
    let options = RuntimeOptions::new()
        .with_extension(os_user_extension().with_module(ALIAS, ExtensionPermission::None))
        .unwrap();
    for (permissions, names, canonical_allowed) in [
        (Permissions::default(), &[ALIAS, MODULE_NAME][..], false),
        (user_permission(), &[ALIAS][..], false),
        (user_permission(), &[ALIAS, MODULE_NAME][..], true),
    ] {
        let runtime = NeoJs::new_with_options(
            permissions,
            options.clone().module_names(names.iter().copied()).unwrap(),
        )
        .unwrap();
        let globals = "JSON.stringify(Object.getOwnPropertyNames(globalThis).sort())";
        let baseline = NeoJs::new_with_options(permissions, RuntimeOptions::new().modules([]))
            .unwrap()
            .eval(globals)
            .unwrap();
        assert_eq!(runtime.eval(globals).unwrap(), baseline);
        let canonical = runtime.run_module(
            "canonical.js",
            "import { currentUserSync } from 'neo:os/user';
             if (typeof currentUserSync !== 'function') throw Error('missing canonical export');",
        );
        assert_eq!(canonical.is_ok(), canonical_allowed);
        let alias = runtime
            .run_module("alias.js", "import 'neo:identity-alias';")
            .unwrap_err();
        assert!(alias.contains("identity module does not support aliases"));
        runtime
            .run_module(
                "dynamic-alias.js",
                "let available = true;
                 try { await import('neo:identity-alias'); } catch { available = false; }
                 if (available) throw Error('alias exposed identity authority');",
            )
            .unwrap();
        assert_eq!(runtime.eval(globals).unwrap(), baseline);
    }
}

#[test]
fn canonical_permission_cannot_be_overridden() {
    let error = RuntimeOptions::new()
        .with_extension(os_user_extension().with_module(MODULE_NAME, ExtensionPermission::None))
        .unwrap_err();
    assert_eq!(
        error,
        ExtensionRegistrationError::DuplicateModule {
            name: MODULE_NAME.into(),
        }
    );
}

#[test]
fn replacing_initialization_does_not_open_the_canonical_loader() {
    let options = RuntimeOptions::new()
        .with_extension(os_user_extension().with_init(|_, _| Ok(Box::new(()))))
        .unwrap();
    let runtime = NeoJs::new_with_options(user_permission(), options).unwrap();
    let error = runtime
        .run_module("replaced-init.js", "import 'neo:os/user';")
        .unwrap_err();
    assert!(error.contains("identity module was not initialized"));
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[test]
fn operations_fail_explicitly_on_unsupported_hosts() {
    let runtime = NeoJs::new_with_options(
        user_permission(),
        with_os_user(RuntimeOptions::new()).unwrap(),
    )
    .unwrap();
    runtime
        .run_module(
            "unsupported-user.js",
            "import * as api from 'neo:os/user';
             for (const operation of Object.values(api)) {
                 let unsupported = false;
                 try { operation(); } catch (error) {
                     unsupported = error instanceof Error && error.code === 'USER_UNSUPPORTED';
                 }
                 if (!unsupported) throw Error('operation did not report unsupported');
             }",
        )
        .unwrap();
}
