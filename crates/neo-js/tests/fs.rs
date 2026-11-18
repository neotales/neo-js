//! Integration tests for `neo:fs` through the public `NeoJs` API.
//!
//! Every fixture is a `tempfile` directory. The tests exercise the permission
//! and allowlist gates, binary and Unicode round trips, symlink semantics,
//! the runtime-local working directory, no-clobber choices, malformed paths,
//! bounded reads and listings, and the stable error shape. Windows-only
//! behavior is gated and is not run by this build.

use std::fs;

use neo_js::{NeoJs, Permissions, RuntimeModule, RuntimeOptions};

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
         if (typeof fs.readText !== 'function') throw Error('missing global readText');
         if (typeof module.writeFile !== 'function') throw Error('missing module writeFile');
         if (module.readFile !== fs.readFile) throw Error('module and global differ');
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
fn text_and_binary_round_trip_with_unicode_names_and_content() {
    let dir = tempfile::tempdir().unwrap();
    let text = dir.path().join("héllo-文件-😀.txt");
    let binary = dir.path().join("bytes.bin");

    let js = runtime();
    let source = r#"
        import {readText, writeText, readFile, writeFile} from 'neo:fs';
        const textPath = __TEXT__;
        const binaryPath = __BINARY__;
        const text = '\uFEFFline 1\r\nhéllo 世界 😀\u0000tail';
        writeText(textPath, text);
        if (readText(textPath) !== text) throw Error('readText text');
        if (fs.readText(textPath) !== text) throw Error('global readText text');

        const bytes = new Uint8Array(256);
        for (let index = 0; index < 256; index++) bytes[index] = index;
        writeFile(binaryPath, bytes);
        const back = readFile(binaryPath);
        if (!(back instanceof Uint8Array)) throw Error('not a Uint8Array');
        if (back.length !== 256) throw Error('wrong length');
        for (let index = 0; index < 256; index++) {
            if (back[index] !== index) throw Error('wrong byte ' + index);
        }

        // A subarray writes only its own view, not the whole backing buffer.
        writeFile(binaryPath, bytes.subarray(10, 13));
        const view = readFile(binaryPath);
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
    let js = runtime();
    let source = r#"
        import {readText, readFile, writeText, writeFile, stat, exists, readDir, mkdir, remove, FsError} from 'neo:fs';

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
        expectFsError('FS_NOT_FOUND', () => readFile(missing));
        expectFsError('FS_NOT_FOUND', () => stat(missing));
        expectFsError('FS_NOT_FOUND', () => remove(missing));

        expectFsError('FS_PATH', () => readText(''));
        expectFsError('FS_PATH', () => readText('a\u0000b'));
        expectFsError('FS_PATH', () => stat(''));
        expectFsError('FS_PATH', () => exists(''));
        expectFsError('FS_PATH', () => readDir(''));
        expectFsError('FS_PATH', () => mkdir(''));
        expectFsError('FS_PATH', () => remove(''));
        expectFsError('FS_LIMIT', () => readText('x'.repeat(fs.MAX_PATH_BYTES + 1)));
        expectFsError('FS_ENCODING', () => readText('bad\uD800'));
        expectFsError('FS_ENCODING', () => writeText(missing, 'bad\uD800'));

        expectTypeError(() => readText(123));
        expectTypeError(() => readFile(null));
        expectTypeError(() => writeFile(missing, 'text'));
        expectTypeError(() => writeFile(missing, new DataView(new ArrayBuffer(2))));
        expectTypeError(() => writeFile(missing, new Uint16Array([1])));
        expectTypeError(() => writeFile(missing, new Uint8ClampedArray([1])));
        expectTypeError(() => writeFile(missing, new Uint8Array(0), null));
        expectTypeError(() => writeFile(missing, new Uint8Array(0), {overwrite: 'yes'}));
        expectTypeError(() => mkdir(missing, null));
        expectTypeError(() => mkdir(missing, {recursive: 'yes'}));
        expectTypeError(() => remove(missing, {recursive: 1}));
        expectTypeError(() => stat({}));
        expectTypeError(() => readDir([]));

        let traps = 0;
        const proxy = new Proxy(new Uint8Array([1]), {
            get() { traps += 1; return undefined; },
            getPrototypeOf() { traps += 1; return Uint8Array.prototype; },
            getOwnPropertyDescriptor() { traps += 1; return undefined; },
        });
        expectTypeError(() => writeFile(missing, proxy));
        if (traps !== 0) throw Error('proxy traps were observed: ' + traps);

        // A caught brand exception must not poison the following
        // host call with a pending QuickJS exception.
        if (readText(__EXISTING__) !== 'still usable') throw Error('pending exception leaked');

        const constructed = new FsError('FS_IO');
        if (!(constructed instanceof Error)) throw Error('FsError is not an Error');
        if (constructed.code !== 'FS_IO') throw Error('FsError code');
        if (!constructed.message.includes('filesystem')) throw Error('FsError message');
    "#
    .replace("__MISSING__", &literal(&dir.path().join("no-such-file")))
    .replace("__EXISTING__", &literal(&dir.path().join("existing.txt")));
    run(&js, &source);
}

