//! Synchronous standard streams, runtime working directory, and host process
//! elevation status exposed as `neo:process`.
//!
//! The module is importable with [`crate::Permissions::process`] or
//! [`crate::Permissions::stdio`]. Writing `stdout`/`stderr`, and reading or
//! changing the runtime working directory, are safe for a Rust embedder: they
//! touch only this runtime's state and the host streams that `console` already
//! reaches. Reading the host's standard input requires [`crate::Permissions::stdio`]
//! or [`crate::Permissions::process`], because that consumes input the embedder
//! or an interactive user may be using. `stdio` is the narrower option: it
//! grants stdin without granting child execution.
//!
//! `chdir` validates the target with a directory metadata check but reads no
//! file contents. `cwd` returns the runtime's stored path.
//! `isElevated` reads process status without changing privileges.
//!
//! Every host callback is private to the module factory. Nothing is installed
//! on the global object, so user code cannot replace a callback before or after
//! the import. The JavaScript wrapper captures the constructors it uses before
//! user code runs.
//!
//! Stream operations are synchronous and binary-safe. Writes are handed
//! directly to the host stream without an intermediate buffer, so memory use
//! is bounded by the caller's chunk. Reads write into the caller's
//! `Uint8Array`; the host does not allocate a copy. A zero-length read buffer
//! returns `0`; end of file returns `null`. `close()` is a no-op because the
//! streams belong to the host process, and closing them would affect the
//! embedder. `isTerm()` uses the host descriptor's terminal status.
//!
//! `stdin.read`/`readSync` can block until input or end of file. There is no
//! timeout and no nonblocking mode. A noninteractive run with closed stdin sees
//! end of file immediately; an interactive terminal can block indefinitely.

use std::{
    cell::RefCell,
    io::{self, IsTerminal, Read, Write},
    rc::Rc,
};

use rquickjs::{
    Ctx, Error as JsError, Function, Object, Persistent, Result as JsResult, TypedArray, Value,
    module::{Declarations, Exports, ModuleDef},
};

use crate::{
    cwd::{CwdError, WorkingDirectory},
    elevation::{self, ElevationError},
};

const EXPORTS: [&str; 9] = [
    "default",
    "cwd",
    "chdir",
    "isElevated",
    "stdin",
    "stdout",
    "stderr",
    "ChangeDirectoryError",
    "ElevationError",
];

/// Persistent factory and working-directory state for `neo:process`.
pub(crate) struct ProcessState {
    pub(crate) factory: Option<Persistent<Function<'static>>>,
    cwd: Rc<RefCell<WorkingDirectory>>,
    #[cfg(test)]
    query_elevation: fn() -> Result<bool, ElevationError>,
}

impl ProcessState {
    pub(crate) fn new(cwd: Rc<RefCell<WorkingDirectory>>) -> Self {
        Self {
            factory: None,
            cwd,
            #[cfg(test)]
            query_elevation: elevation::is_elevated,
        }
    }
}

// ProcessState contains no borrowed JS value; its persistent handle is released
// before the context and runtime are dropped.
unsafe impl<'js> rquickjs::JsLifetime<'js> for ProcessState {
    type Changed<'to> = ProcessState;
}

/// The native `neo:process` module.
pub(crate) struct ProcessModule;

impl ModuleDef for ProcessModule {
    fn declare<'js>(decl: &Declarations<'js>) -> JsResult<()> {
        for name in EXPORTS {
            decl.declare(name)?;
        }
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> JsResult<()> {
        let state = Rc::clone(
            &*ctx
                .userdata::<Rc<RefCell<ProcessState>>>()
                .ok_or(JsError::Unknown)?,
        );
        let cwd = Rc::clone(&state.borrow().cwd);

        let native = Object::new_proto(ctx.clone(), None)?;
        native.set(
            "cwd",
            Function::new(ctx.clone(), {
                let cwd = Rc::clone(&cwd);
                move || cwd_response(&cwd)
            })?,
        )?;
        native.set(
            "chdir",
            Function::new(ctx.clone(), {
                let cwd = Rc::clone(&cwd);
                move |directory: String| chdir_response(&cwd, &directory)
            })?,
        )?;
        #[cfg(not(test))]
        let query_elevation = elevation::is_elevated;
        #[cfg(test)]
        let query_elevation = state.borrow().query_elevation;
        native.set(
            "isElevated",
            Function::new(ctx.clone(), move || elevation_response(query_elevation()))?,
        )?;
        native.set(
            "writeStdoutBytes",
            Function::new(ctx.clone(), write_stdout_bytes)?,
        )?;
        native.set(
            "writeStdoutText",
            Function::new(ctx.clone(), write_stdout_text)?,
        )?;
        native.set(
            "writeStderrBytes",
            Function::new(ctx.clone(), write_stderr_bytes)?,
        )?;
        native.set(
            "writeStderrText",
            Function::new(ctx.clone(), write_stderr_text)?,
        )?;
        native.set(
            "stdoutIsTerm",
            Function::new(ctx.clone(), || io::stdout().is_terminal())?,
        )?;
        native.set(
            "stderrIsTerm",
            Function::new(ctx.clone(), || io::stderr().is_terminal())?,
        )?;
        native.set(
            "stdinIsTerm",
            Function::new(ctx.clone(), || io::stdin().is_terminal())?,
        )?;
        native.set("readStdin", Function::new(ctx.clone(), read_stdin)?)?;

        let factory = state
            .borrow()
            .factory
            .as_ref()
            .cloned()
            .ok_or(JsError::Unknown)?
            .restore(ctx)?;
        let api: Object = factory.call((native,))?;
        for name in EXPORTS {
            if name == "default" {
                exports.export(name, api.clone())?;
            } else {
                exports.export(name, api.get::<_, Value>(name)?)?;
            }
        }
        Ok(())
    }
}

