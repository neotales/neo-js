//! Integration tests for the module allowlist (`RuntimeOptions`).
//!
//! These exercise the public `NeoJs` API. They prove the default
//! (`NeoJs::new`) stays unrestricted, an explicit allowlist fails closed for
//! static, dynamic, and repeated imports, and disabled host modules cannot be
//! reached through the globals or native callbacks that only they use.

use neojs_core::{ModuleAllowlist, NeoJs, Permissions, RuntimeModule, RuntimeOptions};

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

fn runtime(permissions: Permissions, modules: &[RuntimeModule]) -> NeoJs {
    let options = RuntimeOptions::new().modules(modules.iter().copied());
    NeoJs::new_with_options(permissions, options).unwrap()
}

fn import_error(js: &NeoJs, entry: &str, module: &str) -> String {
    let source = format!("import '{module}';");
    js.run_module(entry, &source).unwrap_err()
}

#[test]
fn default_runtime_keeps_the_historical_behavior() {
    // No allowlist: every built-in module is importable once its permission is
    // granted. Optional extensions are owned and tested by their own crates.
    let js = NeoJs::new(everything()).unwrap();
    js.run_module(
        "default-all.js",
        "import 'neo:age';
         import 'neo:ansi';
         import 'neo:assert';
         import 'neo:chars';
         import 'neo:crypto';
         import 'neo:dotenv';
         import 'neo:env';
         import 'neo:exec';
         import 'neo:fetch';
         import 'neo:fs';
         import 'neo:hash';
         import 'neo:path';
         import 'neo:process';
         import 'neo:semver';
         import 'neo:string-builder';
         import 'neo:test';
         import 'neo:toml';
         import 'neo:win-registry';
         import 'neo:yaml';",
    )
    .unwrap();
    assert_eq!(
        js.eval(
            "typeof __neoHash + ',' + typeof __neoSemverValid + ',' + typeof fs + ',' + \
             typeof env + ',' + typeof os + ',' + typeof __neoExec"
        )
        .unwrap(),
        "function,undefined,object,object,undefined,function"
    );
}

#[test]
fn explicit_allowlist_of_every_module_also_admits_every_module() {
    // A fixed allowlist that names everything is equivalent for imports. The
    // default remains distinct only for modules a future build might add.
    let js = runtime(everything(), &RuntimeModule::ALL);
    for (index, module) in RuntimeModule::ALL.into_iter().enumerate() {
        js.run_module(
            &format!("all-explicit-{index}.js"),
            &format!("import '{}';", module.name()),
        )
        .unwrap();
    }
}

#[test]
fn empty_allowlist_denies_every_module_statically_and_dynamically() {
    let js = runtime(Permissions::default(), &[]);
    for (index, module) in RuntimeModule::ALL.into_iter().enumerate() {
        let error = import_error(&js, &format!("deny-static-{index}.js"), module.name());
        assert!(
            error.contains("allowlist"),
            "static import of {} gave {error:?}",
            module.name()
        );

        // The resolver runs before QuickJS consults its cache, so a dynamic
        // import is denied the same way a static one is.
        let source = format!(
            "let leaked = false;\n\
             try {{ await import('{}'); leaked = true; }} catch (_) {{}}\n\
             if (leaked) throw new Error('leaked {}');",
            module.name(),
            module.name()
        );
        js.run_module(&format!("deny-dynamic-{index}.js"), &source)
            .unwrap();
    }
    assert_eq!(js.run_pending_tests().unwrap(), None);
}

#[test]
fn entry_scripts_still_execute_with_an_empty_allowlist() {
    let js = runtime(Permissions::default(), &[]);
    js.run_module("entry.js", "globalThis.ran = true;").unwrap();
    assert_eq!(js.eval("String(globalThis.ran)").unwrap(), "true");

    let error = js
        .run_module("entry-error.js", "throw new Error('boom');")
        .unwrap_err();
    assert!(
        error.contains("boom"),
        "runtime error was not clear: {error:?}"
    );
}

