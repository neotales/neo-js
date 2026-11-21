//! Integration tests for `neo:fs` through the public `NeoJs` API.
//!
//! Every fixture is a `tempfile` directory. The tests exercise the permission
//! and allowlist gates, binary and Unicode round trips, symlink semantics,
//! the runtime-local working directory, no-clobber choices, malformed paths,
//! bounded reads and listings, and the stable error shape. Windows-only
//! behavior is gated and is not run by this build.

use std::fs;

use neojs_core::{NeoJs, Permissions, RuntimeModule, RuntimeOptions};

fn literal(path: &std::path::Path) -> String {
    serde_json::to_string(path.to_str().unwrap()).unwrap()
}

fn runtime() -> NeoJs {
    NeoJs::new(Permissions {
        filesystem: true,
        ..Permissions::default()
    })
    .unwrap()
}

fn chunked_runtime() -> NeoJs {
    NeoJs::new(Permissions {
        filesystem: true,
        stdio: true,
        ..Permissions::default()
    })
    .unwrap()
}

fn run(js: &NeoJs, source: &str) {
    js.run_module("main.js", source).unwrap();
}

/// Fixed-label JavaScript helpers shared by the directory-creation tests.
///
/// Failure and diagnostic text never interpolates `error.code`, `error.name`,
/// `error.message`, or a supplied path. A known `FsError` code maps to the same
/// fixed literal; any other code, unreadable property, or non-`FsError` maps to
/// a fixed placeholder. Property reads are guarded so a hostile getter cannot
/// throw into the assertion or leak its value.
const FS_TEST_ERROR_HELPERS: &str = r#"
function fsErrorCode(error) {
    try {
        if (!(error instanceof FsError)) return null;
        let code;
        try { code = error.code; } catch { return 'FS_UNREADABLE'; }
        switch (code) {
            case 'FS_PATH': return 'FS_PATH';
            case 'FS_NOT_FOUND': return 'FS_NOT_FOUND';
            case 'FS_NOT_DIRECTORY': return 'FS_NOT_DIRECTORY';
            case 'FS_IS_DIRECTORY': return 'FS_IS_DIRECTORY';
            case 'FS_ALREADY_EXISTS': return 'FS_ALREADY_EXISTS';
            case 'FS_NOT_EMPTY': return 'FS_NOT_EMPTY';
            case 'FS_CROSS_DEVICE': return 'FS_CROSS_DEVICE';
            case 'FS_DENIED': return 'FS_DENIED';
            case 'FS_LIMIT': return 'FS_LIMIT';
            case 'FS_ENCODING': return 'FS_ENCODING';
            case 'FS_DETACHED': return 'FS_DETACHED';
            case 'FS_OUT_OF_BOUNDS': return 'FS_OUT_OF_BOUNDS';
            case 'FS_CWD': return 'FS_CWD';
            case 'FS_TYPE': return 'FS_TYPE';
            case 'FS_IO': return 'FS_IO';
            case 'FS_UNSUPPORTED': return 'FS_UNSUPPORTED';
            case 'FS_ASYNC_OUTSIDE_TURN': return 'FS_ASYNC_OUTSIDE_TURN';
            case 'FS_CANCELLED': return 'FS_CANCELLED';
            default: return 'FS_UNRECOGNIZED';
        }
    } catch {
        return 'FS_UNCLASSIFIABLE';
    }
}
function describeError(error) {
    const code = fsErrorCode(error);
    if (code !== null) return code;
    try {
        if (error instanceof TypeError) return 'TypeError';
        if (error instanceof Error) return 'Error';
    } catch {
        return 'UNCLASSIFIABLE';
    }
    return 'NON_ERROR';
}
function leaksPath(error, path) {
    try {
        const message = error && error.message;
        return typeof message === 'string' ? message.includes(path) : true;
    } catch {
        return true;
    }
}
async function expectAsyncSuccess(label, action) {
    try {
        return await action();
    } catch (error) {
        throw Error(label + ': unexpected error ' + describeError(error));
    }
}
function expectSyncSuccess(label, action) {
    try {
        return action();
    } catch (error) {
        throw Error(label + ': unexpected error ' + describeError(error));
    }
}
"#;

#[test]
fn permission_and_allowlist_gate_the_global_and_the_module() {
    let denied = NeoJs::new(Permissions::default()).unwrap();
    assert_eq!(denied.eval("typeof fs").unwrap(), "undefined");
    assert!(denied.run_module("denied.js", "import 'neo:fs';").is_err());

    let options = RuntimeOptions::new().modules([RuntimeModule::Path]);
    let allowlisted = NeoJs::new_with_options(
        Permissions {
            filesystem: true,
            ..Permissions::default()
        },
        options,
    )
    .unwrap();
    assert_eq!(allowlisted.eval("typeof fs").unwrap(), "undefined");
    assert!(
        allowlisted
            .run_module("denied-allowlist.js", "import 'neo:fs';")
            .is_err()
    );

    let allowed = runtime();
    run(
        &allowed,
        "import * as module from 'neo:fs';
         if (typeof fs.readFile !== 'function') throw Error('missing global readFile');
         if (typeof fs.readFileSync !== 'function') throw Error('missing global readFileSync');
         if (typeof fs.readText !== 'function') throw Error('missing global readText');
         if (typeof module.writeFile !== 'function') throw Error('missing module writeFile');
         if (typeof fs.writeFileSync !== 'function') throw Error('missing global writeFileSync');
         if (typeof module.writeFileSync !== 'function') throw Error('missing module writeFileSync');
         if (module.readFile !== fs.readFile) throw Error('module and global differ');
         if (module.readFileSync !== fs.readFileSync) throw Error('sync module and global differ');
         if (module.writeFile !== fs.writeFile) throw Error('async writes differ');
         if (module.writeFileSync !== fs.writeFileSync) throw Error('sync writes differ');
         if (typeof fs.stat !== 'function' || typeof module.stat !== 'function') {
           throw Error('missing async stat');
         }
         if (typeof fs.statSync !== 'function' || typeof module.statSync !== 'function') {
           throw Error('missing sync stat');
         }
         if (typeof fs.lstat !== 'function' || typeof module.lstat !== 'function') {
           throw Error('missing async lstat');
         }
         if (typeof fs.lstatSync !== 'function' || typeof module.lstatSync !== 'function') {
           throw Error('missing sync lstat');
         }
         if (module.stat !== fs.stat || module.statSync !== fs.statSync) {
           throw Error('stat module and global differ');
         }
         if (module.lstat !== fs.lstat || module.lstatSync !== fs.lstatSync) {
           throw Error('lstat module and global differ');
         }
         if (typeof fs.readDir !== 'function' || typeof module.readDir !== 'function') {
           throw Error('missing async readDir');
         }
         if (typeof fs.readDirSync !== 'function' || typeof module.readDirSync !== 'function') {
           throw Error('missing sync readDir');
         }
         if (module.readDir !== fs.readDir || module.readDirSync !== fs.readDirSync) {
           throw Error('readDir module and global differ');
         }
         if (typeof fs.mkdir !== 'function' || typeof module.mkdir !== 'function') {
           throw Error('missing async mkdir');
         }
         if (typeof fs.mkdirSync !== 'function' || typeof module.mkdirSync !== 'function') {
           throw Error('missing sync mkdir');
         }
         if (module.mkdir !== fs.mkdir || module.mkdirSync !== fs.mkdirSync) {
           throw Error('mkdir module and global differ');
         }
         if (typeof fs.remove !== 'function' || typeof module.remove !== 'function') {
           throw Error('missing async remove');
         }
         if (typeof fs.removeSync !== 'function' || typeof module.removeSync !== 'function') {
           throw Error('missing sync remove');
         }
         if (module.remove !== fs.remove || module.removeSync !== fs.removeSync) {
           throw Error('remove module and global differ');
         }
         if (typeof module.rename !== 'function' || module.rename !== fs.rename) {
           throw Error('missing or mismatched rename');
         }
         if (typeof module.copyFile !== 'function' || module.copyFile !== fs.copyFile) {
           throw Error('missing or mismatched copyFile');
         }
         if (module.FsError !== fs.FsError) throw Error('error class differs');
         if (Object.getPrototypeOf(fs) !== null) throw Error('fs is not a null-prototype object');
         if ('__neoFsRename' in globalThis) throw Error('native rename callback leaked');
         if ('__neoFsCopyFile' in globalThis) throw Error('native copyFile callback leaked');
         if ('__neoFsReadText' in globalThis) throw Error('native callback leaked');",
    );
}

#[test]
fn binary_read_rejects_asynchronously_and_sync_read_still_throws() {
    let dir = tempfile::tempdir().unwrap();
    let missing = literal(&dir.path().join("not-here"));
    let file = dir.path().join("fixture");
    fs::write(&file, [0, 255, 7]).unwrap();
    let js = runtime();
    run(
        &js,
        &format!(
            r#"import {{readFile, readFileSync, FsError}} from 'neo:fs';
            const p = readFile({missing});
            if (!(p instanceof Promise)) throw Error('expected a native Promise');
            try {{ await p; throw Error('expected rejection'); }}
            catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_NOT_FOUND') throw error;
            }}
            try {{ readFileSync({missing}); throw Error('expected synchronous throw'); }}
            catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_NOT_FOUND') throw error;
            }}
            try {{ await readFile(null); throw Error('expected type rejection'); }}
            catch (error) {{ if (!(error instanceof TypeError)) throw error; }}
            Object.defineProperty(Uint8Array.prototype, 'then', {{
                get() {{ throw Error('result escaped through then'); }}, configurable: true
            }});
            const bytes = await fs.readFile({file});
            if (!(bytes instanceof Uint8Array) || bytes.length !== 3 || bytes[1] !== 255 ||
                Object.hasOwn(bytes, 'then')) throw Error('wrong bytes');"#,
            file = literal(&file),
        ),
    );
    js.run(&format!(
        "try {{ fs.readFile({file}); throw Error('dispatched outside managed turn'); }}
         catch (error) {{ if (error.code !== 'FS_ASYNC_OUTSIDE_TURN') throw error; }}",
        file = literal(&file),
    ))
    .unwrap();
}

#[test]
fn binary_write_is_asynchronous_and_sync_write_still_throws() {
    let dir = tempfile::tempdir().unwrap();
    let missing = literal(&dir.path().join("missing-parent").join("not-here"));
    let file = dir.path().join("fixture");
    fs::write(&file, b"old").unwrap();
    let js = runtime();
    run(
        &js,
        &format!(
            r#"import {{readFileSync, writeFile, writeFileSync, FsError}} from 'neo:fs';
            const missing = {missing};
            const file = {file};

            const pending = writeFile(file, new Uint8Array([1, 2, 3]));
            if (!(pending instanceof Promise)) throw Error('expected a native Promise');
            if ((await pending) !== undefined) throw Error('write should resolve undefined');
            const bytes = readFileSync(file);
            if (bytes.length !== 3 || bytes[2] !== 3) throw Error('async write did not land');

            try {{ await writeFile(missing, new Uint8Array([1])); throw Error('expected rejection'); }}
            catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_NOT_FOUND') throw error;
                if (error.message.includes(missing)) throw Error('path leaked');
            }}
            try {{ writeFileSync(missing, new Uint8Array([1])); throw Error('expected throw'); }}
            catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_NOT_FOUND') throw error;
            }}

            try {{
                await writeFile(file, new Uint8Array([4]), {{overwrite: false}});
                throw Error('expected no-clobber rejection');
            }} catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_ALREADY_EXISTS') throw error;
            }}
            if (readFileSync(file)[0] !== 1) throw Error('no-clobber changed the file');

            try {{ await writeFile(file, 'text'); throw Error('expected type rejection'); }}
            catch (error) {{ if (!(error instanceof TypeError)) throw error; }}

            await writeFile(file, new Uint8Array(0), {{overwrite: true}});
            if (readFileSync(file).length !== 0) throw Error('overwrite did not truncate');"#,
            missing = missing,
            file = literal(&file),
        ),
    );
    js.run(&format!(
        "try {{ fs.writeFile({file}, new Uint8Array([1])); throw Error('dispatched outside managed turn'); }}
         catch (error) {{ if (error.code !== 'FS_ASYNC_OUTSIDE_TURN') throw error; }}",
        file = literal(&file),
    ))
    .unwrap();
}

#[test]
fn metadata_reads_are_asynchronous_and_sync_forms_still_throw() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("fixture");
    fs::write(&file, b"abc").unwrap();
    let js = chunked_runtime();
    run(
        &js,
        &format!(
            r#"import {{chdir}} from 'neo:process';
            import {{stat, statSync, lstat, lstatSync, FsError, MAX_PATH_BYTES}} from 'neo:fs';
            chdir({dir});
            const file = {file};
            const missing = {missing};

            const pending = stat(file);
            if (!(pending instanceof Promise)) throw Error('stat did not return a Promise');
            const info = await pending;
            if (!info.isFile || info.isDirectory || info.isSymlink) throw Error('stat type');
            if (info.size !== '3') throw Error('stat size is not a decimal string');
            if (typeof info.mtimeMs !== 'string') throw Error('mtime is not a decimal string');

            const nofollow = lstat(file);
            if (!(nofollow instanceof Promise)) throw Error('lstat did not return a Promise');
            const regular = await nofollow;
            if (!regular.isFile || regular.isSymlink) throw Error('lstat type');

            try {{ await stat(missing); throw Error('async stat fulfilled'); }}
            catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_NOT_FOUND') throw error;
                if (error.message.includes(missing)) throw Error('async stat leaked the path');
            }}
            try {{ await lstat(missing); throw Error('async lstat fulfilled'); }}
            catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_NOT_FOUND') throw error;
            }}
            try {{ statSync(missing); throw Error('sync stat fulfilled'); }}
            catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_NOT_FOUND') throw error;
            }}
            try {{ lstatSync(missing); throw Error('sync lstat fulfilled'); }}
            catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_NOT_FOUND') throw error;
            }}
            if (statSync(file).size !== '3' || lstatSync(file).isFile !== true) {{
                throw Error('sync metadata changed');
            }}

            for (const [action, code] of [
                [() => stat(''), 'FS_PATH'],
                [() => lstat('a\0b'), 'FS_PATH'],
                [() => stat('x'.repeat(MAX_PATH_BYTES + 1)), 'FS_LIMIT'],
            ]) {{
                try {{ await action(); throw Error('expected rejection'); }}
                catch (error) {{
                    if (!(error instanceof FsError) || error.code !== code) throw error;
                }}
            }}
            try {{ await stat(null); throw Error('expected type rejection'); }}
            catch (error) {{ if (!(error instanceof TypeError)) throw error; }}
            try {{ statSync({{}}); throw Error('expected type throw'); }}
            catch (error) {{ if (!(error instanceof TypeError)) throw error; }}"#,
            dir = literal(dir.path()),
            file = literal(&file),
            missing = literal(&dir.path().join("not-here")),
        ),
    );
    js.run(&format!(
        "for (const action of [() => fs.stat({file}), () => fs.lstat({file})]) {{
           try {{ action(); throw Error('dispatched outside managed turn'); }}
           catch (error) {{ if (error.code !== 'FS_ASYNC_OUTSIDE_TURN') throw error; }}
         }}",
        file = literal(&file),
    ))
    .unwrap();
}

