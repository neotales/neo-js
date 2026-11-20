//! End-to-end tests for the synchronous `neo:exec` process capability.
//!
//! Each test writes its entry script into a temporary directory and runs the
//! CLI with that directory as the child's working directory. The host process
//! working directory is never changed, so tests can run in parallel.

use std::{
    path::Path,
    process::{Command, Output},
};

use serde_json::to_string;

/// Build a module that runs `program` through `neo:exec` and then executes
/// `body` with the bound `input` and `result` values.
fn exec_script(program: &str, args: &[String], input: &[u8], body: &str) -> String {
    format!(
        "import {{ exec }} from 'neo:exec';\n\
         const input = new Uint8Array({input});\n\
         const result = exec({program}, {args}, input);\n\
         {body}\n",
        input = to_string(input).unwrap(),
        program = to_string(program).unwrap(),
        args = to_string(args).unwrap(),
    )
}

/// Run the CLI on `source` in `dir`, with the supplied permission flags.
fn run_cli(dir: &Path, permissions: &[&str], source: &str) -> Output {
    let entry = dir.join("main.js");
    std::fs::write(&entry, source).unwrap();
    Command::new(env!("CARGO_BIN_EXE_neojs"))
        .current_dir(dir)
        .args(permissions)
        .arg(&entry)
        .output()
        .unwrap()
}

