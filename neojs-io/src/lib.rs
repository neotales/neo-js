//! Optional synchronous in-memory byte reader and writer for `neojs-core`.
//!
//! This crate supplies the exact `neo:io` module through
//! [`neojs_core::extension`]. `neojs-core` itself does not compile this module
//! and contains no byte-reader or byte-writer implementation. Registering this
//! extension restores the import; the CLI and other embedders opt in
//! explicitly.
//!
//! The module is deliberately small and synchronous. It exposes two
//! interfaces, a `ByteReader` over a bounded snapshot copied from a
//! `Uint8Array`, and a `ByteWriter` that owns a bounded growable byte buffer.
//! Both have explicit `read`, `write`, and `close` behaviour. The writer is
//! read back with `toUint8Array`, which returns a fresh copy.
//!
//! This is **not** a Node.js stream, an `EventEmitter`, or a web stream. There
//! are no events, listeners, promises, `pipe`, `on`, backpressure signalling,
//! encodings, or `Buffer` values. It does not touch files, the network, the
//! environment, processes, the registry, or any other host resource, and it
//! needs no permission flag. Its native byte storage is bounded as described
//! below; a `Uint8Array` returned to JavaScript is outside that budget.
//!
//! # Registration
//!
//! Register [`io_extension`] on the runtime options and name the exact module
//! in any explicit allowlist. The extension uses
//! [`neojs_core::ExtensionPermission::None`]; the runtime allowlist still
//! decides whether the exact name exists.
//!
//! ```rust
//! use neojs_core::{NeoJs, Permissions, RuntimeOptions};
//! use neojs_io::{MODULE_NAME, with_io};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let options = with_io(RuntimeOptions::new())?.module_names([MODULE_NAME])?;
//! let runtime = NeoJs::new_with_options(Permissions::default(), options)?;
//! runtime.run_module(
//!     "main.js",
//!     "import {byteReader, byteWriter} from 'neo:io';
//!      const reader = byteReader(Uint8Array.from([1, 2, 3]));
//!      const target = new Uint8Array(2);
//!      if (reader.read(target) !== 2 || target[1] !== 2) throw Error('read');
//!      const writer = byteWriter();
//!      writer.write(target);
//!      if (writer.toUint8Array()[1] !== 2) throw Error('write');
//!      reader.close();
//!      writer.close();",
//! )?;
//! # Ok(())
//! # }
//! ```
//!
//! # Sizes and errors
//!
//! [`MAX_BYTES`] is the limit for a reader's input, a writer's total content,
//! and each `write` call's input. [`MAX_RUNTIME_BYTES`] additionally bounds
//! native bytes across every live reader and writer in one runtime, and
//! [`MAX_OBJECTS`] bounds the number of live objects. Reader input and writer
//! content count against the aggregate; a JavaScript `Uint8Array` returned by
//! `toUint8Array` is JavaScript-owned and outside it after return. A closed
//! object releases its bytes but keeps its object slot until the object is
//! collected or the runtime is dropped.
//!
//! A limit violation or allocation failure is reported before any state
//! changes, so a failed `write` never leaves a partial append. The module
//! throws a native `Error` with `name = "IoError"` and a stable `code`:
//!
//! * `IO_ARGUMENT` — the argument is not a `Uint8Array` or a `read` target is
//!   empty.
//! * `IO_LIMIT` — the data would exceed [`MAX_BYTES`], the runtime aggregate
//!   or object limit, or memory reservation failed. `toUint8Array` can also
//!   report `IO_LIMIT` when its temporary copy would exceed the aggregate.
//! * `IO_CLOSED` — the reader or writer has been closed.
//! * `IO_DETACHED` — the `Uint8Array` backing store is detached.
//! * `IO_SHARED` — the view is over a `SharedArrayBuffer`.
//! * `IO_RESIZABLE` — the backing `ArrayBuffer` is resizable.
//!
//! A `read` returns a positive byte count when it copied bytes, and exactly
//! `0` at end of file. End of file is not an exception. An empty target is an
//! argument error rather than an ambiguous zero, so a caller can use `0` as
//! the only end-of-file result. A closed reader reports `IO_CLOSED` before it
//! validates the target, and a closed writer does the same before validating
//! its input.
//!
//! # Buffer and ownership rules
//!
//! `byteReader` copies the current view of its `Uint8Array` into reader-owned
//! storage. Mutating, detaching, or resizing the caller's buffer afterwards
//! does not change the reader. `byteWriter` owns its storage; `toUint8Array`
//! returns a fresh `Uint8Array` that the caller owns, and changing that result
//! does not change the writer. It is outside the native byte budget after it
//! is returned. `close` releases the stored bytes; it is idempotent and never
//! detaches a caller's buffer. A closed object keeps its object slot until it
//! is collected or the runtime is dropped.
//!
//! Inputs are checked through the engine's internal typed-array brand and the
//! captured intrinsic `%TypedArray%` accessors, not through mutable globals or
//! prototype properties. A detached, shared, resizable, forged, or proxied
//! value is rejected with the matching code. No JavaScript runs between the
//! native length check and the byte copy. The runtime budget is charged before
//! a native allocation and refunded if reservation, allocation, conversion, or
//! an unwind fails. No mutable borrow of a reader or writer is held across a
//! JavaScript call.
//!
//! Calling a method or getter with a receiver that is not an object created by
//! this module throws the engine's native `TypeError`, not an `IoError`.
//!
//! The extension descriptor holds no QuickJS value and is shared only as
//! static metadata. Per-runtime state and the native reader/writer objects
//! stay on the thread that created them; no JavaScript value crosses threads.

