//! End-to-end tests for `neo:path` through the CLI.
//!
//! Exercise both explicit grammars on any host, then check that CLI named
//! exports select the native dialect and relative resolution uses CLI grants.

use std::{
    path::Path,
    process::{Command, Output},
};

fn run_cli(dir: &Path, permissions: &[&str], name: &str, source: &str) -> Output {
    let entry = dir.join(name);
    std::fs::write(&entry, source).unwrap();
    Command::new(env!("CARGO_BIN_EXE_neo-js"))
        .current_dir(dir)
        .args(permissions)
        .arg(&entry)
        .output()
        .unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn assert_ok(output: &Output) {
    assert!(
        output.status.success(),
        "status: {:?}\nstderr: {}",
        output.status,
        stderr(output)
    );
    assert!(output.stderr.is_empty(), "{}", stderr(output));
}

/// POSIX and Windows matrices, argument and NUL errors, aliasing, idempotence,
/// and round-trip expectations. Expected strings are the module's documented
/// output, not a recording of the implementation.
const PATH_MATRIX: &str = r##"import { deepEqual, equal, throws } from 'neo:assert';
import { posix, win32 } from 'neo:path';

function check(label, actual, expected) {
  equal(actual, expected, label);
}
function checkEqual(label, actual, expected) {
  deepEqual(actual, expected, label);
}

const posixNormalize = [
  ['', '.'], ['.', '.'], ['..', '..'], ['/', '/'], ['//', '/'], ['///', '/'],
  ['////a//b///', '/a/b/'], ['/a//b/./c/..', '/a/b'], ['a/../../b', '../b'],
  ['a/..', '.'], ['a/../', './'], ['./', './'], ['../', '../'],
  ['/../../a', '/a'], ['/a/..', '/'], ['a//b///', 'a/b/'],
  ['a\\b', 'a\\b'], ['a\\..\\b', 'a\\..\\b'],
  ['日本語//ファイル/../写真.png', '日本語/写真.png'],
  ['😀/./oi', '😀/oi'], ['\ud800/a', '\ud800/a'],
];
for (const [input, expected] of posixNormalize)
  check('posix.normalize ' + JSON.stringify(input), posix.normalize(input), expected);

const posixJoin = [
  [[], '.'], [[''], '.'], [['', ''], '.'], [['.'], '.'],
  [['a'], 'a'], [['/', 'a'], '/a'], [['a', '/b'], 'a/b'],
  [['a', '..', 'b'], 'b'], [['/a', '..', '..'], '/'],
  [['//foo', 'bar'], '/foo/bar'], [['a/', '/b'], 'a/b'], [['a', ''], 'a'],
  [['日本語', '写真.png'], '日本語/写真.png'], [['a//', '//b'], 'a/b'],
];
for (const [input, expected] of posixJoin)
  check('posix.join ' + JSON.stringify(input), posix.join(...input), expected);

const posixBasename = [
  ['', ''], ['/', ''], ['//', ''], ['///', ''], ['a', 'a'], ['a/', 'a'],
  ['/a', 'a'], ['/a/b.txt', 'b.txt'], ['a//b', 'b'], ['.bashrc', '.bashrc'],
  ['.', '.'], ['..', '..'], ['a.', 'a.'], ['日本語.txt', '日本語.txt'],
  ['😀', '😀'], ['\ud800', '\ud800'],
];
for (const [input, expected] of posixBasename)
  check('posix.basename ' + JSON.stringify(input), posix.basename(input), expected);

const posixSuffix = [
  [['/a/b.txt', '.txt'], 'b'], [['a.txt', 'a.txt'], 'a.txt'],
  [['a.txt', '.md'], 'a.txt'], [['a.txt', ''], 'a.txt'],
  [['.bashrc', '.bashrc'], '.bashrc'], [['a', 'a'], 'a'],
  [['a.txt', 'txt'], 'a.'], [['日本語.txt', '.txt'], '日本語'],
  [['a/', 'a'], 'a'], [['a', 'b'], 'a'],
];
for (const [input, expected] of posixSuffix)
  check('posix.basename ' + JSON.stringify(input), posix.basename(...input), expected);

const posixDirname = [
  ['', '.'], ['.', '.'], ['..', '.'], ['a', '.'], ['a/b', 'a'],
  ['a//b', 'a'], ['a/b/', 'a'], ['/a', '/'], ['/', '/'], ['//', '/'],
  ['///foo', '/'], ['//a', '/'], ['/a/b', '/a'], ['a/./b', 'a/.'],
  ['日本語/ファイル.txt', '日本語'], ['a\\b', '.'], ['\ud800/x', '\ud800'],
];
for (const [input, expected] of posixDirname)
  check('posix.dirname ' + JSON.stringify(input), posix.dirname(input), expected);

const posixExtname = [
  ['', ''], ['/', ''], ['a', ''], ['.bashrc', ''], ['a.', '.'], ['a..', '.'],
  ['a.tar.gz', '.gz'], ['a/b.txt', '.txt'], ['a/b/', ''],
  ['a/b.txt/', '.txt'], ['/a/b.txt//', '.txt'],
  ['日本語.ファイル', '.ファイル'], ['..', ''], ['.', ''], ['a/..', ''],
  ['a\\b.txt', '.txt'],
];
for (const [input, expected] of posixExtname)
  check('posix.extname ' + JSON.stringify(input), posix.extname(input), expected);

const posixAbsolute = [
  ['', false], ['/', true], ['//a', true], ['a', false], ['\\a', false],
  [' /a', false], ['C:\\a', false],
];
for (const [input, expected] of posixAbsolute)
  check('posix.isAbsolute ' + JSON.stringify(input), posix.isAbsolute(input), expected);

const posixParse = [
  ['', { root: '', dir: '', base: '', ext: '', name: '' }],
  ['/', { root: '/', dir: '/', base: '', ext: '', name: '' }],
  ['//', { root: '/', dir: '/', base: '', ext: '', name: '' }],
  ['///foo', { root: '/', dir: '/', base: 'foo', ext: '', name: 'foo' }],
  ['/a/b.txt', { root: '/', dir: '/a', base: 'b.txt', ext: '.txt', name: 'b' }],
  ['a//b', { root: '', dir: 'a', base: 'b', ext: '', name: 'b' }],
  ['a/b/', { root: '', dir: 'a', base: 'b', ext: '', name: 'b' }],
  ['.bashrc', { root: '', dir: '', base: '.bashrc', ext: '', name: '.bashrc' }],
  ['a.tar.gz', { root: '', dir: '', base: 'a.tar.gz', ext: '.gz', name: 'a.tar' }],
  ['../..', { root: '', dir: '..', base: '..', ext: '', name: '..' }],
  ['/a//b/', { root: '/', dir: '/a', base: 'b', ext: '', name: 'b' }],
  ['a/../..', { root: '', dir: 'a/..', base: '..', ext: '', name: '..' }],
  ['a/./b', { root: '', dir: 'a/.', base: 'b', ext: '', name: 'b' }],
  ['/a/./b', { root: '/', dir: '/a/.', base: 'b', ext: '', name: 'b' }],
  ['a\\b.txt', { root: '', dir: '', base: 'a\\b.txt', ext: '.txt', name: 'a\\b' }],
  ['\ud800/x.txt', { root: '', dir: '\ud800', base: 'x.txt', ext: '.txt', name: 'x' }],
];
for (const [input, expected] of posixParse)
  checkEqual('posix.parse ' + JSON.stringify(input), posix.parse(input), expected);

const posixFormat = [
  [{}, ''], [{ base: 'a' }, 'a'], [{ name: 'a', ext: '.txt' }, 'a.txt'],
  [{ base: 'a', name: 'b', ext: '.c' }, 'a'], [{ root: '/' }, '/'],
  [{ root: '/', base: 'a' }, '/a'], [{ root: '/', dir: '/a', base: 'b' }, '/a/b'],
  [{ dir: 'a', base: 'b' }, 'a/b'], [{ base: '/' }, '/'],
  [{ root: '/', base: '/' }, '//'], [{ dir: '/a', base: '/' }, '/a//'],
  [{ dir: 'foo', base: '/' }, 'foo//'],
  [{ root: '/', dir: '/', base: '/' }, '//'],
  [{ dir: 'a' }, 'a/'], [{ root: '/', dir: '/' }, '/'],
  [{ root: '/', dir: '/a', name: 'b', ext: '.txt' }, '/a/b.txt'],
];
for (const [input, expected] of posixFormat)
  check('posix.format ' + JSON.stringify(input), posix.format(input), expected);

const winNormalize = [
  ['', '.'], ['.', '.'], ['..', '..'], ['\\', '\\'], ['/', '\\'],
  ['///', '\\'], ['///foo', '\\foo'],
  ['\\\\server\\share', '\\\\server\\share\\'],
  ['\\\\server\\share\\', '\\\\server\\share\\'],
  ['//server/share', '\\\\server\\share\\'],
  ['\\\\server\\share\\..\\foo', '\\\\server\\share\\foo'],
  ['\\\\server', '\\server'], ['\\\\\\foo', '\\foo'],
  ['\\\\\\\\server\\share', '\\server\\share'],
  ['C:', 'C:.'], ['C:\\', 'C:\\'], ['C:/', 'C:\\'], ['C:foo', 'C:foo'],
  ['C:\\foo\\..\\bar', 'C:\\bar'], ['C:/foo\\bar/../baz', 'C:\\foo\\baz'],
  ['C:..\\foo', 'C:..\\foo'], ['C:..\\..', 'C:..\\..'],
  ['1:\\foo', '1:\\foo'], ['a:b\\c', 'a:b\\c'],
  ['c:/FOO/Bar.TXT', 'c:\\FOO\\Bar.TXT'],
  ['\\\\?\\C:\\foo', '\\\\?\\C:\\foo'],
  ['\\\\?\\UNC\\server\\share\\foo', '\\\\?\\UNC\\server\\share\\foo'],
  ['\\\\.\\C:\\foo', '\\\\.\\C:\\foo'],
  ['C:\\a\\..\\..', 'C:\\'], ['a\\\\b', 'a\\b'], ['a//b', 'a\\b'],
  ['日本語\\ファイル\\..\\写真.png', '日本語\\写真.png'],
  ['C:\\日本語\\..\\é', 'C:\\é'],
  ['\\\\server\\share\\..\\..\\x', '\\\\server\\share\\x'],
  ['\\..\\foo', '\\foo'], ['a/../../b', '..\\b'],
  ['C:\\foo\\..', 'C:\\'], ['C:\\foo\\..\\', 'C:\\'],
  ['\ud800\\x', '\ud800\\x'],
];
for (const [input, expected] of winNormalize)
  check('win32.normalize ' + JSON.stringify(input), win32.normalize(input), expected);

const winJoin = [
  [[], '.'], [[''], '.'], [['C:', 'foo'], 'C:\\foo'],
  [['C:\\a', '..', 'b'], 'C:\\b'], [['//server', 'share'], '\\\\server\\share\\'],
  [['//server', 'share', 'dir'], '\\\\server\\share\\dir'],
  [['/', 'foo'], '\\foo'], [['a', '/b'], 'a\\b'], [['a', '..', '..'], '..'],
  [['C:\\a', 'D:\\b'], 'C:\\a\\D:\\b'],
  [['\\\\server\\share', '..', 'x'], '\\\\server\\share\\x'],
  [['C:\\', '..'], 'C:\\'], [['a\\\\', 'b'], 'a\\b'],
  [['日本語', 'ファイル.txt'], '日本語\\ファイル.txt'],
  [['C:', '..'], 'C:\\'], [['C:foo', '..'], 'C:.'],
  [['\\\\server\\share\\', '..'], '\\\\server\\share\\'],
];
for (const [input, expected] of winJoin)
  check('win32.join ' + JSON.stringify(input), win32.join(...input), expected);

const winBasename = [
  ['', ''], ['\\', ''], ['/', ''], ['///', ''], ['C:\\', ''], ['C:', ''],
  ['C:foo', 'foo'], ['C:\\foo\\', 'foo'], ['C:\\foo\\bar.txt', 'bar.txt'],
  ['C:/foo/bar', 'bar'], ['\\\\server\\share', ''],
  ['\\\\server\\share\\', ''], ['\\\\server\\share.txt', ''],
  ['\\\\server\\share.txt\\', ''], ['\\\\server\\share.txt\\\\', ''],
  ['\\\\server\\share.txt\\dir', 'dir'],
  ['\\\\server\\share.txt\\file.txt', 'file.txt'],
  ['\\\\server\\share.txt\\file.txt\\', 'file.txt'],
  ['//server/share.txt', ''], ['//server/share.txt/', ''],
  ['\\\\?\\C:\\', ''], ['\\\\?\\UNC\\', ''],
  ['\\\\server\\share\\file.txt', 'file.txt'],
  ['.bashrc', '.bashrc'], ['日本語.txt', '日本語.txt'], ['a//b', 'b'],
  ['C:..\\foo', 'foo'],
];
for (const [input, expected] of winBasename)
  check('win32.basename ' + JSON.stringify(input), win32.basename(input), expected);

const winSuffix = [
  [['C:\\foo\\bar.txt', '.txt'], 'bar'], [['C:\\a', 'a'], 'a'],
  [['C:foo', 'foo'], 'foo'], [['a.txt', '.md'], 'a.txt'],
  [['a.tar.gz', '.gz'], 'a.tar'], [['日本語.txt', '.txt'], '日本語'],
  [['\\\\server\\share.txt', '.txt'], ''],
  [['\\\\server\\share.txt\\', '.txt'], ''],
];
for (const [input, expected] of winSuffix)
  check('win32.basename ' + JSON.stringify(input), win32.basename(...input), expected);

const winDirname = [
  ['', '.'], ['foo', '.'], ['C:', 'C:'], ['C:foo', 'C:'], ['C:\\', 'C:\\'],
  ['C:/', 'C:/'], ['C:\\foo', 'C:\\'], ['C:/foo', 'C:/'],
  ['C:\\foo\\bar', 'C:\\foo'], ['C:/foo/bar', 'C:/foo'],
  ['C:\\foo\\bar\\', 'C:\\foo'], ['\\foo', '\\'], ['/foo', '/'], ['\\', '\\'],
  ['\\\\server\\share', '\\\\server\\share'],
  ['\\\\server\\share\\', '\\\\server\\share\\'],
  ['\\\\server\\share\\foo', '\\\\server\\share\\'],
  ['\\\\server\\share\\dir\\foo', '\\\\server\\share\\dir'],
  ['\\\\server\\share\\dir\\\\foo', '\\\\server\\share\\dir'],
  ['a//b', 'a'], ['a\\\\b', 'a'], ['1:\\foo', '1:'], ['C:foo\\bar', 'C:foo'],
  ['C:\\\\foo', 'C:\\'], ['\\\\?\\C:\\foo', '\\\\?\\C:\\'],
];
for (const [input, expected] of winDirname)
  check('win32.dirname ' + JSON.stringify(input), win32.dirname(input), expected);

const winExtname = [
  ['', ''], ['\\', ''], ['/', ''], ['///', ''], ['C:', ''], ['C:\\', ''],
  ['C:\\foo', ''], ['C:\\foo\\bar.txt', '.txt'], ['C:/foo/bar.TXT', '.TXT'],
  ['C:\\foo\\bar.txt\\', '.txt'],
  ['C:\\foo\\.bashrc', ''], ['C:\\foo\\a..', '.'], ['C:\\foo\\a.', '.'],
  ['\\\\server\\share', ''], ['\\\\server\\share\\', ''],
  ['\\\\server\\share.txt', ''], ['\\\\server\\share.txt\\', ''],
  ['\\\\server\\share.txt\\\\', ''],
  ['\\\\server\\share.txt\\dir', ''],
  ['\\\\server\\share.txt\\file.txt', '.txt'],
  ['\\\\server\\share.txt\\file.txt\\', '.txt'],
  ['//server/share.txt', ''], ['//server/share.txt/', ''],
  ['\\\\?\\C:\\', ''], ['\\\\?\\UNC\\', ''],
  ['\\\\server\\share\\file.tar.gz', '.gz'], ['日本語.ファイル', '.ファイル'],
  ['C:.txt', ''], ['a\\b.txt', '.txt'],
];
for (const [input, expected] of winExtname)
  check('win32.extname ' + JSON.stringify(input), win32.extname(input), expected);

const winAbsolute = [
  ['', false], ['\\', true], ['/', true], ['C:', false], ['C:foo', false],
  ['C:\\foo', true], ['C:/foo', true], ['c:\\', true], ['1:\\foo', false],
  ['\\\\server', true], [' foo', false], ['C|\\foo', false], ['C', false],
  ['C:..', false],
];
for (const [input, expected] of winAbsolute)
  check('win32.isAbsolute ' + JSON.stringify(input), win32.isAbsolute(input), expected);

const winParse = [
  ['', { root: '', dir: '', base: '', ext: '', name: '' }],
  ['\\', { root: '\\', dir: '\\', base: '', ext: '', name: '' }],
  ['/', { root: '/', dir: '/', base: '', ext: '', name: '' }],
  ['C:', { root: 'C:', dir: 'C:', base: '', ext: '', name: '' }],
  ['C:\\', { root: 'C:\\', dir: 'C:\\', base: '', ext: '', name: '' }],
  ['C:/', { root: 'C:/', dir: 'C:/', base: '', ext: '', name: '' }],
  ['C:foo', { root: 'C:', dir: 'C:', base: 'foo', ext: '', name: 'foo' }],
  ['C:\\foo\\bar.txt', { root: 'C:\\', dir: 'C:\\foo', base: 'bar.txt', ext: '.txt', name: 'bar' }],
  ['\\\\server\\share', { root: '\\\\server\\share', dir: '\\\\server\\share', base: '', ext: '', name: '' }],
  ['\\\\server\\share\\', { root: '\\\\server\\share\\', dir: '\\\\server\\share\\', base: '', ext: '', name: '' }],
  ['\\\\server\\share.txt', { root: '\\\\server\\share.txt', dir: '\\\\server\\share.txt', base: '', ext: '', name: '' }],
  ['\\\\server\\share.txt\\', { root: '\\\\server\\share.txt\\', dir: '\\\\server\\share.txt\\', base: '', ext: '', name: '' }],
  ['\\\\server\\share.txt\\file.txt', { root: '\\\\server\\share.txt\\', dir: '\\\\server\\share.txt\\', base: 'file.txt', ext: '.txt', name: 'file' }],
  ['\\\\server\\share.txt\\file.txt\\', { root: '\\\\server\\share.txt\\', dir: '\\\\server\\share.txt\\', base: 'file.txt', ext: '.txt', name: 'file' }],
  ['//server/share.txt/', { root: '//server/share.txt/', dir: '//server/share.txt/', base: '', ext: '', name: '' }],
  ['\\\\server\\share\\foo', { root: '\\\\server\\share\\', dir: '\\\\server\\share\\', base: 'foo', ext: '', name: 'foo' }],
  ['\\\\?\\C:\\foo', { root: '\\\\?\\C:\\', dir: '\\\\?\\C:\\', base: 'foo', ext: '', name: 'foo' }],
  ['\\\\?\\UNC\\', { root: '\\\\?\\UNC\\', dir: '\\\\?\\UNC\\', base: '', ext: '', name: '' }],
  ['\\\\?\\UNC\\server\\share\\foo', { root: '\\\\?\\UNC\\', dir: '\\\\?\\UNC\\server\\share', base: 'foo', ext: '', name: 'foo' }],
  ['\\\\\\foo', { root: '\\', dir: '\\', base: 'foo', ext: '', name: 'foo' }],
  ['1:\\foo', { root: '', dir: '1:', base: 'foo', ext: '', name: 'foo' }],
  ['a//b', { root: '', dir: 'a', base: 'b', ext: '', name: 'b' }],
  ['a\\\\b', { root: '', dir: 'a', base: 'b', ext: '', name: 'b' }],
  ['C:\\foo\\\\bar\\\\', { root: 'C:\\', dir: 'C:\\foo', base: 'bar', ext: '', name: 'bar' }],
  ['C:..\\foo', { root: 'C:', dir: 'C:..', base: 'foo', ext: '', name: 'foo' }],
  ['C:\\foo\\..\\', { root: 'C:\\', dir: 'C:\\foo', base: '..', ext: '', name: '..' }],
  ['\\\\server\\share\\..\\foo', { root: '\\\\server\\share\\', dir: '\\\\server\\share\\..', base: 'foo', ext: '', name: 'foo' }],
];
for (const [input, expected] of winParse)
  checkEqual('win32.parse ' + JSON.stringify(input), win32.parse(input), expected);

const winFormat = [
  [{}, ''], [{ base: 'a' }, 'a'], [{ name: 'a', ext: '.txt' }, 'a.txt'],
  [{ base: 'a', name: 'b', ext: '.c' }, 'a'], [{ root: '\\' }, '\\'],
  [{ root: '\\', base: 'a' }, '\\a'], [{ root: 'C:\\', base: 'a' }, 'C:\\a'],
  [{ root: 'C:', base: 'a' }, 'C:a'], [{ dir: 'C:\\a', base: 'b' }, 'C:\\a\\b'],
  [{ root: '\\\\server\\share\\', base: 'file' }, '\\\\server\\share\\file'],
  [{ dir: 'a' }, 'a\\'], [{ base: '\\' }, '\\'],
  [{ root: 'C:\\', base: '\\' }, 'C:\\\\'],
  [{ dir: 'C:\\a', base: '\\' }, 'C:\\a\\\\'],
  [{ dir: 'foo', base: '\\' }, 'foo\\\\'],
  [{ root: 'C:\\', dir: 'C:\\', base: '\\' }, 'C:\\\\'],
  [{ root: 'C:\\', dir: 'C:\\a', name: 'b', ext: '.txt' }, 'C:\\a\\b.txt'],
];
for (const [input, expected] of winFormat)
  check('win32.format ' + JSON.stringify(input), win32.format(input), expected);

// Aliasing is lexical only. Distinct spellings may name the same object; this
// module never resolves them. Case and neighbouring separators are preserved
// or normalized as documented instead.
check('alias posix dot', posix.normalize('a/./b'), posix.normalize('a/b'));
check('alias posix link', posix.normalize('link/../x'), 'x');
check('alias win link', win32.normalize('C:\\link\\..\\x'), 'C:\\x');
check('alias separators', win32.normalize('C:/foo/bar'), win32.normalize('C:\\foo\\bar'));
check('case preserved', win32.normalize('c:/FOO'), 'c:\\FOO');
check('posix backslash', posix.normalize('C:\\foo'), 'C:\\foo');
check('trailing differs', posix.normalize('/a/'), '/a/');
check('trailing dirname same', posix.dirname('/a/'), '/');

// Documented Windows corner: normalization is not always a fixed point when a
// leading dot segment is removed to expose a bare drive-relative name.
check('win32 bare drive from relative', win32.normalize('./a:'), 'a:');
check('win32 bare drive again', win32.normalize('a:'), 'a:.');
check('win32 bare drive fixed', win32.normalize('a:.'), 'a:.');

for (const [input] of posixNormalize)
  check('posix idempotent ' + JSON.stringify(input),
    posix.normalize(posix.normalize(input)), posix.normalize(input));
for (const [input] of winNormalize)
  check('win32 idempotent ' + JSON.stringify(input),
    win32.normalize(win32.normalize(input)), win32.normalize(input));

check('roundtrip posix', posix.format(posix.parse('/a/b.txt')), '/a/b.txt');
check('roundtrip posix root', posix.format(posix.parse('/')), '/');
check('roundtrip win drive', win32.format(win32.parse('C:\\')), 'C:\\');
check('roundtrip win unc', win32.format(win32.parse('\\\\server\\share')), '\\\\server\\share');

// Namespaces are frozen and their parse results are independent objects.
equal(posix.sep, '/');
equal(win32.sep, '\\');
equal(posix.delimiter, ':');
equal(win32.delimiter, ';');
equal(Object.isFrozen(posix), true);
equal(Object.isFrozen(win32), true);
equal(posix === win32, false);
throws(() => { posix.sep = '!'; }, TypeError);
throws(() => { delete win32.join; }, TypeError);
const firstParse = posix.parse('/a/b.txt');
const secondParse = posix.parse('/a/b.txt');
equal(firstParse === secondParse, false);
firstParse.base = 'changed';
equal(secondParse.base, 'b.txt');

// Argument and NUL errors: every function validates, nothing is coerced, and
// the message never repeats the rejected path.
for (const api of [posix, win32]) {
  for (const value of [
    undefined, null, 1, true, {}, [], new String('a'), Symbol('a'),
  ]) {
    throws(() => api.normalize(value), TypeError);
    throws(() => api.join('a', value), TypeError);
    throws(() => api.join('', value), TypeError);
  }
  for (const name of [
    'normalize', 'basename', 'dirname', 'extname', 'isAbsolute', 'parse',
  ]) {
    throws(() => api[name]('x\u0000y'), TypeError);
  }
  throws(() => api.basename('/a', 1), TypeError);
  throws(() => api.basename('/a', null), TypeError);
  throws(() => api.format(null), TypeError);
  throws(() => api.format([]), TypeError);
  throws(() => api.format({ dir: 1 }), TypeError);
  throws(() => api.format({ root: null }), TypeError);
  for (const field of ['root', 'dir', 'base', 'ext', 'name']) {
    throws(() => api.format({ [field]: 'x\u0000' }), TypeError);
  }
  throws(() => api.normalize('sentinel\u0000value'), TypeError);
  throws(() => api.basename('/a', 'x\u0000'), TypeError);
}
const untouched = { dir: 'a', base: 'b' };
equal(posix.format(untouched), 'a/b');
equal(untouched.dir, 'a');
equal(untouched.base, 'b');
try {
  posix.normalize('sentinel\u0000value');
  throw Error('expected a throw');
} catch (error) {
  equal(error.name, 'TypeError');
  equal(error.message.includes('sentinel'), false);
  equal(error.message.includes('NUL'), true);
}

// The namespaces are independent of the host platform.
equal(posix.isAbsolute('/a'), true);
equal(posix.isAbsolute('C:\\a'), false);
equal(win32.isAbsolute('C:\\a'), true);
equal(win32.isAbsolute('/a'), true);
equal(posix.normalize('C:\\a\\..\\b'), 'C:\\a\\..\\b');
equal(win32.normalize('C:\\a\\..\\b'), 'C:\\b');
equal(typeof globalThis.posix, 'undefined');
equal(typeof globalThis.win32, 'undefined');
equal(typeof globalThis.path, 'undefined');

console.log('ok');
"##;

#[test]
fn path_matrix_without_permissions() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(dir.path(), &[], "main.js", PATH_MATRIX);
    assert_ok(&output);
    assert_eq!(output.stdout, b"ok\n");
}

