//! Bounded filesystem access for `neo:fs`.
//!
//! The module is available only when [`crate::Permissions::filesystem`] is
//! granted and the runtime's module allowlist admits [`crate::RuntimeModule::Fs`].
//! Both gates are checked before the `fs` global or any native callback is
//! constructed. Only `readFile` runs on workers; `readFileSync` and all
//! other operations block. Nothing here starts a shell, a child process, or
//! a network request directly. The OS may still perform network I/O for
//! network paths and filesystems.
//!
//! # Authority
//!
//! This is not a sandbox. A script with this module can read, create, truncate,
//! rename, and delete anything the host user can, subject only to the operating
//! system's own access checks. The permission is therefore equivalent to
//! granting the script the host user's filesystem authority. The module does
//! not add path confinement, a virtual root, or a copy-on-write layer.
//!
//! # Working directory
//!
//! Relative paths use this runtime's [`WorkingDirectory`], not the host
//! process's mutable working directory. An absolute path is used unchanged. If
//! the runtime directory is unavailable, a relative operation reports
//! `FS_CWD` and never falls back to the host directory. The working directory
//! is coordination, not confinement: absolute paths and `..` components still
//! reach the whole filesystem the host user can reach.
//!
//! # Files and sizes
//!
//! `readFile`, `readText`, `writeFile`, and `writeText` require a regular file.
//! An open may occur before a non-regular object is rejected. If the type is
//! identified, the error is `FS_IS_DIRECTORY` or `FS_TYPE`; if the open fails
//! first, the OS error may take precedence. Unix data opens avoid waiting for
//! FIFO peers and do not acquire a controlling terminal. Other device-open
//! side effects and latency are still possible. `stat` and `lstat` inspect any
//! path the OS exposes.
//!
//! Reads and writes are bounded before an unbounded allocation:
//!
//! * A path may be at most [`MAX_PATH_BYTES`] UTF-8 bytes.
//! * A read may be at most [`MAX_READ_BYTES`] bytes. The file's metadata is
//!   checked before allocation, and the actual read is capped at one byte past
//!   the limit so a growing file is still rejected.
//! * A binary write may be at most [`MAX_WRITE_BYTES`] bytes. The
//!   current `Uint8Array` view is checked before it is copied. Detached and
//!   out-of-bounds views are rejected, as are views over `SharedArrayBuffer`.
//! * A text write may be at most [`MAX_WRITE_BYTES`] UTF-8 bytes. The UTF-16
//!   export itself can allocate up to 128 MiB of string data. QuickJS can
//!   also flatten a narrow string rope, adding up to 64 MiB that can coexist
//!   with the export (192 MiB of additional engine string data in that case).
//!   This is not a total memory bound: it excludes the original input,
//!   overhead, and the separately capped Rust UTF-8 snapshot allocated after
//!   validation.
//!
//! File sizes in [`FileInfo`] are decimal strings, never JavaScript numbers.
//! A `u64` byte size above `Number.MAX_SAFE_INTEGER` therefore keeps every
//! digit instead of being silently rounded. Times are whole milliseconds since
//! the Unix epoch, also as decimal strings, or `null` when the host cannot
//! report the modification time. A pre-epoch time is negative; a sub-millisecond
//! pre-epoch time is reported as `0`.
//!
//! # Symlinks
//!
//! The behavior is deliberately explicit:
//!
//! * `stat`, `readFile`, `readText`, `writeFile`, `writeText`, `exists`, and
//!   `readDir` follow a symlink at the final path component. `readDir` follows
//!   a symlink to a directory and reports the entries inside it.
//! * `lstat` does not follow the final component and reports `isSymlink: true`.
//! * `readDir` does not follow entries; a symlink entry is reported with
//!   `isSymlink: true` and `isFile`/`isDirectory` both `false`.
//! * `remove` does not follow a symlink at the target path. It removes the
//!   link itself, except a Windows directory symlink at the target path is
//!   rejected with `FS_UNSUPPORTED`, even if dangling. Recursive deletion
//!   uses the OS-backed `remove_dir_all`, which does not follow symlinks in
//!   the tree.
//! * `rename` acts on the final directory entries, including symlinks
//!   themselves, without canonicalizing either path on disk. Joining a
//!   relative path to a Windows verbatim cwd can normalize intermediate
//!   components lexically.
//! * `mkdir` follows the operating system's rules for existing path
//!   components; it does not add its own symlink policy.
//!
//! A dangling symlink is reported by `lstat` but `exists` follows the link and
//! returns `false`, because there is no target. `writeFile` with the default
//! `overwrite: true` follows a dangling symlink and creates its target;
//! `overwrite: false` refuses the path if the link itself exists. Windows
//! reparse-point and junction behavior is not verified by this build.
//!
//! # Listing
//!
//! `readDir` is non-recursive. It returns entries sorted by name in UTF-8 byte
//! order, which is also Unicode code-point order. The listing is bounded by
//! [`MAX_LIST_ENTRIES`], [`MAX_LIST_NAME_BYTES`] per entry name, and
//! [`MAX_LIST_TOTAL_BYTES`] for the sum of names. A name that is not valid
//! UTF-8 reports `FS_ENCODING`; the module never substitutes a lossy name.
//!
//! # Deletion
//!
//! `remove(path)` removes a file, symlink, or empty directory. A non-empty
//! directory reports `FS_NOT_EMPTY`. `remove(path, {recursive: true})` removes
//! a tree, including files, directories, and symlinks as links. Deletion has no
//! undo, no trash, and no count limit; the caller is responsible for choosing
//! the path. Parsed roots (including Windows UNC share roots), trailing
//! separators, and final `.` or `..` are rejected with `FS_PATH` before I/O.
//!
//! # Rename
//!
//! `rename(source, destination)` validates both entry paths, resolves them
//! against the same runtime working directory, and delegates replacement to
//! the OS. A rejected replacement of a non-empty directory can report
//! `FS_ALREADY_EXISTS` or `FS_NOT_EMPTY`, depending on the OS error. It
//! never pre-deletes the destination or copies and deletes across filesystems.
//! An OS cross-device error reports `FS_CROSS_DEVICE`.
//! It is pathname based: entries can change between resolution and use, and
//! the runtime does not roll back or guarantee the outcome after an OS or
//! network filesystem failure.
//!
//! # Copy
//!
//! `copyFile(source, destination)` streams up to [`MAX_READ_BYTES`] from a
//! regular file into an exclusively created regular file. It follows a final
//! source symlink but never opens an existing destination, including aliases
//! and dangling links. Both paths use one runtime cwd snapshot. It copies
//! bytes only and can copy between filesystems. It uses no temporary path,
//! lock, or rollback: after creation, an error can leave a partial destination.
//! Namespace changes and concurrent source modifications are not prevented.
//!
//! # Errors and races
//!
//! Filesystem failures are `FsError` with a stable `code` and a fixed message.
//! No such message contains the supplied path, an OS error string, or file
//! contents. Argument type errors are `TypeError`, not `FsError`; exceptions
//! raised by option proxy traps can also propagate.
//!
//! The checks are advisory snapshots, not a security boundary:
//!
//! * `exists` and `stat` are time-of-check/time-of-use race prone. Do not use
//!   them to decide whether a later operation is safe. Perform the operation
//!   and handle the error instead.
//! * A path can be replaced between resolution and open or delete. Unix data
//!   opens use nonblocking mode and `O_NOCTTY`. The opened descriptor is
//!   inspected before truncation, but path-based deletion and OS/network
//!   filesystem behavior are not race-free.
//! * `writeFile` and `writeText` truncate before writing and are not atomic.
//!   A failure can leave a truncated or partially written file. There is no
//!   temporary file and no rename step.
//! * `readDir` is not an atomic snapshot. Recursive deletion can remove some
//!   entries before an OS error stops it; there is no rollback.
//! * `FS_LIMIT` also reports quota, storage-full, and fallible allocation
//!   failures. This is best effort: allocator/runtime failures outside the
//!   explicitly fallible reservations may still abort or raise an engine error.
//! * No file locking is performed.
//!
//! # Windows
//!
//! Windows behavior has not been verified by running on Windows. Drive-relative
//! paths are rejected by the shared cwd resolver. Directory symlinks named
//! directly as removal targets are rejected with `FS_UNSUPPORTED`. Other
//! symlink, junction, verbatim-path, `FILE_ATTRIBUTE_*`, and
//! `DirectoryNotEmpty` details are operating-system dependent.

mod read_async;
mod worker;
pub(crate) use read_async::{ReadDriver, ReadTurn};

use std::{
    cell::RefCell,
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    ptr::NonNull,
    rc::Rc,
    time::{SystemTime, UNIX_EPOCH},
};