#[cfg(unix)]
#[test]
fn async_metadata_follows_or_keeps_a_final_symlink_like_the_sync_forms() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target.txt");
    let link = dir.path().join("link.txt");
    let dangling = dir.path().join("dangling.txt");
    fs::write(&target, b"abc").unwrap();
    symlink(&target, &link).unwrap();
    symlink(dir.path().join("missing"), &dangling).unwrap();

    let js = runtime();
    run(
        &js,
        &format!(
            r#"import {{stat, statSync, lstat, lstatSync, FsError}} from 'neo:fs';
            const link = {link};
            const dangling = {dangling};

            const followed = await stat(link);
            if (!followed.isFile || followed.isDirectory || followed.isSymlink) {{
                throw Error('async stat did not follow');
            }}
            if (followed.size !== '3') throw Error('stat size is not a decimal string');

            const linkInfo = await lstat(link);
            if (linkInfo.isFile || linkInfo.isDirectory || !linkInfo.isSymlink) {{
                throw Error('async lstat followed');
            }}
            if (!(await lstat(dangling)).isSymlink) throw Error('async lstat dangling');
            try {{ await stat(dangling); throw Error('dangling stat fulfilled'); }}
            catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_NOT_FOUND') throw error;
            }}

            if (statSync(link).isSymlink || !statSync(link).isFile) {{
                throw Error('sync stat did not follow');
            }}
            if (!lstatSync(link).isSymlink || lstatSync(dangling).isFile) {{
                throw Error('sync lstat followed');
            }}"#,
            link = literal(&link),
            dangling = literal(&dangling),
        ),
    );
}

#[test]
fn metadata_reads_use_the_cwd_captured_at_call_time() {
    let old = tempfile::tempdir().unwrap();
    let new = tempfile::tempdir().unwrap();
    fs::write(old.path().join("entry"), b"ab").unwrap();
    fs::write(new.path().join("entry"), b"abcde").unwrap();
    let js = chunked_runtime();
    js.run_module(
        "stat-cwd.js",
        &format!(
            r#"import {{chdir}} from 'neo:process';
            import {{stat, statSync}} from 'neo:fs';
            chdir({old});
            const pending = stat('entry');
            if (statSync('entry').size !== '2') throw Error('sync size before chdir');
            chdir({new});
            if ((await pending).size !== '2') throw Error('async stat did not keep the old cwd');
            if (statSync('entry').size !== '5') throw Error('sync stat did not use the new cwd');"#,
            old = literal(old.path()),
            new = literal(new.path()),
        ),
    )
    .unwrap();
}

#[test]
fn directory_reads_are_asynchronous_and_sync_forms_still_throw() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.txt"), b"a").unwrap();
    fs::write(dir.path().join("b.txt"), b"bb").unwrap();
    fs::create_dir(dir.path().join("child")).unwrap();
    let missing = dir.path().join("not-here");
    let file = dir.path().join("a.txt");
    let js = chunked_runtime();
    run(
        &js,
        &format!(
            r#"import {{chdir}} from 'neo:process';
            import {{readDir, readDirSync, FsError, MAX_PATH_BYTES}} from 'neo:fs';
            chdir({dir});
            const dir = {dir};
            const missing = {missing};
            const file = {file};

            const pending = readDir(dir);
            if (!(pending instanceof Promise)) throw Error('readDir did not return a Promise');
            const entries = await pending;
            if (!Array.isArray(entries)) throw Error('readDir result is not an array');
            const names = entries.map((entry) => entry.name).join(',');
            if (names !== 'a.txt,b.txt,child') throw Error('listing order or names: ' + names);
            const fileEntry = entries[0];
            if (!fileEntry.isFile || fileEntry.isDirectory || fileEntry.isSymlink) {{
                throw Error('async entry type');
            }}
            const directoryEntry = entries[2];
            if (!directoryEntry.isDirectory || directoryEntry.isFile) {{
                throw Error('async directory entry type');
            }}
            if (Object.hasOwn(entries, 'then')) throw Error('result kept the then guard');

            const syncEntries = readDirSync(dir);
            if (syncEntries.map((entry) => entry.name).join(',') !== names) {{
                throw Error('sync listing differs');
            }}

            try {{ await readDir(missing); throw Error('async readDir fulfilled'); }}
            catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_NOT_FOUND') throw error;
                if (error.message.includes(missing)) throw Error('async readDir leaked the path');
            }}
            try {{ await readDir(file); throw Error('async readDir accepted a file'); }}
            catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_NOT_DIRECTORY') throw error;
            }}
            try {{ readDirSync(missing); throw Error('sync readDir fulfilled'); }}
            catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_NOT_FOUND') throw error;
            }}
            try {{ readDirSync(file); throw Error('sync readDir accepted a file'); }}
            catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_NOT_DIRECTORY') throw error;
            }}

            for (const [action, code] of [
                [() => readDir(''), 'FS_PATH'],
                [() => readDirSync(''), 'FS_PATH'],
                [() => readDir('x'.repeat(MAX_PATH_BYTES + 1)), 'FS_LIMIT'],
            ]) {{
                try {{ await action(); throw Error('expected rejection'); }}
                catch (error) {{
                    if (!(error instanceof FsError) || error.code !== code) throw error;
                }}
            }}
            try {{ await readDir(null); throw Error('expected type rejection'); }}
            catch (error) {{ if (!(error instanceof TypeError)) throw error; }}
            try {{ readDirSync({{}}); throw Error('expected type throw'); }}
            catch (error) {{ if (!(error instanceof TypeError)) throw error; }}"#,
            dir = literal(dir.path()),
            missing = literal(&missing),
            file = literal(&file),
        ),
    );
    js.run(&format!(
        "try {{ fs.readDir({dir}); throw Error('dispatched outside managed turn'); }}
         catch (error) {{ if (error.code !== 'FS_ASYNC_OUTSIDE_TURN') throw error; }}",
        dir = literal(dir.path()),
    ))
    .unwrap();
}

#[test]
fn directory_read_result_resists_polluted_array_prototypes() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("only.txt"), b"x").unwrap();
    let js = runtime();
    run(
        &js,
        &format!(
            r#"import {{readDir}} from 'neo:fs';
            let setterRan = false;
            let thenRan = false;
            Object.defineProperty(Array.prototype, '0', {{
                set() {{ setterRan = true; throw Error('numeric setter ran'); }},
                configurable: true,
            }});
            Object.defineProperty(Array.prototype, 'then', {{
                get() {{ thenRan = true; throw Error('then getter ran'); }},
                configurable: true,
            }});
            try {{
                const entries = await readDir({dir});
                if (setterRan) throw Error('numeric setter intercepted the listing');
                if (thenRan) throw Error('then getter intercepted resolution');
                if (entries.length !== 1 || entries[0].name !== 'only.txt') {{
                    throw Error('listing changed');
                }}
                if (Object.hasOwn(entries, 'then')) throw Error('then guard leaked');
            }} finally {{
                delete Array.prototype[0];
                delete Array.prototype.then;
            }}"#,
            dir = literal(dir.path()),
        ),
    );
}

#[cfg(unix)]
#[test]
fn async_directory_reads_use_the_sync_symlink_rules() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let tree = dir.path().join("tree");
    fs::create_dir(&tree).unwrap();
    fs::write(dir.path().join("target.txt"), b"x").unwrap();
    fs::write(tree.join("inside.txt"), b"y").unwrap();
    symlink(dir.path().join("target.txt"), dir.path().join("link.txt")).unwrap();

    let js = runtime();
    run(
        &js,
        &format!(
            r#"import {{readDir, readDirSync}} from 'neo:fs';
            const dir = {dir};
            const tree = {tree};

            const entries = await readDir(dir);
            const names = entries.map((entry) => entry.name).join(',');
            if (names !== 'link.txt,target.txt,tree') throw Error('names: ' + names);
            const link = entries.find((entry) => entry.name === 'link.txt');
            if (!link.isSymlink || link.isFile || link.isDirectory) {{
                throw Error('async listing followed a symlink entry');
            }}

            const followed = await readDir(dir + '/tree');
            if (followed.length !== 1 || followed[0].name !== 'inside.txt') {{
                throw Error('async readDir did not follow a directory symlink path');
            }}
            const syncNames = readDirSync(tree).map((entry) => entry.name).join(',');
            if (syncNames !== 'inside.txt') throw Error('sync listing differs');"#,
            dir = literal(dir.path()),
            tree = literal(&tree),
        ),
    );
}

#[test]
fn directory_reads_use_the_cwd_captured_at_call_time() {
    let old = tempfile::tempdir().unwrap();
    let new = tempfile::tempdir().unwrap();
    fs::write(old.path().join("old-entry"), b"a").unwrap();
    fs::write(new.path().join("new-entry"), b"b").unwrap();
    let js = chunked_runtime();
    js.run_module(
        "read-dir-cwd.js",
        &format!(
            r#"import {{chdir}} from 'neo:process';
            import {{readDir, readDirSync}} from 'neo:fs';
            chdir({old});
            const pending = readDir('.');
            const before = readDirSync('.').map((entry) => entry.name).join(',');
            chdir({new});
            if (before !== 'old-entry') throw Error('sync listing before chdir');
            if ((await pending).map((entry) => entry.name).join(',') !== 'old-entry') {{
                throw Error('async readDir did not keep the old cwd');
            }}
            if (readDirSync('.').map((entry) => entry.name).join(',') !== 'new-entry') {{
                throw Error('sync readDir did not use the new cwd');
            }}"#,
            old = literal(old.path()),
            new = literal(new.path()),
        ),
    )
    .unwrap();
}

#[test]
fn directory_creation_is_asynchronous_and_sync_forms_still_throw() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("file.txt");
    fs::write(&file, b"x").unwrap();
    let existing = dir.path().join("existing");
    fs::create_dir(&existing).unwrap();
    let async_dir = dir.path().join("async");
    let nested = dir.path().join("one/two/three");
    let sync_dir = dir.path().join("sync");
    let missing_parent = dir.path().join("missing/child");
    let js = runtime();
    run(
        &js,
        &format!(
            r#"import {{mkdir, mkdirSync, exists, FsError, MAX_PATH_BYTES}} from 'neo:fs';
            {error_helpers}
            const asyncDir = {async_dir};
            const nested = {nested};
            const syncDir = {sync_dir};
            const existing = {existing};
            const missingParent = {missing_parent};
            const file = {file};

            // Every assertion names its own case in the thrown message. The
            // message uses only a fixed label and a sanitized code literal,
            // never a supplied path or an arbitrary error property.
            async function expectAsyncCode(label, code, action) {{
                try {{ await action(); }}
                catch (error) {{
                    if (fsErrorCode(error) === code) return error;
                    throw Error(label + ': expected ' + code + ', got ' + describeError(error));
                }}
                throw Error(label + ': expected ' + code + ' but the call succeeded');
            }}
            function expectSyncCode(label, code, action) {{
                try {{ action(); }}
                catch (error) {{
                    if (fsErrorCode(error) === code) return error;
                    throw Error(label + ': expected ' + code + ', got ' + describeError(error));
                }}
                throw Error(label + ': expected ' + code + ' but the call succeeded');
            }}
            async function expectAsyncType(label, action) {{
                try {{ await action(); }}
                catch (error) {{
                    let isTypeError = false;
                    try {{ isTypeError = error instanceof TypeError; }} catch {{}}
                    if (isTypeError) return error;
                    throw Error(label + ': expected TypeError, got ' + describeError(error));
                }}
                throw Error(label + ': expected TypeError but the call succeeded');
            }}
            function expectSyncType(label, action) {{
                try {{ action(); }}
                catch (error) {{
                    let isTypeError = false;
                    try {{ isTypeError = error instanceof TypeError; }} catch {{}}
                    if (isTypeError) return error;
                    throw Error(label + ': expected TypeError, got ' + describeError(error));
                }}
                throw Error(label + ': expected TypeError but the call succeeded');
            }}

            // Sanitizer self-check: known codes stay fixed literals; unknown
            // and unreadable code properties become fixed placeholders, so a
            // hostile value can never reach an assertion message.
            if (fsErrorCode(new FsError('FS_DENIED')) !== 'FS_DENIED') {{
                throw Error('sanitizer did not preserve a known code');
            }}
            if (fsErrorCode(new FsError('not-a-known-code')) !== 'FS_UNRECOGNIZED') {{
                throw Error('sanitizer did not replace an unknown code');
            }}
            const unreadableCode = new FsError('FS_IO');
            Object.defineProperty(unreadableCode, 'code', {{
                configurable: true,
                get() {{ throw Error('hostile getter'); }},
            }});
            if (fsErrorCode(unreadableCode) !== 'FS_UNREADABLE') {{
                throw Error('sanitizer did not replace an unreadable code');
            }}

            const pending = expectSyncSuccess(
                'async mkdir create call', () => mkdir(asyncDir));
            if (!(pending instanceof Promise)) throw Error('mkdir did not return a Promise');
            if ((await expectAsyncSuccess(
                'async mkdir create promise', () => pending)) !== undefined) {{
                throw Error('mkdir resolved with a value');
            }}
            if (!exists(asyncDir)) throw Error('async mkdir did not create the directory');

            await expectAsyncSuccess(
                'recursive async mkdir creates parents', () => mkdir(nested, {{recursive: true}}));
            if (!exists(nested)) throw Error('recursive async mkdir did not create parents');
            expectSyncSuccess('sync mkdir creates a directory', () => mkdirSync(syncDir));
            if (!exists(syncDir)) throw Error('sync mkdir did not create the directory');
            expectSyncSuccess('recursive sync mkdir accepts an existing directory',
                () => mkdirSync(nested, {{recursive: true}}));

            const asyncMissing = await expectAsyncCode(
                'async mkdir missing parent', 'FS_NOT_FOUND', () => mkdir(missingParent));
            if (leaksPath(asyncMissing, missingParent)) {{
                throw Error('async mkdir missing parent leaked the path');
            }}
            await expectAsyncCode('async mkdir existing path', 'FS_ALREADY_EXISTS',
                () => mkdir(existing));
            await expectAsyncSuccess('async recursive mkdir accepts an existing directory',
                () => mkdir(existing, {{recursive: true}}));
            expectSyncCode('sync mkdir missing parent', 'FS_NOT_FOUND',
                () => mkdirSync(missingParent));
            expectSyncCode('sync mkdir existing path', 'FS_ALREADY_EXISTS',
                () => mkdirSync(existing));

            await expectAsyncCode('async mkdir empty path', 'FS_PATH', () => mkdir(''));
            await expectAsyncCode('async mkdir oversize path', 'FS_LIMIT',
                () => mkdir('x'.repeat(MAX_PATH_BYTES + 1)));
            await expectAsyncType('async mkdir null path', () => mkdir(null));
            expectSyncType('sync mkdir object path', () => mkdirSync({{}}));
            await expectAsyncType('async mkdir invalid recursive option',
                () => mkdir({file} + '/bad-options', {{recursive: 'yes'}}));
            if (exists({file} + '/bad-options')) {{
                throw Error('invalid options created a directory');
            }}"#,
            error_helpers = FS_TEST_ERROR_HELPERS,
            async_dir = literal(&async_dir),
            nested = literal(&nested),
            sync_dir = literal(&sync_dir),
            existing = literal(&existing),
            missing_parent = literal(&missing_parent),
            file = literal(&file),
        ),
    );
    js.run(
        "try { fs.mkdir('outside'); throw Error('dispatched outside managed turn'); }
         catch (error) {
           let code = null;
           try { code = error && error.code; } catch {}
           if (code !== 'FS_ASYNC_OUTSIDE_TURN') {
             throw Error('mkdir outside managed turn: unexpected error');
           }
         }",
    )
    .unwrap();
}

