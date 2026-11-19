//! Integration tests for `neo:string-builder` through the public `NeoJs` API.
//!
//! The module needs no permission flag and grants no host capability.

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
fn appends_chains_and_reports_length() {
    run(r#"import test from 'neo:test';
import { equal } from 'neo:assert';
import { StringBuilder } from 'neo:string-builder';

test('append returns the builder and concatenates in order', () => {
  const builder = new StringBuilder();
  equal(builder.length, 0);
  equal(builder.toString(), '');
  equal(builder.string, '');
  equal(builder.append('hello'), builder);
  equal(builder.append(' ').append('world').toString(), 'hello world');
  equal(builder.length, 11);
  equal(builder.string, 'hello world');
});

test('an empty append changes nothing', () => {
  const builder = new StringBuilder();
  builder.append('a').append('').append('b');
  equal(builder.toString(), 'ab');
  equal(builder.length, 2);
});"#);
}

#[test]
fn converts_only_documented_primitive_types() {
    run(r#"import test from 'neo:test';
import { equal, throws } from 'neo:assert';
import { StringBuilder } from 'neo:string-builder';

test('numbers, booleans, and bigints are converted with String', () => {
  const builder = new StringBuilder();
  builder.append(42).append('|').append(true).append('|').append(7n);
  builder.append('|').append(1.5).append('|').append(NaN);
  equal(builder.toString(), '42|true|7|1.5|NaN');
});

test('other values are rejected instead of coerced', () => {
  const builder = new StringBuilder();
  throws(() => builder.append(), TypeError);
  throws(() => builder.append(undefined), TypeError);
  throws(() => builder.append(null), TypeError);
  throws(() => builder.append(Symbol('x')), TypeError);
  throws(() => builder.append({}), TypeError);
  throws(() => builder.append([]), TypeError);
  throws(() => builder.append(() => {}), TypeError);
  throws(() => builder.append(new String('x')), TypeError);
  throws(() => builder.append(new Date(0)), TypeError);
  equal(builder.length, 0);
});"#);
}

#[test]
fn clear_returns_a_reusable_empty_builder() {
    run(r#"import test from 'neo:test';
import { equal } from 'neo:assert';
import { StringBuilder } from 'neo:string-builder';

test('clear resets text and length', () => {
  const builder = new StringBuilder();
  builder.append('first');
  equal(builder.clear(), builder);
  equal(builder.length, 0);
  equal(builder.codePointLength, 0);
  equal(builder.toString(), '');
});

test('the same builder accepts new text after clear', () => {
  const builder = new StringBuilder();
  builder.append('first').clear().append('second');
  equal(builder.toString(), 'second');
  equal(builder.length, 6);
  builder.clear();
  builder.clear();
  equal(builder.toString(), '');
  equal(builder.append('third').toString(), 'third');
});"#);
}

#[test]
fn code_points_and_utf16_units_are_counted_separately() {
    run(r#"import test from 'neo:test';
import { equal, throws } from 'neo:assert';
import { StringBuilder } from 'neo:string-builder';

test('an astral code point is two units and one code point', () => {
  const builder = new StringBuilder();
  builder.appendCodePoint(0x1f600);
  equal(builder.toString(), '😀');
  equal(builder.length, 2);
  equal(builder.codePointLength, 1);
  builder.append('x');
  equal(builder.length, 3);
  equal(builder.codePointLength, 2);
});

test('a combining mark is a separate code point', () => {
  const builder = new StringBuilder();
  builder.append('e').appendCodePoint(0x301);
  equal(builder.length, 2);
  equal(builder.codePointLength, 2);
});

test('appendCodePoint rejects non-scalars', () => {
  const builder = new StringBuilder();
  throws(() => builder.appendCodePoint('65'), TypeError);
  throws(() => builder.appendCodePoint(1.5), RangeError);
  throws(() => builder.appendCodePoint(-1), RangeError);
  throws(() => builder.appendCodePoint(0x110000), RangeError);
  throws(() => builder.appendCodePoint(0xd800), RangeError);
  throws(() => builder.appendCodePoint(0xdfff), RangeError);
  equal(builder.length, 0);
});"#);
}

#[test]
fn cached_text_is_invalidated_by_append_and_clear() {
    run(r#"import test from 'neo:test';
import { equal } from 'neo:assert';
import { StringBuilder } from 'neo:string-builder';

test('toString and string follow every change', () => {
  const builder = new StringBuilder();
  builder.append('a');
  equal(builder.toString(), 'a');
  equal(builder.string, 'a');
  builder.append('b');
  equal(builder.toString(), 'ab');
  equal(builder.string, 'ab');
  equal(builder.codePointLength, 2);
  builder.clear();
  equal(builder.codePointLength, 0);
  equal(builder.toString(), '');
});"#);
}

#[test]
fn inherited_array_index_setters_cannot_divert_appended_text() {
    run(r#"import test from 'neo:test';
import { equal } from 'neo:assert';
import { StringBuilder } from 'neo:string-builder';

test('append defines its own array element', () => {
  const descriptor = Object.getOwnPropertyDescriptor(Array.prototype, '0');
  try {
    Object.defineProperty(Array.prototype, '0', {
      set() { throw Error('Array.prototype index setter was used'); },
      configurable: true,
    });
    const builder = new StringBuilder();
    builder.append('a').append('b');
    if (builder.toString() !== 'ab') throw Error('appended text was diverted');
    if (builder.length !== 2) throw Error('length was diverted');
  } finally {
    if (descriptor) Object.defineProperty(Array.prototype, '0', descriptor);
    else delete Array.prototype[0];
  }
  equal(new StringBuilder().append('x').toString(), 'x');
});"#);
}

#[test]
fn inherited_descriptor_fields_do_not_break_append() {
    run(r#"import test from 'neo:test';
import { StringBuilder } from 'neo:string-builder';

test('append ignores inherited descriptor fields and array setters', () => {
  const objectPrototype = Object.prototype;
  const arrayPrototype = Array.prototype;
  const get = Object.getOwnPropertyDescriptor(objectPrototype, 'get');
  const set = Object.getOwnPropertyDescriptor(objectPrototype, 'set');
  const index = Object.getOwnPropertyDescriptor(arrayPrototype, '0');
  try {
    Object.defineProperty(arrayPrototype, '0', {
      __proto__: null,
      set() { throw Error('array setter called'); },
      configurable: true,
    });
    Object.defineProperty(objectPrototype, 'get', {
      __proto__: null, value: () => {}, configurable: true,
    });
    Object.defineProperty(objectPrototype, 'set', {
      __proto__: null, value: () => {}, configurable: true,
    });
    const builder = new StringBuilder();
    builder.append('a').appendCodePoint(0x1f600);
    if (builder.toString() !== 'a😀' || builder.length !== 3 ||
        builder.codePointLength !== 2) throw Error('incorrect builder state');
    builder.clear().append('b');
    if (builder.toString() !== 'b') throw Error('clear or append failed');
  } finally {
    delete objectPrototype.get;
    delete objectPrototype.set;
    if (get) Object.defineProperty(objectPrototype, 'get', get);
    if (set) Object.defineProperty(objectPrototype, 'set', set);
    if (index) Object.defineProperty(arrayPrototype, '0', index);
    else delete arrayPrototype[0];
  }
});"#);
}

#[test]
fn module_is_available_with_every_permission_enabled() {
    let js = NeoJs::new(Permissions {
        filesystem: true,
        environment: true,
        stdio: true,
        process: true,
        registry: true,
        network: true,
        user: true,
    })
    .unwrap();
    js.run_module(
        "main.js",
        "import { StringBuilder } from 'neo:string-builder';
         const text = new StringBuilder().append('a').append(1).toString();
         if (text !== 'a1') throw Error('builder failed');",
    )
    .unwrap();
}
