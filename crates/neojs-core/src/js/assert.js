/**
 * Synchronous assertions for neo-js.
 *
 * Every assertion throws an `AssertionError` when the check fails. Invalid
 * arguments (a non-string message, a non-function callback, an unsupported
 * `expected` matcher) throw a `TypeError` instead, so a mistake in the test is
 * not reported as a failed assertion.
 *
 * There are no asynchronous forms. The runtime has no external event loop, so
 * a callback that returns a Promise is never awaited; `throws` and
 * `doesNotThrow` reject one with a `TypeError` rather than treating the pending
 * work as a pass. Generator objects, thenables and ambiguous `then` lookups
 * are rejected the same way.
 */

const {invalidCallback, unsupportedReturn} = guard;
const MAX_TEXT = 400;
const MAX_ITEMS = 50;
const MAX_DEPTH = 8;

// Capture the intrinsics used by comparison and reporting. A script under test
// shares this realm; replacing these globals later must not change whether an
// assertion passes or make the formatter throw.
const objectIs = Object.is;
const objectKeys = Object.keys;
const getOwnPropertySymbols = Object.getOwnPropertySymbols;
const getOwnPropertyDescriptor = Object.getOwnPropertyDescriptor;
const defineProperty = Object.defineProperty;
const getPrototypeOf = Object.getPrototypeOf;
const propertyIsEnumerable = Object.prototype.propertyIsEnumerable;
const hasOwnProperty = Object.prototype.hasOwnProperty;
const stringify = JSON.stringify;
const arrayIsArray = Array.isArray;
const arrayBufferIsView = ArrayBuffer.isView;
const NativeUint8Array = Uint8Array;
const NativeMap = Map;
const NativeSet = Set;
const NativeDate = Date;
const NativeRegExp = RegExp;
const NativeError = Error;
const NativeTypeError = TypeError;
const NativeBoolean = Boolean;
const NativeNumber = Number;
const NativeString = String;
const NativeArrayBuffer = ArrayBuffer;
const NativeSharedArrayBuffer =
  typeof SharedArrayBuffer === 'function' ? SharedArrayBuffer : null;
const NativePromise = Promise;
const NativeWeakMap = WeakMap;
const NativeWeakSet = WeakSet;
const NativeWeakRef = typeof WeakRef === 'function' ? WeakRef : null;
const NativeBigInt = typeof BigInt === 'function' ? BigInt : null;
const NativeSymbol = Symbol;
const NativeDataView = typeof DataView === 'function' ? DataView : null;

const dateGetTime = NativeDate.prototype.getTime;
const regexpSource = getOwnPropertyDescriptor(NativeRegExp.prototype, 'source').get;
const regexpFlags = getOwnPropertyDescriptor(NativeRegExp.prototype, 'flags').get;
const mapSize = getOwnPropertyDescriptor(NativeMap.prototype, 'size').get;
const mapForEach = NativeMap.prototype.forEach;
const setSize = getOwnPropertyDescriptor(NativeSet.prototype, 'size').get;
const setForEach = NativeSet.prototype.forEach;
const booleanValueOf = NativeBoolean.prototype.valueOf;
const numberValueOf = NativeNumber.prototype.valueOf;
const stringValueOf = NativeString.prototype.valueOf;
const bigintValueOf = NativeBigInt ? NativeBigInt.prototype.valueOf : null;
const symbolValueOf = NativeSymbol.prototype.valueOf;

// Bind once so replacing `Function.prototype.call` later cannot change an
// equality result or turn a comparison into a false pass.
const call = Function.prototype.call.bind(Function.prototype.call);
const regexpTest = NativeRegExp.prototype.test;
const mapGet = NativeMap.prototype.get;
const mapSet = NativeMap.prototype.set;
const mapDelete = NativeMap.prototype.delete;
const setHas = NativeSet.prototype.has;
const setAdd = NativeSet.prototype.add;
const setDelete = NativeSet.prototype.delete;

