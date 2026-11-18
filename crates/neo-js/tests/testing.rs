//! Integration tests for `neo:test` and `neo:assert` through the public
//! `NeoJs` API.
//!
//! These exercise the modules the way an embedding application does: evaluate
//! a module, call `run_pending_tests`, and inspect the report. No host
//! permission is enabled; the test modules must work without one.

use neo_js::{NeoJs, Permissions, TestReport};

fn runtime() -> NeoJs {
    NeoJs::new(Permissions::default()).unwrap()
}

#[test]
fn run_pending_tests_is_none_until_the_module_is_imported() {
    let js = runtime();
    assert_eq!(js.run_pending_tests().unwrap(), None);

    // A preexisting, unrelated global must not be probed or parsed.
    js.run("globalThis.__neoTestRunPending = () => { throw new Error('must not be probed'); };")
        .unwrap();
    assert_eq!(js.run_pending_tests().unwrap(), None);

    js.run_module(
        "assert-only.js",
        "import {equal} from 'neo:assert'; equal(1, 1);",
    )
    .unwrap();
    assert_eq!(js.run_pending_tests().unwrap(), None);
}

#[test]
fn run_pending_tests_reports_counts_and_retains_failures() {
    let js = runtime();
    js.run_module(
        "tests.js",
        "import test from 'neo:test';
         import {equal} from 'neo:assert';
         test('first passes', () => equal(1, 1));
         test('second fails', () => equal(1, 2));
         test('third passes', () => equal('a', 'a'));",
    )
    .unwrap();

    let report = js.run_pending_tests().unwrap().unwrap();
    assert_eq!(report.total, 3);
    assert_eq!(report.passed, 2);
    assert_eq!(report.failed, 1);
    assert_eq!(report.failures.len(), 1);
    assert!(
        report.failures[0].contains("second fails"),
        "{:?}",
        report.failures
    );
    assert!(
        report.failures[0].contains("ERR_ASSERTION"),
        "{:?}",
        report.failures
    );
    assert!(!report.all_passed());

    // The report is cumulative and a second call does not rerun the tests.
    assert_eq!(js.run_pending_tests().unwrap(), Some(report));
}

#[test]
fn tests_registered_later_run_on_the_next_call() {
    let js = runtime();
    js.run_module(
        "one.js",
        "import test from 'neo:test'; test('one', () => {});",
    )
    .unwrap();
    assert_eq!(
        js.run_pending_tests().unwrap(),
        Some(TestReport {
            total: 1,
            passed: 1,
            failed: 0,
            failures: Vec::new(),
        })
    );

    js.run_module(
        "two.js",
        "import test from 'neo:test'; test('two', () => {});",
    )
    .unwrap();
    let report = js.run_pending_tests().unwrap().unwrap();
    assert_eq!(report.total, 2);
    assert_eq!(report.passed, 2);
}

#[test]
fn invalid_registrations_are_rejected_at_their_source() {
    let js = runtime();
    let error = js
        .run_module(
            "bad-name.js",
            "import test from 'neo:test'; test('   ', () => {});",
        )
        .unwrap_err();
    assert!(error.contains("non-empty string"), "{error}");

    let error = js
        .run_module(
            "bad-callback.js",
            "import test from 'neo:test'; test('name', 42);",
        )
        .unwrap_err();
    assert!(error.contains("must be a function"), "{error}");

    let error = js
        .run_module(
            "limit.js",
            "import {test, MAX_TESTS} from 'neo:test';
             for (let i = 0; i < MAX_TESTS; i++) test('test ' + i, () => {});
             test('one too many', () => {});",
        )
        .unwrap_err();
    assert!(error.contains("cannot register more than"), "{error}");
}