fn assert_cli_ok(output: &Output) {
    assert!(
        output.status.success(),
        "status: {:?}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A program path and arguments for the CLI itself. Running it as a child
/// exercises direct program invocation without depending on a platform shell.
fn neo_js_child(script: &Path) -> (String, Vec<String>) {
    (
        env!("CARGO_BIN_EXE_neojs").to_owned(),
        vec![script.to_str().unwrap().to_owned()],
    )
}

#[test]
fn exec_returns_success_and_captured_output() {
    let dir = tempfile::tempdir().unwrap();
    let child = dir.path().join("child.js");
    std::fs::write(&child, "console.log('child output');").unwrap();
    let (program, args) = neo_js_child(&child);

    let source = exec_script(
        &program,
        &args,
        b"",
        "if (result.code !== 0) throw Error('code ' + result.code);\n\
         if (result.success !== true) throw Error('success');\n\
         if (!(result.stdout instanceof Uint8Array)) throw Error('stdout type');\n\
         if (!(result.stderr instanceof Uint8Array)) throw Error('stderr type');\n\
         if (String.fromCharCode(...result.stdout) !== 'child output\\n') \
             throw Error('stdout bytes');\n\
         if (result.stderr.length !== 0) throw Error('stderr bytes');\n\
         console.log('ok');",
    );
    let output = run_cli(dir.path(), &["--allow-run"], &source);
    assert_cli_ok(&output);
    assert_eq!(output.stdout, b"ok\n");
}

#[test]
fn exec_reports_nonzero_exit_as_data() {
    let dir = tempfile::tempdir().unwrap();
    let child = dir.path().join("child.js");
    std::fs::write(&child, "throw Error('child failed');").unwrap();
    let (program, args) = neo_js_child(&child);

    let source = exec_script(
        &program,
        &args,
        b"",
        "if (result.code !== 1) throw Error('code ' + result.code);\n\
         if (result.success !== false) throw Error('success');\n\
         if (result.stdout.length !== 0) throw Error('stdout bytes');\n\
         if (!String.fromCharCode(...result.stderr).includes('child failed')) \
             throw Error('stderr bytes');\n\
         console.log('ok');",
    );
    let output = run_cli(dir.path(), &["--allow-run"], &source);
    assert_cli_ok(&output);
    assert_eq!(output.stdout, b"ok\n");
}

#[cfg(windows)]
#[test]
fn exec_rejects_batch_and_extensionless_programs_on_windows() {
    let dir = tempfile::tempdir().unwrap();
    let bat = dir.path().join("child.bat");
    std::fs::write(&bat, "@echo off\r\necho shell > marker.txt\r\n").unwrap();
    let cmd = dir.path().join("child.cmd");
    std::fs::write(&cmd, "@echo off\r\necho shell > marker.txt\r\n").unwrap();

    let candidates = to_string(&[
        bat.to_str().unwrap().to_owned(),
        cmd.to_str().unwrap().to_owned(),
        "child".to_owned(),
        "script.ps1".to_owned(),
        "cmd".to_owned(),
        r"C:\Windows\System32\cmd".to_owned(),
        r"C:\Windows\System32\cmd.exe.".to_owned(),
    ])
    .unwrap();
    let source = format!(
        "import {{ exec }} from 'neo:exec';\n\
         const candidates = {candidates};\n\
         for (const program of candidates) {{\n\
             let rejected = false;\n\
             try {{ exec(program); }} catch (error) {{\n\
                 if (!String(error).includes('.exe')) \
                     throw Error('unexpected error for ' + program + ': ' + error);\n\
                 rejected = true;\n\
             }}\n\
             if (!rejected) throw Error('accepted ' + program);\n\
         }}\n\
         console.log('ok');"
    );
    let output = run_cli(dir.path(), &["--allow-run"], &source);
    assert_cli_ok(&output);
    assert_eq!(output.stdout, b"ok\n");
    assert!(!dir.path().join("marker.txt").exists());
}

#[cfg(windows)]
#[test]
fn exec_allows_an_explicit_shell_on_windows() {
    let dir = tempfile::tempdir().unwrap();
    let source = "import { exec } from 'neo:exec';\n\
                  const result = exec('cmd.exe', ['/C', 'echo shell>marker.txt']);\n\
                  if (result.code !== 0 || !result.success) \
                      throw Error('cmd failed: ' + result.code);\n\
                  console.log('ok');";
    let output = run_cli(dir.path(), &["--allow-run"], source);
    assert_cli_ok(&output);
    assert_eq!(output.stdout, b"ok\n");
    let marker = std::fs::read_to_string(dir.path().join("marker.txt")).unwrap();
    assert!(marker.contains("shell"), "{marker:?}");
}

#[cfg(any(unix, windows))]
#[test]
fn exec_preserves_binary_stdin_and_output_streams() {
    let dir = tempfile::tempdir().unwrap();
    let (program, args) = raw_stream_child();

    // A full byte range, large enough to fill the stdin pipe. The child writes
    // 128 KiB of non-UTF-8 bytes to stderr before reading stdin, so the parent
    // must drain both streams and write stdin concurrently.
    const SIZE: usize = 128 * 1024;
    let input: Vec<u8> = (0..SIZE).map(|index| (index % 256) as u8).collect();

    let source = exec_script(
        &program,
        &args,
        &input,
        "if (result.code !== 0) throw Error('code ' + result.code);\n\
         if (result.success !== true) throw Error('success');\n\
         if (result.stdout.length !== input.length) \
             throw Error('stdout length ' + result.stdout.length);\n\
         for (let i = 0; i < input.length; i++) \
             if (result.stdout[i] !== input[i]) throw Error('stdout byte ' + i);\n\
         if (result.stderr.length !== 131072) \
             throw Error('stderr length ' + result.stderr.length);\n\
         for (let i = 0; i < result.stderr.length; i++) \
             if (result.stderr[i] !== 255) throw Error('stderr byte ' + i);\n\
         console.log('ok');",
    );
    let output = run_cli(dir.path(), &["--allow-run"], &source);
    assert_cli_ok(&output);
    assert_eq!(output.stdout, b"ok\n");
}

#[test]
fn exec_is_denied_by_default() {
    let dir = tempfile::tempdir().unwrap();

    let output = run_cli(
        dir.path(),
        &[],
        "if (typeof globalThis.__neoExec !== 'undefined') \
             throw Error('host function leaked');\n\
         console.log('ok');",
    );
    assert_cli_ok(&output);
    assert_eq!(output.stdout, b"ok\n");

    let output = run_cli(dir.path(), &[], "import { exec } from 'neo:exec';");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("neo:exec"), "{stderr}");

    // Other permissions must not imply process execution.
    let output = run_cli(
        dir.path(),
        &["--allow-fs", "--allow-env"],
        "import { exec } from 'neo:exec';",
    );
    assert!(!output.status.success());
}

/// Return the program and arguments for a child that writes 128 KiB of `0xff`
/// to stderr, then copies stdin to stdout byte for byte.
#[cfg(unix)]
fn raw_stream_child() -> (String, Vec<String>) {
    (
        "sh".to_owned(),
        vec![
            "-c".to_owned(),
            "dd if=/dev/zero bs=65536 count=2 2>/dev/null | tr '\\000' '\\377' >&2\ncat\n"
                .to_owned(),
        ],
    )
}

#[cfg(windows)]
fn raw_stream_child() -> (String, Vec<String>) {
    let script = r#"
$err = [Console]::OpenStandardError()
$bytes = New-Object byte[] 131072
for ($i = 0; $i -lt $bytes.Length; $i++) { $bytes[$i] = 255 }
$err.Write($bytes, 0, $bytes.Length)
$err.Flush()
$in = [Console]::OpenStandardInput()
$out = [Console]::OpenStandardOutput()
$buffer = New-Object byte[] 8192
while (($count = $in.Read($buffer, 0, $buffer.Length)) -gt 0) {
    $out.Write($buffer, 0, $count)
}
$out.Flush()
"#;
    (
        "powershell.exe".to_owned(),
        vec![
            "-NoProfile".to_owned(),
            "-NonInteractive".to_owned(),
            "-Command".to_owned(),
            script.to_owned(),
        ],
    )
}

#[test]
fn exec_spawn_error_has_stable_code_and_arguments_are_type_checked() {
    let dir = tempfile::tempdir().unwrap();
    let source = "import {exec} from 'neo:exec';\n\
        try { exec('neo-js-nonexistent-program.exe'); throw Error('missing error'); }\n\
        catch (e) { if (e.code !== 'EXEC_SPAWN') throw Error('code: ' + e.code); }\n\
        try { exec(42); throw Error('missing type error'); }\n\
        catch (e) { if (!(e instanceof TypeError)) throw e; }";
    assert_cli_ok(&run_cli(dir.path(), &["--allow-run"], source));
}

#[cfg(unix)]
#[test]
fn exec_rejects_oversized_output_and_stops_child() {
    let dir = tempfile::tempdir().unwrap();
    let source = "import {exec} from 'neo:exec';\n\
        try { exec('sh', ['-c', 'head -c 8388609 /dev/zero; exec sleep 30']); throw Error('missing error'); }\n\
        catch (e) { if (e.code !== 'EXEC_IO' || !e.message.includes('limit')) throw e; }";
    assert_cli_ok(&run_cli(dir.path(), &["--allow-run"], source));
}

// The external watchdog prevents a regression from hanging the test suite.
#[cfg(unix)]
#[test]
fn exec_failure_returns_while_descendant_holds_pipe() {
    use std::{
        thread,
        time::{Duration, Instant},
    };
    let dir = tempfile::tempdir().unwrap();
    let entry = dir.path().join("main.js");
    std::fs::write(&entry, "import {exec} from 'neo:exec';\n\
        try { exec('sh', ['-c', '(sleep 4) & head -c 8388609 /dev/zero; wait']); throw Error('missing error'); }\n\
        catch (e) { if (e.code !== 'EXEC_IO' || !e.message.includes('cleanup pending')) throw e; }\n\
        console.log('returned');").unwrap();
    let mut cli = Command::new(env!("CARGO_BIN_EXE_neojs"))
        .current_dir(dir.path())
        .arg("--allow-run")
        .arg(entry)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = cli.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        if Instant::now() >= deadline {
            cli.kill().unwrap();
            let _ = cli.wait();
            panic!("CLI did not return before descendant closed pipe");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

// ---------------------------------------------------------------------------
// `neo:exec.spawn`: owned, synchronous child processes.
//
// The child program is always this same CLI binary. That keeps the tests
// portable (no shell, no external program) and exercises a direct launch of a
// real executable.
// ---------------------------------------------------------------------------

/// A child that drains stdin to stdout after writing 128 KiB of `0xff` to
/// stderr, so both capture streams have to be drained concurrently.
const ECHO_CHILD: &str = "import {stdin, stdout, stderr} from 'neo:process';\n\
     const err = new Uint8Array(131072).fill(255);\n\
     stderr.write(err);\n\
     const buffer = new Uint8Array(8192);\n\
     let count;\n\
     while ((count = stdin.readSync(buffer)) !== null) \
         stdout.write(buffer.subarray(0, count));\n";

/// A child that stays alive until its stdin is closed or the process is killed.
const STDIN_SLEEPER: &str =
    "import {stdin} from 'neo:process';\nstdin.readSync(new Uint8Array(1));\n";

/// Write `source` into `dir` and return the program and arguments that run it
/// through the CLI with `flags`.
fn self_child(dir: &Path, name: &str, flags: &[&str], source: &str) -> (String, Vec<String>) {
    let path = dir.join(name);
    std::fs::write(&path, source).unwrap();
    let mut args: Vec<String> = flags.iter().map(|flag| (*flag).to_owned()).collect();
    args.push(path.to_str().unwrap().to_owned());
    (env!("CARGO_BIN_EXE_neojs").to_owned(), args)
}

#[test]
fn spawn_returns_success_output_and_caches_terminal_results() {
    let dir = tempfile::tempdir().unwrap();
    let (program, args) = self_child(dir.path(), "child.js", &[], "console.log('child output');");
    let source = format!(
        "import {{spawn}} from 'neo:exec';\n\
         const handle = spawn({program}, {args});\n\
         const result = handle.communicate();\n\
         if (result.code !== 0 || result.success !== true) throw Error('status');\n\
         if (result.complete !== true) throw Error('complete');\n\
         if (String.fromCharCode(...result.stdout) !== 'child output\\n') \
             throw Error('stdout');\n\
         if (result.stderr.length !== 0) throw Error('stderr');\n\
         const again = handle.wait();\n\
         if (again.code !== 0 || again.success !== true) throw Error('repeat status');\n\
         if (String.fromCharCode(...again.stdout) !== 'child output\\n') \
             throw Error('repeat stdout');\n\
         if (again.stdout === result.stdout) throw Error('cached array was shared');\n\
         const killed = handle.kill();\n\
         if (killed.code !== 0 || killed.success !== true) \
             throw Error('cached kill result');\n\
         if (killed.stdout.length !== result.stdout.length) throw Error('cached output');\n\
         console.log('ok');",
        program = to_string(&program).unwrap(),
        args = to_string(&args).unwrap(),
    );
    let output = run_cli(dir.path(), &["--allow-run"], &source);
    assert_cli_ok(&output);
    assert_eq!(output.stdout, b"ok\n");
}

#[test]
fn spawn_reports_nonzero_exit_as_data() {
    let dir = tempfile::tempdir().unwrap();
    let (program, args) = self_child(dir.path(), "child.js", &[], "throw Error('child failed');");
    let source = format!(
        "import {{spawn}} from 'neo:exec';\n\
         const result = spawn({program}, {args}).communicate();\n\
         if (result.code !== 1) throw Error('code');\n\
         if (result.success !== false) throw Error('success');\n\
         if (result.complete !== true) throw Error('complete');\n\
         if (result.stdout.length !== 0) throw Error('stdout');\n\
         if (!String.fromCharCode(...result.stderr).includes('child failed')) \
             throw Error('stderr');\n\
         console.log('ok');",
        program = to_string(&program).unwrap(),
        args = to_string(&args).unwrap(),
    );
    let output = run_cli(dir.path(), &["--allow-run"], &source);
    assert_cli_ok(&output);
    assert_eq!(output.stdout, b"ok\n");
}

#[test]
fn spawn_failures_use_stable_codes_and_argument_checks() {
    let dir = tempfile::tempdir().unwrap();
    let source = "import {spawn} from 'neo:exec';\n\
        try { spawn('neo-js-nonexistent-program.exe'); throw Error('missing error'); }\n\
        catch (e) { if (e.code !== 'EXEC_SPAWN') throw Error('code: ' + e.code); }\n\
        try { spawn(42); throw Error('missing type error'); }\n\
        catch (e) { if (!(e instanceof TypeError)) throw e; }\n\
        try { spawn('neo-js-nonexistent-program.exe', [1]); throw Error('missing args error'); }\n\
        catch (e) { if (!(e instanceof TypeError)) throw e; }";
    assert_cli_ok(&run_cli(dir.path(), &["--allow-run"], source));
}

#[test]
fn spawn_is_denied_before_launch_without_allow_run() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        "if (typeof globalThis.__neoExecSpawn !== 'undefined' || \
             typeof globalThis.__neoExecCall !== 'undefined') \
             throw Error('host function leaked');\n\
         console.log('ok');",
    );
    assert_cli_ok(&output);
    assert_eq!(output.stdout, b"ok\n");

    let output = run_cli(dir.path(), &[], "import {spawn} from 'neo:exec';");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("neo:exec"));

    let output = run_cli(
        dir.path(),
        &["--allow-fs", "--allow-env"],
        "import {spawn} from 'neo:exec';",
    );
    assert!(!output.status.success());
}

