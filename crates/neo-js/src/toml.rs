//! A synchronous, permission-free TOML parser.
//!
//! The parser is given text. It does not read a file, consult the process
//! environment, access the network, or execute anything. Reading a TOML file is
//! a separate, explicit decision through a permission-gated module such as
//! `neo:fs`.
//!
//! Parsing is delegated to the maintained `toml` crate. Conversion to
//! JavaScript values is this module's own: objects are null-prototype records,
//! arrays are arrays, and values are strings, finite numbers, and booleans.
//! TOML date and time values are rejected rather than guessed at, because
//! JavaScript has no built-in exact date/time type. Integers outside
//! ±(2^53 - 1) are rejected because they cannot round-trip through a
//! JavaScript number. Non-finite floats are rejected because JSON and ordinary
//! JavaScript configuration output cannot represent them portably.
//!
//! Limits are 1 MiB of source, 64 collection levels (root counts as one),
//! 100,000 nodes, 100,000 entries per collection, and a bounded output budget.
//! A streaming event pass over the bounded source enforces depth and node
//! budgets before the `toml` crate builds a value tree. The preflight also
//! limits conservative node-counting work to 100,000 units; shared prefixes
//! can consume more than one unit. Header depth is conservative when arrays
//! of tables occur. Preflight limits take precedence over semantic errors.
//! The event pass and the
//! parser allocate a token vector proportional to the source; this is not a
//! zero-allocation parser. Scalar leaves do not add depth. Errors carry a
//! stable code and, where an event or parser span exists, a one-based line and
//! column. Messages never include source text or values.

use rquickjs::{
    Ctx, Exception, Function, Object, Persistent, Result as JsResult, Value,
    module::{Declarations, Exports, ModuleDef},
};
use serde_json::{Map, Number, Value as Json, json};
use toml_parser::{
    Source, Span,
    parser::{EventReceiver, parse_document},
};

use crate::js_text::{InvalidText, decode_js_string, position, string_length};

/// Largest accepted source, in UTF-8 bytes.
pub(crate) const MAX_SOURCE_BYTES: usize = 1024 * 1024;
/// Maximum collection nesting depth, counting the root collection as one.
pub(crate) const MAX_DEPTH: usize = 64;
/// Maximum number of scalar and collection nodes.
pub(crate) const MAX_NODES: usize = 100_000;
/// Maximum conservative preflight node-accounting work, including repeated prefixes.
const MAX_PREFLIGHT_WORK: usize = 100_000;
/// Maximum entries in one array or table.
pub(crate) const MAX_ITEMS: usize = 100_000;
/// Upper bound on the JSON response built for JavaScript, in bytes.
pub(crate) const MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;

/// Largest integer exactly representable in a JavaScript number.
const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

const MESSAGES: &str = r#"{
  "TOML_SYNTAX": "invalid TOML syntax",
  "TOML_DATETIME": "TOML date and time values are not supported",
  "TOML_INTEGER": "TOML integer is outside the exact JavaScript range",
  "TOML_FLOAT": "TOML floating-point value is not finite",
  "TOML_DEPTH": "TOML nesting exceeds the supported depth",
  "TOML_LIMIT": "TOML input exceeds a documented limit",
  "TOML_ENCODING": "TOML source is not valid UTF-16",
  "TOML_NUL": "TOML source contains a NUL character"
}"#;

const EXPORTS: [&str; 8] = [
    "default",
    "parse",
    "TomlParseError",
    "MAX_SOURCE_BYTES",
    "MAX_DEPTH",
    "MAX_NODES",
    "MAX_ITEMS",
    "MAX_OUTPUT_BYTES",
];

/// A deterministic parse failure with a stable code and location.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ParseError {
    /// Stable `TOML_*` code; match on this, never on a message.
    pub(crate) code: &'static str,
    /// One-based source line, or zero when conversion has no source span.
    pub(crate) line: u32,
    /// One-based column in Unicode scalar values, or zero when unknown.
    pub(crate) column: u32,
}