#[test]
fn binary_write_snapshots_live_view_bounds_and_rejects_invalid_views_before_options() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("view.bin");
    fs::write(&file, [7, 8, 9]).unwrap();

    let source = r#"
        import {writeFile, readFile, FsError} from 'neo:fs';
        const file = __FILE__;
        let traps = 0;
        const options = new Proxy({}, {
            ownKeys() { traps++; return []; },
        });
        function assertBytes(expected) {
            const actual = readFile(file);
            if (actual.length !== expected.length) throw Error('wrong length: ' + actual);
            for (let i = 0; i < actual.length; i++) {
                if (actual[i] !== expected[i]) throw Error('wrong byte: ' + actual);
            }
        }
        function reject(data, code) {
            let failure;
            try { writeFile(file, data, options); }
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
        reject(fixed, 'FS_OUT_OF_BOUNDS');

        const detachedBuffer = new ArrayBuffer(0);
        const detached = new Uint8Array(detachedBuffer);
        detachedBuffer.transfer();
        reject(detached, 'FS_DETACHED');
        reject(new Uint8Array(new SharedArrayBuffer(4)), 'FS_UNSUPPORTED');

        buffer.resize(5);
        tracking.set([1, 2, 3, 4]);
        for (const key of ['length', 'byteLength', 'byteOffset', 'buffer', 'constructor']) {
            Object.defineProperty(tracking, key, {
                get() { throw Error('input getter ran: ' + key); },
            });
        }
        writeFile(file, tracking, new Proxy({}, {
            ownKeys() {
                buffer.resize(1);
                return [];
            },
        }));
        assertBytes([1, 2, 3, 4]);

        // A length-tracking view at the buffer's end is valid, not OOB.
        writeFile(file, tracking);
        assertBytes([]);
        writeFile(file, new Uint8Array(0));
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
        import {readText, writeText, writeFile, mkdir, remove, exists, FsError} from 'neo:fs';
        function expectFsError(code, action) {
            try { action(); }
            catch (error) {
                if (error.code !== code) throw Error('wrong code: ' + error.code);
                return;
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
        expectFsError('FS_ALREADY_EXISTS', () => writeFile(file, new Uint8Array([50]), {overwrite: false}));
        if (readText(file) !== 'first') throw Error('no-clobber changed the file');

        writeFile(file, new Uint8Array([0x73]));
        if (readText(file) !== 's') throw Error('default overwrite failed');
        writeText(file, 'third');
        if (readText(file) !== 'third') throw Error('writeText overwrite failed');

        writeFile(fresh, new Uint8Array([0x78]), {overwrite: false});
        if (readText(fresh) !== 'x') throw Error('create-new failed');

        expectFsError('FS_IS_DIRECTORY', () => writeFile(dir, new Uint8Array([1])));
        expectFsError('FS_NOT_FOUND', () => writeFile(missing, new Uint8Array([1])));

        mkdir(empty);
        expectFsError('FS_ALREADY_EXISTS', () => mkdir(empty));
        mkdir(empty, {recursive: true});
        mkdir(nested, {recursive: true});
        if (!exists(nested)) throw Error('recursive mkdir failed');
        remove(empty);
        if (exists(empty)) throw Error('remove empty dir failed');

        mkdir(tree);
        writeText(tree + '/inside.txt', 'inside');
        expectFsError('FS_NOT_EMPTY', () => remove(tree));
        remove(tree, {recursive: true});
        if (exists(tree)) throw Error('recursive remove failed');
    "#
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
        import {readFile, readText, writeFile, mkdir, remove, exists} from 'neo:fs';
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
            remove('target', Object.create({recursive: true})));
        writeFile('file', new Uint8Array([111]), Object.create({overwrite: false}));
        if (readText('file') !== 'o') throw Error('inherited overwrite changed mode');

        let getterCalls = 0;
        const accessor = {};
        Object.defineProperty(accessor, 'recursive', {
            __proto__: null,
            enumerable: true,
            get() { getterCalls++; return true; },
        });
        expectTypeError(() => remove('target', accessor));
        if (getterCalls !== 0) throw Error('option getter was invoked');

        const hidden = {};
        Object.defineProperty(hidden, 'recursive', {
            __proto__: null,
            enumerable: false,
            value: true,
        });
        expectTypeError(() => remove('target', hidden));
        expectTypeError(() => mkdir('another', {recursive: true, unexpected: 1}));

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
        remove('target', proxy);
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
        import {readText, readFile, stat, lstat, exists, readDir, remove, FsError} from 'neo:fs';
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

        const followed = stat(link);
        if (!followed.isFile || followed.isDirectory || followed.isSymlink) throw Error('stat did not follow');
        if (followed.size !== '3') throw Error('stat size is not a decimal string');
        if (typeof followed.mtimeMs !== 'string') throw Error('mtime is not a decimal string');

        const linkInfo = lstat(link);
        if (linkInfo.isFile || linkInfo.isDirectory || !linkInfo.isSymlink) throw Error('lstat followed');

        if (exists(link) !== true) throw Error('exists link');
        if (exists(dangling) !== false) throw Error('exists dangling');
        if (lstat(dangling).isSymlink !== true) throw Error('lstat dangling');
        expectFsError('FS_NOT_FOUND', () => stat(dangling));
        if (readFile(link)[0] !== 97) throw Error('read did not follow link');

        const entries = readDir(dir);
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

        remove(link);
        if (exists(link)) throw Error('remove link left the link');
        if (readText(target) !== 'abc') throw Error('remove link followed to target');

        // Recursive deletion must not follow a symlink inside the tree.
        remove(tree, {recursive: true});
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
        import {readText, remove, exists, FsError} from 'neo:fs';
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
            expectPathError(() => remove(link + suffix, {recursive: true}));
        }
        // Even a regressed guard must not attempt filesystem I/O at root.
        expectPathError(() => remove('/', new Proxy({}, {
            ownKeys() { throw Error('root path reached option reflection'); },
        })));
        if (!exists(link)) throw Error('link was deleted');
        if (readText(targetFile) !== 'keep') throw Error('target was touched');

        // Bare directory symlink removal deletes the link itself, not target.
        remove(link, {recursive: true});
        if (exists(link)) throw Error('bare symlink was retained');
        if (readText(targetFile) !== 'keep') throw Error('bare link removal followed target');
    "#
    .replace("__LINK__", &literal(&link))
    .replace("__TARGET_FILE__", &literal(&target.join("important.txt")));
    run(&js, &source);
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
    let root = dir.path().ancestors().last().unwrap();
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
            ['FS_PATH', ''], ['FS_PATH', 'bad\0path'], ['FS_PATH', __ROOT__],
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
    .replace("__DIR__", &literal(dir.path()))
    .replace("__ROOT__", &literal(root));
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

#[cfg(windows)]
#[test]
fn copy_file_rejects_windows_namespaces_and_streams_before_opening() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("source"), b"unchanged").unwrap();
    let script = r#"
        import {chdir} from 'neo:process';
        import {copyFile, FsError} from 'neo:fs';
        chdir(__DIR__);
        for (const path of [
            'C:relative', '\\rooted', '\\\\?\\C:\\', '\\\\server\\share', '\\\\server\\share\\file',
            '\\\\.\\pipe\\neo-fs-test', '\\\\?\\GLOBALROOT\\Device\\Null',
            'NUL', 'con.txt', 'COM¹.bin', 'source:stream', 'source.', 'source ',
        ]) {
            for (const [from, to] of [[path, 'output'], ['source', path]]) {
                try { copyFile(from, to); }
                catch (error) {
                    if (!(error instanceof FsError) || error.code !== 'FS_PATH') throw error;
                    continue;
                }
                throw Error('copy accepted ambiguous Windows path');
            }
        }
    "#
    .replace("__DIR__", &literal(dir.path()));
    run(&chunked_runtime(), &script);
    assert_eq!(fs::read(dir.path().join("source")).unwrap(), b"unchanged");
    assert!(!dir.path().join("output").exists());
}