#[test]
fn disabled_modules_do_not_leave_globals_or_native_callbacks() {
    // `neo:path` needs no permission and no callback. Every other capability
    // is present in `Permissions` but must stay absent because it is not on
    // the allowlist.
    let js = runtime(everything(), &[RuntimeModule::Path]);
    assert_eq!(
        js.eval(
            "typeof fs + ',' + typeof env + ',' + typeof os + ',' + \
             typeof __neoHash + ',' + typeof __neoHashFile + ',' + \
             typeof __neoSemverValid + ',' + typeof __neoSemverCompare + ',' + \
             typeof __neoExec + ',' + typeof __neoExecSpawn + ',' + typeof __neoExecCall"
        )
        .unwrap(),
        "undefined,undefined,undefined,undefined,undefined,undefined,undefined,undefined,undefined,undefined"
    );
    assert_eq!(js.eval("typeof console.log").unwrap(), "function");

    js.run_module(
        "path-only.js",
        "import {posix} from 'neo:path'; if (posix.join('a', 'b') !== 'a/b') throw Error('path');",
    )
    .unwrap();

    for (index, module) in RuntimeModule::ALL
        .into_iter()
        .filter(|module| *module != RuntimeModule::Path)
        .enumerate()
    {
        let error = import_error(&js, &format!("deny-cap-{index}.js"), module.name());
        assert!(error.contains("allowlist"), "{error}");
    }
}

#[test]
fn allowlisted_modules_still_need_their_permission() {
    let allowlist = [
        RuntimeModule::Env,
        RuntimeModule::Exec,
        RuntimeModule::Fetch,
        RuntimeModule::Fs,
        RuntimeModule::Process,
        RuntimeModule::WinRegistry,
    ];
    let js = runtime(Permissions::default(), &allowlist);
    assert_eq!(
        js.eval("typeof fs + ',' + typeof env + ',' + typeof __neoExec")
            .unwrap(),
        "undefined,undefined,undefined"
    );
    for (index, module) in allowlist.into_iter().enumerate() {
        assert!(
            js.run_module(
                &format!("denied-permission-{index}.js"),
                &format!("import '{}';", module.name())
            )
            .is_err(),
            "{} was importable without its permission",
            module.name()
        );
    }

    // Granting every permission makes the same allowlist admit them.
    let js = runtime(everything(), &allowlist);
    js.run_module(
        "granted.js",
        "import 'neo:env';
         import 'neo:exec';
         import 'neo:fetch';
         import 'neo:fs';
         import 'neo:process';
         import 'neo:win-registry';
         if (typeof fs.readText !== 'function' || typeof env.get !== 'function') throw Error('globals');",
    )
    .unwrap();
}

#[test]
fn env_import_and_global_need_both_the_allowlist_and_permission() {
    let denied = runtime(Permissions::default(), &[RuntimeModule::Env]);
    assert!(
        denied
            .run_module("env-denied.js", "import 'neo:env';")
            .is_err()
    );
    assert_eq!(denied.eval("typeof env").unwrap(), "undefined");

    let allowed = runtime(
        Permissions {
            environment: true,
            ..Permissions::default()
        },
        &[RuntimeModule::Env],
    );
    allowed
        .run_module(
            "env-allowed.js",
            "import {has} from 'neo:env';
             if (typeof env.get !== 'function' || !has('PATH')) throw Error('env');",
        )
        .unwrap();
}

#[test]
fn hash_file_is_part_of_hash_not_fs() {
    // `digestFile` belongs to `neo:hash` and additionally needs filesystem
    // permission. Admitting hash without fs still gives the digest and keeps
    // the `fs` global absent.
    let hash_only = runtime(
        Permissions {
            filesystem: true,
            ..Permissions::default()
        },
        &[RuntimeModule::Hash],
    );
    hash_only
        .run_module(
            "hash-file.js",
            "import {digest, digestFile} from 'neo:hash';
             if (typeof __neoHashFile !== 'function') throw Error('missing callback');
             if (typeof fs !== 'undefined') throw Error('fs leaked');
             if (digestFile('sha256', 'Cargo.toml').length !== 64) throw Error('digestFile');
             if (digest('sha256', Uint8Array.from([97, 98, 99])).length !== 64) throw Error('digest');",
        )
        .unwrap();

    // Admitting fs without hash keeps the hash module and its file callback
    // absent, even with filesystem permission.
    let fs_only = runtime(
        Permissions {
            filesystem: true,
            ..Permissions::default()
        },
        &[RuntimeModule::Fs],
    );
    fs_only
        .run_module(
            "fs-only.js",
            "import {readText} from 'neo:fs';
             if (!readText('Cargo.toml').includes('neojs-core')) throw Error('fs');
             if (typeof __neoHashFile !== 'undefined') throw Error('hash callback leaked');",
        )
        .unwrap();
    assert!(
        fs_only
            .run_module("hash-denied.js", "import 'neo:hash';")
            .is_err()
    );

    // Hash admitted, filesystem denied: text digest works, file digest export
    // does not exist and the callback is absent.
    let no_fs = runtime(Permissions::default(), &[RuntimeModule::Hash]);
    assert_eq!(no_fs.eval("typeof __neoHash").unwrap(), "function");
    assert_eq!(no_fs.eval("typeof __neoHashFile").unwrap(), "undefined");
    assert!(
        no_fs
            .run_module("no-file.js", "import {digestFile} from 'neo:hash';")
            .is_err()
    );
    no_fs
        .run_module(
            "hash-digest.js",
            "import {digest} from 'neo:hash';
             if (digest('sha256', Uint8Array.from([97, 98, 99])).length !== 64) throw Error('digest');",
        )
        .unwrap();
}

