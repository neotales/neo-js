//! Synchronous, private in-memory SQLite. Native handles never retain a JS
//! value, and SQLite callbacks never call into JavaScript.

use std::{
    cell::{Cell, RefCell},
    mem::MaybeUninit,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use rquickjs::{
    Array, BigInt, Class, Ctx, Error, Function, IntoJs, JsLifetime, Object, Persistent,
    Result as JsResult, TypedArray, Value,
    class::{JsClass, Trace, Tracer, Writable},
    function::{Constructor, Opt, This},
    module::{Declarations, Exports, ModuleDef},
    object::Property,
    qjs,
};
use rusqlite::{
    Connection, OpenFlags, PrepFlags, Statement,
    config::DbConfig,
    ffi,
    hooks::{AuthAction, AuthContext, Authorization},
    limits::Limit,
    types::{Value as SqlValue, ValueRef},
};

const SQL_BYTES: usize = 64 * 1024;
const VALUE_BYTES: usize = 1024 * 1024;
const PARAMS: usize = 64;
const COLUMN_COUNT: usize = 64;
const RESULT_BYTES: usize = 4 * VALUE_BYTES;
const RESULT_ROWS: usize = 1024;
const LIVE_DATABASES: usize = 8;
const PROGRESS_INTERVAL: i32 = 100;
const PROGRESS_CALLS: usize = 2_000;
const PROGRESS_TIME: Duration = Duration::from_millis(250);
const PREP_FLAGS: PrepFlags =
    PrepFlags::SQLITE_PREPARE_NO_VTAB.union(PrepFlags::SQLITE_PREPARE_DONT_LOG);

#[derive(Clone, Copy)]
enum Failure {
    Argument,
    Encoding,
    Limit,
    Work,
    Denied,
    Closed,
    Constraint,
    Sql,
    Open,
}

impl Failure {
    /// Produce an error using only static, module-owned strings.
    fn throw(self, ctx: &Ctx<'_>) -> Error {
        let (code, message) = match self {
            Self::Argument => (
                "SQLITE_ARGUMENT",
                "invalid SQLite argument or statement kind",
            ),
            Self::Encoding => ("SQLITE_ENCODING", "invalid SQLite text encoding"),
            Self::Limit => ("SQLITE_LIMIT", "SQLite size or count limit exceeded"),
            Self::Work => ("SQLITE_WORK_LIMIT", "SQLite statement work budget exceeded"),
            Self::Denied => ("SQLITE_DENIED", "SQLite action is not permitted"),
            Self::Closed => ("SQLITE_CLOSED", "SQLite database is closed"),
            Self::Constraint => ("SQLITE_CONSTRAINT", "SQLite constraint failed"),
            Self::Sql => ("SQLITE_SQL", "SQLite could not complete the statement"),
            Self::Open => (
                "SQLITE_OPEN",
                "SQLite could not initialize an in-memory database",
            ),
        };
        let result = (|| {
            let state = sqlite_state(ctx)?;
            let class = state.borrow().error_class.ok_or(Error::Unknown)?;
            drop(state);
            // SAFETY: this context supplied the built-in Error class ID
            // during initialization. Unlike JS_NewError, class allocation
            // does not build a backtrace or consult the stack-trace hooks.
            let error = unsafe { qjs::JS_NewObjectClass(ctx.as_raw().as_ptr(), class) };
            if unsafe { qjs::JS_IsException(error) } {
                return Err(Error::Exception);
            }
            let object = Object::from_value(unsafe { Value::from_raw(ctx.clone(), error) })?;
            object.prop("name", "SqliteError")?;
            object.prop("message", Property::from(message).configurable().writable())?;
            object.prop("code", Property::from(code).enumerable())?;
            // Suppress the per-Error stack. QuickJS can still build a
            // context backtrace while propagating the exception.
            object.prop("stack", Value::new_undefined(ctx.clone()))?;
            Ok(ctx.throw(object.into_value()))
        })();
        result.unwrap_or_else(|error| error)
    }

    fn from_rc(rc: i32) -> Self {
        match rc & 0xff {
            ffi::SQLITE_AUTH | ffi::SQLITE_PERM => Self::Denied,
            ffi::SQLITE_INTERRUPT => Self::Work,
            ffi::SQLITE_TOOBIG | ffi::SQLITE_FULL | ffi::SQLITE_NOMEM => Self::Limit,
            ffi::SQLITE_CONSTRAINT => Self::Constraint,
            _ => Self::Sql,
        }
    }
}

impl From<rusqlite::Error> for Failure {
    fn from(error: rusqlite::Error) -> Self {
        match error {
            rusqlite::Error::SqliteFailure(error, _)
            | rusqlite::Error::SqlInputError { error, .. } => Self::from_rc(error.extended_code),
            _ => Self::Sql,
        }
    }
}

#[derive(Default)]
pub(crate) struct SqliteState {
    blob_validator: Option<Persistent<Function<'static>>>,
    error_class: Option<qjs::JSClassID>,
    live: Rc<Cell<usize>>,
}

// The persistent validator is explicitly released in NeoJs::drop.
unsafe impl<'js> JsLifetime<'js> for SqliteState {
    type Changed<'to> = Self;
}

impl SqliteState {
    /// Capture only pristine intrinsics, before evaluating any user code.
    pub(crate) fn install(ctx: &Ctx<'_>, state: &Rc<RefCell<Self>>) -> JsResult<()> {
        let factory: Function = ctx.eval(
            r#"(isArrayBuffer => {
                const descriptor = Object.getOwnPropertyDescriptor;
                const apply = Reflect.apply;
                const typed = Object.getPrototypeOf(Uint8Array.prototype);
                const arrayBuffer = ArrayBuffer.prototype;
                const buffer = descriptor(typed, 'buffer').get;
                const detached = descriptor(arrayBuffer, 'detached').get;
                const resizable = descriptor(arrayBuffer, 'resizable').get;
                return value => {
                    const backing = apply(buffer, value, []);
                    if (!isArrayBuffer(backing)) return false;
                    return !apply(detached, backing, []) && !apply(resizable, backing, []);
                };
            })"#,
        )?;
        let validator: Function = factory.call((Function::new(ctx.clone(), is_array_buffer)?,))?;
        let error: Value = ctx.eval("new Error()")?;
        let class = unsafe { qjs::JS_GetClassID(error.as_raw()) };
        ctx.store_userdata(Rc::clone(state))
            .map_err(|_| Error::Unknown)?;
        let mut state = state.borrow_mut();
        state.error_class = Some(class);
        state.blob_validator = Some(Persistent::save(ctx, validator));
        Ok(())
    }

    /// Release persistent values before the context and runtime are dropped.
    pub(crate) fn clear(&mut self) {
        self.blob_validator = None;
    }
}

