//! End-to-end tests for the synchronous `neo:process` stream and directory API.
//!
//! The CLI is run with a temporary directory as its working directory. Tests
//! that read stdin use an explicit pipe and close it, so end of file is
//! reached without an interactive terminal or a timeout.

use std::{
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

fn run_cli(dir: &Path, permissions: &[&str], source: &str) -> Output {
    let entry = dir.join("main.js");
    std::fs::write(&entry, source).unwrap();
    Command::new(env!("CARGO_BIN_EXE_neojs"))
        .current_dir(dir)
        .stdin(Stdio::null())
        .args(permissions)
        .arg(&entry)
        .output()
        .unwrap()
}

fn run_cli_with_input(dir: &Path, permissions: &[&str], source: &str, input: &[u8]) -> Output {
    let entry = dir.join("main.js");
    std::fs::write(&entry, source).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_neojs"))
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .args(permissions)
        .arg(&entry)
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}

fn assert_cli_ok(output: &Output) {
    assert!(
        output.status.success(),
        "status: {:?}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Run `source` in `dir` and decode the initial `cwd()` it reports as a JSON
/// string on the first line of stdout.
///
/// The string is compared by path identity, not text: Windows may name one
/// directory with short (8.3) components, a verbatim `\\?\` prefix, or another
/// alias, and `cwd()` reports the spelling the host process was started with,
/// which is not guaranteed to equal the exact input.
fn run_cli_cwd_report(dir: &Path, permissions: &[&str], source: &str) -> (String, String) {
    let output = run_cli(dir, permissions, source);
    assert_cli_ok(&output);
    let stdout = std::str::from_utf8(&output.stdout).unwrap();
    let (reported, rest) = stdout
        .split_once('\n')
        .unwrap_or_else(|| panic!("stdout has no reported cwd line: {stdout:?}"));
    let reported = serde_json::from_str(reported).unwrap();
    (reported, rest.to_owned())
}

#[test]
fn streams_copy_binary_stdin_to_separate_stdout_and_stderr() {
    let dir = tempfile::tempdir().unwrap();
    let source = r#"import {stdin, stdout, stderr} from 'neo:process';
const buffer = new Uint8Array(2);
const chunks = [];
while (true) {
  const count = stdin.readSync(buffer);
  if (count === null) break;
  if (count > 0) chunks.push(buffer.slice(0, count));
}
const size = chunks.reduce((total, chunk) => total + chunk.length, 0);
const all = new Uint8Array(size);
let offset = 0;
for (const chunk of chunks) { all.set(chunk, offset); offset += chunk.length; }
stdout.writeSync('out:');
stdout.write(all);
stdout.writeSync('\n');
stderr.writeSync('err:');
stderr.write(all);
stderr.writeSync('\n');
"#;
    let input = [0_u8, 255, 10, 65, 13, 0];
    let output = run_cli_with_input(dir.path(), &["--allow-stdio"], source, &input);
    assert_cli_ok(&output);
    assert_eq!(output.stdout, b"out:\x00\xff\nA\r\x00\n");
    assert_eq!(output.stderr, b"err:\x00\xff\nA\r\x00\n");
}

#[test]
fn process_is_denied_by_default_and_has_no_output_side_effect() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        "import {stdout} from 'neo:process';\nstdout.writeSync('leak');",
    );
    assert!(!output.status.success());
    assert!(output.stdout.is_empty(), "{:?}", output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("neo:process"), "{stderr}");

    let output = run_cli(
        dir.path(),
        &["--allow-fs", "--allow-env"],
        "import 'neo:process';",
    );
    assert!(!output.status.success());
}

#[test]
fn process_permission_also_reads_piped_stdin() {
    let dir = tempfile::tempdir().unwrap();
    let source = "import {stdin, stdout} from 'neo:process';
                  const buffer = new Uint8Array(16);
                  const count = stdin.readSync(buffer);
                  if (count === null) throw Error('unexpected eof');
                  stdout.write(buffer.subarray(0, count));
                  stdout.writeSync('\\n');";
    let output = run_cli_with_input(dir.path(), &["--allow-run"], source, b"run-stdin");
    assert_cli_ok(&output);
    assert_eq!(output.stdout, b"run-stdin\n");
}

#[test]
fn invalid_stream_and_directory_inputs_throw_without_side_effects() {
    let dir = tempfile::tempdir().unwrap();
    let source = r#"import {stdin, stdout, stderr, cwd, chdir, ChangeDirectoryError} from 'neo:process';
for (const value of [42, null, undefined, {}, []]) {
  let rejected = false;
  try { stdout.write(value); }
  catch (error) { if (!(error instanceof TypeError)) throw error; rejected = true; }
  if (!rejected) throw Error('write accepted ' + String(value));
}
let rejected = false;
try { stdin.read('not bytes'); }
catch (error) { if (!(error instanceof TypeError)) throw error; rejected = true; }
if (!rejected) throw Error('read accepted a string');
rejected = false;
try { chdir(42); }
catch (error) { if (!(error instanceof TypeError)) throw error; rejected = true; }
if (!rejected) throw Error('chdir accepted a number');
for (const directory of ['', 'neo-js-no-such-directory']) {
  rejected = false;
  try { chdir(directory); }
  catch (error) {
    if (!(error instanceof ChangeDirectoryError)) throw error;
    if (error.code !== 'PROCESS_CHDIR') throw error;
    rejected = true;
  }
  if (!rejected) throw Error('chdir accepted ' + directory);
}
const before = cwd();
if (typeof before !== 'string' || before.length === 0) throw Error('cwd');
if (typeof stdout.isTerm() !== 'boolean' || typeof stdin.isTerm() !== 'boolean') throw Error('isTerm');
stdout.close();
stderr.close();
stdin.close();
stdout.writeSync('ok\n');
"#;
    let output = run_cli(dir.path(), &["--allow-stdio"], source);
    assert_cli_ok(&output);
    assert_eq!(output.stdout, b"ok\n");
}

#[test]
fn cwd_and_chdir_are_runtime_local_and_coordinate_with_filesystem() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("a")).unwrap();
    std::fs::create_dir(dir.path().join("b")).unwrap();
    std::fs::write(dir.path().join("a/data.txt"), "A").unwrap();
    std::fs::write(dir.path().join("b/data.txt"), "B").unwrap();
    std::fs::write(dir.path().join("marker.txt"), "initial").unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let a = std::fs::canonicalize(dir.path().join("a")).unwrap();
    let b = std::fs::canonicalize(dir.path().join("b")).unwrap();

    let source = format!(
        r#"import {{cwd, chdir, ChangeDirectoryError, stdout}} from 'neo:process';
import {{readText}} from 'neo:fs';
// The initial cwd() is the host-reported spelling of the process directory: on
// Windows it may use short (8.3) components while canonicalize() returns a
// verbatim path, and the OS may normalize it, so it is not guaranteed to equal
// the exact path supplied to the process. The initial value is therefore
// checked by resolving a relative read here and by path identity in the
// parent. chdir() stores a canonical path, so the comparisons after it remain
// exact.
if (readText('marker.txt') !== 'initial') throw Error('initial cwd ' + cwd());
stdout.writeSync(JSON.stringify(cwd()) + '\n');
chdir('a');
if (cwd() !== {a}) throw Error('a cwd ' + cwd());
if (readText('data.txt') !== 'A') throw Error('a data');
chdir('../b');
if (cwd() !== {b}) throw Error('b cwd ' + cwd());
if (readText('data.txt') !== 'B') throw Error('b data');
let rejected = false;
try {{ chdir('../missing'); }}
catch (error) {{
  if (!(error instanceof ChangeDirectoryError)) throw error;
  if (error.code !== 'PROCESS_CHDIR') throw error;
  rejected = true;
}}
if (!rejected) throw Error('missing directory accepted');
if (cwd() !== {b}) throw Error('failed chdir changed cwd');
chdir({root});
if (readText('a/data.txt') !== 'A') throw Error('root data');
console.log('ok');
"#,
        a = serde_json::to_string(a.to_str().unwrap()).unwrap(),
        b = serde_json::to_string(b.to_str().unwrap()).unwrap(),
        root = serde_json::to_string(root.to_str().unwrap()).unwrap(),
    );
    let (initial, rest) = run_cli_cwd_report(dir.path(), &["--allow-fs", "--allow-stdio"], &source);
    assert!(
        Path::new(&initial).is_absolute(),
        "initial cwd is not absolute: {initial:?}"
    );
    assert!(
        same_file::is_same_file(&initial, dir.path()).unwrap(),
        "initial cwd {initial:?} does not name the host directory {:?}",
        dir.path()
    );
    assert_eq!(rest, "ok\n");
}