#[test]
fn exec_and_process_are_independently_gated() {
    let exec_only = runtime(
        Permissions {
            process: true,
            ..Permissions::default()
        },
        &[RuntimeModule::Exec],
    );
    exec_only
        .run_module(
            "exec-only.js",
            "import {exec, spawn} from 'neo:exec';
             if (typeof exec !== 'function' || typeof spawn !== 'function') throw Error('exec');
             if (typeof __neoExec !== 'function' || typeof __neoExecCall !== 'function')
                 throw Error('callbacks');",
        )
        .unwrap();
    assert!(
        exec_only
            .run_module("process-denied.js", "import 'neo:process';")
            .is_err()
    );

    let process_only = runtime(
        Permissions {
            stdio: true,
            ..Permissions::default()
        },
        &[RuntimeModule::Process],
    );
    process_only
        .run_module(
            "process-only.js",
            "import {cwd, chdir, stdout, stderr} from 'neo:process';
             if (typeof cwd !== 'function' || typeof chdir !== 'function') throw Error('directory');
             if (typeof stdout.write !== 'function' || typeof stderr.write !== 'function')
                 throw Error('streams');",
        )
        .unwrap();
    assert_eq!(process_only.eval("typeof __neoExec").unwrap(), "undefined");
    assert!(
        process_only
            .run_module("exec-denied.js", "import 'neo:exec';")
            .is_err()
    );
}

#[test]
fn test_module_and_its_private_runner_require_the_allowlist() {
    let denied = runtime(Permissions::default(), &[]);
    assert!(
        denied
            .run_module(
                "test-denied.js",
                "import test from 'neo:test'; test('x', () => {});"
            )
            .is_err()
    );
    assert_eq!(denied.run_pending_tests().unwrap(), None);

    let allowed = runtime(Permissions::default(), &[RuntimeModule::Test]);
    allowed
        .run_module(
            "test-allowed.js",
            "import test from 'neo:test';
             test('passes', () => {});
             test('fails', () => { throw new Error('expected'); });",
        )
        .unwrap();
    let report = allowed.run_pending_tests().unwrap().unwrap();
    assert_eq!((report.total, report.passed, report.failed), (2, 1, 1));
}

#[test]
fn crypto_and_age_need_only_their_own_allowlist() {
    let crypto = runtime(Permissions::default(), &[RuntimeModule::Crypto]);
    crypto
        .run_module(
            "crypto.js",
            "import {decrypt, encrypt, generateKey} from 'neo:crypto';
             const key = generateKey();
             const out = decrypt(key, encrypt(key, Uint8Array.from([1, 2, 3])));
             if (out.length !== 3 || out[0] !== 1) throw Error('roundtrip');",
        )
        .unwrap();
    assert!(
        crypto
            .run_module("age-denied.js", "import 'neo:age';")
            .is_err()
    );

    let age = runtime(Permissions::default(), &[RuntimeModule::Age]);
    age.run_module(
        "age.js",
        "import {decrypt, encrypt, generateIdentity, identityToRecipient} from 'neo:age';
         if (typeof decrypt !== 'function' || typeof encrypt !== 'function' ||
             typeof generateIdentity !== 'function' ||
             typeof identityToRecipient !== 'function') throw Error('age');",
    )
    .unwrap();
    assert!(
        age.run_module("crypto-denied.js", "import 'neo:crypto';")
            .is_err()
    );
}

