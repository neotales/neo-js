//! Public API and authority checks for the synchronous SQLite module.

use neojs_core::{NeoJs, Permissions, RuntimeModule, RuntimeOptions};

const SETUP: &str = r#"
import { openMemory } from 'neo:sqlite';
function eq(actual, expected) {
  if (actual !== expected) throw Error(`expected ${expected}, got ${actual}`);
}
function fails(code, call) {
  try { call(); } catch (error) { eq(error.code, code); return error; }
  throw Error(`expected ${code}`);
}
"#;

fn permitted() -> Permissions {
    Permissions {
        sqlite: true,
        ..Permissions::default()
    }
}

fn script(body: &str) {
    NeoJs::new(permitted())
        .unwrap()
        .run_module("sqlite.js", &format!("{SETUP}\n{body}"))
        .unwrap();
}

#[test]
fn sqlite_needs_permission_and_allowlist_for_static_and_dynamic_imports() {
    let js = NeoJs::new(Permissions::default()).unwrap();
    assert!(js.run_module("denied.js", "import 'neo:sqlite';").is_err());
    let js = NeoJs::new_with_options(
        Permissions::default(),
        RuntimeOptions::new().modules([RuntimeModule::Sqlite]),
    )
    .unwrap();
    assert!(
        js.run_module("allowlisted.js", "import 'neo:sqlite';")
            .is_err()
    );
    let js = NeoJs::new_with_options(permitted(), RuntimeOptions::new().modules([])).unwrap();
    assert!(js.run_module("omitted.js", "import 'neo:sqlite';").is_err());
    assert!(
        js.run_module("dynamic.js", "await import('neo:sqlite');")
            .is_err()
    );
    let js = NeoJs::new_with_options(
        permitted(),
        RuntimeOptions::new().module_names(["neo:sqlite"]).unwrap(),
    )
    .unwrap();
    js.run_module(
        "admitted.js",
        "const {openMemory} = await import('neo:sqlite'); openMemory().close();",
    )
    .unwrap();
}

#[test]
fn binds_and_returns_owned_exact_values_without_invoking_hooks() {
    script(
        r#"
const db = openMemory();
eq(db.execute('CREATE TABLE t(a,b,c,d,e)'), 0n);
eq(db.execute('CREATE INDEX ix ON t(a)'), 0n);
const input = new Uint8Array([9, 0, 255, 9]).subarray(1, 3);
for (const key of ['length', 'byteLength', 'buffer', 'constructor']) {
  Object.defineProperty(input, key, { get() { throw Error('blob hook'); } });
}
Object.defineProperty(Array.prototype, '0', { set() { throw Error('array setter'); } });
eq(db.execute('INSERT INTO t VALUES (?, ?, ?, ?, ?)',
  [9223372036854775807n, 4.5, '\ufeff𝄞\r\n\u0000x', input, null]), 1n);
input[0] = 16;
const [row] = db.query('SELECT * FROM t WHERE a = ?', [9223372036854775807n]);
eq(row[0], 9223372036854775807n);
eq(row[1], 4.5);
eq(row[2], '\ufeff𝄞\r\n\u0000x');
eq(row[3] instanceof Uint8Array, true);
eq(row[3].join(','), '0,255');
eq(row[4], null);
eq(db.query('SELECT typeof(?), typeof(?)', [1, 1n])[0].join(','), 'real,integer');
eq(db.query('SELECT ?, ?', [-9223372036854775808n, 9223372036854775807n])[0][0],
  -9223372036854775808n);
row[3][0] = 17;
eq(db.query('SELECT d FROM t')[0][0][0], 0);
eq(db.query('SELECT ?', [new Uint8Array(0)])[0][0].length, 0);
const resizing = new ArrayBuffer(2, { maxByteLength: 8 });
const view = new Uint8Array(resizing, 1);
fails('SQLITE_ARGUMENT', () => db.query('SELECT ?', [view]));
db.close();
"#,
    );
}

