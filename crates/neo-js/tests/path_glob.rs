//! Integration tests for the public `neo:path` glob helpers.

use neo_js::{NeoJs, Permissions, RuntimeModule, RuntimeOptions};

fn run(source: &str) {
    let js = NeoJs::new_with_options(
        Permissions::default(),
        RuntimeOptions::new().modules([RuntimeModule::Path, RuntimeModule::Assert]),
    )
    .unwrap();
    js.run_module("glob.js", source).unwrap();
}

#[test]
fn glob_matching_reference_matrix_and_explicit_dialect_rules() {
    run(r#"
import { equal } from 'neo:assert';
import * as host from 'neo:path';
const { posix, win32 } = host;
const selected = host.SEPARATOR === '\\' ? win32 : posix;
for (const name of ['isGlob', 'globToRegExp', 'normalizeGlob', 'joinGlobs', 'match', 'matchesGlob'])
  equal(host[name], selected[name], name);
equal(host.match, host.matchesGlob);
equal(posix.match, posix.matchesGlob);
equal(win32.match, win32.matchesGlob);

// Representative behavior from @neotales/path's Deno-derived glob tests.
const shared = [
  ['*.js', 'file.js', true], ['*.js', '.file.js', true],
  ['*.js', 'sub/file.js', false], ['f?o', 'foo', true],
  ['f?o', 'fo', false], ['foo/bar', 'foo///bar//', true],
  ['foo///bar///', 'foo/bar', true], ['foo', 'xfoo', false],
  ['foo', 'foo\n', false], ['', '', false], ['', 'foo', false],
  ['**/foo.txt', 'foo.txt', true], ['**/*.txt', '/foo/bar.txt', true],
  ['/foo/**/**/bar.txt', '/foo/bar.txt', true], ['/foo/**', '/foo/baz/qux', true],
  ['/foo/**', '/foo/', true], ['/foo/**', '/foo', false],
  ['a/**/b/**/c', 'a/b/c', true], ['a/**/b/**/c', 'a/x/b/y/c', true],
  ['a/**/b/**/c', 'a/x/B/y/c', false], ['/foo/**.txt', '/foo/bar/baz.txt', false],
  ['fo[oz]', 'foz', true], ['fo[a-d]', 'fot', false],
  ['fo[!tz]', 'fob', true], ['fo[!tz]', 'foz', false],
  ['[[:digit:]abc]', '1', true], ['[[:digit:]abc]', 'b', true],
  ['[[:alnum:]]', '!', false], ['[[:punct:]]', '[', true],
  ['[^^]', '^', true], ['[[:space:]]', '\n', true],
  ['foo{bar,b*z}', 'foobuzz', true], ['{a,{b,c}}', 'c', true],
  ['{1..3}', '1..3', true], ['{1..3}', '2', false],
  ['?(ba?(z|?(r))|qux)baz.txt', 'bazbaz.txt', true],
  ['?(foo|bar).txt', '.txt', true], ['?(foo|bar).txt', 'foobar.txt', false],
  ['@(foo|baz)bar.txt', 'foobar.txt', true],
  ['*(foo|fizz).txt', 'foofizz.txt', true], ['*(foo|fizz).txt', '.txt', true],
  ['+(foo|ba[rt]).txt', 'batfoo.txt', true], ['+(foo|bar).txt', '.txt', false],
  ['!(foo|baz)bar.txt', 'buzbar.txt', true], ['!({foo,bar})baz.txt', 'foobaz.txt', false],
  ['!(foo).js', 'foobar.js', false], ['!foo', '!foo', true],
  ['日本/**/é?.txt', '日本/猫/é字.txt', true], ['é', 'e\u0301', false],
  ['?', '😀', false], ['??', '😀', true], ['\ud800', '\ud800', true],
  // Deliberate segment boundaries. A root-only pattern stays rooted.
  ['a?b', 'a/b', false], ['[[:punct:]]', '/', false], ['[!x]', '/', false],
  ['/', '//', true], ['/', '', false], ['/foo', 'foo', false],
];
for (const api of [posix, win32]) {
  for (const [pattern, path, expected] of shared) {
    const label = JSON.stringify([api.sep, pattern, path]);
    equal(api.match(path, pattern), expected, label);
    equal(api.globToRegExp(pattern).test(path), expected, label);
  }
  equal(api.globToRegExp('*.js') instanceof RegExp, true);
  equal(api.globToRegExp('*.js') === api.globToRegExp('*.js'), false);
  equal(api.globToRegExp('*.js').global, false);
  equal(api.globToRegExp('Ä*.JS', { caseInsensitive: true }).test('äfile.js'), true);
  equal(api.globToRegExp('FOO').test('foo'), false);
  equal(api.globToRegExp('**/foo', { globstar: false }).test('a/b/foo'), false);
  equal(api.globToRegExp('**/foo', { globstar: false }).test('a/foo'), true);
  equal(api.globToRegExp('?(foo)', { extended: false }).test('x(foo)'), true);
  equal(api.globToRegExp('@(foo)', { extended: false }).test('@(foo)'), true);
  equal(api.globToRegExp('{a,b}', { extended: false }).test('b'), true);
}
const r = String.raw;
equal(posix.globToRegExp('*.js').source, /^[^/]*\.js\/*$/.source);
equal(posix.match('[ab]', r`\[ab]`), true);
equal(posix.match('a\\b.txt', r`a\\*.txt`), true);
equal(posix.match('src\\path.ts', 'src/*.ts'), false);
equal(posix.match('src\\path.ts', '*.ts'), true);
equal(win32.match('src\\path.ts', 'src/*.ts'), true);
equal(win32.match('src\\path.ts', '*.ts'), false);
equal(win32.match('src/path.ts', r`src\*.ts`), true);
equal(win32.match('[ab]', '`[ab]'), true);
equal(win32.match('ab', '`[ab]'), false);
equal(win32.match('C:\\src\\\\sub/path.ts', 'C:/src/**/path.ts'), true);
equal(win32.match('a\\b', 'a?b'), false);
equal(win32.globToRegExp('[!x]').test('\\'), false);
"#);
}

#[test]
fn detection_joining_and_normalization_do_not_compile_or_read_cwd() {
    run(r#"
import { equal } from 'neo:assert';
import { posix, win32 } from 'neo:path';
for (const api of [posix, win32]) {
  const sep = api.sep;
  for (const input of ['!foo.js', 'f?o.js', '**/abc.js', 'abc/(a|b).js',
    'abc/[a-z].js', 'abc/{a..z}.js', 'abc/@(a).js', 'abc/+(foo|bar)'])
    equal(api.isGlob(input), true, input);
  for (const input of ['', 'foo/../bar', 'abc/[abc', 'abc/(a|b', 'abc/\\?.js',
    'abc/\\{a,*}.js', 'abc/\\(a|b).js', 'abc/def/!ghi.js'])
    equal(api.isGlob(input), false, input);
  equal(api.joinGlobs([]), '.');
  equal(api.joinGlobs(['', ''], { globstar: true }), '.');
  const parts = Object.freeze(['foo', '*', 'bar']);
  equal(api.joinGlobs(parts), 'foo' + sep + '*' + sep + 'bar');
  equal(api.joinGlobs(['**', '..']), '.');
  equal(api.joinGlobs(['**', '..'], { globstar: true }), '**' + sep + '..');
  equal(api.normalizeGlob('**///.././x/', { globstar: true }), ['**', '..', 'x', ''].join(sep));
  equal(api.normalizeGlob('**/../..', { globstar: true }), '**');
  equal(api.normalizeGlob('**/./..', { globstar: true }), '.');
  equal(api.normalizeGlob('foo/bar/../baz'), 'foo' + sep + 'baz');
  equal(api.normalizeGlob('[malformed'), '[malformed');
  equal(api.normalizeGlob('/\ud800/**/..', { globstar: true }), ['', '\ud800', '**', '..'].join(sep));
}
equal(win32.normalizeGlob('C:/a\\**/..\\b', { globstar: true }), 'C:\\a\\**\\..\\b');
equal(win32.joinGlobs(['//server', 'share', '**', '..'], { globstar: true }), '\\\\server\\share\\**\\..');
equal(win32.joinGlobs(['/', '/server', 'share']), '\\server\\share');
equal(win32.joinGlobs(['/', '/server', 'share'], { globstar: true }), '\\server\\share');
equal(win32.joinGlobs(['/', 'server', 'share'], { globstar: true }), '\\\\server\\share\\');
"#);
}

#[test]
fn compiler_and_matcher_do_not_consult_mutated_prototypes() {
    run(r#"
import { equal } from 'neo:assert';
import { posix, win32 } from 'neo:path';

let interceptionCount = 0;
function unexpected() {
  ++interceptionCount;
  throw Error('unexpected inherited property access');
}
const indices = ['0', '1', '2', '3'];
const fields = ['min', 'literal', 'get', 'set', 'writable'];
const parts = ['a', '**', '..'];
const poisonedLength = { valueOf: unexpected, toString: unexpected };
let posixResult, windowsResult, invalidError, lengthError;
try {
  for (let i = 0; i < indices.length; ++i) {
    Object.defineProperty(Array.prototype, indices[i], {
      __proto__: null, configurable: true, get: unexpected, set: unexpected,
    });
  }
  for (let i = 0; i < fields.length; ++i) {
    Object.defineProperty(Object.prototype, fields[i], {
      __proto__: null, configurable: true, get: unexpected,
    });
  }
  posixResult = [
    posix.match('a/x', 'a/*x'),
    posix.match('a/3', 'a/?([[:digit:]]|b)'),
    posix.match('a/xyab', 'a/+(a|xy)b'),
    posix.globToRegExp('+(ab|ax)', { caseInsensitive: true }).test('AXab'),
    posix.globToRegExp('a/**/**').test('a/folder/file'),
    posix.isGlob('\\*'),
    posix.joinGlobs(parts, { globstar: true }),
  ];
  windowsResult = [
    win32.match('a\\x', 'a/*x'),
    win32.globToRegExp('a/!(foo)').test('a\\bar'),
    win32.joinGlobs(parts),
  ];
  try { posix.globToRegExp('sentinel[bad'); } catch (error) { invalidError = error; }
  try {
    posix.joinGlobs(new Proxy([], { get: () => poisonedLength }));
  } catch (error) { lengthError = error; }
} finally {
  for (let i = 0; i < fields.length; ++i) delete Object.prototype[fields[i]];
  for (let i = 0; i < indices.length; ++i) delete Array.prototype[indices[i]];
}
equal(interceptionCount, 0);
equal(JSON.stringify(posixResult), '[true,true,true,true,true,false,"a/**/.."]');
equal(JSON.stringify(windowsResult), '[true,true,"a"]');
equal(invalidError.code, 'PATH_GLOB_SYNTAX');
equal(invalidError.message.includes('sentinel'), false);
equal(lengthError instanceof TypeError, true);
"#);
}

#[test]
fn array_length_is_snapshotted_once_without_coercion() {
    run(r#"
import { equal, throws } from 'neo:assert';
import { posix, win32 } from 'neo:path';
const parts = Array(257).fill('');
let coercions = 0;
const changing = {
  [Symbol.toPrimitive]() { return ++coercions === 1 ? 1 : 257; },
};
for (const api of [posix, win32]) {
  for (const invalid of [changing, new Number(1), '1', 1n, null, Symbol(), -1, 1.5,
    NaN, Infinity, Number.MAX_SAFE_INTEGER + 1]) {
    let lengths = 0;
    let elements = 0;
    const proxy = new Proxy(parts, {
      get(target, key) {
        if (key === 'length') { ++lengths; return invalid; }
        ++elements;
        return target[key];
      },
    });
    throws(() => api.joinGlobs(proxy), TypeError);
    throws(() => api.common(proxy), TypeError);
    equal(lengths, 2);
    equal(elements, 0);
  }
  let lengths = 0;
  let elements = 0;
  const oversize = new Proxy(parts, {
    get(target, key) {
      if (key === 'length') { ++lengths; return 257; }
      ++elements;
      return target[key];
    },
  });
  let oversizedError;
  try { api.joinGlobs(oversize); } catch (error) { oversizedError = error; }
  equal(oversizedError instanceof RangeError, true);
  equal(oversizedError.code, 'PATH_GLOB_LIMIT');
  equal(lengths, 1);
  equal(elements, 0);
  lengths = 0;
  const single = new Proxy(['a', 'b'], {
    get(target, key) {
      return key === 'length' ? (++lengths === 1 ? 1 : 257) : target[key];
    },
  });
  equal(api.joinGlobs(single), 'a');
  equal(lengths, 1);
  equal(api.joinGlobs(Array(256).fill('')), '.');
}
equal(coercions, 0);
"#);
}

#[test]
fn pinned_upstream_fixture_records_agreements_and_migration_differences() {
    let fixture = include_str!("fixtures/path-glob-reference.json");
    run(&format!(
        r#"
import {{ equal }} from 'neo:assert';
import {{ posix, win32 }} from 'neo:path';
const {{ cases }} = {fixture};
function outcome(call) {{
  try {{ return call(); }} catch (error) {{ return error.code; }}
}}
for (const [dialect, pattern, subject, upstream, expected] of cases) {{
  const apis = dialect === 'both' ? [posix, win32] : [dialect === 'posix' ? posix : win32];
  for (const api of apis) {{
    const label = JSON.stringify([api.sep, pattern, subject, {{ upstream }}]);
    equal(outcome(() => api.globToRegExp(pattern).test(subject)), expected, label);
    equal(outcome(() => api.match(subject, pattern)), expected, label);
  }}
}}
"#
    ));
}

#[test]
fn rejects_invalid_values_syntax_and_limits_without_echoing_input() {
    run(r#"
import { equal, throws } from 'neo:assert';
import { posix, win32 } from 'neo:path';
function fails(call, type, code) {
  try { call(); } catch (error) {
    equal(error instanceof type, true, error.toString());
    equal(error.code, code);
    equal(error.message.includes('sentinel'), false);
    return;
  }
  throw Error('expected rejection');
}
for (const api of [posix, win32]) {
  for (const value of [undefined, {}, [], new String('*'), null, 42, 'sentinel\0']) {
    throws(() => api.isGlob(value), TypeError);
    throws(() => api.globToRegExp(value), TypeError);
    throws(() => api.normalizeGlob(value), TypeError);
    throws(() => api.joinGlobs([value, '']), TypeError);
    throws(() => api.match(value, '*'), TypeError);
    throws(() => api.match('file', value), TypeError);
  }
  for (const options of [null, [], 3, { globstar: 1 }]) {
    throws(() => api.globToRegExp('', options), TypeError);
    throws(() => api.normalizeGlob('', options), TypeError);
    throws(() => api.joinGlobs([], options), TypeError);
  }
  throws(() => api.globToRegExp('file', { extended: 'yes' }), TypeError);
  throws(() => api.globToRegExp('file', { caseInsensitive: null }), TypeError);
  throws(() => api.joinGlobs([, 'a']), TypeError);
  throws(() => api.joinGlobs('a'), TypeError);
  equal(api.globToRegExp('a', Object.create({ caseInsensitive: true })).test('A'), false);
  for (const pattern of ['sentinel[ab', 'sentinel{a,b', 'sentinel?(a|b', '[z-a]',
    '[]', '[!]', '[[:unknown:]]', '[[:digit]', '@(a/b)', '{a/b,c}', '[a/b]']) {
    fails(() => api.globToRegExp(pattern), SyntaxError, 'PATH_GLOB_SYNTAX');
    fails(() => api.match('file', pattern), SyntaxError, 'PATH_GLOB_SYNTAX');
  }
  const escape = api === posix ? '\\' : '`';
  fails(() => api.globToRegExp('sentinel' + escape), SyntaxError, 'PATH_GLOB_SYNTAX');
  fails(() => api.globToRegExp('sentinel' + escape + '/x'), SyntaxError, 'PATH_GLOB_SYNTAX');
  const max = 'x'.repeat(4096);
  equal(api.globToRegExp(max).test(max), true);
  equal(api.match(max, max), true);
  equal(api.normalizeGlob(max), max);
  for (const pattern of [max + 'sentinel', '@('.repeat(17) + 'a' + ')'.repeat(17),
    '{a,b}'.repeat(7), 'a*'.repeat(7), '+(a|aa)b', '+(a*)b', '?(a|b)'.repeat(4),
    '?'.repeat(3000)]) {
    fails(() => api.globToRegExp(pattern), RangeError, 'PATH_GLOB_LIMIT');
    fails(() => api.match('', pattern), RangeError, 'PATH_GLOB_LIMIT');
  }
  fails(() => api.globToRegExp('+(a|A)', { caseInsensitive: true }), RangeError, 'PATH_GLOB_LIMIT');
  fails(() => api.isGlob(max + 'x'), RangeError, 'PATH_GLOB_LIMIT');
  fails(() => api.normalizeGlob(max + 'x'), RangeError, 'PATH_GLOB_LIMIT');
  fails(() => api.joinGlobs([max, 'x']), RangeError, 'PATH_GLOB_LIMIT');
  fails(() => api.joinGlobs(Array(257)), RangeError, 'PATH_GLOB_LIMIT');
  fails(() => api.match('x'.repeat(16385), '*'), RangeError, 'PATH_GLOB_LIMIT');
}
"#);
}
