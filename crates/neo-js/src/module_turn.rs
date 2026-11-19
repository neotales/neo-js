//! One blocking module facade over the owner-thread QuickJS queue and FS worker.

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::Rc,
    time::Duration,
};

use rquickjs::{
    CaughtError, Constructor, Context, Ctx, Error, Function, Object, Persistent, Promise, Runtime,
    Value,
    context::intrinsic::WeakRef,
    function::This,
    promise::{PromiseHookType, PromiseState},
    qjs,
};

use crate::fs::ReadTurn;

#[cfg(test)]
mod tests;

const MAX_TURN_STEPS: usize = 100_000;
const MAX_TRACKED_REJECTIONS: usize = 128;
const MAX_TRACKED_PENDING: usize = 4096;

/// The tracker runs on the owner. It retains neither Promises nor the script
/// objects reachable from their results or pending reactions.
pub(crate) struct ModuleTurns {
    rejections: Rc<RefCell<Option<Rejections>>>,
    pending: Rc<RefCell<Pending>>,
    weak: WeakPromises,
    abandoned: Cell<bool>,
    #[cfg(test)]
    quiescence_root: Rc<RefCell<Option<Persistent<Value<'static>>>>>,
}

#[derive(Default)]
struct Rejections {
    promises: HashSet<usize>,
    collected: usize,
    overflow: bool,
}

impl Rejections {
    fn on_init(&mut self, id: usize) {
        // Init runs before any settlement/handled event on a new native
        // Promise. If its address was reused, the previous rejection can
        // no longer be handled and still counts.
        if self.promises.remove(&id) {
            self.collected += 1;
        }
    }

    fn update(&mut self, id: usize, handled: bool) {
        if handled {
            self.promises.remove(&id);
        } else if self.promises.len() + self.collected < MAX_TRACKED_REJECTIONS {
            self.promises.insert(id);
        } else {
            self.overflow = true;
        }
    }

    fn has_unhandled(&self) -> bool {
        self.collected != 0 || !self.promises.is_empty()
    }
}

#[derive(Default)]
struct Pending {
    promises: HashMap<usize, Persistent<Object<'static>>>,
    overflow: bool,
}

fn promise_id(promise: &Value<'_>) -> usize {
    // SAFETY: All callers receive a native Promise object from a QuickJS
    // hook. The address is only an identity key; it is never dereferenced.
    unsafe { qjs::JS_VALUE_GET_PTR(promise.as_raw()) as usize }
}

#[derive(Clone)]
struct WeakPromises {
    constructor: Persistent<Constructor<'static>>,
    deref: Persistent<Function<'static>>,
}

impl WeakPromises {
    fn capture(ctx: &Ctx<'_>) -> rquickjs::Result<Self> {
        // Use a private realm so neither the intrinsics nor the prototypes
        // of the weak handles can be modified by script. The QuickJS native
        // constructor and deref do not read properties of the target.
        let constructor: Constructor = ctx.globals().get("WeakRef")?;
        let prototype: Object = constructor.get("prototype")?;
        Ok(Self {
            constructor: Persistent::save(ctx, constructor),
            deref: Persistent::save(ctx, prototype.get::<_, Function>("deref")?),
        })
    }

    fn downgrade<'js>(&self, ctx: &Ctx<'js>, promise: Value<'js>) -> rquickjs::Result<Object<'js>> {
        self.constructor.clone().restore(ctx)?.construct((promise,))
    }

    fn is_pending<'js>(&self, ctx: &Ctx<'js>, weak: Object<'js>) -> rquickjs::Result<bool> {
        let target: Value = self.deref.clone().restore(ctx)?.call((This(weak),))?;
        // QuickJS's native deref just duplicates a live target for this call
        // (it does not keep it alive until the end of the job).
        Ok(target
            .into_promise()
            .is_some_and(|promise| promise.state() == PromiseState::Pending))
    }
}