fn sqlite_state(ctx: &Ctx<'_>) -> JsResult<Rc<RefCell<SqliteState>>> {
    ctx.userdata::<Rc<RefCell<SqliteState>>>()
        .map(|state| Rc::clone(&state))
        .ok_or(Error::Unknown)
}

fn is_array_buffer(value: Value<'_>) -> bool {
    // SAFETY: this operation only tests the engine's native backing-store brand.
    unsafe { qjs::JS_IsArrayBuffer(value.as_raw()) }
}

struct Slot(Rc<Cell<usize>>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.set(self.0.get() - 1);
    }
}

struct Database {
    // Drop the connection before releasing its slot.
    owned: Option<(Memory, Slot)>,
}

struct Memory {
    db: Connection,
    denied: Arc<AtomicBool>,
}

unsafe impl<'js> JsLifetime<'js> for Database {
    type Changed<'to> = Self;
}

impl<'js> Trace<'js> for Database {
    fn trace<'a>(&self, _tracer: Tracer<'a, 'js>) {}
}

impl<'js> JsClass<'js> for Database {
    const NAME: &'static str = "Database";
    type Mutable = Writable;

    fn constructor(_ctx: &Ctx<'js>) -> JsResult<Option<Constructor<'js>>> {
        Ok(None)
    }

    fn prototype(ctx: &Ctx<'js>) -> JsResult<Option<Object<'js>>> {
        let proto = Object::new_proto(ctx.clone(), None)?;
        proto.prop("execute", Function::new(ctx.clone(), execute)?)?;
        proto.prop("query", Function::new(ctx.clone(), query)?)?;
        let close = Function::new(ctx.clone(), close)?;
        proto.prop("close", close.clone())?;
        // SAFETY: use the engine's well-known symbol, not mutable global
        // `Symbol`. Defining this own property never executes script code.
        let flags = qjs::JS_PROP_HAS_VALUE | qjs::JS_PROP_THROW;
        let status = unsafe {
            qjs::JS_DefineProperty(
                ctx.as_raw().as_ptr(),
                proto.as_raw(),
                qjs::JS_ATOM_Symbol_dispose as qjs::JSAtom,
                close.as_raw(),
                qjs::JS_UNDEFINED,
                qjs::JS_UNDEFINED,
                flags as i32,
            )
        };
        if status < 0 {
            return Err(Error::Exception);
        }
        Ok(Some(proto))
    }
}

/// Only intended main-schema operations are authorized. Everything else
/// (including unknown operations in future SQLite versions) is denied.
fn authorize(context: AuthContext<'_>) -> Authorization {
    use AuthAction as A;
    // SQLite also sets the accessor for CTEs. Authorize the operation
    // itself regardless of source; triggers and views cannot be created.
    // Plain transactions cannot nest. Deny savepoints (the default arm)
    // so repeated calls cannot accumulate a persistent savepoint stack.
    let allowed = match context.action {
        A::Select | A::Recursive | A::Transaction { .. } => true,
        A::CreateTable { .. }
        | A::CreateIndex { .. }
        | A::DropTable { .. }
        | A::DropIndex { .. }
        | A::Reindex { .. }
        | A::Insert { .. }
        | A::Delete { .. }
        | A::Update { .. } => context.database_name == Some("main"),
        A::Read { column_name, .. } => {
            // SQLite's synthetic read for e.g. count(*) leaves the database
            // null for an unqualified table name, and the column name empty.
            // No temp or attached tables can be created by this connection.
            context.database_name == Some("main")
                || (context.database_name.is_none() && column_name.is_empty())
        }
        A::Function { function_name } => [
            "abs",
            "avg",
            "coalesce",
            "count",
            "hex",
            "ifnull",
            "length",
            "lower",
            "max",
            "min",
            "nullif",
            "round",
            "substr",
            "substring",
            "sum",
            "total",
            "typeof",
            "unhex",
            "upper",
            "zeroblob",
        ]
        .iter()
        .any(|name| name.eq_ignore_ascii_case(function_name)),
        _ => false,
    };
    if allowed {
        Authorization::Allow
    } else {
        Authorization::Deny
    }
}

/// Configure the connection before it can accept caller SQL.
fn memory_connection() -> Result<Memory, Failure> {
    fn configure() -> rusqlite::Result<Memory> {
        // Use bundled SQLite's memory VFS as a second filesystem barrier,
        // in addition to the private :memory: database and MEMORY temp store.
        let db = Connection::open_in_memory_with_flags_and_vfs(
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_PRIVATE_CACHE,
            "memdb",
        )?;
        db.busy_timeout(Duration::ZERO)?;
        use DbConfig as C;
        for (config, enabled) in [
            (C::SQLITE_DBCONFIG_DEFENSIVE, true),
            (C::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false),
            (C::SQLITE_DBCONFIG_WRITABLE_SCHEMA, false),
            (C::SQLITE_DBCONFIG_ENABLE_TRIGGER, false),
            (C::SQLITE_DBCONFIG_ENABLE_VIEW, false),
            (C::SQLITE_DBCONFIG_ENABLE_FTS3_TOKENIZER, false),
            (C::SQLITE_DBCONFIG_ENABLE_ATTACH_CREATE, false),
            (C::SQLITE_DBCONFIG_ENABLE_ATTACH_WRITE, false),
            (C::SQLITE_DBCONFIG_ENABLE_FKEY, true),
            (C::SQLITE_DBCONFIG_DQS_DDL, false),
            (C::SQLITE_DBCONFIG_DQS_DML, false),
        ] {
            if db.set_db_config(config, enabled)? != enabled {
                return Err(rusqlite::Error::InvalidQuery);
            }
        }
        // SAFETY: this handle is live and uniquely owned. Unlike the
        // load-extension db_config verb, this disables both the C and SQL APIs.
        if unsafe { ffi::sqlite3_enable_load_extension(db.handle(), 0) } != ffi::SQLITE_OK {
            return Err(rusqlite::Error::InvalidQuery);
        }
        // Temp B-trees, sorts and statement journals must never spill to
        // files. Check the policy actually took effect on this build.
        db.execute_batch("PRAGMA temp_store=MEMORY; PRAGMA main.page_size=4096;")?;
        let temp: i32 = db.query_row("PRAGMA temp_store", [], |row| row.get(0))?;
        let pages: i32 = db.query_row("PRAGMA main.max_page_count=2048", [], |row| row.get(0))?;
        let page_size: i32 = db.query_row("PRAGMA main.page_size", [], |row| row.get(0))?;
        if (temp, pages, page_size) != (2, 2048, 4096) {
            return Err(rusqlite::Error::InvalidQuery);
        }
        use Limit as L;
        for (limit, value) in [
            (L::SQLITE_LIMIT_LENGTH, VALUE_BYTES as i32),
            (L::SQLITE_LIMIT_SQL_LENGTH, SQL_BYTES as i32),
            (L::SQLITE_LIMIT_COLUMN, COLUMN_COUNT as i32),
            (L::SQLITE_LIMIT_EXPR_DEPTH, 64),
            (L::SQLITE_LIMIT_COMPOUND_SELECT, 16),
            (L::SQLITE_LIMIT_VDBE_OP, 10_000),
            (L::SQLITE_LIMIT_FUNCTION_ARG, 32),
            (L::SQLITE_LIMIT_ATTACHED, 0),
            (L::SQLITE_LIMIT_LIKE_PATTERN_LENGTH, 512),
            (L::SQLITE_LIMIT_VARIABLE_NUMBER, PARAMS as i32),
            // Foreign-key actions use SQLite's trigger machinery. Allow a
            // modest cascade depth, but still deny user triggers in SQL.
            (L::SQLITE_LIMIT_TRIGGER_DEPTH, 8),
            (L::SQLITE_LIMIT_WORKER_THREADS, 0),
        ] {
            db.set_limit(limit, value)?;
            if db.limit(limit)? > value {
                return Err(rusqlite::Error::InvalidQuery);
            }
        }
        let denied = Arc::new(AtomicBool::new(false));
        let denied_in_hook = Arc::clone(&denied);
        db.authorizer(Some(move |context: AuthContext<'_>| {
            let result = authorize(context);
            if result == Authorization::Deny {
                // SQLite reports some authorization failures as SQLITE_ERROR
                // (not SQLITE_AUTH), e.g. rejection of SQL functions.
                denied_in_hook.store(true, Ordering::Relaxed);
            }
            result
        }))?;
        Ok(Memory { db, denied })
    }
    configure().map_err(|_| Failure::Open)
}

/// Open a fresh private database, never accepting a path or URI.
fn open_memory<'js>(ctx: Ctx<'js>, extra: Opt<Value<'js>>) -> JsResult<Class<'js, Database>> {
    if extra.0.is_some() {
        return Err(Failure::Argument.throw(&ctx));
    }
    let state = sqlite_state(&ctx)?;
    let live = Rc::clone(&state.borrow().live);
    drop(state);
    if live.get() >= LIVE_DATABASES {
        return Err(Failure::Limit.throw(&ctx));
    }
    let db = memory_connection().map_err(|failure| failure.throw(&ctx))?;
    live.set(live.get() + 1);
    Class::instance(
        ctx,
        Database {
            owned: Some((db, Slot(live))),
        },
    )
}

/// Close and discard the database. Repeated disposal does nothing.
fn close(this: This<Class<'_, Database>>) -> JsResult<()> {
    this.0.try_borrow_mut()?.owned.take();
    Ok(())
}

/// Strict UTF-16 to UTF-8 without normalization or coercion.
fn text<'js>(ctx: &Ctx<'js>, value: &Value<'js>, max: usize) -> JsResult<String> {
    if !value.is_string() {
        return Err(Failure::Argument.throw(ctx));
    }
    if crate::js_text::string_length(ctx, value)? > max {
        return Err(Failure::Limit.throw(ctx));
    }
    let raw = ctx.as_raw().as_ptr();
    let mut count = 0;
    // SAFETY: the string is rooted and verified primitive; the export is
    // owned by QuickJS and freed once after all reads.
    let ptr = unsafe { qjs::JS_ToCStringLenUTF16(raw, &mut count, value.as_raw()) };
    if ptr.is_null() {
        return Err(Error::Exception);
    }
    let units = unsafe { std::slice::from_raw_parts(ptr, count as usize) };
    let decoded = if crate::js_text::utf8_len(units) > max {
        Err(Failure::Limit)
    } else {
        String::from_utf16(units).map_err(|_| Failure::Encoding)
    };
    unsafe { qjs::JS_FreeCStringUTF16(raw, ptr) };
    decoded.map_err(|failure| failure.throw(ctx))
}

