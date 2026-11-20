//! Public API, allowlist, buffer-safety, and ownership checks for `neo:io`.

use neojs_core::{ExtensionPermission, ModuleAllowlist, NeoJs, Permissions, RuntimeOptions};
use neojs_io::{MAX_BYTES, MAX_OBJECTS, MAX_RUNTIME_BYTES, MODULE_NAME, io_extension, with_io};

const SETUP: &str = r#"
import { MAX_BYTES, MAX_OBJECTS, MAX_RUNTIME_BYTES, byteReader, byteWriter } from 'neo:io';
const RealError = Error;
const RealUint8Array = Uint8Array;
function eq(actual, expected) {
  if (actual !== expected) throw new RealError(`expected ${expected}, got ${actual}`);
}
function failsTypeError(call) {
  try {
    call();
  } catch (error) {
    if (error.name !== 'TypeError' || 'code' in error) {
      throw new RealError('expected a native TypeError');
    }
    return;
  }
  throw new RealError('expected a native TypeError');
}
function fails(code, call) {
  try {
    call();
  } catch (error) {
    if (!(error instanceof RealError)) throw new RealError('not an Error');
    eq(error.name, 'IoError');
    eq(error.code, code);
    eq(error.stack, undefined);
    return;
  }
  throw new RealError(`expected ${code}`);
}
"#;

fn script(body: &str) {
    NeoJs::new_with_options(Permissions::default(), options())
        .unwrap()
        .run_module("io.js", &format!("{SETUP}\n{body}"))
        .unwrap();
}

fn options() -> RuntimeOptions {
    with_io(RuntimeOptions::new()).unwrap()
}

fn import_error(js: &NeoJs, entry: &str, module: &str) -> String {
    js.run_module(entry, &format!("import '{module}';"))
        .unwrap_err()
}

#[test]
fn module_is_exact_allowlisted_and_needs_no_permission() {
    // Registration alone does not bypass an explicit allowlist.
    let denied = with_io(RuntimeOptions::new().modules([])).unwrap();
    let js = NeoJs::new_with_options(Permissions::default(), denied).unwrap();
    let error = import_error(&js, "denied.js", MODULE_NAME);
    assert!(error.contains("allowlist"), "{error}");
    assert!(
        js.run_module("dynamic-denied.js", "await import('neo:io');")
            .is_err()
    );

    // The exact registered name admits it under default-deny permissions.
    let allowed = options().module_names([MODULE_NAME]).unwrap();
    assert!(allowed.is_module_name_allowed(MODULE_NAME));
    let js = NeoJs::new_with_options(Permissions::default(), allowed).unwrap();
    js.run_module(
        "allowed.js",
        r#"import * as staticModule from 'neo:io';
           const dynamicModule = await import('neo:io');
           if (staticModule !== dynamicModule) throw Error('module identity');
           if (Object.keys(dynamicModule).sort().join(',') !== 'MAX_BYTES,MAX_OBJECTS,MAX_RUNTIME_BYTES,byteReader,byteWriter')
             throw Error('export surface');
           if (typeof Buffer !== 'undefined') throw Error('Buffer exists');
           const reader = dynamicModule.byteReader(new Uint8Array([1]));
           if ('on' in reader || 'pipe' in reader) throw Error('stream surface');
           reader.close();"#,
    )
    .unwrap();

    // A host may add a name to the descriptor, but this loader must not
    // declare a module under that alias.
    let alias = "neo:io-alias";
    let augmented = RuntimeOptions::new()
        .with_extension(io_extension().with_module(alias, ExtensionPermission::None))
        .unwrap()
        .module_names([alias])
        .unwrap();
    let js = NeoJs::new_with_options(Permissions::default(), augmented).unwrap();
    let error = import_error(&js, "alias.js", alias);
    assert!(error.contains("unknown byte I/O module name"), "{error}");

    // Similar names are not this module, in the allowlist or the resolver.
    let invalid = [
        "neo:io/bytes",
        "neo:IO",
        "neo:io.js",
        "neo:io/",
        "neo:io/../io",
        "neo:iobytes",
    ];
    let error = ModuleAllowlist::from_names_with(invalid, options().extensions()).unwrap_err();
    assert_eq!(error.names(), invalid);
    let js = NeoJs::new_with_options(Permissions::default(), options()).unwrap();
    for (index, name) in invalid.into_iter().enumerate() {
        let error = import_error(&js, &format!("bad-{index}.js"), name);
        assert!(error.contains(name), "{error}");
    }
}