// A void engine hook cannot propagate allocation/stack errors into its
// caller. Fail closed via the tracking budget instead, without replacing any
// exception the interrupted engine operation was already carrying.
fn tracking_call<T>(ctx: &Ctx<'_>, call: impl FnOnce() -> rquickjs::Result<T>) -> Option<T> {
    let saved = ctx.has_exception().then(|| ctx.catch());
    let result = call();
    if result.is_err() && ctx.has_exception() {
        drop(ctx.catch());
    }
    if let Some(saved) = saved {
        ctx.throw(saved);
    }
    result.ok()
}

impl Pending {
    fn prune(&mut self, ctx: &Ctx<'_>, weak: &WeakPromises) {
        // Resolve hooks do not cover handled rejections. Scan weak targets
        // at quiescence and at saturation; settled/dead slots carry no JS
        // payload in the meantime.
        self.promises.retain(|_, target| {
            match tracking_call(ctx, || weak.is_pending(ctx, target.clone().restore(ctx)?)) {
                Some(pending) => pending,
                None => {
                    self.overflow = true;
                    true
                }
            }
        });
    }
}

fn prune_pending(table: &RefCell<Pending>, ctx: &Ctx<'_>, weak: &WeakPromises) -> (bool, bool) {
    // No tracker borrow spans a QuickJS native call. These private
    // intrinsics cannot run script or call the Promise hooks.
    let mut pending = table.take();
    pending.prune(ctx, weak);
    let state = (!pending.promises.is_empty(), pending.overflow);
    table.replace(pending);
    state
}

fn collect_and_prune(table: &RefCell<Pending>, ctx: &Ctx<'_>, weak: &WeakPromises) -> (bool, bool) {
    // GC may enqueue finalization jobs but does not run them. If enqueueing
    // exhausts memory, tracking cannot safely assert quiescence.
    let collected = tracking_call(ctx, || {
        ctx.run_gc();
        if ctx.has_exception() {
            Err(Error::Exception)
        } else {
            Ok(())
        }
    });
    if collected.is_none() {
        table.borrow_mut().overflow = true;
    }
    prune_pending(table, ctx, weak)
}

impl ModuleTurns {
    pub(crate) fn new(runtime: &Runtime) -> rquickjs::Result<Self> {
        let private = Context::custom::<WeakRef>(runtime)?;
        Ok(Self {
            pending: Rc::default(),
            rejections: Rc::default(),
            weak: private.with(|ctx| WeakPromises::capture(&ctx))?,
            abandoned: Cell::default(),
            #[cfg(test)]
            quiescence_root: Rc::default(),
        })
    }

    pub(crate) fn install(&self, runtime: &Runtime) {
        let pending = Rc::clone(&self.pending);
        let rejections = Rc::clone(&self.rejections);
        let weak = self.weak.clone();
        runtime.set_promise_hook(Some(Box::new(move |ctx, event, promise, _| match event {
            PromiseHookType::Init => {
                let id = promise_id(&promise);
                if let Some(state) = rejections.borrow_mut().as_mut() {
                    state.on_init(id);
                }
                {
                    let mut table = pending.borrow_mut();
                    if table.overflow {
                        return;
                    }
                    table.promises.remove(&id);
                }
                if pending.borrow().promises.len() == MAX_TRACKED_PENDING {
                    let (_, overflow) = prune_pending(&pending, &ctx, &weak);
                    if !overflow && pending.borrow().promises.len() == MAX_TRACKED_PENDING {
                        // Refcounting alone cannot clear an unreachable
                        // cycle. Do one collection before declaring a sticky
                        // overflow.
                        collect_and_prune(&pending, &ctx, &weak);
                    }
                }
                let full = {
                    let table = pending.borrow();
                    table.overflow || table.promises.len() == MAX_TRACKED_PENDING
                };
                if full {
                    pending.borrow_mut().overflow = true;
                } else if let Some(target) = tracking_call(&ctx, || weak.downgrade(&ctx, promise)) {
                    let target = Persistent::save(&ctx, target);
                    pending.borrow_mut().promises.insert(id, target);
                } else {
                    pending.borrow_mut().overflow = true;
                }
            }
            PromiseHookType::Resolve => {
                pending.borrow_mut().promises.remove(&promise_id(&promise));
            }
            _ => {}
        })));
        let rejections = Rc::clone(&self.rejections);
        runtime.set_host_promise_rejection_tracker(Some(Box::new(
            move |_, promise, _, handled| {
                let mut rejections = rejections.borrow_mut();
                let Some(state) = rejections.as_mut() else {
                    return;
                };
                state.update(promise_id(&promise), handled);
            },
        )));
    }

