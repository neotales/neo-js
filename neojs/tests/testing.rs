//! End-to-end tests for `neo:test` and `neo:assert` through the CLI.
//!
//! Each test writes an entry module into a temporary directory, runs the CLI
//! there, and asserts on output and process status. The failed-test cases are
//! as important as the passing ones: a runner that reports success when a test
//! failed is worse than no runner at all.

use std::{
    path::Path,
    process::{Command, Output},
};

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

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn assert_ok(output: &Output) {
    assert!(
        output.status.success(),
        "status: {:?}\nstdout: {}\nstderr: {}",
        output.status,
        stdout(output),
        stderr(output)
    );
}

fn assert_failed(output: &Output) {
    assert!(
        !output.status.success(),
        "expected a non-zero exit\nstdout: {}\nstderr: {}",
        stdout(output),
        stderr(output)
    );
}

#[test]
fn passing_tests_exit_zero_without_an_explicit_run() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        "import test from 'neo:test';
         import {equal} from 'neo:assert';
         test('first', () => equal(1, 1));
         test('second', () => equal('a', 'a'));",
    );
    assert_ok(&output);
    assert_eq!(
        stdout(&output),
        "ok 1 - first\nok 2 - second\n# 2 tests: 2 passed, 0 failed\n"
    );
    assert_eq!(stderr(&output), "");
}