#[test]
fn spawn_preserves_binary_stdin_and_separate_output_streams() {
    let dir = tempfile::tempdir().unwrap();
    let (program, args) = self_child(dir.path(), "echo.js", &["--allow-stdio"], ECHO_CHILD);
    const SIZE: usize = 128 * 1024;
    let source = format!(
        "import {{spawn}} from 'neo:exec';\n\
         const input = new Uint8Array({SIZE});\n\
         for (let i = 0; i < input.length; i++) input[i] = i % 256;\n\
         const result = spawn({program}, {args}).communicate(input);\n\
         if (result.code !== 0 || result.success !== true) throw Error('status');\n\
         if (result.complete !== true) throw Error('complete');\n\
         if (result.stdout.length !== input.length) \
             throw Error('stdout length ' + result.stdout.length);\n\
         for (let i = 0; i < input.length; i++) \
             if (result.stdout[i] !== input[i]) throw Error('stdout byte ' + i);\n\
         if (result.stderr.length !== 131072) \
             throw Error('stderr length ' + result.stderr.length);\n\
         for (let i = 0; i < result.stderr.length; i++) \
             if (result.stderr[i] !== 255) throw Error('stderr byte ' + i);\n\
         console.log('ok');",
        program = to_string(&program).unwrap(),
        args = to_string(&args).unwrap(),
    );
    let output = run_cli(dir.path(), &["--allow-run"], &source);
    assert_cli_ok(&output);
    assert_eq!(output.stdout, b"ok\n");
}