impl ParseError {
    const fn new(code: &'static str, line: u32, column: u32) -> Self {
        Self { code, line, column }
    }

    const fn limit() -> Self {
        Self::new("TOML_LIMIT", 1, 1)
    }

    const fn semantic(code: &'static str) -> Self {
        Self::new(code, 0, 0)
    }
}

/// Parse TOML text into a JSON value. Test-facing; the native callback adds
/// string decoding and error ordering.
pub(crate) fn parse(source: &str) -> Result<Json, ParseError> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err(ParseError::limit());
    }
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    // Reject an over-budget document before `toml` builds a value tree. The
    // preflight lexes the bounded source and consumes events; it allocates a
    // token vector proportional to that 1 MiB input but no value tree.
    preflight(source)?;
    let value: toml::Value = toml::from_str(source).map_err(|error| {
        let (line, column) = error
            .span()
            .map(|span| position(source, span.start))
            .unwrap_or((0, 0));
        // The pinned parser has a deeper internal limit than this module.
        // Report its recursion rejection as the module's depth code so a
        // caller does not see two different codes for the same condition.
        let code = if error.message().contains("recursion limit exceeded") {
            "TOML_DEPTH"
        } else {
            "TOML_SYNTAX"
        };
        ParseError::new(code, line, column)
    })?;
    let mut budget = Budget::default();
    convert(&value, 1, &mut budget)
}

/// Enforce depth and node budgets in a streaming event pass over the source.
///
/// The event receiver counts collections and values without building a tree.
/// Work accounting is conservative: a table prefix shared by several headers
/// or dotted keys, and the array container of a repeated
/// `[[array-of-tables]]` header, may be counted more than once. It never
/// undercounts, so this pass can reject a few nodes early but cannot admit an
/// over-limit tree.
fn preflight(source: &str) -> Result<(), ParseError> {
    let source_handle = Source::new(source);
    let tokens = source_handle.lex().into_vec();
    let mut budget = TomlBudget::new();
    let mut syntax: Option<toml_parser::ParseError> = None;
    parse_document(&tokens, &mut budget, &mut syntax);
    // The receiver cannot detect semantic errors such as duplicate keys.
    // Limits therefore take precedence, even over earlier semantic errors.
    if let Some((code, offset)) = budget.failure {
        let (line, column) = position(source, offset);
        return Err(ParseError::new(code, line, column));
    }
    if let Some(offset) = budget.syntax_failure {
        let (line, column) = position(source, offset);
        return Err(ParseError::new("TOML_SYNTAX", line, column));
    }
    if let Some(error) = syntax {
        let offset = error
            .unexpected()
            .or_else(|| error.context())
            .map_or(0, |span| span.start());
        let (line, column) = position(source, offset);
        return Err(ParseError::new("TOML_SYNTAX", line, column));
    }
    Ok(())
}

/// Streaming budget counter for the TOML event preflight.
///
/// The node budget is a stricter aggregate than the per-collection item
/// limit: a collection with more than `MAX_ITEMS` entries necessarily
/// contributes more than `MAX_ITEMS` value nodes, so an over-wide collection
/// is rejected here before the tree exists.
struct TomlBudget {
    depth: usize,
    depths: Vec<usize>,
    nodes: usize,
    pending_depth: Option<usize>,
    in_header: bool,
    header_components: usize,
    array_headers: usize,
    in_key: bool,
    key_components: usize,
    failure: Option<(&'static str, usize)>,
    syntax_failure: Option<usize>,
}

impl TomlBudget {
    fn new() -> Self {
        Self {
            depth: 1,
            depths: Vec::new(),
            nodes: 1,
            pending_depth: None,
            in_header: false,
            header_components: 0,
            array_headers: 0,
            in_key: false,
            key_components: 0,
            failure: None,
            syntax_failure: None,
        }
    }

    fn fail(&mut self, code: &'static str, span: Span) {
        if self.failure.is_none() {
            self.failure = Some((code, span.start()));
        }
    }

