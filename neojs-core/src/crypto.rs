//! Synchronous AES-256-GCM authenticated encryption for `neo:crypto`.
//!
//! The cryptographic primitive is the RustCrypto `aes-gcm` crate; this module
//! only defines the byte envelope, the associated-data construction, the size
//! bounds, and the JavaScript-facing error/code mapping. It is not a sandbox
//! and it does not manage keys for the caller.
//!
//! # Private callbacks
//!
//! The module is a native [`ModuleDef`], not a JavaScript source module with
//! host functions on the global object. The JavaScript wrapper is created by
//! [`factory`] in the fresh realm before user code runs. The wrapper closes
//! over the original `Error` constructor, the original `Object.defineProperty`,
//! and the host callbacks passed by [`CryptoModule::evaluate`]. None is stored
//! as a mutable global. This is not isolation from other code with access to
//! values in the same realm.
//!
//! # Envelope
//!
//! Every value returned by [`encrypt`] has this layout:
//!
//! ```text
//! offset  size  field
//! 0       4     magic, ASCII "NEO1"
//! 4       1     envelope version, 1
//! 5       1     algorithm, 1 = AES-256-GCM
//! 6       12    nonce
//! 18      n     ciphertext (same length as the plaintext)
//! 18+n    16    authentication tag
//! ```
//!
//! The first 18 bytes (header and nonce) followed by the caller's associated
//! data are passed to AES-GCM as associated data. A decryptor that supplies a
//! different header, nonce, or associated-data value gets an authentication
//! failure, never plaintext. The magic, version, and algorithm bytes are
//! checked before the primitive runs, so an unknown format is reported as
//! malformed rather than as an authentication failure.
//!
//! The envelope is specific to neo-js. The primitive is standard AES-256-GCM
//! (NIST SP 800-38D), and the unit tests below check it against published
//! vectors, but another tool must reproduce this envelope and
//! associated-data construction to interoperate. One fixture in those tests
//! was produced independently with Node's OpenSSL-backed AES-256-GCM.
//!
//! # Nonces
//!
//! [`encrypt`] draws a fresh 96-bit nonce from the operating system CSPRNG via
//! `getrandom` for every call. The nonce is not derived from a counter and the
//! caller cannot choose it. A random 96-bit nonce is safe for a limited number
//! of messages under one key: the probability of any collision reaches about
//! 2^-32 at roughly 2^32 encryptions, so a key should be retired well before
//! that. Reusing a key with a repeated nonce destroys AES-GCM's confidentiality
//! and authenticity guarantees; the module avoids the common cause by never
//! exposing a nonce-reuse path, but key lifetime is still the caller's
//! responsibility.
//!
//! # Keys and memory
//!
//! Keys are exactly 32 bytes. [`generate_key`] returns 32 CSPRNG bytes; keys
//! can also come from any other vetted CSPRNG. The module does not store,
//! derive, wrap, rotate, or persist keys. Callers must keep the key secret,
//! use a distinct key per purpose, and protect the places it is stored.
//!
//! This module does not promise zeroization. Key bytes are copied into Rust
//! buffers and into the AES key schedule, plaintext and ciphertext are copied
//! into Rust buffers, and the JavaScript heap retains its own copies. Those
//! copies are reclaimed normally but are not guaranteed to be erased first.
//! Callers that require memory erasure must arrange it at a lower layer. No
//! secret is logged or included in an error message.
//!
//! # Limits and allocation
//!
//! The public module accepts at most [`MAX_PLAINTEXT_BYTES`] of plaintext,
//! [`MAX_ASSOCIATED_DATA_BYTES`] of caller associated data, and an envelope of
//! at most [`MAX_ENVELOPE_BYTES`]. The host checks these sizes before copying
//! or allocating. The core functions use checked arithmetic and fallible
//! reservations; a failed reservation is reported as
//! `NEO_CRYPTO_LIMIT`.
//!
//! That mapping is best effort, not a guarantee. An allocation failure inside
//! QuickJS, the Rust allocator, or the JavaScript heap can abort the process or
//! raise an error the module cannot catch, so recovery from out-of-memory is
//! not guaranteed.

use aes_gcm::{
    Aes256Gcm, Nonce, Tag,
    aead::{AeadInPlace, KeyInit},
};
use rquickjs::{
    Ctx, Error, Function, Object, Persistent, Result as JsResult, TypedArray, Value,
    function::Opt,
    module::{Declarations, Exports, ModuleDef},
};

/// Length of an AES-256 key in bytes.
pub(crate) const KEY_LEN: usize = 32;
/// Length of the AES-GCM nonce in bytes.
pub(crate) const NONCE_LEN: usize = 12;
/// Length of the AES-GCM authentication tag in bytes.
pub(crate) const TAG_LEN: usize = 16;

