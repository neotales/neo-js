//! Host-embedding tests for the runtime-local working directory and the
//! synchronous `neo:process` capability.
//!
//! These tests use the public `NeoJs` API. They never change the host process
//! working directory, so they can run in parallel with every other test.

use std::path::Path;

use neo_js::{NeoJs, Permissions, RuntimeModule, RuntimeOptions};

fn js_string(value: &str) -> String {
    serde_json::to_string(value).unwrap()
}

fn runtime(filesystem: bool, stdio: bool, process: bool) -> NeoJs {
    NeoJs::new(Permissions {
        filesystem,
        stdio,
        process,
        ..Permissions::default()
    })
    .unwrap()
}

fn chdir(js: &NeoJs, directory: &Path) {
    js.run_module(
        "chdir.js",
        &format!(
            "import {{chdir}} from 'neo:process';\nchdir({});\n",
            js_string(directory.to_str().unwrap())
        ),
    )
    .unwrap();
}

#[test]
fn spawn_is_exposed_only_with_process_permission() {
    let denied = runtime(false, false, false);
    assert!(
        denied
            .run_module("denied-spawn.js", "import {spawn} from 'neo:exec';")
            .is_err()
    );
    assert_eq!(
        denied.eval("typeof globalThis.__neoExecSpawn").unwrap(),
        "undefined"
    );
    assert_eq!(
        denied.eval("typeof globalThis.__neoExecCall").unwrap(),
        "undefined"
    );

    let allowed = runtime(false, false, true);
    allowed
        .run_module(
            "allowed-spawn.js",
            "import {exec, spawn} from 'neo:exec';
             if (typeof exec !== 'function') throw Error('missing exec');
             if (typeof spawn !== 'function') throw Error('missing spawn');",
        )
        .unwrap();
}

#[test]
fn process_module_is_absent_without_permission() {
    let js = runtime(false, false, false);
    assert!(js.run_module("denied.js", "import 'neo:process';").is_err());
    assert_eq!(
        js.eval("typeof globalThis.__neoProcess").unwrap(),
        "undefined"
    );

    let filesystem = runtime(true, false, false);
    assert!(
        filesystem
            .run_module("filesystem.js", "import 'neo:process';")
            .is_err()
    );
}

#[test]
fn elevation_requires_an_admitted_process_module_and_existing_permission() {
    let entry = "import {isElevated} from 'neo:process';";
    let allowed = RuntimeOptions::new().modules([RuntimeModule::Process]);
    let denied = RuntimeOptions::new().modules([RuntimeModule::Path]);
    let no_permissions = NeoJs::new_with_options(Permissions::default(), allowed).unwrap();
    assert!(no_permissions.run_module("elevation.js", entry).is_err());

    for permissions in [
        Permissions {
            process: true,
            ..Permissions::default()
        },
        Permissions {
            stdio: true,
            ..Permissions::default()
        },
    ] {
        let js = NeoJs::new_with_options(permissions, denied.clone()).unwrap();
        assert!(js.run_module("elevation.js", entry).is_err());
        assert_eq!(
            js.eval("typeof globalThis.isElevated").unwrap(),
            "undefined"
        );
        assert_eq!(
            js.eval("typeof globalThis.__neoProcess").unwrap(),
            "undefined"
        );
    }
}

#[test]
fn stdio_and_process_each_expose_the_module() {
    let stdio = runtime(false, true, false);
    stdio
        .run_module(
            "stdio.js",
            "import process, {cwd, chdir, isElevated, stdin, stdout, stderr, ChangeDirectoryError} from 'neo:process';
             if (typeof cwd !== 'function' || typeof chdir !== 'function') throw Error('directory');
             if (typeof isElevated !== 'function' || process.isElevated !== isElevated) throw Error('elevation');
             if (typeof stdin.readSync !== 'function' || typeof stdin.read !== 'function') throw Error('stdin');
             if (typeof stdout.writeSync !== 'function' || typeof stderr.write !== 'function') throw Error('writers');
             if (typeof ChangeDirectoryError !== 'function') throw Error('error class');
             if (process.cwd !== cwd) throw Error('default export');
             if (stdin.readSync(new Uint8Array(0)) !== 0) throw Error('zero read');",
        )
        .unwrap();

    let process = runtime(false, false, true);
    process
        .run_module(
            "process.js",
            "import {isElevated, stdin} from 'neo:process';
             if (typeof isElevated !== 'function') throw Error('elevation');
             if (typeof stdin.readSync !== 'function') throw Error('stdin');
             if (stdin.readSync(new Uint8Array(0)) !== 0) throw Error('zero read');",
        )
        .unwrap();
}

