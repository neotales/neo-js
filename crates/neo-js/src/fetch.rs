//! Synchronous HTTP and HTTPS downloads for `neo:fetch`.
//!
//! The transport is `reqwest` over rustls with the bundled Mozilla webpki roots
//! (certificate-chain and hostname verification are always on). This module
//! adds the URL policy, redirect and size bounds, destination handling,
//! progress reporting, and the stable JavaScript error codes.
//! Transport and file work run in a joined worker (at most 32 process-wide);
//! synchronous progress callbacks remain on the caller's thread. Exceeding the
//! worker cap fails with `FETCH_NETWORK` rather than blocking nested calls.
//!
//! # Permissions
//!
//! `neo:fetch` is absent unless [`crate::Permissions::network`] is set.
//! `download` also requires [`crate::Permissions::filesystem`], because it
//! writes the destination. A relative destination is resolved against the
//! runtime working directory, not the host process directory. The filesystem
//! check happens after JavaScript argument validation (which can run option
//! getters or proxy traps) but before the URL is parsed, any progress callback
//! is called, or the download performs file or network I/O.
//!
//! The network permission is broad outbound authority under the current user's
//! network access. It is not an allowlist, a sandbox, or an SSRF defense:
//! loopback, private, link-local, and metadata addresses are reachable, and a
//! redirect may change the host, port, or scheme, including an HTTPS-to-HTTP
//! downgrade.
//!
//! # URL policy
//!
//! Only absolute `http` and `https` URLs with a host and no userinfo are
//! accepted. Userinfo is rejected on the initial URL and on every redirect
//! target before a request is made, including empty `@` forms and forms the
//! URL parser would otherwise normalize. neo-js never adds
//! credentials and never includes the URL in an error message.
//!
//! # Destination handling
//!
//! The body is streamed to a new private temporary directory in the
//! destination's parent directory, to a file neo-js opens there. On Unix the
//! directory is created with mode `0700` and the file with mode `0600` (both
//! subject to the process umask). The open file handle is retained so its
//! identity can be checked before publication and cleanup. The temporary
//! directory is also identity-checked before it is removed (device/inode on
//! Unix, a same-file handle on other platforms; Windows is unverified), and
//! removal is non-recursive. The file is synchronized with `sync_all` before
//! it is published. The parent directory
//! is not synchronized, so durability of the directory entry across a power
//! loss is filesystem-dependent. The destination is never written directly,
//! so a failed, truncated, or over-limit download leaves an existing
//! destination untouched on normal error paths, provided the directory and
//! callbacks are trusted. Temporary cleanup is best-effort; failure to clean
//! up can leave temporary output behind and does not replace the download error.
//!
//! With `overwrite: false` (the default) an existing destination is rejected
//! before the request, and publication uses a no-clobber hard link. That is
//! atomic on Unix; Windows behavior is unverified. Hard links stay on the same
//! filesystem because the temporary directory is in the destination
//! directory. A filesystem without hard-link support fails with `FETCH_IO` and
//! leaves the destination untouched. With `overwrite: true`, publication is a
//! rename that replaces the destination atomically on Unix; Windows behavior
//! is not verified. On Unix the destination receives the temporary file's
//! mode, so a new download is private (`0600` before umask) and replacing an
//! existing destination does not preserve its old permissions. Windows ACL
//! inheritance is unverified.
//!
//! The parent directory and its path must be trusted against modification by
//! other users and same-user processes, including progress callbacks. Even a
//! Unix `0700` temporary directory can be renamed or replaced by someone who
//! can modify its parent. Identity checks and hard-link, rename, and removal
//! operations are separate: on every platform substitution can race with
//! publication or cleanup, changing the destination or deleting a replacement.
//! Portable standard-library APIs do not exclude these races. An
//! abrupt process termination can leave a `.neo-download-*.tmp` directory
//! behind; it is never the destination and it never replaces one.
//!
//! # Not covered
//!
//! There is no authentication, cookie jar, proxy, decompression, resume, async
//! API, or content-type interpretation. Environment proxy discovery is
//! explicitly disabled. The URL is never logged or included in an error
//! message, and no credentials are added.

use std::{
    cell::RefCell,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::sync_channel,
    },
    thread,
    time::{Duration, Instant},
};

use rquickjs::{
    Ctx, Error as JsError, Function, Object, Persistent, Result as JsResult, Value,
    module::{Declarations, Exports, ModuleDef},
    qjs,
};
use url::{Position, Url};

/// Default maximum number of response-body bytes written to the destination.
pub(crate) const DEFAULT_MAX_BYTES: u64 = 64 * 1024 * 1024;
/// Largest accepted `maxBytes` option.
pub(crate) const MAX_MAX_BYTES: u64 = 512 * 1024 * 1024;
/// Default whole-operation timeout, including connection and body reads.
pub(crate) const DEFAULT_TIMEOUT_MS: u64 = 30_000;
/// Largest accepted `timeoutMs` option.
pub(crate) const MAX_TIMEOUT_MS: u64 = 600_000;
/// Redirects neo-js follows. A chain that reaches this limit fails with
/// `FETCH_REDIRECT`.
pub(crate) const MAX_REDIRECTS: u32 = 5;
/// Read buffer size. Also the largest progress interval.
const CHUNK: usize = 64 * 1024;
/// Bounded attempts to create a unique temporary file.
const TEMP_ATTEMPTS: u32 = 16;
/// A nonblocking limit prevents recursive progress callbacks from deadlocking
/// while bounding threads even when many runtimes download concurrently.
const MAX_DOWNLOAD_WORKERS: usize = 32;
static DOWNLOAD_WORKERS: AtomicUsize = AtomicUsize::new(0);

struct WorkerPermit<'a>(&'a AtomicUsize);

impl<'a> WorkerPermit<'a> {
    fn acquire(workers: &'a AtomicUsize, limit: usize) -> Option<Self> {
        let mut count = workers.load(Ordering::Acquire);
        loop {
            if count >= limit {
                return None;
            }
            match workers.compare_exchange_weak(
                count,
                count + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(Self(workers)),
                Err(current) => count = current,
            }
        }
    }
}