#[test]
fn assertion_semantics_are_synchronous_and_strict() {
    let js = runtime();
    js.run_module(
        "assertions.js",
        "import {
             AssertionError,
             deepEqual,
             doesNotThrow,
             equal,
             fail,
             notDeepEqual,
             notEqual,
             ok,
             throws,
         } from 'neo:assert';

         equal(NaN, NaN);
         notEqual(-0, 0);
         notEqual('1', 1);
         ok(1);

         const cyclic = { value: 1 };
         cyclic.self = cyclic;
         const cyclicCopy = { value: 1 };
         cyclicCopy.self = cyclicCopy;
         deepEqual(cyclic, cyclicCopy);
         notDeepEqual(cyclic, { value: 1, self: {} });

         deepEqual(new Date(0), new Date(0));
         notDeepEqual(new Date(0), new Date(1));
         deepEqual(/a+/g, /a+/g);
         notDeepEqual(/a+/g, /a+/i);
         const regexpA = /a/g;
         const regexpB = /a/g;
         deepEqual(regexpA, regexpB);
         regexpA.exec('a');
         notDeepEqual(regexpA, regexpB);
         regexpB.exec('a');
         deepEqual(regexpA, regexpB);
         notDeepEqual([], Array(1));
         notDeepEqual(Array(2), Array(1));
         deepEqual(Array(2), Array(2));
         deepEqual(new Map([[{ k: 1 }, new Set([1, 2])]]), new Map([[{ k: 1 }, new Set([2, 1])]]));
         notDeepEqual(new Map([[1, 2]]), new Map([[1, 3]]));
         deepEqual(new Uint8Array([1, 2, 3]).subarray(1), new Uint8Array([2, 3]));
         deepEqual(
             new Uint8Array([0, 1, 2, 3]).subarray(1, 3),
             new Uint8Array([9, 1, 2]).subarray(1, 3),
         );
         notDeepEqual(new Uint8Array([0, 1, 2, 3]).subarray(1, 3), new Uint8Array([2, 3]));
         notDeepEqual(new Uint8Array([1, 2]), new Uint8Array([1, 3]));
         const selfMapA = new Map();
         selfMapA.set('self', selfMapA);
         const selfMapB = new Map();
         selfMapB.set('self', selfMapB);
         deepEqual(selfMapA, selfMapB);
         deepEqual(new DataView(new Uint8Array([0, 9]).buffer, 1, 1), new DataView(new Uint8Array([9]).buffer));
         deepEqual(new Error('x'), new Error('x'));
         notDeepEqual(new Error('x'), new Error('y'));
         deepEqual(new Number(3), new Number(3));
         notDeepEqual(new Number(3), new Number(4));
         notDeepEqual(Object.create(null), {});
         notDeepEqual(() => 1, () => 1);
         const promise = Promise.resolve(1);
         deepEqual(promise, promise);
         notDeepEqual(Promise.resolve(1), Promise.resolve(1));
         const symbol = Symbol('key');
         deepEqual({ [symbol]: 1 }, { [symbol]: 1 });
         notDeepEqual({ [symbol]: 1 }, { [Symbol('key')]: 1 });
         deepEqual([1, , 3], [1, , 3]);
         notDeepEqual([1, , 3], [1, undefined, 3]);

         const spoofedMap = { [Symbol.toStringTag]: 'Map' };
         deepEqual(spoofedMap, { [Symbol.toStringTag]: 'Map' });
         notDeepEqual(spoofedMap, new Map());

         const originalIs = Object.is;
         Object.is = () => true;
         try {
             equal(1, 2);
             throw new Error('equal used a replaced Object.is');
         } catch (error) {
             if (!(error instanceof AssertionError)) throw error;
         }
         Object.is = originalIs;

         try {
             deepEqual({ a: [1, { b: 2 }] }, { a: [1, { b: 3 }] });
             throw new Error('deepEqual accepted different values');
         } catch (error) {
             if (!(error instanceof AssertionError)) throw error;
             if (error.code !== 'ERR_ASSERTION') throw error;
             if (!error.message.includes('$.a[1].b')) throw error;
         }

         const thrown = throws(() => { throw new RangeError('bad range'); });
         if (!(thrown instanceof RangeError)) throw new Error('throws did not return the error');
         throws(() => { throw new RangeError('bad range'); }, RangeError);
         throws(() => { throw new Error('boom 42'); }, /42/);
         throws(() => { throw 'plain'; }, /plain/);
         throws(() => equal(1, 1, 7), TypeError);
         throws(() => throws(() => { throw new Error('x'); }, 'message'), TypeError);
         throws(() => doesNotThrow(() => Promise.resolve()), TypeError);
         throws(() => throws((function* () {}).bind(null)), TypeError);
         throws(() => doesNotThrow((function* () {}).bind(null)), TypeError);

         if (doesNotThrow(() => 7) !== 7) throw new Error('doesNotThrow return value');

         const getter = {};
         Object.defineProperty(getter, 'x', {
             enumerable: true,
             get() { throw new Error('getter boom'); },
         });
         try {
             deepEqual(getter, { x: 1 });
             throw new Error('deepEqual did not invoke the getter');
         } catch (error) {
             if (error.message !== 'getter boom') throw error;
         }

         throws(() => throws(() => 1), AssertionError);
         throws(() => doesNotThrow(() => { throw new Error('no'); }), AssertionError);
         throws(() => fail('manual failure'), AssertionError);
         throws(() => ok(false), AssertionError);
         throws(() => deepEqual(1, 2), AssertionError);
         throws(() => notDeepEqual(1, 1), AssertionError);
         throws(() => notEqual(1, 1), AssertionError);",
    )
    .unwrap();
    assert_eq!(js.run_pending_tests().unwrap(), None);
}

