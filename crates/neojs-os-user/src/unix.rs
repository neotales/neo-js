use std::{
    ffi::{CStr, c_char, c_int},
    mem::MaybeUninit,
    ptr,
};

use super::{Account, Backend, Lookup, ProcessIds, UserFailure};

pub(super) struct System;

const INITIAL_ACCOUNT_BYTES: usize = 4 * 1024;
const MAX_ACCOUNT_BYTES: usize = 256 * 1024;
const MAX_GROUPS: usize = 4096;

impl Backend for System {
    fn current_user() -> Lookup<Account> {
        with_real_account(|record, buffer| {
            Ok(Account {
                uid: record.pw_uid,
                primary_gid: record.pw_gid,
                username: text_field(buffer, record.pw_name)?,
                home_dir: text_field(buffer, record.pw_dir)?,
            })
        })
    }

    fn home_dir() -> Lookup<String> {
        with_real_account(|record, buffer| text_field(buffer, record.pw_dir))
    }

    fn process_ids() -> Lookup<ProcessIds> {
        // SAFETY: these getters need no arguments and read the caller's IDs.
        // They are separate syscalls, so this is not an atomic snapshot.
        Ok(unsafe {
            ProcessIds {
                real_uid: libc::getuid(),
                effective_uid: libc::geteuid(),
                real_gid: libc::getgid(),
                effective_gid: libc::getegid(),
            }
        })
    }

    fn account_group_ids() -> Lookup<Vec<u32>> {
        with_real_account(|record, buffer| {
            // The name is returned to libc as bytes, not exposed to JS.
            let name = field(buffer, record.pw_name)?;
            lookup_groups(record.pw_gid, |groups, count| {
                // SAFETY: name is NUL-terminated; groups and count are a
                // writable bounded buffer and its initialized in/out length.
                unsafe {
                    libc::getgrouplist(
                        name.as_ptr(),
                        record.pw_gid as GroupEntry,
                        groups.as_mut_ptr(),
                        count,
                    )
                }
            })
        })
    }
}

// Darwin's getgrouplist ABI uses signed int rather than gid_t. The cast
// preserves the gid_t bit pattern in both directions.
#[cfg(target_os = "macos")]
type GroupEntry = c_int;
#[cfg(target_os = "linux")]
type GroupEntry = libc::gid_t;

fn lookup_groups(
    primary: u32,
    lookup: impl FnOnce(&mut [GroupEntry], &mut c_int) -> c_int,
) -> Lookup<Vec<u32>> {
    // Use one full-capacity call. In particular a too-small buffer need not
    // return the exact required size on every libc.
    let mut groups = [0; MAX_GROUPS];
    let mut count = MAX_GROUPS as c_int;
    let status = lookup(&mut groups, &mut count);
    if count > MAX_GROUPS as c_int || (status == -1 && count == MAX_GROUPS as c_int) {
        return Err(UserFailure::Limit);
    }
    if status < 0 || count <= 0 {
        return Err(UserFailure::Lookup);
    }
    let count = count as usize;
    #[cfg(target_os = "linux")]
    if status as usize != count {
        return Err(UserFailure::Lookup);
    }
    #[cfg(target_os = "macos")]
    if status != 0 {
        return Err(UserFailure::Lookup);
    }
    let mut result = Vec::new();
    result
        .try_reserve_exact(count)
        .map_err(|_| UserFailure::Limit)?;
    #[cfg(target_os = "linux")]
    result.extend_from_slice(&groups[..count]);
    #[cfg(target_os = "macos")]
    result.extend(groups[..count].iter().map(|&group| group as u32));
    result.sort_unstable();
    result.dedup();
    if result.binary_search(&primary).is_err() {
        return Err(UserFailure::Lookup);
    }
    Ok(result)
}

fn with_real_account<T>(read: impl FnOnce(&libc::passwd, &[u8]) -> Lookup<T>) -> Lookup<T> {
    // SAFETY: getuid requires no arguments and reads the caller's real UID.
    let uid = unsafe { libc::getuid() };
    lookup_account(uid, read, |record, buffer, result| {
        // SAFETY: record is writable for passwd, buffer is writable for its
        // full length, and result is a writable pointer slot. Their storage
        // outlives this synchronous call.
        unsafe {
            libc::getpwuid_r(
                uid,
                record.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                result,
            )
        }
    })
}

fn lookup_account<T>(
    uid: libc::uid_t,
    read: impl FnOnce(&libc::passwd, &[u8]) -> Lookup<T>,
    mut lookup: impl FnMut(&mut MaybeUninit<libc::passwd>, &mut [u8], &mut *mut libc::passwd) -> c_int,
) -> Lookup<T> {
    let mut buffer = Vec::new();
    let mut size = INITIAL_ACCOUNT_BYTES;
    loop {
        buffer
            .try_reserve_exact(size - buffer.len())
            .map_err(|_| UserFailure::Limit)?;
        // Nonzero fill means an unterminated field cannot borrow a NUL from
        // unwritten space or from the previous attempt's record.
        buffer.fill(u8::MAX);
        buffer.resize(size, u8::MAX);
        let mut record = MaybeUninit::zeroed();
        let mut result = ptr::null_mut();
        match lookup(&mut record, &mut buffer, &mut result) {
            libc::ERANGE if size < MAX_ACCOUNT_BYTES => size *= 2,
            libc::ERANGE => return Err(UserFailure::Limit),
            libc::ENOENT => return Err(UserFailure::NotFound),
            0 if result.is_null() => return Err(UserFailure::NotFound),
            0 if result == record.as_mut_ptr() => {
                // SAFETY: storage was initialized to zero (valid for the
                // integer and raw-pointer fields in passwd); libc reports a
                // completed record in that storage. Never dereference result.
                let record = unsafe { record.assume_init() };
                if record.pw_uid != uid {
                    return Err(UserFailure::Lookup);
                }
                return read(&record, &buffer);
            }
            _ => return Err(UserFailure::Lookup),
        }
    }
}

