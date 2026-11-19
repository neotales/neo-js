//! Synchronous Windows Registry access for `neo:win-registry`.
//!
//! The API is a set of free functions. Every call opens the key it needs and
//! drops the handle before returning, so a caller never owns a registry handle
//! and there is nothing to close. It is deliberately bounded: key paths and
//! value names have UTF-16 length limits, values are limited to 8 MiB, and
//! enumeration requires an entry limit.
//!
//! On Windows, the maintained Microsoft `windows-sys` bindings perform the
//! platform calls. Every query is sized and bounded here; no helper fetches a
//! registry value or enumeration buffer without a limit. On other hosts the
//! same argument validation runs, then every operation reports
//! `WINREGISTRY_UNSUPPORTED` rather than pretending to succeed. No registry
//! handle is ever opened on a non-Windows host.
//!
//! Validation, encoding, and the value-type rules are platform-independent so
//! they can be tested on any host. The platform module is the only code that
//! exists behind `cfg(windows)`.

use rquickjs::{
    Array, Ctx, Error as JsError, Function, Object, Persistent, Result as JsResult, TypedArray,
    Value,
    module::{Declarations, Exports, ModuleDef},
};

/// Maximum UTF-16 code units in a registry key path, including separators.
pub(crate) const MAX_PATH_UTF16: usize = 16_384;
/// Maximum UTF-16 code units in one registry key name component.
pub(crate) const MAX_KEY_COMPONENT_UTF16: usize = 255;
/// Maximum UTF-16 code units in a value name. Windows documents 16,383.
pub(crate) const MAX_VALUE_NAME_UTF16: usize = 16_383;
/// Maximum UTF-16 code units in a hive name accepted by this module.
pub(crate) const MAX_HIVE_UTF16: usize = 32;
/// Maximum bytes returned or written for one registry value.
pub(crate) const MAX_VALUE_BYTES: usize = 8 * 1024 * 1024;
/// Maximum UTF-16 code units in one string value, excluding its terminator.
pub(crate) const MAX_VALUE_UTF16: usize = MAX_VALUE_BYTES / 2 - 1;
/// Maximum entries returned by one enumeration call.
pub(crate) const MAX_ENUM_ENTRIES: u32 = 4096;
/// Maximum bounded retries when registry data grows between calls.
#[cfg_attr(not(windows), allow(dead_code))]
const MAX_QUERY_ATTEMPTS: u32 = 8;

/// Numeric registry value types exposed to JavaScript.
pub(crate) const TYPE_SZ: u32 = 1;
pub(crate) const TYPE_EXPAND_SZ: u32 = 2;
pub(crate) const TYPE_BINARY: u32 = 3;
pub(crate) const TYPE_DWORD: u32 = 4;
pub(crate) const TYPE_QWORD: u32 = 11;

/// A predefined registry root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Hive {
    ClassesRoot,
    CurrentUser,
    LocalMachine,
    Users,
    CurrentConfig,
}

/// Stable error categories. Each maps to one `WINREGISTRY_*` code.
///
/// The Windows-only categories are part of the documented JavaScript surface
/// even when this host is not Windows, so they are kept on every platform.
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ErrorCode {
    InvalidHive,
    InvalidPath,
    InvalidName,
    InvalidValue,
    Encoding,
    TypeMismatch,
    NotFound,
    AccessDenied,
    Limit,
    #[cfg(not(windows))]
    Unsupported,
    Io,
}

impl ErrorCode {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidHive => "WINREGISTRY_INVALID_HIVE",
            Self::InvalidPath => "WINREGISTRY_INVALID_PATH",
            Self::InvalidName => "WINREGISTRY_INVALID_NAME",
            Self::InvalidValue => "WINREGISTRY_INVALID_VALUE",
            Self::Encoding => "WINREGISTRY_ENCODING",
            Self::TypeMismatch => "WINREGISTRY_TYPE_MISMATCH",
            Self::NotFound => "WINREGISTRY_NOT_FOUND",
            Self::AccessDenied => "WINREGISTRY_ACCESS_DENIED",
            Self::Limit => "WINREGISTRY_LIMIT",
            #[cfg(not(windows))]
            Self::Unsupported => "WINREGISTRY_UNSUPPORTED",
            Self::Io => "WINREGISTRY_IO",
        }
    }

    /// A fixed message. It never includes a path, value name, or value data.
    pub(crate) const fn message(self) -> &'static str {
        match self {
            Self::InvalidHive => "the registry hive name is not supported",
            Self::InvalidPath => "the registry key path is invalid",
            Self::InvalidName => "the registry value name is invalid",
            Self::InvalidValue => "the registry value is invalid",
            Self::Encoding => "the registry value is not valid UTF-16 text",
            Self::TypeMismatch => "the registry value has a different type",
            Self::NotFound => "the registry key or value does not exist",
            Self::AccessDenied => "access to the registry key or value was denied",
            Self::Limit => "the registry operation exceeded a neo-js limit",
            #[cfg(not(windows))]
            Self::Unsupported => "Windows Registry access is not supported on this host",
            Self::Io => "the registry operation failed",
        }
    }
}

/// A fixed, input-free registry failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RegistryError {
    code: ErrorCode,
}

impl RegistryError {
    pub(crate) const fn new(code: ErrorCode) -> Self {
        Self { code }
    }

    pub(crate) const fn code(self) -> &'static str {
        self.code.as_str()
    }

    pub(crate) const fn message(self) -> &'static str {
        self.code.message()
    }
}

fn invalid_hive() -> RegistryError {
    RegistryError::new(ErrorCode::InvalidHive)
}

fn invalid_path() -> RegistryError {
    RegistryError::new(ErrorCode::InvalidPath)
}

fn invalid_name() -> RegistryError {
    RegistryError::new(ErrorCode::InvalidName)
}

fn invalid_value() -> RegistryError {
    RegistryError::new(ErrorCode::InvalidValue)
}

fn limit_error() -> RegistryError {
    RegistryError::new(ErrorCode::Limit)
}

fn type_mismatch() -> RegistryError {
    RegistryError::new(ErrorCode::TypeMismatch)
}

#[cfg(not(windows))]
fn unsupported() -> RegistryError {
    RegistryError::new(ErrorCode::Unsupported)
}

fn io_error() -> RegistryError {
    RegistryError::new(ErrorCode::Io)
}

impl RegistryError {
    #[cfg_attr(not(windows), allow(dead_code))]
    fn is_not_found(self) -> bool {
        self.code == ErrorCode::NotFound
    }
}

/// Reject an enumeration whose reported entry count exceeds the caller's limit
/// before any entry is fetched.
#[cfg_attr(not(windows), allow(dead_code))]
fn check_enum_count(count: u32, limit: u32) -> Result<(), RegistryError> {
    if count as usize > limit as usize {
        Err(limit_error())
    } else {
        Ok(())
    }
}

/// Reject a name longer than the cap for its registry object.
#[cfg_attr(not(windows), allow(dead_code))]
fn check_name_len(len: usize, max: usize) -> Result<(), RegistryError> {
    if len > max {
        Err(limit_error())
    } else {
        Ok(())
    }
}

/// Allocate a zero-filled byte buffer after checking its bound.
#[cfg_attr(not(windows), allow(dead_code))]
fn zeroed_vec(len: usize) -> Result<Vec<u8>, RegistryError> {
    let mut buffer = Vec::new();
    buffer.try_reserve_exact(len).map_err(|_| limit_error())?;
    buffer.resize(len, 0);
    Ok(buffer)
}

/// Grow a UTF-16 output buffer by one more bounded retry.
///
/// `needed` includes the terminating NUL for registry name queries. A request
/// above `max` is a limit failure; no growth is a no-progress I/O failure.
#[cfg_attr(not(windows), allow(dead_code))]
fn grow_wide_buffer(buffer: &mut Vec<u16>, needed: usize, max: usize) -> Result<(), RegistryError> {
    if needed > max {
        return Err(limit_error());
    }
    if needed <= buffer.len() {
        return Err(io_error());
    }
    buffer
        .try_reserve_exact(needed - buffer.len())
        .map_err(|_| limit_error())?;
    buffer.resize(needed, 0);
    Ok(())
}