const INDEX_KEY = /^(0|[1-9][0-9]*)$/;
const IDENTIFIER_KEY = /^[A-Za-z_$][A-Za-z0-9_$]*$/;

// Create an own public field with the attributes plain assignment would give
// it. A same-named inherited setter must not run, and the descriptor must not
// inherit Object.prototype.get/set.
function defineField(object, key, value) {
  defineProperty(object, key, {
    __proto__: null,
    value: value,
    enumerable: true,
    configurable: true,
    writable: true,
  });
}

// Create own data properties rather than assigning through a possibly hostile
// inherited setter on Array.prototype.
function defineIndex(array, index, value) {
  defineField(array, index, value);
}

function append(array, value) {
  defineIndex(array, array.length, value);
}

function contains(array, value) {
  for (let i = 0; i < array.length; i++) if (array[i] === value) return true;
  return false;
}

/** Thrown when an assertion fails. */
class AssertionError extends Error {
  constructor(message, options) {
    super(message);
    // Own data properties: an inherited setter for a field name must not
    // replace this error or drop its code. Attributes match assignment.
    defineField(this, 'name', 'AssertionError');
    defineField(this, 'code', 'ERR_ASSERTION');
    const details = options || {};
    defineField(this, 'operator', details.operator || 'assert');
    if ('actual' in details) defineField(this, 'actual', details.actual);
    if ('expected' in details) defineField(this, 'expected', details.expected);
  }
}

function checkMessage(message) {
  if (message !== undefined && typeof message !== 'string')
    throw new NativeTypeError('message must be a string');
}

function withMessage(message, detail) {
  return message === undefined ? detail : message + ': ' + detail;
}

function truncate(text) {
  const value = NativeString(text);
  return value.length <= MAX_TEXT
    ? value
    : value.slice(0, MAX_TEXT) + '... (' + (value.length - MAX_TEXT) + ' more characters)';
}

function quote(text) {
  return stringify(truncate(text));
}

function formatKey(key) {
  if (typeof key === 'symbol') return '[' + NativeString(key) + ']';
  if (call(regexpTest, INDEX_KEY, key)) return '[' + key + ']';
  return call(regexpTest, IDENTIFIER_KEY, key) ? '.' + key : '[' + quote(key) + ']';
}

/** Own enumerable string and symbol keys, in property order. */
function ownKeys(value) {
  const keys = objectKeys(value);
  const symbols = getOwnPropertySymbols(value);
  for (let i = 0; i < symbols.length; i++) {
    if (call(propertyIsEnumerable, value, symbols[i]))
      append(keys, symbols[i]);
  }
  return keys;
}

function constructorName(value) {
  const prototype = getPrototypeOf(value);
  if (prototype === null) return 'null-prototype';
  const constructor = prototype.constructor;
  const name = constructor && typeof constructor.name === 'string' ? constructor.name : '';
  return name === 'Object' ? '' : name;
}