/// Read only a real array's own indexed data entries, never JS getters.
fn parameter_at<'js>(ctx: &Ctx<'js>, array: &Value<'js>, index: u32) -> JsResult<Value<'js>> {
    let mut descriptor = MaybeUninit::<qjs::JSPropertyDescriptor>::uninit();
    let raw = ctx.as_raw().as_ptr();
    // SAFETY: the array was branded by JS_IsArray (proxies are not arrays
    // in this C API). Fetching an own descriptor does not invoke its getter.
    let status = unsafe {
        let atom = qjs::JS_NewAtomUInt32(raw, index);
        let status = qjs::JS_GetOwnProperty(raw, descriptor.as_mut_ptr(), array.as_raw(), atom);
        qjs::JS_FreeAtom(raw, atom);
        status
    };
    if status < 0 {
        return Err(Error::Exception);
    }
    if status == 0 {
        return Err(Failure::Argument.throw(ctx));
    }
    // SAFETY: a successful property lookup initializes and transfers its
    // three rooted values, including undefined placeholders.
    let descriptor = unsafe { descriptor.assume_init() };
    let value = unsafe { Value::from_raw(ctx.clone(), descriptor.value) };
    unsafe {
        qjs::JS_FreeValue(raw, descriptor.getter);
        qjs::JS_FreeValue(raw, descriptor.setter);
    }
    if descriptor.flags & qjs::JS_PROP_TMASK as i32 == qjs::JS_PROP_GETSET as i32 {
        return Err(Failure::Argument.throw(ctx));
    }
    Ok(value)
}