#[test]
fn path_matrix_is_identical_with_every_permission_enabled() {
    let dir = tempfile::tempdir().unwrap();
    let disabled = run_cli(dir.path(), &[], "main.js", PATH_MATRIX);
    assert_ok(&disabled);

    let enabled = run_cli(
        dir.path(),
        &[
            "--allow-fs",
            "--allow-env",
            "--allow-stdio",
            "--allow-run",
            "--allow-registry",
            "--allow-net",
        ],
        "main-permitted.js",
        PATH_MATRIX,
    );
    assert_ok(&enabled);

    assert_eq!(disabled.stdout, enabled.stdout);
}

#[test]
fn lexical_operations_use_reference_examples_and_keep_windows_roots_intact() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        "resolution.js",
        r##"
import { equal, throws } from 'neo:assert';
import * as path from 'neo:path';
const { posix, win32 } = path;
const w = String.raw;

equal(posix.common([]), '');
equal(posix.common(['only/file']), 'only/file');
equal(posix.common(['./deno/std/path/mod.ts', './deno/std/fs/mod.ts']), './deno/std/');
equal(posix.common(['/foo', '/foo/bar']), '/foo');
equal(posix.common(['/foo', '/bar']), '/');
equal(posix.common(['foo', 'foobar']), '');
equal(posix.common(['日本//😀/猫', '日本//😀/犬']), '日本//😀/');
equal(win32.common([w`C:\foo\bar`, w`C:\foo\baz`]), w`C:\foo` + '\\');
equal(win32.common([w`C:\FOO\bar`, w`C:\foo\bar`]), 'C:\\');
equal(win32.common(['C:/foo/bar', 'C:/foo/baz']), '');