#[test]
fn spawn_kill_reaps_the_direct_child_and_caches_the_result() {
    let dir = tempfile::tempdir().unwrap();
    let (program, args) = self_child(dir.path(), "sleeper.js", &["--allow-stdio"], STDIN_SLEEPER);
    let source = format!(
        "import {{spawn}} from 'neo:exec';\n\
         const handle = spawn({program}, {args});\n\
         const result = handle.kill();\n\
         if (result.success !== false) throw Error('success');\n\
         if (result.code !== null && result.code === 0) throw Error('live code');\n\
         if (typeof result.complete !== 'boolean') throw Error('complete type');\n\
         const waited = handle.wait();\n\
         if (waited.code !== result.code || waited.success !== result.success) \
             throw Error('wait after kill');\n\
         const again = handle.kill();\n\
         if (again.code !== result.code) throw Error('repeated kill');\n\
         console.log('ok');",
        program = to_string(&program).unwrap(),
        args = to_string(&args).unwrap(),
    );
    let output = run_cli(dir.path(), &["--allow-run"], &source);
    assert_cli_ok(&output);
    assert_eq!(output.stdout, b"ok\n");
}

#[test]
fn spawn_caps_outstanding_handles_and_releases_them_after_kill() {
    let dir = tempfile::tempdir().unwrap();
    let (program, args) = self_child(dir.path(), "sleeper.js", &["--allow-stdio"], STDIN_SLEEPER);
    let (plain, plain_args) = self_child(dir.path(), "child.js", &[], "console.log('ok');");
    let source = format!(
        "import {{spawn}} from 'neo:exec';\n\
         const handles = [];\n\
         for (let i = 0; i < 8; i++) handles.push(spawn({program}, {args}));\n\
         let exhausted = false;\n\
         try {{ spawn({program}, {args}); }}\n\
         catch (e) {{ if (e.code !== 'EXEC_WORKER') throw e; exhausted = true; }}\n\
         if (!exhausted) throw Error('ninth handle was not rejected');\n\
         for (const handle of handles) {{\n\
             const result = handle.kill();\n\
             if (result.success) throw Error('killed child reported success');\n\
         }}\n\
         const end = Date.now() + 3000;\n\
         while (true) {{\n\
             try {{\n\
                 const result = spawn({plain}, {plain_args}).communicate();\n\
                 if (!result.success) throw Error('replacement child failed');\n\
                 break;\n\
             }} catch (e) {{\n\
                 if (e.code !== 'EXEC_WORKER') throw e;\n\
                 if (Date.now() >= end) throw Error('killed handles never released slots');\n\
                 const spin = Date.now() + 10;\n\
                 while (Date.now() < spin) {{}}\n\
             }}\n\
         }}\n\
         console.log('ok');",
        program = to_string(&program).unwrap(),
        args = to_string(&args).unwrap(),
        plain = to_string(&plain).unwrap(),
        plain_args = to_string(&plain_args).unwrap(),
    );
    let output = run_cli(dir.path(), &["--allow-run"], &source);
    assert_cli_ok(&output);
    assert_eq!(output.stdout, b"ok\n");
}