/// Build the private JavaScript wrapper in the fresh realm.
///
/// It validates JavaScript types, turns host response objects into stable
/// errors, and builds the stream objects. The wrapper never uses a host global.
pub(crate) fn factory<'js>(ctx: &Ctx<'js>) -> JsResult<Function<'js>> {
    ctx.eval(
        r#"(function() {
            'use strict';
            const NativeError = Error;
            const NativeTypeError = TypeError;
            const NativeUint8Array = Uint8Array;
            const parseJSON = JSON.parse;
            const defineProperty = Object.defineProperty;

            class ChangeDirectoryError extends NativeError {
                constructor(message, options) {
                    super(message);
                    this.name = 'ChangeDirectoryError';
                    if (options && 'cause' in options) this.cause = options.cause;
                }
            }

            class ElevationError extends NativeError {
                constructor(message) {
                    super(message);
                    defineProperty(this, 'name', {
                        __proto__: null,
                        value: 'ElevationError',
                        writable: true,
                        configurable: true,
                    });
                }
            }

            function unwrap(text, ErrorClass) {
                const response = parseJSON(text);
                if (response.ok) return response.value;
                const error = new ErrorClass(response.message);
                defineProperty(error, 'code', {
                    __proto__: null,
                    value: response.code,
                    writable: true,
                    configurable: true,
                    enumerable: false,
                });
                throw error;
            }

            function requireBytes(data) {
                if (!(data instanceof NativeUint8Array)) {
                    throw new NativeTypeError('data must be a Uint8Array');
                }
                return data;
            }

            function write(bytes, text, chunk) {
                if (typeof chunk === 'string') return unwrap(text(chunk), NativeError);
                if (chunk instanceof NativeUint8Array) return unwrap(bytes(chunk), NativeError);
                throw new NativeTypeError('chunk must be a string or Uint8Array');
            }

            function read(native, data) {
                requireBytes(data);
                return unwrap(native.readStdin(data), NativeError);
            }

            return function(native) {
                function writer(bytes, text, isTerm) {
                    return {
                        write(chunk) { return write(bytes, text, chunk); },
                        writeSync(chunk) { return write(bytes, text, chunk); },
                        isTerm() { return isTerm(); },
                        close() {},
                    };
                }

                const stdout = writer(
                    native.writeStdoutBytes,
                    native.writeStdoutText,
                    native.stdoutIsTerm,
                );
                const stderr = writer(
                    native.writeStderrBytes,
                    native.writeStderrText,
                    native.stderrIsTerm,
                );
                const stdin = {
                    read(data) { return read(native, data); },
                    readSync(data) { return read(native, data); },
                    isTerm() { return native.stdinIsTerm(); },
                    close() {},
                };

                function cwd() {
                    return unwrap(native.cwd(), NativeError);
                }
                function chdir(directory) {
                    if (typeof directory !== 'string') {
                        throw new NativeTypeError('directory must be a string');
                    }
                    unwrap(native.chdir(directory), ChangeDirectoryError);
                }
                function isElevated() {
                    return unwrap(native.isElevated(), ElevationError);
                }

                return {
                    cwd,
                    chdir,
                    isElevated,
                    stdin,
                    stdout,
                    stderr,
                    ChangeDirectoryError,
                    ElevationError,
                };
            };
        })()"#,
    )
}

fn ok_response(value: serde_json::Value) -> String {
    serde_json::json!({ "ok": true, "value": value }).to_string()
}

fn error_response(error: CwdError) -> String {
    coded_error(error.code, error.message)
}

fn coded_error(code: &str, message: &str) -> String {
    serde_json::json!({
        "ok": false,
        "code": code,
        "message": message,
    })
    .to_string()
}

fn elevation_response(status: Result<bool, ElevationError>) -> String {
    match status {
        Ok(elevated) => ok_response(elevated.into()),
        Err(error) => coded_error(error.code, error.message),
    }
}

fn io_error() -> String {
    serde_json::json!({
        "ok": false,
        "code": "PROCESS_IO",
        "message": "standard stream operation failed",
    })
    .to_string()
}