#[test]
fn callback_guards_reject_disguised_brands_without_inspecting_user_properties() {
    let js = runtime();
    js.run_module(
        "callback-brands.js",
        r#"let observed = 0;
           function trap() { observed++; throw new Error('must not inspect or resume'); }
           const ownDescriptor = Object.getOwnPropertyDescriptor;
           Object.getOwnPropertyDescriptor = trap;
           const {default: test} = await import('neo:test');
           const assertions = await import('neo:assert');
           const {equal, throws, doesNotThrow, AssertionError} = assertions;
           equal(Object.keys(assertions).length, 9);
           equal(Object.getOwnPropertyDescriptor, trap);
           Object.getOwnPropertyDescriptor = ownDescriptor;

           const cases = [
               ['promise', 'a Promise', () => {
                   const p = Promise.reject('private reason');
                   Object.defineProperties(p, {
                       then: {get: trap},
                       constructor: {get: trap},
                       [Symbol.toStringTag]: {get: trap},
                   });
                   return p;
               }],
               ['generator', 'a generator', () => {
                   const g = (function* () { trap(); })();
                   Object.defineProperty(g, Symbol.toStringTag, {get: trap});
                   return Object.setPrototypeOf(g, null);
               }],
               ['async generator', 'a generator', () => {
                   const g = (async function* () { trap(); })();
                   return Object.setPrototypeOf(g, null);
               }],
               ['thenable', 'a thenable', () => Object.create({then: trap})],
               ['accessor', 'a value with a then accessor', () => Object.create({get then() { trap(); }})],
               ['proxy', 'a value with a proxy', () => new Proxy(Promise.resolve(), {
                   get: trap, getOwnPropertyDescriptor: trap, getPrototypeOf: trap,
               })],
               ['revoked proxy', 'a value with a proxy', () => {
                   const {proxy, revoke} = Proxy.revocable({}, {});
                   revoke();
                   return proxy;
               }],
               ['proxy prototype', 'a value with a proxy', () => Object.create(new Proxy({}, {
                   get: trap, getOwnPropertyDescriptor: trap, getPrototypeOf: trap,
               }))],
           ];
           for (const [name, kind, callback] of cases) {
               for (const assertCallback of [throws, doesNotThrow]) {
                   const error = throws(() => assertCallback(callback), TypeError);
                   if (!error.message.includes('returned ' + kind)) throw error;
               }
               test(name, callback);
           }

           const callback = () => 42;
           Object.defineProperties(callback, {
               constructor: {get: trap},
               [Symbol.toStringTag]: {get: trap},
           });
           equal(doesNotThrow(callback), 42);
           test('ordinary callback with accessors', callback);
           for (const fn of [async () => trap(), function* () { trap(); },
                             async function* () { trap(); }]) {
               Object.defineProperty(fn, Symbol.toStringTag, {get: trap});
               Object.setPrototypeOf(fn, null);
               equal(throws(() => test('invalid callback', fn), TypeError).code, 'NEO_TEST_ASYNC');
               throws(() => doesNotThrow(fn), TypeError);
           }
           const proxyCallback = new Proxy(() => {}, {apply: trap, get: trap});
           equal(throws(() => test('proxy callback', proxyCallback), TypeError).code, 'NEO_TEST_ASYNC');
           throws(() => throws(proxyCallback), TypeError);

           const ordinary = Object.create({get then() { trap(); }});
           Object.defineProperties(ordinary, {
               then: {value: false},
               [Symbol.toStringTag]: {get: trap},
               detail: {get: trap, enumerable: true},
           });
           equal(doesNotThrow(() => ordinary), ordinary);
           equal(throws(() => throws(() => ordinary), AssertionError).actual, ordinary);
           const fakeGenerator = {[Symbol.toStringTag]: 'Generator'};
           equal(doesNotThrow(() => fakeGenerator), fakeGenerator);
           test('ordinary return', () => ordinary);
           test('queues unrelated work', () => {
               Promise.resolve().then(() => { observed++; });
           });
           test('no getters, traps, generator body, or pending jobs ran', () => equal(observed, 0));"#,
    )
    .unwrap();
    let report = js.run_pending_tests().unwrap().unwrap();
    assert_eq!((report.total, report.passed, report.failed), (12, 4, 8));
    for (summary, expected) in report.failures.iter().zip([
        "a Promise",
        "a generator",
        "a generator",
        "a thenable",
        "a value with a then accessor",
        "a value with a proxy",
        "a value with a proxy",
        "a value with a proxy",
    ]) {
        assert!(summary.contains("NEO_TEST_ASYNC"), "{summary}");
        assert!(
            summary.contains(&format!("returned {expected}")),
            "{summary}"
        );
        assert!(!summary.contains("private reason"), "{summary}");
    }
}

#[test]
fn failure_summaries_replace_lone_surrogates_and_do_not_split_emoji() {
    let js = runtime();
    js.run_module(
        "unicode-summary.js",
        r#"String.prototype.charCodeAt = () => { throw new Error('mutated charCodeAt'); };
           String.prototype.toWellFormed = () => { throw new Error('mutated toWellFormed'); };
           const {default: test, run, TestFailureError} = await import('neo:test');
           test('bad \ud800 name \udfff', () => Promise.reject('private reason'));
           test('x'.repeat(293) + '🦊end', () => Promise.resolve());
           test('unreadable message', () => {
               const error = new Error();
               Object.defineProperty(error, 'message', {
                   get() { throw new Error('formatter failed'); },
               });
               throw error;
           });
           let ran = false;
           test('later', () => { ran = true; });
           let failure;
           try { run(); } catch (error) { failure = error; }
           if (!ran || !(failure instanceof TestFailureError) || failure.failures.length !== 3)
               throw new Error('incomplete test report');"#,
    )
    .unwrap();
    let report = js.run_pending_tests().unwrap().unwrap();
    assert_eq!((report.total, report.passed, report.failed), (4, 1, 3));
    assert_eq!(
        report.failures[0],
        "bad � name �: TypeError [NEO_TEST_ASYNC]: test \"bad � name �\" returned a Promise; neo:test runs synchronous tests only"
    );
    let prefix = "x".repeat(293);
    assert_eq!(
        report.failures[1],
        format!("{prefix}🦊end: TypeError [NEO_TEST_ASYNC]: test \"{prefix}...")
    );
    assert_eq!(
        report.failures[2],
        "unreadable message: Error: <message unreadable>"
    );
}

