//! Public file URL conversions, including Windows syntax on every host.

use neo_js::{NeoJs, Permissions, RuntimeModule, RuntimeOptions};

#[test]
fn reference_tables_and_file_url_behavior() {
    let js = NeoJs::new_with_options(
        Permissions::default(),
        RuntimeOptions::new().modules([
            RuntimeModule::Path,
            RuntimeModule::Test,
            RuntimeModule::Assert,
        ]),
    )
    .unwrap();
    js.run_module(
        "path-url.js",
        r#"
import test from 'neo:test';
import { equal, throws } from 'neo:assert';
import { toFileUrl, fromFileUrl, posix, win32, SEPARATOR } from 'neo:path';

test('pinned upstream conversion tables', () => {
  // js-libraries c7e88644f45a78f65bf639c9d365bb9eff21bcfe, std/jsr/path
  for (const [path, href] of [
    ['/home/foo', 'file:///home/foo'],
    ['/home/ ', 'file:///home/%20'],
    ['/home/%20', 'file:///home/%2520'],
    ['/home\\foo', 'file:///home%5Cfoo'],
    ['//localhost/home/foo', 'file:///localhost/home/foo'],
    ['//localhost/', 'file:///localhost/'],
    ['//:/home/foo', 'file:///:/home/foo'],
    ['///home/foo', 'file:///home/foo'],
    ['////home/foo', 'file:///home/foo'],
  ]) equal(posix.toFileUrl(path).href, href);
  for (const [path, href] of [
    ['/home/foo', 'file:///home/foo'],
    ['/home/ ', 'file:///home/%20'],
    ['/home/%20', 'file:///home/%2520'],
    ['/home\\foo', 'file:///home/foo'],
    ['C:/', 'file:///C:/'],
    ['//localhost/home/foo', 'file:///home/foo'],
    ['//127.0.0.1/home/foo', 'file://127.0.0.1/home/foo'],
    ['//localhost/', 'file:///'],
    ['//127.0.0.1/', 'file://127.0.0.1/'],
  ]) equal(win32.toFileUrl(path).href, href);
  throws(() => posix.toFileUrl('foo'), TypeError);
  throws(() => posix.toFileUrl('C:/'), TypeError);
  throws(() => win32.toFileUrl('foo'), TypeError);
  throws(() => win32.toFileUrl('//:/home/foo'), TypeError);

  for (const [input, posixPath, windowsPath] of [
    ['file:///', '/', '\\'],
    ['file:///home/foo', '/home/foo', '\\home\\foo'],
    ['file:///home/foo%20bar', '/home/foo bar', '\\home\\foo bar'],
    ['file:///%', '/%', '\\%'],
    ['file://localhost/foo', '/foo', '\\foo'],
    ['file://127.0.0.1/foo', '/foo', '\\\\127.0.0.1\\foo'],
    ['file:///C:', '/C:', 'C:\\'],
    ['file:///C:/', '/C:/', 'C:\\'],
    ['file://localhost/C:/', '/C:/', 'C:\\'],
    ['file:///C:/Users/', '/C:/Users/', 'C:\\Users\\'],
    ['file:///C:foo/bar', '/C:foo/bar', '\\C:foo\\bar'],
  ]) {
    equal(posix.fromFileUrl(input), posixPath);
    equal(posix.fromFileUrl(new URL(input)), posixPath);
    equal(win32.fromFileUrl(input), windowsPath);
    equal(win32.fromFileUrl(new URL(input)), windowsPath);
  }
  for (const input of ['http://localhost/foo', 'abcd://localhost/foo']) {
    throws(() => posix.fromFileUrl(input), TypeError);
    throws(() => win32.fromFileUrl(input), TypeError);
  }
  const native = SEPARATOR === '/' ? posix : win32;
  equal(toFileUrl, native.toFileUrl);
  equal(fromFileUrl, native.fromFileUrl);
});

test('WHATWG file path cases preserve host and empty segments', () => {
  // Selected WPT rows shipped in url 2.5.8: urltestdata.json and setters_tests.json.
  for (const [input, base, expected] of [
    ['file:////foo', undefined, 'file:////foo'],
    ['file:\\\\localhost//', undefined, 'file:////'],
    ['file://localhost//a//../..//foo', undefined, 'file://///foo'],
    ['file:/.//p', undefined, 'file:////p'],
    ['file://spider///', undefined, 'file://spider///'],
    ['file://example.net/C:/', undefined, 'file://example.net/C:/'],
    ['file://[1::8]/C:/', undefined, 'file://[1::8]/C:/'],
    ['file://1.2.3.4/C:/', undefined, 'file://1.2.3.4/C:/'],
    ['file://loC𝐀𝐋𝐇𝐨𝐬𝐭/usr/bin', undefined, 'file:///usr/bin'],
    ['file://a%C2%ADb/p', undefined, 'file://ab/p'],
    ['file://C|/', undefined, 'file:///C:/'],
    ['file:///C%3A/', undefined, 'file:///C%3A/'],
    ['file:///./Y:', undefined, 'file:///Y:'],
    ['file:', 'file:///test?test#test', 'file:///test?test'],
    ['#x', 'file:///test?test#test', 'file:///test?test#x'],
    ['C|/', 'file://host/', 'file://host/C:/'],
    ['/', 'file://h/C:/a/b', 'file://h/C:/'],
    ['..', 'file:///C:/', 'file:///C:/'],
    ['/////mouse', 'file:///elephant', 'file://///mouse'],
  ]) equal(new URL(input, base).href, expected);
  for (const input of [
    'file://example:1/', 'file://%43%3A/', 'file://%C2%AD/p', 'file://user@host/',
    'file://[example]/', 'file://xn--/p',
  ]) throws(() => new URL(input), TypeError);

  const url = new URL('file://host/a?b#c');
  url.pathname = '//a//b';
  equal(url.href, 'file://host//a//b?b#c');
  url.pathname = '\\\\';
  equal(url.href, 'file://host//?b#c');
  url.pathname = 'C|/a/../%2e/é?#';
  equal(url.href, 'file://host/C:/%C3%A9%3F%23?b#c');
  url.hostname = 'bücher.example';
  equal(url.hostname, 'xn--bcher-kva.example');
  url.host = '127.1';
  equal(url.hostname, '127.0.0.1');
  url.host = 'invalid:8000';
  equal(url.hostname, '127.0.0.1');
  url.search = '?';
  url.hash = '#';
  equal(url.search, '');
  equal(url.hash, '');
  equal(url.href, 'file://127.0.0.1/C:/%C3%A9%3F%23?#');
  url.search = 'a#b\'';
  url.hash = 'a b';
  equal(url.href, 'file://127.0.0.1/C:/%C3%A9%3F%23?a%23b%27#a%20b');
  equal(url.origin, 'null');
  equal(url.searchParams, undefined);
  url.protocol = 'https:';
  url.username = 'user';
  url.password = 'pass';
  url.port = '1';
  equal(url.protocol, 'file:');
  equal(url.username + url.password + url.port, '');
  throws(() => { url.href = 'https://example.net/'; }, TypeError);
  equal(url.protocol, 'file:');
  url.href = 'file:///elsewhere';
  equal(win32.fromFileUrl(url), '\\elsewhere');
  equal(win32.fromFileUrl('file://example.net/C:/'), '\\\\example.net\\C:\\');
  equal(win32.fromFileUrl('file:////server/share'), '\\\\server\\share');
});

test('encoding, decoding, and errors do not expose the input', () => {
  const path = '/a//é/💡\uD800?#[\\ %\t\n\v\f\r';
  const url = posix.toFileUrl(path);
  equal(url.href, 'file:///a//%C3%A9/%F0%9F%92%A1%EF%BF%BD%3F%23[%5C%20%25%09%0A%0B%0C%0D');
  equal(posix.fromFileUrl(url), '/a//é/💡�?#[\\ %\t\n\v\f\r');
  equal(new URL('file:///a^b?^#^').href, 'file:///a%5Eb?^#^');
  url.pathname = '/a^b';
  equal(url.pathname, '/a%5Eb');
  const windowsPath = 'C:\\a\r\nb\\c';
  equal(win32.toFileUrl(windowsPath).href, 'file:///C:/a%0D%0Ab/c');
  equal(win32.fromFileUrl(win32.toFileUrl(windowsPath)), windowsPath);
  equal(posix.fromFileUrl(' file:///🌱\uDC00/%zz/%2F/%5c?%00#x '), '/🌱�/%zz///\\');
  equal(win32.fromFileUrl('file:///C:/a%2Fb%5cc'), 'C:\\a/b\\c');
  equal(posix.fromFileUrl('file://server/a#b?c'), '/a');
  for (const [operation, type] of [
    [() => posix.fromFileUrl('file:///sentinel%C0%80'), URIError],
    [() => win32.fromFileUrl('file:///sentinel%ED%A0%80'), URIError],
    [() => posix.fromFileUrl(new URL('file:///sentinel%00')), TypeError],
    [() => win32.fromFileUrl('file:///sentinel%00'), TypeError],
    [() => posix.fromFileUrl('file:///sentinel\u0000'), TypeError],
    [() => posix.toFileUrl('/sentinel\u0000'), TypeError],
    [() => posix.toFileUrl('sentinel'), TypeError],
    [() => new URL('file://sentinel@/'), TypeError],
    [() => posix.toFileUrl('/sentinel' + 'x'.repeat(65536)), RangeError],
    [() => posix.toFileUrl('/sentinel' + '💡'.repeat(6000)), RangeError],
    [() => posix.fromFileUrl('file:///sentinel' + 'x'.repeat(65536)), RangeError],
    [() => new URL('file://sentinel' + 'x'.repeat(8192)), RangeError],
  ]) {
    try { operation(); }
    catch (error) {
      equal(error instanceof type, true);
      equal(error.message.includes('sentinel'), false);
      continue;
    }
    throw new Error('expected conversion failure');
  }
});

test('brand and captured operations do not trust user mutation', () => {
  const Original = globalThis.URL;
  const url = posix.toFileUrl('/actual%/file');
  const descriptor = Object.getOwnPropertyDescriptor(Original.prototype, 'pathname');
  equal(url instanceof Original, true);
  equal(Object.prototype.toString.call(url), '[object URL]');
  equal(url.toJSON(), String(url));
  equal(new Original(url).href, url.href);
  equal(new Original('../b', url).href, 'file:///b');
  equal(Object.getPrototypeOf(posix.toFileUrl('/')), Original.prototype);
  throws(() => Original('file:///'), TypeError);

  const fake = Object.create(Original.prototype);
  equal(fake instanceof Original, true);
  throws(() => posix.fromFileUrl(fake), TypeError);
  throws(() => descriptor.get.call(fake), TypeError);
  throws(() => posix.fromFileUrl(new Proxy(url, {})), TypeError);
  const evil = { toString() { throw Error('user callback invoked'); } };
  throws(() => posix.fromFileUrl(evil), TypeError);
  throws(() => new Original(evil), TypeError);

  globalThis.URL = function Fake() { throw Error('global used'); };
  Object.defineProperty(url, 'href', { get() { throw Error('own href used'); } });
  Object.defineProperty(Original.prototype, 'pathname', {
    get() { throw Error('prototype getter used'); },
    set() { throw Error('prototype setter used'); },
  });
  const exec = Object.getOwnPropertyDescriptor(RegExp.prototype, 'exec');
  const slice = Object.getOwnPropertyDescriptor(String.prototype, 'slice');
  const first = Object.getOwnPropertyDescriptor(Array.prototype, '0');
  function poison() { throw Error('mutable prototype used'); }
  let decoded, created, mutated;
  try {
    RegExp.prototype.exec = String.prototype.slice = poison;
    Object.defineProperty(Array.prototype, '0', { set: poison, configurable: true });
    decoded = posix.fromFileUrl(url);
    created = win32.toFileUrl('\\\\example.net\\a\\b');
    descriptor.set.call(url, '//set//path');
    mutated = posix.fromFileUrl(url);
  } finally {
    if (first) Object.defineProperty(Array.prototype, '0', first);
    else delete Array.prototype[0];
    Object.defineProperty(String.prototype, 'slice', slice);
    Object.defineProperty(RegExp.prototype, 'exec', exec);
    Object.defineProperty(Original.prototype, 'pathname', descriptor);
    globalThis.URL = Original;
  }
  equal(decoded, '/actual%/file');
  equal(created instanceof Original, true);
  equal(created.href, 'file://example.net/a/b');
  equal(mutated, '//set//path');
});
"#,
    )
    .unwrap();
    let report = js.run_pending_tests().unwrap().unwrap();
    assert_eq!(report.total, 4);
    assert_eq!(report.failed, 0, "{:?}", report.failures);
}