#[test]
fn directory_creation_uses_the_cwd_captured_before_option_reflection() {
    let old = tempfile::tempdir().unwrap();
    let new = tempfile::tempdir().unwrap();
    let js = chunked_runtime();
    js.run_module(
        "mkdir-cwd.js",
        &format!(
            r#"import {{chdir}} from 'neo:process';
            import {{mkdir, mkdirSync, exists, FsError}} from 'neo:fs';
            {error_helpers}
            const oldDir = {old};
            const newDir = {new};
            chdir(oldDir);
            let trapped = false;
            const options = new Proxy({{}}, {{
                ownKeys() {{
                    trapped = true;
                    chdir(newDir);
                    return [];
                }},
            }});
            await expectAsyncSuccess('async mkdir keeps the captured cwd',
                () => mkdir('late', options));
            if (!trapped) throw Error('option trap did not run');
            if (!exists(oldDir + '/late') || exists(newDir + '/late')) {{
                throw Error('async mkdir did not keep the old cwd');
            }}
            chdir(newDir);
            expectSyncSuccess('sync mkdir uses the captured cwd', () => mkdirSync('sync'));
            if (!exists(newDir + '/sync')) throw Error('sync mkdir did not use the new cwd');"#,
            error_helpers = FS_TEST_ERROR_HELPERS,
            old = literal(old.path()),
            new = literal(new.path()),
        ),
    )
    .unwrap();
}

#[test]
fn removal_is_asynchronous_and_sync_forms_still_throw() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("file.txt");
    fs::write(&file, b"x").unwrap();
    let empty = dir.path().join("empty");
    fs::create_dir(&empty).unwrap();
    let tree = dir.path().join("tree");
    fs::create_dir(&tree).unwrap();
    fs::write(tree.join("inside.txt"), b"x").unwrap();
    let nonempty = dir.path().join("nonempty");
    fs::create_dir(&nonempty).unwrap();
    fs::write(nonempty.join("child"), b"x").unwrap();
    let sync_file = dir.path().join("sync-file.txt");
    fs::write(&sync_file, b"x").unwrap();
    let sync_empty = dir.path().join("sync-empty");
    fs::create_dir(&sync_empty).unwrap();
    let sync_tree = dir.path().join("sync-tree");
    fs::create_dir(&sync_tree).unwrap();
    fs::write(sync_tree.join("inside.txt"), b"x").unwrap();
    let missing = dir.path().join("missing");
    let js = runtime();
    run(
        &js,
        &format!(
            r#"import {{remove, removeSync, exists, FsError, MAX_PATH_BYTES}} from 'neo:fs';
            {error_helpers}
            const file = {file};
            const empty = {empty};
            const tree = {tree};
            const nonempty = {nonempty};
            const syncFile = {sync_file};
            const syncEmpty = {sync_empty};
            const syncTree = {sync_tree};
            const missing = {missing};

            // Every assertion names its own case in the thrown message.
            async function expectAsyncCode(label, code, action) {{
                try {{ await action(); }}
                catch (error) {{
                    if (fsErrorCode(error) === code) return error;
                    throw Error(label + ': expected ' + code + ', got ' + describeError(error));
                }}
                throw Error(label + ': expected ' + code + ' but the call succeeded');
            }}
            function expectSyncCode(label, code, action) {{
                try {{ action(); }}
                catch (error) {{
                    if (fsErrorCode(error) === code) return error;
                    throw Error(label + ': expected ' + code + ', got ' + describeError(error));
                }}
                throw Error(label + ': expected ' + code + ' but the call succeeded');
            }}
            async function expectAsyncType(label, action) {{
                try {{ await action(); }}
                catch (error) {{
                    let isTypeError = false;
                    try {{ isTypeError = error instanceof TypeError; }} catch {{}}
                    if (isTypeError) return error;
                    throw Error(label + ': expected TypeError, got ' + describeError(error));
                }}
                throw Error(label + ': expected TypeError but the call succeeded');
            }}
            function expectSyncType(label, action) {{
                try {{ action(); }}
                catch (error) {{
                    let isTypeError = false;
                    try {{ isTypeError = error instanceof TypeError; }} catch {{}}
                    if (isTypeError) return error;
                    throw Error(label + ': expected TypeError, got ' + describeError(error));
                }}
                throw Error(label + ': expected TypeError but the call succeeded');
            }}

            const pending = expectSyncSuccess('async remove call', () => remove(file));
            if (!(pending instanceof Promise)) throw Error('remove did not return a Promise');
            if ((await expectAsyncSuccess(
                'async remove promise', () => pending)) !== undefined) {{
                throw Error('remove resolved with a value');
            }}
            if (exists(file)) throw Error('async remove left the file');

            await expectAsyncSuccess('async remove empty directory', () => remove(empty));
            if (exists(empty)) throw Error('async remove left the empty directory');

            const missingError = await expectAsyncCode(
                'async remove missing path', 'FS_NOT_FOUND', () => remove(missing));
            if (leaksPath(missingError, missing)) {{
                throw Error('async remove missing path leaked the path');
            }}
            await expectAsyncCode('async remove non-empty directory', 'FS_NOT_EMPTY',
                () => remove(nonempty));
            if (!exists(nonempty)) throw Error('failed async remove deleted the directory');
            await expectAsyncSuccess('async recursive remove',
                () => remove(tree, {{recursive: true}}));
            if (exists(tree)) throw Error('recursive async remove left the tree');

            expectSyncSuccess('sync remove file', () => removeSync(syncFile));
            if (exists(syncFile)) throw Error('sync remove left the file');
            expectSyncSuccess('sync remove empty directory', () => removeSync(syncEmpty));
            if (exists(syncEmpty)) throw Error('sync remove left the empty directory');
            expectSyncCode('sync remove missing path', 'FS_NOT_FOUND', () => removeSync(missing));
            expectSyncCode('sync remove non-empty directory', 'FS_NOT_EMPTY',
                () => removeSync(nonempty));
            expectSyncSuccess('sync recursive remove',
                () => removeSync(syncTree, {{recursive: true}}));
            if (exists(syncTree)) throw Error('recursive sync remove left the tree');

            await expectAsyncCode('async remove empty path', 'FS_PATH', () => remove(''));
            await expectAsyncCode('async remove trailing separator', 'FS_PATH',
                () => remove(empty + '/', {{recursive: true}}));
            await expectAsyncCode('async remove root', 'FS_PATH',
                () => remove('/', {{recursive: true}}));
            await expectAsyncCode('async remove oversize path', 'FS_LIMIT',
                () => remove('x'.repeat(MAX_PATH_BYTES + 1)));
            await expectAsyncType('async remove null path', () => remove(null));
            expectSyncType('sync remove object path', () => removeSync({{}}));
            await expectAsyncType('async remove invalid recursive option',
                () => remove(nonempty, {{recursive: 'yes'}}));
            if (!exists(nonempty)) throw Error('invalid async options deleted the directory');
            expectSyncType('sync remove invalid recursive option',
                () => removeSync(nonempty, {{recursive: 'yes'}}));
            if (!exists(nonempty)) throw Error('invalid sync options deleted the directory');"#,
            error_helpers = FS_TEST_ERROR_HELPERS,
            file = literal(&file),
            empty = literal(&empty),
            tree = literal(&tree),
            nonempty = literal(&nonempty),
            sync_file = literal(&sync_file),
            sync_empty = literal(&sync_empty),
            sync_tree = literal(&sync_tree),
            missing = literal(&missing),
        ),
    );
    assert!(nonempty.exists());
    js.run(
        "try { fs.remove('outside'); throw Error('dispatched outside managed turn'); }
         catch (error) {
           let code = null;
           try { code = error && error.code; } catch {}
           if (code !== 'FS_ASYNC_OUTSIDE_TURN') {
             throw Error('remove outside managed turn: unexpected error');
           }
         }",
    )
    .unwrap();
}

#[test]
fn removal_uses_the_cwd_captured_before_option_reflection() {
    let old = tempfile::tempdir().unwrap();
    let new = tempfile::tempdir().unwrap();
    for root in [old.path(), new.path()] {
        fs::create_dir(root.join("target")).unwrap();
        fs::write(root.join("target/child"), b"x").unwrap();
        fs::create_dir(root.join("sync-target")).unwrap();
        fs::write(root.join("sync-target/child"), b"x").unwrap();
    }
    let js = chunked_runtime();
    js.run_module(
        "remove-cwd.js",
        &format!(
            r#"import {{chdir}} from 'neo:process';
            import {{remove, removeSync, exists, FsError}} from 'neo:fs';
            {error_helpers}
            const oldDir = {old};
            const newDir = {new};
            chdir(oldDir);
            let trapped = false;
            const options = new Proxy({{}}, {{
                ownKeys() {{
                    trapped = true;
                    chdir(newDir);
                    return ['recursive'];
                }},
                getOwnPropertyDescriptor(_target, key) {{
                    if (key !== 'recursive') return undefined;
                    return {{value: true, enumerable: true, configurable: true, writable: true}};
                }},
            }});
            await expectAsyncSuccess('async remove keeps the captured cwd',
                () => remove('target', options));
            if (!trapped) throw Error('option trap did not run');
            if (exists(oldDir + '/target') || !exists(newDir + '/target/child')) {{
                throw Error('async remove did not keep the old cwd');
            }}
            chdir(newDir);
            expectSyncSuccess('sync remove uses the captured cwd',
                () => removeSync('sync-target', {{recursive: true}}));
            if (exists(newDir + '/sync-target')) {{
                throw Error('sync remove did not use the new cwd');
            }}"#,
            error_helpers = FS_TEST_ERROR_HELPERS,
            old = literal(old.path()),
            new = literal(new.path()),
        ),
    )
    .unwrap();
    assert!(!old.path().join("target").exists());
    assert!(new.path().join("target/child").exists());
    assert!(old.path().join("sync-target/child").exists());
    assert!(!new.path().join("sync-target").exists());
}

#[cfg(unix)]
#[test]
fn directory_creation_follows_unix_symlinks_without_containment() {
    use std::os::unix::fs::symlink;

    let base = tempfile::tempdir().unwrap();
    let root = base.path().join("root");
    let outside = base.path().join("outside");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&outside).unwrap();
    let final_dir = root.join("final-dir");
    let outside_dir = outside.join("final-dir");
    fs::create_dir(&outside_dir).unwrap();
    symlink(&outside_dir, &final_dir).unwrap();
    let missing_final = root.join("missing-final");
    let dangling = root.join("dangling");
    symlink(&missing_final, &dangling).unwrap();
    let intermediate = root.join("intermediate");
    symlink(&outside, &intermediate).unwrap();
    let missing_parent = root.join("missing-parent");
    let intermediate_dangling = root.join("intermediate-dangling");
    symlink(&missing_parent, &intermediate_dangling).unwrap();
    let file = root.join("plain-file");
    fs::write(&file, b"x").unwrap();
    let intermediate_file = root.join("intermediate-file");
    symlink(&file, &intermediate_file).unwrap();

    let js = runtime();
    run(
        &js,
        &format!(
            r#"import {{mkdir, mkdirSync, lstatSync, statSync, exists, FsError}} from 'neo:fs';
            {error_helpers}
            const finalDir = {final_dir};
            const outsideDir = {outside_dir};
            const dangling = {dangling};
            const missingFinal = {missing_final};
            const intermediate = {intermediate};
            const outside = {outside};
            const intermediateDangling = {intermediate_dangling};
            const missingParent = {missing_parent};
            const intermediateFile = {intermediate_file};
            const plainFile = {plain_file};

            function expectSync(code, action) {{
                try {{ action(); }}
                catch (error) {{
                    if (fsErrorCode(error) === code) return;
                    throw Error('expected ' + code + ', got ' + describeError(error));
                }}
                throw Error('did not throw ' + code);
            }}
            async function expectAsync(code, action) {{
                try {{ await action(); }}
                catch (error) {{
                    if (fsErrorCode(error) === code) return;
                    throw Error('expected ' + code + ', got ' + describeError(error));
                }}
                throw Error('did not reject ' + code);
            }}

            // A final symlink to a directory: non-recursive creation refuses
            // the entry; recursive creation accepts the directory but does not
            // replace the link.
            await expectAsync('FS_ALREADY_EXISTS', () => mkdir(finalDir));
            expectSync('FS_ALREADY_EXISTS', () => mkdirSync(finalDir));
            await expectAsyncSuccess(
                'unix async recursive mkdir accepts a final directory link',
                () => mkdir(finalDir, {{recursive: true}}));
            expectSyncSuccess(
                'unix sync recursive mkdir accepts a final directory link',
                () => mkdirSync(finalDir, {{recursive: true}}));
            if (!lstatSync(finalDir).isSymlink || !statSync(outsideDir).isDirectory) {{
                throw Error('recursive call did not keep the final directory link');
            }}

            // A final dangling symlink is still an existing entry. Neither
            // mode follows it or creates its target.
            for (const options of [undefined, {{recursive: true}}]) {{
                await expectAsync('FS_ALREADY_EXISTS', () => mkdir(dangling, options));
                expectSync('FS_ALREADY_EXISTS', () => mkdirSync(dangling, options));
            }}
            if (!lstatSync(dangling).isSymlink || exists(missingFinal)) {{
                throw Error('dangling final link was replaced or followed');
            }}

            // Intermediate directory links are followed. Creation lands under
            // the link target, outside the apparent root, in both modes.
            await expectAsyncSuccess(
                'unix async mkdir follows an intermediate directory link',
                () => mkdir(intermediate + '/async-nonrec'));
            if (!statSync(outside + '/async-nonrec').isDirectory) {{
                throw Error('async non-recursive creation did not follow the link');
            }}
            await expectAsyncSuccess(
                'unix async recursive mkdir follows an intermediate directory link',
                () => mkdir(intermediate + '/async-rec/nested', {{recursive: true}}));
            if (!statSync(outside + '/async-rec/nested').isDirectory) {{
                throw Error('async recursive creation did not follow the link');
            }}
            expectSyncSuccess(
                'unix sync mkdir follows an intermediate directory link',
                () => mkdirSync(intermediate + '/sync-nonrec'));
            expectSyncSuccess(
                'unix sync recursive mkdir follows an intermediate directory link',
                () => mkdirSync(intermediate + '/sync-rec/nested', {{recursive: true}}));
            if (!statSync(outside + '/sync-nonrec').isDirectory ||
                !statSync(outside + '/sync-rec/nested').isDirectory) {{
                throw Error('sync creation did not follow the link');
            }}

            // A dangling intermediate link is not followed. The non-recursive
            // call reports a missing component, while create_dir_all reports
            // the existing link entry and never creates the missing target.
            await expectAsync('FS_NOT_FOUND', () => mkdir(intermediateDangling + '/async'));
            expectSync('FS_NOT_FOUND', () => mkdirSync(intermediateDangling + '/sync'));
            await expectAsync('FS_ALREADY_EXISTS',
                () => mkdir(intermediateDangling + '/async-rec/child', {{recursive: true}}));
            expectSync('FS_ALREADY_EXISTS',
                () => mkdirSync(intermediateDangling + '/sync-rec/child', {{recursive: true}}));
            if (exists(missingParent)) throw Error('dangling intermediate target was created');

            // A component that resolves to a non-directory is rejected in both
            // modes and both forms.
            await expectAsync('FS_NOT_DIRECTORY', () => mkdir(intermediateFile + '/child'));
            expectSync('FS_NOT_DIRECTORY', () => mkdirSync(intermediateFile + '/child'));
            await expectAsync('FS_NOT_DIRECTORY',
                () => mkdir(intermediateFile + '/child', {{recursive: true}}));
            expectSync('FS_NOT_DIRECTORY',
                () => mkdirSync(intermediateFile + '/child', {{recursive: true}}));

            // A direct regular-file component follows the same Unix rule as a
            // symlink that resolves to a regular file. The labels let a
            // failure identify one of the four cases without echoing a path.
            async function expectAsyncCode(label, code, action) {{
                try {{ await action(); }}
                catch (error) {{
                    if (fsErrorCode(error) === code) return;
                    throw Error(label + ': expected ' + code + ', got ' + describeError(error));
                }}
                throw Error(label + ': expected ' + code + ' but the call succeeded');
            }}
            function expectSyncCode(label, code, action) {{
                try {{ action(); }}
                catch (error) {{
                    if (fsErrorCode(error) === code) return;
                    throw Error(label + ': expected ' + code + ', got ' + describeError(error));
                }}
                throw Error(label + ': expected ' + code + ' but the call succeeded');
            }}
            await expectAsyncCode('unix async mkdir below a regular file', 'FS_NOT_DIRECTORY',
                () => mkdir(plainFile + '/direct-child'));
            expectSyncCode('unix sync mkdir below a regular file', 'FS_NOT_DIRECTORY',
                () => mkdirSync(plainFile + '/direct-child'));
            await expectAsyncCode('unix async recursive mkdir below a regular file',
                'FS_NOT_DIRECTORY',
                () => mkdir(plainFile + '/direct-recursive', {{recursive: true}}));
            expectSyncCode('unix sync recursive mkdir below a regular file', 'FS_NOT_DIRECTORY',
                () => mkdirSync(plainFile + '/direct-recursive', {{recursive: true}}));
            if (exists(plainFile + '/direct-child') || exists(plainFile + '/direct-recursive')) {{
                throw Error('unix mkdir below a regular file created a directory');
            }}"#,
            error_helpers = FS_TEST_ERROR_HELPERS,
            final_dir = literal(&final_dir),
            outside_dir = literal(&outside_dir),
            dangling = literal(&dangling),
            missing_final = literal(&missing_final),
            intermediate = literal(&intermediate),
            outside = literal(&outside),
            intermediate_dangling = literal(&intermediate_dangling),
            missing_parent = literal(&missing_parent),
            intermediate_file = literal(&intermediate_file),
            plain_file = literal(&file),
        ),
    );
    assert!(outside.join("async-nonrec").is_dir());
    assert!(outside.join("async-rec/nested").is_dir());
    assert!(outside.join("sync-nonrec").is_dir());
    assert!(outside.join("sync-rec/nested").is_dir());
    assert!(!missing_parent.exists());
}