use rquickjs::{
    Ctx, Error as JsError, Function, Object, Persistent, Result as JsResult, String as JsString,
    TypedArray, Value,
    module::{Declarations, Exports, ModuleDef},
    qjs,
};

use crate::{cwd::WorkingDirectory, js_text::string_length};

/// Largest accepted path, in UTF-8 bytes.
pub(crate) const MAX_PATH_BYTES: usize = 32 * 1024;
/// Largest accepted read, in bytes.
pub(crate) const MAX_READ_BYTES: usize = 64 * 1024 * 1024;
/// Largest accepted write, in bytes.
pub(crate) const MAX_WRITE_BYTES: usize = 64 * 1024 * 1024;
/// Largest number of entries `readDir` returns.
pub(crate) const MAX_LIST_ENTRIES: usize = 100_000;
/// Largest accepted single directory-entry name, in UTF-8 bytes.
pub(crate) const MAX_LIST_NAME_BYTES: usize = 1024;
/// Largest sum of directory-entry names `readDir` returns, in UTF-8 bytes.
pub(crate) const MAX_LIST_TOTAL_BYTES: usize = 16 * 1024 * 1024;

const EXPORTS: [&str; 20] = [
    "readText",
    "writeText",
    "readFile",
    "readFileSync",
    "writeFile",
    "stat",
    "lstat",
    "exists",
    "readDir",
    "mkdir",
    "remove",
    "rename",
    "copyFile",
    "FsError",
    "MAX_PATH_BYTES",
    "MAX_READ_BYTES",
    "MAX_WRITE_BYTES",
    "MAX_LIST_ENTRIES",
    "MAX_LIST_NAME_BYTES",
    "MAX_LIST_TOTAL_BYTES",
];

/// A stable filesystem failure code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Code {
    Path,
    NotFound,
    NotDirectory,
    IsDirectory,
    AlreadyExists,
    NotEmpty,
    CrossDevice,
    Denied,
    Limit,
    Encoding,
    Detached,
    OutOfBounds,
    Cwd,
    NotRegular,
    Io,
    Unsupported,
    AsyncOutsideTurn,
    Cancelled,
}

impl Code {
    const ALL: [Self; 18] = [
        Self::Path,
        Self::NotFound,
        Self::NotDirectory,
        Self::IsDirectory,
        Self::AlreadyExists,
        Self::NotEmpty,
        Self::CrossDevice,
        Self::Denied,
        Self::Limit,
        Self::Encoding,
        Self::Detached,
        Self::OutOfBounds,
        Self::Cwd,
        Self::NotRegular,
        Self::Io,
        Self::Unsupported,
        Self::AsyncOutsideTurn,
        Self::Cancelled,
    ];

    const fn as_str(self) -> &'static str {
        match self {
            Self::Path => "FS_PATH",
            Self::NotFound => "FS_NOT_FOUND",
            Self::NotDirectory => "FS_NOT_DIRECTORY",
            Self::IsDirectory => "FS_IS_DIRECTORY",
            Self::AlreadyExists => "FS_ALREADY_EXISTS",
            Self::NotEmpty => "FS_NOT_EMPTY",
            Self::CrossDevice => "FS_CROSS_DEVICE",
            Self::Denied => "FS_DENIED",
            Self::Limit => "FS_LIMIT",
            Self::Encoding => "FS_ENCODING",
            Self::Detached => "FS_DETACHED",
            Self::OutOfBounds => "FS_OUT_OF_BOUNDS",
            Self::Cwd => "FS_CWD",
            Self::NotRegular => "FS_TYPE",
            Self::Io => "FS_IO",
            Self::Unsupported => "FS_UNSUPPORTED",
            Self::AsyncOutsideTurn => "FS_ASYNC_OUTSIDE_TURN",
            Self::Cancelled => "FS_CANCELLED",
        }
    }

    /// Fixed text. This must never include a supplied path or an OS error string.
    const fn message(self) -> &'static str {
        match self {
            Self::Path => "invalid filesystem path",
            Self::NotFound => "filesystem path does not exist",
            Self::NotDirectory => "a path component is not a directory",
            Self::IsDirectory => "the path is a directory",
            Self::AlreadyExists => "the path already exists",
            Self::NotEmpty => "the directory is not empty",
            Self::CrossDevice => "cannot rename across filesystems",
            Self::Denied => "filesystem access is denied",
            Self::Limit => "filesystem limit or resource budget exceeded",
            Self::Encoding => "filesystem data is not valid UTF-8",
            Self::Detached => "the Uint8Array buffer is detached",
            Self::OutOfBounds => "the Uint8Array view is out of bounds",
            // This text is part of the existing runtime contract for the
            // working directory and is asserted by the crate tests.
            Self::Cwd => "the runtime working directory is unavailable",
            Self::NotRegular => "the path is not a regular file",
            Self::Io => "filesystem operation failed",
            Self::Unsupported => "this filesystem operation or input is unsupported",
            Self::AsyncOutsideTurn => {
                "readFile requires managed module evaluation; use readFileSync here"
            }
            Self::Cancelled => "filesystem request was cancelled after module evaluation failed",
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct FsError {
    code: Code,
}

impl FsError {
    const fn new(code: Code) -> Self {
        Self { code }
    }

    fn code(self) -> &'static str {
        self.code.as_str()
    }

    fn message(self) -> &'static str {
        self.code.message()
    }
}

impl From<io::Error> for FsError {
    fn from(error: io::Error) -> Self {
        Self::new(map_io_error(error.kind()))
    }
}

fn map_io_error(kind: io::ErrorKind) -> Code {
    match kind {
        io::ErrorKind::NotFound => Code::NotFound,
        io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem => Code::Denied,
        io::ErrorKind::AlreadyExists => Code::AlreadyExists,
        io::ErrorKind::InvalidInput | io::ErrorKind::InvalidFilename => Code::Path,
        io::ErrorKind::InvalidData => Code::Encoding,
        io::ErrorKind::IsADirectory => Code::IsDirectory,
        io::ErrorKind::NotADirectory => Code::NotDirectory,
        io::ErrorKind::DirectoryNotEmpty => Code::NotEmpty,
        io::ErrorKind::CrossesDevices => Code::CrossDevice,
        io::ErrorKind::StorageFull
        | io::ErrorKind::QuotaExceeded
        | io::ErrorKind::FileTooLarge
        | io::ErrorKind::OutOfMemory => Code::Limit,
        _ => Code::Io,
    }
}

fn validate_path(path: &str) -> Result<&Path, FsError> {
    if path.is_empty() || path.as_bytes().contains(&0) {
        return Err(FsError::new(Code::Path));
    }
    if path.len() > MAX_PATH_BYTES {
        return Err(FsError::new(Code::Limit));
    }
    Ok(Path::new(path))
}

fn resolve(cwd: &Rc<RefCell<WorkingDirectory>>, path: &str) -> Result<PathBuf, FsError> {
    let path = validate_path(path)?;
    resolve_path(&cwd.borrow(), path)
}

fn resolve_path(cwd: &WorkingDirectory, path: &Path) -> Result<PathBuf, FsError> {
    cwd.resolve_path(path).map_err(|error| {
        FsError::new(if error.code == "PROCESS_PATH" {
            Code::Path
        } else {
            Code::Cwd
        })
    })
}

fn resolve_text(cwd: &Rc<RefCell<WorkingDirectory>>, path: &str) -> Result<String, FsError> {
    path_into_text(resolve(cwd, path)?)
}

fn path_into_text(path: PathBuf) -> Result<String, FsError> {
    path.into_os_string()
        .into_string()
        .map_err(|_| FsError::new(Code::Encoding))
}

/// Require a specific directory entry before resolving against cwd.
///
/// Parsed roots include Windows UNC shares without a trailing separator.
/// `.` and `..` as the final component are also rejected, so
/// `link/`, `link/.`, and `link/..` cannot make the OS follow a final symlink
/// while interpreting a deletion or rename path.
fn validate_entry_path(path: &str) -> Result<&Path, FsError> {
    let parsed = validate_path(path)?;
    #[cfg(windows)]
    if crate::cwd::is_drive_relative(parsed) {
        return Err(FsError::new(Code::Path));
    }
    if parsed.has_root() && parsed.parent().is_none() {
        return Err(FsError::new(Code::Path));
    }
    let last = *path.as_bytes().last().ok_or(FsError::new(Code::Path))?;
    #[cfg(windows)]
    let separator = last == b'/' || last == b'\\';
    #[cfg(not(windows))]
    let separator = last == b'/';
    if separator {
        return Err(FsError::new(Code::Path));
    }

    #[cfg(windows)]
    let final_component = path.rsplit(['/', '\\']).next().unwrap_or(path);
    #[cfg(not(windows))]
    let final_component = path.rsplit('/').next().unwrap_or(path);
    if final_component == "." || final_component == ".." {
        return Err(FsError::new(Code::Path));
    }
    Ok(parsed)
}

