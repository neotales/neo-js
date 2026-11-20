//! Owner-side promise table and bounded, Rust-only binary reads.

use std::{
    cell::RefCell, collections::BTreeMap, num::NonZeroUsize, path::PathBuf, rc::Rc, time::Duration,
};

use rquickjs::{
    Ctx, Function, Object, Persistent, Promise, TypedArray, Undefined, Value, object::Property,
};

use super::{
    Code, FsError, JsError, JsResult, JsString, MAX_PATH_BYTES, MAX_READ_BYTES, WorkingDirectory,
    checked_js_path, open_regular_source, read_bounded_until, resolve, validate_path, worker,
};

const MAX_PENDING: usize = 4;
const LIMITS: worker::Limits = worker::Limits {
    workers: NonZeroUsize::new(2).unwrap(),
    queued: NonZeroUsize::new(MAX_PENDING).unwrap(),
    pending: NonZeroUsize::new(MAX_PENDING).unwrap(),
    bytes: MAX_PENDING * (MAX_READ_BYTES + MAX_PATH_BYTES),
};

struct Read {
    path: PathBuf,
    #[cfg(test)]
    gate: Option<ReadGate>,
}

type ReadResult = Result<Vec<u8>, FsError>;
type Pool = worker::Pool<Read, ReadResult>;

/// Owned Rust data is all that reaches this thread.
fn execute(read: Read, cancellation: &worker::Cancellation) -> ReadResult {
    #[cfg(test)]
    if let Some(ReadGate(gate)) = read.gate {
        gate(cancellation)?;
    }
    if cancellation.requested() {
        return Err(FsError::new(Code::Cancelled));
    }
    let (file, metadata) = open_regular_source(&read.path)?;
    let length = usize::try_from(metadata.len()).map_err(|_| FsError::new(Code::Limit))?;
    read_bounded_until(file, length, MAX_READ_BYTES, || cancellation.requested())
}

struct Settlement {
    resolve: Persistent<Function<'static>>,
    reject: Persistent<Function<'static>>,
    make_error: Persistent<Function<'static>>,
}

impl Settlement {
    fn reject(self, ctx: &Ctx<'_>, code: Code) -> JsResult<()> {
        let reject = self.reject.restore(ctx)?;
        let make_error = self.make_error.restore(ctx)?;
        let reason = match make_error.call::<_, Value>((code.as_str(),)) {
            Ok(reason) => reason,
            // Even if QuickJS cannot build an FsError, reject with the engine
            // exception instead of dropping the only settlement handles.
            Err(JsError::Exception) => ctx.catch(),
            Err(error) => return Err(error),
        };
        reject.call((reason,))
    }

    fn finish(self, ctx: &Ctx<'_>, result: ReadResult) -> JsResult<()> {
        let bytes = match result {
            Ok(bytes) => bytes,
            Err(error) => return self.reject(ctx, error.code),
        };
        let resolve = self.resolve.clone().restore(ctx)?;
        let array = match TypedArray::<u8>::new(ctx.clone(), bytes) {
            Ok(array) => array,
            Err(JsError::Exception) => {
                return self.reject.restore(ctx)?.call((ctx.catch(),));
            }
            Err(error) => return Err(error),
        };
        // Resolving a promise normally looks up `then`. Do not allow a
        // script-modified prototype to intercept the fresh result; no promise
        // reaction runs until the next owner turn, after we remove the guard.
        let object = array.as_object();
        if let Err(error) = object.prop("then", Property::from(Undefined).configurable()) {
            return if matches!(error, JsError::Exception) {
                self.reject.restore(ctx)?.call((ctx.catch(),))
            } else {
                Err(error)
            };
        }
        resolve.call::<_, ()>((array.clone(),))?;
        object.remove("then")
    }
}

#[derive(Default)]
struct Active {
    pool: Option<Pool>,
    pending: BTreeMap<u64, Settlement>,
    next: u64,
}

/// This table is owner-only. No JS calls while it or the worker queue are
/// borrowed. An inactive driver cannot allocate a pool or dispatch a read.
#[derive(Clone, Default)]
pub(crate) struct ReadDriver {
    active: Rc<RefCell<Option<Active>>>,
    #[cfg(test)]
    gates: Rc<RefCell<std::collections::VecDeque<ReadGate>>>,
}