#[test]
fn assertion_messages_show_object_content_and_only_real_cycles() {
    let js = runtime();
    js.run_module(
        "formatting.js",
        r#"import {deepEqual} from 'neo:assert';

           function messageFor(actual, expected) {
             try {
               deepEqual(actual, expected);
             } catch (error) {
               return String(error.message);
             }
             throw new Error('deepEqual accepted different values');
           }

           const plain = messageFor({a: 1}, {a: 2, b: 3});
           if (!plain.includes('actual: { .a: 1 }'))
             throw new Error('actual object content missing: ' + plain);
           if (!plain.includes('expected: { .a: 2, .b: 3 }'))
             throw new Error('expected object content missing: ' + plain);
           if (plain.includes('[Circular]'))
             throw new Error('an acyclic object was reported as circular: ' + plain);

           const shared = {x: 1};
           const repeated = messageFor({a: shared, b: shared}, {a: shared, b: shared, c: 1});
           if (!repeated.includes('.a: { .x: 1 }') || !repeated.includes('.b: { .x: 1 }'))
             throw new Error('repeated reference content missing: ' + repeated);
           if (repeated.includes('[Circular]'))
             throw new Error('a repeated reference was reported as circular: ' + repeated);

           const cyclic = {value: 1};
           cyclic.self = cyclic;
           const cycle = messageFor(cyclic, {value: 1, self: {value: 1}});
           if (!cycle.includes('.self: [Circular]'))
             throw new Error('a real cycle was not reported: ' + cycle);
           if (!cycle.includes('.value: 1'))
             throw new Error('cyclic object content missing: ' + cycle);"#,
    )
    .unwrap();
    assert_eq!(js.run_pending_tests().unwrap(), None);
}

#[test]
fn assertion_error_fields_ignore_inherited_setters_before_import() {
    let js = runtime();
    js.run_module(
        "assert-fields-before.js",
        r#"
        const RealError = Error;
        const FIELDS = ['name', 'code', 'operator', 'actual', 'expected'];
        function install(mode) {
            for (const field of FIELDS) {
                const descriptor = { __proto__: null, configurable: true };
                descriptor.set = mode === 'throwing'
                    ? function () { throw new RealError('inherited ' + field + ' setter ran'); }
                    : function () {};
                Object.defineProperty(Object.prototype, field, descriptor);
            }
        }

        install('throwing');
        const {AssertionError, deepEqual, equal, fail} = await import('neo:assert');

        function expectOwn(error, field, value) {
            const descriptor = Object.getOwnPropertyDescriptor(error, field);
            if (!descriptor || descriptor.value !== value ||
                descriptor.enumerable !== true || descriptor.writable !== true ||
                descriptor.configurable !== true) {
                throw new RealError('field ' + field + ' is not an own data property');
            }
            if (!Object.keys(error).includes(field))
                throw new RealError('field ' + field + ' is not enumerable');
        }

        function expectAssertion(fn, operator) {
            let failure;
            try {
                fn();
            } catch (error) {
                failure = error;
            }
            if (!(failure instanceof AssertionError))
                throw new RealError('not an AssertionError: ' + failure);
            if (!(failure instanceof RealError)) throw new RealError('not an Error');
            if (failure.name !== 'AssertionError') throw new RealError('name lost: ' + failure.name);
            if (failure.code !== 'ERR_ASSERTION') throw new RealError('code lost: ' + failure.code);
            if (failure.operator !== operator) throw new RealError('operator lost: ' + failure.operator);
            expectOwn(failure, 'name', 'AssertionError');
            expectOwn(failure, 'code', 'ERR_ASSERTION');
            expectOwn(failure, 'operator', operator);
            return failure;
        }

        function runPhase(label) {
            const equalFailure = expectAssertion(() => equal(1, 2), 'equal');
            expectOwn(equalFailure, 'actual', 1);
            expectOwn(equalFailure, 'expected', 2);
            const deepFailure = expectAssertion(() => deepEqual({a: 1}, {a: 2, b: 3}), 'deepEqual');
            if (deepFailure.actual.a !== 1 || deepFailure.expected.a !== 2)
                throw new RealError(label + ': deepEqual pair lost');
            expectOwn(deepFailure, 'actual', deepFailure.actual);
            expectOwn(deepFailure, 'expected', deepFailure.expected);
            const failFailure = expectAssertion(() => fail('manual ' + label), 'fail');
            if (failFailure.message !== 'manual ' + label)
                throw new RealError(label + ': message lost');
        }

        runPhase('throwing');
        install('noop');
        runPhase('noop');"#,
    )
    .unwrap();
    assert_eq!(js.run_pending_tests().unwrap(), None);
}