/// Copy a byte slice after checking its length against `max`.
fn bounded_copy(bytes: &[u8], max: usize) -> Result<Vec<u8>, FsError> {
    if bytes.len() > max {
        return Err(FsError::new(Code::Limit));
    }
    let mut copy = Vec::new();
    copy.try_reserve_exact(bytes.len())
        .map_err(|_| FsError::new(Code::Limit))?;
    copy.extend_from_slice(bytes);
    Ok(copy)
}

/// Convert a path checked by `requirePath`, which bounds its length and
/// rejects lone surrogates before the private native callback is reached.
fn checked_js_path(value: JsString<'_>, max: usize) -> Result<String, FsError> {
    let encoded = value
        .to_cstring()
        .map_err(|_| FsError::new(Code::Encoding))?;
    if encoded.len() > max {
        return Err(FsError::new(Code::Limit));
    }
    let mut text = String::new();
    text.try_reserve_exact(encoded.len())
        .map_err(|_| FsError::new(Code::Limit))?;
    text.push_str(encoded.as_str());
    Ok(text)
}

/// Owns the retained QuickJS string reference behind a UTF-16 export.
struct Utf16Export<'ctx, 'js> {
    ctx: &'ctx Ctx<'js>,
    pointer: NonNull<u16>,
    length: usize,
}

impl Utf16Export<'_, '_> {
    fn units(&self) -> &[u16] {
        // SAFETY: this export owns the aligned, initialized span. Its
        // checked length fits in a Rust slice and the borrow cannot outlive
        // the export. Callers do not call JS while borrowing it.
        unsafe { std::slice::from_raw_parts(self.pointer.as_ptr(), self.length) }
    }
}

impl Drop for Utf16Export<'_, '_> {
    fn drop(&mut self) {
        // SAFETY: the pointer came from this context's UTF-16 export and
        // is released exactly once, after its last Rust borrow ends.
        unsafe { qjs::JS_FreeCStringUTF16(self.ctx.as_raw().as_ptr(), self.pointer.as_ptr()) };
    }
}

fn snapshot_write_text(value: &JsString<'_>, max: usize) -> Result<String, FsError> {
    snapshot_write_text_with(value, max, encode_write_text)
}

/// Separate the length/encoding guard from the encoder so tests can assert
/// that rejected input never reaches the UTF-8 allocation.
fn snapshot_write_text_with(
    value: &JsString<'_>,
    max: usize,
    encode: impl FnOnce(&[u16], usize) -> Result<String, FsError>,
) -> Result<String, FsError> {
    let ctx = value.ctx();
    let code_units = string_length(ctx, value.as_value()).map_err(|_| {
        ctx.catch();
        FsError::new(Code::Limit)
    })?;
    if code_units > max {
        return Err(FsError::new(Code::Limit));
    }

    let mut length = 0;
    // SAFETY: `value` is rooted in `ctx` and is a primitive string, so no
    // user coercion can run. QuickJS either retains the existing immutable
    // wide string or owns a wide copy. The returned length counts u16s;
    // the export is not assumed to have a terminator.
    let pointer = unsafe {
        qjs::JS_ToCStringLenUTF16(
            ctx.as_raw().as_ptr(),
            &mut length,
            value.as_value().as_raw(),
        )
    };
    let Some(pointer) = NonNull::new(pointer.cast_mut()) else {
        ctx.catch();
        return Err(FsError::new(Code::Limit));
    };
    // Own the pointer before any fallible length checks so that it is freed
    // on both success and failure.
    let export = Utf16Export {
        ctx,
        pointer,
        length: code_units,
    };
    if usize::try_from(length).ok() != Some(code_units)
        || code_units > isize::MAX as usize / size_of::<u16>()
    {
        return Err(FsError::new(Code::Limit));
    }
    let units = export.units();

    // Check for lone surrogates through the end of the string even if the
    // valid prefix already exceeds the byte limit: FS_ENCODING wins there.
    // At most three bytes per code unit, which fits given the bound above.
    let mut byte_len = 0;
    for scalar in char::decode_utf16(units.iter().copied()) {
        let scalar = scalar.map_err(|_| FsError::new(Code::Encoding))?;
        byte_len += scalar.len_utf8();
    }
    if byte_len > max {
        return Err(FsError::new(Code::Limit));
    }
    encode(units, byte_len)
}

fn encode_write_text(units: &[u16], byte_len: usize) -> Result<String, FsError> {
    let mut text = String::new();
    text.try_reserve_exact(byte_len)
        .map_err(|_| FsError::new(Code::Limit))?;
    for scalar in char::decode_utf16(units.iter().copied()) {
        text.push(scalar.map_err(|_| FsError::new(Code::Encoding))?);
    }
    Ok(text)
}

/// Snapshot a branded view. The private wrapper obtains `backing` and `len`
/// from the captured %TypedArray% getters, without invoking user code.
fn copy_typed<'js>(
    array: &TypedArray<'js, u8>,
    backing: &Value<'js>,
    len: usize,
    max: usize,
) -> Result<Vec<u8>, FsError> {
    let ctx = array.ctx();
    let raw_ctx = ctx.as_raw().as_ptr();
    // SAFETY: `backing` is a rooted value supplied by the intrinsic getter.
    if !unsafe { qjs::JS_IsArrayBuffer(backing.as_raw()) } {
        // Do not form Rust slices over SharedArrayBuffer memory.
        return Err(FsError::new(Code::Unsupported));
    }
    let mut offset = 0;
    // SAFETY: array is a rooted Uint8Array from this context. QuickJS checks
    // detached/OOB before setting offset and returns an owned value on success.
    // Do NOT request its length: QuickJS 0.14 returns the creation-time
    // ta->length here, not the live byteLength of a length-tracking view.
    let buffer = unsafe {
        qjs::JS_GetTypedArrayBuffer(
            raw_ctx,
            array.as_value().as_raw(),
            &mut offset,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    // SAFETY: this is the valid result (including the exception sentinel)
    // returned by QuickJS above.
    let invalid_view = unsafe { qjs::JS_IsException(buffer) };
    if invalid_view {
        // This API uses the same exception for detached and OOB. Clear it and
        // query the actual backing buffer to distinguish those cases below.
        ctx.catch();
    } else {
        // SAFETY: non-exception buffer is an owned JSValue from this context.
        // `backing` already pins the same buffer throughout the copy.
        drop(unsafe { Value::from_raw(ctx.clone(), buffer) });
    }
    let mut backing_len = 0;
    // SAFETY: the known non-shared ArrayBuffer belongs to this context and
    // JS_GetArrayBuffer does not invoke user code.
    let data = unsafe { qjs::JS_GetArrayBuffer(raw_ctx, &mut backing_len, backing.as_raw()) };
    if data.is_null() && ctx.has_exception() {
        // On a branded ArrayBuffer this API only throws when detached.
        ctx.catch();
        return Err(FsError::new(Code::Detached));
    }
    let offset = offset as usize;
    if invalid_view || offset > backing_len as usize || len > backing_len as usize - offset {
        return Err(FsError::new(Code::OutOfBounds));
    }
    if len > max {
        return Err(FsError::new(Code::Limit));
    }
    if len == 0 {
        return Ok(Vec::new());
    }
    if data.is_null() {
        return Err(FsError::new(Code::Io));
    }
    // SAFETY: backing is rooted, non-shared, attached, and contains the entire
    // checked range. No JS runs between taking the pointer and completing the
    // copy. The resulting Vec owns its bytes before any option reflection.
    bounded_copy(
        unsafe { std::slice::from_raw_parts(data.add(offset), len) },
        max,
    )
}

fn read_bounded(reader: impl Read, capacity: usize, max: usize) -> Result<Vec<u8>, FsError> {
    read_bounded_until(reader, capacity, max, || false)
}

fn read_bounded_until(
    mut reader: impl Read,
    capacity: usize,
    max: usize,
    cancelled: impl Fn() -> bool,
) -> Result<Vec<u8>, FsError> {
    if capacity > max {
        return Err(FsError::new(Code::Limit));
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(capacity)
        .map_err(|_| FsError::new(Code::Limit))?;
    let mut buffer = [0u8; 8192];
    loop {
        if cancelled() {
            return Err(FsError::new(Code::Cancelled));
        }
        if bytes.len() == max {
            // A separate one-byte probe detects growth without asking Vec to
            // grow to max+1 merely to discover it.
            let mut probe = [0u8; 1];
            match reader.read(&mut probe) {
                Ok(0) => break,
                Ok(_) => return Err(FsError::new(Code::Limit)),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(FsError::from(error)),
            }
        }

        let available = (max - bytes.len()).min(buffer.len());
        let count = match reader.read(&mut buffer[..available]) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(FsError::from(error)),
        };
        bytes
            .try_reserve_exact(count)
            .map_err(|_| FsError::new(Code::Limit))?;
        bytes.extend_from_slice(&buffer[..count]);
    }
    Ok(bytes)
}

fn read_bytes_at(path: &Path, max: usize) -> Result<Vec<u8>, FsError> {
    let (file, metadata) = open_regular_source(path)?;
    if metadata.len() > max as u64 {
        return Err(FsError::new(Code::Limit));
    }
    let capacity = usize::try_from(metadata.len()).map_err(|_| FsError::new(Code::Limit))?;
    read_bounded(file, capacity, max)
}

fn open_regular_source(path: &Path) -> Result<(File, Metadata), FsError> {
    // Unix opens are nonblocking so a raced replacement with a FIFO cannot
    // hang between path lookup and descriptor inspection. The descriptor,
    // rather than a separate metadata snapshot, determines file type and size.
    let mut options = OpenOptions::new();
    options.read(true);
    set_data_open_flags(&mut options);
    let file = options
        .open(path)
        .map_err(|error| open_error(path, error))?;
    let metadata = regular_file_metadata(&file)?;
    Ok((file, metadata))
}

fn regular_file_metadata(file: &File) -> Result<Metadata, FsError> {
    let metadata = file.metadata().map_err(FsError::from)?;
    if metadata.is_dir() {
        return Err(FsError::new(Code::IsDirectory));
    }
    if !metadata.is_file() {
        return Err(FsError::new(Code::NotRegular));
    }
    Ok(metadata)
}

fn write_bytes_at(path: &Path, data: &[u8], overwrite: bool, max: usize) -> Result<(), FsError> {
    if data.len() > max {
        return Err(FsError::new(Code::Limit));
    }

    let mut options = OpenOptions::new();
    options.write(true);
    if overwrite {
        // Follow/create (including a dangling symlink target) without
        // truncating until the opened descriptor has been checked.
        options.create(true);
    } else {
        // `create_new` makes the no-clobber decision inside the open call, so
        // there is no separate existence check to lose a race.
        options.create_new(true);
    }
    set_data_open_flags(&mut options);
    let mut file = options
        .open(path)
        .map_err(|error| open_error(path, error))?;
    regular_file_metadata(&file)?;
    if overwrite {
        // Truncate the descriptor just verified, not a pathname that may have
        // been replaced between an earlier metadata lookup and this call.
        file.set_len(0).map_err(FsError::from)?;
    }
    file.write_all(data).map_err(FsError::from)?;
    Ok(())
}

/// Flags for opening a path whose type is not yet known. `O_NOCTTY` prevents
/// acquiring a controlling terminal before descriptor validation.
#[cfg(unix)]
const fn unix_data_open_flags() -> libc::c_int {
    libc::O_NONBLOCK | libc::O_NOCTTY
}

fn set_data_open_flags(options: &mut OpenOptions) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(unix_data_open_flags());
    }
    #[cfg(not(unix))]
    let _ = options;
}