/// Copy only the live range of a fixed, non-shared branded Uint8Array.
fn blob<'js>(ctx: &Ctx<'js>, value: &Value<'js>, max: usize) -> JsResult<Vec<u8>> {
    let raw = ctx.as_raw().as_ptr();
    let tag = unsafe { qjs::JS_GetTypedArrayType(value.as_raw()) };
    if tag != qjs::JSTypedArrayEnum_JS_TYPED_ARRAY_UINT8 as i32 {
        return Err(Failure::Argument.throw(ctx));
    }
    let state = sqlite_state(ctx)?;
    let validator = state
        .borrow()
        .blob_validator
        .clone()
        .ok_or(Error::Unknown)?;
    drop(state);
    // These captured brand/accessor checks do not throw on detached or
    // out-of-bounds views. Calling JS_GetUint8Array on an invalid view would
    // create a TypeError and potentially invoke user stack hooks.
    if !validator.restore(ctx)?.call::<_, bool>((value.clone(),))? {
        return Err(Failure::Argument.throw(ctx));
    }
    let mut len = 0;
    // SAFETY: the rooted value is a genuine, attached, non-shared
    // Uint8Array with fixed backing storage. Its view cannot be out of
    // bounds, and no JS has run since validation.
    let ptr = unsafe { qjs::JS_GetUint8Array(raw, &mut len, value.as_raw()) };
    if ptr.is_null() {
        return Err(Error::Exception);
    }
    let len = len as usize;
    if len > max {
        return Err(Failure::Limit.throw(ctx));
    }
    // SAFETY: the view roots its backing and no engine call runs during
    // the copy. The native pointer and length cover exactly the view.
    Ok(unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec())
}