#[test]
fn assertion_error_fields_ignore_inherited_setters_after_import() {
    let js = runtime();
    js.run_module(
        "assert-fields-after.js",
        r#"
        import {AssertionError, deepEqual, equal, fail} from 'neo:assert';
        const RealError = Error;
        const FIELDS = ['name', 'code', 'operator', 'actual', 'expected'];
        function install(mode) {
            for (const field of FIELDS) {
                const descriptor = { __proto__: null, configurable: true };
                descriptor.set = mode === 'throwing'
                    ? function () { throw new RealError('inherited ' + field + ' setter ran'); }
                    : function () {};
                Object.defineProperty(Object.prototype, field, descriptor);
            }
        }

        function expectOwn(error, field, value) {
            const descriptor = Object.getOwnPropertyDescriptor(error, field);
            if (!descriptor || descriptor.value !== value ||
                descriptor.enumerable !== true || descriptor.writable !== true ||
                descriptor.configurable !== true) {
                throw new RealError('field ' + field + ' is not an own data property');
            }
        }

        function expectAssertion(fn, operator) {
            let failure;
            try {
                fn();
            } catch (error) {
                failure = error;
            }
            if (!(failure instanceof AssertionError))
                throw new RealError('not an AssertionError: ' + failure);
            if (!(failure instanceof RealError)) throw new RealError('not an Error');
            if (failure.code !== 'ERR_ASSERTION') throw new RealError('code lost: ' + failure.code);
            if (failure.operator !== operator) throw new RealError('operator lost: ' + failure.operator);
            expectOwn(failure, 'name', 'AssertionError');
            expectOwn(failure, 'code', 'ERR_ASSERTION');
            expectOwn(failure, 'operator', operator);
            return failure;
        }

        function runPhase(label) {
            const equalFailure = expectAssertion(() => equal(1, 2), 'equal');
            expectOwn(equalFailure, 'actual', 1);
            expectOwn(equalFailure, 'expected', 2);
            const deepFailure = expectAssertion(() => deepEqual({a: 1}, {a: 2, b: 3}), 'deepEqual');
            if (deepFailure.actual.a !== 1 || deepFailure.expected.a !== 2)
                throw new RealError(label + ': deepEqual pair lost');
            expectOwn(deepFailure, 'actual', deepFailure.actual);
            expectOwn(deepFailure, 'expected', deepFailure.expected);
        }

        install('noop');
        runPhase('noop');
        install('throwing');
        runPhase('throwing');"#,
    )
    .unwrap();
    assert_eq!(js.run_pending_tests().unwrap(), None);
}

#[test]
fn test_failure_error_fields_ignore_inherited_setters_before_import() {
    let js = runtime();
    js.run_module(
        "test-fields-before.js",
        r#"
        const RealError = Error;
        const FIELDS = ['name', 'code', 'failures'];
        function install(mode) {
            for (const field of FIELDS) {
                const descriptor = { __proto__: null, configurable: true };
                descriptor.set = mode === 'throwing'
                    ? function () { throw new RealError('inherited ' + field + ' setter ran'); }
                    : function () {};
                Object.defineProperty(Object.prototype, field, descriptor);
            }
        }

        install('throwing');
        const {default: test, run, TestFailureError} = await import('neo:test');

        function expectOwn(error, field, value) {
            const descriptor = Object.getOwnPropertyDescriptor(error, field);
            if (!descriptor || descriptor.value !== value ||
                descriptor.enumerable !== true || descriptor.writable !== true ||
                descriptor.configurable !== true) {
                throw new RealError('field ' + field + ' is not an own data property');
            }
            if (!Object.keys(error).includes(field))
                throw new RealError('field ' + field + ' is not enumerable');
        }

        function registerBatch(label) {
            test(label + ' first', () => { throw new RealError(label + ' first failure'); });
            test(label + ' second', () => { throw new RealError(label + ' second failure'); });
            test(label + ' later', () => {});
        }

        function runAndCheck(label) {
            let failure;
            try {
                run();
            } catch (error) {
                failure = error;
            }
            if (!(failure instanceof TestFailureError))
                throw new RealError('not a TestFailureError: ' + failure);
            if (!(failure instanceof RealError)) throw new RealError('not an Error');
            if (failure.name !== 'TestFailureError')
                throw new RealError('name lost: ' + failure.name);
            if (failure.code !== 'NEO_TEST_FAILURES')
                throw new RealError('code lost: ' + failure.code);
            if (!(failure.failures instanceof Array) || failure.failures.length !== 2)
                throw new RealError('failures lost: ' + failure.failures);
            if (!failure.failures[0].includes(label + ' first failure') ||
                !failure.failures[1].includes(label + ' second failure'))
                throw new RealError('summaries lost: ' + failure.failures);
            let registrationFailure;
            try {
                test('async ' + label, async function () {});
            } catch (error) {
                registrationFailure = error;
            }
            if (!(registrationFailure instanceof RealError) ||
                registrationFailure.code !== 'NEO_TEST_ASYNC') {
                throw new RealError('registration code lost in phase ' + label);
            }
            expectOwn(failure, 'name', 'TestFailureError');
            expectOwn(failure, 'code', 'NEO_TEST_FAILURES');
            expectOwn(failure, 'failures', failure.failures);
        }

        registerBatch('throwing');
        runAndCheck('throwing');
        install('noop');
        registerBatch('noop');
        runAndCheck('noop');"#,
    )
    .unwrap();

    let report = js.run_pending_tests().unwrap().unwrap();
    assert_eq!(report.total, 6);
    assert_eq!(report.passed, 2);
    assert_eq!(report.failed, 4);
    assert_eq!(report.failures.len(), 4);
    assert!(
        report.failures[0].contains("throwing first failure"),
        "{:?}",
        report.failures
    );
    assert!(
        report.failures[1].contains("throwing second failure"),
        "{:?}",
        report.failures
    );
    assert!(
        report.failures[2].contains("noop first failure"),
        "{:?}",
        report.failures
    );
    assert!(
        report.failures[3].contains("noop second failure"),
        "{:?}",
        report.failures
    );
}

