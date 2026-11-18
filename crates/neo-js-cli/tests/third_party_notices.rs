//! End-to-end tests for `--third-party-notices`.
//!
//! The flag must print the embedded notices without a script, without a
//! permission flag, and without changing ordinary script or help behavior.

use std::{fs, process::Command};

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_neo-js"))
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn prints_notices_without_a_script_or_permission() {
    let dir = tempfile::tempdir().unwrap();
    let output = binary()
        .current_dir(dir.path())
        .arg("--third-party-notices")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "status: {:?}\nstderr: {}",
        output.status,
        stderr(&output)
    );
    assert!(output.stderr.is_empty(), "{}", stderr(&output));

    let text = String::from_utf8(output.stdout).expect("notice output is UTF-8");
    assert_eq!(text.as_str(), neo_js::third_party_notices());

    for expected in [
        "Third-party notices for `neo:path`",
        "c7e88644f45a78f65bf639c9d365bb9eff21bcfe",
        "Copyright 2022-2026 Neotales",
        "Copyright 2018-2022 the Deno authors",
        "Copyright (c) 2013 James Halliday",
        "Permission is hereby granted",
    ] {
        assert!(
            text.contains(expected),
            "notice output is missing {expected:?}"
        );
    }

    // The empty working directory proves no entry file was needed or created.
    assert_eq!(
        fs::read_dir(dir.path()).unwrap().count(),
        0,
        "notice mode created or required a file"
    );
}

#[test]
fn ordinary_script_and_help_behavior_is_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("main.js");
    fs::write(&script, "console.log('ok');").unwrap();

    let run = binary().arg(&script).output().unwrap();
    assert!(
        run.status.success(),
        "status: {:?}\nstderr: {}",
        run.status,
        stderr(&run)
    );
    assert_eq!(run.stdout, b"ok\n");

    let no_args = binary().output().unwrap();
    assert!(
        !no_args.status.success(),
        "a missing script must be a usage error"
    );

    for flag in ["-h", "--help"] {
        let help = binary().arg(flag).output().unwrap();
        assert!(help.status.success(), "{}", stderr(&help));
        let help = String::from_utf8(help.stdout).expect("help output is UTF-8");
        assert!(help.contains("Usage:"));
        assert!(help.contains("--third-party-notices"));
        let help = help.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            help.contains("--allow-fs Grant the host user's filesystem authority, including deletion, renaming, and replacement"),
            "{flag} did not describe the authority granted by --allow-fs"
        );
    }
}

#[test]
fn notices_flag_rejects_a_script() {
    let output = binary()
        .args(["--third-party-notices", "missing.js"])
        .output()
        .unwrap();
    assert!(!output.status.success());
}