#[cfg(unix)]
#[test]
fn elevation_reads_the_live_effective_uid() {
    // SAFETY: geteuid takes no arguments and is infallible.
    let expected = unsafe { libc::geteuid() } == 0;
    runtime(false, true, false)
        .run_module(
            "elevation.js",
            &format!(
                "import {{isElevated}} from 'neo:process';
                 if (isElevated() !== {expected}) throw Error('wrong effective UID status');"
            ),
        )
        .unwrap();
}

#[test]
fn runtime_directories_are_independent_and_filesystem_uses_them() {
    let host_before = std::env::current_dir().unwrap();
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    std::fs::write(first.path().join("data.txt"), "first").unwrap();
    std::fs::write(second.path().join("data.txt"), "second").unwrap();

    let first_js = runtime(true, true, false);
    let second_js = runtime(true, true, false);
    chdir(&first_js, first.path());
    chdir(&second_js, second.path());

    assert_eq!(first_js.eval("fs.readText('data.txt')").unwrap(), "first");
    assert_eq!(second_js.eval("fs.readText('data.txt')").unwrap(), "second");
    assert_eq!(std::env::current_dir().unwrap(), host_before);

    first_js
        .run_module(
            "failed-chdir.js",
            "import {cwd, chdir, ChangeDirectoryError} from 'neo:process';
             const before = cwd();
             let failed = false;
             try { chdir('neo-js-no-such-directory'); }
             catch (error) {
                 if (!(error instanceof ChangeDirectoryError)) throw error;
                 if (error.code !== 'PROCESS_CHDIR') throw error;
                 failed = true;
             }
             if (!failed || cwd() !== before) throw Error('failed chdir changed state');",
        )
        .unwrap();
    assert_eq!(first_js.eval("fs.readText('data.txt')").unwrap(), "first");
    assert_eq!(std::env::current_dir().unwrap(), host_before);
}

#[test]
fn relative_entry_and_imports_use_the_runtime_directory() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("dependency.js"),
        "globalThis.loaded = true;",
    )
    .unwrap();

    let js = runtime(true, true, false);
    chdir(&js, dir.path());
    js.run_module("entry.js", "import './dependency.js';")
        .unwrap();
    assert_eq!(js.eval("String(globalThis.loaded)").unwrap(), "true");
}

#[test]
fn virtual_entry_relative_imports_use_the_declaration_directory() {
    let original = tempfile::tempdir().unwrap();
    let later = tempfile::tempdir().unwrap();
    std::fs::write(
        original.path().join("dependency.js"),
        "export const value = 42;",
    )
    .unwrap();

    let js = runtime(true, true, false);
    chdir(&js, original.path());
    // This entry never exists on disk. Its base must still be captured before
    // the script changes the runtime directory.
    js.run_module(
        "virtual-entry.js",
        &format!(
            "import {{chdir}} from 'neo:process';
             chdir({});
             const dependency = await import('./dependency.js');
             if (dependency.value !== 42) throw Error('wrong dependency');",
            js_string(later.path().to_str().unwrap())
        ),
    )
    .unwrap();
    assert!(!original.path().join("virtual-entry.js").exists());
}

#[test]
fn host_identity_entry_captures_its_relative_base_too() {
    let original = tempfile::tempdir().unwrap();
    let later = tempfile::tempdir().unwrap();
    std::fs::write(
        original.path().join("dependency.js"),
        "export const value = 7;",
    )
    .unwrap();

    let js = runtime(true, true, false);
    chdir(&js, original.path());
    js.run_module_with_identity(
        "entry.js",
        "entry",
        &format!(
            "import {{chdir}} from 'neo:process';
             chdir({});
             const dependency = await import('./dependency.js');
             if (dependency.value !== 7) throw Error('wrong dependency');",
            js_string(later.path().to_str().unwrap())
        ),
    )
    .unwrap();
}

#[test]
fn virtual_entry_without_filesystem_still_runs_but_cannot_import_relatively() {
    let js = NeoJs::new(Permissions::default()).unwrap();
    js.run_module("virtual.js", "globalThis.ran = true;")
        .unwrap();
    assert_eq!(js.eval("String(globalThis.ran)").unwrap(), "true");
    assert!(
        js.run_module("virtual-import.js", "import './dependency.js';")
            .is_err()
    );
}

#[cfg(unix)]
#[test]
fn exec_child_uses_the_runtime_directory() {
    let dir = tempfile::tempdir().unwrap();
    let js = runtime(false, false, true);
    chdir(&js, dir.path());
    js.run_module(
        "exec.js",
        "import {cwd} from 'neo:process';
         import {exec} from 'neo:exec';
         const result = exec('sh', ['-c', 'pwd']);
         if (!result.success) throw Error('exec failed');
         const printed = String.fromCharCode(...result.stdout).trim();
         if (printed !== cwd()) throw Error('child cwd ' + printed + ' != ' + cwd());",
    )
    .unwrap();
}