impl Drop for WorkerPermit<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ErrorCode {
    Permission,
    InvalidUrl,
    Option,
    Status,
    Redirect,
    Timeout,
    Network,
    Io,
    Limit,
    Truncated,
    Exists,
    Path,
    Cwd,
    Temp,
    Callback,
    CallbackAsync,
}

impl ErrorCode {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Permission => "FETCH_PERMISSION",
            Self::InvalidUrl => "FETCH_INVALID_URL",
            Self::Option => "FETCH_OPTION",
            Self::Status => "FETCH_STATUS",
            Self::Redirect => "FETCH_REDIRECT",
            Self::Timeout => "FETCH_TIMEOUT",
            Self::Network => "FETCH_NETWORK",
            Self::Io => "FETCH_IO",
            Self::Limit => "FETCH_LIMIT",
            Self::Truncated => "FETCH_TRUNCATED",
            Self::Exists => "FETCH_EXISTS",
            Self::Path => "FETCH_PATH",
            Self::Cwd => "FETCH_CWD",
            Self::Temp => "FETCH_TEMP",
            Self::Callback => "FETCH_CALLBACK",
            Self::CallbackAsync => "FETCH_CALLBACK_ASYNC",
        }
    }

    /// A fixed message. It never includes a URL, path, or server-provided text.
    pub(crate) const fn message(self) -> &'static str {
        match self {
            Self::Permission => "filesystem permission is required to download a file",
            Self::InvalidUrl => "the URL is not a valid http or https URL",
            Self::Option => "a download option is invalid",
            Self::Status => "the server returned an unsuccessful HTTP status",
            Self::Redirect => "too many HTTP redirects",
            Self::Timeout => "the network request timed out",
            Self::Network => "the network request failed",
            Self::Io => "the local file operation failed",
            Self::Limit => "the download exceeded the size limit",
            Self::Truncated => "the response body ended before its declared length",
            Self::Exists => "the destination already exists",
            Self::Path => "the destination path is invalid",
            Self::Cwd => "the runtime working directory is unavailable",
            Self::Temp => "the temporary download file was replaced",
            Self::Callback => "the progress callback threw",
            Self::CallbackAsync => "the progress callback must be synchronous",
        }
    }
}

/// A fixed, input-free failure. `status` is set only for `FETCH_STATUS`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FetchError {
    code: ErrorCode,
    status: Option<u16>,
}

impl FetchError {
    const fn new(code: ErrorCode) -> Self {
        Self { code, status: None }
    }

