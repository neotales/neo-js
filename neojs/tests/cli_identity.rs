//! End-to-end checks for the executable identity.
//!
//! The binary is `neojs`, and its clap command name is pinned to the same
//! spelling so `--help` and `--version` cannot drift from Cargo's `[[bin]]`.

use std::process::Command;

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_neojs"))
}

fn stdout(output: &std::process::Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("CLI output is UTF-8")
}

#[test]
fn runs_a_script() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("identity.js");
    std::fs::write(&script, "console.log('identity ok');").unwrap();

    let output = binary().arg(&script).output().unwrap();

    assert!(
        output.status.success(),
        "status: {:?}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(stdout(&output), "identity ok\n");
}

#[test]
fn help_identifies_neojs() {
    let output = binary().arg("--help").output().unwrap();

    assert!(
        output.status.success(),
        "status: {:?}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let help = stdout(&output);
    assert!(
        help.lines()
            .any(|line| line.trim_start().starts_with("Usage: neojs ")),
        "help does not identify neojs in its usage line:\n{help}"
    );
    assert!(
        !help.contains("Usage: neo-js"),
        "help still uses the old name"
    );
}

#[test]
fn version_identifies_neojs() {
    let output = binary().arg("--version").output().unwrap();

    assert!(
        output.status.success(),
        "status: {:?}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        stdout(&output),
        format!("neojs {}\n", env!("CARGO_PKG_VERSION"))
    );
}
