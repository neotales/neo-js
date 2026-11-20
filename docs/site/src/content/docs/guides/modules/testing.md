---
title: "Test scripts"
description: "Test registration, assertion behavior, and CLI and host reports."
---

`neo:test` registers tests and `neo:assert` supplies assertions. Neither needs
a permission flag, and neither grants any host capability. The CLI runs every
registered test after the entry module settles; importing `neo:test` is enough,
and calling `run()` yourself is optional.

```js
import test from "neo:test";
import { deepEqual, equal, throws } from "neo:assert";

test("adds numbers", () => {
	equal(1 + 1, 2);
});

test("parses nested data", () => {
	deepEqual(JSON.parse('{"a":[1,2]}'), { a: [1, 2] });
});

test("rejects invalid JSON", () => {
	throws(() => JSON.parse("{"), SyntaxError);
});
```

With the default logger and no failures, output is one `ok` line per test and
the process exits 0. With the default logger, a failing test prints `not ok`
with the assertion message and, for `deepEqual`, the first differing path. If
the second test above compared
`[1, 3]` with `[1, 2]`, it would report:

```text
ok 1 - adds numbers
not ok 2 - parses nested data
  AssertionError [ERR_ASSERTION]: values are not deeply equal: values differ at $.a[1]
  actual: 3
  expected: 2
ok 3 - rejects invalid JSON
# 3 tests: 2 passed, 1 failed
neo-js: 1 of 3 test(s) failed
```

Failure semantics:

- Tests run synchronously, in registration order, in the entry module's single
  global context. A test that throws is recorded and the run continues, so one
  failure cannot hide the others.
- The CLI exits non-zero when any executed test failed. Tests registered before
  an ordinary module error still run, and both failures are reported. If the
  module abandoned the runtime, the CLI reports `NEO_MODULE_ABORTED` alongside
  the module error and runs no tests or queued jobs. A runner error or
  inconsistent host counts also causes a failure.
- `run()` is available for a script that wants results immediately. It returns
  this run's counts and throws `TestFailureError` (`code: "NEO_TEST_FAILURES"`)
  after printing. Catching that error does not make the process succeed: the
  module retains every failure, and the CLI still exits non-zero.
- Async work is not supported. `neo:test` rejects a directly supplied async
  function, generator function, or callable proxy at registration with
  `TypeError` (`NEO_TEST_ASYNC`). `neo:assert` applies the same checks to
  callbacks passed to `throws` or `doesNotThrow`. A bound function or other
  wrapper may not expose the underlying function's kind; it is called normally
  and its return value is checked.
- Both modules reject native Promises and generator objects (including async
  generators) regardless of changes to `then`, prototypes, or
  `Symbol.toStringTag`. A directly returned Promise is marked handled by the
  engine; the synchronous error reports the unsupported return, not its
  rejection reason. The guard does not await, advance generators, or drain
  jobs. It does not track async work that the callback starts but does not
  return.
- A callable `then` data property also makes a return unsupported. The check
  walks the `then` lookup without invoking getters or proxy traps: it rejects
  a `then` accessor even if its getter would return a non-function, a returned
  proxy (including revoked proxies), and a proxy reached in the prototype
  chain before finding a `then` property. It does not unwrap proxies.
  Unsupported returns fail `neo:test` with `NEO_TEST_ASYNC` and throw a
  `TypeError` from `throws` or `doesNotThrow`. TypeScript can reject inferred
  Promise, thenable, and generator return types, but cannot identify proxies,
  distinguish accessors from data properties, or recover erased return types
  such as `void` or `unknown`.
- Registering a test from inside a running test throws `NEO_TEST_DURING_RUN`.
  At most 10,000 tests may be registered in one runtime; further registrations
  throw `NEO_TEST_LIMIT`. Names need not be unique.
- `neo:assert` provides `equal`/`notEqual` (strict `Object.is` comparison,
  where `NaN` equals `NaN` and `-0` differs from `0`), `deepEqual`/
  `notDeepEqual`, `ok`, `fail`, `throws`/`doesNotThrow`, and the
  `AssertionError` class (`code: "ERR_ASSERTION"`). `deepEqual` compares own
  enumerable properties plus array length and the internal state of dates,
  regular expressions (including `lastIndex`), boxed primitives, Maps, Sets,
  errors, buffers, and typed arrays; functions, Promises, WeakMaps, WeakSets,
  and WeakRefs compare by identity. Cyclic values are detected instead of
  recursing forever, but this is not a full graph-isomorphism comparison.
  Getter properties are read, and a getter that throws aborts that assertion
  with the getter's error. Recursion is limited by the JavaScript stack rather
  than a count.
- Runner output uses the current `globalThis.console.log` captured when
  `neo:test` initializes, or a no-op if it is not a function. Replacing it
  before a dynamic import can redirect or suppress test details; replacing it
  afterward does not change the captured logger. Counts and failure summaries
  are recorded through host-owned callbacks, so suppressing that output does
  not change the host failure count or the CLI's non-zero exit for a failed
  test. Replacing `JSON.stringify`, mutating `Array.prototype.push`/`splice`,
  installing an inherited array index setter, or adding an inherited
  `Object.prototype.toJSON` after import cannot skip tests or hide failures
  from the host report.
- This is registration-based, not test discovery: only modules evaluated in
  this runtime can register tests. Tests are not isolated from each other, have
  no timeout, retry, or parallelism, and share globals and mutable state. A
  callback that never returns hangs the process. A test can still corrupt
  intrinsics and break later tests or assertion output; the runner protects its
  own bookkeeping, not the whole realm. Do not treat tests as a sandbox for
  untrusted code.

When embedding the library, call `NeoJs::run_pending_tests()` after evaluating
the entry module. It returns `None` when `neo:test` was never imported, or a
cumulative `TestReport` on a usable runtime. An abandoned runtime returns
`NEO_MODULE_ABORTED` without running tests or pending jobs. The method does
not print beyond what the module writes to `console` or choose an exit status.

Declarations: [`neo:test`](/api/modules/test/), [`neo:assert`](/api/modules/assert/).