#[cfg(unix)]
#[test]
fn directory_creation_rejects_an_existing_final_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("existing-file");
    fs::write(&file, b"x").unwrap();

    let js = runtime();
    run(
        &js,
        &format!(
            r#"import {{mkdir, mkdirSync, lstatSync, readFileSync, FsError}} from 'neo:fs';
            {error_helpers}
            const file = {file};

            async function expectAsync(code, action) {{
                try {{ await action(); }}
                catch (error) {{
                    if (fsErrorCode(error) !== code) {{
                        throw Error('async mkdir expected ' + code + ', got ' +
                            describeError(error));
                    }}
                    if (leaksPath(error, file)) throw Error('async mkdir leaked the path');
                    return;
                }}
                throw Error('async mkdir did not reject with ' + code);
            }}
            function expectSync(code, action) {{
                try {{ action(); }}
                catch (error) {{
                    if (fsErrorCode(error) !== code) {{
                        throw Error('sync mkdir expected ' + code + ', got ' +
                            describeError(error));
                    }}
                    if (leaksPath(error, file)) throw Error('sync mkdir leaked the path');
                    return;
                }}
                throw Error('mkdirSync did not throw ' + code);
            }}

            await expectAsync('FS_ALREADY_EXISTS', () => mkdir(file));
            await expectAsync('FS_ALREADY_EXISTS', () => mkdir(file, {{recursive: true}}));
            expectSync('FS_ALREADY_EXISTS', () => mkdirSync(file));
            expectSync('FS_ALREADY_EXISTS', () => mkdirSync(file, {{recursive: true}}));
            if (!lstatSync(file).isFile) throw Error('mkdir replaced the regular file');
            if (readFileSync(file)[0] !== 120) throw Error('mkdir changed the file contents');"#,
            error_helpers = FS_TEST_ERROR_HELPERS,
            file = literal(&file),
        ),
    );
    assert_eq!(fs::read(&file).unwrap(), b"x");
}

#[cfg(unix)]
#[test]
fn operations_report_denied_without_leaking_the_path() {
    use std::fs::Permissions;
    use std::os::unix::fs::PermissionsExt;

    // Traversal needs the execute bit, so denying the parent directory denies
    // metadata for the entry even when the entry itself is readable.
    if unsafe { libc::geteuid() } == 0 {
        eprintln!("denial test skipped: running as root");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let locked = dir.path().join("locked");
    let inner = locked.join("inner.txt");
    fs::create_dir(&locked).unwrap();
    fs::write(&inner, b"secret").unwrap();
    struct Restore(std::path::PathBuf);
    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = fs::set_permissions(&self.0, Permissions::from_mode(0o755));
        }
    }
    let _restore = Restore(locked.clone());
    fs::set_permissions(&locked, Permissions::from_mode(0o000)).unwrap();

    let js = runtime();
    run(
        &js,
        &format!(
            r#"import {{stat, statSync, lstat, readDir, readDirSync, mkdir, mkdirSync, FsError}} from 'neo:fs';
            const inner = {inner};
            const locked = {locked};
            function expectFsError(code, action) {{
                try {{ action(); }}
                catch (error) {{
                    if (!(error instanceof FsError) || error.code !== code) throw error;
                    if (error.message.includes(inner)) throw Error('denial leaked the path');
                    return;
                }}
                throw Error('did not throw ' + code);
            }}
            try {{ await stat(inner); throw Error('async stat succeeded'); }}
            catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_DENIED') throw error;
                if (error.message.includes(inner)) throw Error('async denial leaked the path');
            }}
            try {{ await lstat(inner); throw Error('async lstat succeeded'); }}
            catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_DENIED') throw error;
            }}
            try {{ await readDir(inner); throw Error('async readDir succeeded'); }}
            catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_DENIED') throw error;
                if (error.message.includes(inner)) throw Error('readDir denial leaked the path');
            }}
            try {{ await mkdir(locked + '/child'); throw Error('async mkdir succeeded'); }}
            catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_DENIED') throw error;
                if (error.message.includes(inner)) throw Error('mkdir denial leaked the path');
            }}
            expectFsError('FS_DENIED', () => statSync(inner));
            expectFsError('FS_DENIED', () => readDirSync(inner));
            expectFsError('FS_DENIED', () => mkdirSync(locked + '/child'));"#,
            inner = literal(&inner),
            locked = literal(&locked),
        ),
    );
}

#[test]
fn computed_dynamic_import_and_transitive_top_level_await_use_captured_cwd() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    fs::write(first.path().join("bytes"), b"a").unwrap();
    fs::write(second.path().join("bytes"), b"b").unwrap();
    fs::write(
        first.path().join("leaf.js"),
        format!(
            "import {{readFile}} from 'neo:fs';
             import {{chdir}} from 'neo:process';
             const pending = readFile('bytes'); chdir({});
             export const before = (await pending)[0];
             export const after = (await readFile('bytes'))[0];",
            literal(second.path()),
        ),
    )
    .unwrap();
    fs::write(
        first.path().join("dep.js"),
        "export {before, after} from './leaf.js';",
    )
    .unwrap();
    let entry = first.path().join("entry.js");
    let js = chunked_runtime();
    js.run_module(
        entry.to_str().unwrap(),
        &format!(
            "import {{chdir}} from 'neo:process'; chdir({});
             const pieces = ['.', 'dep.js'];
             const dep = await import(pieces.join('/'));
             if (dep.before !== 97 || dep.after !== 98) throw Error('cwd not captured');",
            literal(first.path()),
        ),
    )
    .unwrap();
}

#[test]
fn owner_reports_unhandled_read_rejections_and_stalled_modules() {
    let dir = tempfile::tempdir().unwrap();
    let js = runtime();
    let missing = literal(&dir.path().join("not-here"));
    js.run_module(
        "caught.js",
        &format!(
            "const pending = fs.readFile({missing});
             await Promise.resolve(); pending.catch(() => {{}});"
        ),
    )
    .unwrap();
    let unhandled = js
        .run_module("unhandled.js", &format!("fs.readFile({missing});"))
        .unwrap_err();
    assert!(unhandled.contains("NEO_UNHANDLED_REJECTION"));
    let stalled = js
        .run_module(
            "stalled.js",
            "await new Promise(resolve => globalThis.resume = resolve);
             throw Error('a later turn must not run this');",
        )
        .unwrap_err();
    assert!(stalled.contains("NEO_MODULE_STALLED"));
    assert!(
        js.run_module("cannot-resume.js", "resume();")
            .unwrap_err()
            .contains("NEO_MODULE_ABORTED")
    );
}

#[test]
fn out_of_turn_jobs_or_resolvers_cannot_enter_a_managed_module() {
    let dir = tempfile::tempdir().unwrap();
    let missing = literal(&dir.path().join("should-never-read"));
    for initial in [
        format!("Promise.resolve().then(() => fs.readFile({missing}));"),
        format!(
            "new Promise(resolve => globalThis.resume = resolve).then(() => fs.readFile({missing}));"
        ),
    ] {
        let js = runtime();
        js.run(&initial).unwrap();
        let refusal = js.run_module("next.js", "").unwrap_err();
        assert!(refusal.contains("NEO_MODULE_DIRTY"), "{refusal}");
        assert!(js.eval("'x'").unwrap_err().contains("NEO_MODULE_ABORTED"));
    }
    let js = runtime();
    js.eval(&format!(
        "Promise.resolve().then(() => fs.readFile({missing})); 'saved'"
    ))
    .unwrap();
    assert!(
        js.run_module("eval-next.js", "")
            .unwrap_err()
            .contains("NEO_MODULE_DIRTY")
    );
    let js = runtime();
    js.run_module(
        "register.js",
        &format!(
            "import test from 'neo:test';
             test('schedules a job', () => {{ Promise.resolve().then(() => fs.readFile({missing})); }});"
        ),
    )
    .unwrap();
    assert!(js.run_pending_tests().unwrap().unwrap().all_passed());
    assert!(
        js.run_module("test-next.js", "")
            .unwrap_err()
            .contains("NEO_MODULE_DIRTY")
    );
}

#[test]
fn detached_dynamic_import_cannot_hide_stalled_top_level_await() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("pending.js"),
        "await new Promise(resolve => globalThis.resume = resolve);",
    )
    .unwrap();
    let entry = dir.path().join("entry.js");
    let js = runtime();
    let error = js
        .run_module(
            entry.to_str().unwrap(),
            "void import(['.', 'pending.js'].join('/'));",
        )
        .unwrap_err();
    assert!(error.contains("NEO_MODULE_STALLED"), "{error}");
    assert!(
        js.run("resume();")
            .unwrap_err()
            .contains("NEO_MODULE_ABORTED")
    );
}

#[test]
fn settled_async_dependency_is_cached_across_successful_turns() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("payload");
    fs::write(&file, b"a").unwrap();
    fs::write(
        dir.path().join("dep.js"),
        format!(
            "import {{readFile}} from 'neo:fs';
             export const byte = (await readFile({}))[0];",
            literal(&file),
        ),
    )
    .unwrap();
    let js = runtime();
    for (entry, byte) in [("first.js", b'a'), ("second.js", b'b')] {
        fs::write(&file, [byte]).unwrap();
        js.run_module(
            dir.path().join(entry).to_str().unwrap(),
            "const loaded = await import(['.', 'dep.js'].join('/'));
             if (loaded.byte !== 97) throw Error('dependency did not retain cached result');",
        )
        .unwrap();
    }
}

#[test]
fn async_path_errors_keep_codes_after_public_error_constructor_tampering() {
    let dir = tempfile::tempdir().unwrap();
    let js = chunked_runtime();
    let source = r#"import {chdir} from 'neo:process';
           import {readFile, writeFile, FsError, MAX_PATH_BYTES} from 'neo:fs';
           chdir(__DIR__);
           Object.setPrototypeOf(FsError, function () { throw Error('intercepted'); });
           for (const [path, code] of [
               ['', 'FS_PATH'], ['\uD800', 'FS_ENCODING'],
               ['x'.repeat(MAX_PATH_BYTES + 1), 'FS_LIMIT'], ['x\0', 'FS_PATH'],
           ]) {
               for (const action of [
                   () => readFile(path),
                   () => writeFile(path, new Uint8Array([1])),
               ]) {
                   try { await action(); throw Error('expected rejection'); }
                   catch (error) {
                       if (!(error instanceof FsError) || error.code !== code) throw error;
                   }
               }
           }"#
    .replace("__DIR__", &literal(dir.path()));
    run(&js, &source);
}

#[test]
fn a_failed_module_cannot_submit_reads_through_jobs_in_a_later_turn() {
    let dir = tempfile::tempdir().unwrap();
    let missing = literal(&dir.path().join("should-not-be-read"));
    let js = runtime();
    let failure = js
        .run_module(
            "failed.js",
            &format!(
                "Promise.resolve().then(() => fs.readFile({missing}));
                 throw Error('entry failure');"
            ),
        )
        .unwrap_err();
    assert!(failure.contains("entry failure"));
    let next = js.run_module("next.js", "").unwrap_err();
    assert!(next.contains("NEO_MODULE_ABORTED"));
}

#[test]
fn text_and_binary_round_trip_with_unicode_names_and_content() {
    let dir = tempfile::tempdir().unwrap();
    let text = dir.path().join("héllo-文件-😀.txt");
    let binary = dir.path().join("bytes.bin");

    let js = runtime();
    let source = r#"
        import {readText, writeText, readFileSync, writeFile} from 'neo:fs';
        const textPath = __TEXT__;
        const binaryPath = __BINARY__;
        const text = '\uFEFFline 1\r\nhéllo 世界 😀\u0000tail';
        writeText(textPath, text);
        if (readText(textPath) !== text) throw Error('readText text');
        if (fs.readText(textPath) !== text) throw Error('global readText text');

        const bytes = new Uint8Array(256);
        for (let index = 0; index < 256; index++) bytes[index] = index;
        await writeFile(binaryPath, bytes);
        const back = readFileSync(binaryPath);
        if (!(back instanceof Uint8Array)) throw Error('not a Uint8Array');
        if (back.length !== 256) throw Error('wrong length');
        for (let index = 0; index < 256; index++) {
            if (back[index] !== index) throw Error('wrong byte ' + index);
        }

        // A subarray writes only its own view, not the whole backing buffer.
        await writeFile(binaryPath, bytes.subarray(10, 13));
        const view = readFileSync(binaryPath);
        if (view.length !== 3 || view[0] !== 10 || view[1] !== 11 || view[2] !== 12) {
            throw Error('subarray length');
        }
    "#
    .replace("__TEXT__", &literal(&text))
    .replace("__BINARY__", &literal(&binary));
    run(&js, &source);

    assert_eq!(fs::read(&binary).unwrap(), vec![10, 11, 12]);
    assert_eq!(
        fs::read_to_string(&text).unwrap(),
        "\u{FEFF}line 1\r\nhéllo 世界 😀\0tail"
    );
}

#[test]
fn writing_empty_text_truncates_an_existing_file_to_zero_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("existing.txt");
    fs::write(&path, b"must be truncated").unwrap();

    let js = runtime();
    let source = r#"
        import {readText, writeText} from 'neo:fs';
        const path = __PATH__;
        writeText(path, '');
        if (readText(path) !== '') throw Error('empty text did not overwrite file');
    "#
    .replace("__PATH__", &literal(&path));
    run(&js, &source);

    let metadata = fs::metadata(path).unwrap();
    assert!(metadata.is_file());
    assert_eq!(metadata.len(), 0);
}