#[test]
fn invalid_input_result_and_work_limits_leave_connection_usable() {
    script(
        r#"
const db = openMemory();
fails('SQLITE_ARGUMENT', () => openMemory('rejected'));
fails('SQLITE_ARGUMENT', () => db.query());
fails('SQLITE_ARGUMENT', () => db.execute('CREATE TABLE extra(x)', [], null));
for (const sql of [42, {}, 'SELECT 1\0', '', '-- only comment']) {
  fails('SQLITE_ARGUMENT', () => db.query(sql));
}
fails('SQLITE_ENCODING', () => db.query('SELECT \ud800'));
fails('SQLITE_ENCODING', () => db.query('SELECT ?', ['\udfff']));
fails('SQLITE_ENCODING', () => db.query("SELECT CAST(X'FF' AS TEXT)"));
fails('SQLITE_LIMIT', () => db.query(' '.repeat(65537)));
fails('SQLITE_LIMIT', () => db.query('é'.repeat(32769)));
fails('SQLITE_LIMIT', () => db.query('SELECT ?', [new Uint8Array(1048577)]));
fails('SQLITE_LIMIT', () => db.query('SELECT 1', new Array(65)));
const accessor = [0];
Object.defineProperty(accessor, '0', {get() { throw Error('getter ran'); }});
const proxy = new Proxy([1], {get() { throw Error('proxy ran'); }});
const buffer = new ArrayBuffer(4);
const detached = new Uint8Array(buffer);
buffer.transfer();
const resized = new ArrayBuffer(3, { maxByteLength: 8 });
const oob = new Uint8Array(resized, 1, 2);
resized.resize(1);
for (const args of [accessor, proxy, [undefined], new Array(1), [NaN], [Infinity],
    [true], [new Uint8ClampedArray(1)], [new Uint8Array(new SharedArrayBuffer(1))],
    [detached], [oob], [9223372036854775808n], [-9223372036854775809n], [1n << 128n]]) {
  fails('SQLITE_ARGUMENT', () => db.query('SELECT ?', args));
}
fails('SQLITE_ARGUMENT', () => db.query('SELECT ?2', [1n, 2n]));
fails('SQLITE_ARGUMENT', () => db.query('SELECT :named', [1n]));
fails('SQLITE_ARGUMENT', () => db.query('SELECT ?'));
fails('SQLITE_ARGUMENT', () => db.execute('SELECT 1'));
fails('SQLITE_ARGUMENT', () => db.execute('CREATE TABLE never_created(x); SELECT 1'));
eq(db.query("SELECT count(*) FROM sqlite_schema WHERE name='never_created'")[0][0], 0n);
db.execute('CREATE TABLE t(x UNIQUE)');
fails('SQLITE_ARGUMENT', () => db.query('INSERT INTO t VALUES (123) RETURNING x'));
eq(db.query('SELECT count(*) FROM t')[0][0], 0n);
db.execute('INSERT INTO t VALUES (?)', ['NOT_FOR_LOGS']);
const duplicate = fails('SQLITE_CONSTRAINT',
    () => db.execute("INSERT INTO t VALUES ('NOT_FOR_LOGS')"));
eq(String(duplicate).includes('NOT_FOR_LOGS'), false);
const syntax = fails('SQLITE_SQL', () => db.query('this is SECRET_SQL'));
eq(String(syntax).includes('SECRET_SQL'), false);
const recursive = `WITH RECURSIVE r(x) AS
  (VALUES(1) UNION ALL SELECT x + 1 FROM r WHERE x < 1000000)`;
fails('SQLITE_LIMIT', () => db.query(`${recursive} SELECT x FROM r`));
fails('SQLITE_LIMIT', () => db.query(`WITH r(x) AS (VALUES(1),(2),(3),(4),(5))
  SELECT zeroblob(900000) FROM r ORDER BY x DESC`));
fails('SQLITE_WORK_LIMIT', () => db.query(`${recursive} SELECT sum(x) FROM r`));
eq(db.query('SELECT 7')[0][0], 7n);
"#,
    );
}

