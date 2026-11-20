// Factory for the private `neo:semver` wrapper.
//
// The native callbacks are supplied only to this closure during module
// evaluation; they are never placed on the global object. Every intrinsic this
// wrapper later calls is captured here, before user code runs, and every output
// property is an own data property with a null-prototype descriptor. A
// polluted `Object.prototype`, `Array.prototype`, or replaced global therefore
// cannot receive, replace, or intercept a result.
(function () {
  'use strict';

  const defineProperty = Object.defineProperty;
  const createObject = Object.create;
  const parseJson = JSON.parse;
  const isArray = Array.isArray;
  const isSafeInteger = Number.isSafeInteger;
  const toString = String;
  const lengthOf = (value) => value.length;
  const call = Function.prototype.call;
  const charCodeAt = call.bind(String.prototype.charCodeAt);
  const NativeError = Error;
  const NativeTypeError = TypeError;

  const MESSAGES = {
    __proto__: null,
    SEMVER_INVALID: 'invalid semantic version',
    SEMVER_RANGE: 'invalid semantic version range',
    SEMVER_INCREMENT: 'invalid increment request',
    SEMVER_OVERFLOW: 'semantic version value exceeds the supported range',
    SEMVER_LIMIT: 'semantic version input exceeds a documented limit',
  };

  function defineValue(target, key, value) {
    defineProperty(target, key, {
      __proto__: null,
      value: value,
      enumerable: true,
      writable: true,
      configurable: true,
    });
  }

  class SemVerError extends NativeError {
    constructor(code, message) {
      super(typeof message === 'string' && message.length > 0
        ? message
        : (MESSAGES[code] || 'invalid semantic version'));
      defineProperty(this, 'name', {
        __proto__: null,
        value: 'SemVerError',
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
    }
  }

  function limitError() {
    return new SemVerError('SEMVER_LIMIT');
  }

  function requireString(value, name) {
    if (typeof value !== 'string') {
      throw new NativeTypeError(name + ' must be a string');
    }
    return value;
  }

  function requireBoundedString(value, name, maximum) {
    requireString(value, name);
    // `length` is the primitive string's own length slot. Reading it does not
    // invoke an inherited getter, and a Proxy/boxed String is rejected above
    // by the primitive brand check.
    if (lengthOf(value) > maximum) throw limitError();
    return value;
  }

  function identifierSectionExceeds(text, start, end, limits) {
    if (start >= end) return false;
    let count = 1;
    let longest = 0;
    let current = 0;
    for (let index = start; index < end; index++) {
      if (charCodeAt(text, index) === 46) {
        if (current > longest) longest = current;
        count += 1;
        if (count > limits.MAX_IDENTIFIERS) return true;
        current = 0;
      } else {
        current += 1;
      }
    }
    if (current > longest) longest = current;
    return longest > limits.MAX_IDENTIFIER_BYTES;
  }

  // Count prerelease and build identifiers before the string crosses into
  // native code. The scan is linear in the already length-bounded string and
  // uses the captured String.prototype.charCodeAt.
  function versionExceedsLimits(text, limits) {
    const length = lengthOf(text);
    let dots = 0;
    let pre = -1;
    let build = -1;
    for (let index = 0; index < length; index++) {
      const code = charCodeAt(text, index);
      if (code === 46) {
        dots += 1;
      } else if (code === 45 && pre < 0 && build < 0 && dots >= 2) {
        pre = index;
      } else if (code === 43 && build < 0 && dots >= 2) {
        build = index;
      }
    }
    if (pre >= 0) {
      const end = build > pre ? build : length;
      if (identifierSectionExceeds(text, pre + 1, end, limits)) return true;
    }
    return build >= 0 && identifierSectionExceeds(text, build + 1, length, limits);
  }

  function requireVersion(value, name, limits) {
    requireBoundedString(value, name, limits.MAX_VERSION_BYTES);
    if (versionExceedsLimits(value, limits)) throw limitError();
    return value;
  }

  function unwrap(response) {
    if (response.ok !== true) {
      const error = response.error;
      throw new SemVerError(error.code, error.message);
    }
    return response.value;
  }

  function stringArray(value, name, limits) {
    if (value === undefined) return [];
    if (!isArray(value)) {
      throw new NativeTypeError(name + ' must be an array of strings');
    }
    // A proxy can return any value, including an object with hostile coercion.
    // Validate the single captured primitive before using it as a loop bound.
    const length = lengthOf(value);
    if (typeof length !== 'number' || !isSafeInteger(length) || length < 0) {
      throw new NativeTypeError(name + ' must have a non-negative integer length');
    }
    if (length > limits.MAX_IDENTIFIERS) throw limitError();
    const copy = [];
    for (let index = 0; index < length; index++) {
      const item = value[index];
      if (typeof item !== 'string') {
        throw new NativeTypeError(name + ' must contain only strings');
      }
      if (lengthOf(item) > limits.MAX_IDENTIFIER_BYTES) throw limitError();
      defineValue(copy, index, item);
    }
    return copy;
  }

  function coreComponent(value, name, limits) {
    let text;
    if (typeof value === 'string') {
      text = value;
    } else if (typeof value === 'number') {
      if (!isSafeInteger(value) || value < 0) {
        throw new NativeTypeError(
          name + ' must be a non-negative safe integer or a decimal string'
        );
      }
      text = toString(value);
    } else {
      throw new NativeTypeError(name + ' must be a number or a decimal string');
    }
    if (lengthOf(text) > limits.MAX_CORE_BYTES) throw limitError();
    return text;
  }

  function addIdentifiers(total, identifiers) {
    if (identifiers.length === 0) return total;
    total += 1;
    for (let index = 0; index < identifiers.length; index++) {
      total += lengthOf(identifiers[index]);
      if (index > 0) total += 1;
    }
    return total;
  }

  function buildVersion(value) {
    const version = createObject(null);
    defineValue(version, 'version', value.version);
    defineValue(version, 'major', value.major);
    defineValue(version, 'minor', value.minor);
    defineValue(version, 'patch', value.patch);
    const prerelease = [];
    for (let index = 0; index < value.prerelease.length; index++) {
      defineValue(prerelease, index, value.prerelease[index]);
    }
    defineValue(version, 'prerelease', prerelease);
    const build = [];
    for (let index = 0; index < value.build.length; index++) {
      defineValue(build, index, value.build[index]);
    }
    defineValue(version, 'build', build);
    return version;
  }

  return function create(
    validNative,
    compareNative,
    parseNative,
    formatNative,
    incrementNative,
    satisfiesNative,
    validRangeNative,
    limitsJson
  ) {
    const limits = parseJson(limitsJson);

    /** True for a syntactically valid SemVer version; false otherwise. */
    function valid(version) {
      requireString(version, 'version');
      if (lengthOf(version) > limits.MAX_VERSION_BYTES) return false;
      if (versionExceedsLimits(version, limits)) return false;
      return validNative(version);
    }

    /** Compare precedence, ignoring build metadata. */
    function compare(a, b) {
      return unwrap(parseJson(compareNative(
        requireVersion(a, 'a', limits),
        requireVersion(b, 'b', limits)
      )));
    }

    /**
     * Parse a version into exact components. Numeric components are decimal
     * strings so an identifier above Number.MAX_SAFE_INTEGER is not rounded.
     */
    function parse(version) {
      return buildVersion(unwrap(parseJson(parseNative(
        requireVersion(version, 'version', limits)
      ))));
    }

    /** Build the canonical string for parsed components. */
    function format(value) {
      if (value === null || typeof value !== 'object' || isArray(value)) {
        throw new NativeTypeError('version must be an object');
      }
      // Property reads on this object may run getters or Proxy traps. The
      // limits cover the primitive strings and array lengths those reads
      // produce; they do not make an arbitrary object side-effect free.
      const major = coreComponent(value.major, 'major', limits);
      const minor = coreComponent(value.minor, 'minor', limits);
      const patch = coreComponent(value.patch, 'patch', limits);
      const prerelease = stringArray(value.prerelease, 'prerelease', limits);
      const build = stringArray(value.build, 'build', limits);
      let total = lengthOf(major) + lengthOf(minor) + lengthOf(patch) + 2;
      total = addIdentifiers(total, prerelease);
      total = addIdentifiers(total, build);
      if (total > limits.MAX_VERSION_BYTES) throw limitError();
      return unwrap(parseJson(formatNative(
        major,
        minor,
        patch,
        prerelease,
        build
      )));
    }

    /** Increment a version and return its canonical string. */
    function increment(version, release, identifier) {
      requireVersion(version, 'version', limits);
      requireString(release, 'release');
      // Reject unsupported names before the binding converts a JS string to
      // native storage, including arbitrarily long primitive strings.
      if (release !== 'major' && release !== 'minor' && release !== 'patch' &&
          release !== 'premajor' && release !== 'preminor' &&
          release !== 'prepatch' && release !== 'prerelease') {
        throw new SemVerError('SEMVER_INCREMENT');
      }
      if (identifier !== undefined) {
        requireString(identifier, 'identifier');
        if (lengthOf(identifier) > limits.MAX_IDENTIFIER_BYTES) throw limitError();
        return unwrap(parseJson(incrementNative(version, release, identifier)));
      }
      return unwrap(parseJson(incrementNative(version, release)));
    }

    /** True when a version satisfies a Cargo-style range. */
    function satisfies(version, range) {
      return unwrap(parseJson(satisfiesNative(
        requireVersion(version, 'version', limits),
        requireBoundedString(range, 'range', limits.MAX_RANGE_BYTES)
      )));
    }

    /** True for a syntactically valid range; false otherwise. */
    function validRange(range) {
      requireString(range, 'range');
      if (lengthOf(range) > limits.MAX_RANGE_BYTES) return false;
      return validRangeNative(range);
    }

    // A null-prototype export object with own data properties: a polluted
    // Object.prototype setter cannot intercept a later assignment and the
    // module cannot accidentally inherit a documented-looking name.
    const api = createObject(null);
    defineValue(api, 'valid', valid);
    defineValue(api, 'compare', compare);
    defineValue(api, 'parse', parse);
    defineValue(api, 'format', format);
    defineValue(api, 'increment', increment);
    defineValue(api, 'satisfies', satisfies);
    defineValue(api, 'validRange', validRange);
    defineValue(api, 'SemVerError', SemVerError);
    defineValue(api, 'MAX_VERSION_BYTES', limits.MAX_VERSION_BYTES);
    defineValue(api, 'MAX_RANGE_BYTES', limits.MAX_RANGE_BYTES);
    defineValue(api, 'MAX_CORE_BYTES', limits.MAX_CORE_BYTES);
    defineValue(api, 'MAX_IDENTIFIERS', limits.MAX_IDENTIFIERS);
    defineValue(api, 'MAX_IDENTIFIER_BYTES', limits.MAX_IDENTIFIER_BYTES);
    return api;
  };
})()
