//! Embedding tests for `neo:path` capability and runtime cwd boundaries.

use neojs_core::{NeoJs, Permissions, RuntimeModule, RuntimeOptions};

#[test]
fn cwd_requires_both_a_grant_and_its_corresponding_allowlist_entry() {
    const SCRIPT: &str = r#"
import * as path from 'neo:path';

function checkError(operation, expected) {
  try { operation(); }
  catch (error) {
    if (error.code !== expected) throw new Error('unexpected code: ' + error.code);
    return;
  }
  throw new Error('cwd unexpectedly disclosed');
}

const windows = path.SEPARATOR === '\\';
const foreign = windows ? path.posix : path.win32;
const absolute = windows ? 'C:\\base' : '/base';
if (path.resolve(absolute, 'x') !== path.join(absolute, 'x')) throw Error('absolute');
if (path.relative(absolute, path.join(absolute, 'x')) !== 'x') throw Error('relative');
for (const api of [path, path.posix, path.win32]) {
  const sep = api.sep ?? path.SEPARATOR;
  const pattern = api.joinGlobs(['**', '*.js'], { globstar: true });
  if (pattern !== '**' + sep + '*.js' || !api.isGlob(pattern)) throw Error('lexical join');
  if (api.normalizeGlob('**/..', { globstar: true }) !== '**' + sep + '..')
    throw Error('lexical normalize');
  if (!api.globToRegExp(pattern).test('relative/file.js') ||
      !api.matchesGlob('relative/file.js', pattern)) throw Error('lexical match');
}
checkError(() => foreign.resolve('child'), 'PATH_CWD_DIALECT');
if (!windows) {
  checkError(() => path.win32.resolve('C:child'), 'PATH_CWD_DIALECT');
  checkError(() => path.win32.resolve('\\rooted'), 'PATH_CWD_DIALECT');
  checkError(() => path.win32.toNamespacedPath('child'), 'PATH_CWD_DIALECT');
}
for (const name of ['readCwd', '__neoPathCwd', '__neoPath', 'path']) {
  if (name in globalThis || name in path) throw Error('host binding exposed');
}

if (globalThis.expectCwd) {
  if (!path.isAbsolute(path.resolve())) throw Error('missing base');
  if (path.relative('.', 'a/../child') !== 'child') throw Error('relative base');
} else {
  checkError(() => path.resolve(), 'PATH_PERMISSION');
  checkError(() => path.relative('.', '.'), 'PATH_PERMISSION');
  checkError(() => path.relative(absolute, 'child'), 'PATH_PERMISSION');
  checkError(() => path.resolve('..'), 'PATH_PERMISSION');
  if (windows) {
    checkError(() => path.win32.resolve('\\rooted'), 'PATH_PERMISSION');
    checkError(() => path.win32.resolve('C:child'), 'PATH_PERMISSION');
    checkError(() => path.win32.toNamespacedPath('child'), 'PATH_PERMISSION');
  }
}
"#;
    let defaults = Permissions::default();
    let fs = Permissions {
        filesystem: true,
        ..defaults
    };
    let stdio = Permissions {
        stdio: true,
        ..defaults
    };
    let process = Permissions {
        process: true,
        ..defaults
    };
    let all = Permissions {
        filesystem: true,
        stdio: true,
        process: true,
        environment: true,
        network: true,
        registry: true,
        sqlite: true,
        user: true,
    };
    let path = RuntimeModule::Path;
    let path_fs = [path, RuntimeModule::Fs];
    let path_process = [path, RuntimeModule::Process];
    for (permissions, modules, expect_cwd) in [
        (defaults, &[path][..], false),
        (fs, &path_fs[..], true),
        (stdio, &path_process[..], true),
        (process, &path_process[..], true),
        (defaults, &path_fs[..], false),
        (defaults, &path_process[..], false),
        (stdio, &path_fs[..], false),
        (fs, &path_process[..], false),
        (
            all,
            &[path, RuntimeModule::Hash, RuntimeModule::Exec][..],
            false,
        ),
    ] {
        let options = RuntimeOptions::new().modules(modules.iter().copied());
        let js = NeoJs::new_with_options(permissions, options).unwrap();
        js.run(&format!("globalThis.expectCwd = {expect_cwd}"))
            .unwrap();
        js.run_module("path.js", SCRIPT).unwrap();
    }
}