function formatObject(value, depth, seen) {
  if (arrayIsArray(value)) {
    const parts = [];
    const length = Math.min(value.length, MAX_ITEMS);
    for (let i = 0; i < length; i++) {
      append(
        parts,
        call(hasOwnProperty, value, i)
          ? formatValue(value[i], depth + 1, seen)
          : '<empty>',
      );
    }
    if (value.length > MAX_ITEMS) append(parts, '...');
    return '[' + parts.join(', ') + ']';
  }

  if (value instanceof NativeDate) {
    const time = call(dateGetTime, value);
    return time === time ? 'Date(' + value.toISOString() + ')' : 'Invalid Date';
  }
  if (value instanceof NativeRegExp) return NativeString(value);
  if (value instanceof NativeMap) {
    const parts = [];
    let count = 0;
    call(mapForEach, value, function (entryValue, key) {
      if (count >= MAX_ITEMS) return;
      count++;
      append(
        parts,
        formatValue(key, depth + 1, seen) + ' => ' + formatValue(entryValue, depth + 1, seen),
      );
    });
    const size = call(mapSize, value);
    if (size > MAX_ITEMS) append(parts, '...');
    return 'Map(' + size + ') {' + (parts.length ? ' ' + parts.join(', ') + ' ' : '') + '}';
  }
  if (value instanceof NativeSet) {
    const parts = [];
    let count = 0;
    call(setForEach, value, function (entryValue) {
      if (count >= MAX_ITEMS) return;
      count++;
      append(parts, formatValue(entryValue, depth + 1, seen));
    });
    const size = call(setSize, value);
    if (size > MAX_ITEMS) append(parts, '...');
    return 'Set(' + size + ') {' + (parts.length ? ' ' + parts.join(', ') + ' ' : '') + '}';
  }
  if (value instanceof NativeArrayBuffer) return 'ArrayBuffer(' + value.byteLength + ')';
  if (NativeSharedArrayBuffer && value instanceof NativeSharedArrayBuffer)
    return 'SharedArrayBuffer(' + value.byteLength + ')';
  if (NativeDataView && value instanceof NativeDataView)
    return 'DataView(' + value.byteLength + ')';
  if (arrayBufferIsView(value)) {
    const name =
      value.constructor && value.constructor.name ? value.constructor.name : 'TypedArray';
    const parts = [];
    const length = Math.min(value.length, MAX_ITEMS);
    for (let i = 0; i < length; i++)
      append(parts, formatValue(value[i], depth + 1, seen));
    if (value.length > MAX_ITEMS) append(parts, '...');
    return name + '(' + value.length + ') [' + parts.join(', ') + ']';
  }
  if (value instanceof NativeError) {
    let name = 'Error';
    let message = '<message unreadable>';
    try {
      if (typeof value.name === 'string' && value.name) name = value.name;
    } catch (_) {}
    try {
      message = truncate(value.message);
    } catch (_) {}
    return name + ': ' + message;
  }
  if (value instanceof NativeBoolean)
    return 'Boolean {' + formatValue(call(booleanValueOf, value), depth + 1, seen) + '}';
  if (value instanceof NativeNumber)
    return 'Number {' + formatValue(call(numberValueOf, value), depth + 1, seen) + '}';
  if (value instanceof NativeString)
    return 'String {' + formatValue(call(stringValueOf, value), depth + 1, seen) + '}';
  if (NativeBigInt && value instanceof NativeBigInt)
    return 'BigInt {' + formatValue(call(bigintValueOf, value), depth + 1, seen) + '}';
  if (value instanceof NativeSymbol)
    return 'Symbol {' + formatValue(call(symbolValueOf, value), depth + 1, seen) + '}';

  const name = constructorName(value);
  const keys = ownKeys(value);
  const parts = [];
  for (let i = 0; i < keys.length && i < MAX_ITEMS; i++) {
    const key = keys[i];
    let text;
    try {
      text = formatKey(key) + ': ' + formatValue(value[key], depth + 1, seen);
    } catch (error) {
      text =
        formatKey(key) +
        ': [threw ' +
        truncate(safeMessage(error)) +
        ']';
    }
    append(parts, text);
  }
  if (keys.length > MAX_ITEMS) append(parts, '...');
  return (
    (name ? name + ' ' : '') +
    '{' +
    (parts.length ? ' ' + parts.join(', ') + ' ' : '') +
    '}'
  );
}

/**
 * Best-effort value formatting for assertion messages. Getters are invoked,
 * cycles print as `[Circular]`, and output is truncated by depth, item count,
 * and total text length. This never throws: a hostile accessor or a mutated
 * string/array prototype yields a fallback string so the assertion still
 * reports its failure.
 */
function formatValue(value, depth, seen) {
  try {
    return formatValueUnsafe(value, depth, seen);
  } catch (_) {
    return '[unprintable value]';
  }
}

