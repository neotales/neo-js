use std::process::Command;

#[test]
fn cli_sqlite_requires_its_own_explicit_permission() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("sqlite.js");
    std::fs::write(
        &main,
        "import {openMemory} from 'neo:sqlite';\n\
         using db = openMemory();\n\
         console.log(String(db.query('SELECT 42')[0][0]));",
    )
    .unwrap();
    for flags in [&[][..], &["--allow-fs", "--allow-net", "--allow-run"][..]] {
        let output = Command::new(env!("CARGO_BIN_EXE_neo-js"))
            .args(flags)
            .arg(&main)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("neo:sqlite"));
    }
    let output = Command::new(env!("CARGO_BIN_EXE_neo-js"))
        .arg("--allow-sqlite")
        .arg(&main)
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output);
    assert_eq!(output.stdout, b"42\n");
    assert_eq!(dir.path().read_dir().unwrap().count(), 1);
}

#[test]
fn cli_exposes_os_metadata_through_the_linked_optional_module() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("os.js");
    std::fs::write(
        &main,
        "import {platform, arch, version, codename, variant} from 'neo:os';\n\
         if (platform !== os.platform || arch !== os.arch || version !== os.version || \
             codename !== os.codename || variant !== os.variant) throw Error('os metadata');\n\
         console.log(platform + ' ' + arch);",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_neo-js"))
        .arg(&main)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        output.stdout,
        format!("{} {}\n", std::env::consts::OS, std::env::consts::ARCH).as_bytes()
    );
}

#[test]
fn cli_loads_relative_modules_only_with_permission() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("main.js");
    std::fs::write(
        &main,
        "import {value} from './dependency.js'; console.log(value);",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("dependency.js"),
        "export const value = 'ok';",
    )
    .unwrap();

    let denied = Command::new(env!("CARGO_BIN_EXE_neo-js"))
        .arg(&main)
        .output()
        .unwrap();
    assert!(!denied.status.success());

    let allowed = Command::new(env!("CARGO_BIN_EXE_neo-js"))
        .arg("--allow-fs")
        .arg(&main)
        .output()
        .unwrap();
    assert!(allowed.status.success(), "{:?}", allowed);
    assert_eq!(allowed.stdout, b"ok\n");
}

#[test]
fn cli_semver_typescript_example_needs_no_permission() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("semver.ts");
    std::fs::write(
        &main,
        r#"import { parse, format, increment, satisfies, compare } from 'neo:semver';
import type { SemVerVersion } from 'neo:semver';
const version: SemVerVersion = parse('1.2.3-alpha.1+build');
if (Object.getPrototypeOf(version) !== null) throw Error('prototype');
console.log([
  format(version),
  increment(version.version, 'prerelease'),
  satisfies(version.version, '^1.2.3-alpha'),
  compare('1.0.0', '1.0.1'),
].join(' '));
"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_neo-js"))
        .arg(&main)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        output.stdout,
        b"1.2.3-alpha.1+build 1.2.3-alpha.2 true -1\n"
    );
}

#[test]
fn cli_registry_module_requires_allow_registry() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("registry.js");
    std::fs::write(
        &main,
        "import { isAvailable } from 'neo:winregistry'; console.log(isAvailable() ? 'true' : 'false');",
    )
    .unwrap();

    let denied = Command::new(env!("CARGO_BIN_EXE_neo-js"))
        .arg(&main)
        .output()
        .unwrap();
    assert!(!denied.status.success());

    let allowed = Command::new(env!("CARGO_BIN_EXE_neo-js"))
        .arg("--allow-registry")
        .arg(&main)
        .output()
        .unwrap();
    assert!(allowed.status.success(), "{:?}", allowed);
    #[cfg(windows)]
    assert_eq!(allowed.stdout, b"true\n");
    #[cfg(not(windows))]
    assert_eq!(allowed.stdout, b"false\n");
}

#[test]
fn cli_fetch_module_requires_allow_net_and_allow_fs() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("fetch.js");
    let destination = dir.path().join("destination.bin");
    let destination = serde_json::to_string(destination.to_str().unwrap()).unwrap();
    std::fs::write(
        &main,
        format!(
            "import {{ download }} from 'neo:fetch';
             try {{ download('http://127.0.0.1:9/file', {destination}); console.log('none'); }}
             catch (error) {{ console.log(error.code); }}"
        ),
    )
    .unwrap();

    let denied = Command::new(env!("CARGO_BIN_EXE_neo-js"))
        .arg(&main)
        .output()
        .unwrap();
    assert!(!denied.status.success());

    let missing_fs = Command::new(env!("CARGO_BIN_EXE_neo-js"))
        .arg("--allow-net")
        .arg(&main)
        .output()
        .unwrap();
    assert!(missing_fs.status.success(), "{missing_fs:?}");
    assert_eq!(missing_fs.stdout, b"FETCH_PERMISSION\n");
    assert!(!dir.path().join("destination.bin").exists());
}