equal(posix.resolve('/foo', 'bar', 'baz/asdf', 'quux', '..'), '/foo/bar/baz/asdf');
equal(posix.resolve('ignored', '/root', '/日本/😀//', '猫', '..', ''), '/日本/😀');
equal(posix.resolve('//foo', '../../..'), '/');
equal(posix.resolve('/\ud800', '😀'), '/\ud800/😀');
equal(posix.relative('/data/orandea/test/aaa', '/data/orandea/impl/bbb'), '../../impl/bbb');
equal(posix.relative('/prefix/fox', '/prefix/foobar'), '../foobar');
equal(posix.relative('/prefix/a', '/prefix/a/../a/'), '');
equal(posix.relative('/', '/日本/😀'), '日本/😀');
equal(posix.relative('/日本/😀', '/'), '../..');
equal(posix.toNamespacedPath('a/../b'), 'a/../b');

equal(win32.resolve(w`C:\foo\bar`, w`..\baz`), w`C:\foo\baz`);
equal(win32.resolve('ignored', 'C:/foo', '/root', 'descendant/'), w`C:\root\descendant`);
equal(win32.resolve(w`C:\base`, w`c:子\😀`), w`c:\base\子\😀`);
equal(win32.resolve('C:/base', 'D:/ignored', 'c:file'), w`c:\base\file`);
equal(win32.resolve('C:', '\\root', 'child'), w`C:\root\child`);
equal(win32.resolve('//server/share', '/path', '../../../leaf'), w`\\server\share\leaf`);
equal(win32.resolve('//server/share/../..'), w`\\server\share` + '\\');
equal(win32.resolve(w`\\?\UNC\SeRVer\Share\..\..\日本`), w`\\?\UNC\SeRVer\Share\日本`);
equal(win32.relative(w`C:\foobar\test\aaa`, w`C:\foobar\impl\bbb`), w`..\..\impl\bbb`);
equal(win32.relative(w`C:\İ\one`, 'c:\\i\u0307\\MiXeD'), w`..\MiXeD`);
equal(win32.relative('C:\\', w`c:\日本\😀`), w`日本\😀`);
equal(win32.relative(w`C:\Case\file`, w`c:\case\FILE`), '');
equal(win32.relative(w`C:\base`, w`C:\base\D:leaf`), w`.\D:leaf`);
equal(win32.relative(w`C:\base`, w`D:\target`), w`D:\target`);
equal(win32.relative(w`\\host\one\dir`, w`\\host\two\dir`), w`\\host\two\dir`);
equal(win32.relative(w`\\host\share\dir`, w`\\HOST\SHARE`), '..');
equal(win32.relative(w`\\?\UNC\host\one\dir`, w`\\?\UNC\host\two\dir`), w`\\?\UNC\host\two\dir`);
equal(win32.toNamespacedPath(''), '');
equal(win32.toNamespacedPath('C:/foo/../bar'), w`\\?\C:\bar`);
equal(win32.toNamespacedPath('//host/share/../日本'), w`\\?\UNC\host\share\日本`);
equal(win32.toNamespacedPath(w`\\?\C:\foo\..\bar`), w`\\?\C:\foo\..\bar`);
equal(win32.toNamespacedPath(w`\\.\pipe\name`), w`\\.\pipe\name`);
try {
  win32.resolve(w`\\?\UNC\server`);
  throw Error('incomplete namespace root accepted');
} catch (error) {
  equal(error.code, 'PATH_WINDOWS_ROOT');
}