    fn check_depth(&mut self, depth: usize, span: Span) {
        if depth > MAX_DEPTH {
            self.fail("TOML_DEPTH", span);
        }
    }

    fn add_nodes(&mut self, count: usize, span: Span) {
        self.nodes = self.nodes.saturating_add(count);
        if self.nodes > MAX_PREFLIGHT_WORK {
            self.fail("TOML_LIMIT", span);
        }
    }

    fn open_collection(&mut self, span: Span) {
        let depth = self
            .pending_depth
            .take()
            .unwrap_or_else(|| self.depth.saturating_add(1));
        self.depths.push(self.depth);
        self.depth = depth;
        self.check_depth(self.depth, span);
        self.add_nodes(1, span);
    }

    fn close_collection(&mut self) {
        self.depth = self.depths.pop().unwrap_or(1);
    }

    fn close_header(&mut self, span: Span, array: bool) {
        if !self.in_header {
            return;
        }
        let depth = self
            .header_components
            .saturating_add(if array { 2 } else { 1 })
            // Each earlier array header might be a distinct ancestor. Count
            // no more than the number of possible ancestor components.
            .saturating_add(
                self.array_headers
                    .min(self.header_components.saturating_sub(1)),
            );
        self.check_depth(depth, span);
        self.add_nodes(self.header_components.saturating_sub(1), span);
        self.depth = depth;
        self.in_header = false;
        self.in_key = false;
        self.pending_depth = None;
    }
}

impl EventReceiver for TomlBudget {
    fn std_table_open(&mut self, span: Span, _error: &mut dyn toml_parser::ErrorSink) {
        self.in_header = true;
        self.header_components = 0;
        self.in_key = false;
        self.pending_depth = None;
        self.add_nodes(1, span);
    }

    fn std_table_close(&mut self, span: Span, _error: &mut dyn toml_parser::ErrorSink) {
        self.close_header(span, false);
    }

    fn array_table_open(&mut self, span: Span, _error: &mut dyn toml_parser::ErrorSink) {
        self.in_header = true;
        self.header_components = 0;
        self.in_key = false;
        self.pending_depth = None;
        // An array-of-tables header contributes the array container and one
        // entry table. Counting both on every header is conservative when the
        // array is reopened; it never undercounts.
        self.add_nodes(2, span);
    }

    fn array_table_close(&mut self, span: Span, _error: &mut dyn toml_parser::ErrorSink) {
        self.close_header(span, true);
        self.array_headers = self.array_headers.saturating_add(1);
    }

    fn inline_table_open(&mut self, span: Span, _error: &mut dyn toml_parser::ErrorSink) -> bool {
        self.open_collection(span);
        self.failure.is_none()
    }

    fn inline_table_close(&mut self, _span: Span, _error: &mut dyn toml_parser::ErrorSink) {
        self.close_collection();
    }

    fn array_open(&mut self, span: Span, _error: &mut dyn toml_parser::ErrorSink) -> bool {
        self.open_collection(span);
        self.failure.is_none()
    }

    fn array_close(&mut self, _span: Span, _error: &mut dyn toml_parser::ErrorSink) {
        self.close_collection();
    }

    fn simple_key(
        &mut self,
        _span: Span,
        _kind: Option<toml_parser::decoder::Encoding>,
        _error: &mut dyn toml_parser::ErrorSink,
    ) {
        if self.in_header {
            self.header_components = self.header_components.saturating_add(1);
        } else if self.in_key {
            self.key_components = self.key_components.saturating_add(1);
        } else {
            self.in_key = true;
            self.key_components = 1;
        }
    }

    fn key_val_sep(&mut self, span: Span, _error: &mut dyn toml_parser::ErrorSink) {
        if !self.in_key {
            return;
        }
        // A dotted key creates `key_components - 1` implicit tables. The value
        // itself, if it is a collection, sits one level below that.
        let implicit_depth = self
            .depth
            .saturating_add(self.key_components.saturating_sub(1));
        self.check_depth(implicit_depth, span);
        self.add_nodes(self.key_components.saturating_sub(1), span);
        self.pending_depth = Some(self.depth.saturating_add(self.key_components));
        self.in_key = false;
    }

