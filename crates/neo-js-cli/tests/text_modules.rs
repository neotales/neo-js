//! End-to-end tests for the pure text modules through the CLI.
//!
//! `neo:chars`, `neo:string-builder`, and `neo:ansi` need no permission flag.
//! These tests run them with none, then again with every flag enabled, and
//! require the same output.

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

const TEXT_SCRIPT: &str = r#"import { equal } from 'neo:assert';
import { equalFold, isAscii, isLetter, isSymbol } from 'neo:chars';
import { StringBuilder } from 'neo:string-builder';
import { red, stripAnsi } from 'neo:ansi';

equal(isAscii(65), true);
equal(isAscii(0x80), false);
equal(isLetter(0x416), true);
equal(isSymbol(0x1f600), true);
equal(equalFold(0x398, 0x3f4), true);
equal(equalFold(0x3f4, 0x3d1), true);
equal(equalFold(0x398, 0x3d1), true);

const builder = new StringBuilder();
builder.append('Hi').append(1);
equal(builder.toString(), 'Hi1');
equal(builder.length, 3);
builder.clear().append('again');
equal(builder.toString(), 'again');

equal(red('x'), '\x1b[31mx\x1b[39m');
equal(red('x', { color: false }), 'x');
equal(stripAnsi(red('x')), 'x');
equal(stripAnsi('a\u009b31mb'), 'ab');
equal(stripAnsi('\x1b]8;;url\u009clabel'), 'label');
console.log('ok');
"#;

#[test]
fn cli_imports_text_modules_without_permissions() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(dir.path(), &[], "main.js", TEXT_SCRIPT);
    assert_ok(&output);
    assert_eq!(output.stdout, b"ok\n");
}

#[test]
fn text_modules_behave_the_same_with_every_permission_enabled() {
    let dir = tempfile::tempdir().unwrap();
    let disabled = run_cli(dir.path(), &[], "main.js", TEXT_SCRIPT);
    assert_ok(&disabled);

    let enabled = run_cli(
        dir.path(),
        &[
            "--allow-fs",
            "--allow-env",
            "--allow-run",
            "--allow-registry",
        ],
        "main-permitted.js",
        TEXT_SCRIPT,
    );
    assert_ok(&enabled);

    assert_eq!(disabled.stdout, enabled.stdout);
}

#[test]
fn invalid_builder_input_surfaces_as_type_error() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        "caught.js",
        r#"import { StringBuilder } from 'neo:string-builder';
try {
  new StringBuilder().append({});
  console.log('no-throw');
} catch (error) {
  console.log(error.name);
}
"#,
    );
    assert_ok(&output);
    assert_eq!(output.stdout, b"TypeError\n");

    let uncaught = run_cli(
        dir.path(),
        &[],
        "uncaught.js",
        "import { StringBuilder } from 'neo:string-builder';\nnew StringBuilder().append({});\n",
    );
    assert!(!uncaught.status.success());
    assert!(
        stderr(&uncaught).contains("value must be a string, number, boolean, or bigint"),
        "{}",
        stderr(&uncaught)
    );
}

#[test]
fn typescript_entry_can_use_the_text_modules() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        "main.ts",
        r#"import { isDigit } from 'neo:chars';
import { StringBuilder } from 'neo:string-builder';
import { bold } from 'neo:ansi';

const amount: number = 42;
const text: string = new StringBuilder().append('n=').append(amount).toString();
if (!isDigit(0x34)) throw new Error('digit');
console.log(bold(text, { color: false }));
"#,
    );
    assert_ok(&output);
    assert_eq!(output.stdout, b"n=42\n");
}
