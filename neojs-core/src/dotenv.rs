//! A synchronous, permission-free parser for `.env` content strings.
//!
//! The parser deliberately stops at text: it does not read a file, consult the
//! process environment, mutate the process environment, expand variables, or
//! execute shell commands. `neo:fs.readText` with filesystem permission and
//! `neo:env.get` with environment permission are the only ways a caller can
//! supply those inputs and capabilities, and the caller does so explicitly.
//!
//! Syntax is line oriented. See the `neo:dotenv` documentation in the crate
//! README for the complete contract. Errors are returned, never panicked or
//! logged, and a message never contains source text.

#[cfg(test)]
use std::borrow::Cow;

use rquickjs::{
    Ctx, Exception, Function, Object, Persistent, Result as JsResult, Value,
    module::{Declarations, Exports, ModuleDef},
};
use serde_json::json;

use crate::js_text::{decode_js_string, string_length};

#[cfg(test)]
use crate::js_text::{decode_units as decode_units_text, position, utf8_len};

/// Largest accepted source, in UTF-8 bytes.
pub(crate) const MAX_SOURCE_BYTES: usize = 1024 * 1024;
/// Largest number of assignments, including duplicate keys.
pub(crate) const MAX_ENTRIES: usize = 16_384;
/// Largest accepted key, in UTF-8 bytes.
pub(crate) const MAX_KEY_BYTES: usize = 1024;

/// A deterministic parse failure with a stable code and one-based location.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ParseError {
    /// Stable `DOTENV_*` code; match on this, never on a message.
    pub code: &'static str,
    /// One-based source line in the normalized document.
    pub line: u32,
    /// One-based column in Unicode scalar values, not bytes or UTF-16 units.
    pub column: u32,
}

impl ParseError {
    const fn new(code: &'static str, line: u32, column: u32) -> Self {
        Self { code, line, column }
    }

    const fn limit() -> Self {
        Self::new("DOTENV_LIMIT", 1, 1)
    }
}

/// One assignment, in source order. Duplicate keys are retained here; the
/// JavaScript wrapper applies last-value-wins semantics.
type Entry = (String, String);

/// Parse `.env` content into assignments in source order.
///
/// The first error in source order is returned. A leading UTF-8 BOM is ignored.
/// CRLF and lone CR are normalized to LF before parsing; columns count Unicode
/// scalar values. NUL is rejected anywhere. Whitespace is ASCII space and tab
/// only. Values are returned verbatim apart from quote handling: no variable
/// expansion, no command substitution, and no environment reads.
#[cfg(test)]
pub(crate) fn parse(source: &str) -> Result<Vec<Entry>, ParseError> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err(ParseError::limit());
    }
    let normalized = normalize(source);
    let text = strip_bom(&normalized);
    let invalid = first_nul(text);
    choose(parse_text(text), invalid)
}

/// Encode a parse result for the host callback. Messages are not included:
/// only a stable code and location cross the boundary.
#[cfg(test)]
pub(crate) fn response(source: &str) -> String {
    encode(parse(source))
}

/// Encode a result for text that the native callback has already normalized,
/// bounded, and checked for unpaired surrogates and NUL. `invalid` is the
/// earliest such position, if any.
pub(crate) fn decoded_response(text: &str, invalid: Option<ParseError>) -> String {
    encode(choose(parse_text(text), invalid))
}

fn encode(result: Result<Vec<Entry>, ParseError>) -> String {
    let value = match result {
        Ok(entries) => json!({"ok": true, "entries": entries}),
        Err(error) => json!({
            "ok": false,
            "error": {
                "code": error.code,
                "line": error.line,
                "column": error.column,
            },
        }),
    };
    value.to_string()
}

/// Normalize CRLF and lone CR to LF. The returned allocation is bounded by
/// `MAX_SOURCE_BYTES`, which callers check before calling this.
#[cfg(test)]
fn normalize(source: &str) -> Cow<'_, str> {
    if source.contains('\r') {
        Cow::Owned(source.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        Cow::Borrowed(source)
    }
}