function safeMessage(error) {
  try {
    if (error && typeof error.message === 'string') return error.message;
    return NativeString(error);
  } catch (_) {
    return 'unprintable error';
  }
}

function formatValueUnsafe(value, depth, seen) {
  const level = depth || 0;
  const path = seen || [];
  if (value === null) return 'null';
  const kind = typeof value;
  if (kind === 'string') return quote(value);
  if (kind === 'number') return objectIs(value, -0) ? '-0' : NativeString(value);
  if (kind === 'bigint') return NativeString(value) + 'n';
  if (kind === 'boolean' || kind === 'undefined') return NativeString(value);
  if (kind === 'symbol') return NativeString(value);
  if (kind === 'function') return '[Function: ' + (value.name || 'anonymous') + ']';
  if (contains(path, value)) return '[Circular]';
  if (level >= MAX_DEPTH) return arrayIsArray(value) ? '[Array]' : '[Object]';
  append(path, value);
  try {
    return formatObject(value, level, path);
  } catch (error) {
    return '[uninspectable: ' + truncate(safeMessage(error)) + ']';
  } finally {
    path.length = path.length - 1;
  }
}

function isObjectLike(value) {
  return value !== null && (typeof value === 'object' || typeof value === 'function');
}

function markPair(seen, actual, expected) {
  let expectedSet = call(mapGet, seen.map, actual);
  if (!expectedSet) {
    expectedSet = new NativeSet();
    call(mapSet, seen.map, actual, expectedSet);
  }
  if (call(setHas, expectedSet, expected)) return true;
  call(setAdd, expectedSet, expected);
  append(seen.log, [actual, expected]);
  return false;
}

function rollback(seen, mark) {
  while (seen.log.length > mark) {
    const pair = seen.log[seen.log.length - 1];
    seen.log.length = seen.log.length - 1;
    const expectedSet = call(mapGet, seen.map, pair[0]);
    if (expectedSet) {
      call(setDelete, expectedSet, pair[1]);
      if (call(setSize, expectedSet) === 0) call(mapDelete, seen.map, pair[0]);
    }
  }
}

/** Compare two values under the shared cycle-tracking state, rolling back on failure. */
function sameValue(actual, expected, path, seen) {
  const mark = seen.log.length;
  if (findMismatch(actual, expected, path, seen) === null) return true;
  rollback(seen, mark);
  return false;
}

function bytesEqual(actual, expected) {
  if (actual.length !== expected.length) return false;
  for (let i = 0; i < actual.length; i++) if (actual[i] !== expected[i]) return false;
  return true;
}

function viewBytes(view) {
  return new NativeUint8Array(view.buffer, view.byteOffset, view.byteLength);
}

/**
 * Return the first structural difference, or null when the values are deeply
 * equal. `path` is a readable JSONPath-like location in `actual`.
 *
 * Equality is strict: prototypes must match, functions and Promises compare by
 * identity, NaNs are equal, and `-0` differs from `0`. Own enumerable string
 * and symbol properties are compared; getters are invoked. Arrays also compare
 * length. Maps and Sets are compared without regard to iteration order.
 * Regular expressions compare source, flags, and `lastIndex`. Own properties of
 * dates, boxed primitives, errors, buffers, and views are compared in addition
 * to their internal state.
 */
