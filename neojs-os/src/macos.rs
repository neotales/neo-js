#[cfg(target_os = "macos")]
use super::Metadata;

/// Reads the product version if this macOS host provides the fixed sysctl.
#[cfg(target_os = "macos")]
pub(super) fn metadata() -> Metadata {
    let mut data = [u8::MAX; 64];
    let mut bytes = data.len();
    // SAFETY: The name is a static NUL-terminated C string. The output buffer
    // is writable for the input byte count, and the length pointer is writable
    // and distinct from it. A null new-value pointer with zero length requests
    // no write to the sysctl. The call retains none of these pointers.
    let status = unsafe {
        libc::sysctlbyname(
            c"kern.osproductversion".as_ptr(),
            data.as_mut_ptr().cast(),
            &mut bytes,
            std::ptr::null_mut(),
            0,
        )
    };
    if status != 0 {
        return Metadata::default();
    }
    Metadata {
        version: decode_product_version(&data, bytes).map(str::to_owned),
        ..Metadata::default()
    }
}

/// Accepts only a bounded NUL-terminated dotted numeric product version.
fn decode_product_version(data: &[u8], bytes: usize) -> Option<&str> {
    let text = data.get(..bytes)?.strip_suffix(&[0])?;
    let version = std::str::from_utf8(text).ok()?;
    if !version.contains('.')
        || !version
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return None;
    }
    Some(version)
}

#[cfg(test)]
mod tests {
    use super::decode_product_version;

    // Codec and bounds fixtures for a sysctl response, independent of the host.
    #[test]
    fn accepts_only_bounded_terminated_dotted_product_versions() {
        for (data, expected) in [
            (&b"14.3\0"[..], "14.3"),
            (b"14.3.1\0", "14.3.1"),
            (b"26.0.0\0", "26.0.0"),
        ] {
            assert_eq!(decode_product_version(data, data.len()), Some(expected));
            assert_eq!(decode_product_version(data, data.len() + 1), None);
            assert_eq!(decode_product_version(data, data.len() - 1), None);
        }
        for data in [
            &b""[..],
            b"\0",
            b"14\0",
            b".14\0",
            b"14.\0",
            b"14..3\0",
            b" 14.3\0",
            b"14.3 beta\0",
            b"14.3\0extra\0",
            b"14.\xff\0",
            "14.３\0".as_bytes(),
        ] {
            assert_eq!(decode_product_version(data, data.len()), None);
        }
        assert_eq!(decode_product_version(b"14.3\0extra", 5), Some("14.3"));
    }
}