    const fn status(status: u16) -> Self {
        Self {
            code: ErrorCode::Status,
            status: Some(status),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Options {
    max_bytes: u64,
    timeout: Duration,
    overwrite: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DownloadResult {
    bytes: u64,
    status: u16,
}

/// The persistent factory for `neo:fetch`.
///
/// Created in [`crate::NeoJs::new`] before user code can replace prototypes.
/// `filesystem` is captured so the native call can reject before any I/O.
pub(crate) struct FetchState {
    pub(crate) factory: Option<Persistent<Function<'static>>>,
    pub(crate) filesystem: bool,
    pub(crate) cwd: Rc<RefCell<crate::cwd::WorkingDirectory>>,
}

impl FetchState {
    pub(crate) fn new(filesystem: bool, cwd: Rc<RefCell<crate::cwd::WorkingDirectory>>) -> Self {
        Self {
            factory: None,
            filesystem,
            cwd,
        }
    }
}

// FetchState contains no borrowed JS value; its persistent handle is released
// before the context and runtime are dropped.
unsafe impl<'js> rquickjs::JsLifetime<'js> for FetchState {
    type Changed<'to> = FetchState;
}

/// The native `neo:fetch` module. Its callbacks are private to the module and
/// are never placed on the global object.
pub(crate) struct FetchModule;

impl ModuleDef for FetchModule {
    fn declare<'js>(decl: &Declarations<'js>) -> JsResult<()> {
        decl.declare("download")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> JsResult<()> {
        let state = Rc::clone(
            &*ctx
                .userdata::<Rc<RefCell<FetchState>>>()
                .ok_or(JsError::Unknown)?,
        );
        let filesystem = state.borrow().filesystem;
        let cwd = Rc::clone(&state.borrow().cwd);
        let download = Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>,
                  url: String,
                  path: String,
                  options: Object<'js>,
                  progress: Function<'js>,
                  has_then: Function<'js>|
                  -> String {
                if !filesystem {
                    return error_json(FetchError::new(ErrorCode::Permission));
                }
                // Inspect the engine's function brand, not user-visible
                // constructor, prototype, or Symbol.toStringTag properties.
                // SAFETY: `progress` holds the live, borrowed QuickJS value.
                if unsafe { qjs::JS_IsAsyncFunction(progress.as_value().as_raw()) } {
                    return error_json(FetchError::new(ErrorCode::CallbackAsync));
                }
                let path = match cwd.borrow().resolve(&path) {
                    Ok(path) => path,
                    Err(_) => return error_json(FetchError::new(ErrorCode::Cwd)),
                };
                download_js(ctx, url, path, options, progress, has_then)
            },
        )?;
        let limits = Object::new_proto(ctx.clone(), None)?;
        limits.set("defaultMaxBytes", DEFAULT_MAX_BYTES)?;
        limits.set("maxMaxBytes", MAX_MAX_BYTES)?;
        limits.set("defaultTimeoutMs", DEFAULT_TIMEOUT_MS)?;
        limits.set("maxTimeoutMs", MAX_TIMEOUT_MS)?;
        let factory = state
            .borrow()
            .factory
            .as_ref()
            .cloned()
            .ok_or(JsError::Unknown)?
            .restore(ctx)?;
        let is_proxy = Function::new(ctx.clone(), |value: Value<'js>| value.is_proxy())?;
        let api: Object = factory.call((download, limits, is_proxy))?;
        exports.export("download", api.get::<_, Value>("download")?)?;
        Ok(())
    }
}

/// Build the private JavaScript wrapper in the fresh realm.
///
/// The wrapper captures the original constructors and helpers before user code
/// runs. It validates JavaScript types and option ranges, then calls the
/// private host callback. The host validates the same values again.
pub(crate) fn factory<'js>(ctx: &Ctx<'js>) -> JsResult<Function<'js>> {
    ctx.eval(
        r#"(function() {
            'use strict';
            const NativeError = Error;
            const NativeTypeError = TypeError;
            const defineProperty = Object.defineProperty;
            const hasOwn = Object.hasOwn;
            const getOwnPropertyDescriptor = Object.getOwnPropertyDescriptor;
            const getPrototypeOf = Object.getPrototypeOf;
            const parseJSON = JSON.parse;
            const isSafeInteger = Number.isSafeInteger;

            function unwrap(response) {
                if (response.ok) {
                    return { bytes: response.bytes, status: response.status };
                }
                const error = new NativeError(response.message);
                // Define own properties instead of assigning: an inherited
                // `Object.prototype.code` or `status` setter must not swallow
                // or replace the stable values.
                defineProperty(error, 'code', {
                    __proto__: null,
                    value: response.code,
                    writable: true,
                    enumerable: true,
                    configurable: true,
                });
                if (hasOwn(response, 'status')) {
                    defineProperty(error, 'status', {
                        __proto__: null,
                        value: response.status,
                        writable: true,
                        enumerable: true,
                        configurable: true,
                    });
                }
                throw error;
            }

            return function(downloadHost, limits, isProxy) {
                const defaultMaxBytes = limits.defaultMaxBytes;
                const maxMaxBytes = limits.maxMaxBytes;
                const defaultTimeoutMs = limits.defaultTimeoutMs;
                const maxTimeoutMs = limits.maxTimeoutMs;

                // Walk descriptors, never evaluating a `then` accessor.
                // A proxy is opaque: even descriptor/prototype inspection
                // could call a trap, so bail out before probing it.
                function hasThen(value) {
                    for (let cursor = value; cursor !== null; cursor = getPrototypeOf(cursor)) {
                        if (isProxy(cursor)) return true;
                        const desc = getOwnPropertyDescriptor(cursor, 'then');
                        if (desc !== undefined) {
                            return !hasOwn(desc, 'value') || typeof desc.value === 'function';
                        }
                    }
                    return false;
                }

                // Inherited properties are never configuration. Read an own
                // option once, so getters see the same value we validate.
                function option(options, key, fallback) {
                    if (!hasOwn(options, key)) return fallback;
                    const value = options[key];
                    return value === undefined ? fallback : value;
                }

                function download(url, path, options) {
                    if (typeof url !== 'string') throw new NativeTypeError('url must be a string');
                    if (typeof path !== 'string') throw new NativeTypeError('path must be a string');
                    if (options === undefined) {
                        options = { __proto__: null };
                    } else if (options === null || typeof options !== 'object') {
                        throw new NativeTypeError('options must be an object');
                    }

                    const maxBytes = option(options, 'maxBytes', defaultMaxBytes);
                    if (typeof maxBytes !== 'number' || !isSafeInteger(maxBytes) ||
                        maxBytes < 0 || maxBytes > maxMaxBytes) {
                        throw new NativeTypeError('maxBytes must be an integer from 0 through ' + maxMaxBytes);
                    }
                    const timeoutMs = option(options, 'timeoutMs', defaultTimeoutMs);
                    if (typeof timeoutMs !== 'number' || !isSafeInteger(timeoutMs) ||
                        timeoutMs < 1 || timeoutMs > maxTimeoutMs) {
                        throw new NativeTypeError('timeoutMs must be an integer from 1 through ' + maxTimeoutMs);
                    }
                    const overwrite = option(options, 'overwrite', false);
                    if (typeof overwrite !== 'boolean') {
                        throw new NativeTypeError('overwrite must be a boolean');
                    }
                    const progress = option(options, 'progress', function() {});
                    if (typeof progress !== 'function') {
                        throw new NativeTypeError('progress must be a function');
                    }

                    return unwrap(parseJSON(downloadHost(
                        url,
                        path,
                        { maxBytes, timeoutMs, overwrite },
                        progress,
                        hasThen,
                    )));
                }

                return { download };
            };
        })()"#,
    )
}

fn download_js<'js>(
    ctx: Ctx<'js>,
    url: String,
    path: std::path::PathBuf,
    options: Object<'js>,
    progress: Function<'js>,
    has_then: Function<'js>,
) -> String {
    let options = match options_from_js(&options) {
        Ok(options) => options,
        Err(error) => return error_json(error),
    };
    let mut report = |received: u64, total: Option<u64>| -> Result<(), FetchError> {
        match progress.call::<_, Value>((received, total)) {
            Ok(value) => check_progress_return(value, &has_then),
            Err(_) => {
                // Clear the callback exception before returning a normal value.
                let _ = ctx.catch();
                Err(FetchError::new(ErrorCode::Callback))
            }
        }
    };
    match download_on_worker(&url, &path, options, &mut report) {
        Ok(result) => success_json(result),
        Err(error) => error_json(error),
    }
}

/// Reject asynchronous results without awaiting, scheduling a reaction, or
/// running user code to read `then` or a Promise's constructor/species.
fn check_progress_return<'js>(
    value: Value<'js>,
    has_then: &Function<'js>,
) -> Result<(), FetchError> {
    if value.is_promise() {
        // Report the stable synchronous failure instead of leaving a rejected
        // (or later rejecting) Promise unhandled.
        // SAFETY: Both raw handles are live and the Promise belongs to this context.
        unsafe { qjs::JS_PromiseMarkAsHandled(value.ctx().as_raw().as_ptr(), value.as_raw()) };
        return Err(FetchError::new(ErrorCode::CallbackAsync));
    }
    if value.is_object() {
        match has_then.call::<_, bool>((&value,)) {
            Ok(false) => {}
            Ok(true) => return Err(FetchError::new(ErrorCode::CallbackAsync)),
            Err(_) => {
                let _ = value.ctx().catch();
                return Err(FetchError::new(ErrorCode::CallbackAsync));
            }
        }
    }
    Ok(())
}