for (const dialect of [posix, win32]) {
  throws(() => dialect.common('a'), TypeError);
  throws(() => dialect.common(['none', 'other', '\0invalid']), TypeError);
  throws(() => dialect.resolve('\0ignored', '/absolute'), TypeError);
  throws(() => dialect.relative('needs cwd', '\0invalid'), TypeError);
  throws(() => dialect.toNamespacedPath('\0invalid'), TypeError);
  throws(() => dialect.toNamespacedPath({}), TypeError);
}

const selected = path.SEPARATOR === '\\' ? win32 : posix;
for (const name of ['join', 'normalize', 'basename', 'dirname', 'extname', 'isAbsolute',
  'parse', 'format', 'common', 'resolve', 'relative', 'toNamespacedPath']) {
  equal(path[name], selected[name]);
}
equal(path.DELIMITER, selected.delimiter);
equal(path.SEPARATOR_PATTERN instanceof RegExp, true);
equal(path.SEPARATOR_PATTERN.global, false);
equal('a///b'.split(path.SEPARATOR_PATTERN).join(','), 'a,b');
equal(w`a\\b`.split(path.SEPARATOR_PATTERN).join(','), selected === win32 ? 'a,b' : w`a\\b`);
equal(path.readCwd, undefined);
console.log(path.SEPARATOR);
"##,
    );
    assert_ok(&output);
    assert_eq!(output.stdout, if cfg!(windows) { b"\\\n" } else { b"/\n" });
}