const MAGIC: &[u8; 4] = b"NEO1";
const VERSION: u8 = 1;
const ALGORITHM_AES_256_GCM: u8 = 1;
/// Magic, version, and algorithm.
const HEADER_LEN: usize = 6;
/// Header, nonce, and tag prepended/appended around the ciphertext.
pub(crate) const OVERHEAD: usize = HEADER_LEN + NONCE_LEN + TAG_LEN;

/// Largest plaintext accepted by the public module.
pub(crate) const MAX_PLAINTEXT_BYTES: usize = 8 * 1024 * 1024;
/// Largest caller associated-data value accepted by the public module.
pub(crate) const MAX_ASSOCIATED_DATA_BYTES: usize = 8 * 1024 * 1024;
/// Largest complete envelope accepted by the public module.
pub(crate) const MAX_ENVELOPE_BYTES: usize = MAX_PLAINTEXT_BYTES + OVERHEAD;

/// Size bounds used by the core and overridable in unit tests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CryptoLimits {
    pub(crate) max_plaintext: usize,
    pub(crate) max_associated_data: usize,
}

impl Default for CryptoLimits {
    fn default() -> Self {
        Self {
            max_plaintext: MAX_PLAINTEXT_BYTES,
            max_associated_data: MAX_ASSOCIATED_DATA_BYTES,
        }
    }
}

impl CryptoLimits {
    fn check_plaintext(self, length: usize) -> Result<(), CryptoError> {
        if length > self.max_plaintext {
            Err(CryptoError::new(CryptoErrorCode::Limit))
        } else {
            Ok(())
        }
    }

    fn check_associated_data(self, length: usize) -> Result<(), CryptoError> {
        if length > self.max_associated_data {
            Err(CryptoError::new(CryptoErrorCode::Limit))
        } else {
            Ok(())
        }
    }

    fn max_envelope(self) -> Result<usize, CryptoError> {
        self.max_plaintext
            .checked_add(OVERHEAD)
            .ok_or_else(|| CryptoError::new(CryptoErrorCode::Limit))
    }
}

/// Stable machine-readable failure category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CryptoErrorCode {
    /// The key is not exactly 32 bytes.
    Key,
    /// The envelope is truncated or names an unknown format.
    Format,
    /// The tag did not verify under the supplied key and associated data.
    Authentication,
    /// The operating system CSPRNG failed.
    Random,
    /// An input exceeded a documented bound or an allocation was refused.
    Limit,
    /// The view is detached or out of bounds.
    Detached,
}

impl CryptoErrorCode {
    /// The value exposed as the JavaScript error's `code` property.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Key => "NEO_CRYPTO_KEY",
            Self::Format => "NEO_CRYPTO_FORMAT",
            Self::Authentication => "NEO_CRYPTO_AUTH",
            Self::Random => "NEO_CRYPTO_RANDOM",
            Self::Limit => "NEO_CRYPTO_LIMIT",
            Self::Detached => "NEO_CRYPTO_DETACHED",
        }
    }
}

/// A failure safe to describe without echoing key, plaintext, or ciphertext.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CryptoError {
    code: CryptoErrorCode,
}

impl CryptoError {
    fn new(code: CryptoErrorCode) -> Self {
        Self { code }
    }

    pub(crate) fn code(self) -> CryptoErrorCode {
        self.code
    }

    /// A fixed message. It never contains input bytes.
    pub(crate) fn message(self) -> &'static str {
        match self.code {
            CryptoErrorCode::Key => "key must be exactly 32 bytes",
            CryptoErrorCode::Format => "malformed or unsupported neo:crypto envelope",
            CryptoErrorCode::Authentication => "authentication failed",
            CryptoErrorCode::Random => "operating system random number generator failed",
            CryptoErrorCode::Limit => "input exceeds the neo:crypto size limit",
            CryptoErrorCode::Detached => "input is detached or out of bounds",
        }
    }
}

/// Length of the associated data this module will authenticate: the header,
/// the nonce, and the caller's bytes.
fn aad_capacity(associated_data_len: usize) -> Result<usize, CryptoError> {
    HEADER_LEN
        .checked_add(NONCE_LEN)
        .and_then(|fixed| fixed.checked_add(associated_data_len))
        .ok_or_else(|| CryptoError::new(CryptoErrorCode::Limit))
}

