//! Integration tests for `neo:ansi` through the public `NeoJs` API.
//!
//! The module needs no permission flag, does no terminal detection, and keeps
//! no module-global enabled flag. Every color decision is explicit.

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
fn style_functions_emit_the_documented_sequences() {
    run(r#"import test from 'neo:test';
import { equal } from 'neo:assert';
import {
  bgBlue, bgBrightWhite, bold, dim, hidden, inverse, italic, red, reset,
  strikethrough, underline,
} from 'neo:ansi';

test('attributes', () => {
  equal(reset('x'), '\x1b[0mx\x1b[0m');
  equal(bold('x'), '\x1b[1mx\x1b[22m');
  equal(dim('x'), '\x1b[2mx\x1b[22m');
  equal(italic('x'), '\x1b[3mx\x1b[23m');
  equal(underline('x'), '\x1b[4mx\x1b[24m');
  equal(inverse('x'), '\x1b[7mx\x1b[27m');
  equal(hidden('x'), '\x1b[8mx\x1b[28m');
  equal(strikethrough('x'), '\x1b[9mx\x1b[29m');
});

test('colors', () => {
  equal(red('x'), '\x1b[31mx\x1b[39m');
  equal(bgBlue('x'), '\x1b[44mx\x1b[49m');
  equal(bgBrightWhite('x'), '\x1b[107mx\x1b[49m');
});

test('empty text still gets a pair of sequences', () => {
  equal(red(''), '\x1b[31m\x1b[39m');
});"#);
}

#[test]
fn color_false_is_explicit_and_has_no_hidden_state() {
    run(r#"import test from 'neo:test';
import { equal, throws } from 'neo:assert';
import { apply, blue, bold, red, rgb24 } from 'neo:ansi';

test('the option returns the input unchanged', () => {
  equal(red('x', { color: false }), 'x');
  equal(bold('x', { color: false }), 'x');
  equal(rgb24('x', 0x112233, { color: false }), 'x');
  equal(apply('x', [bold, blue], { color: false }), 'x');
});

test('an explicit true or an omitted option still emits', () => {
  equal(red('x'), '\x1b[31mx\x1b[39m');
  equal(red('x', { color: true }), '\x1b[31mx\x1b[39m');
  equal(red('x', {}), '\x1b[31mx\x1b[39m');
});

test('a false call does not disable later calls', () => {
  equal(red('a', { color: false }), 'a');
  equal(red('a'), '\x1b[31ma\x1b[39m');
});

test('invalid options throw', () => {
  throws(() => red('x', null), TypeError);
  throws(() => red('x', 'off'), TypeError);
  throws(() => red('x', { color: 'off' }), TypeError);
  throws(() => red(5), TypeError);
});"#);
}

#[test]
fn an_embedded_close_sequence_is_reopened() {
    run(r#"import test from 'neo:test';
import { equal } from 'neo:assert';
import { red, stripAnsi } from 'neo:ansi';

test('the same style is reopened', () => {
  equal(red('a\x1b[39mb'), '\x1b[31ma\x1b[31mb\x1b[39m');
  equal(stripAnsi(red('a\x1b[39mb')), 'ab');
});

test('an unrelated sequence is left alone', () => {
  equal(red('\x1b[32mz'), '\x1b[31m\x1b[32mz\x1b[39m');
  equal(stripAnsi(red('\x1b[32mz')), 'z');
});

test('a full reset is passed through, not restored', () => {
  // Only the style's own close code is reopened. ESC[0m resets everything.
  equal(red('a\x1b[0mb'), '\x1b[31ma\x1b[0mb\x1b[39m');
  equal(stripAnsi(red('a\x1b[0mb')), 'ab');
});"#);
}

#[test]
fn apply_validates_then_nests_left_to_right() {
    run(r#"import test from 'neo:test';
import { equal, throws } from 'neo:assert';
import { apply, blue, bold, dim } from 'neo:ansi';

test('the last style is outermost', () => {
  equal(apply('x', [bold, blue]), '\x1b[34m\x1b[1mx\x1b[22m\x1b[39m');
  equal(apply('x', [bold]), '\x1b[1mx\x1b[22m');
  equal(apply('x', []), 'x');
});

test('invalid styles are rejected even when color is disabled', () => {
  throws(() => apply('x', bold), TypeError);
  throws(() => apply('x', [bold, 1]), TypeError);
  throws(() => apply(1, [bold]), TypeError);
  throws(() => apply('x', [bold], { color: 'no' }), TypeError);
  throws(() => apply('x', [1], { color: false }), TypeError);
});

test('a style function is not mutated by apply', () => {
  equal(dim('x'), '\x1b[2mx\x1b[22m');
  equal(apply('y', [dim]), '\x1b[2my\x1b[22m');
  equal(dim('x'), '\x1b[2mx\x1b[22m');
});"#);
}

#[test]
fn extended_colors_clamp_object_channels_and_range_check_integers() {
    run(r#"import test from 'neo:test';
import { equal, throws } from 'neo:assert';
import { bgRgb24, bgRgb8, rgb8, rgb24 } from 'neo:ansi';

test('palette indexes are truncated and clamped', () => {
  equal(rgb8('x', 208), '\x1b[38;5;208mx\x1b[39m');
  equal(rgb8('x', 300), '\x1b[38;5;255mx\x1b[39m');
  equal(rgb8('x', -1), '\x1b[38;5;0mx\x1b[39m');
  equal(rgb8('x', 3.9), '\x1b[38;5;3mx\x1b[39m');
  equal(bgRgb8('x', 9), '\x1b[48;5;9mx\x1b[49m');
});

test('true color accepts a 24-bit integer', () => {
  equal(rgb24('x', 0x112233), '\x1b[38;2;17;34;51mx\x1b[39m');
  equal(rgb24('x', 0), '\x1b[38;2;0;0;0mx\x1b[39m');
  equal(bgRgb24('x', 0xffffff), '\x1b[48;2;255;255;255mx\x1b[49m');
});

test('object channels are truncated and clamped', () => {
  equal(rgb24('x', { r: 300, g: 1.9, b: -4 }), '\x1b[38;2;255;1;0mx\x1b[39m');
});

test('invalid colors throw', () => {
  throws(() => rgb8('x', NaN), RangeError);
  throws(() => rgb8('x', Infinity), RangeError);
  throws(() => rgb8('x', '3'), TypeError);
  throws(() => rgb24('x', 0x1000000), RangeError);
  throws(() => rgb24('x', 1.5), RangeError);
  throws(() => rgb24('x', { r: 1, g: 2 }), TypeError);
  throws(() => rgb24('x', { r: NaN, g: 2, b: 3 }), RangeError);
  throws(() => rgb24('x', null), TypeError);
});"#);
}

#[test]
fn strip_ansi_removes_common_sequences_and_preserves_text() {
    run(r#"import test from 'neo:test';
import { equal } from 'neo:assert';
import { red, stripAnsi } from 'neo:ansi';

test('SGR, 256-color, and cursor CSI', () => {
  equal(stripAnsi('\x1b[31merror\x1b[39m'), 'error');
  equal(stripAnsi('\x1b[38;5;208mwarm\x1b[39m'), 'warm');
  equal(stripAnsi('a\x1b[2Kb'), 'ab');
  equal(stripAnsi(red('text')), 'text');
});

test('OSC hyperlinks terminated by BEL or ST', () => {
  equal(stripAnsi('\x1b]8;;https://example.test\x07link\x1b]8;;\x07'), 'link');
  equal(stripAnsi('\x1b]8;;https://example.test\x1b\\link\x1b]8;;\x1b\\'), 'link');
});

test('C1 ST terminates control strings and keeps following text', () => {
  equal(stripAnsi('\x1b]8;;https://example.test\u009clabel'), 'label');
  equal(stripAnsi('\x1bPpayload\u009cafter'), 'after');
  equal(stripAnsi('\x1bXpayload\u009cafter'), 'after');
  equal(stripAnsi('\x1b]8;;url\u009c'), '');
});

test('single-byte C1 CSI', () => {
  equal(stripAnsi('a\u009b31mb'), 'ab');
  equal(stripAnsi('\u009b2K'), '');
  equal(stripAnsi('a\u009b31'), 'a');       // unterminated CSI
});

test('ESC c and Fe two-byte escapes', () => {
  equal(stripAnsi('a\x1bcb'), 'ab');        // RIS
  equal(stripAnsi('\x1bc'), '');
  equal(stripAnsi('a\x1bMb'), 'ab');        // RI, a two-byte Fe escape
});

test('DCS and unterminated control strings', () => {
  equal(stripAnsi('a\x1bP1;2|payload\x1b\\b'), 'ab');
  equal(stripAnsi('a\x1b]8;;unterminated'), 'a');
});

test('plain text, other controls, and unrecognized escapes are preserved', () => {
  equal(stripAnsi('plain'), 'plain');
  equal(stripAnsi('a\x1bb'), 'a\x1bb');
  equal(stripAnsi('a\u0085b'), 'a\u0085b');
  equal(stripAnsi(''), '');
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
        "import { red, stripAnsi } from 'neo:ansi';
         if (red('x') !== '\\x1b[31mx\\x1b[39m') throw Error('ansi failed');
         if (stripAnsi(red('x')) !== 'x') throw Error('strip failed');",
    )
    .unwrap();
}