    fn scalar(
        &mut self,
        span: Span,
        _kind: Option<toml_parser::decoder::Encoding>,
        _error: &mut dyn toml_parser::ErrorSink,
    ) {
        self.pending_depth = None;
        self.add_nodes(1, span);
    }

    fn value_sep(&mut self, _span: Span, _error: &mut dyn toml_parser::ErrorSink) {
        self.pending_depth = None;
    }

    fn whitespace(&mut self, _span: Span, _error: &mut dyn toml_parser::ErrorSink) {}

    fn comment(&mut self, _span: Span, _error: &mut dyn toml_parser::ErrorSink) {}

    fn newline(&mut self, _span: Span, _error: &mut dyn toml_parser::ErrorSink) {}

    fn error(&mut self, span: Span, _error: &mut dyn toml_parser::ErrorSink) {
        self.syntax_failure.get_or_insert(span.start());
    }
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

/// Prefer the earlier concrete error. A conversion error has no source span
/// (line zero), so a real encoding error at a known position wins over it.
fn choose(
    parsed: Result<Json, ParseError>,
    invalid: Option<ParseError>,
) -> Result<Json, ParseError> {
    match (parsed, invalid) {
        (Ok(value), None) => Ok(value),
        (Ok(_), Some(invalid)) => Err(invalid),
        (Err(error), None) => Err(error),
        (Err(error), Some(invalid)) => {
            let error_position = if error.line == 0 {
                (u32::MAX, u32::MAX)
            } else {
                (error.line, error.column)
            };
            if (invalid.line, invalid.column) <= error_position {
                Err(invalid)
            } else {
                Err(error)
            }
        }
    }
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
    /// Count one value node. Scalar leaves do not carry a depth: depth is a
    /// property of collections, and a scalar can sit inside the deepest
    /// permitted collection.
    fn node(&mut self) -> Result<(), ParseError> {
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes > MAX_NODES {
            return Err(ParseError::semantic("TOML_LIMIT"));
        }
        self.charge(24)
    }

    /// Count one collection and enforce its depth. The root collection is at
    /// depth one; a nested collection increments the depth of its parent.
    fn collection(&mut self, depth: usize) -> Result<(), ParseError> {
        if depth > MAX_DEPTH {
            return Err(ParseError::semantic("TOML_DEPTH"));
        }
        self.node()
    }

    fn item(&mut self) -> Result<(), ParseError> {
        self.charge(8)
    }

    fn string(&mut self, bytes: usize) -> Result<(), ParseError> {
        self.strings = self.strings.saturating_add(bytes);
        if self.strings > MAX_SOURCE_BYTES {
            return Err(ParseError::semantic("TOML_LIMIT"));
        }
        self.charge(bytes.saturating_mul(6).saturating_add(16))
    }

    fn charge(&mut self, bytes: usize) -> Result<(), ParseError> {
        let Some(remaining) = self.output.checked_sub(bytes) else {
            return Err(ParseError::semantic("TOML_LIMIT"));
        };
        self.output = remaining;
        Ok(())
    }
}

fn convert(value: &toml::Value, depth: usize, budget: &mut Budget) -> Result<Json, ParseError> {
    match value {
        toml::Value::String(text) => {
            budget.node()?;
            budget.string(text.len())?;
            Ok(Json::String(text.clone()))
        }
        toml::Value::Integer(number) => {
            budget.node()?;
            if !(-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(number) {
                Err(ParseError::semantic("TOML_INTEGER"))
            } else {
                Ok(Json::Number(Number::from(*number)))
            }
        }
        toml::Value::Float(number) => {
            budget.node()?;
            if !number.is_finite() {
                return Err(ParseError::semantic("TOML_FLOAT"));
            }
            Number::from_f64(*number)
                .map(Json::Number)
                .ok_or_else(|| ParseError::semantic("TOML_FLOAT"))
        }
        toml::Value::Boolean(value) => {
            budget.node()?;
            Ok(Json::Bool(*value))
        }
        toml::Value::Datetime(_) => {
            budget.node()?;
            Err(ParseError::semantic("TOML_DATETIME"))
        }
        toml::Value::Array(values) => {
            budget.collection(depth)?;
            if values.len() > MAX_ITEMS {
                return Err(ParseError::semantic("TOML_LIMIT"));
            }
            let mut output = Vec::with_capacity(values.len());
            for value in values {
                budget.item()?;
                output.push(convert(value, depth.saturating_add(1), budget)?);
            }
            Ok(Json::Array(output))
        }
        toml::Value::Table(table) => {
            budget.collection(depth)?;
            if table.len() > MAX_ITEMS {
                return Err(ParseError::semantic("TOML_LIMIT"));
            }
            let mut output = Map::new();
            for (key, value) in table {
                budget.string(key.len())?;
                budget.item()?;
                output.insert(
                    key.clone(),
                    convert(value, depth.saturating_add(1), budget)?,
                );
            }
            Ok(Json::Object(output))
        }
    }
}

/// Native callback for `neo:toml`. It is passed only to the module factory
/// during module evaluation and is never installed on the global object.
pub(crate) fn parse_js<'js>(ctx: Ctx<'js>, value: Value<'js>) -> JsResult<String> {
    if !value.is_string() {
        return Err(Exception::throw_type(&ctx, "source must be a string"));
    }
    if string_length(&ctx, &value)? > MAX_SOURCE_BYTES {
        return Ok(encode(Err(ParseError::limit())));
    }
    let Some((source, invalid)) =
        decode_js_string(&ctx, &value, MAX_SOURCE_BYTES, "TOML_ENCODING", "TOML_NUL")?
    else {
        return Ok(encode(Err(ParseError::limit())));
    };
    if source.len() > MAX_SOURCE_BYTES {
        return Ok(encode(Err(ParseError::limit())));
    }
    Ok(decoded_response(&source, invalid))
}

/// Persistent factory for the private `neo:toml` wrapper.
#[derive(Default)]
pub(crate) struct TomlState {
    pub(crate) factory: Option<Persistent<Function<'static>>>,
}

// TomlState contains no borrowed JS value; its persistent handle is explicitly
// released before the context and runtime are dropped.
unsafe impl<'js> rquickjs::JsLifetime<'js> for TomlState {
    type Changed<'to> = TomlState;
}

/// The native `neo:toml` module.
pub(crate) struct TomlModule;

impl ModuleDef for TomlModule {
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
                .userdata::<Rc<RefCell<TomlState>>>()
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
        let api: Object = factory.call((parse_native, "TomlParseError", MESSAGES, limits))?;
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
            title = "héllo ☃"
            count = 42
            ratio = 1.5
            enabled = true
            nothing = []
            [server]
            host = "localhost"
            ports = [80, 443]
            [[items]]
            name = "one"
            [[items]]
            name = "two"
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
    fn empty_source_is_an_empty_table() {
        assert_eq!(parse_ok(""), json!({}));
    }