#[test]
fn excluding_path_excludes_its_global() {
    let js = NeoJs::new_with_options(
        Permissions::default(),
        RuntimeOptions::new().modules([RuntimeModule::Assert]),
    )
    .unwrap();
    assert_eq!(js.eval("typeof globalThis.URL").unwrap(), "undefined");
    assert!(js.run_module("no-path.js", "import 'neo:path';").is_err());
}

#[test]
fn out_of_range_string_probes_do_not_run_inherited_numeric_getters() {
    let js = NeoJs::new_with_options(
        Permissions::default(),
        RuntimeOptions::new().modules([RuntimeModule::Path]),
    )
    .unwrap();
    // Poison before evaluating any module. The getter's canary also bounds
    // read-through scans if a regression starts consulting numeric indices.
    js.eval(
        r#"
        globalThis.reads = 0;
        globalThis.poison = (throwImmediately) => {
          for (let n = 0; n < 16; ++n) {
            Object.defineProperty(String.prototype, n, {
              configurable: true,
              get() {
                if (++globalThis.reads > 8 || throwImmediately) throw Error('numeric getter called');
                return '/';
              },
            });
          }
        };
        globalThis.restore = () => {
          for (let n = 0; n < 16; ++n) delete String.prototype[n];
        };
        poison(true);
        if (new URL('file:/').href !== 'file:///') throw Error('pre-import URL root changed');
        ''"#,
    )
    .unwrap();
    js.run_module(
        "string-prototype.js",
        r#"import { posix, win32 } from 'neo:path';
        function same(actual, expected) {
          if (actual !== expected) throw Error('file URL root or separator changed');
        }
        function roots() {
          same(new URL('file:').href, 'file:///');
          same(new URL('file:/').href, 'file:///');
          same(new URL('file:/', 'file:///C:/dir').href, 'file:///C:/');
          same(posix.fromFileUrl('file:/'), '/');
          same(posix.toFileUrl('/').href, 'file:///');
          same(posix.toFileUrl('////').href, 'file:///');
          same(win32.toFileUrl('\\').href, 'file:///');
          same(win32.toFileUrl('\\\\').href, 'file:////');
          same(win32.toFileUrl('\\\\server\\').href, 'file://server/');
          const url = new URL('file://server/path');
          url.pathname = '';
          url.hostname = '';
          url.search = '?';
          url.hash = '#';
          same(url.href, 'file:///?#');
          same(posix.fromFileUrl(url), '/');
        }
        try {
          roots();
          poison(false);
          roots();
          same(reads, 0);
        } finally {
          restore();
        }"#,
    )
    .unwrap();
}