/// One result from a bounded value query.
#[cfg_attr(not(windows), allow(dead_code))]
enum QueryProgress {
    Complete { kind: u32, used: usize },
    MoreData { needed: usize },
}

/// Read a value through a bounded retry loop.
///
/// The initial size comes from a separate metadata query. Every retry checks
/// the new required size against [`MAX_VALUE_BYTES`] before allocating. A
/// registry value that changes every call therefore cannot force unbounded
/// memory or an unbounded loop.
#[cfg_attr(not(windows), allow(dead_code))]
fn read_value_with<F>(
    kind: u32,
    initial_size: u32,
    mut query: F,
) -> Result<(u32, Vec<u8>), RegistryError>
where
    F: FnMut(&mut [u8]) -> Result<QueryProgress, RegistryError>,
{
    let size = initial_size as usize;
    if size > MAX_VALUE_BYTES {
        return Err(limit_error());
    }
    if size == 0 {
        return Ok((kind, Vec::new()));
    }
    let mut buffer = zeroed_vec(size)?;
    for _ in 0..MAX_QUERY_ATTEMPTS {
        match query(&mut buffer)? {
            QueryProgress::Complete { kind, used } => {
                if used > buffer.len() {
                    return Err(limit_error());
                }
                buffer.truncate(used);
                return Ok((kind, buffer));
            }
            QueryProgress::MoreData { needed } => {
                if needed > MAX_VALUE_BYTES {
                    return Err(limit_error());
                }
                if needed <= buffer.len() {
                    return Err(io_error());
                }
                buffer
                    .try_reserve_exact(needed - buffer.len())
                    .map_err(|_| limit_error())?;
                buffer.resize(needed, 0);
            }
        }
    }
    Err(io_error())
}

/// Parse a hive name. Short and long names are accepted case-insensitively.
pub(crate) fn parse_hive(value: &str) -> Result<Hive, RegistryError> {
    if value.is_empty() || value.encode_utf16().count() > MAX_HIVE_UTF16 {
        return Err(invalid_hive());
    }
    match value.to_ascii_uppercase().as_str() {
        "HKCR" | "HKEY_CLASSES_ROOT" => Ok(Hive::ClassesRoot),
        "HKCU" | "HKEY_CURRENT_USER" => Ok(Hive::CurrentUser),
        "HKLM" | "HKEY_LOCAL_MACHINE" => Ok(Hive::LocalMachine),
        "HKU" | "HKEY_USERS" => Ok(Hive::Users),
        "HKCC" | "HKEY_CURRENT_CONFIG" => Ok(Hive::CurrentConfig),
        _ => Err(invalid_hive()),
    }
}

/// Validate a relative key path. The empty path names the hive root.
///
/// Backslash is the only separator. A path may not contain an empty, `.` or
/// `..` component, a NUL, more than [`MAX_PATH_UTF16`] UTF-16 units in total,
/// or more than [`MAX_KEY_COMPONENT_UTF16`] units in one component.
pub(crate) fn validate_path(path: &str) -> Result<(), RegistryError> {
    if path.contains('\0') {
        return Err(invalid_path());
    }
    let units = path.encode_utf16().count();
    if units > MAX_PATH_UTF16 {
        return Err(invalid_path());
    }
    if path.is_empty() {
        return Ok(());
    }
    for component in path.split('\\') {
        if component.is_empty() || component == "." || component == ".." {
            return Err(invalid_path());
        }
        if component.encode_utf16().count() > MAX_KEY_COMPONENT_UTF16 {
            return Err(invalid_path());
        }
    }
    Ok(())
}

/// Validate a non-root key path for create or delete.
pub(crate) fn validate_key_path(path: &str) -> Result<(), RegistryError> {
    validate_path(path)?;
    if path.is_empty() {
        return Err(invalid_path());
    }
    Ok(())
}

/// Validate a value name. The empty name is the key's default value.
pub(crate) fn validate_value_name(name: &str) -> Result<(), RegistryError> {
    if name.contains('\0') || name.encode_utf16().count() > MAX_VALUE_NAME_UTF16 {
        return Err(invalid_name());
    }
    Ok(())
}

/// Encode a string as a NUL-terminated UTF-16LE `REG_SZ`/`REG_EXPAND_SZ` value.
///
/// An embedded NUL is rejected: writing it would silently make the stored
/// value shorter than the caller supplied.
pub(crate) fn encode_string(value: &str) -> Result<Vec<u8>, RegistryError> {
    if value.contains('\0') {
        return Err(invalid_value());
    }
    let units = value.encode_utf16().count();
    let bytes = units
        .checked_add(1)
        .and_then(|count| count.checked_mul(2))
        .ok_or_else(limit_error)?;
    if bytes > MAX_VALUE_BYTES {
        return Err(limit_error());
    }
    let mut out = Vec::new();
    out.try_reserve_exact(bytes).map_err(|_| limit_error())?;
    for unit in value.encode_utf16() {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    out.extend_from_slice(&[0, 0]);
    Ok(out)
}

/// Decode one `REG_SZ` or `REG_EXPAND_SZ` value.
///
/// The data must be an even number of bytes, contain a terminating UTF-16 NUL,
/// and contain only NUL units after that terminator. An odd length, a missing
/// terminator, data after the terminator, or an unpaired surrogate is rejected
/// rather than truncated or replaced.
pub(crate) fn decode_string(bytes: &[u8]) -> Result<String, RegistryError> {
    if bytes.len() > MAX_VALUE_BYTES {
        return Err(limit_error());
    }
    if bytes.is_empty() || !bytes.len().is_multiple_of(2) {
        return Err(invalid_value());
    }
    let mut units = Vec::new();
    units
        .try_reserve_exact(bytes.len() / 2)
        .map_err(|_| limit_error())?;
    for pair in bytes.chunks_exact(2) {
        units.push(u16::from_le_bytes([pair[0], pair[1]]));
    }
    let end = units
        .iter()
        .position(|&unit| unit == 0)
        .ok_or_else(invalid_value)?;
    if units[end..].iter().any(|&unit| unit != 0) {
        return Err(invalid_value());
    }
    String::from_utf16(&units[..end]).map_err(|_| RegistryError::new(ErrorCode::Encoding))
}

/// Parse the decimal text of an unsigned 64-bit value.
///
/// Only ASCII digits are accepted; a sign, whitespace, or a value above
/// `u64::MAX` is rejected. The length check happens before parsing.
pub(crate) fn parse_u64_decimal(value: &str) -> Result<u64, RegistryError> {
    if value.is_empty() || value.len() > 20 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid_value());
    }
    value.parse::<u64>().map_err(|_| invalid_value())
}

/// Read one raw value. Returns its numeric type and exact bytes.
fn read_value_op(hive: &str, path: &str, name: &str) -> Result<(u32, Vec<u8>), RegistryError> {
    let hive = parse_hive(hive)?;
    validate_path(path)?;
    validate_value_name(name)?;
    platform::read_value(hive, path, name)
}

fn read_typed_op(
    hive: &str,
    path: &str,
    name: &str,
    expected: u32,
) -> Result<Vec<u8>, RegistryError> {
    let (kind, bytes) = read_value_op(hive, path, name)?;
    if kind != expected {
        return Err(type_mismatch());
    }
    Ok(bytes)
}

fn write_value_op(
    hive: &str,
    path: &str,
    name: &str,
    kind: u32,
    bytes: Vec<u8>,
) -> Result<(), RegistryError> {
    let hive = parse_hive(hive)?;
    validate_path(path)?;
    validate_value_name(name)?;
    if bytes.len() > MAX_VALUE_BYTES {
        return Err(limit_error());
    }
    platform::write_value(hive, path, name, kind, bytes)
}

fn write_string_op(
    hive: &str,
    path: &str,
    name: &str,
    value: &str,
    kind: u32,
) -> Result<(), RegistryError> {
    let bytes = encode_string(value)?;
    write_value_op(hive, path, name, kind, bytes)
}

