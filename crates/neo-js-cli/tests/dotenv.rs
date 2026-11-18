//! End-to-end tests for `neo:dotenv` and `neo:env.has` through the CLI.
//!
//! `neo:dotenv` must work with no permission flags and must not acquire file or
//! environment access. These tests also prove the file gate is real: there is no
//! dotenv file loader, so a script needs `--allow-fs` plus `neo:fs.readText`.

use std::{
    path::Path,
    process::{Command, Output},
};

fn run_cli(dir: &Path, permissions: &[&str], source: &str) -> Output {
    let entry = dir.join("main.js");
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

const DOTENV_SCRIPT: &str = r#"import { parse, DotenvParseError } from 'neo:dotenv';
const parsed = parse('__proto__=safe\nA=1\nA=2\nB="two words"\nC=$HOME');
if (Object.getPrototypeOf(parsed) !== null) throw Error('prototype');
if (parsed.__proto__ !== 'safe' || parsed.A !== '2') throw Error('values');
if (parsed.B !== 'two words' || parsed.C !== '$HOME') throw Error('literal');
try {
  parse('A="unterminated');
  throw Error('expected a throw');
} catch (error) {
  if (!(error instanceof DotenvParseError)) throw Error('type');
  if (error.code !== 'DOTENV_QUOTE' || error.line !== 1 || error.column !== 3) {
    throw Error('error fields');
  }
}
console.log('dotenv-ok');
"#;

#[test]
fn dotenv_works_without_permissions() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(dir.path(), &[], DOTENV_SCRIPT);
    assert_ok(&output);
    assert_eq!(output.stdout, b"dotenv-ok\n");
}

#[test]
fn dotenv_is_identical_with_every_permission_enabled() {
    let dir = tempfile::tempdir().unwrap();
    let disabled = run_cli(dir.path(), &[], DOTENV_SCRIPT);
    assert_ok(&disabled);

    let enabled = run_cli(
        dir.path(),
        &[
            "--allow-fs",
            "--allow-env",
            "--allow-run",
            "--allow-registry",
        ],
        DOTENV_SCRIPT,
    );
    assert_ok(&enabled);
    assert_eq!(disabled.stdout, enabled.stdout);
}

#[test]
fn dotenv_exposes_no_file_or_environment_loader() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        "import * as dotenv from 'neo:dotenv';
         for (const name of ['load', 'parseFile', 'readFile']) {
           if (name in dotenv) throw Error('unexpected export: ' + name);
         }
         console.log('no-io');",
    );
    assert_ok(&output);
    assert_eq!(output.stdout, b"no-io\n");
}

#[test]
fn env_module_requires_allow_env() {
    let dir = tempfile::tempdir().unwrap();
    let denied = run_cli(
        dir.path(),
        &[],
        "import { has } from 'neo:env'; console.log(String(has('PATH')));",
    );
    assert!(!denied.status.success());

    let allowed = run_cli(
        dir.path(),
        &["--allow-env"],
        "import { get, has } from 'neo:env';
         if (has('NEO_JS_TEST_SURELY_ABSENT_VARIABLE_7C1F') !== false) throw Error('absent');
         if (env.has('NEO_JS_TEST_SURELY_ABSENT_VARIABLE_7C1F') !== false) throw Error('global');
         if (get('NEO_JS_TEST_SURELY_ABSENT_VARIABLE_7C1F') !== undefined) throw Error('get');
         if (env.get('NEO_JS_TEST_SURELY_ABSENT_VARIABLE_7C1F') !== undefined) throw Error('global get');
         console.log('env-ok');",
    );
    assert_ok(&allowed);
    assert_eq!(allowed.stdout, b"env-ok\n");
}

#[test]
fn malformed_input_reports_a_code_and_never_the_value() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        r#"import { parse } from 'neo:dotenv';
           try {
             parse('PASSWORD=supersecret\nBROKEN');
           } catch (error) {
             throw Error(error.code + ':' + error.line + ':' + error.column);
           }"#,
    );
    assert!(!output.status.success());
    let message = stderr(&output);
    assert!(message.contains("DOTENV_SYNTAX:2:7"), "{message}");
    assert!(!message.contains("supersecret"), "{message}");
}
