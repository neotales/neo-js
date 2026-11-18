//! Shared, bounded conversion of JavaScript strings for text parsers.
//!
//! A primitive JavaScript string is UTF-16. Converting it to UTF-8 through a
//! host binding can silently replace an unpaired surrogate, so the parsers use
//! QuickJS's UTF-16 export directly, enforce the source size limit before
//! allocating, and report the first unpaired surrogate or NUL as a stable code
//! and one-based location. No JavaScript callback is invoked: the value is
//! checked as a primitive string first, and array-index/prototype properties
//! are not consulted for the exotic string length.

use rquickjs::{Ctx, Result as JsResult, Value, qjs};

/// The first invalid character in decoded source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct InvalidText {
    /// Stable module-specific code, such as `DOTENV_ENCODING`.
    pub(crate) code: &'static str,
    /// One-based line in the decoded source.
    pub(crate) line: u32,
    /// One-based column in Unicode scalar values, not bytes or UTF-16 units.
    pub(crate) column: u32,
}

/// Read a primitive string's UTF-16 length through QuickJS's own special-case
/// for string length. No JavaScript hook runs.
pub(crate) fn string_length<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> JsResult<usize> {
    let raw = unsafe {
        qjs::JS_GetPropertyStr(ctx.as_raw().as_ptr(), value.as_raw(), c"length".as_ptr())
    };
    if unsafe { qjs::JS_IsException(raw) } {
        unsafe { qjs::JS_FreeValue(ctx.as_raw().as_ptr(), raw) };
        return Err(rquickjs::Error::Exception);
    }
    let tag = unsafe { qjs::JS_VALUE_GET_TAG(raw) };
    let length = if tag == qjs::JS_TAG_INT {
        usize::try_from(unsafe { qjs::JS_VALUE_GET_INT(raw) })
            .map_err(|_| rquickjs::Error::Unknown)?
    } else {
        unsafe { qjs::JS_FreeValue(ctx.as_raw().as_ptr(), raw) };
        return Err(rquickjs::Error::Unknown);
    };
    unsafe { qjs::JS_FreeValue(ctx.as_raw().as_ptr(), raw) };
    Ok(length)
}

/// Exact UTF-8 byte length of UTF-16 input, before line-ending normalization.
/// A lone surrogate is counted as three bytes; the parser reports it as an
/// encoding error after the size check.
pub(crate) fn utf8_len(units: &[u16]) -> usize {
    let mut length = 0usize;
    let mut index = 0;
    while index < units.len() {
        let unit = units[index];
        if unit < 0x80 {
            length += 1;
        } else if unit < 0x800 {
            length += 2;
        } else if (0xD800..=0xDBFF).contains(&unit)
            && units
                .get(index + 1)
                .is_some_and(|low| (0xDC00..=0xDFFF).contains(low))
        {
            length += 4;
            index += 1;
        } else {
            length += 3;
        }
        index += 1;
    }
    length
}