fn write_dword_op(hive: &str, path: &str, name: &str, value: f64) -> Result<(), RegistryError> {
    if !value.is_finite() || value.fract() != 0.0 || value < 0.0 || value > f64::from(u32::MAX) {
        return Err(invalid_value());
    }
    write_value_op(
        hive,
        path,
        name,
        TYPE_DWORD,
        (value as u32).to_le_bytes().to_vec(),
    )
}

fn write_qword_op(hive: &str, path: &str, name: &str, value: &str) -> Result<(), RegistryError> {
    let value = parse_u64_decimal(value)?;
    write_value_op(hive, path, name, TYPE_QWORD, value.to_le_bytes().to_vec())
}

fn create_key_op(hive: &str, path: &str) -> Result<bool, RegistryError> {
    let hive = parse_hive(hive)?;
    validate_key_path(path)?;
    platform::create_key(hive, path)
}

fn key_exists_op(hive: &str, path: &str) -> Result<bool, RegistryError> {
    let hive = parse_hive(hive)?;
    validate_path(path)?;
    platform::key_exists(hive, path)
}

fn delete_key_op(hive: &str, path: &str) -> Result<bool, RegistryError> {
    let hive = parse_hive(hive)?;
    validate_key_path(path)?;
    platform::delete_key(hive, path)
}

fn delete_value_op(hive: &str, path: &str, name: &str) -> Result<bool, RegistryError> {
    let hive = parse_hive(hive)?;
    validate_path(path)?;
    validate_value_name(name)?;
    platform::delete_value(hive, path, name)
}

fn list_subkeys_op(hive: &str, path: &str, limit: u32) -> Result<Vec<String>, RegistryError> {
    let hive = parse_hive(hive)?;
    validate_path(path)?;
    platform::list_subkeys(hive, path, limit)
}

fn list_values_op(hive: &str, path: &str, limit: u32) -> Result<Vec<(String, u32)>, RegistryError> {
    let hive = parse_hive(hive)?;
    validate_path(path)?;
    platform::list_values(hive, path, limit)
}

/// Validate an enumeration limit.
fn validate_limit(limit: u32) -> Result<u32, RegistryError> {
    if limit == 0 || limit > MAX_ENUM_ENTRIES {
        return Err(invalid_value());
    }
    Ok(limit)
}

#[cfg(windows)]
mod platform {
    use std::ptr;

    use windows_sys::Win32::{
        Foundation::{
            ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS,
            ERROR_PATH_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR,
        },
        Storage::FileSystem::DELETE,
        System::Registry::{
            HKEY, HKEY_CLASSES_ROOT, HKEY_CURRENT_CONFIG, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE,
            HKEY_USERS, KEY_READ, KEY_SET_VALUE, KEY_WRITE, REG_CREATED_NEW_KEY,
            REG_OPTION_NON_VOLATILE, RegCloseKey, RegCreateKeyExW, RegDeleteKeyExW,
            RegDeleteValueW, RegEnumKeyExW, RegEnumValueW, RegOpenKeyExW, RegQueryInfoKeyW,
            RegQueryValueExW, RegSetValueExW,
        },
    };

    use super::{
        ErrorCode, Hive, MAX_KEY_COMPONENT_UTF16, MAX_PATH_UTF16, MAX_VALUE_BYTES,
        MAX_VALUE_NAME_UTF16, QueryProgress, RegistryError, check_enum_count, check_name_len,
        enumerate_bounded, grow_wide_buffer, io_error, limit_error, read_value_with,
    };

    const MAX_ENUM_ATTEMPTS: u32 = 8;

    /// An owned registry handle. Predefined root handles are never wrapped.
    struct Key(HKEY);

    impl Drop for Key {
        fn drop(&mut self) {
            // SAFETY: `Key` is constructed only from a successful
            // RegOpenKeyExW or RegCreateKeyExW, and owns that handle once.
            unsafe {
                RegCloseKey(self.0);
            }
        }
    }

    impl Key {
        fn open(root: HKEY, path: &str, access: u32) -> Result<Self, RegistryError> {
            let wide = to_wide(path, MAX_PATH_UTF16)?;
            let mut handle: HKEY = ptr::null_mut();
            // SAFETY: `wide` is NUL-terminated and alive for the call; `handle`
            // is a valid out pointer. On success the handle is owned by Key.
            let status = unsafe { RegOpenKeyExW(root, wide.as_ptr(), 0, access, &mut handle) };
            if status == ERROR_SUCCESS {
                Ok(Self(handle))
            } else {
                Err(map_status(status))
            }
        }
    }

    pub(crate) fn is_available() -> bool {
        true
    }

    fn root(hive: Hive) -> HKEY {
        match hive {
            Hive::ClassesRoot => HKEY_CLASSES_ROOT,
            Hive::CurrentUser => HKEY_CURRENT_USER,
            Hive::LocalMachine => HKEY_LOCAL_MACHINE,
            Hive::Users => HKEY_USERS,
            Hive::CurrentConfig => HKEY_CURRENT_CONFIG,
        }
    }

    fn map_status(status: WIN32_ERROR) -> RegistryError {
        match status {
            ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => RegistryError::new(ErrorCode::NotFound),
            ERROR_ACCESS_DENIED => RegistryError::new(ErrorCode::AccessDenied),
            _ => RegistryError::new(ErrorCode::Io),
        }
    }

    /// Encode a bounded, NUL-terminated UTF-16 key path or value name.
    ///
    /// The loop stops at `max` units before allocating beyond that bound.
    fn to_wide(value: &str, max: usize) -> Result<Vec<u16>, RegistryError> {
        let mut wide = Vec::with_capacity(value.len().min(max).saturating_add(1));
        for unit in value.encode_utf16() {
            if wide.len() >= max {
                return Err(limit_error());
            }
            wide.push(unit);
        }
        wide.push(0);
        Ok(wide)
    }

    fn zeroed_wide(len: usize) -> Result<Vec<u16>, RegistryError> {
        let mut buffer = Vec::new();
        buffer.try_reserve_exact(len).map_err(|_| limit_error())?;
        buffer.resize(len, 0);
        Ok(buffer)
    }

    fn decode_wide(units: &[u16]) -> Result<String, RegistryError> {
        String::from_utf16(units).map_err(|_| RegistryError::new(ErrorCode::Encoding))
    }

    fn split_parent(path: &str) -> (&str, &str) {
        match path.rfind('\\') {
            Some(index) => (&path[..index], &path[index + 1..]),
            None => ("", path),
        }
    }

    struct KeyInfo {
        subkeys: u32,
        max_subkey: u32,
        values: u32,
        max_value_name: u32,
    }