#[test]
fn reader_returns_counts_and_zero_only_at_eof() {
    script(
        r#"
const input = new RealUint8Array([9, 1, 2, 3, 9]).subarray(1, 4);
const reader = byteReader(input);
eq(reader.closed, false);
eq(reader.remaining, 3);
input[0] = 99;
const three = new RealUint8Array(2);
eq(reader.read(three), 2);
eq(three.join(','), '1,2');
eq(reader.remaining, 1);
const one = new RealUint8Array(8);
eq(reader.read(one), 1);
eq(one[0], 3);
eq(reader.remaining, 0);
eq(reader.read(one), 0);
eq(reader.read(one), 0);
const empty = byteReader(new RealUint8Array(0));
eq(empty.read(one), 0);
fails('IO_ARGUMENT', () => empty.read(new RealUint8Array(0)));
fails('IO_ARGUMENT', () => empty.read(new RealUint8Array(1).subarray(0, 0)));
fails('IO_ARGUMENT', () => byteReader('not bytes'));
fails('IO_ARGUMENT', () => byteReader(new Uint16Array(2)));
fails('IO_ARGUMENT', () => byteReader(new Proxy(new RealUint8Array(1), {})));
"#,
    );
}

#[test]
fn writer_owns_its_bytes_and_returns_copies() {
    script(
        r#"
const writer = byteWriter();
eq(writer.closed, false);
eq(writer.length, 0);
eq(writer.write(new RealUint8Array(0)), 0);
eq(writer.write(RealUint8Array.from([1, 2])), 2);
eq(writer.write(RealUint8Array.from([3])), 1);
eq(writer.length, 3);
const first = writer.toUint8Array();
eq(first instanceof RealUint8Array, true);
eq(first.join(','), '1,2,3');
first[0] = 99;
eq(writer.toUint8Array()[0], 1);
const second = writer.toUint8Array();
if (first === second) throw new RealError('same output array');
eq(writer.length, 3);
"#,
    );
}

#[test]
fn close_is_idempotent_and_invalidates_both_interfaces() {
    script(
        r#"
const reader = byteReader(new RealUint8Array([1]));
reader.close();
reader.close();
eq(reader.closed, true);
eq(reader.remaining, 0);
fails('IO_CLOSED', () => reader.read(new RealUint8Array(1)));

const writer = byteWriter();
writer.write(new RealUint8Array([1]));
writer.close();
writer.close();
eq(writer.closed, true);
eq(writer.length, 0);
fails('IO_CLOSED', () => writer.write(new RealUint8Array(1)));
fails('IO_CLOSED', () => writer.toUint8Array());
"#,
    );
}

#[test]
fn limits_are_exact_and_write_is_atomic() {
    script(
        r#"
fails('IO_LIMIT', () => byteReader(new RealUint8Array(MAX_BYTES + 1)));
const atLimit = new RealUint8Array(MAX_BYTES);
const writer = byteWriter();
eq(writer.write(atLimit), MAX_BYTES);
eq(writer.length, MAX_BYTES);
fails('IO_LIMIT', () => writer.write(new RealUint8Array(1)));
eq(writer.length, MAX_BYTES);
eq(writer.toUint8Array().length, MAX_BYTES);
"#,
    );
}