#[test]
fn errors_allow_safe_reentry_from_engine_stack_hooks() {
    script(
        r#"
const buffer = new ArrayBuffer(2);
const detached = new Uint8Array(buffer);
buffer.transfer();
const resizing = new ArrayBuffer(4, { maxByteLength: 8 });
const oob = new Uint8Array(resizing, 2, 2);
resizing.resize(0);
let hooks = 0;
let limitReads = 0;
let prototypeSetters = 0;
let blobHooks = 0;
let target;
let propagated;
const typed = Object.getPrototypeOf(Uint8Array.prototype);
for (const [object, name] of [
    [typed, 'buffer'], [ArrayBuffer.prototype, 'detached'], [ArrayBuffer.prototype, 'resizable'],
]) {
  Object.defineProperty(object, name, { get() { blobHooks++; return false; } });
}
Error.prepareStackTrace = error => {
  hooks++;
  propagated = error;
  // QuickJS invokes this only when propagating the thrown error. Closing
  // the handle here must succeed: no database borrow is still active.
  target.close();
  return 'not attached to a SQLite error';
};
Error.stackTraceLimit = { valueOf() { limitReads++; return 5; } };
for (const name of ['message', 'code', 'stack']) {
  Object.defineProperty(Error.prototype, name, { set() { prototypeSetters++; } });
}
for (const [code, run] of [
    ['SQLITE_ARGUMENT', db => db.query('SELECT ?', [detached])],
    ['SQLITE_ARGUMENT', db => db.query('SELECT ?', [oob])],
    ['SQLITE_SQL', db => db.query('SECRET SQL')],
    ['SQLITE_DENIED', db => db.execute('PRAGMA temp_store=FILE')],
]) {
  target = openMemory();
  const before = hooks;
  const error = fails(code, () => run(target));
  eq(error instanceof Error, true);
  eq(error.name, 'SqliteError');
  eq(error.stack, undefined);
  eq(propagated, error);
  eq(hooks, before + 1); // No extra TypeError or error-construction hook.
  fails('SQLITE_CLOSED', () => target.query('SELECT 5'));
  target[Symbol.dispose]();
}
using db = openMemory();
eq(db.query('SELECT 5')[0][0], 5n);
eq(hooks, 8);
eq(limitReads, 8);
eq(prototypeSetters, 0);
eq(blobHooks, 0);
"#,
    );
}

#[test]
fn authorizer_denies_string_search_and_replacement_without_running_them() {
    script(
        r#"
using db = openMemory();
const text = 'a'.repeat(500000);
const nearMatch = 'a'.repeat(400000) + 'b';
for (const sql of [
    "SELECT replace(?, ?, 'x')",
    'SELECT instr(?, ?)',
    'SELECT trim(?, ?)', 'SELECT ltrim(?, ?)', 'SELECT rtrim(?, ?)',
    'SELECT ? LIKE ?', 'SELECT ? GLOB ?',
]) {
  fails('SQLITE_DENIED', () => db.query(sql, [text, nearMatch]));
}
eq(db.query('SELECT length(?)', [text])[0][0], 500000n);
"#,
    );
}

#[test]
fn foreign_key_actions_work_with_a_depth_limit_and_savepoints_are_denied() {
    script(
        r#"
using db = openMemory();
db.execute('CREATE TABLE parent(id INTEGER PRIMARY KEY)');
db.execute(`CREATE TABLE child(parent INTEGER REFERENCES parent(id)
             ON UPDATE CASCADE ON DELETE CASCADE)`);
fails('SQLITE_CONSTRAINT', () => db.execute('INSERT INTO child VALUES (7)'));
db.execute('INSERT INTO parent VALUES (1)');
db.execute('INSERT INTO child VALUES (1)');
eq(db.execute('UPDATE parent SET id=2'), 2n);
eq(db.query('SELECT parent FROM child')[0][0], 2n);
eq(db.execute('DELETE FROM parent'), 2n);
eq(db.query('SELECT count(*) FROM child')[0][0], 0n);
db.execute('CREATE TABLE chain(id INTEGER PRIMARY KEY, parent REFERENCES chain(id) ON DELETE CASCADE)');
db.execute('INSERT INTO chain VALUES (1,NULL)');
for (let i = 2; i <= 12; i++) {
  db.execute('INSERT INTO chain VALUES (?, ?)', [BigInt(i), BigInt(i - 1)]);
}
fails('SQLITE_SQL', () => db.execute('DELETE FROM chain WHERE id=1'));
eq(db.query('SELECT count(*) FROM chain')[0][0], 12n);
db.execute('BEGIN');
for (let i = 0; i < 32; i++) fails('SQLITE_DENIED', () => db.execute('SAVEPOINT s'));
fails('SQLITE_DENIED', () => db.execute('RELEASE s'));
fails('SQLITE_DENIED', () => db.execute('ROLLBACK TO s'));
db.execute('INSERT INTO parent VALUES (3)');
db.execute('ROLLBACK');
eq(db.query('SELECT count(*) FROM parent')[0][0], 0n);
"#,
    );
}