function findMismatch(actual, expected, path, seen) {
  if (objectIs(actual, expected)) return null;
  if (!isObjectLike(actual) || !isObjectLike(expected))
    return { path: path, reason: 'values differ', actual: actual, expected: expected };
  if (typeof actual === 'function' || typeof expected === 'function')
    return {
      path: path,
      reason: 'functions are compared by identity',
      actual: actual,
      expected: expected,
    };
  if (markPair(seen, actual, expected)) return null;

  if (getPrototypeOf(actual) !== getPrototypeOf(expected))
    return { path: path, reason: 'prototypes differ', actual: actual, expected: expected };

  if (arrayIsArray(actual) && actual.length !== expected.length)
    return { path: path, reason: 'array lengths differ', actual: actual, expected: expected };

  if (actual instanceof NativeDate) {
    if (!objectIs(call(dateGetTime, actual), call(dateGetTime, expected)))
      return { path: path, reason: 'dates differ', actual: actual, expected: expected };
  } else if (actual instanceof NativeRegExp) {
    if (
      call(regexpSource, actual) !== call(regexpSource, expected) ||
      call(regexpFlags, actual) !== call(regexpFlags, expected) ||
      !objectIs(actual.lastIndex, expected.lastIndex)
    )
      return { path: path, reason: 'regular expressions differ', actual: actual, expected: expected };
  } else if (actual instanceof NativeMap) {
    if (call(mapSize, actual) !== call(mapSize, expected))
      return { path: path, reason: 'Map sizes differ', actual: actual, expected: expected };
    const expectedEntries = [];
    call(mapForEach, expected, function (entryValue, entryKey) {
      append(expectedEntries, [entryKey, entryValue]);
    });
    const used = [];
    let missing = false;
    call(mapForEach, actual, function (entryValue, entryKey) {
      if (missing) return;
      let found = false;
      for (let i = 0; i < expectedEntries.length; i++) {
        if (used[i]) continue;
        if (
          sameValue(entryKey, expectedEntries[i][0], path + '<Map key>', seen) &&
          sameValue(entryValue, expectedEntries[i][1], path + '<Map value>', seen)
        ) {
          defineIndex(used, i, true);
          found = true;
          break;
        }
      }
      if (!found) missing = true;
    });
    if (missing)
      return { path: path, reason: 'Map entries differ', actual: actual, expected: expected };
  } else if (actual instanceof NativeSet) {
    if (call(setSize, actual) !== call(setSize, expected))
      return { path: path, reason: 'Set sizes differ', actual: actual, expected: expected };
    const expectedValues = [];
    call(setForEach, expected, function (entryValue) {
      append(expectedValues, entryValue);
    });
    const used = [];
    let missing = false;
    call(setForEach, actual, function (entryValue) {
      if (missing) return;
      let found = false;
      for (let i = 0; i < expectedValues.length; i++) {
        if (used[i]) continue;
        if (sameValue(entryValue, expectedValues[i], path + '<Set value>', seen)) {
          defineIndex(used, i, true);
          found = true;
          break;
        }
      }
      if (!found) missing = true;
    });
    if (missing)
      return { path: path, reason: 'Set values differ', actual: actual, expected: expected };
  } else if (
    actual instanceof NativeArrayBuffer ||
    (NativeSharedArrayBuffer && actual instanceof NativeSharedArrayBuffer)
  ) {
    if (!bytesEqual(new NativeUint8Array(actual), new NativeUint8Array(expected)))
      return { path: path, reason: 'buffer bytes differ', actual: actual, expected: expected };
  } else if (arrayBufferIsView(actual)) {
    if (
      actual.byteLength !== expected.byteLength ||
      !bytesEqual(viewBytes(actual), viewBytes(expected))
    )
      return { path: path, reason: 'view bytes differ', actual: actual, expected: expected };
  } else if (actual instanceof NativeBoolean) {
    if (!objectIs(call(booleanValueOf, actual), call(booleanValueOf, expected)))
      return { path: path, reason: 'boxed values differ', actual: actual, expected: expected };
  } else if (actual instanceof NativeNumber) {
    if (!objectIs(call(numberValueOf, actual), call(numberValueOf, expected)))
      return { path: path, reason: 'boxed values differ', actual: actual, expected: expected };
  } else if (actual instanceof NativeString) {
    if (!objectIs(call(stringValueOf, actual), call(stringValueOf, expected)))
      return { path: path, reason: 'boxed values differ', actual: actual, expected: expected };
  } else if (NativeBigInt && actual instanceof NativeBigInt) {
    if (!objectIs(call(bigintValueOf, actual), call(bigintValueOf, expected)))
      return { path: path, reason: 'boxed values differ', actual: actual, expected: expected };
  } else if (actual instanceof NativeSymbol) {
    if (!objectIs(call(symbolValueOf, actual), call(symbolValueOf, expected)))
      return { path: path, reason: 'boxed values differ', actual: actual, expected: expected };
  } else if (actual instanceof NativeError) {
    if (actual.name !== expected.name || actual.message !== expected.message)
      return { path: path, reason: 'errors differ', actual: actual, expected: expected };
  } else if (
    actual instanceof NativePromise ||
    actual instanceof NativeWeakMap ||
    actual instanceof NativeWeakSet ||
    (NativeWeakRef && actual instanceof NativeWeakRef)
  ) {
    return {
      path: path,
      reason: 'Promise, WeakMap, WeakSet, and WeakRef values are compared by identity',
      actual: actual,
      expected: expected,
    };
  }

  const actualKeys = ownKeys(actual);
  const expectedKeys = ownKeys(expected);
  if (actualKeys.length !== expectedKeys.length)
    return {
      path: path,
      reason: 'objects have different numbers of own enumerable properties',
      actual: actual,
      expected: expected,
    };
  for (let i = 0; i < actualKeys.length; i++) {
    const key = actualKeys[i];
    if (!call(propertyIsEnumerable, expected, key))
      return {
        path: path,
        reason: 'property ' + formatKey(key) + ' is missing',
        actual: actual,
        expected: expected,
      };
    const mismatch = findMismatch(actual[key], expected[key], path + formatKey(key), seen);
    if (mismatch) return mismatch;
  }
  return null;
}

