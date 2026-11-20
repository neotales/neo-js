//! Disposal of caught panic payloads.
//!
//! A `panic_any` payload can have a destructor that itself panics. Dropping such
//! a payload without a catch turns the destructor failure into a double panic,
//! which aborts. Every payload is therefore dropped under its own catch, and a
//! replacement payload is consumed the same way.
//!
//! A payload whose destructor unconditionally panics with another payload whose
//! destructor unconditionally panics creates an endless chain. There is no way
//! to dispose of such a payload without either looping forever or leaking it,
//! so this helper requires the chain to be finite. Ordinary payloads and every
//! finite chain are dropped without leaking. The fs worker has the same
//! documented limitation.

use std::{
    any::Any,
    panic::{AssertUnwindSafe, catch_unwind},
};

/// Drop a caught panic payload, containing any panic from its destructor.
///
/// The replacement payload from a panicking destructor is dropped in the next
/// iteration. This keeps ordinary payloads from leaking and contains finite
/// chains. An intentionally endless destructor chain hangs this function; see
/// the module documentation.
pub(crate) fn discard_panic(mut payload: Box<dyn Any + Send>) {
    while let Err(next) = catch_unwind(AssertUnwindSafe(|| drop(payload))) {
        payload = next;
    }
}

/// Drop an ordinary value, disposing of a panic payload if its destructor
/// panics.
pub(crate) fn discard<T>(value: T) {
    if let Err(payload) = catch_unwind(AssertUnwindSafe(|| drop(value))) {
        discard_panic(payload);
    }
}