/// Refine otherwise ambiguous open failures with a best-effort metadata
/// snapshot. Nothing is read, written or truncated based on this snapshot;
/// a concurrently replaced path can still yield a different error code.
fn open_error(path: &Path, error: io::Error) -> FsError {
    #[cfg(not(any(unix, windows)))]
    let _ = path;
    #[cfg(unix)]
    {
        // Nonblocking write-only FIFO open returns ENXIO if no reader exists.
        // Other objects can return ENXIO too; confirm a non-regular type.
        if error.raw_os_error() == Some(libc::ENXIO)
            && fs::metadata(path).is_ok_and(|metadata| !metadata.is_file() && !metadata.is_dir())
        {
            return FsError::new(Code::NotRegular);
        }
    }
    #[cfg(windows)]
    if error.kind() == io::ErrorKind::PermissionDenied
        && fs::metadata(path).is_ok_and(|metadata| metadata.is_dir())
    {
        // Windows may deny a regular file open on a directory before we have
        // a descriptor to inspect. Keep the original error if type is unknown.
        return FsError::new(Code::IsDirectory);
    }
    FsError::from(error)
}

fn size_text(size: u64) -> String {
    size.to_string()
}

fn mtime_millis(time: SystemTime) -> String {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_millis().to_string(),
        Err(error) => {
            let millis = error.duration().as_millis();
            if millis == 0 {
                "0".to_owned()
            } else {
                format!("-{millis}")
            }
        }
    }
}

#[derive(Clone, Debug)]
struct FileInfo {
    is_file: bool,
    is_directory: bool,
    is_symlink: bool,
    size: String,
    mtime_ms: Option<String>,
}

fn file_info_at(path: &Path, follow: bool) -> Result<FileInfo, FsError> {
    let metadata: Metadata = if follow {
        fs::metadata(path).map_err(FsError::from)?
    } else {
        fs::symlink_metadata(path).map_err(FsError::from)?
    };
    Ok(FileInfo {
        is_file: metadata.is_file(),
        is_directory: metadata.is_dir(),
        // `metadata` for a followed path describes the target, so it can never
        // be a symlink. Only lstat can report a link.
        is_symlink: !follow && metadata.file_type().is_symlink(),
        size: size_text(metadata.len()),
        mtime_ms: metadata.modified().ok().map(mtime_millis),
    })
}

fn exists_at(path: &Path) -> Result<bool, FsError> {
    match fs::metadata(path) {
        Ok(_) => Ok(true),
        Err(error) => match error.kind() {
            // A dangling symlink has no target, and a path below a non-directory
            // cannot exist. Other failures (notably permission denial) are
            // reported rather than hidden as `false`.
            io::ErrorKind::NotFound | io::ErrorKind::NotADirectory => Ok(false),
            _ => Err(FsError::from(error)),
        },
    }
}

#[derive(Clone, Debug)]
struct DirEntryInfo {
    name: String,
    is_file: bool,
    is_directory: bool,
    is_symlink: bool,
}

fn read_dir_at(
    path: &Path,
    max_entries: usize,
    max_name_bytes: usize,
    max_total_bytes: usize,
) -> Result<Vec<DirEntryInfo>, FsError> {
    let iterator = fs::read_dir(path).map_err(FsError::from)?;
    let mut entries = Vec::new();
    let mut total_bytes = 0usize;
    for entry in iterator {
        let entry = entry.map_err(FsError::from)?;
        let name = entry.file_name();
        let name = name.to_str().ok_or(FsError::new(Code::Encoding))?;
        if name.len() > max_name_bytes {
            return Err(FsError::new(Code::Limit));
        }
        total_bytes = total_bytes
            .checked_add(name.len())
            .ok_or(FsError::new(Code::Limit))?;
        if total_bytes > max_total_bytes {
            return Err(FsError::new(Code::Limit));
        }
        if entries.len() >= max_entries {
            return Err(FsError::new(Code::Limit));
        }

        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            // Some filesystems do not provide a directory-entry type. Fall back
            // to a no-follow metadata query rather than reporting a wrong type.
            Err(_) => fs::symlink_metadata(entry.path())
                .map_err(FsError::from)?
                .file_type(),
        };

        let mut owned = String::new();
        owned
            .try_reserve_exact(name.len())
            .map_err(|_| FsError::new(Code::Limit))?;
        owned.push_str(name);
        entries
            .try_reserve(1)
            .map_err(|_| FsError::new(Code::Limit))?;
        entries.push(DirEntryInfo {
            name: owned,
            is_file: file_type.is_file(),
            is_directory: file_type.is_dir(),
            is_symlink: file_type.is_symlink(),
        });
    }
    // `read_dir` yields an unspecified order. Sorting makes the JavaScript
    // result deterministic without following entries.
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(entries)
}

fn mkdir_at(path: &Path, recursive: bool) -> Result<(), FsError> {
    if recursive {
        fs::create_dir_all(path).map_err(FsError::from)
    } else {
        fs::create_dir(path).map_err(FsError::from)
    }
}