#[test]
fn fetch_needs_network_and_download_still_needs_filesystem() {
    let no_network = runtime(Permissions::default(), &[RuntimeModule::Fetch]);
    assert!(
        no_network
            .run_module("fetch-denied.js", "import 'neo:fetch';")
            .is_err()
    );

    let no_filesystem = runtime(
        Permissions {
            network: true,
            ..Permissions::default()
        },
        &[RuntimeModule::Fetch],
    );
    no_filesystem
        .run_module(
            "fetch-no-fs.js",
            "import {download} from 'neo:fetch';
             let code = '';
             try { download('not-a-url', 'unused.bin'); } catch (error) { code = error.code; }
             if (code !== 'FETCH_PERMISSION') throw Error('expected FETCH_PERMISSION, got ' + code);",
        )
        .unwrap();

    let with_filesystem = runtime(
        Permissions {
            network: true,
            filesystem: true,
            ..Permissions::default()
        },
        &[RuntimeModule::Fetch],
    );
    with_filesystem
        .run_module(
            "fetch-invalid.js",
            "import {download} from 'neo:fetch';
             let code = '';
             try { download('not-a-url', 'unused.bin'); } catch (error) { code = error.code; }
             if (code !== 'FETCH_INVALID_URL') throw Error('expected FETCH_INVALID_URL, got ' + code);",
        )
        .unwrap();
}

#[test]
fn semver_is_withdrawn_with_its_module() {
    let denied = runtime(Permissions::default(), &[RuntimeModule::Path]);
    assert_eq!(denied.eval("typeof __neoSemverValid").unwrap(), "undefined");
    // Core does not provide `neo:os` or install its global. The optional
    // `neojs-os` crate owns both and is tested there.
    assert_eq!(denied.eval("typeof os").unwrap(), "undefined");

    let allowed = runtime(Permissions::default(), &[RuntimeModule::Semver]);
    allowed
        .run_module(
            "semver.js",
            "import {valid, compare} from 'neo:semver';
             if (!valid('1.2.3') || compare('1.0.0-alpha', '1.0.0') !== -1) throw Error('semver');",
        )
        .unwrap();
    // The callbacks are private to the module: no legacy host global exists
    // even after a successful import.
    assert_eq!(
        allowed.eval("typeof __neoSemverValid").unwrap(),
        "undefined"
    );
    assert_eq!(
        allowed.eval("typeof __neoSemverCompare").unwrap(),
        "undefined"
    );
}

#[test]
fn both_registry_names_require_allowlist_and_permission() {
    for (registry, modules, denied_by_allowlist) in [
        (true, &[RuntimeModule::Path], true),
        (false, &[RuntimeModule::WinRegistry], false),
    ] {
        let js = runtime(
            Permissions {
                registry,
                ..Permissions::default()
            },
            modules,
        );
        for (index, name) in ["neo:win-registry", "neo:winregistry"]
            .into_iter()
            .enumerate()
        {
            for (case, source) in [
                format!("import '{name}';"),
                format!("await import('{name}');"),
                format!("import '{name}' with {{ type: 'json' }};"),
                format!("await import('{name}', {{ with: {{ type: 'json' }} }});"),
            ]
            .into_iter()
            .enumerate()
            {
                let error = js
                    .run_module(&format!("registry-denied-{index}-{case}.js"), &source)
                    .unwrap_err();
                assert!(error.contains(name), "{error}");
                assert_eq!(error.contains("allowlist"), denied_by_allowlist, "{error}");
                assert!(!error.contains("do not support attributes"), "{error}");
            }
        }
    }
}

#[test]
fn registry_aliases_share_one_allowlist_entry_and_cached_namespace() {
    let names = ["neo:win-registry", "neo:winregistry"];
    let allowlist = ModuleAllowlist::from_names(names).unwrap();
    assert_eq!(
        allowlist.iter().collect::<Vec<_>>(),
        [RuntimeModule::WinRegistry]
    );
    assert_eq!(RuntimeModule::WinRegistry.name(), names[0]);
    for name in names {
        let js = NeoJs::new_with_options(
            Permissions {
                registry: true,
                ..Permissions::default()
            },
            RuntimeOptions::new().module_names([name]).unwrap(),
        )
        .unwrap();
        js.run_module(
            "registry-first.js",
            &format!("import * as registry from '{name}'; globalThis.registry = registry;"),
        )
        .unwrap();
        js.run_module(
            "registry-cached.js",
            "import * as canonical from 'neo:win-registry';
             import * as legacy from 'neo:winregistry';
             if (typeof canonical.isAvailable !== 'function' || canonical.ValueType.DWORD !== 4)
                 throw Error('missing registry exports');
             if (canonical !== globalThis.registry || canonical !== legacy ||
                 canonical !== await import('neo:win-registry') ||
                 canonical !== await import('neo:winregistry'))
                 throw Error('registry module identity changed');",
        )
        .unwrap();
        js.run_module(
            "registry-empty-attributes.ts",
            "import * as canonical from 'neo:win-registry' with {};
             import * as legacy from 'neo:winregistry' with {};
             const cached: typeof canonical = globalThis.registry;
             if (canonical !== cached || legacy !== cached ||
                 cached !== await import('neo:win-registry', { with: {} }) ||
                 cached !== await import('neo:winregistry', { with: {} }))
                 throw Error('empty registry attributes changed module identity');",
        )
        .unwrap();
    }
}