fn parameters<'js>(ctx: &Ctx<'js>, argument: Opt<Value<'js>>) -> JsResult<Vec<SqlValue>> {
    let Some(array) = argument.0.filter(|value| !value.is_undefined()) else {
        return Ok(Vec::new());
    };
    if !array.is_array() {
        return Err(Failure::Argument.throw(ctx));
    }
    // Real arrays always have their own non-configurable data length.
    let length: u32 = array.as_object().ok_or(Error::Unknown)?.get("length")?;
    if length as usize > PARAMS {
        return Err(Failure::Limit.throw(ctx));
    }
    let mut params = Vec::with_capacity(length as usize);
    let mut remaining = RESULT_BYTES;
    for index in 0..length {
        let value = parameter_at(ctx, &array, index)?;
        let param = if value.is_null() {
            SqlValue::Null
        } else if let Some(number) = value.as_number() {
            if !number.is_finite() {
                return Err(Failure::Argument.throw(ctx));
            }
            SqlValue::Real(number)
        } else if let Some(bigint) = value.as_big_int() {
            let integer = bigint.clone().to_i64()?;
            // QuickJS's int64 conversion truncates modulo 2^64. Comparing
            // with the exact primitive round-trip detects overflow without
            // formatting an unbounded decimal string or running JS.
            let round_trip = BigInt::from_i64(ctx.clone(), integer)?;
            let equal = unsafe {
                qjs::JS_IsStrictEqual(ctx.as_raw().as_ptr(), value.as_raw(), round_trip.as_raw())
            };
            if !equal {
                return Err(Failure::Argument.throw(ctx));
            }
            SqlValue::Integer(integer)
        } else if value.is_string() {
            let string = text(ctx, &value, VALUE_BYTES.min(remaining))?;
            remaining -= string.len();
            SqlValue::Text(string)
        } else {
            let bytes = blob(ctx, &value, VALUE_BYTES.min(remaining))?;
            remaining -= bytes.len();
            SqlValue::Blob(bytes)
        };
        params.push(param);
    }
    Ok(params)
}