fn remove_at(path: &Path, recursive: bool) -> Result<(), FsError> {
    let metadata = fs::symlink_metadata(path).map_err(FsError::from)?;
    let is_directory = metadata.is_dir() && !metadata.file_type().is_symlink();
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileTypeExt;
        if metadata.file_type().is_symlink_dir() {
            // The reparse entry identifies a directory link even if the
            // target is missing or inaccessible; never inspect the target.
            return Err(FsError::new(Code::Unsupported));
        }
    }
    if recursive {
        if is_directory {
            fs::remove_dir_all(path).map_err(FsError::from)
        } else {
            fs::remove_file(path).map_err(FsError::from)
        }
    } else if is_directory {
        fs::remove_dir(path).map_err(FsError::from)
    } else {
        fs::remove_file(path).map_err(FsError::from)
    }
}

fn read_text(cwd: &Rc<RefCell<WorkingDirectory>>, path: &str) -> Result<String, FsError> {
    let bytes = read_bytes(cwd, path, MAX_READ_BYTES)?;
    String::from_utf8(bytes).map_err(|_| FsError::new(Code::Encoding))
}

fn read_bytes(
    cwd: &Rc<RefCell<WorkingDirectory>>,
    path: &str,
    max: usize,
) -> Result<Vec<u8>, FsError> {
    let path = resolve(cwd, path)?;
    read_bytes_at(&path, max)
}

fn write_text(cwd: &Rc<RefCell<WorkingDirectory>>, path: &str, data: &str) -> Result<(), FsError> {
    if data.len() > MAX_WRITE_BYTES {
        return Err(FsError::new(Code::Limit));
    }
    let path = resolve(cwd, path)?;
    write_bytes_at(&path, data.as_bytes(), true, MAX_WRITE_BYTES)
}

fn write_file(
    cwd: &Rc<RefCell<WorkingDirectory>>,
    path: &str,
    data: &[u8],
    overwrite: bool,
) -> Result<(), FsError> {
    let path = resolve(cwd, path)?;
    write_bytes_at(&path, data, overwrite, MAX_WRITE_BYTES)
}

fn stat_path(cwd: &Rc<RefCell<WorkingDirectory>>, path: &str) -> Result<FileInfo, FsError> {
    let path = resolve(cwd, path)?;
    file_info_at(&path, true)
}

fn lstat_path(cwd: &Rc<RefCell<WorkingDirectory>>, path: &str) -> Result<FileInfo, FsError> {
    let path = resolve(cwd, path)?;
    file_info_at(&path, false)
}

fn exists_path(cwd: &Rc<RefCell<WorkingDirectory>>, path: &str) -> Result<bool, FsError> {
    let path = resolve(cwd, path)?;
    exists_at(&path)
}

fn read_dir_path(
    cwd: &Rc<RefCell<WorkingDirectory>>,
    path: &str,
) -> Result<Vec<DirEntryInfo>, FsError> {
    let path = resolve(cwd, path)?;
    read_dir_at(
        &path,
        MAX_LIST_ENTRIES,
        MAX_LIST_NAME_BYTES,
        MAX_LIST_TOTAL_BYTES,
    )
}

fn mkdir_path(
    cwd: &Rc<RefCell<WorkingDirectory>>,
    path: &str,
    recursive: bool,
) -> Result<(), FsError> {
    let path = resolve(cwd, path)?;
    mkdir_at(&path, recursive)
}

fn remove_path(
    cwd: &Rc<RefCell<WorkingDirectory>>,
    path: &str,
    recursive: bool,
) -> Result<(), FsError> {
    let path = validate_entry_path(path)?;
    let path = resolve_path(&cwd.borrow(), path)?;
    remove_at(&path, recursive)
}

fn resolve_remove_text(cwd: &Rc<RefCell<WorkingDirectory>>, path: &str) -> Result<String, FsError> {
    let path = validate_entry_path(path)?;
    path_into_text(resolve_path(&cwd.borrow(), path)?)
}

/// Resolve two already validated paths using one immutable cwd snapshot.
fn resolve_pair(
    cwd: &RefCell<WorkingDirectory>,
    source: &Path,
    destination: &Path,
) -> Result<(PathBuf, PathBuf), FsError> {
    let cwd = cwd.borrow();
    Ok((
        resolve_path(&cwd, source)?,
        resolve_path(&cwd, destination)?,
    ))
}

/// Rename two validated directory entries without inspecting either on disk.
fn rename_path(
    cwd: &RefCell<WorkingDirectory>,
    source: &str,
    destination: &str,
) -> Result<(), FsError> {
    let source = validate_entry_path(source)?;
    let destination = validate_entry_path(destination)?;
    let (source, destination) = resolve_pair(cwd, source, destination)?;
    fs::rename(source, destination).map_err(FsError::from)
}

/// On Windows restrict copy operands to ordinary disk paths, excluding
/// namespaces, device names and alternate data streams. Also check the
/// resolved paths since a relative operand inherits its cwd's prefix.
#[cfg(windows)]
fn validate_copy_disk_path(path: &Path) -> Result<(), FsError> {
    use std::path::{Component, Prefix};

    let text = path.to_str().ok_or(FsError::new(Code::Encoding))?;
    let tail = match path.components().next() {
        Some(Component::Prefix(prefix))
            if path.is_absolute()
                && matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)) =>
        {
            &text[prefix.as_os_str().len()..]
        }
        Some(Component::Prefix(_) | Component::RootDir) => return Err(FsError::new(Code::Path)),
        _ => text,
    };
    for component in tail.split(['/', '\\']) {
        if matches!(component, "" | "." | "..") {
            continue;
        }
        if component.ends_with(['.', ' '])
            || component.contains([':', '<', '>', '"', '|', '?', '*'])
            || component.bytes().any(|byte| byte < 0x20)
        {
            return Err(FsError::new(Code::Path));
        }
        let stem = component
            .split('.')
            .next()
            .unwrap_or("")
            .trim_end_matches(' ');
        let numbered_device = stem.get(..3).is_some_and(|prefix| {
            (prefix.eq_ignore_ascii_case("COM") || prefix.eq_ignore_ascii_case("LPT"))
                && matches!(
                    &stem[3..],
                    "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
        });
        if ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$", "CLOCK$"]
            .iter()
            .any(|name| stem.eq_ignore_ascii_case(name))
            || numbered_device
        {
            return Err(FsError::new(Code::Path));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn require_disk_handle(file: &File) -> Result<(), FsError> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{FILE_TYPE_DISK, GetFileType};

    // SAFETY: `file` owns the live handle for the duration of the call.
    if unsafe { GetFileType(file.as_raw_handle()) } != FILE_TYPE_DISK {
        return Err(FsError::new(Code::NotRegular));
    }
    Ok(())
}

/// Copy without replacing or removing a destination entry, including on error.
fn copy_file_path(
    cwd: &RefCell<WorkingDirectory>,
    source: &str,
    destination: &str,
) -> Result<(), FsError> {
    let source = validate_entry_path(source)?;
    let destination = validate_entry_path(destination)?;
    #[cfg(windows)]
    {
        validate_copy_disk_path(source)?;
        validate_copy_disk_path(destination)?;
    }
    let (source, destination) = resolve_pair(cwd, source, destination)?;
    #[cfg(windows)]
    {
        validate_copy_disk_path(&source)?;
        validate_copy_disk_path(&destination)?;
    }

    let (source, metadata) = open_regular_source(&source)?;
    #[cfg(windows)]
    require_disk_handle(&source)?;
    if metadata.len() > MAX_READ_BYTES as u64 {
        return Err(FsError::new(Code::Limit));
    }

    // Exclusive creation never opens an existing inode for writing, even if
    // both operands resolve to the same file. Don't check existence first.
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    set_data_open_flags(&mut options);
    let destination = options.open(destination).map_err(FsError::from)?;
    #[cfg(windows)]
    require_disk_handle(&destination)?;
    regular_file_metadata(&destination)?;
    // No unlink on failure: the pathname could now name someone else's file.
    copy_bounded(source, destination, MAX_READ_BYTES)
}

/// Stream at most `remaining` bytes with a fixed buffer and probe for growth.
fn copy_bounded(
    mut source: impl Read,
    mut destination: impl Write,
    mut remaining: usize,
) -> Result<(), FsError> {
    let mut buffer = [0u8; 8192];
    loop {
        let available = remaining.clamp(1, buffer.len());
        match source.read(&mut buffer[..available]) {
            Ok(0) => return Ok(()),
            Ok(_) if remaining == 0 => return Err(FsError::new(Code::Limit)),
            Ok(count) => {
                destination
                    .write_all(&buffer[..count])
                    .map_err(FsError::from)?;
                remaining -= count;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(FsError::from(error)),
        }
    }
}

fn ok_object<'js>(ctx: &Ctx<'js>) -> JsResult<Object<'js>> {
    let object = Object::new_proto(ctx.clone(), None)?;
    object.set("ok", true)?;
    Ok(object)
}

fn error_object<'js>(ctx: &Ctx<'js>, error: FsError) -> JsResult<Object<'js>> {
    let object = Object::new_proto(ctx.clone(), None)?;
    object.set("ok", false)?;
    object.set("code", error.code())?;
    object.set("message", error.message())?;
    Ok(object)
}

