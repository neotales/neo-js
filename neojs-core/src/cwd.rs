//! A runtime-local working directory.
//!
//! neo-js never calls `std::env::set_current_dir`: that would mutate the
//! working directory for the whole host process, including every other
//! [`crate::NeoJs`] instance and every unrelated thread. Instead each runtime
//! owns this value and resolves relative filesystem, module-entry, fetch, hash,
//! and child-process paths against it. Relative imports use a module's
//! declaration-time base, not this mutable value. Absolute paths are passed
//! through unchanged.
//!
//! The value starts as the host process working directory when the runtime is
//! constructed. If the host directory is unavailable or is not valid Unicode,
//! [`WorkingDirectory::text`] and relative path resolution report an error
//! rather than inventing a path or falling back to the mutable host directory.
//! Relative imports with an absolute, captured module base are unaffected.
//! Absolute paths continue to work. An absolute `neo:process.chdir` can
//! establish a runtime directory; a relative one cannot while the value is
//! unavailable. Changing it only changes this runtime. Windows drive-relative
//! Drive-relative paths such as `C:child` are rejected on Windows: their
//! meaning depends on a per-drive process directory and they do not resolve
//! relative to this runtime's directory. Other Windows verbatim-path behavior
//! is unverified.

use std::path::{Path, PathBuf};

/// A failure from the runtime working-directory state.
///
/// Messages are fixed text. They never include the supplied path or an
/// operating-system error string, either of which could contain a path or a
/// secret that the script supplied.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CwdError {
    pub(crate) code: &'static str,
    pub(crate) message: &'static str,
}

impl CwdError {
    pub(crate) fn unavailable() -> Self {
        Self {
            code: "PROCESS_CWD",
            message: "the runtime working directory is unavailable",
        }
    }

    fn chdir() -> Self {
        Self {
            code: "PROCESS_CHDIR",
            message: "cannot change the runtime working directory",
        }
    }

    #[cfg(windows)]
    fn drive_relative() -> Self {
        Self {
            code: "PROCESS_PATH",
            message: "drive-relative paths are not supported",
        }
    }
}

impl std::fmt::Display for CwdError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message)
    }
}

/// The working directory of one runtime. It is not process-global.
#[derive(Debug)]
pub(crate) struct WorkingDirectory {
    current: Option<PathBuf>,
}

impl WorkingDirectory {
    /// Capture the host working directory without changing it.
    ///
    /// A non-Unicode host path is treated exactly like an unavailable one:
    /// every relative operation that needs the runtime directory, and every
    /// child process, fails explicitly instead of using a path JavaScript
    /// cannot represent consistently.
    pub(crate) fn new() -> Self {
        Self {
            current: std::env::current_dir()
                .ok()
                .filter(|path| path.to_str().is_some()),
        }
    }

    /// Construct an unavailable directory for deterministic unit tests.
    #[cfg(test)]
    pub(crate) fn unavailable() -> Self {
        Self { current: None }
    }

    /// Inject a path for deterministic unit tests, including a non-Unicode one.
    #[cfg(test)]
    pub(crate) fn injected(path: Option<PathBuf>) -> Self {
        Self { current: path }
    }

    /// The usable directory. A stored non-Unicode path is unavailable even if
    /// a test or future constructor bypasses [`Self::new`].
    fn valid_current(&self) -> Option<&Path> {
        self.current
            .as_deref()
            .filter(|path| path.to_str().is_some())
    }

    /// Resolve a script-supplied path against this runtime's directory.
    ///
    /// The path is not canonicalized and the filesystem is not consulted, so
    /// callers keep their existing symlink and error behavior. Absolute paths
    /// work even when the runtime directory is unavailable. A relative path
    /// reports [`CwdError::unavailable`] rather than falling back to the
    /// mutable host working directory.
    pub(crate) fn resolve(&self, path: &str) -> Result<PathBuf, CwdError> {
        self.resolve_path(Path::new(path))
    }

    /// Resolve an already-parsed path against this runtime's directory.
    pub(crate) fn resolve_path(&self, path: &Path) -> Result<PathBuf, CwdError> {
        #[cfg(windows)]
        if is_drive_relative(path) {
            return Err(CwdError::drive_relative());
        }
        if path.is_absolute() {
            Ok(path.to_path_buf())
        } else {
            self.valid_current()
                .map(|current| current.join(path))
                .ok_or_else(CwdError::unavailable)
        }
    }

    /// Return the current directory for a child process, if it is available.
    /// `None` means the runtime directory is unavailable; callers must report
    /// an error rather than letting a child inherit the host directory.
    pub(crate) fn current_path(&self) -> Option<PathBuf> {
        self.valid_current().map(Path::to_path_buf)
    }

    /// Return the current directory as text for `neo:process.cwd()`.
    pub(crate) fn text(&self) -> Result<String, CwdError> {
        self.valid_current()
            .and_then(Path::to_str)
            .map(str::to_owned)
            .ok_or_else(CwdError::unavailable)
    }