fn cipher(key: &[u8]) -> Result<Aes256Gcm, CryptoError> {
    if key.len() != KEY_LEN {
        return Err(CryptoError::new(CryptoErrorCode::Key));
    }
    // The only failure `new_from_slice` reports is a wrong key length, which
    // the check above already handled.
    Aes256Gcm::new_from_slice(key).map_err(|_| CryptoError::new(CryptoErrorCode::Key))
}

/// Encrypt with a caller-supplied nonce and associated data.
///
/// This is the primitive used by the known-answer tests. The public envelope
/// API never lets a caller choose a nonce.
#[cfg(test)]
fn seal_detached(
    key: &[u8],
    nonce: &[u8],
    associated_data: &[u8],
    plaintext: &[u8],
) -> Result<(Vec<u8>, [u8; TAG_LEN]), CryptoError> {
    if nonce.len() != NONCE_LEN {
        return Err(CryptoError::new(CryptoErrorCode::Format));
    }
    let cipher = cipher(key)?;
    let mut buffer = plaintext.to_vec();
    let tag = cipher
        .encrypt_in_place_detached(Nonce::from_slice(nonce), associated_data, &mut buffer)
        .map_err(|_| CryptoError::new(CryptoErrorCode::Limit))?;
    let mut tag_bytes = [0u8; TAG_LEN];
    tag_bytes.copy_from_slice(tag.as_slice());
    Ok((buffer, tag_bytes))
}

/// Decrypt a detached ciphertext and tag with a caller-supplied nonce.
#[cfg(test)]
fn open_detached(
    key: &[u8],
    nonce: &[u8],
    associated_data: &[u8],
    ciphertext: &[u8],
    tag: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    if nonce.len() != NONCE_LEN || tag.len() != TAG_LEN {
        return Err(CryptoError::new(CryptoErrorCode::Format));
    }
    let cipher = cipher(key)?;
    let mut buffer = ciphertext.to_vec();
    cipher
        .decrypt_in_place_detached(
            Nonce::from_slice(nonce),
            associated_data,
            &mut buffer,
            Tag::from_slice(tag),
        )
        .map_err(|_| CryptoError::new(CryptoErrorCode::Authentication))?;
    Ok(buffer)
}

/// Encrypt `plaintext` into a fresh versioned envelope.
///
/// The caller must have checked the input sizes; this function checks them
/// again before allocating. `plaintext` is taken by value so its buffer can be
/// encrypted in place without another plaintext copy.
pub(crate) fn encrypt(
    key: &[u8],
    plaintext: Vec<u8>,
    associated_data: &[u8],
    limits: CryptoLimits,
) -> Result<Vec<u8>, CryptoError> {
    if key.len() != KEY_LEN {
        return Err(CryptoError::new(CryptoErrorCode::Key));
    }
    limits.check_plaintext(plaintext.len())?;
    limits.check_associated_data(associated_data.len())?;

    let cipher = cipher(key)?;
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::fill(&mut nonce).map_err(|_| CryptoError::new(CryptoErrorCode::Random))?;

    let envelope_capacity = OVERHEAD
        .checked_add(plaintext.len())
        .ok_or_else(|| CryptoError::new(CryptoErrorCode::Limit))?;
    let mut envelope = Vec::new();
    envelope
        .try_reserve_exact(envelope_capacity)
        .map_err(|_| CryptoError::new(CryptoErrorCode::Limit))?;
    envelope.extend_from_slice(MAGIC);
    envelope.push(VERSION);
    envelope.push(ALGORITHM_AES_256_GCM);
    envelope.extend_from_slice(&nonce);

    // Bind the format bytes, version, algorithm, and nonce to the tag. A
    // decryptor that sees a different header or nonce fails authentication.
    let mut aad = Vec::new();
    aad.try_reserve_exact(aad_capacity(associated_data.len())?)
        .map_err(|_| CryptoError::new(CryptoErrorCode::Limit))?;
    aad.extend_from_slice(&envelope);
    aad.extend_from_slice(associated_data);

    let mut buffer = plaintext;
    let tag = cipher
        .encrypt_in_place_detached(Nonce::from_slice(&nonce), &aad, &mut buffer)
        // Our explicit bounds are far below AES-GCM's size limits, so a
        // primitive error here is a size/bound failure.
        .map_err(|_| CryptoError::new(CryptoErrorCode::Limit))?;

    envelope.extend_from_slice(&buffer);
    envelope.extend_from_slice(tag.as_slice());
    Ok(envelope)
}