impl ReadDriver {
    /// Begin a managed module evaluation. The guard always stops its workers.
    pub(crate) fn begin(&self) -> ReadTurn<'_> {
        assert!(self.active.borrow().is_none());
        *self.active.borrow_mut() = Some(Active::default());
        ReadTurn(self)
    }

    fn submit<'js>(
        &self,
        ctx: &Ctx<'js>,
        path: PathBuf,
        make_error: Function<'js>,
    ) -> JsResult<Promise<'js>> {
        // Allocate JS handles before borrowing the table. No job is submitted
        // until the corresponding settlement is fully owned by the table.
        let (promise, resolve, reject) = Promise::new(ctx)?;
        let settlement = Settlement {
            resolve: Persistent::save(ctx, resolve),
            reject: Persistent::save(ctx, reject),
            make_error: Persistent::save(ctx, make_error),
        };
        let result = self.dispatch(path, settlement);
        if let Err((code, settlement)) = result {
            settlement.reject(ctx, code)?;
        }
        Ok(promise)
    }

    fn dispatch(&self, path: PathBuf, settlement: Settlement) -> Result<(), (Code, Settlement)> {
        let mut active = self.active.borrow_mut();
        let Some(active) = active.as_mut() else {
            return Err((Code::AsyncOutsideTurn, settlement));
        };
        let bytes = MAX_READ_BYTES + path.as_os_str().as_encoded_bytes().len();
        if active.pool.is_none() {
            match Pool::new(LIMITS, execute) {
                Ok(pool) => active.pool = Some(pool),
                Err(_) => return Err((Code::Limit, settlement)),
            }
        }
        let admission = match active.pool.as_mut().unwrap().reserve(bytes) {
            Ok(admission) => admission,
            Err(worker::AdmitError::Closed) => return Err((Code::Cancelled, settlement)),
            Err(worker::AdmitError::Full | worker::AdmitError::TooLarge) => {
                return Err((Code::Limit, settlement));
            }
        };
        let id = active.next;
        let Some(next) = id.checked_add(1) else {
            return Err((Code::Limit, settlement));
        };
        active.next = next;
        active.pending.insert(id, settlement);
        admission.send(
            id,
            Read {
                path,
                #[cfg(test)]
                gate: self.gates.borrow_mut().pop_front(),
            },
        );
        Ok(())
    }
}

pub(crate) struct ReadTurn<'a>(&'a ReadDriver);

pub(crate) struct ReadCompletion(Settlement, ReadResult);

impl ReadCompletion {
    /// Settle on the owner, with neither the pool nor its owner table locked.
    pub(crate) fn deliver(self, ctx: &Ctx<'_>) -> JsResult<()> {
        self.0.finish(ctx, self.1)
    }
}

pub(crate) struct CancelledReads(BTreeMap<u64, Settlement>);

impl CancelledReads {
    /// Reject abandoned reads without executing their reactions.
    pub(crate) fn reject(self, ctx: &Ctx<'_>) {
        for (_, settlement) in self.0 {
            if settlement.reject(ctx, Code::Cancelled).is_err() {
                ctx.catch();
            }
        }
    }
}

impl ReadTurn<'_> {
    /// Count accepted reads still awaiting delivery on the owner.
    pub(crate) fn has_pending(&self) -> bool {
        !self.0.active.borrow().as_ref().unwrap().pending.is_empty()
    }

    /// Receive one completion, releasing its budget before settling it.
    /// Pass zero to poll while QuickJS has runnable jobs.
    pub(crate) fn receive(&self, timeout: Option<Duration>) -> Option<ReadCompletion> {
        let mut active = self.0.active.borrow_mut();
        let active = active.as_mut().unwrap();
        let completion = active.pool.as_mut()?.receive(timeout)?;
        let result = match completion.outcome {
            worker::Outcome::Finished(result) => result,
            worker::Outcome::Panicked => Err(FsError::new(Code::Io)),
        };
        Some(ReadCompletion(
            active.pending.remove(&completion.id).unwrap(),
            result,
        ))
    }

    /// Stop dispatch and join workers without entering QuickJS.
    pub(crate) fn cancel(&self) -> CancelledReads {
        let Active { pool, pending, .. } = self.0.active.borrow_mut().take().unwrap();
        drop(pool);
        CancelledReads(pending)
    }
}

impl Drop for ReadTurn<'_> {
    fn drop(&mut self) {
        let active = self.0.active.borrow_mut().take();
        drop(active);
    }
}

pub(super) fn install<'js>(
    ctx: &Ctx<'js>,
    native: &Object<'js>,
    driver: ReadDriver,
    cwd: Rc<RefCell<WorkingDirectory>>,
) -> JsResult<()> {
    let check = driver.clone();
    native.set(
        "canReadAsync",
        Function::new(ctx.clone(), move || check.active.borrow().is_some())?,
    )?;
    native.set(
        "readFile",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, path: JsString<'js>, make_error: Function<'js>| {
                let path = checked_js_path(path, MAX_PATH_BYTES)
                    .and_then(|path| resolve(&cwd, &path))
                    .and_then(|path| {
                        if !path.is_absolute() {
                            return Err(FsError::new(Code::Path));
                        }
                        validate_path(path.to_str().ok_or(FsError::new(Code::Encoding))?)?;
                        Ok(path)
                    });
                match path {
                    Ok(path) => driver.submit(&ctx, path, make_error),
                    Err(error) => {
                        let reason: Value = make_error.call((error.code(),))?;
                        Err(ctx.throw(reason))
                    }
                }
            },
        )?,
    )
}

#[cfg(test)]
struct ReadGate(Box<GateFn>);

#[cfg(test)]
type GateFn = dyn FnOnce(&worker::Cancellation) -> Result<(), FsError> + Send;

#[cfg(test)]
mod tests;