/// Windows can spell one directory with short (8.3) components, with a
/// verbatim `\\?\` prefix, or with the long components the environment
/// normally uses. `cwd()` reports the spelling the host process was started
/// with, and the OS may normalize that spelling, so it is not guaranteed to
/// equal the exact input. This starts the CLI from the temporary directory as
/// the environment spells it (short when the host's `TEMP` uses 8.3 names;
/// this test neither requires nor constructs one) and from its canonical
/// verbatim spelling. Both runs are compared by directory identity.
#[cfg(windows)]
#[test]
fn cwd_names_the_host_directory_for_host_reported_and_verbatim_spellings() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("marker.txt"), "initial").unwrap();
    let verbatim = std::fs::canonicalize(dir.path()).unwrap();
    assert!(
        verbatim.to_str().is_some(),
        "the canonical temporary path is not Unicode"
    );

    let source = "import {cwd, stdout} from 'neo:process';\n\
                  import {readText} from 'neo:fs';\n\
                  if (readText('marker.txt') !== 'initial') throw Error('initial cwd');\n\
                  stdout.writeSync(JSON.stringify(cwd()) + '\\n');";
    for (spelling, working) in [
        ("host-reported", dir.path().to_path_buf()),
        ("verbatim", verbatim),
    ] {
        let (reported, rest) =
            run_cli_cwd_report(&working, &["--allow-fs", "--allow-stdio"], source);
        assert_eq!(rest, "");
        assert!(
            same_file::is_same_file(&reported, dir.path()).unwrap(),
            "{spelling} cwd {reported:?} from {working:?} does not name {:?}",
            dir.path()
        );
    }
}

#[cfg(unix)]
#[test]
fn exec_child_uses_the_runtime_working_directory() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("a")).unwrap();
    let source = "import {cwd, chdir} from 'neo:process';
                  import {exec} from 'neo:exec';
                  chdir('a');
                  const result = exec('sh', ['-c', 'pwd']);
                  if (!result.success) throw Error('exec failed');
                  const printed = String.fromCharCode(...result.stdout).trim();
                  if (printed !== cwd()) throw Error('child cwd ' + printed + ' != ' + cwd());
                  console.log('ok');";
    let output = run_cli(dir.path(), &["--allow-run"], source);
    assert_cli_ok(&output);
    assert_eq!(output.stdout, b"ok\n");
}
