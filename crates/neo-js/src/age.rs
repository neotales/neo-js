//! Synchronous age file encryption and decryption for `neo:age`.
//!
//! The age format and X25519 recipient/identity handling come from the vetted
//! Rust [`age`] crate (the same implementation used by `rage`). This module
//! only defines the JavaScript-facing API, the size bounds, and the error/code
//! mapping. It is not a sandbox and it does not manage keys for the caller.
//!
//! # Private callbacks
//!
//! The module is a native [`ModuleDef`], not a JavaScript source module with
//! host functions on the global object. The JavaScript wrapper is created by
//! [`factory`] in the fresh realm before user code runs. The wrapper closes
//! over the original `Error` and `TypeError` constructors, the original
//! `Object.defineProperty`, and the host callbacks passed by
//! [`AgeModule::evaluate`]. None is stored as a mutable global. This is not isolation from other code with access to values in the
//! same realm.
//!
//! # Interoperability
//!
//! [`encrypt`] returns a standard binary age v1 file encrypted to one X25519
//! recipient. [`decrypt`] accepts standard binary age v1 files encrypted to
//! X25519 recipients. ASCII armor, passphrase encryption, SSH keys, plugins,
//! and post-quantum recipients are not exposed. A standard `age` command-line
//! tool can decrypt output from this module, and this module can decrypt its
//! binary X25519 output. The tests include a fixture produced independently by
//! `age` 1.3.2.
//!
//! # Keys and memory
//!
//! [`generate_identity`] asks the age crate for a new X25519 identity, which
//! draws from the operating system CSPRNG. The identity and recipient are
//! returned as standard Bech32 text (`AGE-SECRET-KEY-1...` and `age1...`).
//!
//! This module does not promise zeroization. The identity string, plaintext,
//! ciphertext, and intermediate key material are copied into Rust buffers and
//! into the JavaScript heap. Those copies are reclaimed normally but are not
//! guaranteed to be erased first. No secret is logged or included in an error
//! message; the fixed messages below never contain input bytes.
//!
//! # Randomness and panics
//!
//! age 0.11 has no public RNG injection point: `Encryptor` and
//! `x25519::Identity::generate` call the operating system CSPRNG internally
//! and panic if it fails. Identity generation and the whole encryption
//! operation are therefore wrapped in `catch_unwind`, and a panic is reported
//! as `NEO_AGE_RANDOM`. A `panic = "abort"` build cannot contain the panic and
//! aborts the process instead. The only known panic cause in this path is OS
//! RNG failure; an unexpected panic is also reported as `NEO_AGE_RANDOM`
//! rather than being allowed to cross the JavaScript boundary.
//!
//! # Limits and allocation
//!
//! The public module accepts at most [`MAX_PLAINTEXT_BYTES`] of plaintext, and
//! a ciphertext of at most [`MAX_CIPHERTEXT_BYTES`]. Decryption also stops if
//! the recovered plaintext would exceed [`MAX_PLAINTEXT_BYTES`]. Identity and
//! recipient text is limited by [`MAX_KEY_TEXT_BYTES`] before trimming. The
//! host bounds key text and byte inputs before copying them. The core functions
//! use checked arithmetic, a bounded writer, and fallible reservations; a
//! refused reservation is reported as `NEO_AGE_LIMIT`.
//!
//! That mapping is best effort, not a guarantee. An allocation failure inside
//! QuickJS, the Rust allocator, or the JavaScript heap can abort the process or
//! raise an error the module cannot catch, so recovery from out-of-memory is
//! not guaranteed. The age crate parses a header into memory before the
//! ciphertext bound is enforced on the payload; the whole ciphertext is bounded
//! first, so header memory is bounded by the same limit.

use std::io::{self, Read, Write};

use ::age::{
    Decryptor, Encryptor,
    secrecy::ExposeSecret,
    x25519::{Identity, Recipient},
};
use rquickjs::{
    Ctx, Error, Function, Object, Persistent, Result as JsResult, String as JsString, TypedArray,
    Value,
    module::{Declarations, Exports, ModuleDef},
    qjs,
};

/// Largest plaintext accepted by the public module.
pub(crate) const MAX_PLAINTEXT_BYTES: usize = 8 * 1024 * 1024;
/// Largest complete age file accepted by the public module.
///
/// This leaves room for the header and per-chunk authentication tags of an age
/// file whose plaintext is at the public plaintext bound. A valid external age
/// file with an unusually large header can exceed this neo-js limit.
pub(crate) const MAX_CIPHERTEXT_BYTES: usize = MAX_PLAINTEXT_BYTES + 1024 * 1024;
/// Longest identity or recipient text accepted before parsing.
pub(crate) const MAX_KEY_TEXT_BYTES: usize = 512;

const READ_CHUNK_BYTES: usize = 64 * 1024;

/// Size bounds used by the core and overridable in unit tests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AgeLimits {
    pub(crate) max_plaintext: usize,
    pub(crate) max_ciphertext: usize,
}