#[test]
fn stable_errors_never_include_the_supplied_path() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("existing.txt"), "still usable").unwrap();
    let js = chunked_runtime();
    let source = r#"
        import {chdir} from 'neo:process';
        import {readText, readFileSync, writeText, writeFile, writeFileSync, statSync, exists, readDirSync, mkdirSync, removeSync, FsError} from 'neo:fs';
        chdir(__DIR__);

        function expectFsError(code, action) {
            try { action(); }
            catch (error) {
                if (!(error instanceof FsError)) throw Error('not FsError: ' + error);
                if (error.name !== 'FsError') throw Error('wrong name: ' + error.name);
                if (error.code !== code) throw Error('wrong code: ' + error.code + ', wanted ' + code);
                return error;
            }
            throw Error('did not throw ' + code);
        }
        function expectTypeError(action) {
            try { action(); }
            catch (error) {
                if (error instanceof TypeError) return error;
                throw Error('not TypeError: ' + error);
            }
            throw Error('did not throw TypeError');
        }

        const missing = __MISSING__;
        const error = expectFsError('FS_NOT_FOUND', () => readText(missing));
        if (error.message.includes(missing)) throw Error('path leaked in message');
        expectFsError('FS_NOT_FOUND', () => readFileSync(missing));
        expectFsError('FS_NOT_FOUND', () => statSync(missing));
        expectFsError('FS_NOT_FOUND', () => removeSync(missing));

        expectFsError('FS_PATH', () => readText(''));
        expectFsError('FS_PATH', () => readText('a\u0000b'));
        expectFsError('FS_PATH', () => statSync(''));
        expectFsError('FS_PATH', () => exists(''));
        expectFsError('FS_PATH', () => readDirSync(''));
        expectFsError('FS_PATH', () => mkdirSync(''));
        expectFsError('FS_PATH', () => removeSync(''));
        expectFsError('FS_LIMIT', () => readText('x'.repeat(fs.MAX_PATH_BYTES + 1)));
        expectFsError('FS_ENCODING', () => readText('bad\uD800'));
        expectFsError('FS_ENCODING', () => writeText(missing, 'bad\uD800'));

        expectTypeError(() => readText(123));
        expectTypeError(() => readFileSync(null));
        expectTypeError(() => writeFileSync(missing, 'text'));
        expectTypeError(() => writeFileSync(missing, new DataView(new ArrayBuffer(2))));
        expectTypeError(() => writeFileSync(missing, new Uint16Array([1])));
        expectTypeError(() => writeFileSync(missing, new Uint8ClampedArray([1])));
        expectTypeError(() => writeFileSync(missing, new Uint8Array(0), null));
        expectTypeError(() => writeFileSync(missing, new Uint8Array(0), {overwrite: 'yes'}));
        expectTypeError(() => mkdirSync(missing, null));
        expectTypeError(() => mkdirSync(missing, {recursive: 'yes'}));
        expectTypeError(() => removeSync(missing, {recursive: 1}));
        expectTypeError(() => statSync({}));
        expectTypeError(() => readDirSync([]));

        let traps = 0;
        const proxy = new Proxy(new Uint8Array([1]), {
            get() { traps += 1; return undefined; },
            getPrototypeOf() { traps += 1; return Uint8Array.prototype; },
            getOwnPropertyDescriptor() { traps += 1; return undefined; },
        });
        expectTypeError(() => writeFileSync(missing, proxy));
        if (traps !== 0) throw Error('proxy traps were observed: ' + traps);

        // The asynchronous form rejects with the same stable, path-free
        // error instead of throwing synchronously inside a managed turn.
        const missingParent = __MISSING_PARENT__;
        try {
            await writeFile(missingParent, new Uint8Array(0));
            throw Error('expected async rejection');
        } catch (error) {
            if (!(error instanceof FsError) || error.code !== 'FS_NOT_FOUND') throw error;
            if (error.message.includes(missingParent)) throw Error('async path leaked in message');
        }

        // A caught brand exception must not poison the following
        // host call with a pending QuickJS exception.
        if (readText(__EXISTING__) !== 'still usable') throw Error('pending exception leaked');

        const constructed = new FsError('FS_IO');
        if (!(constructed instanceof Error)) throw Error('FsError is not an Error');
        if (constructed.code !== 'FS_IO') throw Error('FsError code');
        if (!constructed.message.includes('filesystem')) throw Error('FsError message');
    "#
    .replace("__DIR__", &literal(dir.path()))
    .replace("__MISSING__", &literal(&dir.path().join("no-such-file")))
    .replace("__MISSING_PARENT__", &literal(&dir.path().join("no-such-dir/file")))
    .replace("__EXISTING__", &literal(&dir.path().join("existing.txt")));
    run(&js, &source);
}

#[test]
fn binary_write_snapshots_live_view_bounds_and_rejects_invalid_views_before_options() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("view.bin");
    fs::write(&file, [7, 8, 9]).unwrap();

    let source = r#"
        import {writeFile, readFileSync, FsError} from 'neo:fs';
        const file = __FILE__;
        let traps = 0;
        const options = new Proxy({}, {
            ownKeys() { traps++; return []; },
        });
        function assertBytes(expected) {
            const actual = readFileSync(file);
            if (actual.length !== expected.length) throw Error('wrong length: ' + actual);
            for (let i = 0; i < actual.length; i++) {
                if (actual[i] !== expected[i]) throw Error('wrong byte: ' + actual);
            }
        }
        async function reject(data, code) {
            let failure;
            try { await writeFile(file, data, options); }
            catch (error) { failure = error; }
            if (!(failure instanceof FsError) || failure.code !== code) {
                throw Error('wanted ' + code + ', got ' + failure?.code);
            }
            if (traps !== 0) throw Error('invalid view reached options');
            assertBytes([7, 8, 9]);
        }

        const buffer = new ArrayBuffer(3, {maxByteLength: 8});
        const fixed = new Uint8Array(buffer, 1, 2);
        const tracking = new Uint8Array(buffer, 1);
        buffer.resize(2);
        await reject(fixed, 'FS_OUT_OF_BOUNDS');

        const detachedBuffer = new ArrayBuffer(0);
        const detached = new Uint8Array(detachedBuffer);
        detachedBuffer.transfer();
        await reject(detached, 'FS_DETACHED');
        await reject(new Uint8Array(new SharedArrayBuffer(4)), 'FS_UNSUPPORTED');

        buffer.resize(5);
        tracking.set([1, 2, 3, 4]);
        for (const key of ['length', 'byteLength', 'byteOffset', 'buffer', 'constructor']) {
            Object.defineProperty(tracking, key, {
                get() { throw Error('input getter ran: ' + key); },
            });
        }
        await writeFile(file, tracking, new Proxy({}, {
            ownKeys() {
                buffer.resize(1);
                return [];
            },
        }));
        assertBytes([1, 2, 3, 4]);

        // A length-tracking view at the buffer's end is valid, not OOB.
        await writeFile(file, tracking);
        assertBytes([]);
        await writeFile(file, new Uint8Array(0));
        assertBytes([]);
    "#
    .replace("__FILE__", &literal(&file));
    run(&runtime(), &source);
}

#[test]
fn overwrite_no_clobber_mkdir_and_remove_are_explicit() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("file.txt");
    let fresh = dir.path().join("fresh.txt");
    let nested = dir.path().join("one/two/three");
    let empty = dir.path().join("empty");
    let tree = dir.path().join("tree");

    let js = runtime();
    let source = r#"
        import {readText, writeText, writeFileSync, mkdirSync, removeSync, exists, FsError} from 'neo:fs';
        __ERROR_HELPERS__
        function expectFsError(code, action) {
            try { action(); }
            catch (error) {
                if (fsErrorCode(error) === code) return;
                throw Error('wrong code: expected ' + code + ', got ' + describeError(error));
            }
            throw Error('did not throw ' + code);
        }

        const file = __FILE__;
        const fresh = __FRESH__;
        const nested = __NESTED__;
        const empty = __EMPTY__;
        const tree = __TREE__;
        const dir = __DIR__;
        const missing = __MISSING__;

        writeText(file, 'first');
        expectFsError('FS_ALREADY_EXISTS', () => writeFileSync(file, new Uint8Array([50]), {overwrite: false}));
        if (readText(file) !== 'first') throw Error('no-clobber changed the file');

        writeFileSync(file, new Uint8Array([0x73]));
        if (readText(file) !== 's') throw Error('default overwrite failed');
        writeText(file, 'third');
        if (readText(file) !== 'third') throw Error('writeText overwrite failed');

        writeFileSync(fresh, new Uint8Array([0x78]), {overwrite: false});
        if (readText(fresh) !== 'x') throw Error('create-new failed');

        expectFsError('FS_IS_DIRECTORY', () => writeFileSync(dir, new Uint8Array([1])));
        expectFsError('FS_NOT_FOUND', () => writeFileSync(missing, new Uint8Array([1])));

        expectSyncSuccess('overwrite test: sync mkdir creates an empty directory',
            () => mkdirSync(empty));
        expectFsError('FS_ALREADY_EXISTS', () => mkdirSync(empty));
        expectSyncSuccess('overwrite test: recursive sync mkdir accepts a directory',
            () => mkdirSync(empty, {recursive: true}));
        expectSyncSuccess('overwrite test: recursive sync mkdir creates parents',
            () => mkdirSync(nested, {recursive: true}));
        if (!exists(nested)) throw Error('recursive mkdir failed');
        removeSync(empty);
        if (exists(empty)) throw Error('remove empty dir failed');

        expectSyncSuccess('overwrite test: sync mkdir creates a tree root',
            () => mkdirSync(tree));
        writeText(tree + '/inside.txt', 'inside');
        expectFsError('FS_NOT_EMPTY', () => removeSync(tree));
        removeSync(tree, {recursive: true});
        if (exists(tree)) throw Error('recursive remove failed');
    "#
    .replace("__ERROR_HELPERS__", FS_TEST_ERROR_HELPERS)
    .replace("__FILE__", &literal(&file))
    .replace("__FRESH__", &literal(&fresh))
    .replace("__NESTED__", &literal(&nested))
    .replace("__EMPTY__", &literal(&empty))
    .replace("__TREE__", &literal(&tree))
    .replace("__DIR__", &literal(dir.path()))
    .replace("__MISSING__", &literal(&dir.path().join("missing/parent/file")));
    run(&js, &source);
}

#[test]
fn options_are_own_data_properties_and_paths_snapshot_cwd_before_proxy_traps() {
    let old = tempfile::tempdir().unwrap();
    let new = tempfile::tempdir().unwrap();
    fs::create_dir(old.path().join("target")).unwrap();
    fs::write(old.path().join("target/child"), b"old").unwrap();
    fs::create_dir(new.path().join("target")).unwrap();
    fs::write(new.path().join("target/child"), b"new").unwrap();
    fs::write(old.path().join("file"), b"old").unwrap();

    let js = chunked_runtime();
    let source = r#"
        import {chdir} from 'neo:process';
        import {readText, writeFileSync, mkdirSync, removeSync, exists} from 'neo:fs';
        function expectFsError(code, action) {
            try { action(); }
            catch (error) {
                if (error.code !== code) throw Error('wrong code: ' + error.code);
                return;
            }
            throw Error('did not throw ' + code);
        }
        function expectTypeError(action) {
            try { action(); }
            catch (error) {
                if (error instanceof TypeError) return;
                throw Error('not TypeError: ' + error);
            }
            throw Error('did not throw TypeError');
        }

        chdir(__OLD__);

        // Inherited options do not select destructive modes.
        expectFsError('FS_NOT_EMPTY', () =>
            removeSync('target', Object.create({recursive: true})));
        writeFileSync('file', new Uint8Array([111]), Object.create({overwrite: false}));
        if (readText('file') !== 'o') throw Error('inherited overwrite changed mode');

        let getterCalls = 0;
        const accessor = {};
        Object.defineProperty(accessor, 'recursive', {
            __proto__: null,
            enumerable: true,
            get() { getterCalls++; return true; },
        });
        expectTypeError(() => removeSync('target', accessor));
        if (getterCalls !== 0) throw Error('option getter was invoked');

        const hidden = {};
        Object.defineProperty(hidden, 'recursive', {
            __proto__: null,
            enumerable: false,
            value: true,
        });
        expectTypeError(() => removeSync('target', hidden));
        expectTypeError(() => mkdirSync('another', {recursive: true, unexpected: 1}));

        // Proxy reflection traps can run user JS, but the path has already
        // been resolved to this runtime's old cwd before the trap changes cwd.
        let trapCalls = 0;
        const proxy = new Proxy({}, {
            ownKeys() {
                trapCalls++;
                chdir(__NEW__);
                return ['recursive'];
            },
            getOwnPropertyDescriptor(_target, key) {
                if (key !== 'recursive') return undefined;
                return {value: true, enumerable: true, configurable: true, writable: true};
            },
        });
        removeSync('target', proxy);
        if (trapCalls !== 1) throw Error('proxy trap count');
        if (exists('target') !== true) throw Error('proxy trap redirected deletion');
        if (readText('target/child') !== 'new') throw Error('new cwd target changed');
        chdir(__OLD__);
        if (exists('target') !== false) throw Error('old cwd target survived');
    "#
    .replace("__OLD__", &literal(old.path()))
    .replace("__NEW__", &literal(new.path()));
    run(&js, &source);
}

#[cfg(unix)]
#[test]
fn symlink_semantics_are_explicit() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target.txt");
    let link = dir.path().join("link.txt");
    let dangling = dir.path().join("dangling.txt");
    let outside = dir.path().join("outside.txt");
    let tree = dir.path().join("tree");
    fs::write(&target, b"abc").unwrap();
    fs::write(&outside, b"keep").unwrap();
    symlink(&target, &link).unwrap();
    symlink(dir.path().join("missing"), &dangling).unwrap();
    fs::create_dir(&tree).unwrap();
    fs::write(tree.join("inside.txt"), b"inside").unwrap();
    symlink(&outside, tree.join("outside-link")).unwrap();

    let js = runtime();
    let source = r#"
        import {readText, readFileSync, statSync, lstatSync, exists, readDirSync, removeSync, FsError} from 'neo:fs';
        function expectFsError(code, action) {
            try { action(); }
            catch (error) {
                if (error.code !== code) throw Error('wrong code: ' + error.code);
                return;
            }
            throw Error('did not throw ' + code);
        }

        const dir = __DIR__;
        const target = __TARGET__;
        const link = __LINK__;
        const dangling = __DANGLING__;
        const outside = __OUTSIDE__;
        const tree = __TREE__;

        const followed = statSync(link);
        if (!followed.isFile || followed.isDirectory || followed.isSymlink) throw Error('stat did not follow');
        if (followed.size !== '3') throw Error('stat size is not a decimal string');
        if (typeof followed.mtimeMs !== 'string') throw Error('mtime is not a decimal string');

        const linkInfo = lstatSync(link);
        if (linkInfo.isFile || linkInfo.isDirectory || !linkInfo.isSymlink) throw Error('lstat followed');

        if (exists(link) !== true) throw Error('exists link');
        if (exists(dangling) !== false) throw Error('exists dangling');
        if (lstatSync(dangling).isSymlink !== true) throw Error('lstat dangling');
        expectFsError('FS_NOT_FOUND', () => statSync(dangling));
        if (readFileSync(link)[0] !== 97) throw Error('read did not follow link');

        const entries = readDirSync(dir);
        const names = entries.map((entry) => entry.name);
        if (names.join(',') !== 'dangling.txt,link.txt,outside.txt,target.txt,tree') {
            throw Error('listing order or names: ' + names.join(','));
        }
        const linkEntry = entries.find((entry) => entry.name === 'link.txt');
        if (!linkEntry.isSymlink || linkEntry.isFile || linkEntry.isDirectory) {
            throw Error('listing followed link');
        }
        const treeEntry = entries.find((entry) => entry.name === 'tree');
        if (!treeEntry.isDirectory || treeEntry.isSymlink) throw Error('listing directory');

        removeSync(link);
        if (exists(link)) throw Error('remove link left the link');
        if (readText(target) !== 'abc') throw Error('remove link followed to target');

        // Recursive deletion must not follow a symlink inside the tree.
        removeSync(tree, {recursive: true});
        if (exists(tree)) throw Error('recursive remove left the tree');
        if (readText(outside) !== 'keep') throw Error('recursive remove followed a link');
        if (fs.exists(outside) !== true) throw Error('outside target disappeared');
    "#
    .replace("__DIR__", &literal(dir.path()))
    .replace("__TARGET__", &literal(&target))
    .replace("__LINK__", &literal(&link))
    .replace("__DANGLING__", &literal(&dangling))
    .replace("__OUTSIDE__", &literal(&outside))
    .replace("__TREE__", &literal(&tree));
    run(&js, &source);
}