#[test]
fn reads_are_bounded_before_allocation_and_sizes_stay_decimal() {
    let dir = tempfile::tempdir().unwrap();
    let big = dir.path().join("big.bin");

    let js = runtime();
    let max_read: u64 = js
        .eval("String(fs.MAX_READ_BYTES)")
        .unwrap()
        .parse()
        .unwrap();
    let handle = fs::File::create(&big).unwrap();
    handle.set_len(max_read + 1).unwrap();
    drop(handle);

    let source = r#"
        import {readText, readFile, writeFile, stat, exists, FsError} from 'neo:fs';
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
        expectFsError('FS_LIMIT', () => readFile(big));
        if (stat(big).size !== String(fs.MAX_READ_BYTES + 1)) throw Error('size rounding');

        // Check the limit before resolving or opening: the output must not exist.
        const tooLarge = new Uint8Array(fs.MAX_WRITE_BYTES + 1);
        expectFsError('FS_LIMIT', () => writeFile(output, tooLarge));
        if (exists(output)) throw Error('oversize write created a file');

        expectFsError('FS_LIMIT', () => readText('x'.repeat(fs.MAX_PATH_BYTES + 1)));
    "#
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

    let js = runtime();
    let source = r#"
        import {writeText, readText, stat, exists, FsError} from 'neo:fs';
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
        if (stat(exact).size !== String(size)) throw Error('exact UTF-8 byte boundary');
        if (readText(exact) !== atLimit) throw Error('exact boundary roundtrip');

        const aboveLimit = 'a'.repeat(size - 3) + '😀';
        expectFsError('FS_LIMIT', () => writeText(tooLarge, aboveLimit));
        if (exists(tooLarge)) throw Error('max+1 UTF-8 text created a file');
        expectFsError('FS_ENCODING', () => readText(invalid));

        // UTF-16 length stays below the cap, but encoded bytes exceed it.
        expectFsError('FS_LIMIT', () =>
            stat('😀'.repeat(fs.MAX_PATH_BYTES / 4) + 'a'));
    "#
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
        import {readText, readFile, stat, readDir, FsError} from 'neo:fs';
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
            if (readFile(__FILE__).length !== 7) throw new oldError('readFile broke');
            if (stat(__FILE__).size !== '7') throw new oldError('stat broke');
            const oldArrayProto = Array.prototype;
            Object.defineProperty(oldArrayProto, '0', {
                __proto__: null,
                set() { throw new oldError('Array.prototype index setter ran'); },
                configurable: true,
            });
            try {
                const entries = readDir(__DIR__);
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
        import {readDir, FsError} from 'neo:fs';
        let failure;
        try { readDir(__DIR__); }
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
fn windows_basic_round_trip_is_gated_and_unverified() {
    // This test is compiled on Windows but has not been run by this build.
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("windows.txt");
    let js = runtime();
    let source = r#"
        import {writeText, readText, stat, FsError} from 'neo:fs';
        writeText(__FILE__, 'windows');
        if (readText(__FILE__) !== 'windows') throw Error('round trip');
        if (stat(__FILE__).isFile !== true) throw Error('stat');
    "#
    .replace("__FILE__", &literal(&file));
    run(&js, &source);
}
