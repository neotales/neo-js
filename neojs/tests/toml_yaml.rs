//! End-to-end tests for `neo:toml` and `neo:yaml` through the CLI.
//!
//! Both parsers must work with no permission flags and must not acquire file or
//! environment access. Reading a file is a separate `--allow-fs` decision.

use std::{
    path::Path,
    process::{Command, Output},
};

fn run_cli(dir: &Path, permissions: &[&str], name: &str, source: &str) -> Output {
    let entry = dir.join(name);
    std::fs::write(&entry, source).unwrap();
    Command::new(env!("CARGO_BIN_EXE_neojs"))
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

const STRUCTURED_SCRIPT: &str = r#"import { parse as toml, TomlParseError } from 'neo:toml';
import { parse as yaml, YamlParseError } from 'neo:yaml';

const config = toml('title = "héllo"\n[server]\nports = [80, 443]\n');
if (Object.getPrototypeOf(config) !== null) throw Error('toml prototype');
if (config.title !== 'héllo' || config.server.ports[1] !== 443) throw Error('toml values');

const doc = yaml('title: héllo\nserver:\n  ports: [80, 443]\n');
if (Object.getPrototypeOf(doc) !== null) throw Error('yaml prototype');
if (doc.title !== 'héllo' || doc.server.ports[1] !== 443) throw Error('yaml values');

let tomlError = false;
try {
  toml('a = 1\na = 2\n');
} catch (error) {
  if (!(error instanceof TomlParseError) || error.code !== 'TOML_SYNTAX')
    throw Error('toml error');
  tomlError = true;
}
if (!tomlError) throw Error('expected TOML error');

let yamlError = false;
try {
  yaml('a: 1\na: 2\n');
} catch (error) {
  if (!(error instanceof YamlParseError) || error.code !== 'YAML_DUPLICATE')
    throw Error('yaml error');
  yamlError = true;
}
if (!yamlError) throw Error('expected YAML error');

console.log('structured-ok');
"#;

#[test]
fn structured_parsers_work_without_permissions() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(dir.path(), &[], "main.js", STRUCTURED_SCRIPT);
    assert_ok(&output);
    assert_eq!(output.stdout, b"structured-ok\n");
}

#[test]
fn structured_parsers_are_identical_with_every_permission_enabled() {
    let dir = tempfile::tempdir().unwrap();
    let disabled = run_cli(dir.path(), &[], "main.js", STRUCTURED_SCRIPT);
    assert_ok(&disabled);

    let enabled = run_cli(
        dir.path(),
        &[
            "--allow-fs",
            "--allow-env",
            "--allow-run",
            "--allow-registry",
        ],
        "main.js",
        STRUCTURED_SCRIPT,
    );
    assert_ok(&enabled);
    assert_eq!(disabled.stdout, enabled.stdout);
}

#[test]
fn structured_parsers_expose_no_file_or_environment_loader() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        "main.js",
        "import * as toml from 'neo:toml';
         import * as yaml from 'neo:yaml';
         if (typeof fs !== 'undefined' || typeof env !== 'undefined')
           throw Error('unrequested capability');
         for (const module of [toml, yaml]) {
           for (const name of ['load', 'parseFile', 'readFile']) {
             if (name in module) throw Error('unexpected export: ' + name);
           }
         }
         console.log('no-io');",
    );
    assert_ok(&output);
    assert_eq!(output.stdout, b"no-io\n");

    let denied = run_cli(
        dir.path(),
        &[],
        "denied.js",
        "import { readText } from 'neo:fs'; console.log(readText('x'));",
    );
    assert!(!denied.status.success());
}

#[test]
fn malformed_input_reports_a_code_and_never_the_value() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        "main.js",
        r#"import { parse } from 'neo:toml';
           try {
             parse('password = "supersecret"\nBROKEN = [\n');
           } catch (error) {
             throw Error(error.code + ':' + error.line + ':' + error.column);
           }"#,
    );
    assert!(!output.status.success());
    let message = stderr(&output);
    assert!(message.contains("TOML_SYNTAX"), "{message}");
    assert!(!message.contains("supersecret"), "{message}");

    let output = run_cli(
        dir.path(),
        &[],
        "yaml.js",
        r#"import { parse } from 'neo:yaml';
           try {
             parse('password: supersecret\nbroken: [\n');
           } catch (error) {
             throw Error(error.code + ':' + error.line + ':' + error.column);
           }"#,
    );
    assert!(!output.status.success());
    let message = stderr(&output);
    assert!(message.contains("YAML_SYNTAX"), "{message}");
    assert!(!message.contains("supersecret"), "{message}");
}

#[test]
fn typescript_entry_can_import_both_parsers() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        "main.ts",
        r#"import { parse as toml } from 'neo:toml';
import { parse as yaml } from 'neo:yaml';

interface TomlConfig { title: string; }
interface YamlConfig { title: string; }

const t = toml('title = "toml"\n') as unknown as TomlConfig;
const y = yaml('title: yaml\n') as unknown as YamlConfig;
const title: string = t.title + ' ' + y.title;
console.log(title);
"#,
    );
    assert_ok(&output);
    assert_eq!(output.stdout, b"toml yaml\n");
}