use std::{any::Any, cell::RefCell, rc::Rc, slice};

use neojs_core::rquickjs::{
    Class, Ctx, Error, Function, JsLifetime, Module, Object, Persistent, Result as JsResult,
    TypedArray, Value,
    class::{JsClass, Trace, Tracer, Writable},
    function::{Constructor, This},
    module::{Declarations, Declared, Exports, ModuleDef},
    object::{Accessor, Property},
    qjs,
};
use neojs_core::{
    ExtensionDescriptor, ExtensionPermission, ExtensionRegistrationError, RuntimeOptions,
};

/// The exact `neo:` module name supplied by this crate.
pub const MODULE_NAME: &str = "neo:io";

/// The maximum number of bytes held by one reader or writer, and the maximum
/// accepted by one `write` call.
pub const MAX_BYTES: usize = 8 * 1024 * 1024;

/// The maximum native bytes one runtime may hold across every live reader and
/// writer. The input copy of a reader and the content of a writer both count;
/// a retained JavaScript `Uint8Array` returned by `toUint8Array` does not.
pub const MAX_RUNTIME_BYTES: usize = 16 * 1024 * 1024;

/// The maximum number of live reader and writer objects in one runtime.
///
/// A closed object releases its bytes but keeps its object slot until the
/// object is collected or the runtime is dropped.
pub const MAX_OBJECTS: usize = 32;

/// Build the static extension descriptor that supplies `neo:io`.
///
/// The descriptor needs no permission flag, installs per-runtime validation
/// state during construction, and releases its one persistent JavaScript
/// handle during teardown. Register it with [`RuntimeOptions::with_extension`],
/// [`RuntimeOptions::with_extensions`], or [`with_io`].
pub fn io_extension() -> ExtensionDescriptor {
    ExtensionDescriptor::new("neojs-io", load)
        .with_module(MODULE_NAME, ExtensionPermission::None)
        .with_init(init)
        .with_teardown(teardown)
}

/// Register `neo:io` on `options` and return the updated options.
///
/// This is [`RuntimeOptions::with_extension`] with [`io_extension`]. It grants
/// no permission, because this module needs none, and does not change an
/// explicit allowlist; call [`RuntimeOptions::module_names`] with
/// [`MODULE_NAME`] when the options carry one.
pub fn with_io(options: RuntimeOptions) -> Result<RuntimeOptions, ExtensionRegistrationError> {
    options.with_extension(io_extension())
}

/// Failure codes exposed as `IoError.code`.
#[derive(Clone, Copy, Debug)]
enum Failure {
    Argument,
    Limit,
    ReaderClosed,
    WriterClosed,
    Detached,
    Shared,
    Resizable,
}