fn field(buffer: &[u8], pointer: *const c_char) -> Lookup<&CStr> {
    // Inspect only the supplied buffer. Reject out-of-bounds fields without
    // ever dereferencing the raw pointer or scanning with unbounded strlen.
    let offset = pointer
        .addr()
        .checked_sub(buffer.as_ptr().addr())
        .ok_or(UserFailure::Lookup)?;
    let bytes = buffer.get(offset..).ok_or(UserFailure::Lookup)?;
    let field = CStr::from_bytes_until_nul(bytes).map_err(|_| UserFailure::Lookup)?;
    if field.is_empty() {
        return Err(UserFailure::Lookup);
    }
    Ok(field)
}

fn text_field(buffer: &[u8], pointer: *const c_char) -> Lookup<String> {
    let text = field(buffer, pointer)?
        .to_str()
        .map_err(|_| UserFailure::Encoding)?;
    let mut owned = String::new();
    owned
        .try_reserve_exact(text.len())
        .map_err(|_| UserFailure::Limit)?;
    owned.push_str(text);
    Ok(owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_and_failed_records_do_not_decode() {
        let decode = |_: &libc::passwd, _: &[u8]| -> Lookup<()> {
            panic!("an absent record must not be decoded")
        };
        assert_eq!(
            lookup_account(42, decode, |_, _, _| 0),
            Err(UserFailure::NotFound)
        );
        assert_eq!(
            lookup_account(42, decode, |_, _, _| libc::ENOENT),
            Err(UserFailure::NotFound)
        );
        assert_eq!(
            lookup_account(42, decode, |_, _, _| libc::EIO),
            Err(UserFailure::Lookup)
        );
        assert_eq!(
            lookup_account(42, decode, |_, _, output| {
                *output = ptr::dangling_mut();
                0
            }),
            Err(UserFailure::Lookup)
        );
    }

    #[test]
    fn account_buffer_grows_only_to_its_cap_and_does_not_reuse_bytes() {
        let mut sizes = Vec::new();
        let no_decode = |_: &libc::passwd, _: &[u8]| -> Lookup<()> { panic!("no record") };
        let result = lookup_account(42, no_decode, |_, buffer, _| {
            assert!(buffer.iter().all(|byte| *byte == u8::MAX));
            sizes.push(buffer.len());
            buffer[0] = 0;
            libc::ERANGE
        });
        assert_eq!(result, Err(UserFailure::Limit));
        assert_eq!(sizes, [4096, 8192, 16384, 32768, 65536, 131072, 262144]);
    }

    #[test]
    fn record_fields_are_bounded_owned_and_strict_utf8() {
        let read_name = |record: &libc::passwd, bytes: &[u8]| text_field(bytes, record.pw_name);
        let lookup = |uid, bytes: &'static [u8]| {
            lookup_account(42, read_name, |record, buffer, output| {
                buffer[..bytes.len()].copy_from_slice(bytes);
                // SAFETY: the production adapter initializes this record to
                // zero before invoking the injected lookup.
                let record = unsafe { record.assume_init_mut() };
                record.pw_uid = uid;
                record.pw_gid = 98;
                record.pw_name = buffer.as_mut_ptr().cast();
                *output = record;
                0
            })
        };
        assert_eq!(lookup(42, b"example-user\0"), Ok("example-user".into()));
        assert_eq!(lookup(100, b"example-user\0"), Err(UserFailure::Lookup));
        assert_eq!(lookup(42, b"example-\xff\0"), Err(UserFailure::Encoding));
        assert_eq!(lookup(42, b"example-user"), Err(UserFailure::Lookup));
        assert_eq!(lookup(42, b"\0"), Err(UserFailure::Lookup));
        let buffer = b"example\0";
        for pointer in [
            ptr::null(),
            buffer.as_ptr().wrapping_add(buffer.len()).cast(),
        ] {
            assert_eq!(field(buffer, pointer), Err(UserFailure::Lookup));
        }
    }

    // Linux returns the count on success; Darwin returns zero.
    fn success(count: c_int) -> c_int {
        if cfg!(target_os = "macos") { 0 } else { count }
    }

    #[test]
    fn account_groups_include_primary_gid_and_are_sorted_and_unique() {
        let primary = u32::MAX - 1;
        let groups = lookup_groups(primary, |groups, count| {
            assert_eq!(*count as usize, groups.len());
            groups[..4].copy_from_slice(&[primary as GroupEntry, 6, 3, 6]);
            *count = 4;
            success(*count)
        });
        assert_eq!(groups, Ok(vec![3, 6, primary]));
        let groups = lookup_groups(80, |groups, count| {
            groups[0] = 81;
            *count = 1;
            success(*count)
        });
        assert_eq!(groups, Err(UserFailure::Lookup));
    }

    #[test]
    fn invalid_group_counts_never_return_partial_results() {
        assert_eq!(
            lookup_groups(13, |_, count| {
                *count += 1;
                -1
            }),
            Err(UserFailure::Limit)
        );
        assert_eq!(lookup_groups(13, |_, _| -1), Err(UserFailure::Limit));
        assert_eq!(
            lookup_groups(13, |_, count| {
                *count = -1;
                success(*count)
            }),
            Err(UserFailure::Lookup)
        );
        assert_eq!(
            lookup_groups(13, |groups, count| {
                groups.fill(13);
                success(*count)
            }),
            Ok(vec![13])
        );
    }
}
