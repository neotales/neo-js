//! A synchronous, permission-free YAML parser.
//!
//! The parser is given text. It does not read a file, consult the process
//! environment, access the network, or execute anything. Reading a YAML file is
//! a separate, explicit decision through a permission-gated module such as
//! `neo:fs`.
//!
//! Syntax is delegated to the maintained `saphyr-parser` crate, the parser
//! behind the `saphyr` YAML 1.2 library. Conversion is this module's own and is
//! deliberately smaller than general YAML:
//!
//! - One document. A second document is rejected.
//! - No aliases. An alias is rejected before it can expand, so an alias bomb
//!   cannot allocate. Anchors without aliases are ignored.
//! - No explicit tags, including core-schema tags such as `!!str`.
//! - Mapping keys must be plain or quoted strings that resolve to a string.
//!   A numeric, boolean, null, sequence, or mapping key is rejected.
//! - Duplicate string keys are rejected, not last-value-wins.
//! - Plain scalars follow the YAML 1.2 core schema: null, boolean, integer,
//!   finite float, or string. YAML has no timestamp type in that schema, so a
//!   date-like scalar remains a string. Non-finite floats and integers outside
//!   ±(2^53 - 1) are rejected.
//! - Objects are null-prototype records, arrays are arrays, and values are
//!   strings, finite numbers, booleans, and null.
//!
//! Limits are enforced while events are consumed: 1 MiB of source, 64
//! collection levels (root counts as one), 100,000 nodes, 100,000 entries per
//! collection, and a bounded output budget. Budget errors report the event that
//! crossed the limit; the source-size check has no event and reports 1:1.
//! Messages never include source text or values.

use std::collections::BTreeMap;

use rquickjs::{
    Ctx, Exception, Function, Object, Persistent, Result as JsResult, Value,
    module::{Declarations, Exports, ModuleDef},
};
use saphyr_parser::{BufferedInput, Event, Marker, Parser, ScalarStyle, Span};
use serde_json::{Map, Number, Value as Json, json};

use crate::js_text::{InvalidText, decode_js_string, string_length};

/// Largest accepted source, in UTF-8 bytes.
pub(crate) const MAX_SOURCE_BYTES: usize = 1024 * 1024;
/// Maximum collection nesting depth, counting the root collection as one.
pub(crate) const MAX_DEPTH: usize = 64;
/// Maximum number of scalar and collection nodes.
pub(crate) const MAX_NODES: usize = 100_000;
/// Maximum entries in one sequence or mapping.
pub(crate) const MAX_ITEMS: usize = 100_000;
/// Upper bound on the JSON response built for JavaScript, in bytes.
pub(crate) const MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;

/// Largest integer exactly representable in a JavaScript number.
const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

const MESSAGES: &str = r#"{
  "YAML_SYNTAX": "invalid YAML syntax",
  "YAML_ALIAS": "YAML aliases are not supported",
  "YAML_TAG": "YAML tags are not supported",
  "YAML_KEY": "YAML mapping keys must be strings",
  "YAML_DUPLICATE": "duplicate YAML mapping key",
  "YAML_MULTIDOC": "multiple YAML documents are not supported",
  "YAML_INTEGER": "YAML integer is outside the exact JavaScript range",
  "YAML_FLOAT": "YAML floating-point value is not finite",
  "YAML_DEPTH": "YAML nesting exceeds the supported depth",
  "YAML_LIMIT": "YAML input exceeds a documented limit",
  "YAML_ENCODING": "YAML source is not valid UTF-16",
  "YAML_NUL": "YAML source contains a NUL character"
}"#;

const EXPORTS: [&str; 8] = [
    "default",
    "parse",
    "YamlParseError",
    "MAX_SOURCE_BYTES",
    "MAX_DEPTH",
    "MAX_NODES",
    "MAX_ITEMS",
    "MAX_OUTPUT_BYTES",
];

/// A deterministic parse failure with a stable code and location.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ParseError {
    /// Stable `YAML_*` code; match on this, never on a message.
    pub(crate) code: &'static str,
    /// One-based source line.
    pub(crate) line: u32,
    /// One-based column in Unicode scalar values.
    pub(crate) column: u32,
}

