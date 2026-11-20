//! Allowlist tests for the independently selectable scheduler specifiers.
//!
//! The optional crate is registered, so the exact names behave like the former
//! built-in modules: the allowlist admits each specifier independently, no
//! permission is required, and a name that is merely similar does not resolve.

use neojs_core::{ModuleAllowlist, NeoJs, Permissions, RuntimeOptions};
use neojs_schedulers::{
    LEGACY_MODULE_NAME, SYSTEMD_MODULE_NAME, WINDOWS_MODULE_NAME, with_schedulers,
};

/// Every permission, so a test can separate "allowlisted" from "permitted".
fn everything() -> Permissions {
    Permissions {
        filesystem: true,
        environment: true,
        stdio: true,
        process: true,
        registry: true,
        network: true,
        sqlite: true,
        user: true,
    }
}

fn options() -> RuntimeOptions {
    with_schedulers(RuntimeOptions::new()).unwrap()
}

fn import_error(js: &NeoJs, entry: &str, module: &str) -> String {
    let source = format!("import '{module}';");
    js.run_module(entry, &source).unwrap_err()
}

#[test]
fn scheduler_specifiers_are_allowlisted_independently_without_permissions() {
    let surfaces = [
        (
            SYSTEMD_MODULE_NAME,
            "MAX_SYSTEMD_LINE_BYTES,SchedulerError,systemdService,systemdTimer,systemdUnits",
        ),
        (
            WINDOWS_MODULE_NAME,
            "MAX_WINDOWS_COMMAND_LINE,SchedulerError,WINDOWS_XML_ENCODING,windowsTask",
        ),
        (
            LEGACY_MODULE_NAME,
            "MAX_SYSTEMD_LINE_BYTES,MAX_WINDOWS_COMMAND_LINE,SchedulerError,\
             WINDOWS_XML_ENCODING,systemdService,systemdTimer,systemdUnits,windowsTask",
        ),
    ];
    for (allowed, exports) in surfaces {
        let options = options().module_names([allowed]).unwrap();
        assert!(options.is_module_name_allowed(allowed));
        let js = NeoJs::new_with_options(Permissions::default(), options.clone()).unwrap();
        js.run_module(
            "scheduler-allowed.js",
            &format!(
                "import * as staticModule from '{allowed}';
                 const dynamicModule = await import('{allowed}');
                 if (staticModule !== dynamicModule || Object.keys(dynamicModule).sort().join(',') !== '{exports}')
                   throw Error('wrong scheduler exports');"
            ),
        )
        .unwrap();
        for (index, (denied, _)) in surfaces.into_iter().enumerate() {
            if allowed == denied {
                continue;
            }
            assert!(!options.is_module_name_allowed(denied));
            let error = import_error(&js, &format!("scheduler-denied-{index}.js"), denied);
            assert!(error.contains("allowlist"), "{error}");
            js.run_module(
                &format!("scheduler-dynamic-denied-{index}.js"),
                &format!(
                    "let error;
                     try {{ await import('{denied}'); }} catch (err) {{ error = err; }}
                     if (!String(error).includes('allowlist')) throw Error('wrong import result');"
                ),
            )
            .unwrap();
        }
    }
}

#[test]
fn scheduler_names_do_not_resolve_parent_paths_or_variations() {
    let invalid = [
        "neo:os/schedulers",
        "neo:os/schedulers/Windows",
        "neo:os/schedulers/systemd.js",
        "neo:os/schedulers/windows/",
        "neo:os/schedulers/../schedulers/systemd",
        "neo:scheduler/windows",
    ];
    let options = options();
    let error = ModuleAllowlist::from_names_with(invalid, options.extensions()).unwrap_err();
    assert_eq!(error.names(), invalid);
    let js = NeoJs::new_with_options(everything(), options).unwrap();
    for (index, name) in invalid.into_iter().enumerate() {
        let error = import_error(&js, &format!("bad-scheduler-{index}.js"), name);
        assert!(error.contains(name), "{error}");
    }
    js.run_module(
        "bad-scheduler-dynamic.js",
        "for (const name of ['neo:os/schedulers', 'neo:os/schedulers/Windows']) {
           let error;
           try { await import(name); } catch (err) { error = err; }
           if (!String(error).includes(name)) throw Error('wrong import result');
         }",
    )
    .unwrap();
}