/// Transport never runs on the JS/Tokio host thread. Only plain progress
/// numbers and fixed errors cross the rendezvous; the JS context stays here.
/// Every worker is joined before return, including callback failure and panic.
fn download_on_worker(
    url: &str,
    path: &Path,
    options: Options,
    progress: &mut dyn FnMut(u64, Option<u64>) -> Result<(), FetchError>,
) -> Result<DownloadResult, FetchError> {
    run_worker(progress, |report| download_core(url, path, options, report))
}

fn run_worker<F>(
    progress: &mut dyn FnMut(u64, Option<u64>) -> Result<(), FetchError>,
    task: F,
) -> Result<DownloadResult, FetchError>
where
    F: FnOnce(
            &mut dyn FnMut(u64, Option<u64>) -> Result<(), FetchError>,
        ) -> Result<DownloadResult, FetchError>
        + Send,
{
    let _permit = WorkerPermit::acquire(&DOWNLOAD_WORKERS, MAX_DOWNLOAD_WORKERS)
        .ok_or_else(|| FetchError::new(ErrorCode::Network))?;
    thread::scope(|scope| {
        let (events, incoming) = sync_channel::<(u64, Option<u64>)>(0);
        let (acknowledge, replies) = sync_channel::<Result<(), FetchError>>(0);
        let worker = thread::Builder::new()
            .name("neo-download".into())
            .spawn_scoped(scope, move || {
                let mut report = |received, total| {
                    events
                        .send((received, total))
                        .map_err(|_| FetchError::new(ErrorCode::Callback))?;
                    replies
                        .recv()
                        .unwrap_or_else(|_| Err(FetchError::new(ErrorCode::Callback)))
                };
                task(&mut report)
            })
            .map_err(|_| FetchError::new(ErrorCode::Network))?;
        while let Ok((received, total)) = incoming.recv() {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                progress(received, total)
            }))
            .unwrap_or_else(|_| Err(FetchError::new(ErrorCode::Callback)));
            if acknowledge.send(result).is_err() || result.is_err() {
                break;
            }
        }
        // Dropping both ends releases a worker waiting on either rendezvous.
        drop(incoming);
        drop(acknowledge);
        worker
            .join()
            .unwrap_or_else(|_| Err(FetchError::new(ErrorCode::Network)))
    })
}

fn options_from_js(options: &Object<'_>) -> Result<Options, FetchError> {
    let max_bytes = bounded_integer(
        options
            .get::<_, Option<f64>>("maxBytes")
            .map_err(|_| FetchError::new(ErrorCode::Option))?,
        DEFAULT_MAX_BYTES,
        0,
        MAX_MAX_BYTES,
    )?;
    let timeout_ms = bounded_integer(
        options
            .get::<_, Option<f64>>("timeoutMs")
            .map_err(|_| FetchError::new(ErrorCode::Option))?,
        DEFAULT_TIMEOUT_MS,
        1,
        MAX_TIMEOUT_MS,
    )?;
    let overwrite = options
        .get::<_, Option<bool>>("overwrite")
        .map_err(|_| FetchError::new(ErrorCode::Option))?
        .unwrap_or(false);
    Ok(Options {
        max_bytes,
        timeout: Duration::from_millis(timeout_ms),
        overwrite,
    })
}

fn bounded_integer(
    value: Option<f64>,
    default: u64,
    min: u64,
    max: u64,
) -> Result<u64, FetchError> {
    let Some(value) = value else {
        return Ok(default);
    };
    if !value.is_finite() || value.fract() != 0.0 || value < min as f64 || value > max as f64 {
        return Err(FetchError::new(ErrorCode::Option));
    }
    Ok(value as u64)
}

fn parse_initial_url(raw: &str) -> Result<Url, FetchError> {
    let url = Url::parse(raw).map_err(|_| FetchError::new(ErrorCode::InvalidUrl))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || has_userinfo(&url)
        || raw_has_userinfo(raw)
    {
        return Err(FetchError::new(ErrorCode::InvalidUrl));
    }
    Ok(url)
}

/// True when the parsed URL has a username or password component. The `url`
/// crate drops an empty `@` userinfo, so the raw checks below also cover that
/// edge case.
fn has_userinfo(url: &Url) -> bool {
    !url[Position::BeforeUsername..Position::BeforeHost].is_empty()
}

/// The authority of an absolute http/https URL, after any special-scheme slash
/// normalization, contains `@` before its path/query/fragment delimiter.
fn raw_has_userinfo(raw: &str) -> bool {
    let normalized = raw
        .trim_matches(|c: char| c <= '\u{20}')
        .replace(['\t', '\n', '\r'], "");
    let raw = normalized.as_str();
    let Some((scheme, rest)) = raw.split_once(':') else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return false;
    }
    raw_authority_has_userinfo(rest.trim_start_matches(['/', '\\']))
}

fn raw_reference_has_userinfo(reference: &str) -> bool {
    let normalized = reference
        .trim_matches(|c: char| c <= '\u{20}')
        .replace(['\t', '\n', '\r'], "");
    let reference = normalized.as_str();
    if raw_has_userinfo(reference) {
        return true;
    }
    let bytes = reference.as_bytes();
    if bytes.len() >= 2 && matches!(bytes[0], b'/' | b'\\') && matches!(bytes[1], b'/' | b'\\') {
        // Special-scheme relative parsing can consume *all* initial slashes
        // when they are mixed. Inspect after the entire run, even for `//`:
        // rejecting an ambiguous invalid reference is safer than losing `@`
        // when the URL parser normalizes its empty userinfo.
        return raw_authority_has_userinfo(reference.trim_start_matches(['/', '\\']));
    }
    false
}

fn raw_authority_has_userinfo(rest: &str) -> bool {
    let end = rest.find(['/', '\\', '?', '#']).unwrap_or(rest.len());
    rest[..end].contains('@')
}

fn check_deadline(deadline: Instant) -> Result<(), FetchError> {
    if Instant::now() >= deadline {
        Err(FetchError::new(ErrorCode::Timeout))
    } else {
        Ok(())
    }
}