impl Default for AgeLimits {
    fn default() -> Self {
        Self {
            max_plaintext: MAX_PLAINTEXT_BYTES,
            max_ciphertext: MAX_CIPHERTEXT_BYTES,
        }
    }
}

/// Stable machine-readable failure category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AgeErrorCode {
    /// The identity text is not a valid age X25519 identity.
    Identity,
    /// The recipient text is not a valid age X25519 recipient.
    Recipient,
    /// The age file is truncated, names an unknown format, or uses a format
    /// this module does not support (such as passphrase encryption).
    Format,
    /// No supplied identity matched a syntactically valid age file.
    IdentityMismatch,
    /// A header MAC or payload authentication tag did not verify.
    Authentication,
    /// The view is detached or out of bounds.
    Detached,
    /// An input or output exceeded a documented bound, or allocation failed.
    Limit,
    /// The operating system CSPRNG failed while generating an identity or
    /// encrypting, or the age crate panicked in the encryption path.
    Random,
    /// The age crate reported an unexpected failure.
    Internal,
}

impl AgeErrorCode {
    /// The value exposed as the JavaScript error's `code` property.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Identity => "NEO_AGE_IDENTITY",
            Self::Recipient => "NEO_AGE_RECIPIENT",
            Self::Format => "NEO_AGE_FORMAT",
            Self::IdentityMismatch => "NEO_AGE_IDENTITY_MISMATCH",
            Self::Authentication => "NEO_AGE_AUTH",
            Self::Detached => "NEO_AGE_DETACHED",
            Self::Limit => "NEO_AGE_LIMIT",
            Self::Random => "NEO_AGE_RANDOM",
            Self::Internal => "NEO_AGE_INTERNAL",
        }
    }
}

/// A failure safe to describe without echoing identity, plaintext, or
/// ciphertext.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AgeError {
    code: AgeErrorCode,
}

impl AgeError {
    fn new(code: AgeErrorCode) -> Self {
        Self { code }
    }

    fn limit() -> Self {
        Self::new(AgeErrorCode::Limit)
    }

    fn internal() -> Self {
        Self::new(AgeErrorCode::Internal)
    }

    pub(crate) fn code(self) -> AgeErrorCode {
        self.code
    }

    /// A fixed message. It never contains input bytes.
    pub(crate) fn message(self) -> &'static str {
        match self.code {
            AgeErrorCode::Identity => "identity must be a valid age X25519 identity",
            AgeErrorCode::Recipient => "recipient must be a valid age X25519 recipient",
            AgeErrorCode::Format => "malformed, truncated, or unsupported age file",
            AgeErrorCode::IdentityMismatch => "no supplied identity matches this age file",
            AgeErrorCode::Authentication => "age file authentication failed",
            AgeErrorCode::Detached => "input is detached or out of bounds",
            AgeErrorCode::Limit => "input or output exceeds the neo:age size limit",
            AgeErrorCode::Random => {
                "operating system random number generator failed or age operation panicked"
            }
            AgeErrorCode::Internal => "age operation failed",
        }
    }
}

/// A Vec writer that refuses to grow past `max` and uses fallible reservation.
struct BoundedVec {
    bytes: Vec<u8>,
    max: usize,
}

impl BoundedVec {
    fn with_limit(max: usize) -> Self {
        Self {
            bytes: Vec::new(),
            max,
        }
    }

    fn into_inner(self) -> Vec<u8> {
        self.bytes
    }
}

fn limit_io() -> io::Error {
    io::Error::other("neo:age output size limit")
}