#[test]
fn test_failure_error_fields_ignore_inherited_setters_after_import() {
    let js = runtime();
    js.run_module(
        "test-fields-after.js",
        r#"
        import test, {run, TestFailureError} from 'neo:test';
        const RealError = Error;
        const FIELDS = ['name', 'code', 'failures'];
        function install(mode) {
            for (const field of FIELDS) {
                const descriptor = { __proto__: null, configurable: true };
                descriptor.set = mode === 'throwing'
                    ? function () { throw new RealError('inherited ' + field + ' setter ran'); }
                    : function () {};
                Object.defineProperty(Object.prototype, field, descriptor);
            }
        }

        function expectOwn(error, field, value) {
            const descriptor = Object.getOwnPropertyDescriptor(error, field);
            if (!descriptor || descriptor.value !== value ||
                descriptor.enumerable !== true || descriptor.writable !== true ||
                descriptor.configurable !== true) {
                throw new RealError('field ' + field + ' is not an own data property');
            }
        }

        function registerBatch(label) {
            test(label + ' first', () => { throw new RealError(label + ' first failure'); });
            test(label + ' second', () => { throw new RealError(label + ' second failure'); });
            test(label + ' later', () => {});
        }

        function runAndCheck(label) {
            let failure;
            try {
                run();
            } catch (error) {
                failure = error;
            }
            if (!(failure instanceof TestFailureError))
                throw new RealError('not a TestFailureError: ' + failure);
            if (!(failure instanceof RealError)) throw new RealError('not an Error');
            if (failure.code !== 'NEO_TEST_FAILURES')
                throw new RealError('code lost: ' + failure.code);
            if (!(failure.failures instanceof Array) || failure.failures.length !== 2)
                throw new RealError('failures lost: ' + failure.failures);
            if (!failure.failures[0].includes(label + ' first failure') ||
                !failure.failures[1].includes(label + ' second failure'))
                throw new RealError('summaries lost: ' + failure.failures);
            let registrationFailure;
            try {
                test('async ' + label, async function () {});
            } catch (error) {
                registrationFailure = error;
            }
            if (!(registrationFailure instanceof RealError) ||
                registrationFailure.code !== 'NEO_TEST_ASYNC') {
                throw new RealError('registration code lost in phase ' + label);
            }
            expectOwn(failure, 'name', 'TestFailureError');
            expectOwn(failure, 'code', 'NEO_TEST_FAILURES');
            expectOwn(failure, 'failures', failure.failures);
        }

        install('noop');
        registerBatch('noop');
        runAndCheck('noop');
        install('throwing');
        registerBatch('throwing');
        runAndCheck('throwing');"#,
    )
    .unwrap();

    let report = js.run_pending_tests().unwrap().unwrap();
    assert_eq!(report.total, 6);
    assert_eq!(report.passed, 2);
    assert_eq!(report.failed, 4);
    assert_eq!(report.failures.len(), 4);
    assert!(
        report.failures[0].contains("noop first failure"),
        "{:?}",
        report.failures
    );
    assert!(
        report.failures[1].contains("noop second failure"),
        "{:?}",
        report.failures
    );
    assert!(
        report.failures[2].contains("throwing first failure"),
        "{:?}",
        report.failures
    );
    assert!(
        report.failures[3].contains("throwing second failure"),
        "{:?}",
        report.failures
    );
}

#[test]
fn tampering_with_json_does_not_hide_test_failures() {
    let js = runtime();
    js.run_module(
        "tampered-json.js",
        "import test from 'neo:test';
         test('fails anyway', () => { throw new Error('visible failure'); });
         JSON.stringify = () => '{\"total\":0,\"passed\":0,\"failed\":0,\"failures\":[]}';",
    )
    .unwrap();
    let report = js.run_pending_tests().unwrap().unwrap();
    assert_eq!(report.total, 1);
    assert_eq!(report.failed, 1);
    assert!(
        report.failures[0].contains("visible failure"),
        "{:?}",
        report.failures
    );
}

#[test]
fn unrelated_global_reports_are_ignored_without_neo_test() {
    let js = runtime();
    js.run(
        "globalThis.__neoTestRunPending = () => '{\"total\":0,\"passed\":0,\"failed\":0,\"failures\":[]}';
         globalThis.__neoTestReport = { forged: true };",
    )
    .unwrap();
    assert_eq!(js.run_pending_tests().unwrap(), None);
}

#[test]
fn prototype_mutation_before_registration_cannot_skip_or_forge_failures() {
    let js = runtime();
    js.run_module(
        "prototypes.js",
        "import test from 'neo:test';
         Array.prototype.push = () => 0;
         Object.defineProperty(Array.prototype, '0', {
             set() {}, configurable: true,
         });
         test('fails', () => { throw new Error('must stay failed'); });
         test('passes', () => {});",
    )
    .unwrap();
    let report = js.run_pending_tests().unwrap().unwrap();
    assert_eq!(report.total, 2);
    assert_eq!(report.passed, 1);
    assert_eq!(report.failed, 1);
    assert!(
        report.failures[0].contains("must stay failed"),
        "{:?}",
        report.failures
    );
}