    /// Remove all host roots before the owning QuickJS runtime is dropped.
    pub(crate) fn clear(&self, runtime: &Runtime) {
        runtime.set_promise_hook(None);
        runtime.set_host_promise_rejection_tracker(None);
        self.pending.borrow_mut().promises.clear();
        #[cfg(test)]
        drop(self.quiescence_root.borrow_mut().take());
    }

    fn pending_state(&self, context: &Context) -> (bool, bool) {
        context.with(|ctx| {
            // Force a last strong handle to be released at exactly this
            // checkpoint for the queue-recheck regression test.
            #[cfg(test)]
            drop(self.quiescence_root.borrow_mut().take());
            let (pending, overflow) = prune_pending(&self.pending, &ctx, &self.weak);
            if pending && !overflow {
                // WeakRef.deref still sees unreachable cycles until a cycle
                // collection. Confirm once before calling work live. The
                // caller must check for finalization jobs after this pass.
                return collect_and_prune(&self.pending, &ctx, &self.weak);
            }
            (pending, overflow)
        })
    }

    /// A managed module never executes pending work from `run`, `eval`, or
    /// the out-of-turn test runner. Don't admit reads or declare an entry if
    /// those calls left work that could start later.
    pub(crate) fn require_idle(&self, context: &Context, runtime: &Runtime) -> Result<(), String> {
        self.ensure_usable()?;
        let (pending, overflow) = self.pending_state(context);
        if pending || overflow || runtime.is_job_pending() {
            self.abandoned.set(true);
            return Err("NEO_MODULE_DIRTY: earlier script or test execution left pending Promises or jobs; construct a new runtime".into());
        }
        Ok(())
    }

    pub(crate) fn ensure_usable(&self) -> Result<(), String> {
        if self.abandoned.get() {
            Err("NEO_MODULE_ABORTED: construct a new runtime after an abandoned module turn".into())
        } else {
            Ok(())
        }
    }

    /// Track newly rejected promises from the start of module evaluation.
    pub(crate) fn begin(&self) -> ModuleTurn<'_> {
        assert!(self.rejections.borrow().is_none());
        *self.rejections.borrow_mut() = Some(Rejections::default());
        ModuleTurn {
            turns: self,
            finished: false,
        }
    }
}

pub(crate) struct ModuleTurn<'a> {
    turns: &'a ModuleTurns,
    finished: bool,
}

/// Mark the entry Promise as observed even if evaluation rejected it before
/// returning to Rust. Entry failures are reported separately from detached jobs.
pub(crate) fn observe_entry(promise: &Promise<'_>) {
    let value = promise.as_value();
    // SAFETY: This is a native Promise and both handles belong to its context.
    unsafe { qjs::JS_PromiseMarkAsHandled(value.ctx().as_raw().as_ptr(), value.as_raw()) };
}

pub(crate) fn js_error(ctx: &Ctx<'_>, error: Error) -> String {
    CaughtError::from_error(ctx, error).to_string()
}