#[test]
fn rejects_path_and_schema_escape_without_creating_files() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.sqlite");
    let path_js = serde_json::to_string(missing.to_str().unwrap()).unwrap();
    let existing = dir.path().join("existing.sqlite");
    std::fs::write(&existing, b"fixture, not a database").unwrap();
    let existing_js = serde_json::to_string(existing.to_str().unwrap()).unwrap();
    script(&format!(
        r#"
const db = openMemory();
const path = {path_js};
const existing = {existing_js};
db.execute('CREATE TABLE data(value)');
for (const sql of [
    '/*not a prefix check*/ ATTACH DATABASE ? AS other',
    'VACUUM main INTO ?',
]) {{
    for (const name of [path, existing, `file:${{path}}?mode=rwc`]) {{
        const error = fails('SQLITE_DENIED', () => db.execute(sql, [name]));
        eq(String(error).includes(name), false);
    }}
}}
for (const sql of [
    'PRAGMA temp_store=FILE', 'PRAGMA writable_schema=ON',
    'PRAGMA journal_mode=WAL', 'CREATE TEMP TABLE storage(value)',
    'CREATE VIRTUAL TABLE search USING fts5(value)',
    'CREATE TRIGGER evil AFTER INSERT ON data BEGIN SELECT load_extension(NEW.value); END',
    "CREATE VIEW evil AS SELECT load_extension('path')",
]) {{
    fails('SQLITE_DENIED', () => db.execute(sql));
}}
fails('SQLITE_SQL', () => db.query('SELECT * FROM pragma_database_list'));
fails('SQLITE_DENIED', () => db.query('SELECT LOAD_EXTENSION(?)', [path]));
fails('SQLITE_SQL', () => db.execute('DELETE FROM sqlite_schema'));
eq(db.query("SELECT count(*) FROM sqlite_schema WHERE type IN ('trigger', 'view')")[0][0], 0n);
eq(db.execute('INSERT INTO data VALUES (?)', [path]), 1n);
eq(db.query('SELECT count(*) FROM data')[0][0], 1n);
"#
    ));
    assert!(!missing.exists());
    assert_eq!(std::fs::read(existing).unwrap(), b"fixture, not a database");
    assert_eq!(dir.path().read_dir().unwrap().count(), 1);
}

#[test]
fn closes_on_using_exception_releases_slots_and_drops_with_runtime() {
    script(
        r#"
let saved;
const marker = {};
try {
  using db = openMemory();
  saved = db;
  db.execute('CREATE TABLE t(x)');
  throw marker;
} catch (error) { eq(error, marker); }
fails('SQLITE_CLOSED', () => saved.query('SELECT 1'));
fails('SQLITE_CLOSED', () => saved.execute('not validated after close'));
saved.close();
saved[Symbol.dispose]();
const live = Array.from({length: 8}, () => openMemory());
fails('SQLITE_LIMIT', () => openMemory());
live[0].close();
using another = openMemory();
// Keep active handles reachable until runtime teardown, including a cycle.
globalThis.live = live;
live[1].cycle = live[1];
"#,
    );
    script("openMemory().close();");
}