impl Write for BoundedVec {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let new_length = self
            .bytes
            .len()
            .checked_add(data.len())
            .ok_or_else(limit_io)?;
        if new_length > self.max {
            return Err(limit_io());
        }
        self.bytes.try_reserve(data.len()).map_err(|_| limit_io())?;
        self.bytes.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn parse_identity(text: &str) -> Result<Identity, AgeError> {
    if text.len() > MAX_KEY_TEXT_BYTES {
        return Err(AgeError::new(AgeErrorCode::Identity));
    }
    text.trim()
        .parse::<Identity>()
        .map_err(|_| AgeError::new(AgeErrorCode::Identity))
}

fn parse_recipient(text: &str) -> Result<Recipient, AgeError> {
    if text.len() > MAX_KEY_TEXT_BYTES {
        return Err(AgeError::new(AgeErrorCode::Recipient));
    }
    text.trim()
        .parse::<Recipient>()
        .map_err(|_| AgeError::new(AgeErrorCode::Recipient))
}

/// Clear a pending QuickJS exception without exposing it to the caller.
fn clear_pending_exception(ctx: *mut qjs::JSContext) {
    let exception = unsafe { qjs::JS_GetException(ctx) };
    unsafe { qjs::JS_FreeValue(ctx, exception) };
}

/// Bound a JavaScript key string before converting it to an owned Rust string.
///
/// `value` is a primitive JavaScript string. QuickJS's native property lookup
/// on a string returns `JSString::len` directly for `length` (UTF-16 code
/// units); it does not box the string or consult `String.prototype`, so no
/// caller-controlled accessor runs. JavaScript strings use at most three UTF-8
/// bytes per UTF-16 code unit. Rejecting more than `max` code units first makes
/// the subsequent copy O(`max`) even for a huge padded input; the exact UTF-8
/// length is then checked before trimming.
fn copy_key_text(value: &JsString<'_>, max: usize, code: AgeErrorCode) -> Result<String, AgeError> {
    let error = || AgeError::new(code);
    let ctx = value.ctx().as_raw().as_ptr();
    // SAFETY: `value` belongs to this context. The property name is static and
    // the returned value is owned, freed below or on the exception path.
    let length_value =
        unsafe { qjs::JS_GetPropertyStr(ctx, value.as_value().as_raw(), c"length".as_ptr()) };
    if unsafe { qjs::JS_IsException(length_value) } {
        clear_pending_exception(ctx);
        return Err(error());
    }
    let mut utf16_length: i64 = 0;
    // SAFETY: `length_value` is an owned value from this context. `length` on a
    // string is always an integer, and the value is freed exactly once.
    let status = unsafe { qjs::JS_ToInt64(ctx, &mut utf16_length, length_value) };
    unsafe { qjs::JS_FreeValue(ctx, length_value) };
    if status < 0 {
        clear_pending_exception(ctx);
        return Err(error());
    }
    let utf16_length = usize::try_from(utf16_length).map_err(|_| error())?;
    if utf16_length > max {
        return Err(error());
    }
    let text = value.to_string().map_err(|_| error())?;
    if text.len() > max {
        return Err(error());
    }
    Ok(text)
}

/// Contain panics from the age crate and map them to `NEO_AGE_RANDOM`.
///
/// age 0.11.5 calls `OsRng` internally and panics on failure; it exposes no
/// fallible or injectable alternative. This wrapper is the boundary used by
/// production encryption. `AssertUnwindSafe` is sound here because no value is
/// observed after a panic: the operation's output and buffers are discarded.
fn catch_age_panic<T>(operation: impl FnOnce() -> Result<T, AgeError>) -> Result<T, AgeError> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)) {
        Ok(result) => result,
        Err(_) => Err(AgeError::new(AgeErrorCode::Random)),
    }
}

/// Generate a new X25519 identity and its recipient.
///
/// The age crate draws the secret from the operating system CSPRNG. A panic
/// from that path is reported as `NEO_AGE_RANDOM` when the build uses
/// unwinding; a `panic = "abort"` build cannot report it.
pub(crate) fn generate_identity() -> Result<(String, String), AgeError> {
    catch_age_panic(|| {
        let identity = Identity::generate();
        let recipient = identity.to_public().to_string();
        let identity = identity.to_string().expose_secret().to_owned();
        Ok((identity, recipient))
    })
}

/// Derive the recipient for an X25519 identity.
pub(crate) fn identity_to_recipient(identity: &str) -> Result<String, AgeError> {
    Ok(parse_identity(identity)?.to_public().to_string())
}

/// Encrypt `plaintext` to one X25519 recipient as a binary age v1 file.
///
/// The whole age call is contained with `catch_unwind`; a panic (the known
/// cause is OS RNG failure) is reported as `NEO_AGE_RANDOM`. A
/// `panic = "abort"` build cannot contain it.
pub(crate) fn encrypt(
    recipient: &str,
    plaintext: Vec<u8>,
    limits: AgeLimits,
) -> Result<Vec<u8>, AgeError> {
    encrypt_with(recipient, plaintext, limits, encrypt_inner)
}

/// Validate inputs, then run `operation` inside the panic boundary.
///
/// Split from [`encrypt_inner`] so a unit test can inject a panic at the same
/// point as a production RNG failure without adding an RNG seam to production.
fn encrypt_with<F>(
    recipient: &str,
    plaintext: Vec<u8>,
    limits: AgeLimits,
    operation: F,
) -> Result<Vec<u8>, AgeError>
where
    F: FnOnce(&Recipient, Vec<u8>, AgeLimits) -> Result<Vec<u8>, AgeError>,
{
    if plaintext.len() > limits.max_plaintext {
        return Err(AgeError::limit());
    }
    let recipient = parse_recipient(recipient)?;
    catch_age_panic(|| operation(&recipient, plaintext, limits))
}

fn encrypt_inner(
    recipient: &Recipient,
    plaintext: Vec<u8>,
    limits: AgeLimits,
) -> Result<Vec<u8>, AgeError> {
    let recipients: [&dyn ::age::Recipient; 1] = [recipient];
    let encryptor =
        Encryptor::with_recipients(recipients.into_iter()).map_err(|_| AgeError::internal())?;

    let mut output = BoundedVec::with_limit(limits.max_ciphertext);
    let mut writer = encryptor
        .wrap_output(&mut output)
        .map_err(|_| AgeError::limit())?;
    writer
        .write_all(&plaintext)
        .map_err(|_| AgeError::limit())?;
    writer.finish().map_err(|_| AgeError::limit())?;
    Ok(output.into_inner())
}

