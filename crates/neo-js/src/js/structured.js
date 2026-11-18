// Shared factory for the private `neo:toml` and `neo:yaml` wrappers.
//
// The native parse callback is supplied only to this closure during module
// evaluation; it is never placed on the global object. Everything this wrapper
// later calls is captured here, before user code runs, and every output
// property is an own data property with a null-prototype descriptor. Parsed
// values therefore never pass through a user-replaced constructor, setter, or
// `__proto__` hook.
(function () {
  'use strict';

  const defineProperty = Object.defineProperty;
  const createObject = Object.create;
  const parseJson = JSON.parse;
  const objectKeys = Object.keys;
  const isArray = Array.isArray;
  const hasOwn = Function.prototype.call.bind(Object.prototype.hasOwnProperty);
  const NativeError = Error;
  const NativeTypeError = TypeError;

  function defineValue(target, key, value) {
    defineProperty(target, key, {
      __proto__: null,
      value: value,
      enumerable: true,
      writable: true,
      configurable: true,
    });
  }

  function build(value) {
    if (value === null || typeof value !== 'object') return value;
    if (isArray(value)) {
      const array = [];
      for (let index = 0; index < value.length; index++) {
        defineValue(array, index, build(value[index]));
      }
      return array;
    }
    const object = createObject(null);
    const keys = objectKeys(value);
    for (let index = 0; index < keys.length; index++) {
      const key = keys[index];
      defineValue(object, key, build(value[key]));
    }
    return object;
  }

  return function create(parseNative, errorName, messagesJson, limitsJson) {
    const messages = parseJson(messagesJson);
    const limits = parseJson(limitsJson);

    class ParseError extends NativeError {
      constructor(code, line, column) {
        super(hasOwn(messages, code) ? messages[code] : 'invalid configuration source');
        defineProperty(this, 'name', {
          __proto__: null,
          value: errorName,
          writable: true,
          enumerable: false,
          configurable: true,
        });
        defineProperty(this, 'code', {
          __proto__: null,
          value: code,
          writable: true,
          enumerable: true,
          configurable: true,
        });
        defineProperty(this, 'line', {
          __proto__: null,
          value: line,
          writable: true,
          enumerable: true,
          configurable: true,
        });
        defineProperty(this, 'column', {
          __proto__: null,
          value: column,
          writable: true,
          enumerable: true,
          configurable: true,
        });
      }
    }

    function parse(source) {
      if (typeof source !== 'string') {
        throw new NativeTypeError('source must be a string');
      }
      const response = parseJson(parseNative(source));
      if (response.ok !== true) {
        const error = response.error;
        throw new ParseError(error.code, error.line, error.column);
      }
      return build(response.value);
    }

    // A null-prototype export object with own data properties: a polluted
    // Object.prototype setter cannot intercept a later assignment.
    const api = createObject(null);
    defineValue(api, 'parse', parse);
    defineValue(api, 'default', parse);
    defineValue(api, errorName, ParseError);
    defineValue(api, 'MAX_SOURCE_BYTES', limits.MAX_SOURCE_BYTES);
    defineValue(api, 'MAX_DEPTH', limits.MAX_DEPTH);
    defineValue(api, 'MAX_NODES', limits.MAX_NODES);
    defineValue(api, 'MAX_ITEMS', limits.MAX_ITEMS);
    defineValue(api, 'MAX_OUTPUT_BYTES', limits.MAX_OUTPUT_BYTES);
    return api;
  };
})()