    fn query_info(key: HKEY) -> Result<KeyInfo, RegistryError> {
        let mut info = KeyInfo {
            subkeys: 0,
            max_subkey: 0,
            values: 0,
            max_value_name: 0,
        };
        // SAFETY: `key` is a live handle. Only the required out parameters are
        // non-null; unused outputs are null as the API permits.
        let status = unsafe {
            RegQueryInfoKeyW(
                key,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
                &mut info.subkeys,
                &mut info.max_subkey,
                ptr::null_mut(),
                &mut info.values,
                &mut info.max_value_name,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        if status == ERROR_SUCCESS {
            Ok(info)
        } else {
            Err(map_status(status))
        }
    }

    fn enumerate_key_name(
        key: HKEY,
        index: u32,
        max_len: u32,
    ) -> Result<Option<String>, RegistryError> {
        let cap = MAX_KEY_COMPONENT_UTF16;
        if max_len as usize > cap {
            return Err(limit_error());
        }
        let mut buffer = zeroed_wide(max_len as usize + 1)?;
        for _ in 0..MAX_ENUM_ATTEMPTS {
            let mut len = buffer.len() as u32;
            // SAFETY: `key` is live and `buffer` is valid for `len` UTF-16
            // units. All optional outputs are null.
            let status = unsafe {
                RegEnumKeyExW(
                    key,
                    index,
                    buffer.as_mut_ptr(),
                    &mut len,
                    ptr::null(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                )
            };
            match status {
                ERROR_SUCCESS => {
                    check_name_len(len as usize, cap)?;
                    return Ok(Some(decode_wide(&buffer[..len as usize])?));
                }
                ERROR_NO_MORE_ITEMS => return Ok(None),
                ERROR_MORE_DATA => {
                    grow_wide_buffer(&mut buffer, len as usize, cap + 1)?;
                }
                other => return Err(map_status(other)),
            }
        }
        Err(io_error())
    }

    fn enumerate_value_name(
        key: HKEY,
        index: u32,
        max_len: u32,
    ) -> Result<Option<(String, u32)>, RegistryError> {
        let cap = MAX_VALUE_NAME_UTF16;
        if max_len as usize > cap {
            return Err(limit_error());
        }
        let mut buffer = zeroed_wide(max_len as usize + 1)?;
        for _ in 0..MAX_ENUM_ATTEMPTS {
            let mut len = buffer.len() as u32;
            let mut kind: u32 = 0;
            // SAFETY: `key` is live and `buffer` is valid for `len` UTF-16
            // units. Both data pointers are null, which the API permits and
            // which asks for the name and type without fetching value data.
            let status = unsafe {
                RegEnumValueW(
                    key,
                    index,
                    buffer.as_mut_ptr(),
                    &mut len,
                    ptr::null(),
                    &mut kind,
                    ptr::null_mut(),
                    ptr::null_mut(),
                )
            };
            match status {
                ERROR_SUCCESS => {
                    check_name_len(len as usize, cap)?;
                    return Ok(Some((decode_wide(&buffer[..len as usize])?, kind)));
                }
                ERROR_NO_MORE_ITEMS => return Ok(None),
                ERROR_MORE_DATA => {
                    grow_wide_buffer(&mut buffer, len as usize, cap + 1)?;
                }
                other => return Err(map_status(other)),
            }
        }
        Err(io_error())
    }

    pub(crate) fn create_key(hive: Hive, path: &str) -> Result<bool, RegistryError> {
        let wide = to_wide(path, MAX_PATH_UTF16)?;
        let mut handle: HKEY = ptr::null_mut();
        let mut disposition: u32 = 0;
        // SAFETY: `wide` is NUL-terminated and alive for the call. Optional
        // class and security pointers are null; the handle is owned by Key on
        // success.
        let status = unsafe {
            RegCreateKeyExW(
                root(hive),
                wide.as_ptr(),
                0,
                ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                ptr::null(),
                &mut handle,
                &mut disposition,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(map_status(status));
        }
        let _key = Key(handle);
        Ok(disposition == REG_CREATED_NEW_KEY)
    }

    pub(crate) fn key_exists(hive: Hive, path: &str) -> Result<bool, RegistryError> {
        match Key::open(root(hive), path, KEY_READ) {
            Ok(_) => Ok(true),
            Err(error) if error.is_not_found() => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub(crate) fn delete_key(hive: Hive, path: &str) -> Result<bool, RegistryError> {
        let (parent, leaf) = split_parent(path);
        let parent_key = if parent.is_empty() {
            None
        } else {
            match Key::open(root(hive), parent, DELETE) {
                Ok(key) => Some(key),
                Err(error) if error.is_not_found() => return Ok(false),
                Err(error) => return Err(error),
            }
        };
        let parent_handle = parent_key.as_ref().map_or_else(|| root(hive), |key| key.0);
        let leaf_wide = to_wide(leaf, MAX_KEY_COMPONENT_UTF16)?;
        // SAFETY: `parent_handle` is either a predefined root or a live opened
        // handle; `leaf_wide` is NUL-terminated and alive for the call.
        let status = unsafe { RegDeleteKeyExW(parent_handle, leaf_wide.as_ptr(), 0, 0) };
        if status == ERROR_SUCCESS {
            Ok(true)
        } else if status == ERROR_FILE_NOT_FOUND || status == ERROR_PATH_NOT_FOUND {
            Ok(false)
        } else {
            Err(map_status(status))
        }
    }

    pub(crate) fn delete_value(hive: Hive, path: &str, name: &str) -> Result<bool, RegistryError> {
        let key = Key::open(root(hive), path, KEY_SET_VALUE)?;
        let name_wide = to_wide(name, MAX_VALUE_NAME_UTF16)?;
        // SAFETY: `key` is live and `name_wide` is NUL-terminated and alive for
        // the call.
        let status = unsafe { RegDeleteValueW(key.0, name_wide.as_ptr()) };
        if status == ERROR_SUCCESS {
            Ok(true)
        } else if status == ERROR_FILE_NOT_FOUND {
            Ok(false)
        } else {
            Err(map_status(status))
        }
    }

    pub(crate) fn read_value(
        hive: Hive,
        path: &str,
        name: &str,
    ) -> Result<(u32, Vec<u8>), RegistryError> {
        let key = Key::open(root(hive), path, KEY_READ)?;
        let name_wide = to_wide(name, MAX_VALUE_NAME_UTF16)?;
        let mut kind: u32 = 0;
        let mut size: u32 = 0;
        // SAFETY: `key` and `name_wide` are valid. A null data pointer asks for
        // the required size only; no data buffer is read or written.
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                name_wide.as_ptr(),
                ptr::null(),
                &mut kind,
                ptr::null_mut(),
                &mut size,
            )
        };
        if status != ERROR_SUCCESS && status != ERROR_MORE_DATA {
            return Err(map_status(status));
        }
        if status == ERROR_MORE_DATA && size == 0 {
            return Err(io_error());
        }
        read_value_with(kind, size, |buffer| {
            let mut used = buffer.len() as u32;
            let mut call_kind: u32 = 0;
            // SAFETY: `buffer` is valid for `used` bytes; `used` is its length.
            // `kind` is an arbitrary numeric registry type and is not filtered.
            let status = unsafe {
                RegQueryValueExW(
                    key.0,
                    name_wide.as_ptr(),
                    ptr::null(),
                    &mut call_kind,
                    buffer.as_mut_ptr(),
                    &mut used,
                )
            };
            match status {
                ERROR_SUCCESS => Ok(QueryProgress::Complete {
                    kind: call_kind,
                    used: used as usize,
                }),
                ERROR_MORE_DATA => Ok(QueryProgress::MoreData {
                    needed: used as usize,
                }),
                other => Err(map_status(other)),
            }
        })
    }

    pub(crate) fn write_value(
        hive: Hive,
        path: &str,
        name: &str,
        kind: u32,
        bytes: Vec<u8>,
    ) -> Result<(), RegistryError> {
        if bytes.len() > MAX_VALUE_BYTES {
            return Err(limit_error());
        }
        let key = Key::open(root(hive), path, KEY_SET_VALUE)?;
        let name_wide = to_wide(name, MAX_VALUE_NAME_UTF16)?;
        let data = if bytes.is_empty() {
            ptr::null()
        } else {
            bytes.as_ptr()
        };
        // SAFETY: `key` and `name_wide` are valid. `data` is valid for
        // `bytes.len()` bytes, or null with a zero length.
        let status =
            unsafe { RegSetValueExW(key.0, name_wide.as_ptr(), 0, kind, data, bytes.len() as u32) };
        if status == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(map_status(status))
        }
    }

    pub(crate) fn list_subkeys(
        hive: Hive,
        path: &str,
        limit: u32,
    ) -> Result<Vec<String>, RegistryError> {
        let key = Key::open(root(hive), path, KEY_READ)?;
        let info = query_info(key.0)?;
        check_enum_count(info.subkeys, limit)?;
        enumerate_bounded(limit, |index| {
            enumerate_key_name(key.0, index, info.max_subkey)
        })
    }

    pub(crate) fn list_values(
        hive: Hive,
        path: &str,
        limit: u32,
    ) -> Result<Vec<(String, u32)>, RegistryError> {
        let key = Key::open(root(hive), path, KEY_READ)?;
        let info = query_info(key.0)?;
        check_enum_count(info.values, limit)?;
        enumerate_bounded(limit, |index| {
            enumerate_value_name(key.0, index, info.max_value_name)
        })
    }
}

#[cfg(any(windows, test))]
fn enumerate_bounded<T>(
    limit: u32,
    mut fetch: impl FnMut(u32) -> Result<Option<T>, RegistryError>,
) -> Result<Vec<T>, RegistryError> {
    let mut entries = Vec::new();
    for index in 0..limit {
        match fetch(index)? {
            Some(entry) => entries.push(entry),
            None => return Ok(entries),
        }
    }
    if fetch(limit)?.is_some() {
        return Err(limit_error());
    }
    Ok(entries)
}

#[cfg(not(windows))]
mod platform {
    use super::{Hive, RegistryError, unsupported};

    pub(crate) fn is_available() -> bool {
        false
    }

    pub(crate) fn create_key(_hive: Hive, _path: &str) -> Result<bool, RegistryError> {
        Err(unsupported())
    }

    pub(crate) fn key_exists(_hive: Hive, _path: &str) -> Result<bool, RegistryError> {
        Err(unsupported())
    }

    pub(crate) fn delete_key(_hive: Hive, _path: &str) -> Result<bool, RegistryError> {
        Err(unsupported())
    }

    pub(crate) fn delete_value(
        _hive: Hive,
        _path: &str,
        _name: &str,
    ) -> Result<bool, RegistryError> {
        Err(unsupported())
    }

    pub(crate) fn read_value(
        _hive: Hive,
        _path: &str,
        _name: &str,
    ) -> Result<(u32, Vec<u8>), RegistryError> {
        Err(unsupported())
    }

    pub(crate) fn write_value(
        _hive: Hive,
        _path: &str,
        _name: &str,
        _kind: u32,
        _bytes: Vec<u8>,
    ) -> Result<(), RegistryError> {
        Err(unsupported())
    }

    pub(crate) fn list_subkeys(
        _hive: Hive,
        _path: &str,
        _limit: u32,
    ) -> Result<Vec<String>, RegistryError> {
        Err(unsupported())
    }

    pub(crate) fn list_values(
        _hive: Hive,
        _path: &str,
        _limit: u32,
    ) -> Result<Vec<(String, u32)>, RegistryError> {
        Err(unsupported())
    }
}

/// Copy a JavaScript view into an owned, bounded buffer.
///
/// A detached or out-of-bounds view is rejected. The length limit is checked
/// before the copy is allocated.
fn copy_input(array: &TypedArray<'_, u8>) -> Result<Vec<u8>, RegistryError> {
    let bytes = match array.as_raw() {
        // SAFETY: no JavaScript runs between reading the pointer and copying
        // it below, and the returned Vec owns its own bytes afterwards.
        Some(raw) => unsafe { raw.as_ref() },
        None => return Err(invalid_value()),
    };
    if bytes.len() > MAX_VALUE_BYTES {
        return Err(limit_error());
    }
    let mut out = Vec::new();
    out.try_reserve_exact(bytes.len())
        .map_err(|_| limit_error())?;
    out.extend_from_slice(bytes);
    Ok(out)
}

/// Build the response object the JavaScript wrapper unwraps.
///
/// A success object is `{ ok: true, ...fields }`; a failure is
/// `{ ok: false, code, message }`. The object has a null prototype so a
/// script cannot intercept an own property through `Object.prototype`.
fn respond<'js, T, F>(
    ctx: &Ctx<'js>,
    result: Result<T, RegistryError>,
    apply: F,
) -> JsResult<Object<'js>>
where
    F: FnOnce(&Object<'js>, T) -> JsResult<()>,
{
    let object = Object::new_proto(ctx.clone(), None)?;
    match result {
        Ok(value) => {
            object.set("ok", true)?;
            apply(&object, value)?;
        }
        Err(error) => {
            object.set("ok", false)?;
            object.set("code", error.code())?;
            object.set("message", error.message())?;
        }
    }
    Ok(object)
}

fn is_available_js<'js>(ctx: Ctx<'js>) -> JsResult<Object<'js>> {
    respond(&ctx, Ok(platform::is_available()), |object, value| {
        object.set("value", value)
    })
}

fn create_key_js<'js>(ctx: Ctx<'js>, hive: String, path: String) -> JsResult<Object<'js>> {
    respond(&ctx, create_key_op(&hive, &path), |object, value| {
        object.set("value", value)
    })
}

