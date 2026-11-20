// Factory for the private `neo:dotenv` wrapper. The native parse callback is
// supplied only to this closure during module evaluation; it is never placed on
// the global object, so user code cannot replace or call it.
(function () {
  'use strict';

  const defineProperty = Object.defineProperty;
  const createObject = Object.create;
  const parseJson = JSON.parse;
  const NativeError = Error;

  const MAX_SOURCE_BYTES = 1048576;
  const MAX_ENTRIES = 16384;
  const MAX_KEY_BYTES = 1024;

  const MESSAGES = {
    __proto__: null,
    DOTENV_SYNTAX: 'invalid dotenv syntax',
    DOTENV_KEY: 'invalid or empty dotenv key',
    DOTENV_QUOTE: 'unterminated quoted dotenv value',
    DOTENV_ESCAPE: 'invalid dotenv escape sequence',
    DOTENV_LIMIT: 'dotenv input exceeds a documented limit',
    DOTENV_ENCODING: 'dotenv source is not valid UTF-16',
    DOTENV_NUL: 'dotenv source contains a NUL character',
  };

  class DotenvParseError extends NativeError {
    constructor(code, line, column) {
      super(MESSAGES[code] || 'invalid dotenv source');
      // Define own data properties with null-prototype descriptors so inherited
      // setters or descriptor fields cannot swallow or replace the stable code
      // and location.
      defineProperty(this, 'name', {
        __proto__: null,
        value: 'DotenvParseError',
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

  return function (parseNative) {
    /**
     * Parse `.env` content into a null-prototype record.
     *
     * The native callback is private to this closure. It checks the primitive
     * string and the UTF-16 and UTF-8 size bounds before converting, and it
     * reports the first source-order error with a stable code, one-based line,
     * and one-based Unicode-scalar column.
     */
    function parse(source) {
      const response = parseJson(parseNative(source));
      if (response.ok !== true) {
        const error = response.error;
        throw new DotenvParseError(error.code, error.line, error.column);
      }
      const values = createObject(null);
      for (let index = 0; index < response.entries.length; index++) {
        const entry = response.entries[index];
        defineProperty(values, entry[0], {
          // ToPropertyDescriptor reads inherited "get" and "set" too; never
          // expose a value-bearing descriptor to user-controlled prototypes.
          __proto__: null,
          value: entry[1],
          enumerable: true,
          writable: true,
          configurable: true,
        });
      }
      return values;
    }

    return {
      default: parse,
      parse,
      DotenvParseError,
      MAX_SOURCE_BYTES,
      MAX_ENTRIES,
      MAX_KEY_BYTES,
    };
  };
})()