#[cfg(unix)]
#[test]
fn remove_rejects_symlink_trailing_components_and_never_resolves_them() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target");
    let link = dir.path().join("link");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("important.txt"), b"keep").unwrap();
    symlink(&target, &link).unwrap();

    let js = runtime();
    let source = r#"
        import {readText, removeSync, exists, FsError} from 'neo:fs';
        function expectPathError(action) {
            try { action(); }
            catch (error) {
                if (!(error instanceof FsError) || error.code !== 'FS_PATH') {
                    throw Error('wrong error: ' + error);
                }
                return;
            }
            throw Error('unsafe terminal path was accepted');
        }
        const link = __LINK__;
        const targetFile = __TARGET_FILE__;
        for (const suffix of ['/', '/.', '/..']) {
            expectPathError(() => removeSync(link + suffix, {recursive: true}));
        }
        // Even a regressed guard must not attempt filesystem I/O at root.
        expectPathError(() => removeSync('/', new Proxy({}, {
            ownKeys() { throw Error('root path reached option reflection'); },
        })));
        if (!exists(link)) throw Error('link was deleted');
        if (readText(targetFile) !== 'keep') throw Error('target was touched');

        // Bare directory symlink removal deletes the link itself, not target.
        removeSync(link, {recursive: true});
        if (exists(link)) throw Error('bare symlink was retained');
        if (readText(targetFile) !== 'keep') throw Error('bare link removal followed target');
    "#
    .replace("__LINK__", &literal(&link))
    .replace("__TARGET_FILE__", &literal(&target.join("important.txt")));
    run(&js, &source);
}

#[cfg(unix)]
#[test]
fn async_removal_rejects_symlink_trailing_components_and_never_resolves_them() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target");
    let guarded = dir.path().join("guarded");
    let bare = dir.path().join("bare");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("important.txt"), b"keep").unwrap();
    symlink(&target, &guarded).unwrap();
    symlink(&target, &bare).unwrap();

    let js = runtime();
    run(
        &js,
        &format!(
            r#"import {{readText, remove, exists, FsError}} from 'neo:fs';
            async function expectPathError(label, action) {{
                try {{ await action(); }}
                catch (error) {{
                    if (!(error instanceof FsError) || error.code !== 'FS_PATH') {{
                        throw Error(label + ': wrong error: ' + error);
                    }}
                    return;
                }}
                throw Error(label + ': unsafe terminal path was accepted');
            }}
            const guarded = {guarded};
            const bare = {bare};
            const targetFile = {target_file};
            for (const suffix of ['/', '/.', '/..']) {{
                await expectPathError('trailing component',
                    () => remove(guarded + suffix, {{recursive: true}}));
            }}
            // Even a regressed guard must not attempt filesystem I/O at root.
            await expectPathError('root path', () => remove('/', new Proxy({{}}, {{
                ownKeys() {{ throw Error('root path reached option reflection'); }},
            }})));
            if (!exists(guarded)) throw Error('link was deleted');
            if (readText(targetFile) !== 'keep') throw Error('target was touched');

            // Bare directory symlink removal deletes the link itself, not target.
            await remove(bare, {{recursive: true}});
            if (exists(bare)) throw Error('bare symlink was retained');
            if (readText(targetFile) !== 'keep') throw Error('bare link removal followed target');"#,
            guarded = literal(&guarded),
            bare = literal(&bare),
            target_file = literal(&target.join("important.txt")),
        ),
    );
}

#[cfg(unix)]
#[test]
fn removal_follows_an_intermediate_directory_link_into_an_owned_target() {
    use std::os::unix::fs::symlink;

    let base = tempfile::tempdir().unwrap();
    let unrelated = tempfile::tempdir().unwrap();
    let tree = base.path().join("tree");
    let target = base.path().join("target");
    fs::create_dir(&tree).unwrap();
    fs::create_dir(&target).unwrap();
    symlink(&target, tree.join("link")).unwrap();
    fs::write(target.join("victim-file.txt"), b"victim").unwrap();
    fs::create_dir(target.join("victim-dir")).unwrap();
    fs::write(target.join("victim-dir/inside.txt"), b"victim").unwrap();
    fs::write(target.join("sync-file.txt"), b"victim").unwrap();
    fs::create_dir(target.join("sync-dir")).unwrap();
    fs::write(target.join("sync-dir/inside.txt"), b"victim").unwrap();
    fs::write(target.join("keep.txt"), b"keep").unwrap();
    let sentinel = base.path().join("sentinel.txt");
    fs::write(&sentinel, b"keep").unwrap();
    fs::write(unrelated.path().join("keep.txt"), b"keep").unwrap();

    let js = runtime();
    run(
        &js,
        &format!(
            r#"import {{remove, removeSync, exists, lstatSync}} from 'neo:fs';
            const tree = {tree};
            const target = {target};

            // An intermediate directory symlink is resolved by the OS. The
            // final victim is deleted inside the link target, outside the
            // apparent tree, while the link and every unrelated owned path
            // remain. There is no containment check or rollback.
            await remove(tree + '/link/victim-file.txt');
            if (exists(target + '/victim-file.txt')) {{
                throw Error('async intermediate link target file survived');
            }}
            await remove(tree + '/link/victim-dir', {{recursive: true}});
            if (exists(target + '/victim-dir')) {{
                throw Error('async intermediate link target tree survived');
            }}
            removeSync(tree + '/link/sync-file.txt');
            if (exists(target + '/sync-file.txt')) {{
                throw Error('sync intermediate link target file survived');
            }}
            removeSync(tree + '/link/sync-dir', {{recursive: true}});
            if (exists(target + '/sync-dir')) {{
                throw Error('sync intermediate link target tree survived');
            }}

            if (!lstatSync(tree + '/link').isSymlink) {{
                throw Error('intermediate link was replaced or removed');
            }}
            if (!exists(target + '/keep.txt')) {{
                throw Error('unrelated target entry was removed');
            }}"#,
            tree = literal(&tree),
            target = literal(&target),
        ),
    );
    assert!(!target.join("victim-file.txt").exists());
    assert!(!target.join("victim-dir").exists());
    assert!(!target.join("sync-file.txt").exists());
    assert!(!target.join("sync-dir").exists());
    assert!(target.join("keep.txt").exists());
    assert!(tree.join("link").is_symlink());
    assert!(sentinel.exists());
    assert!(unrelated.path().join("keep.txt").exists());
}

#[test]
fn relative_paths_use_each_runtime_working_directory() {
    let host_before = std::env::current_dir().unwrap();
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();

    let first_js = chunked_runtime();
    let first_source = r#"
        import {chdir} from 'neo:process';
        import {readText, writeText} from 'neo:fs';
        chdir(__DIR__);
        writeText('same.txt', 'first');
        if (readText('same.txt') !== 'first') throw Error('relative read');
    "#
    .replace("__DIR__", &literal(first.path()));
    run(&first_js, &first_source);

    let second_js = chunked_runtime();
    let second_source = r#"
        import {chdir} from 'neo:process';
        import {readText, writeText} from 'neo:fs';
        chdir(__DIR__);
        if (fs.exists('same.txt') !== false) throw Error('second runtime saw the first cwd');
        writeText('same.txt', 'second');
        if (readText('same.txt') !== 'second') throw Error('relative read');
    "#
    .replace("__DIR__", &literal(second.path()));
    run(&second_js, &second_source);

    assert_eq!(
        fs::read_to_string(first.path().join("same.txt")).unwrap(),
        "first"
    );
    assert_eq!(
        fs::read_to_string(second.path().join("same.txt")).unwrap(),
        "second"
    );
    assert_eq!(std::env::current_dir().unwrap(), host_before);
}

#[test]
fn rename_moves_files_and_nonempty_directories_using_runtime_cwd() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("entrée.txt");
    let tree = dir.path().join("tree");
    fs::write(&file, b"file").unwrap();
    fs::create_dir(&tree).unwrap();
    fs::write(tree.join("child.txt"), b"child").unwrap();

    let js = chunked_runtime();
    let source = r#"
        import {chdir} from 'neo:process';
        import {rename} from 'neo:fs';
        chdir(__DIR__);
        if (fs.rename('entrée.txt', 'moved.txt') !== undefined) throw Error('rename return');
        fs.rename = () => { throw Error('mutated global rename ran'); };
        rename('tree', 'moved-tree', new Proxy({}, {
            ownKeys() { throw Error('rename reflected on an extra argument'); },
        }));
    "#
    .replace("__DIR__", &literal(dir.path()));
    run(&js, &source);

    assert!(!file.exists());
    assert!(!tree.exists());
    assert_eq!(fs::read(dir.path().join("moved.txt")).unwrap(), b"file");
    assert_eq!(
        fs::read(dir.path().join("moved-tree").join("child.txt")).unwrap(),
        b"child"
    );
}

#[test]
fn rename_rejects_invalid_operands_and_reports_missing_paths_without_leaks() {
    let dir = tempfile::tempdir().unwrap();
    let existing = dir.path().join("private-existing.txt");
    let output = dir.path().join("private-output.txt");
    let missing = dir.path().join("private-missing.txt");
    let child = missing.join("child.txt");
    fs::write(&existing, b"keep").unwrap();

    let js = chunked_runtime();
    let source = r#"
        import {chdir} from 'neo:process';
        import {rename, FsError, MAX_PATH_BYTES} from 'neo:fs';
        chdir(__DIR__);
        const existing = 'private-existing.txt';
        const output = 'private-output.txt';
        const missing = 'private-missing.txt';

        function rejects(expected, source, destination) {
            try { rename(source, destination); }
            catch (error) {
                if (expected === 'TypeError' && error instanceof TypeError) return;
                if (!(error instanceof FsError) || error.code !== expected) throw error;
                if (error.message !== new FsError(expected).message) {
                    throw Error('rename did not return the fixed error message');
                }
                return;
            }
            throw Error('rename unexpectedly succeeded');
        }

        const hostile = new Proxy({}, {
            get() { throw Error('rename coerced its operand'); },
        });
        rejects('TypeError', hostile, output);
        rejects('TypeError', existing, hostile);
        rejects('TypeError', existing, undefined);
        for (const [code, path] of [
            ['FS_PATH', ''],
            ['FS_PATH', existing + '\0suffix'],
            ['FS_PATH', existing + '/'],
            ['FS_PATH', existing + '/.'],
            ['FS_PATH', existing + '/..'],
            ['FS_ENCODING', 'private-\uD800'],
            ['FS_LIMIT', '😀'.repeat(MAX_PATH_BYTES / 4) + 'a'],
        ]) {
            rejects(code, path, output);
            rejects(code, existing, path);
        }
        rejects('FS_PATH', missing, output + '/.');
        rejects('FS_NOT_FOUND', missing, existing);
        rejects('FS_NOT_FOUND', existing, missing + '/child.txt');
    "#
    .replace("__DIR__", &literal(dir.path()));
    run(&js, &source);

    assert_eq!(fs::read(&existing).unwrap(), b"keep");
    assert!(!output.exists());
    assert!(!missing.exists());
    assert!(!child.exists());
}

#[cfg(unix)]
#[test]
fn rename_moves_dangling_symlinks_and_replaces_destination_entries() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target");
    let link = dir.path().join("link");
    let replaced = dir.path().join("replaced");
    let source = dir.path().join("source");
    let moved = dir.path().join("moved");
    fs::write(&target, b"keep target").unwrap();
    symlink(&target, &replaced).unwrap();
    fs::write(&source, b"replacement").unwrap();
    symlink("missing-target", &link).unwrap();

    let js = runtime();
    let script = r#"
        import {rename} from 'neo:fs';
        rename(__LINK__, __MOVED__);
        rename(__SOURCE__, __REPLACED__);
    "#
    .replace("__LINK__", &literal(&link))
    .replace("__MOVED__", &literal(&moved))
    .replace("__SOURCE__", &literal(&source))
    .replace("__REPLACED__", &literal(&replaced));
    run(&js, &script);

    assert_eq!(
        fs::symlink_metadata(&link).unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
    assert_eq!(
        fs::read_link(&moved).unwrap(),
        std::path::Path::new("missing-target")
    );
    assert!(fs::symlink_metadata(&replaced).unwrap().is_file());
    assert!(!source.exists());
    assert_eq!(fs::read(&replaced).unwrap(), b"replacement");
    assert_eq!(fs::read(&target).unwrap(), b"keep target");
}

#[cfg(unix)]
#[test]
fn rename_replaces_a_file_but_does_not_predelete_a_nonempty_directory() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("file");
    let replacement = dir.path().join("replacement");
    let source = dir.path().join("source");
    let destination = dir.path().join("destination");
    fs::write(&file, b"old").unwrap();
    fs::write(&replacement, b"new").unwrap();
    fs::create_dir(&source).unwrap();
    fs::write(source.join("child"), b"keep source").unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(destination.join("child"), b"keep destination").unwrap();

    let js = runtime();
    let script = r#"
        import {rename, FsError} from 'neo:fs';
        rename(__REPLACEMENT__, __FILE__);
        let failure;
        try { rename(__SOURCE__, __DESTINATION__); }
        catch (error) { failure = error; }
        // POSIX permits either ENOTEMPTY or EEXIST for this replacement.
        if (!(failure instanceof FsError) ||
            !['FS_NOT_EMPTY', 'FS_ALREADY_EXISTS'].includes(failure.code)) {
            throw Error('expected a nonempty directory error');
        }
    "#
    .replace("__REPLACEMENT__", &literal(&replacement))
    .replace("__FILE__", &literal(&file))
    .replace("__SOURCE__", &literal(&source))
    .replace("__DESTINATION__", &literal(&destination));
    run(&js, &script);

    assert!(!replacement.exists());
    assert_eq!(fs::read(&file).unwrap(), b"new");
    assert_eq!(fs::read(source.join("child")).unwrap(), b"keep source");
    assert_eq!(
        fs::read(destination.join("child")).unwrap(),
        b"keep destination"
    );
}

#[test]
fn copy_file_copies_binary_data_using_the_runtime_cwd() {
    let dir = tempfile::tempdir().unwrap();
    let bytes: Vec<u8> = (0..=255).cycle().take(16_387).collect();
    let original = dir.path().join("entrée.bin");
    let output = dir.path().join("copy.bin");
    fs::write(&original, &bytes).unwrap();

    let host_before = std::env::current_dir().unwrap();
    let script = r#"
        import {chdir} from 'neo:process';
        import {copyFile} from 'neo:fs';
        chdir(__DIR__);
        if (fs.copyFile('entrée.bin', __OUTPUT__) !== undefined) throw Error('copy must be sync');
        fs.copyFile = () => { throw Error('mutated global copy ran'); };
        copyFile(__ORIGINAL__, 'second.bin');
    "#
    .replace("__DIR__", &literal(dir.path()))
    .replace("__OUTPUT__", &literal(&output))
    .replace("__ORIGINAL__", &literal(&original));
    run(&chunked_runtime(), &script);

    assert_eq!(fs::read(original).unwrap(), bytes);
    assert_eq!(fs::read(output).unwrap(), bytes);
    assert_eq!(fs::read(dir.path().join("second.bin")).unwrap(), bytes);
    assert_eq!(std::env::current_dir().unwrap(), host_before);
}