    #[test]
    fn leading_bom_is_ignored() {
        assert_eq!(parse_ok("\u{feff}a = 1")["a"], 1);
    }

    #[test]
    fn rejects_integers_outside_the_exact_range() {
        assert_eq!(error("a = 9007199254740992").code, "TOML_INTEGER");
        assert_eq!(error("a = -9007199254740992").code, "TOML_INTEGER");
        assert_eq!(parse_ok("a = 9007199254740991")["a"], 9007199254740991i64);
        assert_eq!(parse_ok("a = -9007199254740991")["a"], -9007199254740991i64);
    }

    #[test]
    fn rejects_non_finite_floats() {
        assert_eq!(error("a = inf").code, "TOML_FLOAT");
        assert_eq!(error("a = -inf").code, "TOML_FLOAT");
        assert_eq!(error("a = nan").code, "TOML_FLOAT");
    }

    #[test]
    fn rejects_date_and_time_values() {
        assert_eq!(error("a = 1979-05-27T07:32:00Z").code, "TOML_DATETIME");
        assert_eq!(error("a = 1979-05-27").code, "TOML_DATETIME");
        assert_eq!(error("a = 07:32:00").code, "TOML_DATETIME");
    }

    #[test]
    fn rejects_duplicate_keys_as_syntax_errors() {
        let failure = error("a = 1\na = 2\n");
        assert_eq!(failure.code, "TOML_SYNTAX");
        assert!(failure.line >= 1);
    }