fn void_outcome<'js>(ctx: &Ctx<'js>, result: Result<(), FsError>) -> JsResult<Object<'js>> {
    match result {
        Ok(()) => ok_object(ctx),
        Err(error) => error_object(ctx, error),
    }
}

fn string_outcome<'js>(ctx: &Ctx<'js>, result: Result<String, FsError>) -> JsResult<Object<'js>> {
    match result {
        Ok(value) => {
            let object = ok_object(ctx)?;
            object.set("value", value)?;
            Ok(object)
        }
        Err(error) => error_object(ctx, error),
    }
}

fn bool_outcome<'js>(ctx: &Ctx<'js>, result: Result<bool, FsError>) -> JsResult<Object<'js>> {
    match result {
        Ok(value) => {
            let object = ok_object(ctx)?;
            object.set("value", value)?;
            Ok(object)
        }
        Err(error) => error_object(ctx, error),
    }
}

fn bytes_outcome<'js>(ctx: &Ctx<'js>, result: Result<Vec<u8>, FsError>) -> JsResult<Object<'js>> {
    match result {
        Ok(value) => {
            let object = ok_object(ctx)?;
            object.set("value", TypedArray::new(ctx.clone(), value)?)?;
            Ok(object)
        }
        Err(error) => error_object(ctx, error),
    }
}

fn file_info_outcome<'js>(
    ctx: &Ctx<'js>,
    result: Result<FileInfo, FsError>,
) -> JsResult<Object<'js>> {
    match result {
        Ok(info) => {
            let object = ok_object(ctx)?;
            let value = Object::new_proto(ctx.clone(), None)?;
            value.set("isFile", info.is_file)?;
            value.set("isDirectory", info.is_directory)?;
            value.set("isSymlink", info.is_symlink)?;
            value.set("size", info.size)?;
            match info.mtime_ms {
                Some(text) => value.set("mtimeMs", text)?,
                None => value.set("mtimeMs", Value::new_null(ctx.clone()))?,
            }
            object.set("value", value)?;
            Ok(object)
        }
        Err(error) => error_object(ctx, error),
    }
}

fn entries_outcome<'js>(
    ctx: &Ctx<'js>,
    result: Result<Vec<DirEntryInfo>, FsError>,
) -> JsResult<Object<'js>> {
    match result {
        Ok(entries) => {
            let object = ok_object(ctx)?;
            let values: Vec<serde_json::Value> = entries
                .into_iter()
                .map(|entry| {
                    serde_json::json!({
                        "name": entry.name,
                        "isFile": entry.is_file,
                        "isDirectory": entry.is_directory,
                        "isSymlink": entry.is_symlink,
                    })
                })
                .collect();
            // The wrapper parses the bounded JSON with the realm's captured
            // JSON.parse. JSON parsing creates own array index properties and
            // cannot invoke a polluted Array.prototype numeric setter.
            object.set("value", serde_json::Value::Array(values).to_string())?;
            Ok(object)
        }
        Err(error) => error_object(ctx, error),
    }
}

fn build_native<'js>(
    ctx: &Ctx<'js>,
    cwd: Rc<RefCell<WorkingDirectory>>,
    driver: ReadDriver,
) -> JsResult<Object<'js>> {
    let native = Object::new_proto(ctx.clone(), None)?;

    let snapshot_cwd = Rc::clone(&cwd);
    native.set(
        "snapshotPath",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, path: JsString<'js>| -> JsResult<Object<'js>> {
                let result = checked_js_path(path, MAX_PATH_BYTES)
                    .and_then(|path| resolve_text(&snapshot_cwd, &path));
                string_outcome(&ctx, result)
            },
        )?,
    )?;

    let snapshot_remove_cwd = Rc::clone(&cwd);
    native.set(
        "snapshotRemovePath",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, path: JsString<'js>| -> JsResult<Object<'js>> {
                let result = checked_js_path(path, MAX_PATH_BYTES)
                    .and_then(|path| resolve_remove_text(&snapshot_remove_cwd, &path));
                string_outcome(&ctx, result)
            },
        )?,
    )?;

    native.set(
        "copyData",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>,
                  data: TypedArray<'js, u8>,
                  backing: Value<'js>,
                  len: usize|
                  -> JsResult<Object<'js>> {
                bytes_outcome(&ctx, copy_typed(&data, &backing, len, MAX_WRITE_BYTES))
            },
        )?,
    )?;

    let read_text_cwd = Rc::clone(&cwd);
    native.set(
        "readText",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, path: String| -> JsResult<Object<'js>> {
                string_outcome(&ctx, read_text(&read_text_cwd, &path))
            },
        )?,
    )?;

    let write_text_cwd = Rc::clone(&cwd);
    native.set(
        "writeText",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>,
                  path: JsString<'js>,
                  data: JsString<'js>|
                  -> JsResult<Object<'js>> {
                let result = snapshot_write_text(&data, MAX_WRITE_BYTES).and_then(|data| {
                    // Keep the same absolute, UTF-8 path snapshot and checks
                    // as the old wrapper, but only after the text is owned.
                    let path = checked_js_path(path, MAX_PATH_BYTES)?;
                    let path = resolve_text(&write_text_cwd, &path)?;
                    write_text(&write_text_cwd, &path, &data)
                });
                void_outcome(&ctx, result)
            },
        )?,
    )?;

    let read_file_cwd = Rc::clone(&cwd);
    native.set(
        "readFileSync",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, path: String| -> JsResult<Object<'js>> {
                bytes_outcome(&ctx, read_bytes(&read_file_cwd, &path, MAX_READ_BYTES))
            },
        )?,
    )?;
    read_async::install(ctx, &native, driver, Rc::clone(&cwd))?;

    let write_file_cwd = Rc::clone(&cwd);
    native.set(
        "writeFile",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>,
                  path: String,
                  data: TypedArray<'js, u8>,
                  overwrite: bool|
                  -> JsResult<Object<'js>> {
                // SAFETY: the private wrapper passes only the fixed, owned
                // Uint8Array snapshot created by copyData, never the caller's
                // original input. This borrow ends before JS runs again.
                let result = match unsafe { data.as_bytes() } {
                    Some(bytes) => write_file(&write_file_cwd, &path, bytes, overwrite),
                    None => {
                        ctx.catch();
                        Err(FsError::new(Code::Io))
                    }
                };
                void_outcome(&ctx, result)
            },
        )?,
    )?;

    let stat_cwd = Rc::clone(&cwd);
    native.set(
        "stat",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, path: String| -> JsResult<Object<'js>> {
                file_info_outcome(&ctx, stat_path(&stat_cwd, &path))
            },
        )?,
    )?;

    let lstat_cwd = Rc::clone(&cwd);
    native.set(
        "lstat",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, path: String| -> JsResult<Object<'js>> {
                file_info_outcome(&ctx, lstat_path(&lstat_cwd, &path))
            },
        )?,
    )?;

    let exists_cwd = Rc::clone(&cwd);
    native.set(
        "exists",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, path: String| -> JsResult<Object<'js>> {
                bool_outcome(&ctx, exists_path(&exists_cwd, &path))
            },
        )?,
    )?;

    let read_dir_cwd = Rc::clone(&cwd);
    native.set(
        "readDir",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, path: String| -> JsResult<Object<'js>> {
                entries_outcome(&ctx, read_dir_path(&read_dir_cwd, &path))
            },
        )?,
    )?;

    let mkdir_cwd = Rc::clone(&cwd);
    native.set(
        "mkdir",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, path: String, recursive: bool| -> JsResult<Object<'js>> {
                void_outcome(&ctx, mkdir_path(&mkdir_cwd, &path, recursive))
            },
        )?,
    )?;

    let remove_cwd = Rc::clone(&cwd);
    native.set(
        "remove",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, path: String, recursive: bool| -> JsResult<Object<'js>> {
                void_outcome(&ctx, remove_path(&remove_cwd, &path, recursive))
            },
        )?,
    )?;

    type EntryOperation = fn(&RefCell<WorkingDirectory>, &str, &str) -> Result<(), FsError>;
    let operations: [(&str, EntryOperation); 2] =
        [("rename", rename_path), ("copyFile", copy_file_path)];
    for (name, operation) in operations {
        let cwd = Rc::clone(&cwd);
        native.set(
            name,
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'js>,
                      source: JsString<'js>,
                      destination: JsString<'js>|
                      -> JsResult<Object<'js>> {
                    let result = (|| {
                        let source = checked_js_path(source, MAX_PATH_BYTES)?;
                        let destination = checked_js_path(destination, MAX_PATH_BYTES)?;
                        operation(&cwd, &source, &destination)
                    })();
                    void_outcome(&ctx, result)
                },
            )?,
        )?;
    }

    Ok(native)
}