fn key_exists_js<'js>(ctx: Ctx<'js>, hive: String, path: String) -> JsResult<Object<'js>> {
    respond(&ctx, key_exists_op(&hive, &path), |object, value| {
        object.set("value", value)
    })
}

fn delete_key_js<'js>(ctx: Ctx<'js>, hive: String, path: String) -> JsResult<Object<'js>> {
    respond(&ctx, delete_key_op(&hive, &path), |object, value| {
        object.set("value", value)
    })
}

fn delete_value_js<'js>(
    ctx: Ctx<'js>,
    hive: String,
    path: String,
    name: String,
) -> JsResult<Object<'js>> {
    respond(
        &ctx,
        delete_value_op(&hive, &path, &name),
        |object, value| object.set("value", value),
    )
}

fn list_subkeys_js<'js>(
    ctx: Ctx<'js>,
    hive: String,
    path: String,
    limit: u32,
) -> JsResult<Object<'js>> {
    let result = validate_limit(limit).and_then(|limit| list_subkeys_op(&hive, &path, limit));
    respond(&ctx, result, |object, names| {
        let array = Array::new(ctx.clone())?;
        for (index, name) in names.into_iter().enumerate() {
            array.as_object().prop(
                index as u32,
                rquickjs::object::Property::from(name)
                    .writable()
                    .enumerable()
                    .configurable(),
            )?;
        }
        object.set("value", array)
    })
}

fn list_values_js<'js>(
    ctx: Ctx<'js>,
    hive: String,
    path: String,
    limit: u32,
) -> JsResult<Object<'js>> {
    let result = validate_limit(limit).and_then(|limit| list_values_op(&hive, &path, limit));
    respond(&ctx, result, |object, values| {
        let array = Array::new(ctx.clone())?;
        for (index, (name, kind)) in values.into_iter().enumerate() {
            let entry = Object::new_proto(ctx.clone(), None)?;
            entry.set("name", name)?;
            entry.set("type", kind)?;
            array.as_object().prop(
                index as u32,
                rquickjs::object::Property::from(entry)
                    .writable()
                    .enumerable()
                    .configurable(),
            )?;
        }
        object.set("value", array)
    })
}

fn read_value_js<'js>(
    ctx: Ctx<'js>,
    hive: String,
    path: String,
    name: String,
) -> JsResult<Object<'js>> {
    respond(
        &ctx,
        read_value_op(&hive, &path, &name),
        |object, (kind, bytes)| {
            let value = Object::new_proto(ctx.clone(), None)?;
            value.set("type", kind)?;
            value.set("data", TypedArray::new(ctx.clone(), bytes)?)?;
            object.set("value", value)
        },
    )
}

fn read_string_js<'js>(
    ctx: Ctx<'js>,
    hive: String,
    path: String,
    name: String,
) -> JsResult<Object<'js>> {
    let result = read_value_op(&hive, &path, &name).and_then(|(kind, bytes)| {
        if kind != TYPE_SZ && kind != TYPE_EXPAND_SZ {
            return Err(type_mismatch());
        }
        decode_string(&bytes)
    });
    respond(&ctx, result, |object, value| object.set("value", value))
}

fn read_uint32_js<'js>(
    ctx: Ctx<'js>,
    hive: String,
    path: String,
    name: String,
) -> JsResult<Object<'js>> {
    let result = read_typed_op(&hive, &path, &name, TYPE_DWORD).and_then(|bytes| {
        let bytes: [u8; 4] = bytes.try_into().map_err(|_| invalid_value())?;
        Ok(u32::from_le_bytes(bytes))
    });
    respond(&ctx, result, |object, value| object.set("value", value))
}

fn read_uint64_js<'js>(
    ctx: Ctx<'js>,
    hive: String,
    path: String,
    name: String,
) -> JsResult<Object<'js>> {
    let result = read_typed_op(&hive, &path, &name, TYPE_QWORD).and_then(|bytes| {
        let bytes: [u8; 8] = bytes.try_into().map_err(|_| invalid_value())?;
        Ok(u64::from_le_bytes(bytes).to_string())
    });
    respond(&ctx, result, |object, value| object.set("value", value))
}