/// Prepare once through SQLite to find the first statement's real end. Pass
/// only that range to rusqlite: its built-in tail check recursively prepares
/// the remainder without NO_VTAB or DONT_LOG.
fn single_statement(sql: &str, db: &Connection) -> Result<usize, Failure> {
    let mut stmt = std::ptr::null_mut();
    let mut tail = std::ptr::null();
    // SAFETY: SQLite is given the exact byte length of the live UTF-8 slice.
    // The database is owned, and both out-pointers remain live for the call.
    let rc = unsafe {
        ffi::sqlite3_prepare_v3(
            db.handle(),
            sql.as_ptr().cast(),
            sql.len() as i32,
            PREP_FLAGS.bits(),
            &mut stmt,
            &mut tail,
        )
    };
    if !stmt.is_null() {
        // SAFETY: this statement is local and has never been stepped.
        unsafe { ffi::sqlite3_finalize(stmt) };
    }
    if rc != ffi::SQLITE_OK {
        return Err(Failure::from_rc(rc));
    }
    if stmt.is_null() || tail.is_null() {
        return Err(Failure::Argument);
    }
    // SAFETY: on success the documented tail points into the caller's
    // supplied input (possibly exactly one byte past its end).
    let end = unsafe { tail.cast::<u8>().offset_from(sql.as_ptr()) } as usize;
    if end > sql.len() || !sql.as_bytes()[end..].iter().all(u8::is_ascii_whitespace) {
        return Err(Failure::Argument);
    }
    Ok(end)
}