impl Failure {
    fn code(self) -> &'static str {
        match self {
            Self::Argument => "IO_ARGUMENT",
            Self::Limit => "IO_LIMIT",
            Self::ReaderClosed | Self::WriterClosed => "IO_CLOSED",
            Self::Detached => "IO_DETACHED",
            Self::Shared => "IO_SHARED",
            Self::Resizable => "IO_RESIZABLE",
        }
    }

    fn message(self) -> &'static str {
        match self {
            Self::Argument => "argument must be an attached, fixed-length Uint8Array",
            Self::Limit => "byte data exceeds the neo:io size limit",
            Self::ReaderClosed => "the byte reader is closed",
            Self::WriterClosed => "the byte writer is closed",
            Self::Detached => "the Uint8Array buffer is detached",
            Self::Shared => "a Uint8Array over SharedArrayBuffer is not supported",
            Self::Resizable => "a Uint8Array over a resizable ArrayBuffer is not supported",
        }
    }

    /// Throw a native `Error` object with the stable code and a static message.
    ///
    /// The object is created from the captured built-in `Error` class ID, so
    /// `instanceof Error` is true and the realm's `Error` constructor is never
    /// consulted.
    fn throw(self, ctx: &Ctx<'_>) -> Error {
        let result = (|| {
            let state = io_state(ctx)?;
            let class = state.borrow().error_class.ok_or(Error::Unknown)?;
            drop(state);
            // SAFETY: the class ID came from `new Error()` in this context
            // during initialization. Unlike `JS_NewError`, constructing the
            // object directly does not build a backtrace or run stack hooks.
            let error = unsafe { qjs::JS_NewObjectClass(ctx.as_raw().as_ptr(), class) };
            if unsafe { qjs::JS_IsException(error) } {
                return Err(Error::Exception);
            }
            let object = Object::from_value(unsafe { Value::from_raw(ctx.clone(), error) })?;
            object.prop("name", "IoError")?;
            object.prop(
                "message",
                Property::from(self.message()).configurable().writable(),
            )?;
            object.prop("code", Property::from(self.code()).enumerable())?;
            // Suppress the per-error stack; QuickJS can still build a context
            // backtrace while propagating the exception.
            object.prop("stack", Value::new_undefined(ctx.clone()))?;
            Ok(ctx.throw(object.into_value()))
        })();
        result.unwrap_or_else(|error| error)
    }
}

/// Native byte and object accounting for one runtime.
struct Budget {
    bytes: usize,
    objects: usize,
    max_bytes: usize,
    max_objects: usize,
}

impl Budget {
    fn new(max_bytes: usize, max_objects: usize) -> Self {
        Self {
            bytes: 0,
            objects: 0,
            max_bytes,
            max_objects,
        }
    }

    /// Reserve one object slot and its initial bytes.
    fn reserve_object(&mut self, bytes: usize) -> Result<(), Failure> {
        if self.objects >= self.max_objects {
            return Err(Failure::Limit);
        }
        let total = self.bytes.checked_add(bytes).ok_or(Failure::Limit)?;
        if total > self.max_bytes {
            return Err(Failure::Limit);
        }
        self.objects += 1;
        self.bytes = total;
        Ok(())
    }

    /// Reserve additional bytes for an existing object.
    fn reserve_bytes(&mut self, bytes: usize) -> Result<(), Failure> {
        let total = self.bytes.checked_add(bytes).ok_or(Failure::Limit)?;
        if total > self.max_bytes {
            return Err(Failure::Limit);
        }
        self.bytes = total;
        Ok(())
    }

    fn release_bytes(&mut self, bytes: usize) {
        self.bytes = self.bytes.saturating_sub(bytes);
    }
}

/// An object's lifetime lease on the runtime budget.
///
/// Bytes are released by `close`, while the object slot is released only when
/// this value is dropped (collection or runtime teardown).
struct Lease {
    budget: Rc<RefCell<Budget>>,
    bytes: usize,
    slot: bool,
}

impl Lease {
    fn acquire(budget: Rc<RefCell<Budget>>, bytes: usize) -> Result<Self, Failure> {
        budget.borrow_mut().reserve_object(bytes)?;
        Ok(Self {
            budget,
            bytes,
            slot: true,
        })
    }

    fn grow(&mut self, additional: usize) -> Result<(), Failure> {
        self.budget.borrow_mut().reserve_bytes(additional)?;
        self.bytes += additional;
        Ok(())
    }

    fn shrink(&mut self, additional: usize) {
        if additional == 0 {
            return;
        }
        self.budget.borrow_mut().release_bytes(additional);
        self.bytes -= additional;
    }