#[test]
fn host_dialect_and_cwd_policy_do_not_consult_script_globals() {
    let js = NeoJs::new_with_options(
        Permissions::default(),
        RuntimeOptions::new().modules([RuntimeModule::Path]),
    )
    .unwrap();
    let separator = if cfg!(windows) { r"'\\'" } else { "'/'" };
    js.run_module(
        "spoofed-path.js",
        &format!(
            r#"globalThis.os = {{ platform: {separator} === '/' ? 'windows' : 'linux' }};
               globalThis.__neoPathCwd = () => {{
                 throw Error('untrusted callback invoked');
               }};
               const path = await import('neo:path');
               if (path.SEPARATOR !== {separator}) throw Error('spoofed platform');
               let code;
               try {{ path.resolve('child'); }} catch (error) {{ code = error.code; }}
               if (code !== 'PATH_PERMISSION') throw Error('spoofed cwd policy');"#,
        ),
    )
    .unwrap();
}

#[test]
fn omitting_path_denies_import_even_with_cwd_authority() {
    let js = NeoJs::new_with_options(
        Permissions {
            filesystem: true,
            stdio: true,
            ..Permissions::default()
        },
        RuntimeOptions::new().modules([RuntimeModule::Fs, RuntimeModule::Process]),
    )
    .unwrap();
    let error = js
        .run_module("denied-path.js", "import * as path from 'neo:path';")
        .unwrap_err();
    assert!(error.contains("allowlist"), "{error}");
    js.run_module(
        "dynamic-path.js",
        "try { await import('neo:path'); } catch (error) {
           if (!String(error).includes('allowlist')) throw error;
           globalThis.pathDenied = true;
         }
         if (globalThis.pathDenied !== true) throw Error('path unexpectedly admitted');",
    )
    .unwrap();
}

#[test]
fn each_runtime_resolves_against_its_live_cwd_without_mutating_the_host() {
    let host_before = std::env::current_dir().unwrap();
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let runtimes = [first.path(), second.path()].map(|dir| {
        let js = NeoJs::new(Permissions {
            stdio: true,
            ..Permissions::default()
        })
        .unwrap();
        let directory = serde_json::to_string(dir.to_str().unwrap()).unwrap();
        js.run_module(
            "setup.js",
            &format!(
                r#"import {{ chdir, cwd }} from 'neo:process';
                   import {{ resolve, relative, join, SEPARATOR }} from 'neo:path';
                   const before = resolve('x');
                   chdir({directory});
                   if (resolve('x') === before || resolve('x') !== join(cwd(), 'x'))
                     throw Error('cwd did not change');
                   if (relative('.', './x') !== 'x') throw Error('relative cwd');
                   if (SEPARATOR === '\\') {{
                     const drive = /^(?:\\\\\?\\)?([A-Z]:)/i.exec(cwd())?.[1];
                     if (drive !== undefined) {{
                       if (resolve(drive + 'x') !== resolve('x')) throw Error('same drive');
                       const other = drive.toUpperCase() === 'C:' ? 'D:' : 'C:';
                       let code;
                       try {{ resolve(other + 'x'); }} catch (error) {{ code = error.code; }}
                       if (code !== 'PATH_WINDOWS_ROOT') throw Error('foreign drive fallback');
                     }}
                   }}
                   globalThis.resolveForTest = resolve;"#,
            ),
        )
        .unwrap();
        js
    });
    let first_result = runtimes[0].eval("resolveForTest('x')").unwrap();
    assert_ne!(
        first_result,
        runtimes[1].eval("resolveForTest('x')").unwrap()
    );
    assert_eq!(
        first_result,
        runtimes[0].eval("resolveForTest('x')").unwrap()
    );
    assert_eq!(std::env::current_dir().unwrap(), host_before);
}