impl ParseError {
    const fn new(code: &'static str, line: u32, column: u32) -> Self {
        Self { code, line, column }
    }

    const fn limit() -> Self {
        Self::new("YAML_LIMIT", 1, 1)
    }

    fn at(code: &'static str, marker: Marker) -> Self {
        Self::new(
            code,
            u32::try_from(marker.line()).unwrap_or(u32::MAX),
            u32::try_from(marker.col().saturating_add(1)).unwrap_or(u32::MAX),
        )
    }
}

/// Parse YAML text into a JSON value. Test-facing; the native callback adds
/// string decoding and error ordering.
pub(crate) fn parse(source: &str) -> Result<Json, ParseError> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err(ParseError::limit());
    }
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let mut parser = Parser::new(BufferedInput::new(source.chars()));
    let mut builder = Builder::default();
    loop {
        match parser.next_event() {
            None => break,
            Some(Err(error)) => {
                return Err(ParseError::at("YAML_SYNTAX", *error.marker()));
            }
            Some(Ok((event, span))) => builder.event(event, span)?,
        }
    }
    builder.finish()
}

/// Encode text that the native callback has already normalized and bounded.
pub(crate) fn decoded_response(source: &str, invalid: Option<InvalidText>) -> String {
    let invalid =
        invalid.map(|invalid| ParseError::new(invalid.code, invalid.line, invalid.column));
    encode(choose(parse(source), invalid))
}

fn encode(result: Result<Json, ParseError>) -> String {
    match result {
        Ok(value) => json!({"ok": true, "value": value}).to_string(),
        Err(error) => json!({
            "ok": false,
            "error": {
                "code": error.code,
                "line": error.line,
                "column": error.column,
            },
        })
        .to_string(),
    }
}

/// Prefer the earlier concrete error.
fn choose(
    parsed: Result<Json, ParseError>,
    invalid: Option<ParseError>,
) -> Result<Json, ParseError> {
    match (parsed, invalid) {
        (Ok(value), None) => Ok(value),
        (Ok(_), Some(invalid)) => Err(invalid),
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

#[derive(Default)]
struct Builder {
    stack: Vec<Frame>,
    root: Option<Json>,
    document_value: Option<Json>,
    documents: usize,
    budget: Budget,
}

enum Frame {
    Sequence(Vec<Json>),
    Mapping(BTreeMap<String, Json>, Option<(String, Marker)>),
}

struct Budget {
    nodes: usize,
    strings: usize,
    output: usize,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            nodes: 0,
            strings: 0,
            output: MAX_OUTPUT_BYTES,
        }
    }
}

impl Budget {
    fn node(&mut self) -> Result<(), &'static str> {
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes > MAX_NODES {
            return Err("YAML_LIMIT");
        }
        self.charge(24)
    }

    fn item(&mut self) -> Result<(), &'static str> {
        self.charge(8)
    }

    fn string(&mut self, bytes: usize) -> Result<(), &'static str> {
        self.strings = self.strings.saturating_add(bytes);
        if self.strings > MAX_SOURCE_BYTES {
            return Err("YAML_LIMIT");
        }
        self.charge(bytes.saturating_mul(6).saturating_add(16))
    }

    fn charge(&mut self, bytes: usize) -> Result<(), &'static str> {
        let Some(remaining) = self.output.checked_sub(bytes) else {
            return Err("YAML_LIMIT");
        };
        self.output = remaining;
        Ok(())
    }
}

#[derive(Clone, Debug)]
enum ScalarValue {
    Null,
    Boolean(bool),
    Integer(i64),
    Float(f64),
    String(String),
}

