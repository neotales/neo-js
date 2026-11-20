/**
 * A synchronous test registry and runner for neo-js.
 *
 * `test(name, fn)` queues a test; `run()` executes every queued test in
 * registration order. The host calls the runner after the entry module
 * settles, so a script does not have to call `run()` itself. Failures are
 * printed and counted, and the CLI exits non-zero if any test failed.
 *
 * This is not a test-file discovery tool. Only tests registered by the modules
 * evaluated in this runtime are run, and they share one global context and its
 * state. There is no concurrency, no timeout (the runtime cannot interrupt
 * JavaScript), no retry, and no asynchronous form: the runtime has no event
 * loop. A Promise returned by a callback is a test failure, not a pending test.
 * So is a generator object, including one from a bound generator function, whose
 * body would otherwise be skipped.
 *
 * Runner bookkeeping is deliberately independent of mutable prototypes. The
 * queue is a private null-prototype store with an explicit cursor, so
 * `push`/`splice` and inherited index setters cannot drop registrations; counts
 * and failure summaries go to host-owned callbacks as primitives, not through
 * a JSON envelope. This addresses the tested array and object prototype
 * mutations and inherited `toJSON` hooks, not arbitrary realm tampering.
 * It is still one shared realm: tests can corrupt each other and the assertion
 * helpers, so this is not a sandbox.
 */

/** Upper bound on registrations per runtime, so a runaway loop fails loudly. */
const MAX_TESTS = 10000;

// Intrinsics and the queue come from the private factory closure created by
// the host before any user code can replace the realm's built-ins.

// Create an own field with the attributes plain assignment would give it. The
// descriptor must not inherit Object.prototype.get/set, and a same-named
// inherited setter must not run.
function defineField(object, key, value) {
  defineProperty(object, key, {
    __proto__: null,
    value: value,
    enumerable: true,
    configurable: true,
    writable: true,
  });
}

// Append without going through Array.prototype's mutable index setters.
function append(array, value) {
  defineField(array, array.length, value);
}

/** Thrown by `run()` after one or more tests failed. */
class TestFailureError extends NativeError {
  constructor(message, failures) {
    super(message);
    // Own data properties: an inherited setter for a field name must not
    // replace this error, drop its code, or drop the retained failures.
    defineField(this, 'name', 'TestFailureError');
    defineField(this, 'code', 'NEO_TEST_FAILURES');
    defineField(this, 'failures', failures);
  }
}

const hostLog =
  globalThis.console && typeof globalThis.console.log === 'function'
    ? globalThis.console.log.bind(globalThis.console)
    : function () {};


function print(text) {
  hostLog(toWellFormed(text));
}

function displayName(name) {
  let out = '';
  for (let i = 0; i < name.length; i++) {
    const ch = name[i];
    out += ch === '\n' || ch === '\r' ? ' ' : ch;
  }
  return toWellFormed(out);
}

function isBlank(name) {
  for (let i = 0; i < name.length; i++) {
    const ch = name[i];
    if (ch !== ' ' && ch !== '\t' && ch !== '\n' && ch !== '\r') return false;
  }
  return true;
}

function oneLine(text) {
  try {
    const value = nativeString(text);
    let out = '';
    for (let i = 0; i < value.length; i++) {
      if (out.length >= 300) return out + '...';
      const ch = value[i];
      if (ch === '\n' || ch === '\r') {
        if (out.length === 0 || out[out.length - 1] !== ' ') out += ' ';
        continue;
      }
      const unit = charCodeAt(value, i);
      if (unit >= 0xd800 && unit <= 0xdbff) {
        const next = charCodeAt(value, i + 1);
        if (next >= 0xdc00 && next <= 0xdfff) {
          // Preserve the 300-code-unit budget without splitting a scalar.
          if (out.length === 299) return out + '...';
          out += ch + value[++i];
          continue;
        }
        out += '\ufffd';
      } else if (unit >= 0xdc00 && unit <= 0xdfff) {
        out += '\ufffd';
      } else {
        out += ch;
      }
    }
    return out;
  } catch (_) {
    return 'unprintable text';
  }
}