    #[test]
    fn malformed_input_reports_a_line_and_column() {
        let failure = error("a = [1,\n");
        assert_eq!(failure.code, "TOML_SYNTAX");
        assert!(failure.line >= 1, "{failure:?}");
        assert!(failure.column >= 1, "{failure:?}");
    }

    fn nested_arrays(arrays: usize, leaf: &str) -> String {
        format!("a = {} {leaf} {}", "[".repeat(arrays), "]".repeat(arrays))
    }

    #[test]
    fn depth_counts_collections_not_scalar_leaves() {
        // Root collection plus 63 nested arrays is 64 collections; a scalar
        // leaf does not add a 65th.
        assert!(parse(&nested_arrays(MAX_DEPTH - 1, "1")).is_ok());
        assert!(parse(&nested_arrays(MAX_DEPTH - 1, "")).is_ok());

        // Root plus 64 nested arrays is 65 collections.
        assert_eq!(error(&nested_arrays(MAX_DEPTH, "1")).code, "TOML_DEPTH");
        assert_eq!(error(&nested_arrays(MAX_DEPTH, "")).code, "TOML_DEPTH");

        // Inline tables follow the same collection rule.
        let inline = |tables: usize, leaf: &str| {
            let mut source = String::from("a = ");
            for index in 0..tables {
                source.push_str(&format!("{{k{index} = "));
            }
            source.push_str(leaf);
            for _ in 0..tables {
                source.push('}');
            }
            source
        };
        assert!(parse(&inline(MAX_DEPTH - 1, "1")).is_ok());
        // 62 keyed inline tables plus one empty leaf is 63 collections.
        assert!(parse(&inline(MAX_DEPTH - 2, "{}")).is_ok());
        assert_eq!(error(&inline(MAX_DEPTH, "1")).code, "TOML_DEPTH");
        assert_eq!(error(&inline(MAX_DEPTH - 1, "{}")).code, "TOML_DEPTH");

        // The same rule for table headers and dotted keys.
        let header = |components: usize| {
            let path = (0..components)
                .map(|index| format!("k{index}"))
                .collect::<Vec<_>>()
                .join(".");
            format!("[{path}]\nx = 1\n")
        };
        assert!(parse(&header(MAX_DEPTH - 1)).is_ok());
        assert_eq!(error(&header(MAX_DEPTH)).code, "TOML_DEPTH");

        let dotted = |components: usize| {
            let path = (0..components)
                .map(|index| format!("k{index}"))
                .collect::<Vec<_>>()
                .join(".");
            format!("{path} = 1\n")
        };
        assert!(parse(&dotted(MAX_DEPTH)).is_ok());
        assert_eq!(error(&dotted(MAX_DEPTH + 1)).code, "TOML_DEPTH");

        // Deeper than the parser's own limit is still reported as depth.
        let too_deep = format!("a = {}1{}", "[".repeat(100), "]".repeat(100));
        assert_eq!(error(&too_deep).code, "TOML_DEPTH");
    }

    #[test]
    fn preflight_accepts_common_toml_1_0_constructs() {
        let source = r#"
            title = "TOML"
            multiline = """
            line
            """
            dotted.key = 1
            [a.b]
            x = [1, 2, { y = 3 }]
            [[items]]
            n = 1
            [[items]]
            n = 2
        "#;
        preflight(source).unwrap();
        assert!(parse_ok(source)["items"][1]["n"] == 2);
    }