impl Builder {
    fn event(&mut self, event: Event<'_>, span: Span) -> Result<(), ParseError> {
        let marker = span.start;
        match event {
            Event::Nothing | Event::StreamStart | Event::StreamEnd => Ok(()),
            Event::DocumentStart(_) => {
                self.documents = self.documents.saturating_add(1);
                if self.documents > 1 {
                    return Err(ParseError::at("YAML_MULTIDOC", marker));
                }
                if !self.stack.is_empty() || self.root.is_some() || self.document_value.is_some() {
                    return Err(ParseError::at("YAML_SYNTAX", marker));
                }
                Ok(())
            }
            Event::DocumentEnd => {
                if !self.stack.is_empty() {
                    return Err(ParseError::at("YAML_SYNTAX", marker));
                }
                if self.document_value.is_some() {
                    return Err(ParseError::at("YAML_SYNTAX", marker));
                }
                self.document_value = Some(self.root.take().unwrap_or(Json::Null));
                Ok(())
            }
            Event::Alias(_) => Err(ParseError::at("YAML_ALIAS", marker)),
            Event::Scalar(raw, style, _anchor, tag) => {
                if tag.is_some() {
                    return Err(ParseError::at("YAML_TAG", marker));
                }
                if self.expecting_key() {
                    self.set_key(&raw, style, marker)
                } else {
                    let scalar =
                        resolve_scalar(&raw, style).map_err(|code| ParseError::at(code, marker))?;
                    let value = self.scalar_value(scalar, marker)?;
                    self.insert_value(value, marker)
                }
            }
            Event::SequenceStart(_anchor, tag) => {
                if tag.is_some() {
                    return Err(ParseError::at("YAML_TAG", marker));
                }
                if self.expecting_key() {
                    return Err(ParseError::at("YAML_KEY", marker));
                }
                self.start_collection(marker)?;
                self.stack.push(Frame::Sequence(Vec::new()));
                Ok(())
            }
            Event::MappingStart(_anchor, tag) => {
                if tag.is_some() {
                    return Err(ParseError::at("YAML_TAG", marker));
                }
                if self.expecting_key() {
                    return Err(ParseError::at("YAML_KEY", marker));
                }
                self.start_collection(marker)?;
                self.stack.push(Frame::Mapping(BTreeMap::new(), None));
                Ok(())
            }
            Event::SequenceEnd => {
                let Some(Frame::Sequence(items)) = self.stack.pop() else {
                    return Err(ParseError::at("YAML_SYNTAX", marker));
                };
                self.insert_value(Json::Array(items), marker)
            }
            Event::MappingEnd => {
                let Some(Frame::Mapping(entries, key)) = self.stack.pop() else {
                    return Err(ParseError::at("YAML_SYNTAX", marker));
                };
                if key.is_some() {
                    return Err(ParseError::at("YAML_SYNTAX", marker));
                }
                let mut object = Map::new();
                for (name, value) in entries {
                    object.insert(name, value);
                }
                self.insert_value(Json::Object(object), marker)
            }
        }
    }

    fn finish(self) -> Result<Json, ParseError> {
        if !self.stack.is_empty() || self.root.is_some() {
            return Err(ParseError::new("YAML_SYNTAX", 1, 1));
        }
        Ok(self.document_value.unwrap_or(Json::Null))
    }

    fn expecting_key(&self) -> bool {
        matches!(self.stack.last(), Some(Frame::Mapping(_, None)))
    }

    fn start_collection(&mut self, marker: Marker) -> Result<(), ParseError> {
        let depth = self.stack.len().saturating_add(1);
        if depth > MAX_DEPTH {
            return Err(ParseError::at("YAML_DEPTH", marker));
        }
        self.budget
            .node()
            .map_err(|code| ParseError::at(code, marker))
    }

    fn scalar_value(&mut self, scalar: ScalarValue, marker: Marker) -> Result<Json, ParseError> {
        self.budget
            .node()
            .map_err(|code| ParseError::at(code, marker))?;
        match scalar {
            ScalarValue::Null => Ok(Json::Null),
            ScalarValue::Boolean(value) => Ok(Json::Bool(value)),
            ScalarValue::Integer(value) => Ok(Json::Number(Number::from(value))),
            ScalarValue::Float(value) => Number::from_f64(value)
                .map(Json::Number)
                .ok_or_else(|| ParseError::at("YAML_FLOAT", marker)),
            ScalarValue::String(text) => {
                self.budget
                    .string(text.len())
                    .map_err(|code| ParseError::at(code, marker))?;
                Ok(Json::String(text))
            }
        }
    }

