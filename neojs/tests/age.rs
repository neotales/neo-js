//! CLI-level tests for `neo:age`.
//!
//! The module needs no permission flag, so the CLI must run these scripts with
//! no arguments beyond the entry path. The TypeScript case also checks that
//! the module survives type erasure.

use std::process::{Command, Output};

fn run(dir: &std::path::Path, name: &str, source: &str) -> Output {
    let path = dir.join(name);
    std::fs::write(&path, source).unwrap();
    Command::new(env!("CARGO_BIN_EXE_neojs"))
        .arg(&path)
        .output()
        .unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn cli_runs_age_without_a_permission_flag() {
    let dir = tempfile::tempdir().unwrap();
    let output = run(
        dir.path(),
        "main.js",
        "import { decrypt, encrypt, generateIdentity, identityToRecipient } from 'neo:age';
         if ('__neoAgeEncrypt' in globalThis || '__neoAgeDecrypt' in globalThis ||
             '__neoAgeGenerate' in globalThis || '__neoAgeDerive' in globalThis) {
             throw Error('global callback exposed');
         }
         const pair = generateIdentity();
         if (identityToRecipient(pair.identity) !== pair.recipient) throw Error('recipient');
         const plaintext = Uint8Array.from([0, 1, 2, 0, 255, 128]);
         const ciphertext = encrypt(pair.recipient, plaintext);
         const out = decrypt(pair.identity, ciphertext);
         if (out.length !== plaintext.length) throw Error('length');
         for (let i = 0; i < plaintext.length; i++) {
             if (out[i] !== plaintext[i]) throw Error('byte ' + i);
         }
         console.log('ok');",
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(output.stdout, b"ok\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn cli_runs_age_from_a_typescript_entry() {
    let dir = tempfile::tempdir().unwrap();
    let output = run(
        dir.path(),
        "main.ts",
        "import { decrypt, encrypt, generateIdentity } from 'neo:age';
         const pair: { identity: string; recipient: string } = generateIdentity();
         const plaintext: Uint8Array = Uint8Array.from([1, 2, 3]);
         const ciphertext: Uint8Array = encrypt(pair.recipient, plaintext);
         const out: Uint8Array = decrypt(pair.identity, ciphertext);
         if (out.length !== 3) throw Error('length');
         console.log('ts-ok');",
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(output.stdout, b"ts-ok\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn cli_reports_a_wrong_identity_without_printing_keys_or_plaintext() {
    let dir = tempfile::tempdir().unwrap();
    let output = run(
        dir.path(),
        "wrong-identity.js",
        "import { decrypt, encrypt, generateIdentity } from 'neo:age';
         const first = generateIdentity();
         const second = generateIdentity();
         const secret = Uint8Array.from([0xde, 0xad, 0xbe, 0xef, 0x00, 0x99]);
         const ciphertext = encrypt(first.recipient, secret);
         decrypt(second.identity, ciphertext);
         console.log('unreachable');",
    );
    assert!(!output.status.success());
    let text = stderr(&output);
    assert!(text.contains("no supplied identity matches"), "{text}");
    assert!(!text.contains("AGE-SECRET-KEY-1"), "{text}");
    assert!(!text.contains("dead"), "{text}");
    assert!(!text.contains("unreachable"), "{text}");
    assert!(output.stdout.is_empty());
}

#[test]
fn cli_reports_payload_tampering_without_printing_plaintext() {
    let dir = tempfile::tempdir().unwrap();
    let output = run(
        dir.path(),
        "tampered.js",
        "import { decrypt, encrypt, generateIdentity } from 'neo:age';
         const { identity, recipient } = generateIdentity();
         const secret = Uint8Array.from([0xde, 0xad, 0xbe, 0xef, 0x00, 0x99]);
         const ciphertext = encrypt(recipient, secret);
         ciphertext[ciphertext.length - 1] ^= 1;
         decrypt(identity, ciphertext);
         console.log('unreachable');",
    );
    assert!(!output.status.success());
    let text = stderr(&output);
    assert!(text.contains("authentication failed"), "{text}");
    assert!(!text.contains("AGE-SECRET-KEY-1"), "{text}");
    assert!(!text.contains("dead"), "{text}");
    assert!(!text.contains("unreachable"), "{text}");
    assert!(output.stdout.is_empty());
}

#[test]
fn cli_rejects_a_malformed_identity() {
    let dir = tempfile::tempdir().unwrap();
    let output = run(
        dir.path(),
        "bad-identity.js",
        "import { decrypt } from 'neo:age';
         decrypt('not-an-age-identity', new Uint8Array(1));",
    );
    assert!(!output.status.success());
    assert!(stderr(&output).contains("valid age X25519 identity"));
}