#[test]
fn runtime_aggregate_bytes_are_bounded_and_released() {
    script(
        r#"
eq(MAX_BYTES, 8 * 1024 * 1024);
eq(MAX_RUNTIME_BYTES, 16 * 1024 * 1024);
eq(MAX_OBJECTS, 32);
const first = byteReader(new RealUint8Array(MAX_BYTES));
const second = byteReader(new RealUint8Array(MAX_BYTES));
fails('IO_LIMIT', () => byteReader(new RealUint8Array(1)));
first.close();
const one = byteReader(new RealUint8Array(1));
eq(one.remaining, 1);
second.close();
one.close();
const fullAgainA = byteReader(new RealUint8Array(MAX_BYTES));
const fullAgainB = byteReader(new RealUint8Array(MAX_BYTES));
eq(fullAgainA.remaining, MAX_BYTES);
eq(fullAgainB.remaining, MAX_BYTES);
fullAgainA.close();
fullAgainB.close();
"#,
    );
}

#[test]
fn aggregate_write_limit_is_atomic() {
    script(
        r#"
const eight = byteReader(new RealUint8Array(MAX_BYTES));
const five = byteReader(new RealUint8Array(5 * 1024 * 1024));
const writer = byteWriter();
eq(writer.write(new RealUint8Array(3 * 1024 * 1024)), 3 * 1024 * 1024);
fails('IO_LIMIT', () => writer.write(new RealUint8Array(1)));
eq(writer.length, 3 * 1024 * 1024);
eight.close();
five.close();
writer.close();
"#,
    );
}

#[test]
fn to_uint8_array_charges_its_temporary_copy() {
    script(
        r#"
const reader = byteReader(new RealUint8Array(MAX_BYTES));
const writer = byteWriter();
writer.write(new RealUint8Array(MAX_BYTES));
fails('IO_LIMIT', () => writer.toUint8Array());
eq(writer.length, MAX_BYTES);
reader.close();
const bytes = writer.toUint8Array();
eq(bytes.length, MAX_BYTES);
writer.close();
"#,
    );
}

#[test]
fn live_objects_are_bounded_and_drop_releases_a_slot() {
    script(
        r#"
const readers = [];
for (let i = 0; i < MAX_OBJECTS; i++) {
  readers.push(byteReader(new RealUint8Array(0)));
}
fails('IO_LIMIT', () => byteReader(new RealUint8Array(0)));
// close releases bytes, not the live-object slot.
readers[0].close();
fails('IO_LIMIT', () => byteReader(new RealUint8Array(0)));
// Dropping the JS reference releases the native object immediately when it
// is not in a cycle.
readers.pop();
const replacement = byteReader(new RealUint8Array(0));
eq(replacement.closed, false);
"#,
    );
}

#[test]
fn closed_objects_report_closed_before_validating_input() {
    script(
        r#"
function detached(n) {
  const bytes = new RealUint8Array(n);
  bytes.buffer.transfer();
  return bytes;
}
const reader = byteReader(new RealUint8Array([1]));
reader.close();
fails('IO_CLOSED', () => reader.read(detached(1)));
fails('IO_CLOSED', () => reader.read(new RealUint8Array(0)));
fails('IO_CLOSED', () => reader.read(new RealUint8Array(new SharedArrayBuffer(1))));
fails('IO_CLOSED', () => reader.read('not bytes'));

const writer = byteWriter();
writer.close();
fails('IO_CLOSED', () => writer.write(detached(1)));
fails('IO_CLOSED', () => writer.write(new RealUint8Array(new SharedArrayBuffer(1))));
fails('IO_CLOSED', () => writer.write('not bytes'));
"#,
    );
}