/// Run prepare and stepping with a cooperative progress checkpoint budget.
fn run<T>(
    memory: &Memory,
    sql: &str,
    params: &[SqlValue],
    operation: impl FnOnce(&mut Statement<'_>) -> Result<T, Failure>,
) -> Result<T, Failure> {
    let db = &memory.db;
    memory.denied.store(false, Ordering::Relaxed);
    let started = Instant::now();
    let mut ticks = 0;
    db.progress_handler(
        PROGRESS_INTERVAL,
        Some(move || {
            ticks += 1;
            ticks >= PROGRESS_CALLS || started.elapsed() >= PROGRESS_TIME
        }),
    )?;
    let result = (|| {
        let end = single_statement(sql, db)?;
        let mut stmt = db.prepare_with_flags(&sql[..end], PREP_FLAGS)?;
        if stmt.parameter_count() != params.len()
            || (1..=params.len()).any(|index| stmt.parameter_name(index).is_some())
        {
            return Err(Failure::Argument);
        }
        for (index, param) in params.iter().enumerate() {
            stmt.raw_bind_parameter(index + 1, param)?;
        }
        operation(&mut stmt)
    })();
    db.progress_handler(0, None::<fn() -> bool>)?;
    if result.is_err() && memory.denied.load(Ordering::Relaxed) {
        Err(Failure::Denied)
    } else {
        result
    }
}

fn sql_input<'js>(
    ctx: &Ctx<'js>,
    sql: Opt<Value<'js>>,
    extra: Opt<Value<'js>>,
) -> JsResult<String> {
    let Some(sql) = sql.0.filter(|_| extra.0.is_none()) else {
        return Err(Failure::Argument.throw(ctx));
    };
    let sql = text(ctx, &sql, SQL_BYTES)?;
    if sql.is_empty() || sql.contains('\0') {
        return Err(Failure::Argument.throw(ctx));
    }
    Ok(sql)
}

/// Borrow only across Rust/SQLite work, never across JS conversion or errors.
fn with_memory<'js, T>(
    ctx: &Ctx<'js>,
    handle: &Class<'js, Database>,
    operation: impl FnOnce(&Memory) -> Result<T, Failure>,
) -> JsResult<T> {
    let result = {
        let handle = handle.try_borrow()?;
        match &handle.owned {
            Some((memory, _)) => operation(memory),
            None => Err(Failure::Closed),
        }
    };
    result.map_err(|failure| failure.throw(ctx))
}

/// Execute one statement without result columns.
fn execute<'js>(
    ctx: Ctx<'js>,
    this: This<Class<'js, Database>>,
    sql: Opt<Value<'js>>,
    args: Opt<Value<'js>>,
    extra: Opt<Value<'js>>,
) -> JsResult<BigInt<'js>> {
    with_memory(&ctx, &this.0, |_| Ok(()))?;
    let sql = sql_input(&ctx, sql, extra)?;
    let params = parameters(&ctx, args)?;
    let changes = with_memory(&ctx, &this.0, |memory| {
        let before = memory.db.total_changes();
        run(memory, &sql, &params, |stmt| {
            if stmt.column_count() != 0 {
                return Err(Failure::Argument);
            }
            stmt.raw_execute()?;
            Ok(())
        })?;
        Ok(memory.db.total_changes() - before)
    })?;
    BigInt::from_u64(ctx, changes)
}

