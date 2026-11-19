//! Private worker foundation. The runtime owner alone admits requests and
//! consumes completions; workers receive only owned Rust data, never a JS
//! context, JS values, or promise handles.
//!
//! This is for controlled filesystem operations and passive Rust messages,
//! not arbitrary user-supplied Rust code. Isolated unwinds (including from a
//! destructor) are contained, but the operation and destructors must not
//! panic during their own unwind or generate an endless chain of panic
//! payloads. An abort, including a double panic inside a callback's unwind,
//! cannot be caught here.

use std::{
    any::Any,
    collections::VecDeque,
    io,
    num::NonZeroUsize,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

/// Per-pool limits. Pending counts queued, running and undelivered jobs.
struct Limits {
    workers: NonZeroUsize,
    queued: NonZeroUsize,
    pending: NonZeroUsize,
    bytes: usize,
}

#[derive(Debug, PartialEq, Eq)]
enum AdmitError {
    Closed,
    TooLarge,
    Full,
}

#[derive(Debug, PartialEq, Eq)]
enum Outcome<Output> {
    Finished(Output),
    Panicked,
}

#[derive(Debug, PartialEq, Eq)]
struct Completion<Output> {
    id: u64,
    outcome: Outcome<Output>,
}

/// Shared shutdown request. Work that can stop early checks between bounded
/// steps; a blocked OS call cannot be interrupted by this flag.
#[derive(Default)]
struct Cancellation(AtomicBool);

impl Cancellation {
    fn requested(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

struct Request<Input> {
    id: u64,
    bytes: usize,
    input: Input,
}

struct State<Input, Output> {
    queued: VecDeque<Request<Input>>,
    ready: VecDeque<(usize, Completion<Output>)>,
    pending: usize,
    bytes: usize,
}

impl<Input, Output> Default for State<Input, Output> {
    fn default() -> Self {
        Self {
            queued: VecDeque::new(),
            ready: VecDeque::new(),
            pending: 0,
            bytes: 0,
        }
    }
}

struct Shared<Input, Output> {
    state: Mutex<State<Input, Output>>,
    work: Condvar,
    completion: Condvar,
    cancelled: Cancellation,
}

/// An owner-driven pool with no detached threads, including on spawn failure.
struct Pool<Input, Output> {
    shared: Arc<Shared<Input, Output>>,
    limits: Limits,
    workers: Vec<JoinHandle<()>>,
}

/// Drop each panic payload under its own catch. A `panic_any` payload can
/// itself panic on drop; consume the replacement rather than leaking either
/// payload or letting the destructor unwind through the caller.
fn discard_panic(mut payload: Box<dyn Any + Send>) {
    while let Err(next) = catch_unwind(AssertUnwindSafe(|| drop(payload))) {
        payload = next;
    }
}

fn discard<T>(message: T) {
    if let Err(payload) = catch_unwind(AssertUnwindSafe(|| drop(message))) {
        discard_panic(payload);
    }
}

/// Own joining independently of the code that disposes of messages.
struct JoinWorkers<'a>(&'a mut Vec<JoinHandle<()>>);

impl Drop for JoinWorkers<'_> {
    fn drop(&mut self) {
        for worker in self.0.drain(..) {
            if let Err(payload) = worker.join() {
                discard_panic(payload);
            }
        }
    }
}

/// Exclusive permission to take a snapshot and enqueue one job. Dropping it
/// without sending changes nothing.
struct Admission<'a, Input, Output> {
    pool: &'a mut Pool<Input, Output>,
    bytes: usize,
}

impl<Input, Output> Admission<'_, Input, Output> {
    /// Send a fully owned snapshot with an owner-supplied correlation ID.
    /// The owner must not reuse an ID until the previous completion is taken.
    fn send(self, id: u64, input: Input) {
        let shared = &self.pool.shared;
        let mut state = shared.state.lock().unwrap();
        state.queued.push_back(Request {
            id,
            bytes: self.bytes,
            input,
        });
        state.pending += 1;
        state.bytes += self.bytes;
        shared.work.notify_one();
    }
}