fn build_config<'js>(ctx: &Ctx<'js>) -> JsResult<Object<'js>> {
    let limits = Object::new_proto(ctx.clone(), None)?;
    limits.set("MAX_PATH_BYTES", MAX_PATH_BYTES as u64)?;
    limits.set("MAX_READ_BYTES", MAX_READ_BYTES as u64)?;
    limits.set("MAX_WRITE_BYTES", MAX_WRITE_BYTES as u64)?;
    limits.set("MAX_LIST_ENTRIES", MAX_LIST_ENTRIES as u64)?;
    limits.set("MAX_LIST_NAME_BYTES", MAX_LIST_NAME_BYTES as u64)?;
    limits.set("MAX_LIST_TOTAL_BYTES", MAX_LIST_TOTAL_BYTES as u64)?;

    let messages = Object::new_proto(ctx.clone(), None)?;
    for code in Code::ALL {
        messages.set(code.as_str(), code.message())?;
    }

    let config = Object::new_proto(ctx.clone(), None)?;
    config.set("limits", limits)?;
    config.set("messages", messages)?;
    Ok(config)
}

/// The private factory and the pristine export object for `neo:fs`.
pub(crate) struct FsState {
    pub(crate) factory: Option<Persistent<Function<'static>>>,
    pub(crate) pristine: Option<Persistent<Object<'static>>>,
    cwd: Rc<RefCell<WorkingDirectory>>,
}

impl FsState {
    pub(crate) fn new(cwd: Rc<RefCell<WorkingDirectory>>) -> Self {
        Self {
            factory: None,
            pristine: None,
            cwd,
        }
    }
}

// FsState holds no borrowed JS value: its persistent handles are released by
// `NeoJs::drop` while the runtime is still alive.
unsafe impl<'js> rquickjs::JsLifetime<'js> for FsState {
    type Changed<'to> = FsState;
}

/// Build the private wrapper in the fresh realm, before user code runs.
pub(crate) fn factory<'js>(ctx: &Ctx<'js>) -> JsResult<Function<'js>> {
    ctx.eval(include_str!("js/fs.js"))
}

/// Build the single API object for this runtime and keep a pristine copy for
/// `neo:fs` exports.
///
/// The global `fs` object and the module's exports share the same function and
/// `FsError` identities, but the module's copy is a separate object holding the
/// original values. Replacing a property on `fs` therefore cannot replace an
/// imported function.
pub(crate) fn install<'js>(
    ctx: &Ctx<'js>,
    state: &Rc<RefCell<FsState>>,
    driver: ReadDriver,
) -> JsResult<Object<'js>> {
    let (factory, cwd) = {
        let state = state.borrow();
        (
            state.factory.as_ref().cloned().ok_or(JsError::Unknown)?,
            Rc::clone(&state.cwd),
        )
    };
    let factory = factory.restore(ctx)?;
    let native = build_native(ctx, cwd, driver)?;
    let config = build_config(ctx)?;
    let api: Object = factory.call((native, config))?;

    let pristine = Object::new_proto(ctx.clone(), None)?;
    for name in EXPORTS {
        let value: Value = api.get(name)?;
        if value.is_undefined() {
            return Err(JsError::Unknown);
        }
        pristine.set(name, value)?;
    }

    let mut state = state.borrow_mut();
    state.factory = None;
    state.pristine = Some(Persistent::save(ctx, pristine));
    Ok(api)
}

/// The native `neo:fs` module. Its callbacks are private to the runtime and
/// are never placed on the global object except through the `fs` API object.
pub(crate) struct FsModule;