/// Collect bounded, owned data from a read-only statement.
fn collect(stmt: &mut Statement<'_>) -> Result<Vec<Vec<SqlValue>>, Failure> {
    let columns = stmt.column_count();
    if columns == 0 || !stmt.readonly() {
        return Err(Failure::Argument);
    }
    let mut source = stmt.raw_query();
    let mut rows = Vec::new();
    let mut remaining = RESULT_BYTES;
    while let Some(row) = source.next()? {
        if rows.len() == RESULT_ROWS {
            return Err(Failure::Limit);
        }
        let mut values = Vec::with_capacity(columns);
        for index in 0..columns {
            let value = row.get_ref(index)?;
            let size = match value {
                ValueRef::Null | ValueRef::Integer(_) | ValueRef::Real(_) => 8,
                ValueRef::Text(bytes) | ValueRef::Blob(bytes) => bytes.len(),
            };
            remaining = remaining.checked_sub(size).ok_or(Failure::Limit)?;
            values.push(match value {
                ValueRef::Null => SqlValue::Null,
                ValueRef::Integer(integer) => SqlValue::Integer(integer),
                ValueRef::Real(real) => SqlValue::Real(real),
                ValueRef::Text(bytes) => SqlValue::Text(
                    std::str::from_utf8(bytes)
                        .map_err(|_| Failure::Encoding)?
                        .to_owned(),
                ),
                ValueRef::Blob(bytes) => SqlValue::Blob(bytes.to_vec()),
            });
        }
        rows.push(values);
    }
    Ok(rows)
}

/// Query one read-only statement, returning independent positional rows.
fn query<'js>(
    ctx: Ctx<'js>,
    this: This<Class<'js, Database>>,
    sql: Opt<Value<'js>>,
    args: Opt<Value<'js>>,
    extra: Opt<Value<'js>>,
) -> JsResult<Array<'js>> {
    with_memory(&ctx, &this.0, |_| Ok(()))?;
    let sql = sql_input(&ctx, sql, extra)?;
    let params = parameters(&ctx, args)?;
    let rows = with_memory(&ctx, &this.0, |memory| run(memory, &sql, &params, collect))?;
    let result = Array::new(ctx.clone())?;
    for (index, values) in rows.into_iter().enumerate() {
        let row = Array::new(ctx.clone())?;
        for (column, value) in values.into_iter().enumerate() {
            let value = match value {
                SqlValue::Null => Value::new_null(ctx.clone()),
                SqlValue::Integer(integer) => BigInt::from_i64(ctx.clone(), integer)?.into_value(),
                SqlValue::Real(real) => Value::new_number(ctx.clone(), real),
                SqlValue::Text(text) => text.into_js(&ctx)?,
                SqlValue::Blob(blob) => TypedArray::new(ctx.clone(), blob)?.into_value(),
            };
            row.as_object().prop(
                column as u32,
                Property::from(value).writable().enumerable().configurable(),
            )?;
        }
        result.as_object().prop(
            index as u32,
            Property::from(row).writable().enumerable().configurable(),
        )?;
    }
    Ok(result)
}

/// Native module declaration; the loader checks permission and allowlist.
pub(crate) struct SqliteModule;

impl ModuleDef for SqliteModule {
    fn declare<'js>(decl: &Declarations<'js>) -> JsResult<()> {
        decl.declare("openMemory")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> JsResult<()> {
        exports.export("openMemory", Function::new(ctx.clone(), open_memory)?)?;
        Ok(())
    }
}