    fn set_key(&mut self, raw: &str, style: ScalarStyle, marker: Marker) -> Result<(), ParseError> {
        let scalar = resolve_scalar(raw, style).map_err(|code| ParseError::at(code, marker))?;
        let ScalarValue::String(key) = scalar else {
            return Err(ParseError::at("YAML_KEY", marker));
        };
        self.budget
            .node()
            .map_err(|code| ParseError::at(code, marker))?;
        self.budget
            .string(key.len())
            .map_err(|code| ParseError::at(code, marker))?;
        let Some(Frame::Mapping(_, key_slot)) = self.stack.last_mut() else {
            return Err(ParseError::at("YAML_SYNTAX", marker));
        };
        *key_slot = Some((key, marker));
        Ok(())
    }

    fn insert_value(&mut self, value: Json, marker: Marker) -> Result<(), ParseError> {
        let Some(frame) = self.stack.last_mut() else {
            if self.root.is_some() {
                return Err(ParseError::at("YAML_SYNTAX", marker));
            }
            self.root = Some(value);
            return Ok(());
        };
        match frame {
            Frame::Sequence(items) => {
                if items.len() >= MAX_ITEMS {
                    return Err(ParseError::at("YAML_LIMIT", marker));
                }
                self.budget
                    .item()
                    .map_err(|code| ParseError::at(code, marker))?;
                items.push(value);
                Ok(())
            }
            Frame::Mapping(entries, key) => {
                let Some((key, key_marker)) = key.take() else {
                    return Err(ParseError::at("YAML_SYNTAX", marker));
                };
                if entries.len() >= MAX_ITEMS {
                    return Err(ParseError::at("YAML_LIMIT", marker));
                }
                self.budget
                    .item()
                    .map_err(|code| ParseError::at(code, marker))?;
                if entries.insert(key, value).is_some() {
                    return Err(ParseError::at("YAML_DUPLICATE", key_marker));
                }
                Ok(())
            }
        }
    }
}

fn resolve_scalar(raw: &str, style: ScalarStyle) -> Result<ScalarValue, &'static str> {
    if style != ScalarStyle::Plain {
        return Ok(ScalarValue::String(raw.to_owned()));
    }
    if raw.is_empty() {
        return Ok(ScalarValue::Null);
    }

    let bytes = raw.as_bytes();
    if bytes.len() >= 2 {
        match (bytes[0], bytes[1]) {
            (b'0', b'x') => {
                let digits = &raw[2..];
                if !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    return match i64::from_str_radix(digits, 16) {
                        Ok(value) => integer(value),
                        Err(_) => Err("YAML_INTEGER"),
                    };
                }
            }
            (b'0', b'o') => {
                let digits = &raw[2..];
                if !digits.is_empty() && digits.bytes().all(|byte| matches!(byte, b'0'..=b'7')) {
                    return match i64::from_str_radix(digits, 8) {
                        Ok(value) => integer(value),
                        Err(_) => Err("YAML_INTEGER"),
                    };
                }
            }
            _ => {}
        }
    }

    match raw {
        "~" | "null" | "Null" | "NULL" => return Ok(ScalarValue::Null),
        "true" | "True" | "TRUE" => return Ok(ScalarValue::Boolean(true)),
        "false" | "False" | "FALSE" => return Ok(ScalarValue::Boolean(false)),
        _ => {}
    }

    if is_decimal_integer(raw) {
        return match raw.parse::<i64>() {
            Ok(value) => integer(value),
            Err(_) => Err("YAML_INTEGER"),
        };
    }

    if let Some(value) = parse_float(raw) {
        if !value.is_finite() {
            return Err("YAML_FLOAT");
        }
        return Ok(ScalarValue::Float(value));
    }

    Ok(ScalarValue::String(raw.to_owned()))
}