#[test]
fn spawn_drains_multiple_handles_before_they_are_waited_on() {
    let dir = tempfile::tempdir().unwrap();
    let (program, args) = self_child(dir.path(), "echo.js", &["--allow-stdio"], ECHO_CHILD);
    let source = format!(
        "import {{spawn}} from 'neo:exec';\n\
         const first = spawn({program}, {args});\n\
         const second = spawn({program}, {args});\n\
         const input = new Uint8Array(8192).fill(7);\n\
         const later = second.communicate(input);\n\
         const earlier = first.communicate(input);\n\
         for (const result of [earlier, later]) {{\n\
             if (!result.success || !result.complete) throw Error('status');\n\
             if (result.stdout.length !== input.length) throw Error('stdout length');\n\
             for (let i = 0; i < input.length; i++) \
                 if (result.stdout[i] !== 7) throw Error('stdout byte');\n\
             if (result.stderr.length !== 131072) throw Error('stderr length');\n\
         }}\n\
         console.log('ok');",
        program = to_string(&program).unwrap(),
        args = to_string(&args).unwrap(),
    );
    let output = run_cli(dir.path(), &["--allow-run"], &source);
    assert_cli_ok(&output);
    assert_eq!(output.stdout, b"ok\n");
}

#[test]
fn spawn_reports_output_limit_failure_and_recovers_capacity() {
    let dir = tempfile::tempdir().unwrap();
    let (program, args) = self_child(
        dir.path(),
        "flood.js",
        &["--allow-stdio", "--allow-fs"],
        "import {stdout} from 'neo:process';\n\
         import {writeText} from 'neo:fs';\n\
         const chunk = new Uint8Array(1024 * 1024);\n\
         for (let i = 0; i < 9; i++) stdout.write(chunk);\n\
         const alive = Date.now() + 600;\n\
         while (Date.now() < alive) {{}}\n\
         writeText('survived.txt', 'yes');\n",
    );
    let (plain, plain_args) = self_child(dir.path(), "child.js", &[], "console.log('ok');");
    let source = format!(
        "import {{spawn}} from 'neo:exec';\n\
         import {{readText}} from 'neo:fs';\n\
         const handle = spawn({program}, {args});\n\
         let first = null;\n\
         try {{ handle.communicate(); }} catch (e) {{ first = e.code; }}\n\
         if (first !== 'EXEC_IO') throw Error('first code: ' + first);\n\
         let second = null;\n\
         try {{ handle.communicate(); }} catch (e) {{ second = e.code; }}\n\
         if (second !== 'EXEC_IO') throw Error('second code: ' + second);\n\
         const settle = Date.now() + 1200;\n\
         while (Date.now() < settle) {{}}\n\
         let survived = false;\n\
         try {{ readText('survived.txt'); survived = true; }} catch (e) {{}}\n\
         if (survived) throw Error('direct child survived EXEC_IO cleanup');\n\
         const end = Date.now() + 3000;\n\
         while (true) {{\n\
             try {{\n\
                 const result = spawn({plain}, {plain_args}).communicate();\n\
                 if (!result.success) throw Error('replacement child failed');\n\
                 break;\n\
             }} catch (e) {{\n\
                 if (e.code !== 'EXEC_WORKER') throw e;\n\
                 if (Date.now() >= end) throw Error('overflow slot never released');\n\
                 const spin = Date.now() + 10;\n\
                 while (Date.now() < spin) {{}}\n\
             }}\n\
         }}\n\
         console.log('ok');",
        program = to_string(&program).unwrap(),
        args = to_string(&args).unwrap(),
        plain = to_string(&plain).unwrap(),
        plain_args = to_string(&plain_args).unwrap(),
    );
    let output = run_cli(dir.path(), &["--allow-run", "--allow-fs"], &source);
    assert_cli_ok(&output);
    assert_eq!(output.stdout, b"ok\n");
}