#[cfg(unix)]
#[test]
fn symlink_parent_paths_follow_filesystem_for_entry_and_dependency() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("real/child")).unwrap();
    symlink(root.join("real/child"), root.join("link")).unwrap();
    std::fs::write(
        root.join("real/main.js"),
        "import './dep.js'; import './alias/../dep.js'; console.log('entry');",
    )
    .unwrap();
    symlink(root.join("real/child"), root.join("real/alias")).unwrap();
    std::fs::write(
        root.join("real/dep.js"),
        "import './main.js'; console.log('dependency');",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_neo-js"))
        .arg("--allow-fs")
        .arg(root.join("link/../main.js"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, b"dependency\nentry\n");
}

#[cfg(unix)]
#[test]
fn dependency_symlink_parent_resolves_to_target_parent() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("other/child")).unwrap();
    symlink(root.join("other/child"), root.join("link")).unwrap();
    std::fs::write(
        root.join("main.js"),
        "import './link/../dep.js'; import './other/dep.js';",
    )
    .unwrap();
    std::fs::write(root.join("dep.js"), "throw Error('wrong file');").unwrap();
    std::fs::write(root.join("other/dep.js"), "console.log('correct');").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_neo-js"))
        .arg("--allow-fs")
        .arg(root.join("main.js"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, b"correct\n");
}

#[test]
fn missing_entries_and_dependencies_are_errors() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_neo-js"))
        .arg(dir.path().join("missing.js"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    let main = dir.path().join("main.js");
    std::fs::write(&main, "import './missing.js';").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_neo-js"))
        .arg("--allow-fs")
        .arg(main)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("missing.js"));
}

#[cfg(unix)]
#[test]
fn symlink_entry_uses_requested_format() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("target.js"),
        "const value: number = 42; console.log(String(value));",
    )
    .unwrap();
    symlink("target.js", dir.path().join("entry.ts")).unwrap();
    for args in [&[][..], &["--allow-fs"][..]] {
        let out = Command::new(env!("CARGO_BIN_EXE_neo-js"))
            .args(args)
            .arg(dir.path().join("entry.ts"))
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[cfg(unix)]
#[test]
fn symlink_formats_and_rejected_extensions_for_entries_and_imports() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(
        root.join("typed.js"),
        "const value: number = 42; console.log(String(value));",
    )
    .unwrap();
    std::fs::write(root.join("typed.ts"), "const value: number = 42;").unwrap();
    symlink("typed.js", root.join("as.ts")).unwrap();
    symlink("typed.ts", root.join("as.js")).unwrap();
    for ext in ["tsx", "cts", "d.ts", "d.mts"] {
        symlink("typed.js", root.join(format!("bad.{ext}"))).unwrap();
    }
    let run = |entry: &std::path::Path, allowed: bool| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_neo-js"));
        if allowed {
            command.arg("--allow-fs");
        }
        command.arg(entry).output().unwrap()
    };
    for allowed in [false, true] {
        assert!(run(&root.join("as.ts"), allowed).status.success());
        assert!(!run(&root.join("as.js"), allowed).status.success());
        for ext in ["tsx", "cts", "d.ts", "d.mts"] {
            let out = run(&root.join(format!("bad.{ext}")), allowed);
            assert!(!out.status.success());
            assert!(String::from_utf8_lossy(&out.stderr).contains("TS_EXTENSION"));
        }
    }
    let main = root.join("main.js");
    std::fs::write(&main, "import './as.ts';").unwrap();
    assert!(run(&main, true).status.success());
    std::fs::write(&main, "import './as.js';").unwrap();
    assert!(!run(&main, true).status.success());
    for ext in ["tsx", "cts", "d.ts", "d.mts"] {
        for source in [
            format!("import './bad.{ext}';"),
            format!("await import('./bad.{ext}');"),
        ] {
            std::fs::write(&main, source).unwrap();
            let out = run(&main, true);
            assert!(!out.status.success(), "{ext}");
            assert!(
                String::from_utf8_lossy(&out.stderr).contains("TS_EXTENSION"),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn cross_format_aliases_fail_even_after_module_cached() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(
        root.join("dep.js"),
        "globalThis.count = (globalThis.count ?? 0) + 1;",
    )
    .unwrap();
    symlink("dep.js", root.join("dep.ts")).unwrap();
    symlink("dep.js", root.join("dep.tsx")).unwrap();
    let main = root.join("main.js");
    for source in [
        "import './dep.js'; import './dep.ts';",
        "await import('./dep.js'); await import('./dep.ts');",
        "import './dep.ts'; import './dep.js';",
    ] {
        std::fs::write(&main, source).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_neo-js"))
            .arg("--allow-fs")
            .arg(&main)
            .output()
            .unwrap();
        assert!(!out.status.success());
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("module format conflict"),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    std::fs::write(
        &main,
        "await import('./dep.js'); await import('./dep.tsx');",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_neo-js"))
        .arg("--allow-fs")
        .arg(&main)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("TS_EXTENSION"));
}

#[cfg(unix)]
#[test]
fn symlink_identity_is_stable_and_entry_format_conflicts_are_detected() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(
        root.join("target.js"),
        "globalThis.count = (globalThis.count || 0) + 1; console.log(String(globalThis.count));",
    )
    .unwrap();
    symlink("target.js", root.join("alias.js")).unwrap();
    symlink("target.js", root.join("alias.ts")).unwrap();
    let main = root.join("main.js");
    std::fs::write(&main, "import './target.js'; import './alias.js';").unwrap();
    let run = |p: &std::path::Path| {
        Command::new(env!("CARGO_BIN_EXE_neo-js"))
            .arg("--allow-fs")
            .arg(p)
            .output()
            .unwrap()
    };
    let out = run(&main);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, b"1\n");
    std::fs::write(root.join("target.js"), "import './alias.ts';").unwrap();
    let out = run(&root.join("target.js"));
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("module format conflict"));
}