/// Decrypt a binary age v1 file with one X25519 identity.
///
/// Reading is chunked and bounded, so the recovered plaintext cannot grow past
/// `limits.max_plaintext` before the limit is reported.
pub(crate) fn decrypt(
    identity: &str,
    ciphertext: Vec<u8>,
    limits: AgeLimits,
) -> Result<Vec<u8>, AgeError> {
    if ciphertext.len() > limits.max_ciphertext {
        return Err(AgeError::limit());
    }
    let identity = parse_identity(identity)?;
    let decryptor = Decryptor::new_buffered(&ciphertext[..]).map_err(map_decrypt_error)?;
    if decryptor.is_scrypt() {
        // A passphrase-encrypted file is valid age, but this module exposes
        // only X25519 identities. Report it as unsupported rather than as an
        // identity mismatch.
        return Err(AgeError::new(AgeErrorCode::Format));
    }
    let mut reader = decryptor
        .decrypt(std::iter::once(&identity as &dyn ::age::Identity))
        .map_err(map_decrypt_error)?;

    let mut plaintext = Vec::new();
    let mut buffer = [0u8; READ_CHUNK_BYTES];
    loop {
        let count = reader.read(&mut buffer).map_err(map_read_error)?;
        if count == 0 {
            break;
        }
        let new_length = plaintext
            .len()
            .checked_add(count)
            .ok_or_else(AgeError::limit)?;
        if new_length > limits.max_plaintext {
            return Err(AgeError::limit());
        }
        plaintext
            .try_reserve(count)
            .map_err(|_| AgeError::limit())?;
        plaintext.extend_from_slice(&buffer[..count]);
    }
    Ok(plaintext)
}

#[allow(unreachable_patterns)] // `age/plugin` adds a variant through feature unification.
fn map_decrypt_error(error: ::age::DecryptError) -> AgeError {
    match error {
        ::age::DecryptError::NoMatchingKeys => AgeError::new(AgeErrorCode::IdentityMismatch),
        ::age::DecryptError::InvalidMac
        | ::age::DecryptError::KeyDecryptionFailed
        | ::age::DecryptError::DecryptionFailed => AgeError::new(AgeErrorCode::Authentication),
        ::age::DecryptError::InvalidHeader
        | ::age::DecryptError::UnknownFormat
        | ::age::DecryptError::ExcessiveWork { .. } => AgeError::new(AgeErrorCode::Format),
        ::age::DecryptError::Io(error) => map_read_error(error),
        _ => AgeError::internal(),
    }
}

fn map_read_error(error: io::Error) -> AgeError {
    match error.kind() {
        io::ErrorKind::InvalidData => AgeError::new(AgeErrorCode::Authentication),
        // A truncated file and any other in-memory reader failure are
        // structural problems, not a wrong identity.
        _ => AgeError::new(AgeErrorCode::Format),
    }
}

/// Copy an input after checking its length against `max`, before allocating.
fn copy_bounded(array: &TypedArray<'_, u8>, max: usize) -> Result<Vec<u8>, AgeError> {
    let bytes = match array.as_raw() {
        // SAFETY: no JavaScript runs between reading the pointer and copying
        // it below, and the returned Vec owns its own bytes afterwards.
        Some(raw) => unsafe { raw.as_ref() },
        None => return Err(AgeError::new(AgeErrorCode::Detached)),
    };
    if bytes.len() > max {
        return Err(AgeError::limit());
    }
    let mut out = Vec::new();
    out.try_reserve_exact(bytes.len())
        .map_err(|_| AgeError::limit())?;
    out.extend_from_slice(bytes);
    Ok(out)
}

/// Convert a byte result into the object the JavaScript wrapper unwraps.
///
/// The object has a null prototype so a script cannot intercept an own
/// property through `Object.prototype`. `{ ok: true, value }` carries a
/// `Uint8Array`; `{ ok: false, code, message }` carries the stable failure.
/// The wrapper, not this function, throws.
fn outcome<'js>(ctx: &Ctx<'js>, result: Result<Vec<u8>, AgeError>) -> JsResult<Object<'js>> {
    let object = Object::new_proto(ctx.clone(), None)?;
    match result {
        Ok(bytes) => {
            object.set("ok", true)?;
            object.set("value", TypedArray::new(ctx.clone(), bytes)?)?;
        }
        Err(error) => {
            object.set("ok", false)?;
            object.set("code", error.code().as_str())?;
            object.set("message", error.message())?;
        }
    }
    Ok(object)
}

pub(crate) fn string_outcome<'js>(
    ctx: &Ctx<'js>,
    result: Result<String, AgeError>,
) -> JsResult<Object<'js>> {
    let object = Object::new_proto(ctx.clone(), None)?;
    match result {
        Ok(value) => {
            object.set("ok", true)?;
            object.set("value", value)?;
        }
        Err(error) => {
            object.set("ok", false)?;
            object.set("code", error.code().as_str())?;
            object.set("message", error.message())?;
        }
    }
    Ok(object)
}

