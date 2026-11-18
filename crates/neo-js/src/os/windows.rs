use super::Metadata;

#[derive(Clone, Copy)]
enum Value {
    DisplayVersion,
    CurrentBuildNumber,
    EditionId,
}

/// Reads only the fixed, best-effort OS release values for this runtime.
#[cfg(windows)]
pub(super) fn metadata() -> Metadata {
    metadata_from(read_value)
}

#[cfg(windows)]
fn read_value(value: Value) -> Option<String> {
    use windows_sys::{
        Win32::{
            Foundation::ERROR_SUCCESS,
            System::Registry::{HKEY_LOCAL_MACHINE, REG_SZ, RRF_RT_REG_SZ, RegGetValueW},
        },
        w,
    };

    let name = match value {
        Value::DisplayVersion => w!("DisplayVersion"),
        Value::CurrentBuildNumber => w!("CurrentBuildNumber"),
        Value::EditionId => w!("EditionID"),
    };
    let mut data = [u16::MAX; 256];
    let mut bytes = std::mem::size_of_val(&data) as u32;
    let mut kind = 0;
    // SAFETY: The hive is a predefined handle; the key and value are static
    // NUL-terminated UTF-16 strings. The writable type and length outputs are
    // distinct from the aligned buffer, whose byte capacity is passed in.
    // RegGetValueW writes at most that capacity and keeps none of the pointers.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion"),
            name,
            RRF_RT_REG_SZ,
            &mut kind,
            data.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    decode_string(&data, bytes as usize, kind == REG_SZ)
}

/// Decodes the reported bytes of a successful REG_SZ read, including the
/// terminator that RegGetValueW can supply for an unterminated stored string.
fn decode_string(data: &[u16], bytes: usize, is_reg_sz: bool) -> Option<String> {
    if !is_reg_sz || !bytes.is_multiple_of(2) {
        return None;
    }
    let (&terminator, text) = data.get(..bytes / 2)?.split_last()?;
    if terminator != 0 {
        return None;
    }
    let value = String::from_utf16(text).ok()?;
    if value.trim().is_empty() || value.chars().any(char::is_control) {
        return None;
    }
    Some(value)
}

/// Prefers the display version; falls back to an explicitly labelled build.
fn metadata_from(mut read: impl FnMut(Value) -> Option<String>) -> Metadata {
    let version = read(Value::DisplayVersion).or_else(|| {
        let build = read(Value::CurrentBuildNumber)?;
        if build.is_empty() || !build.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        Some(format!("build {build}"))
    });
    Metadata {
        version,
        variant: read(Value::EditionId),
        ..Metadata::default()
    }
}

#[cfg(test)]
mod tests {
    use super::{Value, decode_string, metadata_from};

    // Codec and bounds fixtures for a registry response, independent of the host.
    #[test]
    fn accepts_only_bounded_well_formed_reg_sz_responses() {
        let data: Vec<_> = "Édition 🐧\0".encode_utf16().collect();
        assert_eq!(
            decode_string(&data, data.len() * 2, true).as_deref(),
            Some("Édition 🐧")
        );
        assert_eq!(decode_string(&data, data.len() * 2, false), None);
        for bytes in [0, 1, data.len() * 2 - 2, data.len() * 2 + 2] {
            assert_eq!(decode_string(&data, bytes, true), None);
        }
        for malformed in [
            &[0][..],
            &[32, 0],
            &[0xd800, 0],
            &[65, 0, 66, 0],
            &[65, 10, 0],
            &[65, 0, 0],
        ] {
            assert_eq!(decode_string(malformed, malformed.len() * 2, true), None);
        }
        assert_eq!(decode_string(&[65, 0, 66], 4, true).as_deref(), Some("A"));
    }

    // Precedence and availability with synthetic reads; no live HKLM keys needed.
    #[test]
    fn prefers_display_version_and_uses_only_decimal_builds_as_fallback() {
        let metadata = metadata_from(|name| match name {
            Value::DisplayVersion => Some("24H2".into()),
            Value::CurrentBuildNumber => Some("26100".into()),
            Value::EditionId => Some("Professional".into()),
        });
        assert_eq!(metadata.version.as_deref(), Some("24H2"));
        assert_eq!(metadata.variant.as_deref(), Some("Professional"));
        assert_eq!(metadata.codename, None);

        for (build, expected) in [
            (Some("26100"), Some("build 26100")),
            (Some(""), None),
            (Some("26100.1"), None),
            (Some("+26100"), None),
            (Some(" 26100"), None),
            (Some("２６１００"), None),
            (None, None),
        ] {
            let metadata = metadata_from(|name| match name {
                Value::CurrentBuildNumber => build.map(str::to_owned),
                _ => None,
            });
            assert_eq!(metadata.version.as_deref(), expected);
            assert_eq!(metadata.variant, None);
            assert_eq!(metadata.codename, None);
        }
    }
}