#[cfg(unix)]
#[test]
fn failed_parse_does_not_reserve_format() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("dep.js"), "export const n: number = 7;").unwrap();
    symlink("dep.js", root.join("dep.ts")).unwrap();
    std::fs::write(root.join("main.js"), "try { await import('./dep.js'); throw Error('unexpected success'); } catch (e) { if (String(e).includes('unexpected success')) throw e; } const m = await import('./dep.ts'); if (m.n !== 7) throw Error('wrong');").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_neo-js"))
        .arg("--allow-fs")
        .arg(root.join("main.js"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[cfg(unix)]
#[test]
fn evaluation_failure_keeps_cached_format_but_load_failure_does_not() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("throws.js"), "throw Error('evaluation failed');").unwrap();
    symlink("throws.js", root.join("throws.ts")).unwrap();
    std::fs::create_dir(root.join("directory.js")).unwrap();
    symlink("directory.js", root.join("directory.ts")).unwrap();
    std::fs::write(root.join("main.js"), r#"
        try { await import('./throws.js'); } catch (e) { if (!String(e).includes('evaluation failed')) throw e; }
        try { await import('./throws.ts'); throw Error('expected conflict'); } catch (e) { if (!String(e).includes('module format conflict')) throw e; }
        try { await import('./directory.js'); } catch (_) {}
        try { await import('./directory.ts'); throw Error('expected load error'); } catch (e) { if (String(e).includes('module format conflict') || String(e).includes('expected load error')) throw e; }
    "#).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_neo-js"))
        .arg("--allow-fs")
        .arg(root.join("main.js"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn failed_cyclic_resolution_blocks_subsequent_imports_without_crashing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("a.js"), "import './b.js'; import './missing.js';").unwrap();
    std::fs::write(root.join("b.js"), "import './a.js'; export const x = 1;").unwrap();
    std::fs::write(root.join("main.js"), "try { await import('./a.js'); throw Error('unexpected success'); } catch (e) { if (String(e).includes('unexpected success')) throw e; } try { await import('./b.js'); throw Error('expected disabled'); } catch (e) { if (!String(e).includes('module loading disabled')) throw e; }").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_neo-js"))
        .arg("--allow-fs")
        .arg(root.join("main.js"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "status {:?}: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
}

#[cfg(unix)]
#[test]
fn failed_parent_resolution_keeps_successfully_declared_dependency() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("dep.js"), "export const x = 1;").unwrap();
    symlink("dep.js", root.join("dep.ts")).unwrap();
    std::fs::write(
        root.join("parent.js"),
        "import './dep.js'; import './missing.js';",
    )
    .unwrap();
    std::fs::write(root.join("main.js"), "try { await import('./parent.js'); } catch (_) {} try { await import('./dep.ts'); throw Error('expected conflict'); } catch (e) { if (!String(e).includes('module format conflict')) throw e; } const m = await import('./dep.js'); if (m.x !== 1) throw Error('wrong');").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_neo-js"))
        .arg("--allow-fs")
        .arg(root.join("main.js"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