    #[test]
    fn preflight_enforces_budgets_before_building_a_tree() {
        let deep = nested_arrays(MAX_DEPTH, "1");
        let failure = preflight(&deep).unwrap_err();
        assert_eq!(failure.code, "TOML_DEPTH");
        assert_eq!((failure.line, failure.column), (1, 68));

        let late = format!("x = 1\ny = 2\n{deep}");
        let failure = preflight(&late).unwrap_err();
        assert_eq!(failure.code, "TOML_DEPTH");
        assert_eq!((failure.line, failure.column), (3, 68));

        let wide = format!("a = [{}]", "0,".repeat(MAX_NODES) + "0");
        let failure = preflight(&wide).unwrap_err();
        assert_eq!(failure.code, "TOML_LIMIT");
        assert!(failure.column > 1, "{failure:?}");
        assert_eq!(error(&wide).code, "TOML_LIMIT");
    }

    #[test]
    fn preflight_limits_take_precedence_over_semantic_errors() {
        let source = format!("x = 1\nx = 2\ny = [{}]", "0,".repeat(MAX_NODES));
        assert_eq!(parse(&source).unwrap_err().code, "TOML_LIMIT");
        assert_eq!(parse("x = 1\nx = 2\n").unwrap_err().code, "TOML_SYNTAX");
    }

    #[test]
    fn shared_prefixes_consume_preflight_work() {
        let source: String = (0..50_000).map(|n| format!("a.k{n} = 1\n")).collect();
        assert_eq!(preflight(&source).unwrap_err().code, "TOML_LIMIT");
    }

    #[test]
    fn header_depth_accounts_for_array_ancestors() {
        let mut source = String::new();
        let mut path = Vec::new();
        for n in 0..31 {
            path.push(format!("k{n}"));
            source.push_str(&format!("[[{}]]\n", path.join(".")));
        }
        path.push("leaf".into());
        let at_limit = format!("{source}[{}]\n", path.join("."));
        assert!(preflight(&at_limit).is_ok());
        assert!(parse(&at_limit).is_ok());
        path.push("over".into());
        let over = format!("{source}[{}]\n", path.join("."));
        assert_eq!(preflight(&over).unwrap_err().code, "TOML_DEPTH");
    }

    #[test]
    fn preflight_syntax_failure_is_not_ignored() {
        let source = "x == 1\n";
        assert_eq!(preflight(source).unwrap_err().code, "TOML_SYNTAX");
        assert_eq!(parse(source).unwrap_err().code, "TOML_SYNTAX");
    }

    #[test]
    fn hostile_depth_is_bounded_in_subprocess() {
        const FLAG: &str = "NEO_TOML_DEPTH_CHILD";
        if std::env::var_os(FLAG).is_some() {
            let deep = format!("x = {}0{}", "[".repeat(50_000), "]".repeat(50_000));
            assert_eq!(parse(&deep).unwrap_err().code, "TOML_DEPTH");
            return;
        }
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "toml::tests::hostile_depth_is_bounded_in_subprocess",
            ])
            .env(FLAG, "1")
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success(), "child failed: {status}");
                break;
            }
            if std::time::Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("hostile-depth child exceeded watchdog");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn enforces_source_limits() {
        let over = "A".repeat(MAX_SOURCE_BYTES + 1);
        assert_eq!(error(&over).code, "TOML_LIMIT");
    }

    #[test]
    fn response_is_machine_readable_and_never_echoes_values() {
        let ok: serde_json::Value =
            serde_json::from_str(&decoded_response("a = 'secret'", None)).unwrap();
        assert_eq!(ok["ok"], true);
        assert_eq!(ok["value"]["a"], "secret");

        let bad = decoded_response("a = 'secret", None);
        assert!(!bad.contains("secret"));
        let bad: serde_json::Value = serde_json::from_str(&bad).unwrap();
        assert_eq!(bad["ok"], false);
        assert_eq!(bad["error"]["code"], "TOML_SYNTAX");
    }
}
