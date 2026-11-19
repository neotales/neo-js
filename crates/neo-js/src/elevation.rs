//! Read the host process's elevation status without changing its authority.

/// A failed or unsupported elevation query, with no OS-specific details.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ElevationError {
    pub(crate) code: &'static str,
    pub(crate) message: &'static str,
}

#[cfg(any(windows, test))]
pub(crate) const QUERY_FAILED: ElevationError = ElevationError {
    code: "PROCESS_ELEVATION",
    message: "cannot query the current process elevation",
};

#[cfg(any(not(any(unix, windows)), test))]
pub(crate) const UNSUPPORTED: ElevationError = ElevationError {
    code: "PROCESS_ELEVATION_UNSUPPORTED",
    message: "process elevation is not supported on this platform",
};

/// Read the current effective UID and report whether it is zero.
#[cfg(unix)]
pub(crate) fn is_elevated() -> Result<bool, ElevationError> {
    Ok(from_effective_uid(|| {
        // SAFETY: geteuid takes no arguments and is infallible.
        unsafe { libc::geteuid() }
    }))
}

#[cfg(unix)]
fn from_effective_uid(read_uid: impl FnOnce() -> libc::uid_t) -> bool {
    read_uid() == 0
}

/// Read the current process's primary token, not any thread impersonation token.
#[cfg(windows)]
pub(crate) fn is_elevated() -> Result<bool, ElevationError> {
    use std::{
        os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
        ptr,
    };

    use windows_sys::Win32::{
        Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };

    from_process_token(
        || {
            let mut handle = ptr::null_mut();
            // SAFETY: the current-process pseudo-handle is valid and `handle`
            // is writable. Only the returned token handle must be closed.
            let opened = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle) };
            if opened == 0 {
                return None;
            }
            // SAFETY: successful OpenProcessToken transfers ownership of a
            // real token handle that OwnedHandle releases with CloseHandle.
            Some(unsafe { OwnedHandle::from_raw_handle(handle) })
        },
        |token| {
            let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
            let size = size_of::<TOKEN_ELEVATION>() as u32;
            let mut returned = 0;
            // SAFETY: the token stays open for the call. Both out-pointers are
            // writable and `size` is the exact capacity of `elevation`.
            let queried = unsafe {
                GetTokenInformation(
                    token.as_raw_handle(),
                    TokenElevation,
                    ptr::from_mut(&mut elevation).cast(),
                    size,
                    &mut returned,
                )
            };
            (queried != 0 && returned == size).then_some(elevation.TokenIsElevated)
        },
    )
}

/// Keep the owned token live during the query and drop it on every return path.
#[cfg(any(windows, test))]
fn from_process_token<Token>(
    open: impl FnOnce() -> Option<Token>,
    query: impl FnOnce(&Token) -> Option<u32>,
) -> Result<bool, ElevationError> {
    let token = open().ok_or(QUERY_FAILED)?;
    query(&token)
        .map(|elevated| elevated != 0)
        .ok_or(QUERY_FAILED)
}

/// Report unsupported hosts explicitly, rather than guessing their status.
#[cfg(not(any(unix, windows)))]
pub(crate) fn is_elevated() -> Result<bool, ElevationError> {
    Err(UNSUPPORTED)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[cfg(unix)]
    #[test]
    fn unix_elevation_uses_injected_effective_uid() {
        assert!(from_effective_uid(|| 0));
        assert!(!from_effective_uid(|| 1000));
    }

    #[test]
    fn windows_token_elevation_uses_the_query_and_releases_the_token() {
        struct Token<'a>(&'a Cell<bool>);

        impl Drop for Token<'_> {
            fn drop(&mut self) {
                self.0.set(true);
            }
        }

        for (reply, expected) in [
            (Some(0), Ok(false)),
            (Some(1), Ok(true)),
            (Some(7), Ok(true)),
            (None, Err(QUERY_FAILED)),
        ] {
            let closed = Cell::new(false);
            let result = from_process_token(
                || Some(Token(&closed)),
                |token| {
                    assert!(!token.0.get());
                    reply
                },
            );
            assert_eq!(result, expected);
            assert!(closed.get());
        }

        assert_eq!(
            from_process_token(|| None::<Token<'_>>, |_| panic!("query after open failure")),
            Err(QUERY_FAILED)
        );
    }
}