    /// Change only this runtime's working directory.
    ///
    /// The target must exist, be a directory, and have a Unicode path. A
    /// canonical absolute path is stored so repeated `cwd()` calls are stable.
    /// An absolute target can establish the value when it was unavailable; a
    /// relative target then reports [`CwdError::unavailable`] without falling
    /// back to the host directory. A failure leaves the previous value
    /// untouched.
    pub(crate) fn chdir(&mut self, directory: &str) -> Result<(), CwdError> {
        if directory.is_empty() {
            return Err(CwdError::chdir());
        }
        let target = self.resolve(directory)?;
        let canonical = std::fs::canonicalize(&target).map_err(|_| CwdError::chdir())?;
        let metadata = std::fs::metadata(&canonical).map_err(|_| CwdError::chdir())?;
        if !metadata.is_dir() || canonical.to_str().is_none() {
            return Err(CwdError::chdir());
        }
        self.current = Some(canonical);
        Ok(())
    }
}

/// A `C:child` path has a drive prefix but no root and is relative to the
/// process's per-drive current directory, which is outside this runtime's cwd.
#[cfg(windows)]
pub(crate) fn is_drive_relative(path: &Path) -> bool {
    use std::path::{Component, Prefix};

    !path.has_root()
        && matches!(
            path.components().next(),
            Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_))
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chdir_changes_only_this_value_and_keeps_it_on_failure() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let mut directory = WorkingDirectory {
            current: Some(first.path().to_path_buf()),
        };
        assert_eq!(directory.text().unwrap(), first.path().to_str().unwrap());

        directory.chdir(second.path().to_str().unwrap()).unwrap();
        assert_eq!(directory.text().unwrap(), second.path().to_str().unwrap());

        assert!(directory.chdir("neo-js-no-such-directory").is_err());
        assert_eq!(directory.text().unwrap(), second.path().to_str().unwrap());
        assert!(directory.chdir("").is_err());
        assert_eq!(directory.text().unwrap(), second.path().to_str().unwrap());
    }

    #[test]
    fn relative_paths_use_the_runtime_directory() {
        let dir = tempfile::tempdir().unwrap();
        let working = WorkingDirectory {
            current: Some(dir.path().to_path_buf()),
        };
        assert_eq!(
            working.resolve("file.txt").unwrap(),
            dir.path().join("file.txt")
        );
        let absolute = dir.path().join("absolute.txt");
        assert_eq!(
            working.resolve(absolute.to_str().unwrap()).unwrap(),
            absolute.clone()
        );
    }

    #[test]
    fn unavailable_directory_is_reported_without_inventing_a_path() {
        let working = WorkingDirectory { current: None };
        assert_eq!(working.text().unwrap_err().code, "PROCESS_CWD");
        assert_eq!(
            working.resolve("relative.txt").unwrap_err().code,
            "PROCESS_CWD"
        );

        let dir = tempfile::tempdir().unwrap();
        let absolute = dir.path().join("absolute.txt");
        assert_eq!(
            working.resolve(absolute.to_str().unwrap()).unwrap(),
            absolute
        );
    }

    #[test]
    fn unavailable_constructor_is_a_deterministic_seam() {
        let working = WorkingDirectory::unavailable();
        assert_eq!(working.text().unwrap_err().code, "PROCESS_CWD");
    }

    #[cfg(windows)]
    #[test]
    fn drive_relative_paths_are_rejected_but_rooted_paths_are_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let working = WorkingDirectory {
            current: Some(dir.path().to_path_buf()),
        };
        assert_eq!(
            working.resolve("C:child.txt").unwrap_err().code,
            "PROCESS_PATH"
        );
        assert_eq!(working.resolve("C:").unwrap_err().code, "PROCESS_PATH");
        assert!(working.resolve(r"C:\child.txt").unwrap().is_absolute());
    }
    #[cfg(unix)]
    #[test]
    fn injected_non_unicode_path_is_unavailable_for_relative_operations() {
        use std::os::unix::ffi::OsStringExt;

        let non_unicode = PathBuf::from(std::ffi::OsString::from_vec(
            b"/tmp/neo-js-non-unicode-\xff".to_vec(),
        ));
        let working = WorkingDirectory::injected(Some(non_unicode.clone()));
        assert_eq!(working.text().unwrap_err().code, "PROCESS_CWD");
        assert_eq!(
            working.resolve("relative.txt").unwrap_err().code,
            "PROCESS_CWD"
        );
        assert_eq!(working.current_path(), None);

        let absolute = PathBuf::from("/tmp/absolute.txt");
        assert_eq!(working.resolve_path(&absolute).unwrap(), absolute);
    }

    #[cfg(unix)]
    #[test]
    fn chdir_cannot_establish_a_non_unicode_canonical_target() {
        use std::os::unix::ffi::OsStringExt;

        let root = tempfile::tempdir().unwrap();
        let bad_name = std::ffi::OsString::from_vec(b"bad-\xff".to_vec());
        std::fs::create_dir(root.path().join(&bad_name)).unwrap();
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&bad_name, &link).unwrap();

        let mut working = WorkingDirectory::injected(Some(root.path().to_path_buf()));
        let error = working.chdir(link.to_str().unwrap()).unwrap_err();
        assert_eq!(error.code, "PROCESS_CHDIR");
        assert_eq!(working.text().unwrap(), root.path().to_str().unwrap());
    }
}