impl ModuleTurn<'_> {
    /// Alternate completions and JS jobs. Only sleep when QuickJS is idle
    /// and a worker can make progress; there is no owner-side polling loop.
    pub(crate) fn drive(
        &self,
        context: &Context,
        runtime: &Runtime,
        reads: &ReadTurn<'_>,
        root: Persistent<Promise<'static>>,
    ) -> Result<(), String> {
        let mut root = Some(root);
        for _ in 0..MAX_TURN_STEPS {
            if let Some(root_handle) = &root {
                let fulfilled = context.with(|ctx| {
                    let promise = root_handle
                        .clone()
                        .restore(&ctx)
                        .map_err(|e| js_error(&ctx, e))?;
                    match promise.result::<()>() {
                        Some(result) => result.map(|()| true).map_err(|e| js_error(&ctx, e)),
                        None => Ok(false),
                    }
                })?;
                if fulfilled {
                    // Drop the entry handle before the final queue check.
                    root = None;
                }
            }
            let completed = reads.receive(Some(Duration::ZERO));
            let delivered = completed.is_some();
            if let Some(completed) = completed {
                context.with(|ctx| completed.deliver(&ctx).map_err(|e| js_error(&ctx, e)))?;
            }
            let executed = runtime
                .execute_pending_job()
                .map_err(|job| job.0.with(|ctx| js_error(&ctx, Error::Exception)))?;
            if executed || delivered {
                continue;
            }
            if reads.has_pending() {
                if let Some(completed) = reads.receive(None) {
                    context.with(|ctx| completed.deliver(&ctx).map_err(|e| js_error(&ctx, e)))?;
                }
                continue;
            }
            let (pending, overflow) = self.turns.pending_state(context);
            // The checkpoint may release handles or let QuickJS collect
            // objects. Finalization jobs can resolve an apparently stalled
            // Promise, so discard this snapshot and drive those jobs first.
            if runtime.is_job_pending() {
                continue;
            }
            if overflow {
                self.turns.abandoned.set(true);
                return Err(
                    "NEO_MODULE_LIMIT: module turn exceeded its pending Promise tracking budget"
                        .into(),
                );
            }
            if root.is_some() || pending {
                self.turns.abandoned.set(true);
                return Err("NEO_MODULE_STALLED: unresolved Promise at module quiescence with no runnable jobs or filesystem work".into());
            }
            let rejections = self.turns.rejections.borrow();
            let rejections = rejections.as_ref().unwrap();
            if rejections.overflow {
                return Err(
                    "NEO_MODULE_LIMIT: module turn exceeded its rejection tracking budget".into(),
                );
            }
            if rejections.has_unhandled() {
                return Err(
                    "NEO_UNHANDLED_REJECTION: module turn has an unhandled Promise rejection"
                        .into(),
                );
            }
            return Ok(());
        }
        self.turns.abandoned.set(true);
        Err("NEO_MODULE_LIMIT: module turn exceeded its job and completion budget".into())
    }

    /// Pending reads and runnable jobs cannot escape a failed turn. Even a
    /// job that has not dispatched a read yet could do so if run next time.
    pub(crate) fn finish(
        mut self,
        context: &Context,
        runtime: &Runtime,
        reads: &ReadTurn<'_>,
        failed: bool,
    ) {
        if failed {
            let (pending, overflow) = self.turns.pending_state(context);
            if reads.has_pending() || runtime.is_job_pending() || pending || overflow {
                self.turns.abandoned.set(true);
                let cancelled = reads.cancel();
                context.with(|ctx| cancelled.reject(&ctx));
            }
        }
        self.finished = true;
    }
}

impl Drop for ModuleTurn<'_> {
    fn drop(&mut self) {
        if !self.finished {
            // Rust unwinding is not a JS exception. Do not call QuickJS,
            // execute jobs or settle promises; ReadTurn drops and joins.
            self.turns.abandoned.set(true);
        }
        self.turns.rejections.borrow_mut().take();
    }
}