#[test]
fn borrowed_methods_and_accessors_throw_native_type_errors() {
    script(
        r#"
const reader = byteReader(new RealUint8Array([1]));
const read = reader.read;
failsTypeError(() => read(new RealUint8Array(1)));
failsTypeError(() => read.call({}, new RealUint8Array(1)));
const remaining = Object.getOwnPropertyDescriptor(
  Object.getPrototypeOf(reader), 'remaining').get;
failsTypeError(() => remaining.call({}));
const closed = Object.getOwnPropertyDescriptor(
  Object.getPrototypeOf(reader), 'closed').get;
failsTypeError(() => closed.call({}));
const write = byteWriter().write;
failsTypeError(() => write(new RealUint8Array(1)));
failsTypeError(() => write.call({}, new RealUint8Array(1)));
"#,
    );
}

#[test]
fn detached_shared_and_resizable_views_are_rejected() {
    script(
        r#"
function detached(n) {
  const bytes = new RealUint8Array(n);
  bytes.buffer.transfer();
  return bytes;
}
fails('IO_DETACHED', () => byteReader(detached(4)));
fails('IO_DETACHED', () => byteWriter().write(detached(1)));
const reader = byteReader(new RealUint8Array([1, 2]));
fails('IO_DETACHED', () => reader.read(detached(1)));
fails('IO_SHARED', () => reader.read(new RealUint8Array(new SharedArrayBuffer(1))));
fails('IO_RESIZABLE', () => reader.read(new RealUint8Array(new ArrayBuffer(1, {maxByteLength: 2}))));

fails('IO_SHARED', () => byteReader(new RealUint8Array(new SharedArrayBuffer(4))));
fails('IO_SHARED', () => byteWriter().write(new RealUint8Array(new SharedArrayBuffer(1))));

fails('IO_RESIZABLE', () => byteReader(new RealUint8Array(new ArrayBuffer(4, {maxByteLength: 8}))));
fails('IO_RESIZABLE', () => byteWriter().write(new RealUint8Array(new ArrayBuffer(1, {maxByteLength: 4}))));
"#,
    );
}

#[test]
fn prototype_mutation_does_not_reach_native_validation() {
    script(
        r#"
const input = RealUint8Array.from([1, 2, 3]);
for (const key of ['buffer', 'byteLength', 'length']) {
  Object.defineProperty(input, key, { get() { throw new RealError('own hook'); } });
}
const reader = byteReader(input);
eq(reader.remaining, 3);
const target = new RealUint8Array(2);
for (const key of ['buffer', 'byteLength', 'length']) {
  Object.defineProperty(target, key, { get() { throw new RealError('target hook'); } });
}
eq(reader.read(target), 2);
eq(target[0], 1);
eq(target[1], 2);

const writer = byteWriter();
const source = RealUint8Array.from([7, 8]);
for (const key of ['buffer', 'byteLength', 'length']) {
  Object.defineProperty(source, key, { get() { throw new RealError('source hook'); } });
}
eq(writer.write(source), 2);
eq(writer.toUint8Array().join(','), '7,8');

// An own numeric setter on Array.prototype must not see a copy.
Object.defineProperty(Array.prototype, '0', { configurable: true, set() { throw new RealError('array setter'); } });
eq(reader.read(new RealUint8Array(1)), 1);
delete Array.prototype[0];

// Replacing the global constructor after import affects neither validation
// nor the output brand.
globalThis.Uint8Array = function FakeUint8Array() {};
const after = writer.toUint8Array();
eq(after instanceof RealUint8Array, true);
eq(byteReader(input).remaining, 3);
globalThis.Uint8Array = RealUint8Array;
"#,
    );
}

#[test]
fn extension_registration_is_duplicate_safe() {
    let duplicate = with_io(options()).unwrap_err();
    assert_eq!(duplicate.code(), "EXTENSION_DUPLICATE_EXTENSION");
    let all = ModuleAllowlist::all_with_extensions(options().extensions());
    assert!(all.contains_extension_name(MODULE_NAME));
    assert_eq!(MAX_BYTES, 8 * 1024 * 1024);
    assert_eq!(MAX_RUNTIME_BYTES, 16 * 1024 * 1024);
    assert_eq!(MAX_OBJECTS, 32);
}
