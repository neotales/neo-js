//! Integration tests for `neo:path` through the public `NeoJs` API.
//!
//! No test enables a permission: the module must be available in the default
//! runtime, install nothing on the global object, and behave identically for
//! both path syntaxes regardless of the host platform.

use neo_js::{NeoJs, Permissions, TestReport};

fn run(source: &str) -> TestReport {
    let js = NeoJs::new(Permissions::default()).unwrap();
    js.run_module("main.js", source).unwrap();
    let report = js
        .run_pending_tests()
        .unwrap()
        .expect("neo:test was imported");
    assert!(report.total > 0, "no tests ran");
    assert_eq!(report.failed, 0, "{:?}", report.failures);
    report
}

#[test]
fn imports_without_permissions_and_installs_no_globals() {
    run(r#"import test from 'neo:test';
import { equal } from 'neo:assert';
import { posix, win32 } from 'neo:path';

test('exports two frozen namespaces', () => {
  equal(typeof posix, 'object');
  equal(typeof win32, 'object');
  equal(posix === win32, false);
  equal(Object.isFrozen(posix), true);
  equal(Object.isFrozen(win32), true);
  equal(posix.sep, '/');
  equal(win32.sep, '\\');
  equal(posix.delimiter, ':');
  equal(win32.delimiter, ';');
});

test('each namespace exports the documented functions', () => {
  for (const name of [
    'join', 'normalize', 'basename', 'dirname', 'extname', 'isAbsolute',
    'parse', 'format',
  ]) {
    equal(typeof posix[name], 'function');
    equal(typeof win32[name], 'function');
  }
});

test('no host callbacks or globals are exposed', () => {
  equal(typeof globalThis.posix, 'undefined');
  equal(typeof globalThis.win32, 'undefined');
  equal(typeof globalThis.path, 'undefined');
  equal('__neoPath' in globalThis, false);
});

test('the syntax is chosen by namespace, not by the host', () => {
  equal(posix.isAbsolute('/a'), true);
  equal(posix.isAbsolute('C:\\a'), false);
  equal(win32.isAbsolute('C:\\a'), true);
  equal(win32.isAbsolute('/a'), true);
  equal(posix.normalize('C:\\a\\..\\b'), 'C:\\a\\..\\b');
  equal(win32.normalize('C:\\a\\..\\b'), 'C:\\b');
});"#);
}

#[test]
fn namespaces_are_frozen_and_parse_results_are_fresh() {
    run(r#"import test from 'neo:test';
import { equal, throws } from 'neo:assert';
import { posix } from 'neo:path';

test('namespace properties cannot be replaced or deleted', () => {
  throws(() => { posix.sep = '!'; }, TypeError);
  throws(() => { delete posix.join; }, TypeError);
  equal(posix.sep, '/');
});

test('every parse call returns a new object', () => {
  const first = posix.parse('/a/b.txt');
  const second = posix.parse('/a/b.txt');
  equal(first === second, false);
  first.base = 'changed';
  equal(second.base, 'b.txt');
  equal(posix.parse('/a/b.txt').base, 'b.txt');
});"#);
}

#[test]
fn argument_errors_are_type_errors_and_do_not_echo_input() {
    run(r#"import test from 'neo:test';
import { equal, throws } from 'neo:assert';
import { posix, win32 } from 'neo:path';

test('non-strings are rejected, not coerced', () => {
  for (const value of [
    undefined, null, 1, true, {}, [], new String('a'), Symbol('a'),
  ]) {
    throws(() => posix.normalize(value), TypeError);
    throws(() => win32.normalize(value), TypeError);
  }
  throws(() => posix.join('a', undefined), TypeError);
  throws(() => posix.basename('/a', 1), TypeError);
  throws(() => win32.basename('C:\\a', null), TypeError);
  throws(() => posix.format(null), TypeError);
  throws(() => posix.format([]), TypeError);
  throws(() => posix.format({ dir: 1 }), TypeError);
  throws(() => win32.format({ root: null }), TypeError);
});

test('NUL is rejected and the message never repeats the input', () => {
  try {
    posix.normalize('sentinel\u0000value');
    throw Error('expected a throw');
  } catch (error) {
    equal(error.name, 'TypeError');
    equal(error.message.includes('sentinel'), false);
    equal(error.message.includes('NUL'), true);
  }
  throws(() => win32.join('a', 'b\u0000c'), TypeError);
  throws(() => posix.basename('/a', 'x\u0000'), TypeError);
  throws(() => win32.format({ base: 'x\u0000' }), TypeError);
});"#);
}

#[test]
fn representative_posix_and_win32_semantics() {
    run(r#"import test from 'neo:test';
import { deepEqual, equal } from 'neo:assert';
import { posix, win32 } from 'neo:path';

test('posix roots, dot files, traversal, and trailing separators', () => {
  equal(posix.normalize(''), '.');
  equal(posix.normalize('//'), '/');
  equal(posix.normalize('/a//b/./c/..'), '/a/b');
  equal(posix.normalize('a/../../b'), '../b');
  equal(posix.normalize('a/../'), './');
  equal(posix.basename('/'), '');
  equal(posix.basename('/a/b.txt', '.txt'), 'b');
  equal(posix.dirname('a//b'), 'a');
  equal(posix.dirname('///foo'), '/');
  equal(posix.extname('.bashrc'), '');
  equal(posix.extname('a.tar.gz'), '.gz');
  equal(posix.isAbsolute('//a'), true);
  deepEqual(posix.parse('/a//b/'), {
    root: '/', dir: '/a', base: 'b', ext: '', name: 'b',
  });
});

test('win32 roots, UNC, drive-relative, and separators', () => {
  equal(win32.normalize('C:/foo/..'), 'C:\\');
  equal(win32.normalize('C:'), 'C:.');
  equal(win32.normalize('C:..\\foo'), 'C:..\\foo');
  equal(win32.normalize('\\\\server\\share'), '\\\\server\\share\\');
  equal(win32.normalize('\\\\server\\share\\..\\foo'), '\\\\server\\share\\foo');
  equal(win32.normalize('\\\\\\foo'), '\\foo');
  equal(win32.normalize('1:\\foo'), '1:\\foo');
  equal(win32.join('C:', 'foo'), 'C:\\foo');
  equal(win32.join('//server', 'share'), '\\\\server\\share\\');
  equal(win32.dirname('C:/'), 'C:/');
  equal(win32.dirname('\\\\server\\share\\foo'), '\\\\server\\share\\');
  equal(win32.basename('C:\\foo\\'), 'foo');
  equal(win32.extname('C:\\foo\\a..'), '.');
  equal(win32.isAbsolute('C:foo'), false);
  deepEqual(win32.parse('\\\\?\\C:\\foo'), {
    root: '\\\\?\\C:\\',
    dir: '\\\\?\\C:\\',
    base: 'foo',
    ext: '',
    name: 'foo',
  });
});
"#);
}

#[test]
fn unc_roots_and_trailing_separators_are_not_file_components() {
    run(r#"import test from 'neo:test';
import { equal } from 'neo:assert';
import { posix, win32 } from 'neo:path';

test('a complete UNC root has no basename or extension', () => {
  for (const path of [
    '\\\\server\\share.txt',
    '\\\\server\\share.txt\\',
    '\\\\server\\share.txt\\\\',
  ]) {
    equal(win32.basename(path), '', path);
    equal(win32.extname(path), '', path);
    const parsed = win32.parse(path);
    equal(parsed.base, '', path);
    equal(parsed.ext, '', path);
    equal(parsed.name, '', path);
    equal(parsed.dir, parsed.root, path);
  }
  equal(win32.basename('\\\\?\\C:\\'), '');
  equal(win32.extname('\\\\?\\C:\\'), '');
});

test('the same share is a normal component once something follows it', () => {
  equal(win32.basename('\\\\server\\share.txt\\dir'), 'dir');
  equal(win32.extname('\\\\server\\share.txt\\file.txt'), '.txt');
  equal(win32.extname('\\\\server\\share.txt\\file.txt\\'), '.txt');
});

test('trailing separators are ignored for ordinary components', () => {
  equal(posix.extname('a/b.txt/'), '.txt');
  equal(win32.extname('C:\\foo\\bar.txt\\'), '.txt');
});

test('format appends a separator-valued base instead of dropping it', () => {
  equal(posix.format({ dir: 'foo', base: '/' }), 'foo//');
  equal(win32.format({ dir: 'foo', base: '\\' }), 'foo\\\\');
});"#);
}