    fn release_all(&mut self) {
        if self.bytes == 0 {
            return;
        }
        self.budget.borrow_mut().release_bytes(self.bytes);
        self.bytes = 0;
    }

    fn budget(&self) -> Rc<RefCell<Budget>> {
        Rc::clone(&self.budget)
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.release_all();
        if self.slot {
            let mut budget = self.budget.borrow_mut();
            budget.objects = budget.objects.saturating_sub(1);
            self.slot = false;
        }
    }
}

/// A temporary byte reservation for a copy that becomes JavaScript-owned.
struct Reservation {
    budget: Rc<RefCell<Budget>>,
    bytes: usize,
}

impl Reservation {
    fn acquire(budget: Rc<RefCell<Budget>>, bytes: usize) -> Result<Self, Failure> {
        budget.borrow_mut().reserve_bytes(bytes)?;
        Ok(Self { budget, bytes })
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget.borrow_mut().release_bytes(self.bytes);
    }
}

/// Per-runtime native state: the captured buffer validator, Error class ID,
/// and aggregate byte/object budget.
struct IoState {
    validator: Option<Persistent<Function<'static>>>,
    error_class: Option<qjs::JSClassID>,
    budget: Rc<RefCell<Budget>>,
}

impl Default for IoState {
    fn default() -> Self {
        Self {
            validator: None,
            error_class: None,
            budget: Rc::new(RefCell::new(Budget::new(MAX_RUNTIME_BYTES, MAX_OBJECTS))),
        }
    }
}

// IoState contains no borrowed JavaScript value; its persistent handle is
// released by the extension teardown hook before the context is dropped.
unsafe impl<'js> JsLifetime<'js> for IoState {
    type Changed<'to> = Self;
}

impl IoState {
    /// Capture pristine intrinsics before any user code runs.
    fn install(ctx: &Ctx<'_>, state: &Rc<RefCell<Self>>) -> JsResult<()> {
        let factory: Function = ctx.eval(
            r#"(isArrayBuffer => {
                'use strict';
                const descriptor = Object.getOwnPropertyDescriptor;
                const apply = Reflect.apply;
                const typed = Object.getPrototypeOf(Uint8Array.prototype);
                const arrayBuffer = ArrayBuffer.prototype;
                const buffer = descriptor(typed, 'buffer').get;
                const detached = descriptor(arrayBuffer, 'detached').get;
                const resizable = descriptor(arrayBuffer, 'resizable').get;
                return value => {
                    let backing;
                    try {
                        backing = apply(buffer, value, []);
                    } catch {
                        return 1;
                    }
                    if (!isArrayBuffer(backing)) return 2;
                    if (apply(detached, backing, [])) return 3;
                    if (apply(resizable, backing, [])) return 4;
                    return 0;
                };
            })"#,
        )?;
        let validator: Function = factory.call((Function::new(ctx.clone(), is_array_buffer)?,))?;
        let error: Value = ctx.eval("new Error()")?;
        // SAFETY: this only reads the class identity of an Error created in
        // this context; it does not run user code.
        let class = unsafe { qjs::JS_GetClassID(error.as_raw()) };
        ctx.store_userdata(Rc::clone(state))
            .map_err(|_| Error::Unknown)?;
        let mut state = state.borrow_mut();
        state.error_class = Some(class);
        state.validator = Some(Persistent::save(ctx, validator));
        Ok(())
    }

    fn clear(&mut self) {
        self.validator = None;
    }
}

fn is_array_buffer(value: Value<'_>) -> bool {
    // SAFETY: this only tests the native backing-store brand.
    unsafe { qjs::JS_IsArrayBuffer(value.as_raw()) }
}

fn io_state(ctx: &Ctx<'_>) -> JsResult<Rc<RefCell<IoState>>> {
    ctx.userdata::<Rc<RefCell<IoState>>>()
        .map(|state| Rc::clone(&state))
        .ok_or(Error::Unknown)
}

/// Per-runtime state retained by the extension runtime for teardown.
struct IoExtensionState {
    state: Rc<RefCell<IoState>>,
}

fn init(ctx: &Ctx<'_>, _allowed: &[&str]) -> JsResult<Box<dyn Any>> {
    let state = Rc::new(RefCell::new(IoState::default()));
    IoState::install(ctx, &state)?;
    Ok(Box::new(IoExtensionState { state }))
}