#[test]
fn failed_test_exits_nonzero_and_does_not_stop_later_tests() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        "import test from 'neo:test';
         import {equal} from 'neo:assert';
         test('passes', () => equal(1, 1));
         test('fails', () => equal(1, 2));
         test('also passes', () => equal(2, 2));",
    );
    assert_failed(&output);
    let out = stdout(&output);
    assert!(out.contains("not ok 2 - fails"), "{out}");
    assert!(
        out.contains("expected values to be strictly equal"),
        "{out}"
    );
    assert!(out.contains("actual: 1"), "{out}");
    assert!(out.contains("expected: 2"), "{out}");
    assert!(out.contains("ok 3 - also passes"), "{out}");
    assert!(out.contains("# 3 tests: 2 passed, 1 failed"), "{out}");
    assert!(
        stderr(&output).contains("1 of 3 test(s) failed"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn tests_registered_before_a_module_error_still_run() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        "import test from 'neo:test';
         test('ran before failure', () => {});
         throw new Error('module exploded after registration');",
    );
    assert_failed(&output);
    assert!(
        stdout(&output).contains("ok 1 - ran before failure"),
        "{}",
        stdout(&output)
    );
    assert!(
        stderr(&output).contains("module exploded after registration"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn an_abandoned_module_reports_the_error_without_running_registered_tests_or_jobs() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        "import test from 'neo:test';
         test('must not run', () => console.log('test executed'));
         Promise.resolve().then(() => console.log('job executed'));
         throw Error('module exploded with work queued');",
    );
    assert_failed(&output);
    assert!(stdout(&output).is_empty(), "{}", stdout(&output));
    assert!(stderr(&output).contains("module exploded with work queued"));
    assert!(stderr(&output).contains("NEO_MODULE_ABORTED"));
}

#[test]
fn async_and_generator_callbacks_are_rejected_at_registration() {
    let dir = tempfile::tempdir().unwrap();

    let output = run_cli(
        dir.path(),
        &[],
        "import test from 'neo:test'; test('async', async () => {});",
    );
    assert_failed(&output);
    assert!(
        stderr(&output).contains("must be synchronous"),
        "{}",
        stderr(&output)
    );

    let output = run_cli(
        dir.path(),
        &[],
        "import test from 'neo:test'; test('generator', function* () { yield 1; });",
    );
    assert_failed(&output);
    assert!(
        stderr(&output).contains("must be synchronous"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_promise_returned_by_a_synchronous_callback_is_a_failure() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        "import test from 'neo:test'; test('returns a promise', () => Promise.resolve(1));",
    );
    assert_failed(&output);
    let out = stdout(&output);
    assert!(out.contains("not ok 1 - returns a promise"), "{out}");
    assert!(out.contains("returned a Promise"), "{out}");
    assert!(out.contains("# 1 test: 0 passed, 1 failed"), "{out}");
    assert!(
        stderr(&output).contains("1 of 1 test(s) failed"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_bound_generator_callback_fails_instead_of_passing_without_running() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        "import test from 'neo:test';
         test('bound generator', (function* () { throw new Error('must not be skipped'); }).bind(null));",
    );
    assert_failed(&output);
    let out = stdout(&output);
    assert!(out.contains("not ok 1 - bound generator"), "{out}");
    assert!(out.contains("returned a generator"), "{out}");
    assert!(
        !out.lines().any(|line| line == "ok 1 - bound generator"),
        "{out}"
    );
    assert!(
        stderr(&output).contains("1 of 1 test(s) failed"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn disguised_async_results_fail_without_stopping_later_tests() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        r#"import test, {run} from 'neo:test';
           test('disguised rejection', () => {
               const promise = Promise.reject('private rejection reason');
               Object.setPrototypeOf(promise, null);
               Object.defineProperty(promise, 'then', {value: null});
               return promise;
           });
           const make = (function* () { throw new Error('must not resume'); }).bind(null);
           test('disguised generator', () => {
               const generator = make();
               Object.defineProperty(generator, Symbol.toStringTag, {value: 'Object'});
               return Object.setPrototypeOf(generator, null);
           });
           test('disguised async generator', () => {
               const generator = (async function* () { throw new Error('must not resume'); })();
               Object.defineProperty(generator, Symbol.toStringTag, {value: 'Object'});
               return generator;
           });
           test('ordinary throw', () => { throw new Error('ordinary failure'); });
           test('later runs', () => 42);
           try { run(); } catch (error) { console.log('caught ' + error.code); }"#,
    );
    assert_failed(&output);
    let out = stdout(&output);
    let err = stderr(&output);
    for line in [
        "not ok 1 - disguised rejection",
        "not ok 2 - disguised generator",
        "not ok 3 - disguised async generator",
        "not ok 4 - ordinary throw",
        "ok 5 - later runs",
        "# 5 tests: 1 passed, 4 failed",
        "caught NEO_TEST_FAILURES",
    ] {
        assert!(out.lines().any(|actual| actual == line), "{line}: {out}");
    }
    assert!(out.contains("returned a Promise"), "{out}");
    assert_eq!(out.matches("returned a generator").count(), 2, "{out}");
    assert!(out.contains("ordinary failure"), "{out}");
    assert!(err.contains("4 of 5 test(s) failed"), "{err}");
    assert!(!out.contains("private rejection reason"), "{out}");
    assert!(!err.contains("private rejection reason"), "{err}");
    assert!(!out.contains("must not resume"), "{out}");
}

#[test]
fn malformed_test_names_do_not_break_host_failure_reporting() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        r#"import test from 'neo:test';
           test('bad \ud800 name \udfff', () => Promise.reject('private reason'));
           test('x'.repeat(293) + '🦊end', () => Promise.resolve());
           test('later runs', () => {});"#,
    );
    assert_failed(&output);
    let out = std::str::from_utf8(&output.stdout).unwrap();
    let err = std::str::from_utf8(&output.stderr).unwrap();
    assert!(out.contains("not ok 1 - bad � name �"), "{out}");
    assert!(
        out.contains(&format!("not ok 2 - {}🦊end", "x".repeat(293))),
        "{out}"
    );
    assert!(out.contains("ok 3 - later runs"), "{out}");
    assert!(out.contains("# 3 tests: 1 passed, 2 failed"), "{out}");
    assert!(err.contains("2 of 3 test(s) failed"), "{err}");
    assert!(!out.contains("private reason"), "{out}");
    assert!(!err.contains("private reason"), "{err}");
}

#[test]
fn catching_run_failure_does_not_hide_it_from_the_process() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        "import {test, run} from 'neo:test';
         test('fails now', () => { throw new Error('boom'); });
         try { run(); } catch (error) { console.log('caught ' + error.code); }
         test('registered after run', () => {});",
    );
    assert_failed(&output);
    let out = stdout(&output);
    assert!(out.contains("not ok 1 - fails now"), "{out}");
    assert!(out.contains("caught NEO_TEST_FAILURES"), "{out}");
    assert!(out.contains("ok 1 - registered after run"), "{out}");
    assert!(
        stderr(&output).contains("1 of 2 test(s) failed"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn explicit_run_returns_counts_and_later_tests_still_run() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        "import {test, run} from 'neo:test';
         test('one', () => {});
         const summary = run();
         if (summary.total !== 1 || summary.passed !== 1 || summary.failed !== 0)
             throw new Error('unexpected summary ' + JSON.stringify(summary));
         console.log('summary ok');",
    );
    assert_ok(&output);
    let out = stdout(&output);
    assert!(out.contains("ok 1 - one"), "{out}");
    assert!(out.contains("summary ok"), "{out}");
    assert!(!out.contains("# 0 tests"), "{out}");
}

#[test]
fn deep_equal_failure_names_the_first_differing_path() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        "import test from 'neo:test';
         import {deepEqual} from 'neo:assert';
         test('deep', () => deepEqual({a: [1, {b: 2}]}, {a: [1, {b: 3}]}));",
    );
    assert_failed(&output);
    let out = stdout(&output);
    assert!(out.contains("not ok 1 - deep"), "{out}");
    assert!(out.contains("$.a[1].b"), "{out}");
    assert!(out.contains("actual: 2"), "{out}");
    assert!(out.contains("expected: 3"), "{out}");
}