#[cfg(test)]
fn strip_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

#[cfg(test)]
fn first_nul(text: &str) -> Option<ParseError> {
    text.find('\0').map(|offset| {
        let (line, column) = position(text, offset);
        ParseError::new("DOTENV_NUL", line, column)
    })
}

/// Return the earlier of the parser's first error and an already detected
/// invalid input position. This keeps NUL and encoding failures in source
/// order instead of letting a later global scan hide an earlier syntax error.
fn choose(
    parsed: Result<Vec<Entry>, ParseError>,
    invalid: Option<ParseError>,
) -> Result<Vec<Entry>, ParseError> {
    match (parsed, invalid) {
        (Ok(entries), None) => Ok(entries),
        (Ok(_), Some(error)) => Err(error),
        (Err(error), None) => Err(error),
        (Err(error), Some(invalid)) => {
            if (invalid.line, invalid.column) <= (error.line, error.column) {
                Err(invalid)
            } else {
                Err(error)
            }
        }
    }
}

fn parse_text(text: &str) -> Result<Vec<Entry>, ParseError> {
    let mut parser = Parser {
        text,
        position: 0,
        line: 1,
        column: 1,
        entries: Vec::new(),
    };
    parser.document()?;
    Ok(parser.entries)
}

struct Parser<'a> {
    text: &'a str,
    position: usize,
    line: u32,
    column: u32,
    entries: Vec<Entry>,
}