#[test]
fn prototype_mutation_after_registration_cannot_skip_or_forge_failures() {
    let js = runtime();
    js.run_module(
        "prototypes-after.js",
        "import test from 'neo:test';
         test('fails', () => { throw new Error('must stay failed'); });
         test('passes', () => {});
         Array.prototype.splice = () => [];
         Object.prototype.toJSON = () => ({
             total: 0, passed: 0, failed: 0, failures: []
         });",
    )
    .unwrap();
    let report = js.run_pending_tests().unwrap().unwrap();
    assert_eq!(report.total, 2);
    assert_eq!(report.passed, 1);
    assert_eq!(report.failed, 1);
    assert!(
        report.failures[0].contains("must stay failed"),
        "{:?}",
        report.failures
    );
}

#[test]
fn a_throwing_message_accessor_does_not_skip_later_tests() {
    let js = runtime();
    js.run_module(
        "bad-message.js",
        "import test from 'neo:test';
         test('bad message', () => {
             const error = new Error('base');
             Object.defineProperty(error, 'message', {
                 get() { throw new Error('getter exploded'); },
             });
             throw error;
         });
         test('later', () => {});",
    )
    .unwrap();
    let report = js.run_pending_tests().unwrap().unwrap();
    assert_eq!(report.total, 2);
    assert_eq!(report.passed, 1);
    assert_eq!(report.failed, 1);
    assert!(
        report.failures[0].contains("bad message"),
        "{:?}",
        report.failures
    );
}

#[test]
fn assertion_failures_survive_further_prototype_mutation() {
    let js = runtime();
    js.run_module(
        "mutation-during-tests.js",
        "import test from 'neo:test';
         import {deepEqual} from 'neo:assert';
         test('fails while mutating', () => {
             Array.prototype.push = () => { throw new Error('push disabled'); };
             Function.prototype.call = () => { throw new Error('call disabled'); };
             deepEqual({ a: 1 }, { a: 2 });
         });
         test('later', () => {});",
    )
    .unwrap();
    let report = js.run_pending_tests().unwrap().unwrap();
    assert_eq!(report.total, 2);
    assert_eq!(report.passed, 1);
    assert_eq!(report.failed, 1);
    assert!(
        report.failures[0].contains("fails while mutating"),
        "{:?}",
        report.failures
    );
}

#[test]
fn array_prototype_index_setter_cannot_cause_false_deep_equality() {
    let js = runtime();
    js.run_module(
        "index-setter.js",
        "import {deepEqual, notDeepEqual} from 'neo:assert';
         Object.defineProperty(Array.prototype, '0', {
             set() {}, configurable: true,
         });
         const key = Symbol('k');
         deepEqual({ [key]: 1 }, { [key]: 1 });
         notDeepEqual({ [key]: 1 }, { [key]: 2 });
         notDeepEqual([1], [2]);
         deepEqual([1, 2], [1, 2]);",
    )
    .unwrap();
    assert_eq!(js.run_pending_tests().unwrap(), None);
}

#[test]
fn inherited_descriptor_fields_cannot_hide_failures_or_break_deep_equality() {
    let js = runtime();
    js.run_module(
        "descriptor-inheritance.js",
        r#"import test, {TestFailureError, run} from 'neo:test';
           import {AssertionError, deepEqual, notDeepEqual} from 'neo:assert';

           const get = Object.getOwnPropertyDescriptor(Object.prototype, 'get');
           const set = Object.getOwnPropertyDescriptor(Object.prototype, 'set');
           try {
             // Descriptor literals passed to defineProperty must not inherit
             // these fields, or the runner's own bookkeeping breaks.
             Object.defineProperty(Object.prototype, 'get', {
               __proto__: null, value: () => {}, configurable: true, writable: true,
             });
             Object.defineProperty(Object.prototype, 'set', {
               __proto__: null, value: () => {}, configurable: true, writable: true,
             });

             const key = Symbol('key');
             deepEqual({a: [1, 2], [key]: 3}, {a: [1, 2], [key]: 3});
             notDeepEqual({a: [1, 2]}, {a: [1, 3]});
             let mismatch;
             try {
               deepEqual({a: 1}, {a: 2});
             } catch (error) {
               mismatch = error;
             }
             if (!(mismatch instanceof AssertionError) || mismatch.code !== 'ERR_ASSERTION')
               throw new Error('deepEqual did not report a mismatch as an AssertionError');

             test('first fails', () => { throw new Error('first visible failure'); });
             test('second fails', () => { throw new Error('second visible failure'); });
             test('later runs', () => {});

             let failure;
             try {
               run();
             } catch (error) {
               failure = error;
             }
             if (!(failure instanceof TestFailureError))
               throw new Error('run did not throw TestFailureError: ' + failure);
             if (failure.failures.length !== 2)
               throw new Error('script saw ' + failure.failures.length + ' failure(s)');
             if (!failure.failures[0].includes('first fails') ||
                 !failure.failures[0].includes('first visible failure'))
               throw new Error('first summary missing: ' + failure.failures);
             if (!failure.failures[1].includes('second fails') ||
                 !failure.failures[1].includes('second visible failure'))
               throw new Error('second summary missing: ' + failure.failures);
           } finally {
             delete Object.prototype.get;
             delete Object.prototype.set;
             if (get) Object.defineProperty(Object.prototype, 'get', get);
             if (set) Object.defineProperty(Object.prototype, 'set', set);
           }"#,
    )
    .unwrap();

    let report = js.run_pending_tests().unwrap().unwrap();
    assert_eq!(report.total, 3);
    assert_eq!(report.passed, 1);
    assert_eq!(report.failed, 2);
    assert_eq!(report.failures.len(), 2);
    assert!(
        report.failures[0].contains("first visible failure"),
        "{:?}",
        report.failures
    );
    assert!(
        report.failures[1].contains("second visible failure"),
        "{:?}",
        report.failures
    );
    assert!(!report.all_passed());
}

