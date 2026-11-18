//! CLI-level tests for `neo:crypto`.
//!
//! Pure cryptography needs no permission flag, so the CLI must run these
//! scripts with no arguments beyond the entry path. The TypeScript case also
//! checks that the module survives type erasure.

use std::process::{Command, Output};

fn run(dir: &std::path::Path, name: &str, source: &str) -> Output {
    let path = dir.join(name);
    std::fs::write(&path, source).unwrap();
    Command::new(env!("CARGO_BIN_EXE_neo-js"))
        .arg(&path)
        .output()
        .unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn cli_runs_crypto_without_a_permission_flag() {
    let dir = tempfile::tempdir().unwrap();
    let output = run(
        dir.path(),
        "main.js",
        "import { decrypt, encrypt, generateKey } from 'neo:crypto';
         if ('__neoCryptoEncrypt' in globalThis || '__neoCryptoDecrypt' in globalThis ||
             '__neoCryptoGenerateKey' in globalThis) throw Error('global callback exposed');
         const key = generateKey();
         if (key.length !== 32) throw Error('key length');
         const plaintext = Uint8Array.from([0, 1, 2, 0, 255, 128]);
         const envelope = encrypt(key, plaintext);
         const out = decrypt(key, envelope);
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
fn cli_runs_crypto_from_a_typescript_entry() {
    let dir = tempfile::tempdir().unwrap();
    let output = run(
        dir.path(),
        "main.ts",
        "import { decrypt, encrypt, generateKey } from 'neo:crypto';
         const key: Uint8Array = generateKey();
         const plaintext: Uint8Array = Uint8Array.from([1, 2, 3]);
         const envelope: Uint8Array = encrypt(key, plaintext);
         const out: Uint8Array = decrypt(key, envelope);
         if (out.length !== 3) throw Error('length');
         console.log('ts-ok');",
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(output.stdout, b"ts-ok\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn cli_reports_tampering_without_printing_key_or_plaintext() {
    let dir = tempfile::tempdir().unwrap();
    let output = run(
        dir.path(),
        "tampered.js",
        "import { decrypt, encrypt } from 'neo:crypto';
         const key = new Uint8Array(32).fill(0x41);
         const secret = Uint8Array.from([0xde, 0xad, 0xbe, 0xef, 0x00, 0x99]);
         const envelope = encrypt(key, secret);
         envelope[envelope.length - 1] ^= 1;
         decrypt(key, envelope);
         console.log('unreachable');",
    );
    assert!(!output.status.success());
    let text = stderr(&output);
    assert!(text.contains("authentication failed"), "{text}");
    assert!(!text.contains("41414141"), "{text}");
    assert!(!text.contains("dead"), "{text}");
    assert!(!text.contains("unreachable"), "{text}");
    assert!(output.stdout.is_empty());
}

#[test]
fn cli_rejects_a_wrong_length_key() {
    let dir = tempfile::tempdir().unwrap();
    let output = run(
        dir.path(),
        "bad-key.js",
        "import { encrypt } from 'neo:crypto';
         encrypt(new Uint8Array(16), new Uint8Array(1));",
    );
    assert!(!output.status.success());
    assert!(stderr(&output).contains("key must be exactly 32 bytes"));
}