function describeMismatch(mismatch) {
  return (
    'values are not deeply equal: ' +
    mismatch.reason +
    ' at ' +
    mismatch.path +
    '\nactual: ' +
    formatValue(mismatch.actual) +
    '\nexpected: ' +
    formatValue(mismatch.expected)
  );
}

/**
 * Throws unless `actual` and `expected` are the same value under `Object.is`.
 * `NaN` equals `NaN`; `-0` and `0` differ; objects compare by identity.
 */
function equal(actual, expected, message) {
  checkMessage(message);
  if (objectIs(actual, expected)) return;
  throw new AssertionError(
    withMessage(
      message,
      'expected values to be strictly equal\nactual: ' +
        formatValue(actual) +
        '\nexpected: ' +
        formatValue(expected),
    ),
    { actual: actual, expected: expected, operator: 'equal' },
  );
}

/** Throws when `actual` and `expected` are the same value under `Object.is`. */
function notEqual(actual, expected, message) {
  checkMessage(message);
  if (!objectIs(actual, expected)) return;
  throw new AssertionError(
    withMessage(message, 'expected values to differ, but both are ' + formatValue(actual)),
    { actual: actual, expected: expected, operator: 'notEqual' },
  );
}

/**
 * Throws unless the values are structurally equal. See `findMismatch` for the
 * comparison rules; the message names the first differing path.
 */
function deepEqual(actual, expected, message) {
  checkMessage(message);
  const mismatch = findMismatch(actual, expected, '$', { map: new NativeMap(), log: [] });
  if (mismatch === null) return;
  throw new AssertionError(withMessage(message, describeMismatch(mismatch)), {
    actual: actual,
    expected: expected,
    operator: 'deepEqual',
  });
}

/** Throws when the values are structurally equal. */
function notDeepEqual(actual, expected, message) {
  checkMessage(message);
  if (findMismatch(actual, expected, '$', { map: new NativeMap(), log: [] }) !== null) return;
  throw new AssertionError(
    withMessage(message, 'expected values not to be deeply equal, but both are ' + formatValue(actual)),
    { actual: actual, expected: expected, operator: 'notDeepEqual' },
  );
}