fn generate_identity_js<'js>(ctx: Ctx<'js>) -> JsResult<Object<'js>> {
    let object = Object::new_proto(ctx.clone(), None)?;
    match generate_identity() {
        Ok((identity, recipient)) => {
            object.set("ok", true)?;
            object.set("identity", identity)?;
            object.set("recipient", recipient)?;
        }
        Err(error) => {
            object.set("ok", false)?;
            object.set("code", error.code().as_str())?;
            object.set("message", error.message())?;
        }
    }
    Ok(object)
}

pub(crate) fn identity_to_recipient_js<'js>(
    ctx: Ctx<'js>,
    identity: JsString<'js>,
) -> JsResult<Object<'js>> {
    let result = copy_key_text(&identity, MAX_KEY_TEXT_BYTES, AgeErrorCode::Identity)
        .and_then(|identity| identity_to_recipient(&identity));
    string_outcome(&ctx, result)
}

pub(crate) fn encrypt_js<'js>(
    ctx: Ctx<'js>,
    recipient: JsString<'js>,
    plaintext: TypedArray<'js, u8>,
) -> JsResult<Object<'js>> {
    let limits = AgeLimits::default();
    let result = (|| {
        let recipient = copy_key_text(&recipient, MAX_KEY_TEXT_BYTES, AgeErrorCode::Recipient)?;
        let plaintext = copy_bounded(&plaintext, limits.max_plaintext)?;
        encrypt(&recipient, plaintext, limits)
    })();
    outcome(&ctx, result)
}

pub(crate) fn decrypt_js<'js>(
    ctx: Ctx<'js>,
    identity: JsString<'js>,
    ciphertext: TypedArray<'js, u8>,
) -> JsResult<Object<'js>> {
    let limits = AgeLimits::default();
    let result = (|| {
        let identity = copy_key_text(&identity, MAX_KEY_TEXT_BYTES, AgeErrorCode::Identity)?;
        let ciphertext = copy_bounded(&ciphertext, limits.max_ciphertext)?;
        decrypt(&identity, ciphertext, limits)
    })();
    outcome(&ctx, result)
}

/// The persistent factory and private callbacks for `neo:age`.
///
/// The factory is created in [`crate::NeoJs::new`] before user code runs. The
/// persistent handle is released in `NeoJs::drop`.
#[derive(Default)]
pub(crate) struct AgeState {
    pub(crate) factory: Option<Persistent<Function<'static>>>,
}

// AgeState contains no borrowed JS value; its persistent handle is explicitly
// released before the context and runtime are dropped.
unsafe impl<'js> rquickjs::JsLifetime<'js> for AgeState {
    type Changed<'to> = AgeState;
}

/// The native `neo:age` module.
///
/// Its factory receives the host callbacks privately. Nothing is exported on
/// the global object, so a script cannot replace a callback before or after
/// importing this module.
pub(crate) struct AgeModule;

impl ModuleDef for AgeModule {
    fn declare<'js>(decl: &Declarations<'js>) -> JsResult<()> {
        for name in [
            "generateIdentity",
            "identityToRecipient",
            "encrypt",
            "decrypt",
        ] {
            decl.declare(name)?;
        }
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> JsResult<()> {
        use std::{cell::RefCell, rc::Rc};
        let state = Rc::clone(
            &*ctx
                .userdata::<Rc<RefCell<AgeState>>>()
                .ok_or(Error::Unknown)?,
        );
        let factory = state
            .borrow()
            .factory
            .as_ref()
            .cloned()
            .ok_or(Error::Unknown)?
            .restore(ctx)?;
        let generate = Function::new(ctx.clone(), generate_identity_js)?;
        let derive = Function::new(ctx.clone(), identity_to_recipient_js)?;
        let encrypt = Function::new(ctx.clone(), encrypt_js)?;
        let decrypt = Function::new(ctx.clone(), decrypt_js)?;
        let api: Object = factory.call((generate, derive, encrypt, decrypt))?;
        for name in [
            "generateIdentity",
            "identityToRecipient",
            "encrypt",
            "decrypt",
        ] {
            exports.export(name, api.get::<_, Value>(name)?)?;
        }
        Ok(())
    }
}