impl<Input: Send + 'static, Output: Send + 'static> Pool<Input, Output> {
    /// Start the fixed worker set. All workers run the same Rust-only
    /// operation, outside the queue lock. Unwinding from an operation reports
    /// `Panicked` and does not retire a worker.
    fn new(limits: Limits, execute: fn(Input, &Cancellation) -> Output) -> io::Result<Self> {
        let mut pool = Self {
            shared: Arc::new(Shared {
                state: Mutex::default(),
                work: Condvar::new(),
                completion: Condvar::new(),
                cancelled: Cancellation::default(),
            }),
            workers: Vec::with_capacity(limits.workers.get()),
            limits,
        };
        for _ in 0..pool.limits.workers.get() {
            let shared = Arc::clone(&pool.shared);
            let handle = thread::Builder::new()
                .name("neo-fs".into())
                .spawn(move || work(&shared, execute))?;
            pool.workers.push(handle);
        }
        Ok(pool)
    }
}

impl<Input, Output> Pool<Input, Output> {
    /// Check admission before allocating or copying an owned snapshot.
    /// `bytes` is the caller's upper bound for the job's owned payload,
    /// including any buffered result; the operation must enforce that bound.
    fn reserve(&mut self, bytes: usize) -> Result<Admission<'_, Input, Output>, AdmitError> {
        let state = self.shared.state.lock().unwrap();
        if self.shared.cancelled.requested() {
            return Err(AdmitError::Closed);
        }
        if bytes > self.limits.bytes {
            return Err(AdmitError::TooLarge);
        }
        if state.queued.len() == self.limits.queued.get()
            || state.pending == self.limits.pending.get()
            || bytes > self.limits.bytes - state.bytes
        {
            return Err(AdmitError::Full);
        }
        // Only this owner can enqueue; workers can decrease queue occupancy
        // but never increase it or change either charge.
        drop(state);
        Ok(Admission { pool: self, bytes })
    }

    /// Take the next ready completion and free its admission charge. Return
    /// immediately if idle, or wait for at most `timeout` if work is pending.
    /// Pass zero to poll. Delivery is in completion, not submission, order.
    fn receive(&mut self, timeout: Duration) -> Option<Completion<Output>> {
        let shared = &self.shared;
        let state = shared.state.lock().unwrap();
        let (mut state, _) = shared
            .completion
            .wait_timeout_while(state, timeout, |state| {
                state.ready.is_empty() && state.pending != 0
            })
            .unwrap();
        let (bytes, completion) = state.ready.pop_front()?;
        state.pending -= 1;
        state.bytes -= bytes;
        Some(completion)
    }

    /// Stop admitting jobs, discard queued and undelivered messages, signal
    /// any running operation, and join every worker. This also runs on drop.
    /// Isolated destructor unwinds are contained so cleanup can continue.
    /// If an OS call is blocked, shutdown waits for it to return; it never
    /// detaches an operation or rolls back side effects.
    fn shutdown(&mut self) {
        let shared = &self.shared;
        let (queued, ready) = {
            let mut state = shared.state.lock().unwrap();
            shared.cancelled.0.store(true, Ordering::Relaxed);
            state.pending = 0;
            state.bytes = 0;
            shared.work.notify_all();
            (
                std::mem::take(&mut state.queued),
                std::mem::take(&mut state.ready),
            )
        };
        let _join = JoinWorkers(&mut self.workers);
        // Dispose of each message separately, outside the lock. A single
        // catch around a whole queue would drop the rest during unwinding.
        for request in queued {
            discard(request);
        }
        for completion in ready {
            discard(completion);
        }
    }
}

impl<Input, Output> Drop for Pool<Input, Output> {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn work<Input, Output>(
    shared: &Shared<Input, Output>,
    execute: fn(Input, &Cancellation) -> Output,
) {
    loop {
        let Request { id, bytes, input } = {
            let state = shared.state.lock().unwrap();
            let mut state = shared
                .work
                .wait_while(state, |state| {
                    state.queued.is_empty() && !shared.cancelled.requested()
                })
                .unwrap();
            if shared.cancelled.requested() {
                return;
            }
            state.queued.pop_front().unwrap()
        };

        let outcome = match catch_unwind(AssertUnwindSafe(|| execute(input, &shared.cancelled))) {
            Ok(output) => Outcome::Finished(output),
            Err(payload) => {
                discard_panic(payload);
                Outcome::Panicked
            }
        };
        let mut state = shared.state.lock().unwrap();
        if shared.cancelled.requested() {
            drop(state);
            discard(outcome);
            return;
        }
        state.ready.push_back((bytes, Completion { id, outcome }));
        shared.completion.notify_one();
    }
}

#[cfg(test)]
mod tests;