fn download_core(
    url: &str,
    destination: &Path,
    options: Options,
    progress: &mut dyn FnMut(u64, Option<u64>) -> Result<(), FetchError>,
) -> Result<DownloadResult, FetchError> {
    let parsed = parse_initial_url(url)?;

    // Validate the destination and its parent before making a request. The
    // no-clobber publish later is authoritative; this check only fails fast.
    let parent = destination_parent(destination)?;
    match fs::symlink_metadata(destination) {
        Ok(metadata) => {
            if metadata.is_dir() {
                return Err(FetchError::new(ErrorCode::Path));
            }
            if !options.overwrite {
                return Err(FetchError::new(ErrorCode::Exists));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err(FetchError::new(ErrorCode::Io)),
    }

    // Redirects are followed here rather than by the client. This lets neo-js apply
    // the http/https rule to every hop and enforce the overall timeout across
    // the chain.
    let deadline = Instant::now() + options.timeout;
    let mut current = parsed;
    let mut redirects = 0u32;
    let (response, status) = loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| FetchError::new(ErrorCode::Timeout))?;
        let agent = reqwest::blocking::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(remaining)
            .timeout(remaining)
            .no_proxy()
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .no_zstd()
            .use_rustls_tls()
            .tls_built_in_native_certs(false)
            .tls_built_in_webpki_certs(true)
            .user_agent(concat!("neo-js/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| FetchError::new(ErrorCode::Network))?;
        let response = agent.get(current.as_str()).send().map_err(|error| {
            if error.is_timeout() {
                FetchError::new(ErrorCode::Timeout)
            } else {
                FetchError::new(ErrorCode::Network)
            }
        })?;
        let status = response.status().as_u16();
        if !(300..=399).contains(&status) {
            break (response, status);
        }
        if redirects >= MAX_REDIRECTS {
            return Err(FetchError::new(ErrorCode::Redirect));
        }
        let Some(location) = response
            .headers()
            .get("Location")
            .and_then(|v| v.to_str().ok())
        else {
            return Err(FetchError::status(status));
        };
        // Reject userinfo before the client can turn it into an Authorization
        // header. The raw check also covers an empty `@` userinfo, which the
        // URL parser otherwise normalizes away. HTTP header values may carry
        // optional whitespace.
        let location = location.trim();
        if raw_reference_has_userinfo(location) {
            return Err(FetchError::new(ErrorCode::InvalidUrl));
        }
        let next = current
            .join(location)
            .map_err(|_| FetchError::new(ErrorCode::InvalidUrl))?;
        if !matches!(next.scheme(), "http" | "https")
            || next.host_str().is_none()
            || has_userinfo(&next)
        {
            return Err(FetchError::new(ErrorCode::InvalidUrl));
        }
        current = next;
        redirects += 1;
    };
    if !(200..=299).contains(&status) {
        return Err(FetchError::status(status));
    }
    // RFC 9112 says Content-Length is ignored when Transfer-Encoding is
    // present. Do not let a contradictory header drive the bound or the
    // truncation check for a chunked response.
    let content_length = if response.headers().get("Transfer-Encoding").is_some() {
        None
    } else {
        response
            .headers()
            .get("Content-Length")
            .and_then(|value| value.to_str().ok()?.parse::<u64>().ok())
    };
    if content_length.is_some_and(|length| length > options.max_bytes) {
        return Err(FetchError::new(ErrorCode::Limit));
    }

    let mut temporary = create_temp_file(&parent)?;
    progress(0, content_length)?;
    // Time spent in the callback counts against the operation deadline.
    check_deadline(deadline)?;

    let mut reader = response;
    let mut buffer = [0u8; CHUNK];
    let mut received = 0u64;
    loop {
        let count = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) => {
                let mapped = map_read_error(&error);
                if mapped.code == ErrorCode::Network
                    && content_length.is_some_and(|length| received < length)
                {
                    return Err(FetchError::new(ErrorCode::Truncated));
                }
                return Err(mapped);
            }
        };
        received = received
            .checked_add(count as u64)
            .ok_or_else(|| FetchError::new(ErrorCode::Limit))?;
        if received > options.max_bytes {
            return Err(FetchError::new(ErrorCode::Limit));
        }
        temporary.write_all(&buffer[..count])?;
        progress(received, content_length)?;
        // Time spent in the callback counts even when the body was already
        // buffered and there is no further read to enforce the deadline.
        check_deadline(deadline)?;
    }
    if content_length.is_some_and(|length| length != received) {
        return Err(FetchError::new(ErrorCode::Truncated));
    }
    check_deadline(deadline)?;
    temporary.publish(destination, options.overwrite)?;

    Ok(DownloadResult {
        bytes: received,
        status,
    })
}

fn map_read_error(error: &io::Error) -> FetchError {
    use std::error::Error as _;
    let mut source = error.source();
    while let Some(cause) = source {
        if cause
            .downcast_ref::<reqwest::Error>()
            .is_some_and(reqwest::Error::is_timeout)
        {
            return FetchError::new(ErrorCode::Timeout);
        }
        source = cause.source();
    }
    match error.kind() {
        io::ErrorKind::UnexpectedEof => FetchError::new(ErrorCode::Truncated),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => FetchError::new(ErrorCode::Timeout),
        _ => FetchError::new(ErrorCode::Network),
    }
}

fn destination_parent(destination: &Path) -> Result<PathBuf, FetchError> {
    if destination.file_name().is_none() {
        return Err(FetchError::new(ErrorCode::Path));
    }
    let parent = match destination.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    match fs::metadata(parent) {
        Ok(metadata) if metadata.is_dir() => Ok(parent.to_path_buf()),
        Ok(_) => Err(FetchError::new(ErrorCode::Path)),
        Err(_) => Err(FetchError::new(ErrorCode::Path)),
    }
}

fn create_temp_file(parent: &Path) -> Result<TempFile, FetchError> {
    for _ in 0..TEMP_ATTEMPTS {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).map_err(|_| FetchError::new(ErrorCode::Io))?;
        let directory = parent.join(format!(".neo-download-{}.tmp", hex::encode(bytes)));
        match create_private_directory(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(FetchError::new(ErrorCode::Io)),
        }
        let Some(directory_id) = directory_identity(&directory) else {
            let _ = fs::remove_dir(&directory);
            return Err(FetchError::new(ErrorCode::Io));
        };
        let path = directory.join("download");
        match open_private_file(&path) {
            Ok(file) => {
                return Ok(TempFile {
                    directory,
                    directory_id: Some(directory_id),
                    path,
                    file,
                });
            }
            Err(_) => {
                let _ = fs::remove_dir(&directory);
                return Err(FetchError::new(ErrorCode::Io));
            }
        }
    }
    Err(FetchError::new(ErrorCode::Io))
}