fn teardown(state: &mut dyn Any) {
    if let Some(state) = state.downcast_ref::<IoExtensionState>() {
        state.state.borrow_mut().clear();
    }
}

fn load<'js>(ctx: &Ctx<'js>, state: &mut dyn Any, name: &str) -> JsResult<Module<'js, Declared>> {
    if name != MODULE_NAME {
        return Err(Error::new_loading_message(
            name,
            "unknown byte I/O module name",
        ));
    }
    if state.downcast_ref::<IoExtensionState>().is_none() || io_state(ctx).is_err() {
        return Err(Error::new_loading_message(
            name,
            "byte I/O extension state is unavailable",
        ));
    }
    Module::declare_def::<IoModule, _>(ctx.clone(), name)
}

struct IoModule;

impl ModuleDef for IoModule {
    fn declare<'js>(decl: &Declarations<'js>) -> JsResult<()> {
        for name in [
            "byteReader",
            "byteWriter",
            "MAX_BYTES",
            "MAX_OBJECTS",
            "MAX_RUNTIME_BYTES",
        ] {
            decl.declare(name)?;
        }
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> JsResult<()> {
        exports.export("byteReader", Function::new(ctx.clone(), byte_reader)?)?;
        exports.export("byteWriter", Function::new(ctx.clone(), byte_writer)?)?;
        exports.export("MAX_BYTES", MAX_BYTES)?;
        exports.export("MAX_RUNTIME_BYTES", MAX_RUNTIME_BYTES)?;
        exports.export("MAX_OBJECTS", MAX_OBJECTS)?;
        Ok(())
    }
}

/// A reader over a copied, owned byte snapshot.
struct Reader {
    data: Vec<u8>,
    position: usize,
    closed: bool,
    lease: Lease,
}

unsafe impl<'js> JsLifetime<'js> for Reader {
    type Changed<'to> = Self;
}

impl<'js> Trace<'js> for Reader {
    fn trace<'a>(&self, _tracer: Tracer<'a, 'js>) {}
}

impl<'js> JsClass<'js> for Reader {
    const NAME: &'static str = "ByteReader";
    type Mutable = Writable;