#[test]
fn the_host_runner_cannot_be_replaced_by_the_script() {
    let js = runtime();
    js.run_module(
        "tamper.js",
        "import test from 'neo:test';
         test('still runs', () => {});
         let blocked = false;
         try {
             globalThis.__neoTestInstall(() => {});
         } catch (_) {
             blocked = true;
         }
         if (!blocked) throw new Error('a second runner was installed');
         globalThis.__neoTestInstall = () => {};
         globalThis.__neoTestCounts = () => {};
         globalThis.__neoTestFailure = () => {};",
    )
    .unwrap();
    let report = js.run_pending_tests().unwrap().unwrap();
    assert_eq!(report.total, 1);
    assert_eq!(report.passed, 1);
}

#[test]
fn preimport_globals_cannot_install_or_forge_a_report() {
    let js = runtime();
    js.run("globalThis.__neoTestInstall = () => {}; globalThis.__neoTestCounts = () => {}; globalThis.__neoTestFailure = () => {}; globalThis.__neoTempTestInstall = () => {};").unwrap();
    assert_eq!(js.run_pending_tests().unwrap(), None);
    js.run_module("spoof.js", "import test from 'neo:test'; test('real failure', () => { throw new Error('real'); }); globalThis.__neoTestCounts(1,1,0);").unwrap();
    let report = js.run_pending_tests().unwrap().unwrap();
    assert_eq!((report.total, report.passed, report.failed), (1, 0, 1));
    assert!(report.failures[0].contains("real failure"));
}

#[test]
fn bridge_callbacks_are_not_exposed_and_fabricated_globals_are_ignored() {
    let js = runtime();
    js.run("for (const name of ['__neoTestInstall', '__neoTestCounts', '__neoTestFailure', '__neoTempTestInstall', '__neoTempTestCounts', '__neoTempTestFailure']) { if (typeof globalThis[name] !== 'undefined') throw new Error(name); try { globalThis[name](() => {}); throw new Error('call succeeded'); } catch (e) { if (!(e instanceof TypeError)) throw e; } } globalThis.__neoTestCounts = () => ({total: 10, passed: 10, failed: 0});").unwrap();
    assert_eq!(js.run_pending_tests().unwrap(), None);
}

#[test]
fn dynamic_import_and_its_errors_are_preserved() {
    let js = runtime();
    js.run_module("dynamic.js", "let rejected = false; try { await import('neo:not-a-module'); } catch (_) { rejected = true; } if (!rejected) throw new Error('missing rejection'); const m = await import('neo:test'); m.test('dynamic failure', () => { throw new Error('dynamic'); });").unwrap();
    let report = js.run_pending_tests().unwrap().unwrap();
    assert_eq!((report.total, report.passed, report.failed), (1, 0, 1));
    assert_eq!(js.run_pending_tests().unwrap().unwrap(), report);
}

#[test]
fn dynamic_import_propagates_test_module_initialization_errors() {
    let js = runtime();
    js.run_module(
        "failed-dynamic.js",
        "const original = globalThis.console;
         Object.defineProperty(globalThis, 'console', { configurable: true, get() { throw new Error('factory failed'); } });
         let message = ''; try { await import('neo:test'); } catch (error) { message = String(error); }
         Object.defineProperty(globalThis, 'console', { configurable: true, writable: true, value: original });
         if (!message.includes('factory failed')) throw new Error('lost import error: ' + message);",
    )
    .unwrap();
    assert_eq!(js.run_pending_tests().unwrap(), None);
}

#[test]
fn preimport_intrinsic_replacement_cannot_capture_the_queue() {
    let js = runtime();
    js.run_module(
        "capture.js",
        "const create = Object.create; let captured;
         Object.create = function(proto) { const object = create(proto); captured = object; return object; };
         const {test} = await import('neo:test');
         test('must fail', () => { throw new Error('real failure'); });
         if (captured && captured[0]) captured[0].fn = () => {};",
    )
    .unwrap();
    let report = js.run_pending_tests().unwrap().unwrap();
    assert_eq!((report.total, report.passed, report.failed), (1, 0, 1));
    assert!(report.failures[0].contains("real failure"));
}

#[test]
fn throwing_object_create_before_import_does_not_break_private_queue() {
    let js = runtime();
    js.run_module(
        "intrinsic.js",
        "Object.create = () => { throw new Error('replaced Object.create'); };
         const {test} = await import('neo:test');
         test('still runs', () => {});",
    )
    .unwrap();
    assert_eq!(js.run_pending_tests().unwrap().unwrap().passed, 1);
}

#[test]
fn repeated_runtime_creation_and_drop_releases_test_runners() {
    for _ in 0..32 {
        let js = runtime();
        js.run_module(
            "lifecycle.js",
            "import test from 'neo:test'; test('ok', () => {});",
        )
        .unwrap();
        assert_eq!(js.run_pending_tests().unwrap().unwrap().passed, 1);
    }
}