#[test]
fn cli_permission_controls_cwd_and_resolution_observes_chdir() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("子")).unwrap();
    let denied = run_cli(
        dir.path(),
        &["--allow-env"],
        "denied-cwd.js",
        r#"import {resolve} from 'neo:path';
resolve('sentinel-relative');"#,
    );
    assert!(!denied.status.success());
    let message = stderr(&denied);
    assert!(message.contains("provide an absolute base"), "{message}");
    assert!(!message.contains("sentinel-relative"), "{message}");

    let output = run_cli(
        dir.path(),
        &["--allow-stdio"],
        "cwd.js",
        r#"import { equal } from 'neo:assert';
import { cwd, chdir } from 'neo:process';
import { join, resolve, relative } from 'neo:path';
equal(resolve(), resolve(cwd()));
const before = resolve('file');
chdir('子');
equal(resolve('file'), join(cwd(), 'file'));
equal(relative('.', resolve('😀')), '😀');
equal(resolve('file') === before, false);
console.log('ok');
"#,
    );
    assert_ok(&output);
    assert_eq!(output.stdout, b"ok\n");
}

#[test]
fn typescript_entry_imports_path_without_permissions() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        "main.ts",
        r#"import { posix, win32, resolve, relative, SEPARATOR } from 'neo:path';