/// Decrypt one versioned envelope.
///
/// The caller must have checked the input sizes; this function checks them
/// again before allocating. On success the envelope buffer becomes the
/// plaintext buffer, avoiding a separate plaintext allocation. On any failure
/// no plaintext is returned.
pub(crate) fn decrypt(
    key: &[u8],
    mut envelope: Vec<u8>,
    associated_data: &[u8],
    limits: CryptoLimits,
) -> Result<Vec<u8>, CryptoError> {
    if key.len() != KEY_LEN {
        return Err(CryptoError::new(CryptoErrorCode::Key));
    }
    if envelope.len() < OVERHEAD {
        return Err(CryptoError::new(CryptoErrorCode::Format));
    }
    if envelope.len() > limits.max_envelope()? {
        return Err(CryptoError::new(CryptoErrorCode::Limit));
    }
    limits.check_associated_data(associated_data.len())?;
    if &envelope[..MAGIC.len()] != MAGIC {
        return Err(CryptoError::new(CryptoErrorCode::Format));
    }
    if envelope[MAGIC.len()] != VERSION {
        return Err(CryptoError::new(CryptoErrorCode::Format));
    }
    if envelope[MAGIC.len() + 1] != ALGORITHM_AES_256_GCM {
        return Err(CryptoError::new(CryptoErrorCode::Format));
    }

    let ciphertext_len = envelope.len() - OVERHEAD;
    if ciphertext_len > limits.max_plaintext {
        return Err(CryptoError::new(CryptoErrorCode::Limit));
    }
    let cipher = cipher(key)?;
    let mut aad = Vec::new();
    aad.try_reserve_exact(aad_capacity(associated_data.len())?)
        .map_err(|_| CryptoError::new(CryptoErrorCode::Limit))?;
    aad.extend_from_slice(&envelope[..HEADER_LEN + NONCE_LEN]);
    aad.extend_from_slice(associated_data);

    let tag_start = envelope.len() - TAG_LEN;
    let (body, tag) = envelope.split_at_mut(tag_start);
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(&body[HEADER_LEN..HEADER_LEN + NONCE_LEN]);
    let ciphertext = &mut body[HEADER_LEN + NONCE_LEN..];
    cipher
        .decrypt_in_place_detached(
            Nonce::from_slice(&nonce),
            &aad,
            ciphertext,
            Tag::from_slice(tag),
        )
        // The explicit bounds above are below AES-GCM's size limits, so this
        // error is a tag failure. Size failures were already mapped to Limit.
        .map_err(|_| CryptoError::new(CryptoErrorCode::Authentication))?;

    // Discard the header/nonce prefix and the trailing tag. The remaining
    // bytes are the plaintext that was decrypted in place.
    envelope.drain(..HEADER_LEN + NONCE_LEN);
    envelope.truncate(ciphertext_len);
    Ok(envelope)
}

/// Return 32 random bytes from the operating system CSPRNG.
pub(crate) fn generate_key() -> Result<[u8; KEY_LEN], CryptoError> {
    let mut key = [0u8; KEY_LEN];
    getrandom::fill(&mut key).map_err(|_| CryptoError::new(CryptoErrorCode::Random))?;
    Ok(key)
}

/// Copy a slice with a fallible reservation.
fn fallible_copy(bytes: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let mut out = Vec::new();
    out.try_reserve_exact(bytes.len())
        .map_err(|_| CryptoError::new(CryptoErrorCode::Limit))?;
    out.extend_from_slice(bytes);
    Ok(out)
}

/// Copy a key, requiring exactly 32 bytes before allocating.
fn copy_key(array: &TypedArray<'_, u8>) -> Result<Vec<u8>, CryptoError> {
    let bytes = match array.as_raw() {
        // SAFETY: no JavaScript runs between reading the pointer and copying
        // it below, and the returned Vec owns its own bytes afterwards.
        Some(raw) => unsafe { raw.as_ref() },
        None => return Err(CryptoError::new(CryptoErrorCode::Detached)),
    };
    if bytes.len() != KEY_LEN {
        return Err(CryptoError::new(CryptoErrorCode::Key));
    }
    fallible_copy(bytes)
}

/// Copy an input after checking its length against `max`, before allocating.
fn copy_bounded(array: &TypedArray<'_, u8>, max: usize) -> Result<Vec<u8>, CryptoError> {
    let bytes = match array.as_raw() {
        // SAFETY: no JavaScript runs between reading the pointer and copying
        // it below, and the returned Vec owns its own bytes afterwards.
        Some(raw) => unsafe { raw.as_ref() },
        None => return Err(CryptoError::new(CryptoErrorCode::Detached)),
    };
    if bytes.len() > max {
        return Err(CryptoError::new(CryptoErrorCode::Limit));
    }
    fallible_copy(bytes)
}