/// Build the private JavaScript wrapper in the fresh realm.
///
/// This is called before user code. The wrapper captures the original `Error`
/// and `TypeError` constructors and `Object.defineProperty`; the host callbacks
/// are passed in only when the module is evaluated.
pub(crate) fn factory<'js>(ctx: &Ctx<'js>) -> JsResult<Function<'js>> {
    ctx.eval(
        r#"(function() {
            'use strict';
            const NativeError = Error;
            const NativeTypeError = TypeError;
            const defineProperty = Object.defineProperty;

            function fail(response) {
                const error = new NativeError(response.message);
                // Define an own property instead of assigning: an inherited
                // `Object.prototype.code` accessor must not replace or swallow
                // the stable code. The descriptor must not inherit
                // Object.prototype.get/set either.
                defineProperty(error, 'code', {
                    __proto__: null,
                    value: response.code,
                    writable: true,
                    enumerable: true,
                    configurable: true,
                });
                throw error;
            }

            function unwrap(response) {
                if (response.ok) return response.value;
                fail(response);
            }

            function requireString(value, name) {
                if (typeof value !== 'string') {
                    throw new NativeTypeError(name + ' must be a string');
                }
                return value;
            }

            return function(generateHost, deriveHost, encryptHost, decryptHost) {
                return {
                    generateIdentity() {
                        const response = generateHost();
                        if (!response.ok) fail(response);
                        return { identity: response.identity, recipient: response.recipient };
                    },
                    identityToRecipient(identity) {
                        requireString(identity, 'identity');
                        return unwrap(deriveHost(identity));
                    },
                    encrypt(recipient, plaintext) {
                        requireString(recipient, 'recipient');
                        return unwrap(encryptHost(recipient, plaintext));
                    },
                    decrypt(identity, ciphertext) {
                        requireString(identity, 'identity');
                        return unwrap(decryptHost(identity, ciphertext));
                    }
                };
            };
        })()"#,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(value: &str) -> Vec<u8> {
        hex::decode(value).unwrap()
    }

    fn default_limits() -> AgeLimits {
        AgeLimits::default()
    }

    // X25519 test vector published in the age crate's own unit tests. It is a
    // public test key, never a credential, and must only be used for fixtures.
    const TEST_IDENTITY: &str =
        "AGE-SECRET-KEY-1GQ9778VQXMMJVE8SK7J6VT8UJ4HDQAJUVSFCWCM02D8GEWQ72PVQ2Y5J33";
    const TEST_RECIPIENT: &str = "age1t7rxyev2z3rw82stdlrrepyc39nvn86l5078zqkf5uasdy86jp6svpy7pa";

    #[test]
    fn test_identity_derives_the_published_recipient() {
        assert_eq!(
            identity_to_recipient(TEST_IDENTITY).unwrap(),
            TEST_RECIPIENT
        );
    }

    #[test]
    fn independent_age_cli_fixture_decrypts() {
        // Produced with the age 1.3.2 command-line tool:
        //   printf 'neo:age interop fixture' |
        //     age --encrypt --recipient <TEST_RECIPIENT>
        // The identity above is the matching public upstream test key.
        let ciphertext = decode(
            "6167652d656e6372797074696f6e2e6f72672f76310a2d3e20583235353139203865565a39572b2b7346766b5a2b4f6f52764c3930367346785737726d2f57762b4a79762f4147377067410a30486632774b554d717734646b38466f3479773747354c4b6d67666c336f315a444145426b4a68764c50380a2d2d2d2066343976564d30694d696942456a454972686b6b5237394a72504d656f47744e7730725a305267782b6a730a299f3a639e16f5d4605db5bc12063db20d7dc0b76fce5a856ac3dbfe20e767ae66c874752e9da729e2e610bfd65ef16edcd38ce8f45dfa",
        );
        let plaintext = decrypt(TEST_IDENTITY, ciphertext, default_limits()).unwrap();
        assert_eq!(plaintext, b"neo:age interop fixture");
    }

    #[test]
    fn generated_identities_roundtrip_and_are_distinct() {
        let (first_identity, first_recipient) = generate_identity().unwrap();
        let (second_identity, second_recipient) = generate_identity().unwrap();
        assert_ne!(first_identity, second_identity);
        assert_ne!(first_recipient, second_recipient);
        assert_eq!(
            identity_to_recipient(&first_identity).unwrap(),
            first_recipient
        );
        let ciphertext = encrypt(&first_recipient, b"hello".to_vec(), default_limits()).unwrap();
        assert_eq!(
            decrypt(&first_identity, ciphertext, default_limits()).unwrap(),
            b"hello"
        );
    }

    #[test]
    fn roundtrips_empty_binary_unicode_and_large_payloads() {
        let (identity, recipient) = generate_identity().unwrap();
        let unicode = "Grüße, 世界, 🦀".as_bytes();
        let all_bytes: Vec<u8> = (0..=255u8).collect();
        let mut large = vec![0u8; 1024 * 1024 + 3];
        for (index, byte) in large.iter_mut().enumerate() {
            *byte = (index % 251) as u8;
        }
        for plaintext in [&b""[..], unicode, &all_bytes, &large] {
            let ciphertext = encrypt(&recipient, plaintext.to_vec(), default_limits()).unwrap();
            assert!(ciphertext.starts_with(b"age-encryption.org/v1\n"));
            assert_eq!(
                decrypt(&identity, ciphertext, default_limits()).unwrap(),
                plaintext
            );
        }
    }

    #[test]
    fn wrong_identity_is_reported_separately_from_format_errors() {
        let (identity, _recipient) = generate_identity().unwrap();
        let (_other_identity, other_recipient) = generate_identity().unwrap();
        let ciphertext = encrypt(&other_recipient, b"secret".to_vec(), default_limits()).unwrap();

        assert_eq!(
            decrypt(&identity, ciphertext.clone(), default_limits())
                .unwrap_err()
                .code(),
            AgeErrorCode::IdentityMismatch
        );
        assert_eq!(
            decrypt(&identity, Vec::new(), default_limits())
                .unwrap_err()
                .code(),
            AgeErrorCode::Format
        );
        assert_eq!(
            decrypt(&identity, b"not an age file".to_vec(), default_limits())
                .unwrap_err()
                .code(),
            AgeErrorCode::Format
        );
    }

    #[test]
    fn passphrase_encrypted_files_are_reported_as_unsupported() {
        let passphrase = ::age::secrecy::SecretString::from("not a real passphrase".to_owned());
        let encryptor = Encryptor::with_user_passphrase(passphrase);
        let mut ciphertext = Vec::new();
        {
            let mut writer = encryptor.wrap_output(&mut ciphertext).unwrap();
            writer.write_all(b"secret").unwrap();
            writer.finish().unwrap();
        }
        assert_eq!(
            decrypt(TEST_IDENTITY, ciphertext, default_limits())
                .unwrap_err()
                .code(),
            AgeErrorCode::Format
        );
    }

    #[test]
    fn every_single_bit_flip_is_rejected() {
        let (identity, recipient) = generate_identity().unwrap();
        let ciphertext = encrypt(&recipient, b"\x00\xffbinary".to_vec(), default_limits()).unwrap();
        for byte in 0..ciphertext.len() {
            for bit in 0..8 {
                let mut tampered = ciphertext.clone();
                tampered[byte] ^= 1 << bit;
                let result = decrypt(&identity, tampered, default_limits());
                assert!(result.is_err(), "byte {byte} bit {bit} was accepted");
                let code = result.unwrap_err().code();
                assert!(
                    matches!(
                        code,
                        AgeErrorCode::Authentication
                            | AgeErrorCode::Format
                            | AgeErrorCode::IdentityMismatch
                    ),
                    "unexpected code {code:?} for byte {byte} bit {bit}"
                );
            }
        }
    }

    #[test]
    fn payload_tampering_is_an_authentication_failure() {
        let (identity, recipient) = generate_identity().unwrap();
        let mut ciphertext = encrypt(&recipient, b"payload".to_vec(), default_limits()).unwrap();
        let last = ciphertext.len() - 1;
        ciphertext[last] ^= 1;
        assert_eq!(
            decrypt(&identity, ciphertext, default_limits())
                .unwrap_err()
                .code(),
            AgeErrorCode::Authentication
        );
    }

    #[test]
    fn key_text_limit_is_checked_before_trimming() {
        let padding = |length: usize| " ".repeat(length);
        let identity = format!(
            "{}{}",
            padding(MAX_KEY_TEXT_BYTES - TEST_IDENTITY.len()),
            TEST_IDENTITY
        );
        assert_eq!(identity.len(), MAX_KEY_TEXT_BYTES);
        assert_eq!(identity_to_recipient(&identity).unwrap(), TEST_RECIPIENT);
        assert_eq!(
            identity_to_recipient(&format!(" {identity}"))
                .unwrap_err()
                .code(),
            AgeErrorCode::Identity
        );

        let recipient = format!(
            "{}{}",
            padding(MAX_KEY_TEXT_BYTES - TEST_RECIPIENT.len()),
            TEST_RECIPIENT
        );
        assert_eq!(recipient.len(), MAX_KEY_TEXT_BYTES);
        assert!(
            !encrypt(&recipient, b"x".to_vec(), default_limits())
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            encrypt(&format!(" {recipient}"), b"x".to_vec(), default_limits())
                .unwrap_err()
                .code(),
            AgeErrorCode::Recipient
        );

        let huge = format!("{}{}{}", padding(10_000), TEST_IDENTITY, padding(10_000));
        assert_eq!(
            identity_to_recipient(&huge).unwrap_err().code(),
            AgeErrorCode::Identity
        );
        let huge_recipient = format!("{}{}", padding(10_000), TEST_RECIPIENT);
        assert_eq!(
            encrypt(&huge_recipient, b"x".to_vec(), default_limits())
                .unwrap_err()
                .code(),
            AgeErrorCode::Recipient
        );
    }

    #[test]
    fn unicode_padding_counts_utf8_bytes_before_trimming() {
        // U+00A0 is Unicode whitespace and two UTF-8 bytes. The identity is 74
        // bytes, so 219 of them plus the key is exactly 512 bytes.
        let padding = "\u{00a0}".repeat((MAX_KEY_TEXT_BYTES - TEST_IDENTITY.len()) / 2);
        assert_eq!(padding.len(), 438);
        let exact = format!("{padding}{TEST_IDENTITY}");
        assert_eq!(exact.len(), MAX_KEY_TEXT_BYTES);
        assert_eq!(identity_to_recipient(&exact).unwrap(), TEST_RECIPIENT);
        assert_eq!(
            identity_to_recipient(&format!("\u{00a0}{exact}"))
                .unwrap_err()
                .code(),
            AgeErrorCode::Identity
        );

        // A 512-byte string that is not a key passes the length bound and
        // fails parsing.
        let not_a_key = "\u{00e9}".repeat(MAX_KEY_TEXT_BYTES / 2);
        assert_eq!(not_a_key.len(), MAX_KEY_TEXT_BYTES);
        assert_eq!(
            identity_to_recipient(&not_a_key).unwrap_err().code(),
            AgeErrorCode::Identity
        );
    }

    #[test]
    fn encryption_rng_panic_maps_to_random() {
        // age 0.11.5 exposes no RNG injection point; the OS RNG is internal
        // and panics on failure. Inject the panic at the same point as a
        // production RNG failure inside the validation/containment wrapper,
        // without weakening production randomness.
        let result = encrypt_with(
            TEST_RECIPIENT,
            b"x".to_vec(),
            default_limits(),
            |_, _, _| panic!("simulated OS RNG failure"),
        );
        assert_eq!(result.unwrap_err().code(), AgeErrorCode::Random);
    }

    #[test]
    fn invalid_identity_and_recipient_text_is_rejected() {
        let (identity, recipient) = generate_identity().unwrap();
        for bad in [
            "",
            "not-a-key",
            "age1notavalidrecipient",
            "AGE-SECRET-KEY-1AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        ] {
            assert_eq!(
                identity_to_recipient(bad).unwrap_err().code(),
                AgeErrorCode::Identity
            );
            assert_eq!(
                encrypt(bad, b"x".to_vec(), default_limits())
                    .unwrap_err()
                    .code(),
                AgeErrorCode::Recipient
            );
        }
        assert_eq!(
            decrypt(&recipient, b"x".to_vec(), default_limits())
                .unwrap_err()
                .code(),
            AgeErrorCode::Identity
        );
        assert_eq!(
            decrypt(&identity, b"x".to_vec(), default_limits())
                .unwrap_err()
                .code(),
            AgeErrorCode::Format
        );
        let long = "A".repeat(MAX_KEY_TEXT_BYTES + 1);
        assert_eq!(
            identity_to_recipient(&long).unwrap_err().code(),
            AgeErrorCode::Identity
        );
        assert_eq!(
            encrypt(&long, b"x".to_vec(), default_limits())
                .unwrap_err()
                .code(),
            AgeErrorCode::Recipient
        );
    }

    #[test]
    fn small_limits_are_checked_at_the_boundary() {
        let (identity, recipient) = generate_identity().unwrap();
        let limits = AgeLimits {
            max_plaintext: 16,
            max_ciphertext: 1024,
        };

        let ciphertext = encrypt(&recipient, vec![0u8; 16], limits).unwrap();
        assert!(decrypt(&identity, ciphertext.clone(), limits).is_ok());
        assert_eq!(
            encrypt(&recipient, vec![0u8; 17], limits)
                .unwrap_err()
                .code(),
            AgeErrorCode::Limit
        );
        assert_eq!(
            decrypt(
                &identity,
                ciphertext.clone(),
                AgeLimits {
                    max_plaintext: 16,
                    max_ciphertext: ciphertext.len() - 1,
                }
            )
            .unwrap_err()
            .code(),
            AgeErrorCode::Limit
        );
        assert_eq!(
            decrypt(
                &identity,
                ciphertext,
                AgeLimits {
                    max_plaintext: 15,
                    max_ciphertext: 1024,
                }
            )
            .unwrap_err()
            .code(),
            AgeErrorCode::Limit
        );
    }

    #[test]
    fn ciphertext_limit_is_enforced_before_parsing() {
        let (identity, _recipient) = generate_identity().unwrap();
        assert_eq!(
            decrypt(
                &identity,
                vec![0u8; 4],
                AgeLimits {
                    max_plaintext: 0,
                    max_ciphertext: 3,
                }
            )
            .unwrap_err()
            .code(),
            AgeErrorCode::Limit
        );
    }

    #[test]
    fn bounded_writer_refuses_growth_after_the_limit() {
        let mut writer = BoundedVec::with_limit(3);
        writer.write_all(b"abc").unwrap();
        assert_eq!(writer.write(b"d").unwrap_err().kind(), io::ErrorKind::Other);
        assert_eq!(writer.into_inner(), b"abc");
    }

    #[test]
    fn default_limits_match_the_documented_constants() {
        let limits = AgeLimits::default();
        assert_eq!(limits.max_plaintext, MAX_PLAINTEXT_BYTES);
        assert_eq!(limits.max_ciphertext, MAX_CIPHERTEXT_BYTES);
    }
}