function quoteText(text) {
  let out = "'";
  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    if (ch === '\\' || ch === "'") out += '\\';
    out += ch;
  }
  return out + "'";
}

function printIndented(text) {
  try {
    const value = nativeString(text);
    let line = '';
    for (let i = 0; i <= value.length; i++) {
      if (i === value.length || value[i] === '\n') {
        print('  ' + line);
        line = '';
      } else {
        line += value[i];
      }
    }
  } catch (_) {
    try {
      print('  failure details unavailable');
    } catch (__) {}
  }
}

function describeThrown(error) {
  try {
    if (error instanceof NativeError) {
      let name = 'Error';
      let code = '';
      let message;
      try {
        if (typeof error.name === 'string' && error.name) name = oneLine(error.name);
      } catch (_) {}
      try {
        if (typeof error.code === 'string' && error.code)
          code = ' [' + oneLine(error.code) + ']';
      } catch (_) {}
      try {
        message = oneLine(error.message);
      } catch (_) {
        message = '<message unreadable>';
      }
      return name + code + ': ' + message;
    }
    if (typeof error === 'string') return 'thrown string: ' + quoteText(oneLine(error));
    try {
      return 'thrown ' + typeof error + ': ' + oneLine(nativeString(error));
    } catch (_) {
      return 'thrown ' + typeof error;
    }
  } catch (_) {
    return 'thrown value';
  }
}

function describeDetail(error) {
  try {
    if (error instanceof NativeError) {
      let name = 'Error';
      let code = '';
      let message;
      try {
        if (typeof error.name === 'string' && error.name) name = error.name;
      } catch (_) {}
      try {
        if (typeof error.code === 'string' && error.code) code = ' [' + error.code + ']';
      } catch (_) {}
      try {
        message = nativeString(error.message);
      } catch (_) {
        message = '<message unreadable>';
      }
      return name + code + ': ' + message;
    }
    return describeThrown(error);
  } catch (_) {
    return 'failure details unavailable';
  }
}

function isFrame(line) {
  let i = 0;
  while (i < line.length && (line[i] === ' ' || line[i] === '\t')) i++;
  return (
    line[i] === 'a' &&
    line[i + 1] === 't' &&
    (line[i + 2] === ' ' || line[i + 2] === '\t' || i + 2 === line.length)
  );
}

function printFailure(index, name, error) {
  try {
    print('not ok ' + index + ' - ' + name);
  } catch (_) {}
  printIndented(describeDetail(error));
  try {
    if (error instanceof NativeError && typeof error.stack === 'string') {
      const stack = error.stack;
      let line = '';
      for (let i = 0; i <= stack.length; i++) {
        const ch = i === stack.length ? '\n' : stack[i];
        if (ch === '\n') {
          if (isFrame(line)) {
            try {
              print('  ' + line);
            } catch (_) {}
          }
          line = '';
        } else {
          line += ch;
        }
      }
    }
  } catch (_) {
    // Stack text is decorative; the failure is already recorded.
  }
}

// A null-prototype store with an explicit length avoids inherited setters and
// `Array.prototype` entirely. The host report is separate again in Rust.
let queueLength = 0;
let cursor = 0;
let registered = 0;
let total = 0;
let passed = 0;
let failed = 0;
let running = false;

function testError(kind, code, message) {
  const error =
    kind === 'TypeError' ? new NativeTypeError(message) : new NativeError(message);
  defineField(error, 'code', code);
  return error;
}

/**
 * Register a synchronous test. Tests run in registration order.
 *
 * Registration throws a TypeError for a non-function callback or an async or
 * generator function or callable proxy; a generated `Error` (`NEO_TEST_DURING_RUN`) when called
 * from inside a running test; and `NEO_TEST_LIMIT` past `MAX_TESTS`
 * registrations. These are programmer errors, not test failures.
 */