fn read_binary_js<'js>(
    ctx: Ctx<'js>,
    hive: String,
    path: String,
    name: String,
) -> JsResult<Object<'js>> {
    let result = read_typed_op(&hive, &path, &name, TYPE_BINARY);
    respond(&ctx, result, |object, bytes| {
        object.set("value", TypedArray::new(ctx.clone(), bytes)?)
    })
}

fn write_string_js<'js>(
    ctx: Ctx<'js>,
    hive: String,
    path: String,
    name: String,
    value: String,
) -> JsResult<Object<'js>> {
    let result = write_string_op(&hive, &path, &name, &value, TYPE_SZ);
    respond(&ctx, result, |_object, ()| Ok(()))
}

fn write_expand_string_js<'js>(
    ctx: Ctx<'js>,
    hive: String,
    path: String,
    name: String,
    value: String,
) -> JsResult<Object<'js>> {
    let result = write_string_op(&hive, &path, &name, &value, TYPE_EXPAND_SZ);
    respond(&ctx, result, |_object, ()| Ok(()))
}

fn write_uint32_js<'js>(
    ctx: Ctx<'js>,
    hive: String,
    path: String,
    name: String,
    value: f64,
) -> JsResult<Object<'js>> {
    let result = write_dword_op(&hive, &path, &name, value);
    respond(&ctx, result, |_object, ()| Ok(()))
}

fn write_uint64_js<'js>(
    ctx: Ctx<'js>,
    hive: String,
    path: String,
    name: String,
    value: String,
) -> JsResult<Object<'js>> {
    let result = write_qword_op(&hive, &path, &name, &value);
    respond(&ctx, result, |_object, ()| Ok(()))
}

fn write_binary_js<'js>(
    ctx: Ctx<'js>,
    hive: String,
    path: String,
    name: String,
    data: TypedArray<'js, u8>,
) -> JsResult<Object<'js>> {
    let result =
        copy_input(&data).and_then(|bytes| write_value_op(&hive, &path, &name, TYPE_BINARY, bytes));
    respond(&ctx, result, |_object, ()| Ok(()))
}

/// Persistent factory for the private JavaScript wrapper.
///
/// There is no native state: every operation opens and drops its own key. The
/// factory is created before user code so the wrapper captures the native
/// `Error`, `TypeError`, `Uint8Array`, and `BigInt` values.
#[derive(Default)]
pub(crate) struct WinRegistryState {
    pub(crate) factory: Option<Persistent<Function<'static>>>,
}

// WinRegistryState contains no borrowed JS value; its persistent handle is
// explicitly released before the context and runtime are dropped.
unsafe impl<'js> rquickjs::JsLifetime<'js> for WinRegistryState {
    type Changed<'to> = WinRegistryState;
}

/// The native `neo:win-registry` module.
pub(crate) struct WinRegistryModule;

const EXPORTS: [&str; 19] = [
    "isAvailable",
    "createKey",
    "keyExists",
    "deleteKey",
    "deleteValue",
    "listSubKeys",
    "listValues",
    "readValue",
    "readString",
    "readUint32",
    "readUint64",
    "readBinary",
    "writeString",
    "writeExpandString",
    "writeUint32",
    "writeUint64",
    "writeBinary",
    "ValueType",
    "HIVES",
];

impl ModuleDef for WinRegistryModule {
    fn declare<'js>(decl: &Declarations<'js>) -> JsResult<()> {
        for name in EXPORTS {
            decl.declare(name)?;
        }
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> JsResult<()> {
        use std::{cell::RefCell, rc::Rc};

        let state = Rc::clone(
            &*ctx
                .userdata::<Rc<RefCell<WinRegistryState>>>()
                .ok_or(JsError::Unknown)?,
        );
        let factory = state
            .borrow()
            .factory
            .as_ref()
            .cloned()
            .ok_or(JsError::Unknown)?
            .restore(ctx)?;
        // Module evaluation can happen after user code has modified prototypes.
        // Null prototypes prevent setters from capturing native callbacks or limits.
        let host = Object::new_proto(ctx.clone(), None)?;
        host.set("isAvailable", Function::new(ctx.clone(), is_available_js)?)?;
        host.set("createKey", Function::new(ctx.clone(), create_key_js)?)?;
        host.set("keyExists", Function::new(ctx.clone(), key_exists_js)?)?;
        host.set("deleteKey", Function::new(ctx.clone(), delete_key_js)?)?;
        host.set("deleteValue", Function::new(ctx.clone(), delete_value_js)?)?;
        host.set("listSubKeys", Function::new(ctx.clone(), list_subkeys_js)?)?;
        host.set("listValues", Function::new(ctx.clone(), list_values_js)?)?;
        host.set("readValue", Function::new(ctx.clone(), read_value_js)?)?;
        host.set("readString", Function::new(ctx.clone(), read_string_js)?)?;
        host.set("readUint32", Function::new(ctx.clone(), read_uint32_js)?)?;
        host.set("readUint64", Function::new(ctx.clone(), read_uint64_js)?)?;
        host.set("readBinary", Function::new(ctx.clone(), read_binary_js)?)?;
        host.set("writeString", Function::new(ctx.clone(), write_string_js)?)?;
        host.set(
            "writeExpandString",
            Function::new(ctx.clone(), write_expand_string_js)?,
        )?;
        host.set("writeUint32", Function::new(ctx.clone(), write_uint32_js)?)?;
        host.set("writeUint64", Function::new(ctx.clone(), write_uint64_js)?)?;
        host.set("writeBinary", Function::new(ctx.clone(), write_binary_js)?)?;
        let limits = Object::new_proto(ctx.clone(), None)?;
        limits.set("hive", MAX_HIVE_UTF16)?;
        limits.set("path", MAX_PATH_UTF16)?;
        limits.set("valueName", MAX_VALUE_NAME_UTF16)?;
        limits.set("valueBytes", MAX_VALUE_BYTES)?;
        limits.set("valueUnits", MAX_VALUE_UTF16)?;
        limits.set("enum", MAX_ENUM_ENTRIES)?;
        let api: Object = factory.call((host, limits))?;
        for name in EXPORTS {
            exports.export(name, api.get::<_, Value>(name)?)?;
        }
        Ok(())
    }
}