impl ModuleDef for FsModule {
    fn declare<'js>(decl: &Declarations<'js>) -> JsResult<()> {
        for name in EXPORTS {
            decl.declare(name)?;
        }
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> JsResult<()> {
        let state = Rc::clone(
            &*ctx
                .userdata::<Rc<RefCell<FsState>>>()
                .ok_or(JsError::Unknown)?,
        );
        let pristine = {
            let state = state.borrow();
            state.pristine.as_ref().cloned().ok_or(JsError::Unknown)?
        }
        .restore(ctx)?;
        for name in EXPORTS {
            exports.export(name, pristine.get::<_, Value>(name)?)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn sizes_and_times_are_exact_decimal_strings() {
        assert_eq!(size_text(0), "0");
        assert_eq!(size_text(u64::MAX), "18446744073709551615");
        assert_eq!(size_text(9_007_199_254_740_993), "9007199254740993");
        assert_eq!(mtime_millis(UNIX_EPOCH), "0");
        assert_eq!(
            mtime_millis(UNIX_EPOCH + Duration::from_millis(1_234)),
            "1234"
        );
        assert_eq!(mtime_millis(UNIX_EPOCH - Duration::from_millis(1)), "-1");
        assert_eq!(mtime_millis(UNIX_EPOCH - Duration::from_nanos(1)), "0");
    }

    #[test]
    fn paths_are_bounded_and_reject_embedded_nul() {
        assert!(validate_path("").is_err());
        assert_eq!(validate_path("").unwrap_err().code(), "FS_PATH");
        assert_eq!(validate_path("a\0b").unwrap_err().code(), "FS_PATH");
        let long = "x".repeat(MAX_PATH_BYTES + 1);
        assert_eq!(validate_path(&long).unwrap_err().code(), "FS_LIMIT");
        let at_limit = "x".repeat(MAX_PATH_BYTES);
        assert!(validate_path(&at_limit).is_ok());
        assert!(validate_path("/absolute/path").is_ok());
    }

    #[test]
    // Test root syntax without probing the host root if validation regresses.
    fn entry_validation_rejects_roots_trailing_separators_and_terminal_dot_components() {
        for path in ["/", "tree/", "tree/.", "tree/..", ".", ".."] {
            assert_eq!(
                validate_entry_path(path).unwrap_err().code(),
                "FS_PATH",
                "{path:?}"
            );
        }
        assert!(validate_entry_path("tree/child").is_ok());
        assert!(validate_entry_path("tree/../child").is_ok());
    }

    #[test]
    fn cross_device_error_mapping_does_not_require_multiple_filesystems() {
        let error = FsError::from(io::Error::new(
            io::ErrorKind::CrossesDevices,
            "private path marker",
        ));
        assert_eq!(error.code(), "FS_CROSS_DEVICE");
        assert_eq!(error.message(), "cannot rename across filesystems");
    }

    #[cfg(windows)]
    #[test]
    fn windows_entry_validation_rejects_roots_and_drive_relative_paths() {
        // Pure parser assertions: these paths must never be accessed.
        for path in [
            r"C:\",
            r"C:/",
            r"\",
            r"\\server\share",
            r"\\server\share\",
            "//server/share",
            r"\\?\UNC\server\share",
            r"\\?\UNC\server\share\",
            r"\\?\C:\",
            r"C:child",
            r"C:",
        ] {
            assert_eq!(
                validate_entry_path(path).unwrap_err().code(),
                "FS_PATH",
                "{path:?}"
            );
        }
        for path in [
            r"C:\child",
            r"\\server\share\child",
            r"\\?\UNC\server\share\child",
        ] {
            assert!(validate_entry_path(path).is_ok(), "{path:?}");
        }
    }

    #[test]
    fn io_error_kinds_map_to_stable_codes() {
        for (kind, code) in [
            (io::ErrorKind::NotFound, Code::NotFound),
            (io::ErrorKind::PermissionDenied, Code::Denied),
            (io::ErrorKind::AlreadyExists, Code::AlreadyExists),
            (io::ErrorKind::InvalidInput, Code::Path),
            (io::ErrorKind::InvalidFilename, Code::Path),
            (io::ErrorKind::InvalidData, Code::Encoding),
            (io::ErrorKind::IsADirectory, Code::IsDirectory),
            (io::ErrorKind::NotADirectory, Code::NotDirectory),
            (io::ErrorKind::DirectoryNotEmpty, Code::NotEmpty),
            (io::ErrorKind::StorageFull, Code::Limit),
            (io::ErrorKind::OutOfMemory, Code::Limit),
        ] {
            assert_eq!(map_io_error(kind), code);
        }
    }

    #[test]
    fn bounded_copy_checks_before_allocating() {
        assert_eq!(bounded_copy(&[1, 2, 3], 3).unwrap(), vec![1, 2, 3]);
        assert_eq!(bounded_copy(&[1, 2, 3], 2).unwrap_err().code(), "FS_LIMIT");
    }

    #[test]
    fn text_snapshot_never_calls_encoder_on_invalid_input() {
        use rquickjs::{Context, Runtime};

        // Exact assertions for the codec's hot-path allocation guard.
        let runtime = Runtime::new().unwrap();
        let context = Context::full(&runtime).unwrap();
        context.with(|ctx| {
            for (character, count) in [('a', 12), ('é', 6), ('界', 4), ('😀', 3)] {
                let exact = character.to_string().repeat(count);
                let value = JsString::from_str(ctx.clone(), &exact).unwrap();
                assert_eq!(snapshot_write_text(&value, 12).unwrap(), exact);

                let above = JsString::from_str(ctx.clone(), &format!("{exact}a")).unwrap();
                let error =
                    snapshot_write_text_with(&above, 12, |_, _| panic!("encoder ran")).unwrap_err();
                assert_eq!(error.code, Code::Limit);
            }
            for (source, code) in [
                (r"'界界界界界\uD800'", Code::Encoding),
                (r"'界界界界界\uDC00'", Code::Encoding),
                (r"'a'.repeat(12) + '\uD800'", Code::Limit),
            ] {
                let value: JsString<'_> = ctx.eval(source).unwrap();
                let error =
                    snapshot_write_text_with(&value, 12, |_, _| panic!("encoder ran")).unwrap_err();
                assert_eq!(error.code, code);
            }
        });
    }

    #[test]
    fn bounded_reader_detects_growth_with_a_separate_probe() {
        assert_eq!(
            read_bounded(io::Cursor::new(b"abcd"), 1, 4).unwrap(),
            b"abcd"
        );
        assert_eq!(
            read_bounded(io::Cursor::new(b"abcde"), 1, 4)
                .unwrap_err()
                .code(),
            "FS_LIMIT"
        );
        assert_eq!(read_bounded(io::Cursor::new(b"abc"), 4, 4).unwrap(), b"abc");
    }

    // Exact assertion for the copy budget: a source that grows after the
    // metadata check may contribute a probe byte, but not write that byte.
    #[test]
    fn bounded_copy_checks_a_probe_without_writing_past_the_limit() {
        let mut destination = [0; 4];
        let mut target = destination.as_mut_slice();
        let mut source = &b"abcde"[..];
        let error = copy_bounded(&mut source, &mut target, 3).unwrap_err();
        assert_eq!(error.code(), "FS_LIMIT");
        assert_eq!(source, b"e");
        assert_eq!(destination, *b"abc\0");
    }

    #[test]
    fn reading_rejects_directories_and_oversize_files() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            read_bytes_at(dir.path(), MAX_READ_BYTES)
                .unwrap_err()
                .code(),
            "FS_IS_DIRECTORY"
        );

        let file = dir.path().join("big");
        let handle = fs::File::create(&file).unwrap();
        handle.set_len(MAX_READ_BYTES as u64 + 1).unwrap();
        drop(handle);
        assert_eq!(
            read_bytes_at(&file, MAX_READ_BYTES).unwrap_err().code(),
            "FS_LIMIT"
        );
        assert_eq!(
            file_info_at(&file, false).unwrap().size,
            (MAX_READ_BYTES as u64 + 1).to_string()
        );
    }

    #[test]
    fn writing_obeys_overwrite_and_regular_file_rules() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file");
        write_bytes_at(&file, b"first", true, MAX_WRITE_BYTES).unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"first");

        assert_eq!(
            write_bytes_at(&file, b"second", false, MAX_WRITE_BYTES)
                .unwrap_err()
                .code(),
            "FS_ALREADY_EXISTS"
        );
        assert_eq!(fs::read(&file).unwrap(), b"first");

        write_bytes_at(&file, b"third", true, MAX_WRITE_BYTES).unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"third");

        assert_eq!(
            write_bytes_at(dir.path(), b"x", true, MAX_WRITE_BYTES)
                .unwrap_err()
                .code(),
            "FS_IS_DIRECTORY"
        );
        assert_eq!(
            write_bytes_at(&file, b"x", true, 0).unwrap_err().code(),
            "FS_LIMIT"
        );
    }

    #[test]
    fn listing_is_bounded_and_sorted() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("b"), b"").unwrap();
        fs::write(dir.path().join("a"), b"").unwrap();
        fs::create_dir(dir.path().join("c")).unwrap();

        let entries = read_dir_at(dir.path(), 3, 1024, 1024).unwrap();
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["a", "b", "c"]);
        assert!(entries[0].is_file);
        assert!(entries[2].is_directory);

        assert_eq!(
            read_dir_at(dir.path(), 2, 1024, 1024).unwrap_err().code(),
            "FS_LIMIT"
        );
        assert_eq!(
            read_dir_at(dir.path(), 3, 0, 1024).unwrap_err().code(),
            "FS_LIMIT"
        );
        assert_eq!(
            read_dir_at(dir.path(), 3, 1024, 1).unwrap_err().code(),
            "FS_LIMIT"
        );
    }

    #[test]
    fn missing_paths_and_non_directories_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing");
        assert!(!exists_at(&missing).unwrap());
        assert_eq!(
            file_info_at(&missing, true).unwrap_err().code(),
            "FS_NOT_FOUND"
        );

        let file = dir.path().join("file");
        fs::write(&file, b"").unwrap();
        assert!(exists_at(&file).unwrap());
        assert_eq!(
            read_dir_at(&file, 10, 10, 10).unwrap_err().code(),
            "FS_NOT_DIRECTORY"
        );
        assert!(!exists_at(&file.join("child")).unwrap());
    }

    #[test]
    fn remove_reports_non_empty_directories_without_recursion() {
        let dir = tempfile::tempdir().unwrap();
        let tree = dir.path().join("tree");
        fs::create_dir(&tree).unwrap();
        fs::write(tree.join("file"), b"").unwrap();
        assert_eq!(remove_at(&tree, false).unwrap_err().code(), "FS_NOT_EMPTY");
        remove_at(&tree, true).unwrap();
        assert!(!tree.exists());
    }

    // O_NOCTTY is an open-time flag, not a status flag observable via
    // F_GETFL. Verify the flags without allocating a PTY or changing sessions.
    #[cfg(unix)]
    #[test]
    fn unix_data_opens_request_nonblocking_and_no_controlling_terminal() {
        let flags = unix_data_open_flags();
        assert_eq!(flags & libc::O_NONBLOCK, libc::O_NONBLOCK);
        assert_eq!(flags & libc::O_NOCTTY, libc::O_NOCTTY);
    }

    #[cfg(unix)]
    #[test]
    fn fifo_reads_and_writes_do_not_block_before_descriptor_validation() {
        use std::{ffi::CString, os::unix::ffi::OsStrExt, sync::mpsc, thread, time::Duration};

        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("fifo");
        let c_path = CString::new(fifo.as_os_str().as_bytes()).unwrap();
        // SAFETY: `c_path` is a NUL-terminated path in this test-owned tempdir.
        assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);

        let read_path = fifo.clone();
        let (send, receive) = mpsc::channel();
        thread::spawn(move || {
            let _ = send.send(read_bytes_at(&read_path, MAX_READ_BYTES));
        });
        assert_eq!(
            receive
                .recv_timeout(Duration::from_secs(2))
                .expect("FIFO read blocked")
                .unwrap_err()
                .code(),
            "FS_TYPE"
        );

        let write_path = fifo;
        let (send, receive) = mpsc::channel();
        thread::spawn(move || {
            let _ = send.send(write_bytes_at(&write_path, b"no", true, MAX_WRITE_BYTES));
        });
        assert_eq!(
            receive
                .recv_timeout(Duration::from_secs(2))
                .expect("FIFO write blocked")
                .unwrap_err()
                .code(),
            "FS_TYPE"
        );
    }

    #[cfg(windows)]
    #[test]
    fn directory_symlink_remove_is_explicitly_unsupported() {
        use std::os::windows::fs::symlink_dir;

        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("target");
        fs::create_dir(&live).unwrap();
        let missing = dir.path().join("missing");
        for (target, name) in [(&live, "live-link"), (&missing, "dangling-link")] {
            let link = dir.path().join(name);
            symlink_dir(target, &link).expect("Windows test requires directory symlink privileges");
            for recursive in [false, true] {
                assert_eq!(
                    remove_at(&link, recursive).unwrap_err().code(),
                    "FS_UNSUPPORTED"
                );
                let link_type = fs::symlink_metadata(&link).unwrap().file_type();
                assert!(link_type.is_symlink());
            }
        }
        assert!(live.is_dir());
    }
}