fn integer(value: i64) -> Result<ScalarValue, &'static str> {
    if !(-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&value) {
        Err("YAML_INTEGER")
    } else {
        Ok(ScalarValue::Integer(value))
    }
}

fn is_decimal_integer(raw: &str) -> bool {
    let digits = raw
        .strip_prefix('+')
        .or_else(|| raw.strip_prefix('-'))
        .unwrap_or(raw);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

fn parse_float(raw: &str) -> Option<f64> {
    match raw {
        ".inf" | ".Inf" | ".INF" | "+.inf" | "+.Inf" | "+.INF" => Some(f64::INFINITY),
        "-.inf" | "-.Inf" | "-.INF" => Some(f64::NEG_INFINITY),
        ".nan" | ".NaN" | ".NAN" => Some(f64::NAN),
        _ if raw.bytes().any(|byte| byte.is_ascii_digit()) => raw.parse::<f64>().ok(),
        _ => None,
    }
}

/// Native callback for `neo:yaml`. It is passed only to the module factory
/// during module evaluation and is never installed on the global object.
pub(crate) fn parse_js<'js>(ctx: Ctx<'js>, value: Value<'js>) -> JsResult<String> {
    if !value.is_string() {
        return Err(Exception::throw_type(&ctx, "source must be a string"));
    }
    if string_length(&ctx, &value)? > MAX_SOURCE_BYTES {
        return Ok(encode(Err(ParseError::limit())));
    }
    let Some((source, invalid)) =
        decode_js_string(&ctx, &value, MAX_SOURCE_BYTES, "YAML_ENCODING", "YAML_NUL")?
    else {
        return Ok(encode(Err(ParseError::limit())));
    };
    if source.len() > MAX_SOURCE_BYTES {
        return Ok(encode(Err(ParseError::limit())));
    }
    Ok(decoded_response(&source, invalid))
}

/// Persistent factory for the private `neo:yaml` wrapper.
#[derive(Default)]
pub(crate) struct YamlState {
    pub(crate) factory: Option<Persistent<Function<'static>>>,
}

// YamlState contains no borrowed JS value; its persistent handle is explicitly
// released before the context and runtime are dropped.
unsafe impl<'js> rquickjs::JsLifetime<'js> for YamlState {
    type Changed<'to> = YamlState;
}

/// The native `neo:yaml` module.
pub(crate) struct YamlModule;

impl ModuleDef for YamlModule {
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
                .userdata::<Rc<RefCell<YamlState>>>()
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
        let limits = format!(
            r#"{{"MAX_SOURCE_BYTES":{MAX_SOURCE_BYTES},"MAX_DEPTH":{MAX_DEPTH},"MAX_NODES":{MAX_NODES},"MAX_ITEMS":{MAX_ITEMS},"MAX_OUTPUT_BYTES":{MAX_OUTPUT_BYTES}}}"#
        );
        let api: Object = factory.call((parse_native, "YamlParseError", MESSAGES, limits))?;
        for name in EXPORTS {
            exports.export(name, api.get::<_, Value>(name)?)?;
        }
        Ok(())
    }
}