/// Build the private JavaScript wrapper in the fresh realm.
///
/// The wrapper performs JavaScript type checks (a wrong argument type is a
/// `TypeError`), converts `bigint` to and from decimal text, and converts the
/// host response into a thrown `Error` with a stable `code`.
pub(crate) fn factory<'js>(ctx: &Ctx<'js>) -> JsResult<Function<'js>> {
    ctx.eval(
        r#"(function() {
            'use strict';
            const NativeError = Error;
            const NativeTypeError = TypeError;
            const NativeBigInt = BigInt;
            const isSafeInteger = Number.isSafeInteger;
            const freeze = Object.freeze;
            const defineProperty = Object.defineProperty;
            const regexExec = Function.prototype.call.bind(RegExp.prototype.exec);
            const bigintToString = Function.prototype.call.bind(BigInt.prototype.toString);
            const typedArrayLength = Function.prototype.call.bind(
                Object.getOwnPropertyDescriptor(Object.getPrototypeOf(Uint8Array.prototype), 'length').get
            );
            const typedArrayTag = Function.prototype.call.bind(
                Object.getOwnPropertyDescriptor(Object.getPrototypeOf(Uint8Array.prototype), Symbol.toStringTag).get
            );
            const isView = ArrayBuffer.isView;
            const wellFormed =
                /^(?:[\u0000-\uD7FF\uE000-\uFFFF]|[\uD800-\uDBFF][\uDC00-\uDFFF])*$/;

            const HIVES = freeze(['HKCR', 'HKCU', 'HKLM', 'HKU', 'HKCC']);
            const ValueType = freeze({
                NONE: 0,
                SZ: 1,
                EXPAND_SZ: 2,
                BINARY: 3,
                DWORD: 4,
                DWORD_BIG_ENDIAN: 5,
                LINK: 6,
                MULTI_SZ: 7,
                RESOURCE_LIST: 8,
                FULL_RESOURCE_DESCRIPTOR: 9,
                RESOURCE_REQUIREMENTS_LIST: 10,
                QWORD: 11
            });

            return function(host, limits) {
                const MAX_ENUM_ENTRIES = limits.enum;
            function isWellFormed(value) {
                return regexExec(wellFormed, value) !== null;
            }

            function registryError(code, message) {
                const error = new NativeError(message);
                // ToPropertyDescriptor reads inherited "get" and "set" too.
                // A null prototype keeps user-installed accessors out.
                defineProperty(error, 'code', {
                    __proto__: null, value: code, writable: true, enumerable: true, configurable: true
                });
                return error;
            }

            function inputError(code, message) {
                throw registryError(code, message);
            }

            function requireString(value, what, maxUnits, code) {
                if (typeof value !== 'string') {
                    throw new NativeTypeError(what + ' must be a string');
                }
                if (value.length > maxUnits) {
                    inputError(code, what + ' exceeds the neo-js limit');
                }
                if (!isWellFormed(value)) {
                    inputError('WINREGISTRY_ENCODING', what + ' must be well-formed UTF-16');
                }
            }

            function requireHive(value) {
                requireString(value, 'hive', limits.hive, 'WINREGISTRY_INVALID_HIVE');
            }

            function requirePath(value) {
                requireString(value, 'path', limits.path, 'WINREGISTRY_INVALID_PATH');
            }

            function requireName(value) {
                requireString(value, 'name', limits.valueName, 'WINREGISTRY_INVALID_NAME');
            }

            function requireValueText(value) {
                requireString(value, 'value', limits.valueUnits, 'WINREGISTRY_LIMIT');
            }

            function requireBytes(value, what) {
                if (!isView(value) || typedArrayTag(value) !== 'Uint8Array') {
                    throw new NativeTypeError(what + ' must be a Uint8Array');
                }
                if (typedArrayLength(value) > limits.valueBytes) {
                    inputError('WINREGISTRY_LIMIT', what + ' exceeds the neo-js limit');
                }
            }

            function requireLimit(value) {
                if (typeof value !== 'number' || !isSafeInteger(value) ||
                    value < 1 || value > MAX_ENUM_ENTRIES) {
                    throw new NativeTypeError(
                        'limit must be an integer between 1 and ' + MAX_ENUM_ENTRIES
                    );
                }
            }

            function unwrap(response) {
                if (response.ok) return response;
                throw registryError(response.code, response.message);
            }

            function unwrapValue(response) {
                return unwrap(response).value;
            }

                return {
                    isAvailable() {
                        return unwrapValue(host.isAvailable());
                    },
                    createKey(hive, path) {
                        requireHive(hive);
                        requirePath(path);
                        return unwrapValue(host.createKey(hive, path));
                    },
                    keyExists(hive, path) {
                        requireHive(hive);
                        requirePath(path);
                        return unwrapValue(host.keyExists(hive, path));
                    },
                    deleteKey(hive, path) {
                        requireHive(hive);
                        requirePath(path);
                        return unwrapValue(host.deleteKey(hive, path));
                    },
                    deleteValue(hive, path, name) {
                        requireHive(hive);
                        requirePath(path);
                        requireName(name);
                        return unwrapValue(host.deleteValue(hive, path, name));
                    },
                    listSubKeys(hive, path, limit) {
                        requireHive(hive);
                        requirePath(path);
                        if (limit === undefined) limit = MAX_ENUM_ENTRIES;
                        else requireLimit(limit);
                        return unwrapValue(host.listSubKeys(hive, path, limit));
                    },
                    listValues(hive, path, limit) {
                        requireHive(hive);
                        requirePath(path);
                        if (limit === undefined) limit = MAX_ENUM_ENTRIES;
                        else requireLimit(limit);
                        return unwrapValue(host.listValues(hive, path, limit));
                    },
                    readValue(hive, path, name) {
                        requireHive(hive);
                        requirePath(path);
                        requireName(name);
                        return unwrapValue(host.readValue(hive, path, name));
                    },
                    readString(hive, path, name) {
                        requireHive(hive);
                        requirePath(path);
                        requireName(name);
                        return unwrapValue(host.readString(hive, path, name));
                    },
                    readUint32(hive, path, name) {
                        requireHive(hive);
                        requirePath(path);
                        requireName(name);
                        return unwrapValue(host.readUint32(hive, path, name));
                    },
                    readUint64(hive, path, name) {
                        requireHive(hive);
                        requirePath(path);
                        requireName(name);
                        return NativeBigInt(unwrapValue(host.readUint64(hive, path, name)));
                    },
                    readBinary(hive, path, name) {
                        requireHive(hive);
                        requirePath(path);
                        requireName(name);
                        return unwrapValue(host.readBinary(hive, path, name));
                    },
                    writeString(hive, path, name, value) {
                        requireHive(hive);
                        requirePath(path);
                        requireName(name);
                        requireValueText(value);
                        unwrap(host.writeString(hive, path, name, value));
                    },
                    writeExpandString(hive, path, name, value) {
                        requireHive(hive);
                        requirePath(path);
                        requireName(name);
                        requireValueText(value);
                        unwrap(host.writeExpandString(hive, path, name, value));
                    },
                    writeUint32(hive, path, name, value) {
                        requireHive(hive);
                        requirePath(path);
                        requireName(name);
                        if (typeof value !== 'number' || !isSafeInteger(value) ||
                            value < 0 || value > 4294967295) {
                            throw new NativeTypeError(
                                'value must be an integer between 0 and 4294967295'
                            );
                        }
                        unwrap(host.writeUint32(hive, path, name, value));
                    },
                    writeUint64(hive, path, name, value) {
                        requireHive(hive);
                        requirePath(path);
                        requireName(name);
                        if (typeof value !== 'bigint') {
                            throw new NativeTypeError('value must be a bigint');
                        }
                        if (value < 0n || value > 18446744073709551615n) {
                            inputError(
                                'WINREGISTRY_INVALID_VALUE',
                                'value is outside the QWORD range'
                            );
                        }
                        unwrap(host.writeUint64(hive, path, name, bigintToString(value)));
                    },
                    writeBinary(hive, path, name, data) {
                        requireHive(hive);
                        requirePath(path);
                        requireName(name);
                        requireBytes(data, 'data');
                        unwrap(host.writeBinary(hive, path, name, data));
                    },
                    ValueType,
                    HIVES
                };
            };
        })()"#,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enumeration_checks_one_entry_beyond_the_limit() {
        let mut calls = Vec::new();
        let err = enumerate_bounded(2, |index| {
            calls.push(index);
            Ok(Some(index))
        })
        .unwrap_err();
        assert_eq!(err.code(), "WINREGISTRY_LIMIT");
        assert_eq!(calls, [0, 1, 2]);
        assert_eq!(
            enumerate_bounded(2, |index| Ok((index < 2).then_some(index))).unwrap(),
            [0, 1]
        );
        assert_eq!(
            enumerate_bounded(2, |index| Ok((index < 1).then_some(index))).unwrap(),
            [0]
        );
        assert!(
            enumerate_bounded::<u32>(2, |index| {
                if index == 2 {
                    Err(io_error())
                } else {
                    Ok(Some(index))
                }
            })
            .is_err()
        );
    }

    #[test]
    fn hive_names_are_explicit_and_case_insensitive() {
        for (name, expected) in [
            ("HKCR", Hive::ClassesRoot),
            ("hkey_classes_root", Hive::ClassesRoot),
            ("HKCU", Hive::CurrentUser),
            ("HKEY_CURRENT_USER", Hive::CurrentUser),
            ("HKLM", Hive::LocalMachine),
            ("HKEY_LOCAL_MACHINE", Hive::LocalMachine),
            ("HKU", Hive::Users),
            ("HKEY_USERS", Hive::Users),
            ("HKCC", Hive::CurrentConfig),
            ("HKEY_CURRENT_CONFIG", Hive::CurrentConfig),
        ] {
            assert_eq!(parse_hive(name), Ok(expected), "{name}");
        }
        for name in ["", "HKPD", "HKEY_PERFORMANCE_DATA", "HKCR!", "HKEY_UNKNOWN"] {
            assert_eq!(parse_hive(name), Err(invalid_hive()), "{name}");
        }
    }

    #[test]
    fn paths_are_relative_and_components_are_validated() {
        assert_eq!(validate_path(""), Ok(()));
        assert_eq!(validate_path("Software"), Ok(()));
        assert_eq!(validate_path("Software\\MyApp\\Settings"), Ok(()));
        // Forward slash is an ordinary character, not a separator.
        assert_eq!(validate_path("Software/MyApp"), Ok(()));

        for path in [
            "\\Software",
            "Software\\",
            "Software\\\\App",
            "Software\\\\.\\App",
            "Software\\..\\App",
            "Software\0App",
        ] {
            assert_eq!(validate_path(path), Err(invalid_path()), "{path:?}");
        }

        let long_component = "a".repeat(MAX_KEY_COMPONENT_UTF16 + 1);
        assert_eq!(validate_path(&long_component), Err(invalid_path()));

        let long_path = "a\\".repeat(MAX_PATH_UTF16 / 2 + 1);
        assert_eq!(validate_path(&long_path), Err(invalid_path()));
    }

    #[test]
    fn key_paths_cannot_name_the_root() {
        assert_eq!(validate_key_path(""), Err(invalid_path()));
        assert_eq!(validate_key_path("Software\\MyApp"), Ok(()));
    }

    #[test]
    fn value_names_allow_the_default_and_reject_nul() {
        assert_eq!(validate_value_name(""), Ok(()));
        assert_eq!(validate_value_name("Theme"), Ok(()));
        assert_eq!(validate_value_name("A\0B"), Err(invalid_name()));
        assert_eq!(
            validate_value_name(&"a".repeat(MAX_VALUE_NAME_UTF16 + 1)),
            Err(invalid_name())
        );
    }

    #[test]
    fn strings_roundtrip_utf16_and_reject_embedded_nul() {
        for text in ["", "Theme", "Grüße 世界 🦀"] {
            let bytes = encode_string(text).unwrap();
            assert_eq!(decode_string(&bytes), Ok(text.to_owned()), "{text:?}");
        }
        assert_eq!(encode_string("A\0B"), Err(invalid_value()));
        assert_eq!(decode_string(&[0x41, 0x00]), Err(invalid_value()));
        assert_eq!(
            decode_string(&[0x00, 0x00, 0x41, 0x00]),
            Err(invalid_value())
        );
        assert_eq!(decode_string(&[0x00]), Err(invalid_value()));
        // Lone high surrogate.
        assert_eq!(
            decode_string(&[0x00, 0xd8, 0x00, 0x00]),
            Err(RegistryError::new(ErrorCode::Encoding))
        );
    }

    #[test]
    fn strings_with_data_after_the_terminator_are_rejected() {
        // "a", NUL, "b", NUL
        let bytes = [0x61, 0x00, 0x00, 0x00, 0x62, 0x00, 0x00, 0x00];
        assert_eq!(decode_string(&bytes), Err(invalid_value()));
    }

    #[test]
    fn qword_decimal_text_is_strict() {
        assert_eq!(parse_u64_decimal("0"), Ok(0));
        assert_eq!(
            parse_u64_decimal("9007199254740993"),
            Ok(9_007_199_254_740_993)
        );
        assert_eq!(parse_u64_decimal("18446744073709551615"), Ok(u64::MAX));
        for value in [
            "",
            "-1",
            "+1",
            " 1",
            "1.0",
            "18446744073709551616",
            "000000000000000000000",
        ] {
            assert_eq!(parse_u64_decimal(value), Err(invalid_value()), "{value:?}");
        }
    }

    #[test]
    fn result_objects_have_stable_codes_and_input_free_messages() {
        let error = RegistryError::new(ErrorCode::NotFound);
        assert_eq!(error.code(), "WINREGISTRY_NOT_FOUND");
        assert!(!error.message().contains("HKCU"));
        assert!(!error.message().contains("\\"));
    }

    #[test]
    fn bounded_value_read_retries_when_data_grows() {
        let mut calls = 0;
        let (kind, bytes) = read_value_with(0xFFFF, 4, |buffer| {
            calls += 1;
            if calls == 1 {
                assert_eq!(buffer.len(), 4);
                Ok(QueryProgress::MoreData { needed: 8 })
            } else {
                assert_eq!(buffer.len(), 8);
                buffer.copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
                Ok(QueryProgress::Complete {
                    kind: 0xFFFF,
                    used: 8,
                })
            }
        })
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(kind, 0xFFFF);
        assert_eq!(bytes, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn bounded_value_read_rejects_oversize_and_no_progress() {
        let error = read_value_with(1, (MAX_VALUE_BYTES + 1) as u32, |_| {
            unreachable!("oversized initial size must not be queried")
        })
        .unwrap_err();
        assert_eq!(error.code(), "WINREGISTRY_LIMIT");

        let error = read_value_with(1, 4, |_| {
            Ok(QueryProgress::MoreData {
                needed: MAX_VALUE_BYTES + 1,
            })
        })
        .unwrap_err();
        assert_eq!(error.code(), "WINREGISTRY_LIMIT");

        let error =
            read_value_with(1, 4, |_| Ok(QueryProgress::MoreData { needed: 4 })).unwrap_err();
        assert_eq!(error.code(), "WINREGISTRY_IO");

        let mut calls = 0;
        let error = read_value_with(1, 1, |_| {
            calls += 1;
            Ok(QueryProgress::MoreData { needed: 1 + calls })
        })
        .unwrap_err();
        assert_eq!(error.code(), "WINREGISTRY_IO");
        assert_eq!(calls, MAX_QUERY_ATTEMPTS as usize);
    }

    #[test]
    fn bounded_value_read_returns_any_numeric_type() {
        let (kind, bytes) = read_value_with(0xFFFF_FFFF, 0, |_| unreachable!()).unwrap();
        assert_eq!(kind, 0xFFFF_FFFF);
        assert!(bytes.is_empty());
    }

    #[test]
    fn enumeration_limits_and_name_caps_are_checked_before_fetch() {
        assert!(check_enum_count(MAX_ENUM_ENTRIES, MAX_ENUM_ENTRIES).is_ok());
        assert_eq!(
            check_enum_count(MAX_ENUM_ENTRIES + 1, MAX_ENUM_ENTRIES)
                .unwrap_err()
                .code(),
            "WINREGISTRY_LIMIT"
        );

        assert!(check_name_len(MAX_KEY_COMPONENT_UTF16, MAX_KEY_COMPONENT_UTF16).is_ok());
        assert_eq!(
            check_name_len(MAX_KEY_COMPONENT_UTF16 + 1, MAX_KEY_COMPONENT_UTF16)
                .unwrap_err()
                .code(),
            "WINREGISTRY_LIMIT"
        );
        assert!(check_name_len(MAX_VALUE_NAME_UTF16, MAX_VALUE_NAME_UTF16).is_ok());
        assert_eq!(
            check_name_len(MAX_VALUE_NAME_UTF16 + 1, MAX_VALUE_NAME_UTF16)
                .unwrap_err()
                .code(),
            "WINREGISTRY_LIMIT"
        );
    }

    #[test]
    fn name_buffer_growth_is_bounded_and_requires_progress() {
        let mut buffer = vec![0u16; 1];
        grow_wide_buffer(&mut buffer, 4, 8).unwrap();
        assert_eq!(buffer.len(), 4);

        assert_eq!(
            grow_wide_buffer(&mut buffer, 9, 8).unwrap_err().code(),
            "WINREGISTRY_LIMIT"
        );
        assert_eq!(
            grow_wide_buffer(&mut buffer, 4, 8).unwrap_err().code(),
            "WINREGISTRY_IO"
        );
    }
}
