// Factory for the private `neo:fs` wrapper.
//
// The native callbacks are supplied only to this closure during runtime
// construction; they are never placed on the global object under a callable
// name. Every intrinsic this wrapper later calls is captured here, before user
// code runs. The API object has a null prototype and only own data properties,
// so a replaced global, a polluted `Object.prototype`, or a hostile getter
// cannot intercept a result. The module and the `fs` global share one API
// instance, while `neo:fs` exports a separate pristine object holding the same
// function values; replacing a property on `fs` cannot change an import.
(function () {
  'use strict';

  const defineProperty = Object.defineProperty;
  const createObject = Object.create;
  const setPrototypeOf = Object.setPrototypeOf;
  const ownKeys = Reflect.ownKeys;
  const getOwnPropertyDescriptor = Object.getOwnPropertyDescriptor;
  const bindCall = Function.prototype.bind.bind(Function.prototype.call);
  const typedArrayPrototype = Object.getPrototypeOf(Uint8Array.prototype);
  const typedArrayTag = bindCall(getOwnPropertyDescriptor(typedArrayPrototype, Symbol.toStringTag).get);
  const typedArrayBuffer = bindCall(getOwnPropertyDescriptor(typedArrayPrototype, 'buffer').get);
  const typedArrayByteLength = bindCall(getOwnPropertyDescriptor(typedArrayPrototype, 'byteLength').get);
  const hasOwn = Function.prototype.call.bind(Object.prototype.hasOwnProperty);
  const parseJson = JSON.parse;
  const isWellFormed = Function.prototype.call.bind(String.prototype.isWellFormed);
  const charCodeAt = Function.prototype.call.bind(String.prototype.charCodeAt);
  const NativeError = Error;
  const NativeTypeError = TypeError;
  const rejectPromise = Promise.reject.bind(Promise);
  const lengthOf = (value) => value.length;

  function defineValue(target, key, value) {
    defineProperty(target, key, {
      __proto__: null,
      value: value,
      enumerable: true,
      writable: true,
      configurable: true,
    });
  }

  return function (native, config) {
    const limits = config.limits;
    const messages = config.messages;

    function brandError(error, code) {
      defineProperty(error, 'name', {
        __proto__: null,
        value: 'FsError',
        writable: true,
        enumerable: false,
        configurable: true,
      });
      defineProperty(error, 'code', {
        __proto__: null,
        value: code,
        writable: true,
        enumerable: true,
        configurable: true,
      });
      return error;
    }

    class FsError extends NativeError {
      constructor(code, message) {
        super(
          typeof message === 'string' && message.length > 0
            ? message
            : (messages[code] || 'filesystem operation failed'),
        );
        brandError(this, code);
      }
    }
    const fsErrorPrototype = FsError.prototype;

    function requireString(value, name) {
      if (typeof value !== 'string') {
        throw new NativeTypeError(name + ' must be a string');
      }
      return value;
    }

    function requirePath(value, fail = (code) => new FsError(code)) {
      requireString(value, 'path');
      const maximum = limits.MAX_PATH_BYTES;
      if (lengthOf(value) > maximum) throw fail('FS_LIMIT');
      if (!isWellFormed(value)) throw fail('FS_ENCODING');
      let bytes = 0;
      for (let index = 0; index < lengthOf(value); index++) {
        const unit = charCodeAt(value, index);
        if (unit <= 0x7f) {
          bytes += 1;
        } else if (unit <= 0x7ff) {
          bytes += 2;
        } else if (unit >= 0xd800 && unit <= 0xdbff) {
          // `isWellFormed` guarantees the following code unit is a low
          // surrogate, so this pair encodes as one four-byte scalar.
          bytes += 4;
          index++;
        } else {
          bytes += 3;
        }
        if (bytes > maximum) throw fail('FS_LIMIT');
      }
      if (lengthOf(value) === 0) throw fail('FS_PATH');
      return value;
    }

    function requireOptions(value, allowedKeys) {
      if (value === undefined) return null;
      if (value === null || typeof value !== 'object') {
        throw new NativeTypeError('options must be an object');
      }
      const keys = ownKeys(value);
      if (keys.length > allowedKeys.length) {
        throw new NativeTypeError('options contains an unsupported property');
      }
      const snapshot = createObject(null);
      for (let index = 0; index < keys.length; index++) {
        const key = keys[index];
        if (typeof key !== 'string' || allowedKeys[0] !== key) {
          throw new NativeTypeError('options contains an unsupported property');
        }
        const descriptor = getOwnPropertyDescriptor(value, key);
        if (
          descriptor === undefined ||
          !hasOwn(descriptor, 'value') ||
          descriptor.enumerable !== true
        ) {
          throw new NativeTypeError('options properties must be own enumerable data properties');
        }
        defineValue(snapshot, key, descriptor.value);
      }
      return snapshot;
    }

    function requireBoolean(value, name) {
      if (typeof value !== 'boolean') {
        throw new NativeTypeError(name + ' must be a boolean');
      }
      return value;
    }

    function unwrap(response) {
      if (response.ok !== true) {
        throw new FsError(response.code, response.message);
      }
      return response.value;
    }

    function unwrapWith(response, fail) {
      // Async paths must not call the public FsError constructor: a script
      // can replace its prototype before an owner-thread rejection.
      if (response.ok !== true) {
        throw fail(response.code);
      }
      return response.value;
    }

    function readText(path) {
      requirePath(path);
      const snapshot = unwrap(native.snapshotPath(path));
      return unwrap(native.readText(snapshot));
    }

    function writeText(path, data) {
      requirePath(path);
      requireString(data, 'data');
      // Native code validates and snapshots text before resolving the path.
      unwrap(native.writeText(path, data));
    }

    function readFileSync(path) {
      requirePath(path);
      const snapshot = unwrap(native.snapshotPath(path));
      return unwrap(native.readFileSync(snapshot));
    }

    function makeError(code) {
      // The public class's constructor prototype can be changed by script.
      // Worker settlement must not run a replacement `super` constructor.
      const error = new NativeError(messages[code]);
      setPrototypeOf(error, fsErrorPrototype);
      return brandError(error, code);
    }

    function readFile(path) {
      // Outside a managed turn there is no owner to deliver a rejection.
      if (!native.canAsync()) throw makeError('FS_ASYNC_OUTSIDE_TURN');
      try {
        return native.readFile(requirePath(path, makeError), makeError);
      } catch (error) {
        return rejectPromise(error);
      }
    }

    function writeFileSync(path, data, options) {
      requirePath(path);
      // These captured getters read internal slots, not properties of data.
      // The tag getter rejects proxies and other brands without invoking them.
      if (typedArrayTag(data) !== 'Uint8Array') {
        throw new NativeTypeError('data must be a Uint8Array');
      }
      // Snapshot the live view before option reflection or any filesystem I/O.
      const bytes = unwrap(native.copyData(data, typedArrayBuffer(data), typedArrayByteLength(data)));
      const snapshot = unwrap(native.snapshotPath(path));
      const opts = requireOptions(options, ['overwrite']);
      let overwrite = true;
      if (opts !== null) {
        if (hasOwn(opts, 'overwrite')) {
          overwrite = requireBoolean(opts.overwrite, 'options.overwrite');
        }
      }
      unwrap(native.writeFileSync(snapshot, bytes, overwrite));
    }

    function writeFile(path, data, options) {
      // Outside a managed turn there is no owner to deliver a rejection.
      if (!native.canAsync()) throw makeError('FS_ASYNC_OUTSIDE_TURN');
      try {
        requirePath(path, makeError);
        // The tag getter rejects proxies and other brands without invoking
        // them. A detached or out-of-bounds view rejects through copyData.
        if (typedArrayTag(data) !== 'Uint8Array') {
          throw new NativeTypeError('data must be a Uint8Array');
        }
        // Snapshot the live view and the cwd before option reflection or any
        // filesystem I/O, matching the synchronous error ordering.
        const bytes = unwrapWith(native.copyData(data, typedArrayBuffer(data), typedArrayByteLength(data)), makeError);
        const snapshot = unwrapWith(native.snapshotPath(path), makeError);
        const opts = requireOptions(options, ['overwrite']);
        let overwrite = true;
        if (opts !== null) {
          if (hasOwn(opts, 'overwrite')) {
            overwrite = requireBoolean(opts.overwrite, 'options.overwrite');
          }
        }
        return native.writeFile(snapshot, bytes, overwrite, makeError);
      } catch (error) {
        return rejectPromise(error);
      }
    }

    function stat(path) {
      // Outside a managed turn there is no owner to deliver a rejection.
      if (!native.canAsync()) throw makeError('FS_ASYNC_OUTSIDE_TURN');
      try {
        return native.stat(requirePath(path, makeError), makeError);
      } catch (error) {
        return rejectPromise(error);
      }
    }

    function statSync(path) {
      requirePath(path);
      const snapshot = unwrap(native.snapshotPath(path));
      return unwrap(native.statSync(snapshot));
    }

    function lstat(path) {
      // Outside a managed turn there is no owner to deliver a rejection.
      if (!native.canAsync()) throw makeError('FS_ASYNC_OUTSIDE_TURN');
      try {
        return native.lstat(requirePath(path, makeError), makeError);
      } catch (error) {
        return rejectPromise(error);
      }
    }

    function lstatSync(path) {
      requirePath(path);
      const snapshot = unwrap(native.snapshotPath(path));
      return unwrap(native.lstatSync(snapshot));
    }

    function exists(path) {
      requirePath(path);
      const snapshot = unwrap(native.snapshotPath(path));
      return unwrap(native.exists(snapshot));
    }

    function readDir(path) {
      // Outside a managed turn there is no owner to deliver a rejection.
      if (!native.canAsync()) throw makeError('FS_ASYNC_OUTSIDE_TURN');
      try {
        return native.readDir(requirePath(path, makeError), makeError);
      } catch (error) {
        return rejectPromise(error);
      }
    }

    function readDirSync(path) {
      requirePath(path);
      const snapshot = unwrap(native.snapshotPath(path));
      return parseJson(unwrap(native.readDirSync(snapshot)));
    }

    function mkdir(path, options) {
      // Outside a managed turn there is no owner to deliver a rejection.
      if (!native.canAsync()) throw makeError('FS_ASYNC_OUTSIDE_TURN');
      try {
        requirePath(path, makeError);
        // Snapshot the cwd before option reflection can run script.
        const snapshot = unwrapWith(native.snapshotPath(path), makeError);
        const opts = requireOptions(options, ['recursive']);
        let recursive = false;
        if (opts !== null) {
          if (hasOwn(opts, 'recursive')) {
            recursive = requireBoolean(opts.recursive, 'options.recursive');
          }
        }
        return native.mkdir(snapshot, recursive, makeError);
      } catch (error) {
        return rejectPromise(error);
      }
    }

    function mkdirSync(path, options) {
      requirePath(path);
      const snapshot = unwrap(native.snapshotPath(path));
      const opts = requireOptions(options, ['recursive']);
      let recursive = false;
      if (opts !== null) {
        if (hasOwn(opts, 'recursive')) {
          recursive = requireBoolean(opts.recursive, 'options.recursive');
        }
      }
      unwrap(native.mkdirSync(snapshot, recursive));
    }

    function remove(path, options) {
      requirePath(path);
      // Reject ambiguous terminal components before cwd resolution. The
      // resolved absolute path is captured before proxy reflection can run JS.
      const snapshot = unwrap(native.snapshotRemovePath(path));
      const opts = requireOptions(options, ['recursive']);
      let recursive = false;
      if (opts !== null) {
        if (hasOwn(opts, 'recursive')) {
          recursive = requireBoolean(opts.recursive, 'options.recursive');
        }
      }
      unwrap(native.remove(snapshot, recursive));
    }

    function rename(source, destination) {
      requirePath(source);
      requirePath(destination);
      unwrap(native.rename(source, destination));
    }

    function copyFile(source, destination) {
      requireString(source, 'source');
      requireString(destination, 'destination');
      requirePath(source);
      requirePath(destination);
      unwrap(native.copyFile(source, destination));
    }

    const api = createObject(null);
    defineValue(api, 'readText', readText);
    defineValue(api, 'writeText', writeText);
    defineValue(api, 'readFile', readFile);
    defineValue(api, 'readFileSync', readFileSync);
    defineValue(api, 'writeFile', writeFile);
    defineValue(api, 'writeFileSync', writeFileSync);
    defineValue(api, 'stat', stat);
    defineValue(api, 'statSync', statSync);
    defineValue(api, 'lstat', lstat);
    defineValue(api, 'lstatSync', lstatSync);
    defineValue(api, 'exists', exists);
    defineValue(api, 'readDir', readDir);
    defineValue(api, 'readDirSync', readDirSync);
    defineValue(api, 'mkdir', mkdir);
    defineValue(api, 'mkdirSync', mkdirSync);
    defineValue(api, 'remove', remove);
    defineValue(api, 'rename', rename);
    defineValue(api, 'copyFile', copyFile);
    defineValue(api, 'FsError', FsError);
    defineValue(api, 'MAX_PATH_BYTES', limits.MAX_PATH_BYTES);
    defineValue(api, 'MAX_READ_BYTES', limits.MAX_READ_BYTES);
    defineValue(api, 'MAX_WRITE_BYTES', limits.MAX_WRITE_BYTES);
    defineValue(api, 'MAX_LIST_ENTRIES', limits.MAX_LIST_ENTRIES);
    defineValue(api, 'MAX_LIST_NAME_BYTES', limits.MAX_LIST_NAME_BYTES);
    defineValue(api, 'MAX_LIST_TOTAL_BYTES', limits.MAX_LIST_TOTAL_BYTES);
    return api;
  };
})();