/** Throws when `value` is falsy. */
function ok(value, message) {
  checkMessage(message);
  if (value) return;
  throw new AssertionError(withMessage(message, 'expected a truthy value, got ' + formatValue(value)), {
    actual: value,
    expected: true,
    operator: 'ok',
  });
}

/** Throws unconditionally with the supplied message. */
function fail(message) {
  checkMessage(message);
  throw new AssertionError(message === undefined ? 'failed' : message, { operator: 'fail' });
}

function checkCallback(fn) {
  if (typeof fn !== 'function') throw new NativeTypeError('callback must be a function');
  if (invalidCallback(fn))
    throw new NativeTypeError(
      'assert expects a synchronous function; async functions, generator functions and proxies are not supported',
    );
}

function checkCallbackReturn(method, value) {
  const unsupported = unsupportedReturn(value);
  if (unsupported !== null)
    throw new NativeTypeError('assert.' + method + ' callback returned ' + unsupported + '; only synchronous callbacks are supported');
}

/**
 * Throws unless calling `fn` throws. Returns the thrown value.
 *
 * `expected` may be an error constructor (checked with `instanceof`), a RegExp
 * matched against the thrown error's message (or the string form of a non-Error
 * value), or omitted to accept any throw. Unsupported callback returns are
 * rejected with a TypeError.
 */
function throws(fn, expected, message) {
  checkMessage(message);
  checkCallback(fn);
  if (
    expected !== undefined &&
    expected !== null &&
    !(expected instanceof NativeRegExp) &&
    typeof expected !== 'function'
  )
    throw new NativeTypeError('expected must be an error constructor, a RegExp, or omitted');
  if (typeof expected === 'function' && !(expected.prototype && typeof expected.prototype === 'object'))
    throw new NativeTypeError('expected must be an error constructor, a RegExp, or omitted');

  let didThrow = false;
  let thrown;
  let returned;
  try {
    returned = fn();
  } catch (error) {
    didThrow = true;
    thrown = error;
  }
  if (!didThrow) {
    checkCallbackReturn('throws', returned);
    // Keep the actual value on the error without inspecting its properties
    // or prototypes for the text diagnostic.
    const detail = isObjectLike(returned) ? 'a value' : formatValue(returned);
    throw new AssertionError(withMessage(message, 'expected the function to throw, but it returned ' + detail), {
      actual: returned,
      operator: 'throws',
    });
  }
  if (expected !== undefined && expected !== null) {
    if (expected instanceof NativeRegExp) {
      const matcher = new NativeRegExp(expected.source, expected.flags);
      const text = thrown instanceof NativeError ? NativeString(thrown.message) : NativeString(thrown);
      if (!call(regexpTest, matcher, text))
        throw new AssertionError(
          withMessage(
            message,
            'expected the thrown message to match ' +
              NativeString(expected) +
              ', but it was ' +
              formatValue(text),
          ),
          { actual: text, expected: expected, operator: 'throws' },
        );
    } else if (!(thrown instanceof expected)) {
      throw new AssertionError(
        withMessage(
          message,
          'expected the function to throw ' +
            (expected.name || 'the expected error') +
            ', but it threw ' +
            formatValue(thrown),
        ),
        { actual: thrown, expected: expected, operator: 'throws' },
      );
    }
  }
  return thrown;
}

/**
 * Throws when calling `fn` throws, and otherwise returns its return value.
 * Unsupported callback returns are rejected with a TypeError.
 */
function doesNotThrow(fn, message) {
  checkMessage(message);
  checkCallback(fn);
  let returned;
  try {
    returned = fn();
  } catch (error) {
    throw new AssertionError(
      withMessage(message, 'expected the function not to throw, but it threw ' + formatValue(error)),
      { actual: error, operator: 'doesNotThrow' },
    );
  }
  checkCallbackReturn('doesNotThrow', returned);
  return returned;
}

return {AssertionError, equal, notEqual, deepEqual, notDeepEqual, ok, fail, throws, doesNotThrow};