import type { ParsedPath } from 'neo:path';

const parsed: ParsedPath = posix.parse('/a/b.txt');
const joined: string = win32.join('C:\\', 'foo');
const base = SEPARATOR === '\\' ? 'C:\\base' : '/base';
const fromBase: string = relative(base, resolve(base, 'b'));
if (fromBase !== 'b') throw new Error('path resolution');
if (parsed.ext !== '.txt') throw new Error('ext');
if (joined !== 'C:\\foo') throw new Error('join');
const names: string[] = [
  posix.basename(parsed.base, parsed.ext),
  win32.basename(joined),
];
console.log(names.join(','));
"#,
    );
    assert_ok(&output);
    assert_eq!(output.stdout, b"b,foo\n");
}

#[test]
fn argument_errors_surface_as_type_errors() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        "caught.js",
        r#"import { posix, win32 } from 'neo:path';
for (const [api, value] of [[posix, 1], [win32, null]]) {
  try {
    api.normalize(value);
    console.log('no-throw');
  } catch (error) {
    console.log(error.name);
  }
}
"#,
    );
    assert_ok(&output);
    assert_eq!(output.stdout, b"TypeError\nTypeError\n");
}

#[test]
fn nul_is_rejected_and_the_message_does_not_echo_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        "nul.js",
        "import { posix } from 'neo:path';\nposix.normalize('sentinel\\u0000value');\n",
    );
    assert!(!output.status.success());
    let text = stderr(&output);
    assert!(text.contains("NUL"), "{text}");
    assert!(!text.contains("sentinel"), "{text}");
}