#[test]
fn registration_inside_a_test_fails_that_test_explicitly() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        "import test from 'neo:test';
         test('outer', () => { test('nested', () => {}); });",
    );
    assert_failed(&output);
    let out = stdout(&output);
    assert!(out.contains("not ok 1 - outer"), "{out}");
    assert!(
        out.contains("cannot register a test while tests are running"),
        "{out}"
    );
    assert!(
        !out.contains("nested"),
        "nested test was silently registered: {out}"
    );
}

#[test]
fn test_modules_need_no_permissions_and_grant_none() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        "import test from 'neo:test';
         import {equal} from 'neo:assert';
         test('no capabilities', () => {
             equal(typeof fs, 'undefined');
             equal(typeof env, 'undefined');
             equal(typeof __neoExec, 'undefined');
         });",
    );
    assert_ok(&output);
    assert!(stdout(&output).contains("ok 1 - no capabilities"));
}

#[test]
fn tampering_with_json_cannot_hide_a_failed_test() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        "import test from 'neo:test';
         test('fails', () => { throw new Error('visible failure'); });
         JSON.stringify = () => '{\"total\":0,\"passed\":0,\"failed\":0,\"failures\":[]}';",
    );
    assert_failed(&output);
    assert!(stdout(&output).contains("not ok 1 - fails"));
    assert!(stdout(&output).contains("visible failure"));
    assert!(
        stderr(&output).contains("1 of 1 test(s) failed"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn prototype_mutation_after_registration_cannot_skip_or_forge() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        "import test from 'neo:test';
         test('fails', () => { throw new Error('must stay failed'); });
         test('passes', () => {});
         Array.prototype.splice = () => [];
         Object.prototype.toJSON = () => ({
             total: 0, passed: 0, failed: 0, failures: []
         });",
    );
    assert_failed(&output);
    let out = stdout(&output);
    assert!(out.contains("not ok 1 - fails"), "{out}");
    assert!(out.contains("ok 2 - passes"), "{out}");
    assert!(out.contains("must stay failed"), "{out}");
    assert!(
        stderr(&output).contains("1 of 2 test(s) failed"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn array_prototype_mutation_before_registration_cannot_drop_tests() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        "import test from 'neo:test';
         Array.prototype.push = () => 0;
         Object.defineProperty(Array.prototype, '0', {
             set() {}, configurable: true,
         });
         test('fails', () => { throw new Error('must stay failed'); });
         test('passes', () => {});",
    );
    assert_failed(&output);
    let out = stdout(&output);
    assert!(out.contains("not ok 1 - fails"), "{out}");
    assert!(out.contains("ok 2 - passes"), "{out}");
    assert!(
        stderr(&output).contains("1 of 2 test(s) failed"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_preexisting_test_global_is_ignored_without_the_import() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        "globalThis.__neoTestRunPending = () => { throw new Error('must not be probed'); };
         console.log('no-import-ok');",
    );
    assert_ok(&output);
    assert_eq!(stdout(&output), "no-import-ok\n");
    assert_eq!(stderr(&output), "");
}

#[test]
fn a_throwing_message_accessor_does_not_skip_later_tests() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        "import test from 'neo:test';
         test('bad message', () => {
             const error = new Error('base');
             Object.defineProperty(error, 'message', {
                 get() { throw new Error('getter exploded'); },
             });
             throw error;
         });
         test('later still runs', () => {});",
    );
    assert_failed(&output);
    let out = stdout(&output);
    assert!(out.contains("not ok 1 - bad message"), "{out}");
    assert!(out.contains("ok 2 - later still runs"), "{out}");
    assert!(
        stderr(&output).contains("1 of 2 test(s) failed"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn assertion_failures_survive_further_prototype_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        "import test from 'neo:test';
         import {deepEqual} from 'neo:assert';
         test('fails while mutating', () => {
             Array.prototype.push = () => { throw new Error('push disabled'); };
             Function.prototype.call = () => { throw new Error('call disabled'); };
             deepEqual({ a: 1 }, { a: 2 });
         });
         test('later still runs', () => {});",
    );
    assert_failed(&output);
    let out = stdout(&output);
    assert!(out.contains("not ok 1 - fails while mutating"), "{out}");
    assert!(out.contains("ok 2 - later still runs"), "{out}");
    assert!(
        stderr(&output).contains("1 of 2 test(s) failed"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn inherited_descriptor_fields_cannot_hide_failures_from_the_exit_status() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        "import test from 'neo:test';
         Object.defineProperty(Object.prototype, 'get', {
             __proto__: null, value: () => {}, configurable: true, writable: true,
         });
         Object.defineProperty(Object.prototype, 'set', {
             __proto__: null, value: () => {}, configurable: true, writable: true,
         });
         test('first fails', () => { throw new Error('first visible failure'); });
         test('second fails', () => { throw new Error('second visible failure'); });
         test('later runs', () => {});",
    );
    assert_failed(&output);
    let out = stdout(&output);
    assert!(out.contains("not ok 1 - first fails"), "{out}");
    assert!(out.contains("not ok 2 - second fails"), "{out}");
    assert!(out.contains("ok 3 - later runs"), "{out}");
    assert!(out.contains("# 3 tests: 1 passed, 2 failed"), "{out}");
    assert!(
        stderr(&output).contains("2 of 3 test(s) failed"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn inherited_field_setters_cannot_replace_test_or_assertion_errors() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        "const RealError = Error;
         for (const field of ['name', 'code', 'failures', 'operator', 'actual', 'expected']) {
             const descriptor = { __proto__: null, configurable: true };
             descriptor.set = function () {
                 throw new RealError('inherited ' + field + ' setter ran');
             };
             Object.defineProperty(Object.prototype, field, descriptor);
         }
         const testModule = await import('neo:test');
         const assertModule = await import('neo:assert');
         const test = testModule.default;
         test('first fails', () => { throw new RealError('first visible failure'); });
         test('second fails', () => { throw new RealError('second visible failure'); });
         test('later runs', () => {});
         let failure;
         try {
             testModule.run();
         } catch (error) {
             failure = error;
         }
         if (!(failure instanceof testModule.TestFailureError)) {
             throw new RealError('wrong failure type: ' + failure);
         }
         if (failure.code !== 'NEO_TEST_FAILURES') {
             throw new RealError('failure code lost: ' + failure.code);
         }
         if (!(failure.failures instanceof Array) || failure.failures.length !== 2) {
             throw new RealError('failures lost: ' + failure.failures);
         }
         let assertion;
         try {
             assertModule.deepEqual({a: 1}, {a: 2, b: 3});
         } catch (error) {
             assertion = error;
         }
         if (!(assertion instanceof assertModule.AssertionError)) {
             throw new RealError('wrong assertion type: ' + assertion);
         }
         if (assertion.code !== 'ERR_ASSERTION') {
             throw new RealError('assertion code lost: ' + assertion.code);
         }
         if (assertion.actual.a !== 1) throw new RealError('assertion actual lost');
         console.log('fields-ok');",
    );
    assert_failed(&output);
    let out = stdout(&output);
    assert!(out.contains("fields-ok"), "{out}");
    assert!(out.contains("not ok 1 - first fails"), "{out}");
    assert!(out.contains("not ok 2 - second fails"), "{out}");
    assert!(out.contains("ok 3 - later runs"), "{out}");
    assert!(out.contains("# 3 tests: 1 passed, 2 failed"), "{out}");
    assert!(
        stderr(&output).contains("2 of 3 test(s) failed"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn replacing_console_log_does_not_lose_the_runner_output() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        "import test from 'neo:test';
         test('still reported', () => {});
         console.log = () => { throw new Error('swallowed'); };",
    );
    assert_ok(&output);
    assert!(
        stdout(&output).contains("ok 1 - still reported"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn replacing_console_log_before_dynamic_import_can_hide_details_but_not_failure() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        "console.log = () => {};
         const {test} = await import('neo:test');
         test('hidden detail', () => { throw new Error('still fails'); });",
    );
    assert_failed(&output);
    assert_eq!(stdout(&output), "");
    assert!(
        stderr(&output).contains("1 of 1 test(s) failed"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn assertions_work_without_the_test_module() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        "import {deepEqual, equal, throws} from 'neo:assert';
         equal(2, 2);
         deepEqual({a: 1}, {a: 1});
         throws(() => { throw new Error('expected'); });
         console.log('assert-ok');",
    );
    assert_ok(&output);
    assert_eq!(stdout(&output), "assert-ok\n");

    let output = run_cli(
        dir.path(),
        &[],
        "import {equal} from 'neo:assert'; equal(1, 2);",
    );
    assert_failed(&output);
    assert!(
        stderr(&output).contains("expected values to be strictly equal"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn an_imported_module_can_register_tests() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("suite.js"),
        "import test from 'neo:test';
         test('from imported module', () => {});",
    )
    .unwrap();
    let output = run_cli(dir.path(), &["--allow-fs"], "import './suite.js';");
    assert_ok(&output);
    assert!(
        stdout(&output).contains("ok 1 - from imported module"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn dynamic_import_does_not_expose_queue_through_object_create() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &[],
        "const create = Object.create; let captured;
         Object.create = function(proto) { const obj = create(proto); captured = obj; return obj; };
         const {test} = await import('neo:test');
         test('must fail', () => { throw new Error('real failure'); });
         if (captured && captured[0]) captured[0].fn = () => {};",
    );
    assert_failed(&output);
    assert!(
        stdout(&output).contains("not ok 1 - must fail"),
        "{}",
        stdout(&output)
    );
}