impl Parser<'_> {
    fn document(&mut self) -> Result<(), ParseError> {
        while self.peek().is_some() {
            self.skip_horizontal_space();
            match self.peek() {
                None => break,
                Some('\n') => {
                    self.bump();
                }
                Some('#') => self.skip_line(),
                Some(_) => self.assignment()?,
            }
        }
        Ok(())
    }

    fn assignment(&mut self) -> Result<(), ParseError> {
        if self.export_prefix() {
            for _ in 0.."export".len() {
                self.bump();
            }
            self.skip_horizontal_space();
        }

        let key_start_line = self.line;
        let key_start_column = self.column;
        let key_start = self.position;
        while self.peek().is_some_and(is_key_character) {
            self.bump();
        }
        if self.position == key_start {
            return Err(ParseError::new(
                "DOTENV_KEY",
                key_start_line,
                key_start_column,
            ));
        }
        let key = &self.text[key_start..self.position];
        if key.len() > MAX_KEY_BYTES {
            return Err(ParseError::new(
                "DOTENV_LIMIT",
                key_start_line,
                key_start_column,
            ));
        }
        let key = key.to_owned();

        self.skip_horizontal_space();
        if self.peek() != Some('=') {
            return Err(ParseError::new("DOTENV_SYNTAX", self.line, self.column));
        }
        self.bump();

        self.skip_horizontal_space();
        let value = match self.peek() {
            None => String::new(),
            Some('\n') => {
                self.bump();
                String::new()
            }
            Some('\'') => self.quoted('\'', Quote::Single)?,
            Some('"') => self.quoted('"', Quote::Double)?,
            Some('`') => self.quoted('`', Quote::Raw)?,
            Some(_) => self.unquoted(),
        };

        if self.entries.len() >= MAX_ENTRIES {
            return Err(ParseError::new("DOTENV_LIMIT", self.line, self.column));
        }
        self.entries.push((key, value));
        Ok(())
    }

    /// `export` is a prefix only when the exact token is followed by
    /// horizontal space. `export=value` therefore has the key `export`.
    fn export_prefix(&self) -> bool {
        let rest = &self.text[self.position..];
        let Some(after) = rest.strip_prefix("export") else {
            return false;
        };
        matches!(after.chars().next(), Some(' ' | '\t'))
    }

    fn quoted(&mut self, delimiter: char, quote: Quote) -> Result<String, ParseError> {
        let open_line = self.line;
        let open_column = self.column;
        self.bump(); // opening delimiter
        let mut value = String::new();

        loop {
            match self.peek() {
                None => {
                    return Err(ParseError::new("DOTENV_QUOTE", open_line, open_column));
                }
                Some('\n') => {
                    value.push('\n');
                    self.bump();
                }
                Some('\\') if quote != Quote::Raw => {
                    let escape_line = self.line;
                    let escape_column = self.column;
                    self.bump(); // backslash
                    match quote {
                        Quote::Single => {
                            // Only \' is an escape. Every other backslash is
                            // literal, matching the reference parser.
                            if self.peek() == Some('\'') {
                                value.push('\'');
                                self.bump();
                            } else {
                                value.push('\\');
                            }
                        }
                        Quote::Double => {
                            let Some(next) = self.peek() else {
                                // The unterminated-quote check reports EOF on
                                // the next pass, at the opening delimiter.
                                value.push('\\');
                                continue;
                            };
                            match next {
                                'n' => {
                                    value.push('\n');
                                    self.bump();
                                }
                                'r' => {
                                    value.push('\r');
                                    self.bump();
                                }
                                't' => {
                                    value.push('\t');
                                    self.bump();
                                }
                                'b' => {
                                    value.push('\u{0008}');
                                    self.bump();
                                }
                                '\\' => {
                                    value.push('\\');
                                    self.bump();
                                }
                                '"' => {
                                    value.push('"');
                                    self.bump();
                                }
                                '\'' => {
                                    value.push('\'');
                                    self.bump();
                                }
                                '`' => {
                                    value.push('`');
                                    self.bump();
                                }
                                'u' | 'U' => {
                                    let digits = if next == 'u' { 4 } else { 8 };
                                    self.bump(); // u or U
                                    value.push(self.hex_escape(
                                        digits,
                                        escape_line,
                                        escape_column,
                                    )?);
                                }
                                // Unknown escapes are literal: the backslash is
                                // kept and the next character is processed on
                                // the next pass.
                                _ => value.push('\\'),
                            }
                        }
                        Quote::Raw => unreachable!(),
                    }
                }
                Some(character) if character == delimiter => {
                    self.bump();
                    break;
                }
                Some(character) => {
                    value.push(character);
                    self.bump();
                }
            }
        }

        self.after_quoted()?;
        Ok(value)
    }

    fn hex_escape(
        &mut self,
        digits: usize,
        escape_line: u32,
        escape_column: u32,
    ) -> Result<char, ParseError> {
        let mut code = 0u32;
        for _ in 0..digits {
            let Some(character) = self.bump() else {
                return Err(ParseError::new("DOTENV_ESCAPE", escape_line, escape_column));
            };
            let Some(digit) = character.to_digit(16) else {
                return Err(ParseError::new("DOTENV_ESCAPE", escape_line, escape_column));
            };
            code = code * 16 + digit;
        }
        if code == 0 {
            // An escaped NUL would smuggle a byte that an OS environment
            // variable cannot hold. Reject it with the literal-NUL code.
            return Err(ParseError::new("DOTENV_NUL", escape_line, escape_column));
        }
        char::from_u32(code)
            .ok_or_else(|| ParseError::new("DOTENV_ESCAPE", escape_line, escape_column))
    }

    /// Only horizontal space and an optional comment may follow a closing
    /// quote before the line terminator.
    fn after_quoted(&mut self) -> Result<(), ParseError> {
        self.skip_horizontal_space();
        match self.peek() {
            None => Ok(()),
            Some('\n') => {
                self.bump();
                Ok(())
            }
            Some('#') => {
                self.skip_line();
                Ok(())
            }
            Some(_) => Err(ParseError::new("DOTENV_SYNTAX", self.line, self.column)),
        }
    }

    fn unquoted(&mut self) -> String {
        let mut value = String::new();
        loop {
            match self.peek() {
                None => break,
                Some('\n') => {
                    self.bump();
                    break;
                }
                Some('#') if value.is_empty() || value.ends_with(' ') || value.ends_with('\t') => {
                    self.skip_line();
                    break;
                }
                Some(character) => {
                    value.push(character);
                    self.bump();
                }
            }
        }
        while value.ends_with(' ') || value.ends_with('\t') {
            value.pop();
        }
        value
    }

    fn skip_horizontal_space(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t')) {
            self.bump();
        }
    }

    fn skip_line(&mut self) {
        while let Some(character) = self.peek() {
            self.bump();
            if character == '\n' {
                break;
            }
        }
    }

    fn peek(&self) -> Option<char> {
        self.text[self.position..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let character = self.peek()?;
        self.position += character.len_utf8();
        if character == '\n' {
            self.line = self.line.saturating_add(1);
            self.column = 1;
        } else {
            self.column = self.column.saturating_add(1);
        }
        Some(character)
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Quote {
    Single,
    Double,
    Raw,
}

fn is_key_character(character: char) -> bool {
    character == '_' || character.is_ascii_alphanumeric()
}

/// Decode UTF-16 with the dotenv codes for the module's unit tests.
#[cfg(test)]
fn decode_units(units: &[u16]) -> (String, Option<ParseError>) {
    let (text, invalid) = decode_units_text(units, "DOTENV_ENCODING", "DOTENV_NUL");
    (
        text,
        invalid.map(|invalid| ParseError::new(invalid.code, invalid.line, invalid.column)),
    )
}

/// Native callback for `neo:dotenv`. It is passed only to the module factory
/// during module evaluation and is never installed on the global object.
pub(crate) fn parse_js<'js>(ctx: Ctx<'js>, value: Value<'js>) -> JsResult<String> {
    if !value.is_string() {
        return Err(Exception::throw_type(&ctx, "source must be a string"));
    }
    if string_length(&ctx, &value)? > MAX_SOURCE_BYTES {
        return Ok(encode(Err(ParseError::limit())));
    }
    let Some((source, invalid)) = decode_js_string(
        &ctx,
        &value,
        MAX_SOURCE_BYTES,
        "DOTENV_ENCODING",
        "DOTENV_NUL",
    )?
    else {
        return Ok(encode(Err(ParseError::limit())));
    };
    if source.len() > MAX_SOURCE_BYTES {
        return Ok(encode(Err(ParseError::limit())));
    }
    let invalid =
        invalid.map(|invalid| ParseError::new(invalid.code, invalid.line, invalid.column));
    Ok(decoded_response(&source, invalid))
}

/// Persistent factory for the private `neo:dotenv` wrapper.
#[derive(Default)]
pub(crate) struct DotenvState {
    pub(crate) factory: Option<Persistent<Function<'static>>>,
}

// DotenvState contains no borrowed JS value; its persistent handle is
// explicitly released before the context and runtime are dropped.
unsafe impl<'js> rquickjs::JsLifetime<'js> for DotenvState {
    type Changed<'to> = DotenvState;
}

const EXPORTS: [&str; 6] = [
    "default",
    "parse",
    "DotenvParseError",
    "MAX_SOURCE_BYTES",
    "MAX_ENTRIES",
    "MAX_KEY_BYTES",
];

/// The native `neo:dotenv` module.
///
/// Its factory receives the parse callback privately. Nothing is exported on
/// the global object, so a script cannot replace or call a host callback before
/// or after importing this module.
pub(crate) struct DotenvModule;

impl ModuleDef for DotenvModule {
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
                .userdata::<Rc<RefCell<DotenvState>>>()
                .ok_or(rquickjs::Error::Unknown)?,
        );
        let factory = state
            .borrow()
            .factory
            .as_ref()
            .cloned()
            .ok_or(rquickjs::Error::Unknown)?
            .restore(ctx)?;
        let parse_native = Function::new(ctx.clone(), parse_js)?;
        let api: Object = factory.call((parse_native,))?;
        for name in EXPORTS {
            exports.export(name, api.get::<_, Value>(name)?)?;
        }
        Ok(())
    }
}