#[test]
fn runtime_drop_kills_a_live_spawned_child() {
    use std::time::{Duration, Instant};

    let dir = tempfile::tempdir().unwrap();
    let (program, args) = self_child(
        dir.path(),
        "slow.js",
        &["--allow-fs"],
        "import {writeText} from 'neo:fs';\n\
         writeText('started.txt', 'yes');\n\
         const end = Date.now() + 2000;\n\
         while (Date.now() < end) {}\n\
         writeText('alive.txt', 'yes');\n",
    );
    let source = format!(
        "import {{spawn}} from 'neo:exec';\n\
         import {{readText}} from 'neo:fs';\n\
         spawn({program}, {args});\n\
         while (true) {{\n\
             try {{ if (readText('started.txt') === 'yes') break; }} catch (e) {{}}\n\
         }}",
        program = to_string(&program).unwrap(),
        args = to_string(&args).unwrap(),
    );
    let entry = dir.path().join("main.js");
    std::fs::write(&entry, source).unwrap();
    let start = Instant::now();
    let output = Command::new(env!("CARGO_BIN_EXE_neojs"))
        .current_dir(dir.path())
        .args(["--allow-run", "--allow-fs"])
        .arg(&entry)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "status: {:?}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        dir.path().join("started.txt").exists(),
        "the child never started"
    );
    assert!(
        start.elapsed() < Duration::from_millis(1800),
        "runtime drop waited for the child instead of killing it"
    );
    std::thread::sleep(Duration::from_millis(2500));
    assert!(
        !dir.path().join("alive.txt").exists(),
        "the child survived runtime drop"
    );
}