/// Convert a Rust result into the object the JavaScript wrapper unwraps.
///
/// The object has a null prototype so a script cannot intercept an own
/// property through `Object.prototype`. `{ ok: true, value }` carries a
/// `Uint8Array`; `{ ok: false, code, message }` carries the stable failure.
/// The wrapper, not this function, throws.
fn outcome<'js>(ctx: &Ctx<'js>, result: Result<Vec<u8>, CryptoError>) -> JsResult<Object<'js>> {
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

pub(crate) fn encrypt_js<'js>(
    ctx: Ctx<'js>,
    key: TypedArray<'js, u8>,
    plaintext: TypedArray<'js, u8>,
    associated_data: Opt<TypedArray<'js, u8>>,
) -> JsResult<Object<'js>> {
    let limits = CryptoLimits::default();
    let result = (|| {
        let key = copy_key(&key)?;
        let plaintext = copy_bounded(&plaintext, limits.max_plaintext)?;
        let associated_data = match associated_data.0 {
            Some(value) => copy_bounded(&value, limits.max_associated_data)?,
            None => Vec::new(),
        };
        encrypt(&key, plaintext, &associated_data, limits)
    })();
    outcome(&ctx, result)
}

pub(crate) fn decrypt_js<'js>(
    ctx: Ctx<'js>,
    key: TypedArray<'js, u8>,
    envelope: TypedArray<'js, u8>,
    associated_data: Opt<TypedArray<'js, u8>>,
) -> JsResult<Object<'js>> {
    let limits = CryptoLimits::default();
    let result = (|| {
        let key = copy_key(&key)?;
        let envelope = copy_bounded(&envelope, MAX_ENVELOPE_BYTES)?;
        let associated_data = match associated_data.0 {
            Some(value) => copy_bounded(&value, limits.max_associated_data)?,
            None => Vec::new(),
        };
        decrypt(&key, envelope, &associated_data, limits)
    })();
    outcome(&ctx, result)
}

pub(crate) fn generate_key_js<'js>(ctx: Ctx<'js>) -> JsResult<Object<'js>> {
    outcome(&ctx, generate_key().map(|key| key.to_vec()))
}

/// The persistent factory and private callbacks for `neo:crypto`.
///
/// The factory is created in [`crate::NeoJs::new`] before user code runs. The
/// persistent handle is released in `NeoJs::drop`.
#[derive(Default)]
pub(crate) struct CryptoState {
    pub(crate) factory: Option<Persistent<Function<'static>>>,
}

// CryptoState contains no borrowed JS value; its persistent handle is
// explicitly released before the context and runtime are dropped.
unsafe impl<'js> rquickjs::JsLifetime<'js> for CryptoState {
    type Changed<'to> = CryptoState;
}

/// The native `neo:crypto` module.
///
/// Its factory receives the host callbacks privately. Nothing is exported on
/// the global object, so a script cannot replace a callback before or after
/// importing this module.
pub(crate) struct CryptoModule;

impl ModuleDef for CryptoModule {
    fn declare<'js>(decl: &Declarations<'js>) -> JsResult<()> {
        for name in ["generateKey", "encrypt", "decrypt"] {
            decl.declare(name)?;
        }
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> JsResult<()> {
        use std::{cell::RefCell, rc::Rc};
        let state = Rc::clone(
            &*ctx
                .userdata::<Rc<RefCell<CryptoState>>>()
                .ok_or(Error::Unknown)?,
        );
        let factory = state
            .borrow()
            .factory
            .as_ref()
            .cloned()
            .ok_or(Error::Unknown)?
            .restore(ctx)?;
        let encrypt = Function::new(ctx.clone(), encrypt_js)?;
        let decrypt = Function::new(ctx.clone(), decrypt_js)?;
        let generate_key = Function::new(ctx.clone(), generate_key_js)?;
        let api: Object = factory.call((encrypt, decrypt, generate_key))?;
        for name in ["generateKey", "encrypt", "decrypt"] {
            exports.export(name, api.get::<_, Value>(name)?)?;
        }
        Ok(())
    }
}