function test(name, fn) {
  if (running)
    throw testError('Error', 'NEO_TEST_DURING_RUN', 'cannot register a test while tests are running');
  if (typeof name !== 'string' || isBlank(name))
    throw new NativeTypeError('test name must be a non-empty string');
  if (typeof fn !== 'function') throw new NativeTypeError('test callback must be a function');
  if (invalidCallback(fn))
    throw testError(
      'TypeError',
      'NEO_TEST_ASYNC',
      'test "' + name + '" must be synchronous; async functions, generator functions and proxies are not supported',
    );
  if (registered >= MAX_TESTS)
    throw testError(
      'Error',
      'NEO_TEST_LIMIT',
      'cannot register more than ' + MAX_TESTS + ' tests in one runtime',
    );
  registered++;
  queue[queueLength] = { name: name, fn: fn };
  queueLength++;
}

/**
 * Run every queued test in registration order and return this run's counts.
 *
 * A failing test is recorded and the run continues, so one failure does not
 * hide the rest. Every failure is printed and retained for the process report,
 * even if the caller catches the thrown `TestFailureError`. A callback that
 * returns a Promise, generator object, thenable or ambiguous `then` lookup
 * is a failure with `NEO_TEST_ASYNC`.
 *
 * Calling `run()` with an empty queue is a no-op that returns zero counts.
 * Calling it from inside a test throws `NEO_TEST_ALREADY_RUNNING`, which the
 * current test records as a failure.
 */
function run() {
  if (running)
    throw testError('Error', 'NEO_TEST_ALREADY_RUNNING', 'tests are already running');
  const start = cursor;
  if (start >= queueLength) return { total: 0, passed: 0, failed: 0, failures: [] };

  running = true;
  let runTotal = 0;
  let runPassed = 0;
  let runFailed = 0;
  const runFailures = [];
  try {
    for (let i = start; i < queueLength; i++) {
      const entry = queue[i];
      const name = displayName(entry.name);
      const index = runTotal + 1;
      runTotal++;
      total++;
      try {
        const unsupported = unsupportedReturn(entry.fn());
        if (unsupported !== null)
          throw testError(
            'TypeError',
            'NEO_TEST_ASYNC',
            'test "' + name + '" returned ' + unsupported + '; neo:test runs synchronous tests only',
          );
      } catch (error) {
        runFailed++;
        failed++;
        // Sanitize the complete primitive string at the Rust boundary using
        // a pristine intrinsic, never a caller's string methods.
        const summary = toWellFormed(name + ': ' + describeThrown(error));
        // Host call first: even a failure in output formatting cannot lose it.
        recordFailure(summary);
        try {
          append(runFailures, summary);
        } catch (_) {
          // A hostile inherited setter can break the script-visible summary;
          // the host copy above is authoritative.
        }
        try {
          printFailure(index, name, error);
        } catch (_) {}
        continue;
      }
      runPassed++;
      passed++;
      try {
        print('ok ' + index + ' - ' + name);
      } catch (_) {}
    }
    cursor = queueLength;
  } finally {
    running = false;
  }

  // The host owns the report; set it before printing so output failure cannot
  // leave the process looking clean.
  setCounts(total, passed, failed);
  try {
    print(
      '# ' +
        runTotal +
        (runTotal === 1 ? ' test: ' : ' tests: ') +
        runPassed +
        ' passed, ' +
        runFailed +
        ' failed',
    );
  } catch (_) {}

  const summary = {
    total: runTotal,
    passed: runPassed,
    failed: runFailed,
    failures: runFailures,
  };
  if (runFailed > 0) {
    throw new TestFailureError(
      runFailed + ' of ' + runTotal + (runTotal === 1 ? ' test failed' : ' tests failed'),
      runFailures,
    );
  }
  return summary;
}

function report() {
  try {
    run();
  } catch (error) {
    // Test failures are already printed, recorded with the host, and retained;
    // anything else is a bug in the runner and must reach the host.
    if (!error || error.code !== 'NEO_TEST_FAILURES') throw error;
  }
}

// The host holds this function persistently. No script-writable global is used
// to discover or replace the runner.
installRunner(report);

return {default: test, test, run, MAX_TESTS, TestFailureError};
