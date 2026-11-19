//! Integration tests for `neo:chars` through the public `NeoJs` API.
//!
//! No test enables a permission: the module must be available in the default
//! runtime and must install nothing on the global object.

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
import * as chars from 'neo:chars';

test('exports are functions', () => {
  equal(typeof chars.isLetter, 'function');
  equal(typeof chars.isLetterAt, 'function');
  equal(typeof chars.equalFold, 'function');
});

test('no host callbacks are exposed', () => {
  equal('__neoChars' in globalThis, false);
  equal('__neoCharsIsLetter' in globalThis, false);
});"#);
}

#[test]
fn changing_regexp_exec_after_import_does_not_change_classification() {
    run(r#"import test from 'neo:test';
import { equal } from 'neo:assert';
import {
  equalFold, isAscii, isDigit, isLetter, isLetterOrDigit, isSpace, isSymbol,
  isSymbolAt,
} from 'neo:chars';

test('classification ignores a replaced RegExp.prototype.exec', () => {
  const originalExec = RegExp.prototype.exec;
  const originalTest = RegExp.prototype.test;
  try {
    RegExp.prototype.exec = function () { return null; };
    RegExp.prototype.test = function () { return false; };
    if (!isAscii(0x41)) throw Error('isAscii');
    if (!isDigit(0x30)) throw Error('isDigit');
    if (!isLetter(0x416)) throw Error('isLetter');
    if (!isLetterOrDigit(0x30)) throw Error('isLetterOrDigit');
    if (!isSpace(0x20)) throw Error('isSpace');
    if (!isSymbol(0x1f600)) throw Error('isSymbol');
    if (!isSymbolAt('😀', 0)) throw Error('isSymbolAt');
    if (!equalFold(0x41, 0x61)) throw Error('equalFold ascii');
    if (!equalFold(0x398, 0x3f4)) throw Error('equalFold theta');
    if (equalFold(0x41, 0x42)) throw Error('equalFold negative');
  } finally {
    RegExp.prototype.exec = originalExec;
    RegExp.prototype.test = originalTest;
  }
  equal(isLetter(0x41), true);
});"#);
}

#[test]
fn ascii_latin1_and_scalar_boundaries_are_exact() {
    run(r#"import test from 'neo:test';
import { equal } from 'neo:assert';
import {
  ASCII_MAX, LATIN1_MAX, MAX_CODE_POINT, SURROGATE_MAX, SURROGATE_MIN,
  isAscii, isChar, isLatin1,
} from 'neo:chars';

test('ascii range', () => {
  equal(ASCII_MAX, 0x7f);
  equal(isAscii(0), true);
  equal(isAscii(0x41), true);
  equal(isAscii(0x7f), true);
  equal(isAscii(0x80), false);
  equal(isAscii(-1), false);
  equal(isAscii(1.5), false);
  equal(isAscii(NaN), false);
  equal(isAscii('65'), false);
});

test('latin1 range', () => {
  equal(LATIN1_MAX, 0xff);
  equal(isLatin1(0), true);
  equal(isLatin1(0xff), true);
  equal(isLatin1(0x100), false);
  equal(isLatin1(0x1f600), false);
});

test('scalar values exclude surrogates', () => {
  equal(MAX_CODE_POINT, 0x10ffff);
  equal(SURROGATE_MIN, 0xd800);
  equal(SURROGATE_MAX, 0xdfff);
  equal(isChar(0), true);
  equal(isChar(0xd7ff), true);
  equal(isChar(0xe000), true);
  equal(isChar(0x10ffff), true);
  equal(isChar(0xd800), false);
  equal(isChar(0xdfff), false);
  equal(isChar(0x110000), false);
  equal(isChar(-1), false);
  equal(isChar(0.5), false);
  equal(isChar(NaN), false);
  equal(isChar(Infinity), false);
});"#);
}

#[test]
fn unicode_categories_follow_unicode_properties() {
    run(r#"import test from 'neo:test';
import { equal } from 'neo:assert';
import {
  isControl, isDigit, isLetter, isLetterOrDigit, isLower, isPunc, isSpace,
  isSymbol, isUpper,
} from 'neo:chars';

test('letters and case', () => {
  equal(isLetter(0x41), true);        // A
  equal(isLetter(0x416), true);       // Cyrillic Zhe
  equal(isLetter(0x3b1), true);       // Greek alpha
  equal(isLetter(0x30), false);       // 0
  equal(isLetter(0x1f600), false);    // emoji is a symbol
  equal(isUpper(0x391), true);        // Greek capital alpha
  equal(isLower(0x3b1), true);        // Greek small alpha
  equal(isUpper(0x1c5), false);       // titlecase is neither Lu nor Ll
  equal(isLower(0x1c5), false);
});

test('digits are decimal digits only', () => {
  equal(isDigit(0x30), true);         // ASCII 0
  equal(isDigit(0x0665), true);       // Arabic-Indic five
  equal(isDigit(0xb2), false);        // superscript two, category No
  equal(isDigit(0x41), false);
});

test('letters or digits', () => {
  equal(isLetterOrDigit(0x41), true);
  equal(isLetterOrDigit(0x30), true);
  equal(isLetterOrDigit(0x416), true);
  equal(isLetterOrDigit(0x0665), true);
  equal(isLetterOrDigit(0x21), false);
});

test('whitespace', () => {
  for (const code of [0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x20, 0x85, 0xa0, 0x2007, 0x3000])
    equal(isSpace(code), true, 'code ' + code);
  equal(isSpace(0), false);
  equal(isSpace(0x200b), false);      // zero width space is not whitespace
  equal(isSpace(0x41), false);
});

test('controls, punctuation, and symbols', () => {
  for (const code of [0, 0x1f, 0x7f, 0x9f]) equal(isControl(code), true, 'control ' + code);
  equal(isControl(0x20), false);
  equal(isPunc(0x21), true);          // !
  equal(isPunc(0x41), false);
  equal(isSymbol(0x1f600), true);     // emoji
  equal(isSymbol(0x41), false);
});"#);
}

#[test]
fn at_variants_use_utf16_indices_and_reject_invalid_arguments() {
    run(r#"import test from 'neo:test';
import { equal, throws } from 'neo:assert';
import {
  isAsciiAt, isLatin1At, isLetterAt, isSpaceAt, isSymbolAt,
} from 'neo:chars';

test('astral characters occupy two UTF-16 units', () => {
  const text = '😀x';
  equal(text.length, 3);
  equal(isSymbolAt(text, 0), true);
  equal(isLetterAt(text, 0), false);
  equal(isSymbolAt(text, 1), false);  // trailing surrogate
  equal(isLetterAt(text, 2), true);
  equal(isAsciiAt(text, 2), true);
  equal(isAsciiAt(text, 1), false);
});

test('missing and out-of-range indexes are false', () => {
  equal(isAsciiAt('', 0), false);
  equal(isSpaceAt('', 0), false);
  equal(isAsciiAt('abc', 3), false);
  equal(isAsciiAt('abc', 99), false);
  equal(isSpaceAt('a b', 1), true);
  equal(isLatin1At('é', 0), true);
  equal(isAsciiAt('é', 0), false);
});

test('invalid text and indexes throw TypeError', () => {
  throws(() => isAsciiAt(65, 0), TypeError);
  throws(() => isAsciiAt('a', '0'), TypeError);
  throws(() => isAsciiAt('a', -1), TypeError);
  throws(() => isAsciiAt('a', 0.5), TypeError);
  throws(() => isAsciiAt('a', NaN), TypeError);
});"#);
}

#[test]
fn case_conversion_stays_within_one_code_point() {
    run(r#"import test from 'neo:test';
import { equal, throws } from 'neo:assert';
import { simpleFold, toLower, toUpper } from 'neo:chars';

test('simple mappings are applied', () => {
  equal(toLower(0x41), 0x61);
  equal(toLower(0x61), 0x61);
  equal(toUpper(0x61), 0x41);
  equal(toUpper(0xe9), 0xc9);
  equal(toUpper(0x10428), 0x10400);   // Deseret, one astral code point
  equal(toLower(0x10400), 0x10428);
});

test('expanding mappings leave the input unchanged', () => {
  equal(toUpper(0xdf), 0xdf);         // sharp s uppercases to SS
  equal(toLower(0x130), 0x130);       // I with dot lowercases to i plus a mark
  equal(toLower(0x1e9e), 0xdf);       // capital sharp s has a one-point mapping
});

test('simpleFold flips when a mapping exists', () => {
  equal(simpleFold(0x61), 0x41);
  equal(simpleFold(0x41), 0x61);
  equal(simpleFold(0x30), 0x30);
  equal(simpleFold(0xdf), 0xdf);
});

test('invalid values throw', () => {
  throws(() => toLower('A'), TypeError);
  throws(() => toLower(0xd800), RangeError);
  throws(() => toUpper(0x110000), RangeError);
  throws(() => simpleFold(1.5), RangeError);
});"#);
}

#[test]
fn equal_fold_is_a_transitive_simple_fold() {
    run(r#"import test from 'neo:test';
import { equal, throws } from 'neo:assert';
import { equalFold } from 'neo:chars';

test('common folds match', () => {
  equal(equalFold(0x41, 0x61), true);       // A / a
  equal(equalFold(0x3c2, 0x3c3), true);     // final sigma / sigma
  equal(equalFold(0x212a, 0x6b), true);     // Kelvin sign / k
  equal(equalFold(0x17f, 0x73), true);      // long s / s
  equal(equalFold(0xb5, 0x3bc), true);      // micro sign / Greek mu
  equal(equalFold(0x1e9e, 0xdf), true);     // capital sharp s / sharp s
  equal(equalFold(0x41, 0x42), false);
});

test('the Greek theta class is transitive', () => {
  // These four were not an equivalence class when folding compared
  // toLowerCase and toUpperCase separately.
  equal(equalFold(0x398, 0x3f4), true);     // capital theta / capital theta symbol
  equal(equalFold(0x398, 0x3d1), true);     // capital theta / theta symbol
  equal(equalFold(0x3f4, 0x3d1), true);     // capital theta symbol / theta symbol
  const theta = [0x398, 0x3b8, 0x3d1, 0x3f4];
  for (const a of theta) {
    for (const b of theta) {
      equal(equalFold(a, b), true, 'theta ' + a + '/' + b);
      equal(equalFold(a, b), equalFold(b, a), 'symmetric ' + a + '/' + b);
    }
  }
});

test('every pair in a documented class agrees', () => {
  const groups = [
    [0x41, 0x61],
    [0x3c2, 0x3c3],
    [0x212a, 0x6b],
    [0x17f, 0x73],
    [0xb5, 0x3bc],
    [0x1e9e, 0xdf],
  ];
  for (const group of groups) {
    for (const a of group) {
      for (const b of group) {
        equal(equalFold(a, b), true, 'class ' + a + '/' + b);
        equal(equalFold(a, b), equalFold(b, a), 'symmetric ' + a + '/' + b);
      }
    }
  }
});

test('an expanding case mapping is not applied', () => {
  equal(equalFold(0xdf, 0x73), false);      // sharp s / s
});

test('invalid values throw', () => {
  throws(() => equalFold(0x41, 'a'), TypeError);
  throws(() => equalFold(0x41, 0xd800), RangeError);
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
        sqlite: true,
    })
    .unwrap();
    js.run_module(
        "main.js",
        "import { isLetter, toUpper } from 'neo:chars';
         if (!isLetter(0x416) || toUpper(0x416) !== 0x416) throw Error('chars failed');",
    )
    .unwrap();
}