    fn constructor(_ctx: &Ctx<'js>) -> JsResult<Option<Constructor<'js>>> {
        Ok(None)
    }

    fn prototype(ctx: &Ctx<'js>) -> JsResult<Option<Object<'js>>> {
        let proto = Object::new_proto(ctx.clone(), None)?;
        proto.prop("read", Function::new(ctx.clone(), reader_read)?)?;
        proto.prop("close", Function::new(ctx.clone(), reader_close)?)?;
        proto.prop("remaining", Accessor::from(reader_remaining).enumerable())?;
        proto.prop("closed", Accessor::from(reader_closed).enumerable())?;
        Ok(Some(proto))
    }
}

/// A writer over an owned growable byte buffer.
struct Writer {
    data: Vec<u8>,
    closed: bool,
    lease: Lease,
}

unsafe impl<'js> JsLifetime<'js> for Writer {
    type Changed<'to> = Self;
}

impl<'js> Trace<'js> for Writer {
    fn trace<'a>(&self, _tracer: Tracer<'a, 'js>) {}
}

impl<'js> JsClass<'js> for Writer {
    const NAME: &'static str = "ByteWriter";
    type Mutable = Writable;

    fn constructor(_ctx: &Ctx<'js>) -> JsResult<Option<Constructor<'js>>> {
        Ok(None)
    }

    fn prototype(ctx: &Ctx<'js>) -> JsResult<Option<Object<'js>>> {
        let proto = Object::new_proto(ctx.clone(), None)?;
        proto.prop("write", Function::new(ctx.clone(), writer_write)?)?;
        proto.prop("toUint8Array", Function::new(ctx.clone(), writer_to_bytes)?)?;
        proto.prop("close", Function::new(ctx.clone(), writer_close)?)?;
        proto.prop("length", Accessor::from(writer_length).enumerable())?;
        proto.prop("closed", Accessor::from(writer_closed).enumerable())?;
        Ok(Some(proto))
    }
}

/// Create a reader from an owned snapshot of `value`.
fn byte_reader<'js>(ctx: Ctx<'js>, value: Value<'js>) -> JsResult<Class<'js, Reader>> {
    let state = io_state(&ctx)?;
    let budget = Rc::clone(&state.borrow().budget);
    drop(state);
    let (data, lease) = copy_bytes(&ctx, &value, MAX_BYTES, budget)?;
    Class::instance(
        ctx,
        Reader {
            data,
            position: 0,
            closed: false,
            lease,
        },
    )
}

/// Create an empty writer.
fn byte_writer<'js>(ctx: Ctx<'js>) -> JsResult<Class<'js, Writer>> {
    let state = io_state(&ctx)?;
    let budget = Rc::clone(&state.borrow().budget);
    drop(state);
    let lease = Lease::acquire(budget, 0).map_err(|failure| failure.throw(&ctx))?;
    Class::instance(
        ctx,
        Writer {
            data: Vec::new(),
            closed: false,
            lease,
        },
    )
}

/// Validate that `value` is a genuine, attached, fixed-length, non-shared
/// `Uint8Array` and return its raw view.
fn checked_span<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> JsResult<(*const u8, usize)> {
    let tag = unsafe { qjs::JS_GetTypedArrayType(value.as_raw()) };
    if tag != qjs::JSTypedArrayEnum_JS_TYPED_ARRAY_UINT8 as i32 {
        return Err(Failure::Argument.throw(ctx));
    }
    let state = io_state(ctx)?;
    let validator = state.borrow().validator.clone().ok_or(Error::Unknown)?;
    drop(state);
    let code: i32 = validator.restore(ctx)?.call((value.clone(),))?;
    match code {
        0 => {}
        1 => return Err(Failure::Argument.throw(ctx)),
        2 => return Err(Failure::Shared.throw(ctx)),
        3 => return Err(Failure::Detached.throw(ctx)),
        4 => return Err(Failure::Resizable.throw(ctx)),
        _ => return Err(Failure::Argument.throw(ctx)),
    }
    let mut len = 0;
    // SAFETY: the rooted value is a genuine attached Uint8Array with fixed
    // backing storage. No JavaScript runs between validation and this call.
    let ptr = unsafe { qjs::JS_GetUint8Array(ctx.as_raw().as_ptr(), &mut len, value.as_raw()) };
    if ptr.is_null() {
        if len == 0 {
            return Ok((std::ptr::NonNull::<u8>::dangling().as_ptr(), 0));
        }
        if ctx.has_exception() {
            let _ = ctx.catch();
        }
        return Err(Failure::Detached.throw(ctx));
    }
    Ok((ptr, len as usize))
}

/// Copy a checked view into an owned `Vec` while charging the runtime budget.
fn copy_bytes<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    max: usize,
    budget: Rc<RefCell<Budget>>,
) -> JsResult<(Vec<u8>, Lease)> {
    let (ptr, len) = checked_span(ctx, value)?;
    if len > max {
        return Err(Failure::Limit.throw(ctx));
    }
    let lease = Lease::acquire(budget, len).map_err(|failure| failure.throw(ctx))?;
    let mut out = Vec::new();
    if out.try_reserve_exact(len).is_err() {
        drop(lease);
        return Err(Failure::Limit.throw(ctx));
    }
    if len != 0 {
        // SAFETY: the pointer covers `len` bytes of a rooted view and no
        // JavaScript runs during the copy.
        out.extend_from_slice(unsafe { slice::from_raw_parts(ptr, len) });
    }
    Ok((out, lease))
}

fn reader_read<'js>(
    ctx: Ctx<'js>,
    this: This<Class<'js, Reader>>,
    target: Value<'js>,
) -> JsResult<usize> {
    // Closed precedence: do not validate the target for a closed reader.
    let (closed, remaining) = {
        let reader = this.0.try_borrow()?;
        (reader.closed, reader.data.len() - reader.position)
    };
    if closed {
        return Err(Failure::ReaderClosed.throw(&ctx));
    }
    let (ptr, len) = checked_span(&ctx, &target)?;
    if len == 0 {
        return Err(Failure::Argument.throw(&ctx));
    }
    if remaining == 0 {
        return Ok(0);
    }
    let count = remaining.min(len);
    let mut reader = this.0.try_borrow_mut()?;
    if reader.closed {
        drop(reader);
        return Err(Failure::ReaderClosed.throw(&ctx));
    }
    // SAFETY: `target` is a rooted fixed-length Uint8Array, `reader.data` is
    // reader-owned Rust storage, the ranges are disjoint, and no JavaScript
    // runs during the copy. The mutable borrow ends before any error is
    // thrown.
    unsafe {
        std::ptr::copy_nonoverlapping(
            reader.data.as_ptr().add(reader.position),
            ptr.cast_mut(),
            count,
        );
    }
    reader.position += count;
    Ok(count)
}