/// Decode UTF-16 into normalized text. A leading BOM is removed. Unpaired
/// surrogates become U+FFFD and are recorded with `encoding_code`; NUL is
/// recorded with `nul_code`. The first invalid position in output order is
/// returned.
pub(crate) fn decode_units(
    units: &[u16],
    encoding_code: &'static str,
    nul_code: &'static str,
) -> (String, Option<InvalidText>) {
    let mut output: Vec<(char, Option<&'static str>)> = Vec::with_capacity(units.len());
    let mut index = 0;
    while index < units.len() {
        let unit = units[index];
        if unit == 0x000D {
            if units.get(index + 1) == Some(&0x000A) {
                index += 1;
            }
            output.push(('\n', None));
        } else if unit == 0x000A {
            output.push(('\n', None));
        } else if (0xD800..=0xDBFF).contains(&unit) {
            match units.get(index + 1) {
                Some(&low) if (0xDC00..=0xDFFF).contains(&low) => {
                    let code = 0x10000 + (((unit as u32 - 0xD800) << 10) | (low as u32 - 0xDC00));
                    output.push((char::from_u32(code).unwrap_or('\u{FFFD}'), None));
                    index += 1;
                }
                _ => output.push(('\u{FFFD}', Some(encoding_code))),
            }
        } else if (0xDC00..=0xDFFF).contains(&unit) {
            output.push(('\u{FFFD}', Some(encoding_code)));
        } else if unit == 0 {
            output.push(('\0', Some(nul_code)));
        } else {
            output.push((
                char::from_u32(u32::from(unit)).expect("non-surrogate BMP unit"),
                None,
            ));
        }
        index += 1;
    }

    if output
        .first()
        .is_some_and(|(character, _)| *character == '\u{feff}')
    {
        output.remove(0);
    }

    let mut source = String::with_capacity(units.len());
    let mut invalid = None;
    let mut line = 1u32;
    let mut column = 1u32;
    for (character, code) in output {
        if invalid.is_none() {
            if let Some(code) = code {
                invalid = Some(InvalidText { code, line, column });
            }
        }
        source.push(character);
        if character == '\n' {
            line = line.saturating_add(1);
            column = 1;
        } else {
            column = column.saturating_add(1);
        }
    }
    (source, invalid)
}

/// Convert a primitive string to UTF-16 without first converting to UTF-8.
/// `None` means the actual UTF-16 length or exact UTF-8 byte length exceeds
/// `max_source_bytes`.
pub(crate) fn decode_js_string<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    max_source_bytes: usize,
    encoding_code: &'static str,
    nul_code: &'static str,
) -> JsResult<Option<(String, Option<InvalidText>)>> {
    let mut length = 0;
    let pointer =
        unsafe { qjs::JS_ToCStringLenUTF16(ctx.as_raw().as_ptr(), &mut length, value.as_raw()) };
    if pointer.is_null() {
        return Err(rquickjs::Error::Exception);
    }
    let Ok(length) = usize::try_from(length) else {
        unsafe { qjs::JS_FreeCStringUTF16(ctx.as_raw().as_ptr(), pointer) };
        return Ok(None);
    };
    if length > max_source_bytes {
        unsafe { qjs::JS_FreeCStringUTF16(ctx.as_raw().as_ptr(), pointer) };
        return Ok(None);
    }
    let units = unsafe { std::slice::from_raw_parts(pointer, length) };
    if utf8_len(units) > max_source_bytes {
        unsafe { qjs::JS_FreeCStringUTF16(ctx.as_raw().as_ptr(), pointer) };
        return Ok(None);
    }
    let decoded = decode_units(units, encoding_code, nul_code);
    unsafe { qjs::JS_FreeCStringUTF16(ctx.as_raw().as_ptr(), pointer) };
    Ok(Some(decoded))
}

/// One-based line and Unicode-scalar column of a byte offset.
pub(crate) fn position(text: &str, byte_offset: usize) -> (u32, u32) {
    let mut line = 1u32;
    let mut column = 1u32;
    for (offset, character) in text.char_indices() {
        if offset >= byte_offset {
            break;
        }
        if character == '\n' {
            line = line.saturating_add(1);
            column = 1;
        } else {
            column = column.saturating_add(1);
        }
    }
    (line, column)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_len_counts_original_bytes_before_normalization() {
        assert_eq!(utf8_len(&[0x41, 0x0D, 0x0A, 0xE9]), 5);
        assert_eq!(utf8_len(&[0xD83D, 0xDE00]), 4);
        assert_eq!(utf8_len(&[0xD800]), 3);
    }

    #[test]
    fn decode_units_reports_first_invalid_position() {
        let (text, invalid) = decode_units(&[0x41, 0, 0x42], "ENC", "NUL");
        assert_eq!(text, "A\0B");
        let invalid = invalid.unwrap();
        assert_eq!(invalid.code, "NUL");
        assert_eq!((invalid.line, invalid.column), (1, 2));

        let (text, invalid) = decode_units(&[0x0D, 0x0A, 0xD800], "ENC", "NUL");
        assert_eq!(text, "\n\u{FFFD}");
        let invalid = invalid.unwrap();
        assert_eq!(invalid.code, "ENC");
        assert_eq!((invalid.line, invalid.column), (2, 1));
    }

    #[test]
    fn position_is_one_based_and_counts_scalars() {
        assert_eq!(position("a\nbé", 0), (1, 1));
        assert_eq!(position("a\nbé", 2), (2, 1));
        assert_eq!(position("a\nbé", 4), (2, 3));
    }
}