#[test]
fn spawn_child_uses_the_runtime_directory() {
    let dir = tempfile::tempdir().unwrap();
    let subdirectory = dir.path().join("nested");
    std::fs::create_dir(&subdirectory).unwrap();
    let (program, args) = self_child(
        dir.path(),
        "marker.js",
        &["--allow-fs"],
        "import {writeText} from 'neo:fs';\nwriteText('marker.txt', 'yes');\n",
    );
    let source = format!(
        "import {{chdir}} from 'neo:process';\n\
         import {{spawn}} from 'neo:exec';\n\
         chdir({directory});\n\
         const result = spawn({program}, {args}).wait();\n\
         if (!result.success) throw Error('child failed');\n\
         console.log('ok');",
        directory = to_string(subdirectory.to_str().unwrap()).unwrap(),
        program = to_string(&program).unwrap(),
        args = to_string(&args).unwrap(),
    );
    let output = run_cli(dir.path(), &["--allow-run"], &source);
    assert_cli_ok(&output);
    assert_eq!(output.stdout, b"ok\n");
    assert_eq!(
        std::fs::read_to_string(subdirectory.join("marker.txt")).unwrap(),
        "yes"
    );
    assert!(!dir.path().join("marker.txt").exists());
}

#[test]
fn spawn_wait_closes_stdin_so_a_reader_finishes() {
    let dir = tempfile::tempdir().unwrap();
    let (program, args) = self_child(dir.path(), "sleeper.js", &["--allow-stdio"], STDIN_SLEEPER);
    let source = format!(
        "import {{spawn}} from 'neo:exec';\n\
         const result = spawn({program}, {args}).wait();\n\
         if (!result.success || !result.complete) throw Error('stdin was not closed');\n\
         if (result.code !== 0) throw Error('code');\n\
         console.log('ok');",
        program = to_string(&program).unwrap(),
        args = to_string(&args).unwrap(),
    );
    let output = run_cli(dir.path(), &["--allow-run"], &source);
    assert_cli_ok(&output);
    assert_eq!(output.stdout, b"ok\n");
}

