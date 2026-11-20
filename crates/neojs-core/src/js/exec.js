// `neo:exec` wrapper. Every host callback is synchronous and returns JSON.
// Types and input validation stay here so the host sees only checked values;
// `spawn` caches the single terminal answer so repeated calls need no live
// supervisor.

function failure(response) {
  const error = new Error(response.error.message);
  error.code = response.error.code;
  return error;
}

function decode(response) {
  if (response.error) throw failure(response);
  const result = response.result;
  if (result.stdout !== undefined) result.stdout = Uint8Array.from(result.stdout);
  if (result.stderr !== undefined) result.stderr = Uint8Array.from(result.stderr);
  return result;
}

function copyResult(result) {
  return {
    code: result.code,
    success: result.success,
    complete: result.complete,
    stdout: result.stdout.slice(),
    stderr: result.stderr.slice(),
  };
}

function checkProgram(program, args) {
  if (typeof program !== 'string') throw new TypeError('program must be a string');
  if (!Array.isArray(args) || !args.every((arg) => typeof arg === 'string'))
    throw new TypeError('args must be an array of strings');
}

export function exec(program, args = [], input = new Uint8Array()) {
  checkProgram(program, args);
  if (!(input instanceof Uint8Array)) throw new TypeError('input must be a Uint8Array');
  return decode(JSON.parse(globalThis.__neoExec(program, args, Array.from(input))));
}

export function spawn(program, args = []) {
  checkProgram(program, args);
  const id = decode(JSON.parse(globalThis.__neoExecSpawn(program, args))).id;
  const cache = { done: false };

  function invoke(action, input) {
    if (cache.done) {
      if (cache.error) {
        const error = new Error(cache.error.message);
        error.code = cache.error.code;
        throw error;
      }
      return copyResult(cache.result);
    }
    const response = JSON.parse(globalThis.__neoExecCall(id, action, input));
    if (response.error) {
      cache.done = true;
      cache.error = response.error;
      throw failure(response);
    }
    const result = decode(response);
    cache.done = true;
    cache.result = result;
    return copyResult(result);
  }

  return {
    wait() {
      return invoke('wait', []);
    },
    communicate(input = new Uint8Array()) {
      if (!(input instanceof Uint8Array)) throw new TypeError('input must be a Uint8Array');
      return invoke('communicate', Array.from(input));
    },
    kill() {
      return invoke('kill', []);
    },
  };
}