/// Build the private wrapper in the fresh realm, before user code runs.
pub(crate) fn factory<'js>(ctx: &Ctx<'js>) -> JsResult<Function<'js>> {
    ctx.eval(include_str!("js/dotenv.js"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(source: &str) -> Vec<(String, String)> {
        parse(source).unwrap()
    }

    fn error(source: &str) -> ParseError {
        parse(source).unwrap_err()
    }

    #[test]
    fn parses_basic_assignments_and_ignores_layout() {
        let entries =
            pairs("  # comment\n\n  NAME = Neo Tales \nexport OTHER='two words'\nEMPTY=\n");
        assert_eq!(
            entries,
            vec![
                ("NAME".to_owned(), "Neo Tales".to_owned()),
                ("OTHER".to_owned(), "two words".to_owned()),
                ("EMPTY".to_owned(), String::new()),
            ]
        );
    }

    #[test]
    fn crlf_and_lone_cr_are_normalized_in_multiline_values() {
        let entries = pairs("A=\"one\r\ntwo\"\rB='three\rfour'\r");
        assert_eq!(entries[0].1, "one\ntwo");
        assert_eq!(entries[1].1, "three\nfour");
    }

    #[test]
    fn leading_bom_is_ignored_only_at_the_start() {
        assert_eq!(pairs("\u{feff}A=1")[0], ("A".to_owned(), "1".to_owned()));
        assert_eq!(pairs("A=\u{feff}1")[0].1, "\u{feff}1");
    }

    #[test]
    fn single_quotes_only_escape_a_quote_and_keep_other_backslashes() {
        let entries = pairs(r"A='one\ntwo\'three\\four'");
        assert_eq!(entries[0].1, "one\\ntwo'three\\\\four");
    }

    #[test]
    fn double_quotes_decode_known_escapes() {
        let entries = pairs(r#"A="a\nb\tc\rd\be\\f\"g\'h\`i\u2603\U0001F920""#);
        assert_eq!(entries[0].1, "a\nb\tc\rd\u{0008}e\\f\"g'h`i☃🤠");
    }

    #[test]
    fn unknown_double_quote_escapes_are_literal() {
        assert_eq!(pairs(r#"A="a\qb""#)[0].1, r"a\qb");
    }

    #[test]
    fn backticks_are_raw_and_never_execute_or_expand() {
        let entries = pairs("A=`$(echo hi) $HOME \\n`");
        assert_eq!(entries[0].1, "$(echo hi) $HOME \\n");
    }

    #[test]
    fn values_are_not_expanded() {
        let entries = pairs("A=$HOME\nB=${HOME:-/tmp}\nC=$(whoami)\nD=\"${A}\"");
        assert_eq!(entries[0].1, "$HOME");
        assert_eq!(entries[1].1, "${HOME:-/tmp}");
        assert_eq!(entries[2].1, "$(whoami)");
        assert_eq!(entries[3].1, "${A}");
    }

    #[test]
    fn inline_comments_require_preceding_space_or_empty_value() {
        let entries = pairs("A=abc#def\nB=abc # comment\nC=# only\nD=abc\t# tab");
        assert_eq!(entries[0].1, "abc#def");
        assert_eq!(entries[1].1, "abc");
        assert_eq!(entries[2].1, "");
        assert_eq!(entries[3].1, "abc");
    }

    #[test]
    fn comments_after_quotes_are_allowed_and_other_text_is_an_error() {
        assert_eq!(pairs("A=\"x\"#c\nB='y' #c")[1].1, "y");
        let failure = error("A=\"x\" tail");
        assert_eq!(failure.code, "DOTENV_SYNTAX");
        assert_eq!((failure.line, failure.column), (1, 7));
    }

    #[test]
    fn duplicate_keys_are_retained_in_source_order() {
        let entries = pairs("A=1\nB=2\nA=3");
        assert_eq!(
            entries,
            vec![
                ("A".to_owned(), "1".to_owned()),
                ("B".to_owned(), "2".to_owned()),
                ("A".to_owned(), "3".to_owned()),
            ]
        );
    }

    #[test]
    fn export_is_case_sensitive_and_needs_a_separator() {
        let entries = pairs("export A=1\n export\tB=2\nexport=3\nexportC=4\nEXPORT=5");
        assert_eq!(entries[0], ("A".to_owned(), "1".to_owned()));
        assert_eq!(entries[1], ("B".to_owned(), "2".to_owned()));
        assert_eq!(entries[2], ("export".to_owned(), "3".to_owned()));
        assert_eq!(entries[3], ("exportC".to_owned(), "4".to_owned()));
        assert_eq!(entries[4], ("EXPORT".to_owned(), "5".to_owned()));
    }

    #[test]
    fn unicode_is_preserved() {
        let entries = pairs("A=日本語🎉\nB='héllo'\nC=\"☃\"");
        assert_eq!(entries[0].1, "日本語🎉");
        assert_eq!(entries[1].1, "héllo");
        assert_eq!(entries[2].1, "☃");
    }

    #[test]
    fn hostile_object_keys_are_ordinary_keys() {
        let entries = pairs("__proto__=x\nconstructor=y\nprototype=z");
        assert_eq!(entries[0].0, "__proto__");
        assert_eq!(entries[1].0, "constructor");
        assert_eq!(entries[2].0, "prototype");
    }

    #[test]
    fn rejects_missing_equals_and_bad_keys_with_locations() {
        let failure = error("A=1\nBROKEN\n");
        assert_eq!(failure.code, "DOTENV_SYNTAX");
        assert_eq!((failure.line, failure.column), (2, 7));

        let failure = error("=value");
        assert_eq!(failure.code, "DOTENV_KEY");
        assert_eq!((failure.line, failure.column), (1, 1));

        let failure = error("BAD-KEY=1");
        assert_eq!(failure.code, "DOTENV_SYNTAX");
        assert_eq!((failure.line, failure.column), (1, 4));

        let failure = error("A=1\né=2");
        assert_eq!(failure.code, "DOTENV_KEY");
        assert_eq!((failure.line, failure.column), (2, 1));
    }

    #[test]
    fn rejects_unterminated_and_malformed_quotes() {
        let failure = error("A=\"first\nsecond");
        assert_eq!(failure.code, "DOTENV_QUOTE");
        assert_eq!((failure.line, failure.column), (1, 3));

        let failure = error("A='one\n\nthree");
        assert_eq!(failure.code, "DOTENV_QUOTE");
        assert_eq!((failure.line, failure.column), (1, 3));
    }

    #[test]
    fn rejects_malformed_unicode_escapes_and_surrogates() {
        for source in [r#"A="\u12""#, r#"A="\uZZZZ""#, r#"A="\U0000D800""#] {
            let failure = error(source);
            assert_eq!(failure.code, "DOTENV_ESCAPE", "{source}");
            assert_eq!((failure.line, failure.column), (1, 4), "{source}");
        }
    }

    #[test]
    fn rejects_nul_anywhere_with_a_location() {
        let failure = error("A=1\nB=two\0three");
        assert_eq!(failure.code, "DOTENV_NUL");
        assert_eq!((failure.line, failure.column), (2, 6));

        let failure = error(r#"A="\u0000""#);
        assert_eq!(failure.code, "DOTENV_NUL");
        assert_eq!((failure.line, failure.column), (1, 4));
    }

    #[test]
    fn source_order_prefers_an_earlier_syntax_error_over_a_later_nul() {
        let failure = error("BROKEN\nA=\0B");
        assert_eq!(failure.code, "DOTENV_SYNTAX");
        assert_eq!((failure.line, failure.column), (1, 7));

        let failure = error("A=\0B\nBROKEN");
        assert_eq!(failure.code, "DOTENV_NUL");
        assert_eq!((failure.line, failure.column), (1, 3));

        let failure = error(
            r#"BROKEN
A="\u0000""#,
        );
        assert_eq!(failure.code, "DOTENV_SYNTAX");

        let failure = error("A=\"\\u0000\"\nBROKEN");
        assert_eq!(failure.code, "DOTENV_NUL");
    }

    #[test]
    fn decodes_utf16_with_normalized_scalars_and_positions() {
        assert_eq!(decode_units(&[0xD83D, 0xDE00]), ("😀".to_owned(), None));
        assert_eq!(
            decode_units(&[0x41, 0x0D, 0x0A, 0x42]),
            ("A\nB".to_owned(), None)
        );

        let (text, invalid) = decode_units(&[0x41, 0, 0x42]);
        assert_eq!(text, "A\0B");
        let invalid = invalid.unwrap();
        assert_eq!(invalid.code, "DOTENV_NUL");
        assert_eq!((invalid.line, invalid.column), (1, 2));

        let (text, invalid) = decode_units(&[0x0D, 0x0A, 0xD800]);
        assert_eq!(text, "\n\u{FFFD}");
        let invalid = invalid.unwrap();
        assert_eq!(invalid.code, "DOTENV_ENCODING");
        assert_eq!((invalid.line, invalid.column), (2, 1));

        let (text, invalid) = decode_units(&[0xDC00]);
        assert_eq!(text, "\u{FFFD}");
        let invalid = invalid.unwrap();
        assert_eq!(invalid.code, "DOTENV_ENCODING");
        assert_eq!((invalid.line, invalid.column), (1, 1));
    }

    #[test]
    fn utf8_len_counts_original_bytes_before_normalization() {
        assert_eq!(utf8_len(&[0x41, 0x0D, 0x0A, 0xE9]), 5);
        assert_eq!(utf8_len(&[0xD83D, 0xDE00]), 4);
        assert_eq!(utf8_len(&[0xD800]), 3);
    }

    #[test]
    fn leading_bom_is_removed_before_invalid_positions_are_counted() {
        let (text, invalid) = decode_units(&[0xFEFF, 0x41, 0xD800]);
        assert_eq!(text, "A\u{FFFD}");
        let invalid = invalid.unwrap();
        assert_eq!(invalid.code, "DOTENV_ENCODING");
        assert_eq!((invalid.line, invalid.column), (1, 2));
    }

    #[test]
    fn enforces_documented_limits() {
        let long_key = format!("{}=1", "A".repeat(MAX_KEY_BYTES + 1));
        let failure = error(&long_key);
        assert_eq!(failure.code, "DOTENV_LIMIT");

        let over = "A".repeat(MAX_SOURCE_BYTES + 1);
        assert_eq!(error(&over).code, "DOTENV_LIMIT");

        let too_many = "A=1\n".repeat(MAX_ENTRIES + 1);
        assert_eq!(error(&too_many).code, "DOTENV_LIMIT");
    }

    #[test]
    fn empty_and_comment_only_sources_have_no_entries() {
        assert!(pairs("").is_empty());
        assert!(pairs("\n\n# one\n   # two\n").is_empty());
    }

    #[test]
    fn response_is_machine_readable_and_never_echoes_values() {
        let ok: serde_json::Value = serde_json::from_str(&response("A=secret")).unwrap();
        assert_eq!(ok["ok"], true);
        assert_eq!(ok["entries"][0][0], "A");
        assert_eq!(ok["entries"][0][1], "secret");

        let bad = response("A=\"secret");
        assert!(!bad.contains("secret"));
        let bad: serde_json::Value = serde_json::from_str(&bad).unwrap();
        assert_eq!(bad["ok"], false);
        assert_eq!(bad["error"]["code"], "DOTENV_QUOTE");
    }
}