fn reader_remaining<'js>(this: This<Class<'js, Reader>>) -> JsResult<usize> {
    let reader = this.0.try_borrow()?;
    Ok(if reader.closed {
        0
    } else {
        reader.data.len() - reader.position
    })
}

fn reader_closed<'js>(this: This<Class<'js, Reader>>) -> JsResult<bool> {
    Ok(this.0.try_borrow()?.closed)
}

fn reader_close<'js>(this: This<Class<'js, Reader>>) -> JsResult<()> {
    let mut reader = this.0.try_borrow_mut()?;
    reader.data = Vec::new();
    reader.position = 0;
    reader.lease.release_all();
    reader.closed = true;
    Ok(())
}

fn writer_write<'js>(
    ctx: Ctx<'js>,
    this: This<Class<'js, Writer>>,
    value: Value<'js>,
) -> JsResult<usize> {
    // Closed precedence: do not validate the input for a closed writer.
    if this.0.try_borrow()?.closed {
        return Err(Failure::WriterClosed.throw(&ctx));
    }
    let (ptr, len) = checked_span(&ctx, &value)?;
    let result = {
        let mut writer = this.0.try_borrow_mut()?;
        if writer.closed {
            Err(Failure::WriterClosed)
        } else {
            writer_append(&mut writer, ptr, len)
        }
    };
    result.map_err(|failure| failure.throw(&ctx))
}

/// Append a validated source without running JavaScript.
fn writer_append(writer: &mut Writer, ptr: *const u8, len: usize) -> Result<usize, Failure> {
    let new_len = writer.data.len().checked_add(len).ok_or(Failure::Limit)?;
    if new_len > MAX_BYTES {
        return Err(Failure::Limit);
    }
    if len == 0 {
        return Ok(0);
    }
    writer.lease.grow(len)?;
    if writer.data.try_reserve_exact(len).is_err() {
        writer.lease.shrink(len);
        return Err(Failure::Limit);
    }
    // SAFETY: `ptr` covers `len` bytes of a rooted fixed-length Uint8Array,
    // `writer.data` is writer-owned Rust storage, the ranges are disjoint, and
    // no JavaScript runs during the copy. A panic would release the lease.
    writer
        .data
        .extend_from_slice(unsafe { slice::from_raw_parts(ptr, len) });
    Ok(len)
}

fn writer_to_bytes<'js>(
    ctx: Ctx<'js>,
    this: This<Class<'js, Writer>>,
) -> JsResult<TypedArray<'js, u8>> {
    let (closed, budget, len) = {
        let writer = this.0.try_borrow()?;
        (writer.closed, writer.lease.budget(), writer.data.len())
    };
    if closed {
        return Err(Failure::WriterClosed.throw(&ctx));
    }
    let reservation = Reservation::acquire(budget, len).map_err(|failure| failure.throw(&ctx))?;
    let mut bytes = Vec::new();
    if bytes.try_reserve_exact(len).is_err() {
        drop(reservation);
        return Err(Failure::Limit.throw(&ctx));
    }
    {
        let writer = this.0.try_borrow()?;
        if writer.closed {
            drop(writer);
            drop(reservation);
            return Err(Failure::WriterClosed.throw(&ctx));
        }
        bytes.extend_from_slice(&writer.data);
    }
    // No native borrow is held across this JavaScript call. The returned
    // Uint8Array is JavaScript-owned and outside the native budget.
    let array = TypedArray::new(ctx, bytes)?;
    drop(reservation);
    Ok(array)
}

fn writer_length<'js>(this: This<Class<'js, Writer>>) -> JsResult<usize> {
    let writer = this.0.try_borrow()?;
    Ok(if writer.closed { 0 } else { writer.data.len() })
}

fn writer_closed<'js>(this: This<Class<'js, Writer>>) -> JsResult<bool> {
    Ok(this.0.try_borrow()?.closed)
}