fn detached_error() -> String {
    serde_json::json!({
        "ok": false,
        "code": "PROCESS_IO",
        "message": "the Uint8Array buffer is detached",
    })
    .to_string()
}

fn cwd_response(cwd: &Rc<RefCell<WorkingDirectory>>) -> String {
    match cwd.borrow().text() {
        Ok(text) => ok_response(text.into()),
        Err(error) => error_response(error),
    }
}

fn chdir_response(cwd: &Rc<RefCell<WorkingDirectory>>, directory: &str) -> String {
    match cwd.borrow_mut().chdir(directory) {
        Ok(()) => ok_response(serde_json::Value::Null),
        Err(error) => error_response(error),
    }
}

fn write_stdout_bytes<'js>(data: TypedArray<'js, u8>) -> String {
    let bytes = match data.as_raw() {
        // SAFETY: no JavaScript runs while the slice is borrowed; it is used
        // only for the synchronous write below.
        Some(raw) => unsafe { raw.as_ref() },
        None => return detached_error(),
    };
    match write_stream(io::stdout().lock(), bytes) {
        Ok(written) => ok_response(written.into()),
        Err(()) => io_error(),
    }
}

fn write_stderr_bytes<'js>(data: TypedArray<'js, u8>) -> String {
    let bytes = match data.as_raw() {
        // SAFETY: no JavaScript runs while the slice is borrowed; it is used
        // only for the synchronous write below.
        Some(raw) => unsafe { raw.as_ref() },
        None => return detached_error(),
    };
    match write_stream(io::stderr().lock(), bytes) {
        Ok(written) => ok_response(written.into()),
        Err(()) => io_error(),
    }
}

fn write_stdout_text(text: String) -> String {
    match write_stream(io::stdout().lock(), text.as_bytes()) {
        Ok(written) => ok_response(written.into()),
        Err(()) => io_error(),
    }
}

fn write_stderr_text(text: String) -> String {
    match write_stream(io::stderr().lock(), text.as_bytes()) {
        Ok(written) => ok_response(written.into()),
        Err(()) => io_error(),
    }
}

fn write_stream(mut stream: impl Write, bytes: &[u8]) -> Result<usize, ()> {
    stream
        .write_all(bytes)
        .and_then(|()| stream.flush())
        .map_err(|_| ())?;
    Ok(bytes.len())
}

fn read_stdin<'js>(data: TypedArray<'js, u8>) -> String {
    let Some(raw) = data.as_raw() else {
        return detached_error();
    };
    let len = raw.len();
    if len == 0 {
        return ok_response(0.into());
    }
    // SAFETY: the buffer is alive for this call and no JavaScript runs while
    // the blocking read writes into it.
    let bytes = unsafe { std::slice::from_raw_parts_mut(raw.as_ptr() as *mut u8, len) };
    let mut stdin = io::stdin().lock();
    loop {
        match stdin.read(bytes) {
            Ok(0) => return ok_response(serde_json::Value::Null),
            Ok(count) => return ok_response(count.into()),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return io_error(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NeoJs, Permissions};

    #[test]
    fn elevation_exports_boolean_results_and_typed_failures() {
        for (query, expected) in [
            ((|| Ok(false)) as fn() -> _, "false"),
            (|| Ok(true), "true"),
            (|| Err(elevation::QUERY_FAILED), "'PROCESS_ELEVATION'"),
            (
                || Err(elevation::UNSUPPORTED),
                "'PROCESS_ELEVATION_UNSUPPORTED'",
            ),
        ] {
            let js = NeoJs::new(Permissions {
                stdio: true,
                ..Permissions::default()
            })
            .unwrap();
            js.stdio.borrow_mut().query_elevation = query;
            js.run_module(
                "elevation.js",
                &format!(
                    "import process, {{isElevated, ElevationError}} from 'neo:process';
                     if (process.isElevated !== isElevated ||
                         process.ElevationError !== ElevationError) throw Error('invalid exports');
                     let result;
                     try {{ result = isElevated(); }}
                     catch (error) {{
                         if (!(error instanceof ElevationError) || !(error instanceof Error) ||
                             error.name !== 'ElevationError') throw error;
                         result = error.code;
                     }}
                     if (result !== {expected}) throw Error('invalid elevation result');"
                ),
            )
            .unwrap();
        }
    }

    struct FailingWriter;
    impl Write for FailingWriter {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("injected"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn error_responses_do_not_echo_input() {
        let error = CwdError {
            code: "PROCESS_CHDIR",
            message: "cannot change the runtime working directory",
        };
        let text = error_response(error);
        assert!(text.contains("PROCESS_CHDIR"));
        assert!(!text.contains("secret-path"));
    }

    #[test]
    fn stream_write_failures_are_reported_without_panicking() {
        assert!(write_stream(FailingWriter, b"x").is_err());
        assert_eq!(write_stream(Vec::<u8>::new(), b"abc").unwrap(), 3);
    }
}