#[test]
fn copy_file_never_clobbers_existing_destinations_or_hardlinks() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("original");
    let existing = dir.path().join("existing");
    let alias = dir.path().join("alias");
    fs::write(&original, b"source data").unwrap();
    fs::write(&existing, b"destination data").unwrap();
    fs::hard_link(&original, &alias).unwrap();

    let script = r#"
        import {chdir} from 'neo:process';
        import {copyFile, FsError} from 'neo:fs';
        chdir(__DIR__);
        for (const [from, to, code] of [
            ['original', 'original', 'FS_ALREADY_EXISTS'],
            ['original', './original', 'FS_ALREADY_EXISTS'],
            ['original', 'alias', 'FS_ALREADY_EXISTS'],
            ['alias', 'original', 'FS_ALREADY_EXISTS'],
            ['original', 'existing', 'FS_ALREADY_EXISTS'],
            ['missing', 'existing', 'FS_NOT_FOUND'],
        ]) {
            try { copyFile(from, to); }
            catch (error) {
                if (!(error instanceof FsError) || error.code !== code) throw error;
                if (error.message !== new FsError(code).message) throw Error('unexpected message');
                continue;
            }
            throw Error('copy unexpectedly succeeded');
        }
    "#
    .replace("__DIR__", &literal(dir.path()));
    run(&chunked_runtime(), &script);

    assert_eq!(fs::read(original).unwrap(), b"source data");
    assert_eq!(fs::read(alias).unwrap(), b"source data");
    assert_eq!(fs::read(existing).unwrap(), b"destination data");
}

#[test]
fn copy_file_validates_both_primitive_entry_paths_before_opening() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("source"), b"unchanged").unwrap();
    fs::create_dir(dir.path().join("directory")).unwrap();
    let script = r#"
        import {chdir} from 'neo:process';
        import {copyFile, FsError, MAX_PATH_BYTES} from 'neo:fs';
        chdir(__DIR__);
        function rejects(code, from, to) {
            try { copyFile(from, to); }
            catch (error) {
                if (code === 'TypeError' && error instanceof TypeError) return;
                if (!(error instanceof FsError) || error.code !== code) throw error;
                if (error.message !== new FsError(code).message) throw Error('unexpected message');
                return;
            }
            throw Error('copy unexpectedly succeeded');
        }
        const hostile = new Proxy({}, {get() { throw Error('copy tried to coerce'); }});
        rejects('TypeError', hostile, 'output');
        rejects('TypeError', '', hostile);
        for (const [code, path] of [
            ['FS_PATH', ''], ['FS_PATH', 'bad\0path'],
            ['FS_PATH', 'directory/'], ['FS_PATH', 'directory/.'], ['FS_PATH', 'directory/..'],
            ['FS_ENCODING', 'bad-\uD800'], ['FS_LIMIT', 'a'.repeat(MAX_PATH_BYTES + 1)],
        ]) {
            rejects(code, path, 'output');
            rejects(code, 'source', path);
        }
        rejects('FS_PATH', 'missing', 'directory/.');
        rejects('FS_IS_DIRECTORY', 'directory', 'output');
        rejects('FS_NOT_FOUND', 'source', 'missing/child');
    "#
    .replace("__DIR__", &literal(dir.path()));
    run(&chunked_runtime(), &script);
    assert_eq!(fs::read(dir.path().join("source")).unwrap(), b"unchanged");
    assert!(!dir.path().join("output").exists());
    assert!(!dir.path().join("missing").exists());
}

#[cfg(unix)]
#[test]
fn copy_file_follows_source_symlinks_but_never_follows_destination_symlinks() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("source"), b"source").unwrap();
    symlink("source", dir.path().join("linked")).unwrap();
    symlink("absent", dir.path().join("dangling")).unwrap();
    let script = r#"
        import {chdir} from 'neo:process';
        import {copyFile, FsError} from 'neo:fs';
        chdir(__DIR__);
        copyFile('linked', 'copy');
        for (const [from, to, code] of [
            ['source', 'linked', 'FS_ALREADY_EXISTS'],
            ['linked', 'source', 'FS_ALREADY_EXISTS'],
            ['source', 'dangling', 'FS_ALREADY_EXISTS'],
            ['dangling', 'copy', 'FS_NOT_FOUND'],
        ]) {
            try { copyFile(from, to); }
            catch (error) {
                if (!(error instanceof FsError) || error.code !== code) throw error;
                continue;
            }
            throw Error('copy unexpectedly succeeded');
        }
    "#
    .replace("__DIR__", &literal(dir.path()));
    run(&chunked_runtime(), &script);
    assert_eq!(fs::read(dir.path().join("copy")).unwrap(), b"source");
    assert_eq!(fs::read(dir.path().join("source")).unwrap(), b"source");
    assert_eq!(
        fs::read_link(dir.path().join("linked")).unwrap(),
        std::path::Path::new("source")
    );
    assert_eq!(
        fs::read_link(dir.path().join("dangling")).unwrap(),
        std::path::Path::new("absent")
    );
    assert!(!dir.path().join("absent").exists());
}

#[test]
fn copy_file_accepts_exact_limit_and_rejects_oversize_before_creation() {
    use std::io::{Read, Seek, SeekFrom, Write};

    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("exact");
    let oversize = dir.path().join("oversize");
    let js = chunked_runtime();
    let max: u64 = js
        .eval("String(fs.MAX_READ_BYTES)")
        .unwrap()
        .parse()
        .unwrap();
    let mut exact = fs::File::create(&original).unwrap();
    exact.seek(SeekFrom::Start(max - 1)).unwrap();
    exact.write_all(&[0xff]).unwrap();
    drop(exact);
    fs::File::create(&oversize)
        .unwrap()
        .set_len(max + 1)
        .unwrap();
    let script = r#"
        import {chdir} from 'neo:process';
        import {copyFile, FsError} from 'neo:fs';
        chdir(__DIR__);
        copyFile('exact', 'copy');
        for (const to of ['fresh', 'copy']) {
            try { copyFile('oversize', to); }
            catch (error) {
                if (!(error instanceof FsError) || error.code !== 'FS_LIMIT') throw error;
                continue;
            }
            throw Error('copied oversize file');
        }
    "#
    .replace("__DIR__", &literal(dir.path()));
    run(&js, &script);
    let mut copy = fs::File::open(dir.path().join("copy")).unwrap();
    assert_eq!(copy.metadata().unwrap().len(), max);
    copy.seek(SeekFrom::End(-1)).unwrap();
    let mut tail = [0];
    copy.read_exact(&mut tail).unwrap();
    assert_eq!(tail, [0xff]);
    assert!(!dir.path().join("fresh").exists());
}

#[cfg(unix)]
#[test]
fn copy_file_rejects_fifos_without_waiting_for_a_peer() {
    use std::{
        ffi::CString, os::unix::ffi::OsStrExt, os::unix::fs::FileTypeExt, sync::mpsc, thread,
        time::Duration,
    };

    let dir = tempfile::tempdir().unwrap();
    let fifo = dir.path().join("fifo");
    let path = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    // SAFETY: `path` is a NUL-terminated path in this test-owned tempdir.
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    fs::write(dir.path().join("source"), b"source").unwrap();
    let script = r#"
        import {chdir} from 'neo:process';
        import {copyFile, FsError} from 'neo:fs';
        chdir(__DIR__);
        for (const [from, to, code] of [
            ['fifo', 'no-output', 'FS_TYPE'],
            ['source', 'fifo', 'FS_ALREADY_EXISTS'],
        ]) {
            try { copyFile(from, to); }
            catch (error) {
                if (!(error instanceof FsError) || error.code !== code) throw error;
                continue;
            }
            throw Error('copied FIFO');
        }
    "#
    .replace("__DIR__", &literal(dir.path()));
    let (send, receive) = mpsc::channel();
    let worker = thread::spawn(move || {
        run(&chunked_runtime(), &script);
        send.send(()).unwrap();
    });
    receive
        .recv_timeout(Duration::from_secs(5))
        .expect("FIFO copy blocked or failed");
    worker.join().unwrap();
    assert!(fs::symlink_metadata(fifo).unwrap().file_type().is_fifo());
    assert!(!dir.path().join("no-output").exists());
}

#[test]
fn reads_are_bounded_before_allocation_and_sizes_stay_decimal() {
    let dir = tempfile::tempdir().unwrap();
    let big = dir.path().join("big.bin");

    let js = chunked_runtime();
    let max_read: u64 = js
        .eval("String(fs.MAX_READ_BYTES)")
        .unwrap()
        .parse()
        .unwrap();
    let handle = fs::File::create(&big).unwrap();
    handle.set_len(max_read + 1).unwrap();
    drop(handle);

    let source = r#"
        import {chdir} from 'neo:process';
        import {readText, readFileSync, writeFile, writeFileSync, statSync, exists, FsError} from 'neo:fs';
        chdir(__DIR__);
        function expectFsError(code, action) {
            try { action(); }
            catch (error) {
                if (error.code !== code) throw Error('wrong code: ' + error.code);
                return;
            }
            throw Error('did not throw ' + code);
        }

        const big = __BIG__;
        const output = __OUTPUT__;

        expectFsError('FS_LIMIT', () => readText(big));
        expectFsError('FS_LIMIT', () => readFileSync(big));
        try {
            await fs.readFile(big);
            throw Error('oversize async read fulfilled');
        } catch (error) {
            if (error.code !== 'FS_LIMIT') throw error;
        }
        if (statSync(big).size !== String(fs.MAX_READ_BYTES + 1)) throw Error('size rounding');

        // Check the limit before resolving or opening: the output must not exist.
        const tooLarge = new Uint8Array(fs.MAX_WRITE_BYTES + 1);
        expectFsError('FS_LIMIT', () => writeFileSync(output, tooLarge));
        if (exists(output)) throw Error('oversize write created a file');
        try {
            await writeFile(output, tooLarge);
            throw Error('oversize async write fulfilled');
        } catch (error) {
            if (error.code !== 'FS_LIMIT') throw error;
        }
        if (exists(output)) throw Error('oversize async write created a file');

        expectFsError('FS_LIMIT', () => readText('x'.repeat(fs.MAX_PATH_BYTES + 1)));
    "#
    .replace("__DIR__", &literal(dir.path()))
    .replace("__BIG__", &literal(&big))
    .replace("__OUTPUT__", &literal(&dir.path().join("output.bin")));
    run(&js, &source);

    assert!(!dir.path().join("output.bin").exists());
}

#[test]
fn utf8_limits_are_checked_in_bytes_before_native_string_conversion() {
    let dir = tempfile::tempdir().unwrap();
    let exact = dir.path().join("exact.txt");
    let too_large = dir.path().join("too-large.txt");
    let invalid_text = dir.path().join("invalid-utf8.txt");
    fs::write(&invalid_text, [0xff]).unwrap();

    let js = chunked_runtime();
    let source = r#"
        import {chdir} from 'neo:process';
        import {writeText, readText, statSync, exists, FsError} from 'neo:fs';
        chdir(__DIR__);
        function expectFsError(code, action) {
            try { action(); }
            catch (error) {
                if (error.code !== code) throw Error('wrong code: ' + error.code);
                return;
            }
            throw Error('did not throw ' + code);
        }

        const exact = __EXACT__;
        const tooLarge = __TOO_LARGE__;
        const invalid = __INVALID__;
        const size = fs.MAX_WRITE_BYTES;

        const atLimit = 'a'.repeat(size - 4) + '😀';
        writeText(exact, atLimit);
        if (statSync(exact).size !== String(size)) throw Error('exact UTF-8 byte boundary');
        if (readText(exact) !== atLimit) throw Error('exact boundary roundtrip');

        const aboveLimit = 'a'.repeat(size - 3) + '😀';
        expectFsError('FS_LIMIT', () => writeText(tooLarge, aboveLimit));
        if (exists(tooLarge)) throw Error('max+1 UTF-8 text created a file');
        expectFsError('FS_ENCODING', () => readText(invalid));

        // UTF-16 length stays below the cap, but encoded bytes exceed it.
        expectFsError('FS_LIMIT', () =>
            statSync('😀'.repeat(fs.MAX_PATH_BYTES / 4) + 'a'));
    "#
    .replace("__DIR__", &literal(dir.path()))
    .replace("__EXACT__", &literal(&exact))
    .replace("__TOO_LARGE__", &literal(&too_large))
    .replace("__INVALID__", &literal(&invalid_text));
    run(&js, &source);
    assert!(!too_large.exists());
}

#[test]
fn write_text_validates_content_before_resolving_or_opening_path() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("write-text-rejected.txt");
    let js = runtime();
    let source = r#"
        import {writeText, exists, FsError, MAX_WRITE_BYTES} from 'neo:fs';
        const output = __OUTPUT__;
        const secret = 'private-text-marker';
        function expectFsError(code, path, text) {
            try { writeText(path, text); }
            catch (error) {
                if (!(error instanceof FsError) || error.code !== code) {
                    throw Error('wrong error: expected ' + code + ', got ' + error.code);
                }
                if (error.message.includes(output) || error.message.includes(secret)) {
                    throw Error('argument leaked in error');
                }
                if (exists(output)) throw Error('rejected write created a file');
                return;
            }
            throw Error('did not throw ' + code);
        }

        expectFsError('FS_ENCODING', output, secret + '\uD800x');
        expectFsError('FS_ENCODING', output, secret + '\uDC00');
        // This prefix alone is over the byte cap, but the whole string fits
        // the code-unit cap, so the trailing lone surrogate wins.
        expectFsError('FS_ENCODING', output, 'é'.repeat(MAX_WRITE_BYTES / 2 + 1) + '\uD800');
        // If the code-unit cap is exceeded, the lone surrogate loses.
        expectFsError('FS_LIMIT', output, '\uDC00' + 'a'.repeat(MAX_WRITE_BYTES));
        expectFsError('FS_ENCODING', output + '\0tail', '\uD800');
        expectFsError('FS_PATH', output + '\0tail', 'valid');

        let hooks = 0;
        const boxed = new String(secret);
        boxed[Symbol.toPrimitive] = () => { hooks++; return secret; };
        const proxy = new Proxy(boxed, {
            get() { hooks++; },
            getPrototypeOf() { hooks++; },
        });
        for (const value of [boxed, proxy]) {
            try { writeText(output, value); }
            catch (error) {
                if (!(error instanceof TypeError)) throw error;
                continue;
            }
            throw Error('accepted a non-primitive string');
        }
        if (hooks !== 0) throw Error('non-primitive text hooks ran');
        if (exists(output)) throw Error('rejected write created a file');
    "#
    .replace("__OUTPUT__", &literal(&output));
    run(&js, &source);
    assert!(!output.exists());
}

#[test]
fn module_exports_survive_global_prototype_and_callback_replacement() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("payload.txt");
    fs::write(&file, b"payload").unwrap();

    let js = runtime();
    let source = r#"
        import {readText, readFileSync, statSync, readDirSync, FsError} from 'neo:fs';
        const oldError = Error;
        const oldParse = JSON.parse;
        try {
            Object.defineProperty(Object.prototype, 'ok', {
                __proto__: null,
                get() { throw new oldError('Object.prototype.ok ran'); },
                configurable: true,
            });
            Object.defineProperty(Object.prototype, 'value', {
                __proto__: null,
                get() { throw new oldError('Object.prototype.value ran'); },
                configurable: true,
            });
            Object.defineProperty(Object.prototype, 'code', {
                __proto__: null,
                get() { throw new oldError('Object.prototype.code ran'); },
                configurable: true,
            });

            if (readText(__FILE__) !== 'payload') throw new oldError('readText broke');
            if (readFileSync(__FILE__).length !== 7) throw new oldError('readFileSync broke');
            if (statSync(__FILE__).size !== '7') throw new oldError('stat broke');
            const oldArrayProto = Array.prototype;
            Object.defineProperty(oldArrayProto, '0', {
                __proto__: null,
                set() { throw new oldError('Array.prototype index setter ran'); },
                configurable: true,
            });
            try {
                const entries = readDirSync(__DIR__);
                if (entries.length !== 1 || entries[0].name !== 'payload.txt') {
                    throw new oldError('readDir entries changed');
                }
            } finally {
                delete oldArrayProto[0];
            }

            JSON.parse = () => { throw new oldError('JSON.parse replaced'); };
            fs.readText = () => { throw new oldError('global fs replaced'); };
            if (readText(__FILE__) !== 'payload') throw new oldError('module export changed');
            if (fs.readText === readText) throw new oldError('global mutation did not take');

            globalThis.Error = function () { throw new oldError('Error replaced'); };
            const constructed = new FsError('FS_IO');
            if (!(constructed instanceof oldError)) throw new oldError('FsError lost its base');
            if (constructed.code !== 'FS_IO') throw new oldError('FsError code lost');
        } finally {
            delete Object.prototype.ok;
            delete Object.prototype.value;
            delete Object.prototype.code;
            JSON.parse = oldParse;
            globalThis.Error = oldError;
        }
    "#
    .replace("__FILE__", &literal(&file))
    .replace("__DIR__", &literal(dir.path()));
    run(&js, &source);
}