/// Build the private wrapper in the fresh realm, before user code runs.
pub(crate) fn factory<'js>(ctx: &Ctx<'js>) -> JsResult<Function<'js>> {
    ctx.eval(include_str!("js/structured.js"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(source: &str) -> Json {
        parse(source).unwrap()
    }

    fn error(source: &str) -> ParseError {
        parse(source).unwrap_err()
    }

    #[test]
    fn parses_nested_types_and_unicode() {
        let value = parse_ok(
            r#"
            title: héllo ☃
            count: 42
            ratio: 1.5
            enabled: true
            nothing: []
            server:
              host: localhost
              ports:
                - 80
                - 443
            items:
              - name: one
              - name: two
            "#,
        );
        assert_eq!(value["title"], "héllo ☃");
        assert_eq!(value["count"], 42);
        assert_eq!(value["ratio"], 1.5);
        assert_eq!(value["enabled"], true);
        assert_eq!(value["server"]["ports"][1], 443);
        assert_eq!(value["items"][1]["name"], "two");
    }

    #[test]
    fn empty_source_is_null() {
        assert_eq!(parse_ok(""), Json::Null);
        assert_eq!(parse_ok("---"), Json::Null);
    }

    #[test]
    fn leading_bom_is_ignored() {
        assert_eq!(parse_ok("\u{feff}a: 1")["a"], 1);
    }

    #[test]
    fn rejects_aliases_before_expanding_them() {
        assert_eq!(error("a: &a [1, 2]\nb: *a\n").code, "YAML_ALIAS");
    }

    #[test]
    fn rejects_tags() {
        assert_eq!(error("a: !!str 1").code, "YAML_TAG");
        assert_eq!(error("a: !custom 1").code, "YAML_TAG");
    }

    #[test]
    fn rejects_non_string_and_complex_keys() {
        assert_eq!(error("1: value").code, "YAML_KEY");
        assert_eq!(error("true: value").code, "YAML_KEY");
        assert_eq!(error("~: value").code, "YAML_KEY");
        assert_eq!(error("? [a, b]\n: value").code, "YAML_KEY");
    }

    #[test]
    fn rejects_duplicate_string_keys() {
        assert_eq!(error("a: 1\na: 2").code, "YAML_DUPLICATE");
        assert_eq!(error("a: 1\n\"a\": 2").code, "YAML_DUPLICATE");
    }

    #[test]
    fn rejects_multiple_documents() {
        assert_eq!(error("---\na: 1\n---\nb: 2\n").code, "YAML_MULTIDOC");
    }

    #[test]
    fn rejects_integers_outside_the_exact_range() {
        assert_eq!(error("a: 9007199254740992").code, "YAML_INTEGER");
        assert_eq!(error("a: 99999999999999999999999999").code, "YAML_INTEGER");
        assert_eq!(parse_ok("a: 9007199254740991")["a"], 9007199254740991i64);
    }

    #[test]
    fn rejects_non_finite_floats() {
        assert_eq!(error("a: .inf").code, "YAML_FLOAT");
        assert_eq!(error("a: -.inf").code, "YAML_FLOAT");
        assert_eq!(error("a: .nan").code, "YAML_FLOAT");
        assert_eq!(error("a: 1e999").code, "YAML_FLOAT");
    }

    #[test]
    fn date_like_scalars_remain_strings() {
        assert_eq!(parse_ok("a: 2020-01-01")["a"], "2020-01-01");
        assert_eq!(
            parse_ok("a: 2020-01-01T00:00:00Z")["a"],
            "2020-01-01T00:00:00Z"
        );
    }

    #[test]
    fn malformed_input_reports_a_line_and_column() {
        let failure = error("a: [1,\n");
        assert_eq!(failure.code, "YAML_SYNTAX");
        assert!(failure.line >= 1, "{failure:?}");
        assert!(failure.column >= 1, "{failure:?}");
    }

    #[test]
    fn enforces_depth_and_source_limits() {
        let mut nested = String::new();
        for _ in 0..(MAX_DEPTH + 1) {
            nested.push('[');
        }
        nested.push('1');
        for _ in 0..(MAX_DEPTH + 1) {
            nested.push(']');
        }
        assert_eq!(error(&nested).code, "YAML_DEPTH");

        let over = "A".repeat(MAX_SOURCE_BYTES + 1);
        assert_eq!(error(&over).code, "YAML_LIMIT");
    }

    #[test]
    fn response_is_machine_readable_and_never_echoes_values() {
        let ok: serde_json::Value =
            serde_json::from_str(&decoded_response("a: secret", None)).unwrap();
        assert_eq!(ok["ok"], true);
        assert_eq!(ok["value"]["a"], "secret");

        let bad = decoded_response("a: [secret", None);
        assert!(!bad.contains("secret"));
        let bad: serde_json::Value = serde_json::from_str(&bad).unwrap();
        assert_eq!(bad["ok"], false);
        assert_eq!(bad["error"]["code"], "YAML_SYNTAX");
    }
}