fn writer_close<'js>(this: This<Class<'js, Writer>>) -> JsResult<()> {
    let mut writer = this.0.try_borrow_mut()?;
    writer.data = Vec::new();
    writer.lease.release_all();
    writer.closed = true;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use neojs_core::rquickjs::{Context, Runtime};

    fn reader(budget: &Rc<RefCell<Budget>>, bytes: &[u8]) -> Reader {
        let lease = Lease::acquire(Rc::clone(budget), bytes.len()).unwrap();
        Reader {
            data: bytes.to_vec(),
            position: 0,
            closed: false,
            lease,
        }
    }

    #[test]
    fn budget_accounts_objects_bytes_and_rolls_back_failed_growth() {
        // Byte pressure: the second acquire fails and changes nothing.
        let bytes = Rc::new(RefCell::new(Budget::new(4, 2)));
        let lease = Lease::acquire(Rc::clone(&bytes), 4).unwrap();
        assert_eq!(bytes.borrow().bytes, 4);
        assert_eq!(bytes.borrow().objects, 1);
        assert!(Lease::acquire(Rc::clone(&bytes), 1).is_err());
        assert_eq!(bytes.borrow().bytes, 4);
        assert_eq!(bytes.borrow().objects, 1);
        drop(lease);
        assert_eq!(bytes.borrow().bytes, 0);
        assert_eq!(bytes.borrow().objects, 0);

        // Growth pressure: a failed reservation leaves the lease unchanged.
        let mut lease = Lease::acquire(Rc::clone(&bytes), 1).unwrap();
        assert!(lease.grow(4).is_err());
        assert_eq!(bytes.borrow().bytes, 1);
        lease.release_all();
        assert_eq!(bytes.borrow().bytes, 0);
        assert_eq!(bytes.borrow().objects, 1);
        drop(lease);
        assert_eq!(bytes.borrow().objects, 0);

        // Object pressure: the first slot is released only by drop/teardown.
        let objects = Rc::new(RefCell::new(Budget::new(100, 1)));
        let first = Lease::acquire(Rc::clone(&objects), 0).unwrap();
        assert!(Lease::acquire(Rc::clone(&objects), 0).is_err());
        drop(first);
        assert!(Lease::acquire(Rc::clone(&objects), 0).is_ok());

        // A temporary output reservation is released when it is dropped.
        let temporary = Rc::new(RefCell::new(Budget::new(4, 1)));
        let reservation = Reservation::acquire(Rc::clone(&temporary), 4).unwrap();
        assert_eq!(temporary.borrow().bytes, 4);
        drop(reservation);
        assert_eq!(temporary.borrow().bytes, 0);
    }

    #[test]
    fn drop_releases_bytes_and_object_slot() {
        let budget = Rc::new(RefCell::new(Budget::new(8, 2)));
        {
            let _reader = reader(&budget, b"abc");
            let lease = Lease::acquire(Rc::clone(&budget), 1).unwrap();
            assert_eq!(budget.borrow().bytes, 4);
            drop(lease);
            assert_eq!(budget.borrow().bytes, 3);
        }
        assert_eq!(budget.borrow().bytes, 0);
        assert_eq!(budget.borrow().objects, 0);
    }

    #[test]
    fn collection_releases_class_owned_budget() {
        let budget = Rc::new(RefCell::new(Budget::new(8, 1)));
        let runtime = Runtime::new().unwrap();
        let context = Context::full(&runtime).unwrap();
        context.with(|ctx| {
            let object = Class::instance(ctx.clone(), reader(&budget, b"abc")).unwrap();
            // A self-cycle requires the cycle collector rather than refcounts.
            object.set("self", object.clone()).unwrap();
            assert_eq!(budget.borrow().bytes, 3);
            drop(object);
            ctx.run_gc();
        });
        assert_eq!(budget.borrow().bytes, 0);
        assert_eq!(budget.borrow().objects, 0);
    }

    #[test]
    fn runtime_teardown_releases_class_owned_budget() {
        let budget = Rc::new(RefCell::new(Budget::new(8, 1)));
        {
            let runtime = Runtime::new().unwrap();
            let context = Context::full(&runtime).unwrap();
            context.with(|ctx| {
                let _object = Class::instance(ctx.clone(), reader(&budget, b"abc")).unwrap();
                assert_eq!(budget.borrow().bytes, 3);
            });
        }
        assert_eq!(budget.borrow().bytes, 0);
        assert_eq!(budget.borrow().objects, 0);
    }
}