#[test]
fn both_registry_names_reject_nonempty_import_attributes_before_and_after_caching() {
    let js = runtime(
        Permissions {
            registry: true,
            ..Permissions::default()
        },
        &[RuntimeModule::WinRegistry],
    );
    for cached in [false, true] {
        if cached {
            js.run_module("registry-cache.js", "import 'neo:win-registry';")
                .unwrap();
        }
        for extension in ["js", "ts"] {
            for (index, name) in ["neo:win-registry", "neo:winregistry"]
                .into_iter()
                .enumerate()
            {
                for (case, source) in [
                    format!("import '{name}' with {{ type: 'json' }};"),
                    format!("import '{name}' with {{ encoding: 'unsupported' }};"),
                    format!("await import('{name}', {{ with: {{ type: 'json' }} }});"),
                    format!("await import('{name}', {{ with: {{ encoding: 'unsupported' }} }});"),
                ]
                .into_iter()
                .enumerate()
                {
                    let error = js
                        .run_module(
                            &format!("registry-attributes-{cached}-{index}-{case}.{extension}"),
                            &source,
                        )
                        .unwrap_err();
                    assert!(error.contains(name), "{error}");
                    assert!(
                        error.contains("Windows Registry imports do not support attributes"),
                        "{error}"
                    );
                }
            }
        }
    }
}

#[test]
fn repeated_and_dynamic_imports_still_obey_the_allowlist() {
    // A cache hit is still checked by the resolver. Hash is admitted, so a
    // static import followed by a dynamic one succeeds.
    let allowed = runtime(Permissions::default(), &[RuntimeModule::Hash]);
    allowed
        .run_module(
            "cache-first.js",
            "import {digest} from 'neo:hash';
             globalThis.first = digest('sha256', Uint8Array.from([97]));",
        )
        .unwrap();
    allowed
        .run_module(
            "cache-second.js",
            "const {digest} = await import('neo:hash');
             if (digest('sha256', Uint8Array.from([97])) !== globalThis.first) throw Error('cache');",
        )
        .unwrap();

    // The same two steps fail closed when hash is not admitted.
    let denied = runtime(Permissions::default(), &[]);
    assert!(
        denied
            .run_module("deny-first.js", "import {digest} from 'neo:hash';")
            .is_err()
    );
    denied
        .run_module(
            "deny-second.js",
            "let leaked = false;
             try { await import('neo:hash'); leaked = true; } catch (_) {}
             if (leaked) throw Error('leaked after denial');",
        )
        .unwrap();
}

#[test]
fn unknown_names_are_rejected_by_the_fallible_name_builder() {
    let error = RuntimeOptions::new()
        .module_names(["neo:fs", "neo:nope", "fs", "neo:Hash"])
        .unwrap_err();
    assert_eq!(error.names(), ["neo:nope", "fs", "neo:Hash"]);
    assert!(error.to_string().contains("neo:nope"));
    assert!(error.to_string().contains("\"fs\""));
    assert!(error.to_string().contains("neo:Hash"));

    let options = RuntimeOptions::new()
        .module_names(["neo:fs", "neo:path"])
        .unwrap();
    assert!(options.is_module_allowed(RuntimeModule::Fs));
    assert!(options.is_module_allowed(RuntimeModule::Path));
    assert!(!options.is_module_allowed(RuntimeModule::Hash));

    // An empty explicit allowlist is valid and denies everything.
    let empty = ModuleAllowlist::from_names(std::iter::empty::<&str>()).unwrap();
    assert!(empty.is_empty());
    assert_eq!(empty.len(), 0);
}