#[cfg(unix)]
fn create_private_directory(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new().mode(0o700).create(path)
}

#[cfg(not(unix))]
fn create_private_directory(path: &Path) -> io::Result<()> {
    fs::create_dir(path)
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DirectoryIdentity {
    dev: u64,
    ino: u64,
}

#[cfg(unix)]
fn directory_identity(path: &Path) -> Option<DirectoryIdentity> {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_dir() {
        return None;
    }
    Some(DirectoryIdentity {
        dev: metadata.dev(),
        ino: metadata.ino(),
    })
}

#[cfg(unix)]
fn directory_is_ours(identity: &DirectoryIdentity, path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    metadata.is_dir() && metadata.dev() == identity.dev && metadata.ino() == identity.ino
}

#[cfg(not(unix))]
type DirectoryIdentity = same_file::Handle;

#[cfg(not(unix))]
fn directory_identity(path: &Path) -> Option<DirectoryIdentity> {
    if !fs::symlink_metadata(path)
        .map(|metadata| metadata.is_dir())
        .unwrap_or(false)
    {
        return None;
    }
    same_file::Handle::from_path(path).ok()
}

#[cfg(not(unix))]
fn directory_is_ours(identity: &DirectoryIdentity, path: &Path) -> bool {
    if !fs::symlink_metadata(path)
        .map(|metadata| metadata.is_dir())
        .unwrap_or(false)
    {
        return false;
    }
    same_file::Handle::from_path(path)
        .map(|current| &current == identity)
        .unwrap_or(false)
}

#[cfg(unix)]
fn open_private_file(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn open_private_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
}

/// Prove that `path` still names the exact file open as `file`. A regular file
/// that reuses the old pathname after our file is unlinked cannot compare
/// equal while the open handle keeps the old inode alive on Unix.
#[cfg(unix)]
fn path_is_open_file(file: &File, path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(current) = fs::symlink_metadata(path) else {
        return false;
    };
    if !current.file_type().is_file() {
        return false;
    }
    let Ok(opened) = file.metadata() else {
        return false;
    };
    opened.dev() == current.dev() && opened.ino() == current.ino()
}

#[cfg(not(unix))]
fn path_is_open_file(file: &File, path: &Path) -> bool {
    let Ok(current) = fs::symlink_metadata(path) else {
        return false;
    };
    if !current.file_type().is_file() {
        return false;
    }
    let Ok(opened) = file.try_clone() else {
        return false;
    };
    let Ok(opened) = same_file::Handle::from_file(opened) else {
        return false;
    };
    let Ok(current) = same_file::Handle::from_path(path) else {
        return false;
    };
    opened == current
}

/// A destination-local private temporary directory and the file neo-js opened
/// inside it. Cleanup only removes the file while its path still names the open
/// file, and only removes the directory when it is empty.
struct TempFile {
    directory: PathBuf,
    // `None` only during drop, after the identity handle is released.
    directory_id: Option<DirectoryIdentity>,
    path: PathBuf,
    file: File,
}

impl TempFile {
    fn write_all(&mut self, bytes: &[u8]) -> Result<(), FetchError> {
        self.file
            .write_all(bytes)
            .map_err(|_| FetchError::new(ErrorCode::Io))
    }

    fn publish(self, destination: &Path, overwrite: bool) -> Result<(), FetchError> {
        self.file
            .sync_all()
            .map_err(|_| FetchError::new(ErrorCode::Io))?;
        if !path_is_open_file(&self.file, &self.path) {
            return Err(FetchError::new(ErrorCode::Temp));
        }
        if overwrite {
            fs::rename(&self.path, destination).map_err(|_| FetchError::new(ErrorCode::Io))?;
            if !path_is_open_file(&self.file, destination) {
                return Err(FetchError::new(ErrorCode::Temp));
            }
        } else {
            match fs::hard_link(&self.path, destination) {
                Ok(()) => {
                    // The destination must be the open file, not a pathname
                    // that a same-user process substituted at the last moment.
                    if !path_is_open_file(&self.file, destination) {
                        return Err(FetchError::new(ErrorCode::Temp));
                    }
                    // Remove our temporary name only while it is still ours.
                    if path_is_open_file(&self.file, &self.path) {
                        let _ = fs::remove_file(&self.path);
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    return Err(FetchError::new(ErrorCode::Exists));
                }
                Err(_) => return Err(FetchError::new(ErrorCode::Io)),
            }
        }
        Ok(())
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        // Never delete a pathname that no longer names the file we opened.
        if path_is_open_file(&self.file, &self.path) {
            let _ = fs::remove_file(&self.path);
        }
        // Never recurse, follow, or remove a replacement directory. Drop the
        // stored identity handle first; on Windows an open directory handle
        // can prevent removal.
        let ours = self
            .directory_id
            .as_ref()
            .is_some_and(|identity| directory_is_ours(identity, &self.directory));
        self.directory_id.take();
        if ours {
            let _ = fs::remove_dir(&self.directory);
        }
    }
}

fn success_json(result: DownloadResult) -> String {
    serde_json::json!({
        "ok": true,
        "bytes": result.bytes,
        "status": result.status,
    })
    .to_string()
}

fn error_json(error: FetchError) -> String {
    let mut value = serde_json::json!({
        "ok": false,
        "code": error.code.as_str(),
        "message": error.code.message(),
    });
    if let Some(status) = error.status {
        value["status"] = serde_json::Value::from(status);
    }
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_capacity_is_nonblocking_and_released() {
        let count = AtomicUsize::new(0);
        let permits: Vec<_> = (0..MAX_DOWNLOAD_WORKERS)
            .map(|_| WorkerPermit::acquire(&count, MAX_DOWNLOAD_WORKERS).unwrap())
            .collect();
        assert!(WorkerPermit::acquire(&count, MAX_DOWNLOAD_WORKERS).is_none());
        drop(permits);
        assert_eq!(count.load(Ordering::Acquire), 0);
        assert!(WorkerPermit::acquire(&count, MAX_DOWNLOAD_WORKERS).is_some());
    }

    #[test]
    fn worker_panics_and_callback_panics_are_joined_and_reported() {
        use std::sync::atomic::AtomicBool;
        let dropped = AtomicBool::new(false);
        struct MarkDrop<'a>(&'a AtomicBool);
        impl Drop for MarkDrop<'_> {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let error = run_worker(&mut |_, _| Ok(()), |_| {
            let _guard = MarkDrop(&dropped);
            panic!("injected worker panic");
        })
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::Network);
        assert!(dropped.load(Ordering::Acquire));

        dropped.store(false, Ordering::Release);
        let error = run_worker(&mut |_, _| panic!("injected callback panic"), |report| {
            let _guard = MarkDrop(&dropped);
            report(0, None)?;
            unreachable!("callback panic must cancel worker");
        })
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::Callback);
        assert!(dropped.load(Ordering::Acquire));
    }

    #[test]
    fn worker_progress_runs_only_on_the_calling_thread() {
        let caller = thread::current().id();
        let mut calls = 0;
        let result = run_worker(
            &mut |_, _| {
                assert_eq!(thread::current().id(), caller);
                calls += 1;
                Ok(())
            },
            |report| {
                assert_ne!(thread::current().id(), caller);
                report(0, Some(3))?;
                report(3, Some(3))?;
                Ok(DownloadResult {
                    bytes: 3,
                    status: 200,
                })
            },
        )
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(result.bytes, 3);
    }

    #[test]
    fn rejected_progress_promises_are_marked_handled_without_draining_jobs() {
        // The runtime does not expose a rejection tracker to scripts. Inspect
        // the engine notification directly, including a later rejection.
        use std::cell::Cell;

        let unhandled = Rc::new(Cell::new(0));
        let tracked = Rc::clone(&unhandled);
        let runtime = rquickjs::Runtime::new().unwrap();
        runtime.set_host_promise_rejection_tracker(Some(Box::new(move |_, _, _, handled| {
            tracked.set(tracked.get() + if handled { -1 } else { 1 });
        })));
        let context = rquickjs::Context::full(&runtime).unwrap();
        context.with(|ctx| {
            let has_then = ctx.eval::<Function, _>("() => { throw Error('unexpected lookup'); }").unwrap();
            let rejected = ctx.eval::<Value, _>("Promise.reject('already rejected')").unwrap();
            assert_eq!(unhandled.get(), 1);
            assert_eq!(
                check_progress_return(rejected, &has_then).unwrap_err().code,
                ErrorCode::CallbackAsync
            );
            assert_eq!(unhandled.get(), 0);
            let pending = ctx
                .eval::<Value, _>("Promise.resolve().then(() => { globalThis.ran = true; throw Error('later'); })")
                .unwrap();
            assert_eq!(
                check_progress_return(pending, &has_then).unwrap_err().code,
                ErrorCode::CallbackAsync
            );
            assert!(!ctx.eval::<bool, _>("globalThis.ran === true").unwrap());
            assert!(ctx.execute_pending_job());
            assert!(ctx.eval::<bool, _>("globalThis.ran === true").unwrap());
            assert_eq!(unhandled.get(), 0);
        });
    }

    #[test]
    fn initial_urls_require_http_https_host_and_no_userinfo() {
        for url in [
            "",
            "not a url",
            "file:///etc/passwd",
            "ftp://example.test/file",
            "javascript:alert(1)",
            "http://",
            "https://",
            "http://user:pass@example.test/x",
            "http://user@example.test/x",
            "http://@example.test/x",
            " http://@example.test/x",
            "\u{1}http://@example.test/x",
            "ht\ttp://@example.test/x",
            "http://:pass@example.test/x",
            "http://user:@example.test/x",
            "http:/@example.test/x",
            "http:@example.test/x",
            "http:///@example.test/x",
            "http://user%3Apass@example.test/x",
        ] {
            assert_eq!(
                parse_initial_url(url).unwrap_err().code,
                ErrorCode::InvalidUrl,
                "{url}"
            );
        }
        for url in [
            "http://example.test/x",
            "https://example.test/x",
            "http://example.test/path@x",
            "http://example.test?x=@",
        ] {
            assert!(parse_initial_url(url).is_ok(), "{url}");
        }
    }

    #[test]
    fn redirect_references_with_userinfo_are_rejected_before_joining() {
        for reference in [
            "http://user:pass@example.test/x",
            "http://@example.test/x",
            " http://@example.test/x",
            "\u{1}http://@example.test/x",
            "ht\ttp://@example.test/x",
            " \t//@example.test/x",
            "/\\@example.test/x",
            "\\/@example.test/x",
            "/\\/@example.test/x",
            "\\//@example.test/x",
            "/\\\\/@example.test/x",
            "\\/\\/@example.test/x",
            "\u{1} \t/\\/@example.test/x",
            "/\t\\/@example.test/x",
            "http:/@example.test/x",
            "//user:pass@example.test/x",
            "//@example.test/x",
            "\\\\user@example.test/x",
        ] {
            assert!(raw_reference_has_userinfo(reference), "{reference}");
        }
        for reference in [
            "/path@x",
            "path@x",
            "http://example.test/path@x",
            "//example.test/path@x",
            "/\\/example.test/path@x",
            "?x=@",
        ] {
            assert!(!raw_reference_has_userinfo(reference), "{reference}");
        }
    }

    #[test]
    fn every_slash_prefix_normalized_to_empty_userinfo_is_rejected() {
        let base = Url::parse("http://origin.test/start").unwrap();
        for length in 2..=6 {
            for bits in 0..(1 << length) {
                let prefix: String = (0..length)
                    .map(|i| if bits & (1 << i) == 0 { '/' } else { '\\' })
                    .collect();
                for layout in ["", " \u{1}", "\t"] {
                    let reference = format!("{layout}{prefix}@target.test/path");
                    if base
                        .join(&reference)
                        .is_ok_and(|url| url.host_str() == Some("target.test"))
                    {
                        assert!(
                            raw_reference_has_userinfo(&reference),
                            "{reference:?} was normalized to an authority"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn options_are_bounded_and_integral() {
        assert_eq!(
            bounded_integer(Some(0.0), DEFAULT_MAX_BYTES, 0, MAX_MAX_BYTES).unwrap(),
            0
        );
        assert_eq!(
            bounded_integer(None, DEFAULT_MAX_BYTES, 0, MAX_MAX_BYTES).unwrap(),
            DEFAULT_MAX_BYTES
        );
        for value in [
            -1.0,
            0.5,
            f64::NAN,
            f64::INFINITY,
            (MAX_MAX_BYTES as f64) + 1.0,
        ] {
            assert_eq!(
                bounded_integer(Some(value), DEFAULT_MAX_BYTES, 0, MAX_MAX_BYTES)
                    .unwrap_err()
                    .code,
                ErrorCode::Option
            );
        }
        assert_eq!(
            bounded_integer(Some(1.0), DEFAULT_TIMEOUT_MS, 1, MAX_TIMEOUT_MS).unwrap(),
            1
        );
        assert_eq!(
            bounded_integer(Some(0.0), DEFAULT_TIMEOUT_MS, 1, MAX_TIMEOUT_MS)
                .unwrap_err()
                .code,
            ErrorCode::Option
        );
    }

    #[test]
    fn destination_paths_need_a_parent_directory_and_file_name() {
        assert_eq!(
            destination_parent(Path::new("/")).unwrap_err().code,
            ErrorCode::Path
        );
        assert_eq!(
            destination_parent(Path::new("..")).unwrap_err().code,
            ErrorCode::Path
        );
        assert!(destination_parent(Path::new(".")).is_err());
        assert!(destination_parent(Path::new("/nonexistent-neo-js-dir/file")).is_err());
        assert_eq!(
            destination_parent(Path::new("file")).unwrap(),
            Path::new(".")
        );
    }

    #[test]
    fn every_error_code_has_a_fixed_message() {
        for code in [
            ErrorCode::Permission,
            ErrorCode::InvalidUrl,
            ErrorCode::Option,
            ErrorCode::Status,
            ErrorCode::Redirect,
            ErrorCode::Timeout,
            ErrorCode::Network,
            ErrorCode::Io,
            ErrorCode::Limit,
            ErrorCode::Truncated,
            ErrorCode::Exists,
            ErrorCode::Path,
            ErrorCode::Cwd,
            ErrorCode::Temp,
            ErrorCode::Callback,
            ErrorCode::CallbackAsync,
        ] {
            assert!(!code.as_str().is_empty());
            assert!(!code.message().is_empty());
            assert!(!code.message().contains("http://"));
            assert!(!code.message().contains("https://"));
        }
    }

    #[test]
    fn deadline_checks_reject_elapsed_time() {
        assert!(check_deadline(Instant::now() + Duration::from_secs(60)).is_ok());
        assert_eq!(
            check_deadline(Instant::now()).unwrap_err().code,
            ErrorCode::Timeout
        );
    }

    #[test]
    fn temporary_file_publishes_and_cleans_up() {
        let parent = tempfile::tempdir().unwrap();
        let destination = parent.path().join("destination");
        let mut temporary = create_temp_file(parent.path()).unwrap();
        let directory = temporary.directory.clone();
        temporary.write_all(b"downloaded").unwrap();
        temporary.publish(&destination, false).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"downloaded");
        assert!(!directory.exists());
        assert_no_temp_entries(parent.path());
    }

    #[cfg(unix)]
    #[test]
    fn temporary_directory_and_file_are_private_on_unix() {
        use std::os::unix::fs::PermissionsExt;
        let parent = tempfile::tempdir().unwrap();
        let temporary = create_temp_file(parent.path()).unwrap();
        let directory_mode = fs::metadata(&temporary.directory)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        let file_mode = fs::metadata(&temporary.path).unwrap().permissions().mode() & 0o777;
        assert_eq!(directory_mode, 0o700);
        assert_eq!(file_mode & 0o077, 0);
        assert_eq!(file_mode & 0o600, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn substituted_temporary_file_is_not_published_or_deleted() {
        for overwrite in [false, true] {
            let parent = tempfile::tempdir().unwrap();
            let destination = parent.path().join("destination");
            if overwrite {
                fs::write(&destination, b"original").unwrap();
            }
            let mut temporary = create_temp_file(parent.path()).unwrap();
            temporary.write_all(b"ours").unwrap();
            let temp_path = temporary.path.clone();
            let temp_directory = temporary.directory.clone();
            fs::remove_file(&temp_path).unwrap();
            fs::write(&temp_path, b"attacker").unwrap();

            let error = temporary.publish(&destination, overwrite).unwrap_err();
            assert_eq!(error.code, ErrorCode::Temp);
            if overwrite {
                assert_eq!(fs::read(&destination).unwrap(), b"original");
            } else {
                assert!(!destination.exists());
            }
            assert_eq!(fs::read(&temp_path).unwrap(), b"attacker");
            assert!(temp_directory.exists());
        }
    }

    #[cfg(unix)]
    #[test]
    fn substituted_temporary_directory_is_not_deleted() {
        let parent = tempfile::tempdir().unwrap();
        let temporary = create_temp_file(parent.path()).unwrap();
        let directory = temporary.directory.clone();
        fs::remove_file(&temporary.path).unwrap();
        fs::remove_dir(&directory).unwrap();
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("keep"), b"attacker").unwrap();

        drop(temporary);
        assert_eq!(fs::read(directory.join("keep")).unwrap(), b"attacker");
    }

    fn assert_no_temp_entries(directory: &Path) {
        for entry in fs::read_dir(directory).unwrap() {
            let name = entry.unwrap().file_name();
            assert!(
                !name.to_string_lossy().starts_with(".neo-download-"),
                "temporary entry left behind: {name:?}"
            );
        }
    }
}