// The external watchdog prevents a regression from hanging the test suite.
// The background descendant holds the inherited output pipe so the direct
// child can be reaped without waiting for EOF; it exits on its own after one
// second, so the test leaves no long-lived fixture process.
#[cfg(unix)]
#[test]
fn spawn_kill_returns_while_a_descendant_holds_the_output_pipe() {
    use std::{
        thread,
        time::{Duration, Instant},
    };

    let dir = tempfile::tempdir().unwrap();
    let entry = dir.path().join("main.js");
    std::fs::write(
        &entry,
        "import {spawn} from 'neo:exec';\n\
         import {readText} from 'neo:fs';\n\
         const handle = spawn('sh', ['-c',\n\
             '(echo started > started.txt; sleep 1) & while [ ! -f started.txt ]; do sleep 0.01; done; exec sleep 30']);\n\
         while (true) {\n\
             try { if (readText('started.txt').includes('started')) break; } catch (e) {}\n\
         }\n\
         const start = Date.now();\n\
         const result = handle.kill();\n\
         if (Date.now() - start > 1000) throw Error('kill waited for the pipe');\n\
         if (result.complete !== false) throw Error('expected an incomplete snapshot');\n\
         if (result.success) throw Error('killed child reported success');\n\
         console.log('ok');",
    )
    .unwrap();
    let mut cli = Command::new(env!("CARGO_BIN_EXE_neojs"))
        .current_dir(dir.path())
        .args(["--allow-run", "--allow-fs"])
        .arg(entry)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = cli.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        if Instant::now() >= deadline {
            cli.kill().unwrap();
            let _ = cli.wait();
            panic!("kill blocked on a descendant holding the pipe");
        }
        thread::sleep(Duration::from_millis(10));
    }
}