#[cfg(unix)]
#[test]
fn listing_rejects_names_that_are_not_utf8() {
    use std::os::unix::ffi::OsStringExt;

    let dir = tempfile::tempdir().unwrap();
    let bad = std::ffi::OsString::from_vec(b"bad-\xff".to_vec());
    fs::write(dir.path().join(&bad), b"").unwrap();

    let js = runtime();
    let source = r#"
        import {readDirSync, FsError} from 'neo:fs';
        let failure;
        try { readDirSync(__DIR__); }
        catch (error) { failure = error; }
        if (!(failure instanceof FsError) || failure.code !== 'FS_ENCODING') {
            throw Error('invalid UTF-8 name: expected FS_ENCODING, got ' + failure?.code);
        }
    "#
    .replace("__DIR__", &literal(dir.path()));
    run(&js, &source);
}

#[cfg(windows)]
#[test]
fn windows_metadata_and_directory_reads_use_owned_fixtures() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("windows-file.txt");
    let child = dir.path().join("windows-child");
    fs::write(&file, b"win").unwrap();
    fs::create_dir(&child).unwrap();
    fs::write(child.join("inside.txt"), b"inside").unwrap();

    let js = runtime();
    run(
        &js,
        &format!(
            r#"import {{stat, statSync, lstat, lstatSync, readDir, readDirSync}} from 'neo:fs';
            const dir = {dir};
            const file = {file};
            const child = {child};

            const fileInfo = await stat(file);
            if (!fileInfo.isFile || fileInfo.isDirectory || fileInfo.isSymlink) {{
                throw Error('async stat did not report a regular file');
            }}
            if (fileInfo.size !== '3') throw Error('async stat size is not decimal bytes');
            if (typeof fileInfo.mtimeMs !== 'string') throw Error('async stat mtime is not a string');

            const fileLinkInfo = await lstat(file);
            if (!fileLinkInfo.isFile || fileLinkInfo.isSymlink) {{
                throw Error('async lstat did not report a regular file');
            }}
            if (statSync(file).size !== '3' || lstatSync(file).isFile !== true) {{
                throw Error('sync file metadata differs');
            }}

            const directoryInfo = await stat(child);
            if (!directoryInfo.isDirectory || directoryInfo.isFile || directoryInfo.isSymlink) {{
                throw Error('async stat did not report a directory');
            }}
            const directoryLinkInfo = await lstat(child);
            if (!directoryLinkInfo.isDirectory || directoryLinkInfo.isSymlink) {{
                throw Error('async lstat did not report a directory');
            }}

            const entries = await readDir(dir);
            const names = entries.map((entry) => entry.name).join(',');
            if (names !== 'windows-child,windows-file.txt') throw Error('async names: ' + names);
            const directoryEntry = entries[0];
            if (!directoryEntry.isDirectory || directoryEntry.isFile || directoryEntry.isSymlink) {{
                throw Error('async directory entry type');
            }}
            const fileEntry = entries[1];
            if (!fileEntry.isFile || fileEntry.isDirectory || fileEntry.isSymlink) {{
                throw Error('async file entry type');
            }}
            const syncEntries = readDirSync(dir);
            if (syncEntries.map((entry) => entry.name).join(',') !== names) {{
                throw Error('sync directory listing differs');
            }}
            const nested = await readDir(child);
            if (nested.length !== 1 || nested[0].name !== 'inside.txt' || !nested[0].isFile) {{
                throw Error('nested async directory listing');
            }}"#,
            dir = literal(dir.path()),
            file = literal(&file),
            child = literal(&child),
        ),
    );
}

#[cfg(windows)]
#[test]
fn windows_final_symlink_metadata_and_listing_fails_closed_without_capability() {
    use std::os::windows::fs::symlink_file;
    use windows_sys::Win32::Foundation::ERROR_PRIVILEGE_NOT_HELD;

    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("windows-target.txt");
    let link = dir.path().join("windows-link.txt");
    fs::write(&target, b"abc").unwrap();
    match symlink_file(&target, &link) {
        Ok(()) => {}
        Err(error) if error.raw_os_error() == Some(ERROR_PRIVILEGE_NOT_HELD as i32) => {
            // This gate proves native reparse behavior. A missing privilege is
            // a failed gate, not a skipped test that could be mistaken for
            // coverage.
            panic!(
                "Windows file symlink creation requires \
                 SeCreateSymbolicLinkPrivilege or Developer Mode; the native gate fails closed: \
                 {error}"
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::Unsupported => {
            panic!(
                "the file system does not support Windows file symlinks; the native gate fails \
                 closed instead of skipping: {error}"
            );
        }
        Err(error) => {
            panic!(
                "unexpected failure creating a Windows file symlink in an owned temp fixture: \
                 {error}"
            );
        }
    }

    let js = runtime();
    run(
        &js,
        &format!(
            r#"import {{stat, statSync, lstat, lstatSync, readDir, readDirSync}} from 'neo:fs';
            const dir = {dir};
            const link = {link};

            const followed = await stat(link);
            if (!followed.isFile || followed.isDirectory || followed.isSymlink) {{
                throw Error('async stat did not follow a final Windows symlink');
            }}
            if (followed.size !== '3') throw Error('async stat size after following is not 3');

            const linkInfo = await lstat(link);
            if (linkInfo.isFile || linkInfo.isDirectory || !linkInfo.isSymlink) {{
                throw Error('async lstat followed a final Windows symlink');
            }}
            if (!statSync(link).isFile || statSync(link).isSymlink) {{
                throw Error('sync stat did not follow a final Windows symlink');
            }}
            if (!lstatSync(link).isSymlink) {{
                throw Error('sync lstat did not report a final Windows symlink');
            }}

            const entries = await readDir(dir);
            const entry = entries.find((candidate) => candidate.name === 'windows-link.txt');
            if (!entry || !entry.isSymlink || entry.isFile || entry.isDirectory) {{
                throw Error('async readDir followed a reparse entry');
            }}
            const syncEntry = readDirSync(dir).find(
                (candidate) => candidate.name === 'windows-link.txt');
            if (!syncEntry || !syncEntry.isSymlink) {{
                throw Error('sync readDir followed a reparse entry');
            }}"#,
            dir = literal(dir.path()),
            link = literal(&link),
        ),
    );
}

#[cfg(windows)]
#[test]
fn windows_directory_creation_covers_missing_parents_and_intermediate_files() {
    // The shared mkdir test asserts only the documented cross-platform
    // contract. The Unix rule that a non-directory intermediate component
    // reports `FS_NOT_DIRECTORY` is explicitly Unix-scoped in the module docs.
    // Native Windows run 44943444444 at 8475b21 observed `FS_ALREADY_EXISTS`
    // for the async recursive form; the non-recursive form stayed within
    // `FS_NOT_FOUND`/`FS_NOT_DIRECTORY`. This is consistent with
    // `create_dir_all` returning the existing-file error from an ancestor
    // after its first creation attempt fails. The recursive case therefore
    // accepts the observed `FS_ALREADY_EXISTS`; the non-recursive case keeps
    // the narrower set. This is an owned-fixture platform expectation, not a
    // pre-I/O classification and not a production change.
    //
    // The module records every sanitized outcome and the Rust side logs it
    // before any assertion, so a failing classification still reports all
    // observed codes. Paths and arbitrary error properties are never included.
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("windows-file.txt");
    fs::write(&file, b"x").unwrap();
    let missing_parent = dir.path().join("missing/child");
    let nested_async = dir.path().join("nested-async/one/two");
    let nested_sync = dir.path().join("nested-sync/one/two");
    let below_file = file.join("child");
    let below_file_recursive = file.join("child-recursive");
    let js = runtime();
    run(
        &js,
        &format!(
            r#"import {{mkdir, mkdirSync, FsError}} from 'neo:fs';
            {error_helpers}
            const missingParent = {missing_parent};
            const nestedAsync = {nested_async};
            const nestedSync = {nested_sync};
            const belowFile = {below_file};
            const belowFileRecursive = {below_file_recursive};

            // Collect sanitized outcomes; do not assert here. The Rust side
            // logs the complete observation before checking it.
            async function collectAsyncSuccess(action) {{
                try {{ await action(); return 'OK'; }}
                catch (error) {{
                    const code = fsErrorCode(error);
                    return code !== null ? code : describeError(error);
                }}
            }}
            function collectSyncSuccess(action) {{
                try {{ action(); return 'OK'; }}
                catch (error) {{
                    const code = fsErrorCode(error);
                    return code !== null ? code : describeError(error);
                }}
            }}
            async function collectAsyncFailure(action, path) {{
                try {{ await action(); return 'OK'; }}
                catch (error) {{
                    const code = fsErrorCode(error);
                    const shown = code !== null ? code : describeError(error);
                    return leaksPath(error, path) ? shown + '+leak' : shown;
                }}
            }}
            function collectSyncFailure(action, path) {{
                try {{ action(); return 'OK'; }}
                catch (error) {{
                    const code = fsErrorCode(error);
                    const shown = code !== null ? code : describeError(error);
                    return leaksPath(error, path) ? shown + '+leak' : shown;
                }}
            }}

            const missingAsync = await collectAsyncFailure(
                () => mkdir(missingParent), missingParent);
            const missingSync = collectSyncFailure(
                () => mkdirSync(missingParent), missingParent);
            const nestedAsyncResult = await collectAsyncSuccess(
                () => mkdir(nestedAsync, {{recursive: true}}));
            const nestedSyncResult = collectSyncSuccess(
                () => mkdirSync(nestedSync, {{recursive: true}}));
            const belowAsync = await collectAsyncFailure(
                () => mkdir(belowFile), belowFile);
            const belowSync = collectSyncFailure(
                () => mkdirSync(belowFile), belowFile);
            const belowAsyncRecursive = await collectAsyncFailure(
                () => mkdir(belowFileRecursive, {{recursive: true}}), belowFileRecursive);
            const belowSyncRecursive = collectSyncFailure(
                () => mkdirSync(belowFileRecursive, {{recursive: true}}), belowFileRecursive);

            globalThis.__neoWindowsMkdirCodes =
                'missingAsync=' + missingAsync +
                ' missingSync=' + missingSync +
                ' nestedAsync=' + nestedAsyncResult +
                ' nestedSync=' + nestedSyncResult +
                ' belowAsync=' + belowAsync +
                ' belowSync=' + belowSync +
                ' belowAsyncRecursive=' + belowAsyncRecursive +
                ' belowSyncRecursive=' + belowSyncRecursive;
            "#,
            error_helpers = FS_TEST_ERROR_HELPERS,
            missing_parent = literal(&missing_parent),
            nested_async = literal(&nested_async),
            nested_sync = literal(&nested_sync),
            below_file = literal(&below_file),
            below_file_recursive = literal(&below_file_recursive),
        ),
    );
    // libtest captures ordinary test output, so a passing run would otherwise
    // hide the codes. Write directly to the process stderr with a fixed label;
    // only sanitized literals are included, never a path. The CI guard pins
    // the exact `--test fs` argument shape, so this deliberately avoids adding
    // `--nocapture` to the gate. This write happens before every assertion
    // below.
    let observed = js
        .eval("globalThis.__neoWindowsMkdirCodes")
        .expect("the Windows mkdir diagnostic did not record the observed codes");
    let mut stderr = std::io::stderr();
    use std::io::Write as _;
    writeln!(stderr, "windows mkdir observed codes: {observed}").unwrap();

    fn observed_field<'a>(observed: &'a str, key: &str) -> &'a str {
        observed
            .split(' ')
            .find_map(|field| {
                field
                    .strip_prefix(key)
                    .and_then(|rest| rest.strip_prefix('='))
            })
            .unwrap_or("MISSING")
    }
    let missing_async = observed_field(&observed, "missingAsync");
    let missing_sync = observed_field(&observed, "missingSync");
    let nested_async_result = observed_field(&observed, "nestedAsync");
    let nested_sync_result = observed_field(&observed, "nestedSync");
    let below_async = observed_field(&observed, "belowAsync");
    let below_sync = observed_field(&observed, "belowSync");
    let below_async_recursive = observed_field(&observed, "belowAsyncRecursive");
    let below_sync_recursive = observed_field(&observed, "belowSyncRecursive");

    assert!(
        !missing_async.ends_with("+leak"),
        "windows async missing parent leaked the path: {missing_async}"
    );
    assert!(
        !missing_sync.ends_with("+leak"),
        "windows sync missing parent leaked the path: {missing_sync}"
    );
    assert_eq!(
        missing_async, "FS_NOT_FOUND",
        "windows async missing parent"
    );
    assert_eq!(missing_sync, "FS_NOT_FOUND", "windows sync missing parent");
    assert_eq!(nested_async_result, "OK", "windows async recursive mkdir");
    assert_eq!(nested_sync_result, "OK", "windows sync recursive mkdir");
    for (label, code) in [
        ("windows async mkdir below a file", below_async),
        ("windows sync mkdir below a file", below_sync),
    ] {
        assert!(!code.ends_with("+leak"), "{label} leaked the path: {code}");
        assert!(
            matches!(code, "FS_NOT_FOUND" | "FS_NOT_DIRECTORY"),
            "{label}: {code}"
        );
    }
    for (label, code) in [
        (
            "windows async recursive mkdir below a file",
            below_async_recursive,
        ),
        (
            "windows sync recursive mkdir below a file",
            below_sync_recursive,
        ),
    ] {
        assert!(!code.ends_with("+leak"), "{label} leaked the path: {code}");
        assert!(
            matches!(
                code,
                "FS_NOT_FOUND" | "FS_NOT_DIRECTORY" | "FS_ALREADY_EXISTS"
            ),
            "{label}: {code}"
        );
    }
    assert_eq!(
        below_async, below_sync,
        "windows mkdir below a file differs by form"
    );
    assert_eq!(
        below_async_recursive, below_sync_recursive,
        "windows recursive mkdir below a file differs by form"
    );
    assert!(!missing_parent.exists());
    assert!(!below_file.exists());
    assert!(!below_file_recursive.exists());
    assert!(nested_async.is_dir());
    assert!(nested_sync.is_dir());
}

#[cfg(windows)]
#[test]
fn windows_basic_round_trip_is_gated_and_unverified() {
    // This test is selected by the native Windows gate. It had not passed a
    // hosted Windows run when this branch was written.
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("windows.txt");
    let js = runtime();
    let source = r#"
        import {writeText, readText, statSync, FsError} from 'neo:fs';
        writeText(__FILE__, 'windows');
        if (readText(__FILE__) !== 'windows') throw Error('round trip');
        if (statSync(__FILE__).isFile !== true) throw Error('stat');
    "#
    .replace("__FILE__", &literal(&file));
    run(&js, &source);
}