/// Build the private JavaScript wrapper in the fresh realm.
///
/// This is called before user code. The wrapper captures the original
/// `Error` constructor and `Object.defineProperty`;
/// the host callbacks are passed in only when the module is evaluated.
pub(crate) fn factory<'js>(ctx: &Ctx<'js>) -> JsResult<Function<'js>> {
    ctx.eval(
        r#"(function() {
            'use strict';
            const NativeError = Error;
            const defineProperty = Object.defineProperty;

            function unwrap(response) {
                if (response.ok) return response.value;
                const error = new NativeError(response.message);
                // Define an own property instead of assigning: an inherited
                // `Object.prototype.code` setter must not swallow or replace
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

            return function(encryptHost, decryptHost, keyHost) {
                return {
                    generateKey() {
                        return unwrap(keyHost());
                    },
                    encrypt(key, plaintext, associatedData) {
                        if (associatedData === undefined) {
                            return unwrap(encryptHost(key, plaintext));
                        }
                        return unwrap(encryptHost(key, plaintext, associatedData));
                    },
                    decrypt(key, envelope, associatedData) {
                        if (associatedData === undefined) {
                            return unwrap(decryptHost(key, envelope));
                        }
                        return unwrap(decryptHost(key, envelope, associatedData));
                    }
                };
            };
        })()"#,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        hex::encode(bytes)
    }

    fn default_limits() -> CryptoLimits {
        CryptoLimits::default()
    }

    // Test Cases 13-16 from "The Galois/Counter Mode of Operation (GCM)",
    // McGrew and Viega, Appendix B, also published in BoringSSL's
    // aes_256_gcm_tests.txt. These exercise the standard AES-256-GCM
    // primitive, not the neo-js envelope, so they use the vectors' own
    // associated data.
    struct Vector {
        name: &'static str,
        key: &'static str,
        nonce: &'static str,
        aad: &'static str,
        plaintext: &'static str,
        ciphertext: &'static str,
        tag: &'static str,
    }

    const VECTORS: [Vector; 4] = [
        Vector {
            name: "case 13 (empty)",
            key: "0000000000000000000000000000000000000000000000000000000000000000",
            nonce: "000000000000000000000000",
            aad: "",
            plaintext: "",
            ciphertext: "",
            tag: "530f8afbc74536b9a963b4f1c4cb738b",
        },
        Vector {
            name: "case 14 (one zero block)",
            key: "0000000000000000000000000000000000000000000000000000000000000000",
            nonce: "000000000000000000000000",
            aad: "",
            plaintext: "00000000000000000000000000000000",
            ciphertext: "cea7403d4d606b6e074ec5d3baf39d18",
            tag: "d0d1c8a799996bf0265b98b5d48ab919",
        },
        Vector {
            name: "case 15 (four blocks)",
            key: "feffe9928665731c6d6a8f9467308308feffe9928665731c6d6a8f9467308308",
            nonce: "cafebabefacedbaddecaf888",
            aad: "",
            plaintext: "d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a721c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b391aafd255",
            ciphertext: "522dc1f099567d07f47f37a32a84427d643a8cdcbfe5c0c97598a2bd2555d1aa8cb08e48590dbb3da7b08b1056828838c5f61e6393ba7a0abcc9f662898015ad",
            tag: "b094dac5d93471bdec1a502270e3cc6c",
        },
        Vector {
            name: "case 16 (four blocks with AAD)",
            key: "feffe9928665731c6d6a8f9467308308feffe9928665731c6d6a8f9467308308",
            nonce: "cafebabefacedbaddecaf888",
            aad: "feedfacedeadbeeffeedfacedeadbeefabaddad2",
            plaintext: "d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a721c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b39",
            ciphertext: "522dc1f099567d07f47f37a32a84427d643a8cdcbfe5c0c97598a2bd2555d1aa8cb08e48590dbb3da7b08b1056828838c5f61e6393ba7a0abcc9f662",
            tag: "76fc6ece0f4e1768cddf8853bb2d551b",
        },
    ];

    fn decode(value: &str) -> Vec<u8> {
        hex::decode(value).unwrap()
    }

    #[test]
    fn published_aes_256_gcm_vectors_match() {
        for vector in &VECTORS {
            let key = decode(vector.key);
            let nonce = decode(vector.nonce);
            let aad = decode(vector.aad);
            let plaintext = decode(vector.plaintext);
            let (ciphertext, tag) = seal_detached(&key, &nonce, &aad, &plaintext).unwrap();
            assert_eq!(hex(&ciphertext), vector.ciphertext, "{}", vector.name);
            assert_eq!(hex(&tag), vector.tag, "{}", vector.name);
            assert_eq!(
                open_detached(&key, &nonce, &aad, &ciphertext, &tag).unwrap(),
                plaintext,
                "{}",
                vector.name
            );
        }
    }

    #[test]
    fn independent_envelope_fixture_decrypts() {
        // Produced with Node's OpenSSL-backed aes-256-gcm using this module's
        // documented header, nonce, and associated-data layout. The key is a
        // deliberately public test vector, not a credential.
        let key = decode("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f");
        let envelope = decode(
            "4e454f310101000102030405060708090a0b2967b921a697bb6bf92eb7e2df9d1d1feca6a75299032b094a0230ed39f87f14b520fd7a392dc47ab999",
        );
        let plaintext = decrypt(&key, envelope, b"context-bytes", CryptoLimits::default()).unwrap();
        assert_eq!(plaintext, b"neo:crypto interop fixture");
    }

    #[test]
    fn envelope_has_the_documented_layout() {
        let key = [7u8; KEY_LEN];
        let plaintext = b"layout".to_vec();
        let envelope = encrypt(&key, plaintext.clone(), b"", default_limits()).unwrap();
        assert_eq!(&envelope[..4], b"NEO1");
        assert_eq!(envelope[4], VERSION);
        assert_eq!(envelope[5], ALGORITHM_AES_256_GCM);
        assert_eq!(envelope.len(), OVERHEAD + plaintext.len());
        let ciphertext = &envelope[HEADER_LEN + NONCE_LEN..envelope.len() - TAG_LEN];
        assert_eq!(ciphertext.len(), plaintext.len());
        assert_ne!(ciphertext, plaintext.as_slice());
        assert_ne!(
            &envelope[HEADER_LEN..HEADER_LEN + NONCE_LEN],
            &[0u8; NONCE_LEN]
        );
        assert_eq!(
            decrypt(&key, envelope, b"", default_limits()).unwrap(),
            plaintext
        );
    }

    #[test]
    fn roundtrips_empty_binary_and_unicode_payloads() {
        let key = [0x5au8; KEY_LEN];
        let unicode = "Grüße, 世界, 🦀".as_bytes();
        let all_bytes: Vec<u8> = (0..=255u8).collect();
        for (plaintext, aad) in [
            (&b""[..], &b""[..]),
            (unicode, "context: ünïcode".as_bytes()),
            (&all_bytes, &[0u8, 1, 2, 0xff]),
        ] {
            let envelope = encrypt(&key, plaintext.to_vec(), aad, default_limits()).unwrap();
            assert_eq!(
                decrypt(&key, envelope, aad, default_limits()).unwrap(),
                plaintext
            );
        }
    }

    #[test]
    fn every_single_bit_flip_is_rejected() {
        let key = [0x11u8; KEY_LEN];
        let envelope = encrypt(
            &key,
            b"\x00\xffbinary".to_vec(),
            b"associated",
            default_limits(),
        )
        .unwrap();
        for byte in 0..envelope.len() {
            for bit in 0..8 {
                let mut tampered = envelope.clone();
                tampered[byte] ^= 1 << bit;
                let result = decrypt(&key, tampered, b"associated", default_limits());
                assert!(result.is_err(), "byte {byte} bit {bit} was accepted");
            }
        }
    }

    #[test]
    fn wrong_key_and_wrong_associated_data_are_rejected() {
        let key = [0x22u8; KEY_LEN];
        let envelope = encrypt(&key, b"secret".to_vec(), b"context", default_limits()).unwrap();
        assert!(
            decrypt(
                &[0x23u8; KEY_LEN],
                envelope.clone(),
                b"context",
                default_limits()
            )
            .is_err()
        );
        assert!(decrypt(&key, envelope.clone(), b"other", default_limits()).is_err());
        assert!(decrypt(&key, envelope.clone(), b"", default_limits()).is_err());
        assert_eq!(
            decrypt(&key, envelope, b"context", default_limits()).unwrap(),
            b"secret"
        );
    }

    #[test]
    fn key_length_is_enforced() {
        for length in [0usize, 1, 16, 31, 33, 64] {
            let key = vec![0x33u8; length];
            assert_eq!(
                encrypt(&key, b"x".to_vec(), b"", default_limits())
                    .unwrap_err()
                    .code(),
                CryptoErrorCode::Key
            );
            assert_eq!(
                decrypt(&key, vec![0u8; OVERHEAD], b"", default_limits())
                    .unwrap_err()
                    .code(),
                CryptoErrorCode::Key
            );
        }
        assert!(encrypt(&[0x33u8; KEY_LEN], b"x".to_vec(), b"", default_limits()).is_ok());
    }

    #[test]
    fn malformed_envelopes_are_rejected() {
        let key = [0x44u8; KEY_LEN];
        assert_eq!(
            decrypt(&key, Vec::new(), b"", default_limits())
                .unwrap_err()
                .code(),
            CryptoErrorCode::Format
        );
        assert_eq!(
            decrypt(&key, vec![0u8; OVERHEAD - 1], b"", default_limits())
                .unwrap_err()
                .code(),
            CryptoErrorCode::Format
        );

        let envelope = encrypt(&key, b"payload".to_vec(), b"", default_limits()).unwrap();
        let mut bad_magic = envelope.clone();
        bad_magic[0] = b'X';
        assert_eq!(
            decrypt(&key, bad_magic, b"", default_limits())
                .unwrap_err()
                .code(),
            CryptoErrorCode::Format
        );

        let mut bad_version = envelope.clone();
        bad_version[4] = VERSION + 1;
        assert_eq!(
            decrypt(&key, bad_version, b"", default_limits())
                .unwrap_err()
                .code(),
            CryptoErrorCode::Format
        );

        let mut bad_algorithm = envelope.clone();
        bad_algorithm[5] = 2;
        assert_eq!(
            decrypt(&key, bad_algorithm, b"", default_limits())
                .unwrap_err()
                .code(),
            CryptoErrorCode::Format
        );

        let mut bad_tag = envelope;
        let last = bad_tag.len() - 1;
        bad_tag[last] ^= 1;
        assert_eq!(
            decrypt(&key, bad_tag, b"", default_limits())
                .unwrap_err()
                .code(),
            CryptoErrorCode::Authentication
        );
    }

    #[test]
    fn small_limits_are_checked_at_the_boundary() {
        let limits = CryptoLimits {
            max_plaintext: 8,
            max_associated_data: 5,
        };
        let key = [0x77u8; KEY_LEN];

        // Exactly at the limit is allowed.
        let envelope = encrypt(&key, vec![0u8; 8], b"12345", limits).unwrap();
        assert_eq!(envelope.len(), OVERHEAD + 8);
        assert!(decrypt(&key, envelope.clone(), b"12345", limits).is_ok());

        // One byte over is rejected before the core allocates.
        assert_eq!(
            encrypt(&key, vec![0u8; 9], b"", limits).unwrap_err().code(),
            CryptoErrorCode::Limit
        );
        assert_eq!(
            encrypt(&key, vec![0u8; 1], b"123456", limits)
                .unwrap_err()
                .code(),
            CryptoErrorCode::Limit
        );
        assert_eq!(
            decrypt(&key, envelope.clone(), b"123456", limits)
                .unwrap_err()
                .code(),
            CryptoErrorCode::Limit
        );
        assert_eq!(
            decrypt(&key, vec![0u8; OVERHEAD + 9], b"", limits)
                .unwrap_err()
                .code(),
            CryptoErrorCode::Limit
        );

        // At the envelope bound, a zero-filled envelope is a format error,
        // not a limit error.
        assert_eq!(
            decrypt(&key, vec![0u8; OVERHEAD + 8], b"", limits)
                .unwrap_err()
                .code(),
            CryptoErrorCode::Format
        );
    }

    #[test]
    fn checked_envelope_arithmetic_reports_a_limit() {
        let limits = CryptoLimits {
            max_plaintext: usize::MAX,
            max_associated_data: 0,
        };
        assert_eq!(
            limits.max_envelope().unwrap_err().code(),
            CryptoErrorCode::Limit
        );
        assert_eq!(
            decrypt(&[0u8; KEY_LEN], vec![0u8; OVERHEAD], b"", limits)
                .unwrap_err()
                .code(),
            CryptoErrorCode::Limit
        );
    }

    #[test]
    fn nonces_are_fresh_and_unique() {
        let key = [0x55u8; KEY_LEN];
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1_000 {
            let envelope =
                encrypt(&key, b"same plaintext".to_vec(), b"", default_limits()).unwrap();
            let nonce = &envelope[HEADER_LEN..HEADER_LEN + NONCE_LEN];
            assert_ne!(nonce, &[0u8; NONCE_LEN]);
            assert!(seen.insert(nonce.to_vec()), "duplicate nonce");
        }
    }

    #[test]
    fn large_payload_roundtrips() {
        let key = [0x66u8; KEY_LEN];
        let mut plaintext = vec![0u8; 1024 * 1024 + 3];
        for (index, byte) in plaintext.iter_mut().enumerate() {
            *byte = (index % 251) as u8;
        }
        let envelope = encrypt(&key, plaintext.clone(), b"large", default_limits()).unwrap();
        assert_eq!(
            decrypt(&key, envelope, b"large", default_limits()).unwrap(),
            plaintext,
            "large payload changed"
        );
    }

    #[test]
    fn generated_keys_are_full_length_and_distinct() {
        let first = generate_key().unwrap();
        let second = generate_key().unwrap();
        assert_eq!(first.len(), KEY_LEN);
        assert_eq!(second.len(), KEY_LEN);
        assert_ne!(first, second);
    }

    #[test]
    fn default_limits_match_the_documented_constants() {
        assert_eq!(
            CryptoLimits::default().max_envelope().unwrap(),
            MAX_ENVELOPE_BYTES
        );
        assert_eq!(MAX_ENVELOPE_BYTES, MAX_PLAINTEXT_BYTES + OVERHEAD);
    }
}
