//! Lifetime and access checks for registry handles owned by scripts.

use std::{
    cell::RefCell,
    rc::{Rc, Weak},
};

use rquickjs::{Ctx, Function, Object, Result as JsResult};

use super::{
    ErrorCode, Hive, RegistryError, TYPE_EXPAND_SZ, TYPE_SZ, decode_string, encode_string,
    invalid_path, invalid_value, limit_error, parse_hive, respond, type_mismatch,
    validate_key_path, validate_value_name,
};

/// Max simultaneously open script-owned keys in one runtime.
pub(super) const MAX_OPEN_KEYS: usize = 64;
const QUERY_VALUE: u32 = 0x00001;
const SET_VALUE: u32 = 0x00002;
const ALL_ACCESS: u32 = 0xf003f;

/// The backend receives only validated paths/masks, and operations use the
/// same owned handle that was returned by acquisition. Dropping it closes it.
pub(super) trait Handle {
    /// Read a bounded raw value from this handle.
    fn read(&self, name: &str) -> Result<(u32, Vec<u8>), RegistryError>;
    /// Store a validated UTF-16LE REG_SZ on this handle.
    fn write(&self, name: &str, bytes: &[u8]) -> Result<(), RegistryError>;
}

pub(super) type Acquired = Result<(Box<dyn Handle>, bool), RegistryError>;
#[cfg(test)]
type TestBackend = Box<dyn Fn(Hive, &str, u32, bool) -> Acquired>;

struct OpenKey {
    handle: Box<dyn Handle>,
    access: u32,
}

#[derive(Default)]
struct Slot(RefCell<Option<OpenKey>>);

impl Slot {
    fn close(&self) {
        let key = self.0.borrow_mut().take();
        drop(key);
    }

    fn with_access<T>(
        &self,
        right: u32,
        call: impl FnOnce(&dyn Handle) -> Result<T, RegistryError>,
    ) -> Result<T, RegistryError> {
        let key = self.0.borrow();
        let key = key
            .as_ref()
            .ok_or_else(|| RegistryError::new(ErrorCode::Closed))?;
        if key.access & right == 0 {
            return Err(RegistryError::new(ErrorCode::AccessDenied));
        }
        call(&*key.handle)
    }
}

/// A weak index for deterministic runtime shutdown, not a second owner.
/// JS callbacks own the slots; GC, explicit disposal, and shutdown all drop
/// the same handle at most once. Closed/dead slots are pruned on acquisition.
#[derive(Default)]
pub(super) struct Keys {
    slots: Vec<Weak<Slot>>,
    #[cfg(test)]
    backend: Option<TestBackend>,
}

impl Keys {
    /// Close every still-live slot before the enclosing runtime is destroyed.
    pub(super) fn close_all(&mut self) {
        for slot in self.slots.drain(..).filter_map(|slot| slot.upgrade()) {
            slot.close();
        }
    }

    fn acquire(
        &mut self,
        path: &str,
        access: u32,
        create: bool,
    ) -> Result<(Rc<Slot>, bool), RegistryError> {
        let (hive, subkey) = path.split_once('\\').ok_or_else(invalid_path)?;
        let hive = parse_hive(hive)?;
        // An empty subkey can return a borrowed predefined handle on Windows.
        validate_key_path(subkey)?;
        if access == 0 || access & !ALL_ACCESS != 0 {
            return Err(invalid_value());
        }
        self.slots
            .retain(|slot| slot.upgrade().is_some_and(|slot| slot.0.borrow().is_some()));
        if self.slots.len() == MAX_OPEN_KEYS {
            return Err(limit_error());
        }
        self.slots.try_reserve(1).map_err(|_| limit_error())?;
        let (handle, created) = self.acquire_handle(hive, subkey, access, create)?;
        let slot = Rc::new(Slot(RefCell::new(Some(OpenKey { handle, access }))));
        self.slots.push(Rc::downgrade(&slot));
        Ok((slot, created))
    }

    fn acquire_handle(&self, hive: Hive, path: &str, access: u32, create: bool) -> Acquired {
        #[cfg(test)]
        if let Some(backend) = &self.backend {
            return backend(hive, path, access, create);
        }
        #[cfg(windows)]
        {
            super::platform::acquire_owned(hive, path, access, create)
        }
        #[cfg(not(windows))]
        {
            let _ = (hive, path, access, create);
            Err(super::unsupported())
        }
    }
}

/// Acquire a handle and return private callbacks to the pristine wrapper.
/// If response construction fails, close the slot immediately, even if
/// QuickJS has already allocated some of the callbacks.
pub(super) fn acquire_js<'js>(
    ctx: Ctx<'js>,
    path: String,
    access: u32,
    create: bool,
) -> JsResult<Object<'js>> {
    use super::WinRegistryState;

    let result = {
        let state = ctx
            .userdata::<Rc<RefCell<WinRegistryState>>>()
            .ok_or(rquickjs::Error::Unknown)?;
        state.borrow_mut().keys.acquire(&path, access, create)
    };
    match result {
        Ok((slot, created)) => finish_acquisition(&ctx, &slot, |object| {
            install_callbacks(&ctx, object, &slot)?;
            object.set("created", created)
        }),
        Err(error) => respond(&ctx, Err(error), |_, ()| Ok(())),
    }
}

fn finish_acquisition<'js>(
    ctx: &Ctx<'js>,
    slot: &Slot,
    build: impl FnOnce(&Object<'js>) -> JsResult<()>,
) -> JsResult<Object<'js>> {
    let response = respond(ctx, Ok(()), |object, ()| build(object));
    if response.is_err() {
        slot.close();
    }
    response
}

fn install_callbacks<'js>(ctx: &Ctx<'js>, object: &Object<'js>, slot: &Rc<Slot>) -> JsResult<()> {
    let close = Rc::clone(slot);
    let read = Rc::clone(slot);
    let write = Rc::clone(slot);
    object.set("close", Function::new(ctx.clone(), move || close.close())?)?;
    object.set(
        "getString",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, name: String| {
            let result = read.with_access(QUERY_VALUE, |handle| {
                validate_value_name(&name)?;
                let (kind, data) = handle.read(&name)?;
                if kind != TYPE_SZ && kind != TYPE_EXPAND_SZ {
                    return Err(type_mismatch());
                }
                decode_string(&data)
            });
            respond(&ctx, result, |object, value| object.set("value", value))
        })?,
    )?;
    object.set(
        "setString",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, name: String, value: String| {
                let result = write.with_access(SET_VALUE, |handle| {
                    validate_value_name(&name)?;
                    let bytes = encode_string(&value)?;
                    handle.write(&name, &bytes)
                });
                respond(&ctx, result, |_, ()| Ok(()))
            },
        )?,
    )
}

#[cfg(test)]
mod tests;
