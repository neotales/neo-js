use std::{
    cell::RefCell,
    collections::HashMap,
    io::{self, Read, Write},
    panic::{AssertUnwindSafe, catch_unwind},
    process::{Child, ChildStdin, Command, ExitStatus, Stdio},
    rc::Rc,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

// Per stream; an extra byte detects overflow without unbounded buffering.
const OUTPUT_LIMIT: u64 = 8 * 1024 * 1024;
const SLOT_LIMIT: usize = 8;
static SLOTS: AtomicUsize = AtomicUsize::new(0);

struct Slot {
    counter: &'static AtomicUsize,
    safe_to_release: bool,
}
impl Slot {
    fn reserve(counter: &'static AtomicUsize, limit: usize) -> Result<Self, ExecError> {
        reserve_counter(counter, limit)?;
        Ok(Self {
            counter,
            safe_to_release: true,
        })
    }
}

fn reserve_counter(counter: &'static AtomicUsize, limit: usize) -> Result<(), ExecError> {
    let mut current = counter.load(Ordering::Acquire);
    loop {
        if current >= limit {
            return Err(ExecError::new(
                "EXEC_WORKER",
                "process capacity exhausted; no child spawned",
            ));
        }
        match counter.compare_exchange_weak(
            current,
            current + 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return Ok(()),
            Err(value) => current = value,
        }
    }
}
impl Drop for Slot {
    fn drop(&mut self) {
        if self.safe_to_release {
            self.counter.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

#[derive(Clone, Debug)]
struct ExecError {
    code: &'static str,
    message: String,
}

impl ExecError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

enum Event {
    Stdin(Result<(), ExecError>),
    Stdout(Result<Vec<u8>, ExecError>),
    Stderr(Result<Vec<u8>, ExecError>),
}

/// Synchronous result; on fatal errors the bounded supervisor continues cleanup.
///
/// `directory` is the runtime-local working directory. It is applied with
/// [`Command::current_dir`] only to the child; the host process directory is
/// never changed. A relative program path is therefore resolved against it,
/// while a bare program name still uses the host's program search path. If the
/// runtime directory is unavailable, the call fails with `EXEC_CWD` before a
/// slot is reserved or a child is spawned, so a child never inherits the
/// mutable host directory.
pub(crate) fn exec(
    program: &str,
    args: &[String],
    input: Vec<u8>,
    directory: Option<&std::path::Path>,
) -> String {
    let Some(directory) = directory else {
        return serde_json::json!({
            "error": {
                "code": "EXEC_CWD",
                "message": "the runtime working directory is unavailable",
            }
        })
        .to_string();
    };
    let result = run(program, args, input, directory.to_path_buf());
    match result {
        Ok(value) => serde_json::json!({"result": value}).to_string(),
        Err(error) => {
            serde_json::json!({"error": {"code": error.code, "message": error.message}}).to_string()
        }
    }
}

type Reply = Sender<Result<serde_json::Value, ExecError>>;
fn run(
    program: &str,
    args: &[String],
    input: Vec<u8>,
    directory: std::path::PathBuf,
) -> Result<serde_json::Value, ExecError> {
    check_program(program).map_err(|e| ExecError::new("EXEC_UNSUPPORTED", e))?;
    let slot = Slot::reserve(&SLOTS, SLOT_LIMIT)?;
    let program = program.to_owned();
    let args = args.to_owned();
    let (tx, rx) = mpsc::channel();
    start_supervisor(
        slot,
        tx,
        Box::new(move |slot, reply| supervise(&program, &args, input, directory, slot, reply)),
        |task| {
            thread::Builder::new()
                .name("neo-js-supervisor".into())
                .spawn(task)
        },
    )?;
    rx.recv().unwrap_or_else(|_| {
        Err(ExecError::new(
            "EXEC_WORKER",
            "supervisor stopped unexpectedly; cleanup state unknown",
        ))
    })
}

type SupervisorTask = Box<dyn FnOnce(Slot, Reply) + Send>;
fn start_supervisor(
    slot: Slot,
    tx: Reply,
    task: SupervisorTask,
    spawn: impl FnOnce(Box<dyn FnOnce() + Send>) -> io::Result<JoinHandle<()>>,
) -> Result<(), ExecError> {
    spawn(Box::new(move || task(slot, tx)))
        .map(|_| ())
        .map_err(|e| {
            ExecError::new(
                "EXEC_WORKER",
                format!("cannot start supervisor; no child spawned: {e}"),
            )
        })
}

fn notify_failure(reply: &mut Option<Reply>, mut error: ExecError) {
    error
        .message
        .push_str("; cleanup pending or unknown; child or descendants may survive");
    if let Some(reply) = reply.take() {
        let _ = reply.send(Err(error));
    }
}

fn supervise(
    program: &str,
    args: &[String],
    input: Vec<u8>,
    directory: std::path::PathBuf,
    mut slot: Slot,
    tx: Reply,
) {
    let mut reply = Some(tx);
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = command.spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(e) => {
            drop(slot);
            let _ = reply.take().unwrap().send(Err(ExecError::new(
                "EXEC_SPAWN",
                format!("cannot start child: {e}"),
            )));
            return;
        }
    };
    // An unexpected panic or failed reap retains the reservation permanently:
    // we cannot prove the child and its resources are gone.
    slot.safe_to_release = false;
    let pipes = (child.stdin.take(), child.stdout.take(), child.stderr.take());
    let (Some(stdin), Some(stdout), Some(stderr)) = pipes else {
        notify_failure(
            &mut reply,
            ExecError::new("EXEC_IO", "child pipes unavailable"),
        );
        slot.safe_to_release = kill_and_reap(&mut child);
        return;
    };
    let (tx, rx) = mpsc::channel();
    let mut workers = Vec::new();
    let starts: [Box<dyn FnOnce() -> Event + Send>; 3] = [
        Box::new(move || Event::Stdin(write_input(stdin, &input))),
        Box::new(move || Event::Stdout(read_output(stdout, "stdout"))),
        Box::new(move || Event::Stderr(read_output(stderr, "stderr"))),
    ];
    for task in starts {
        match start_worker(tx.clone(), task) {
            Ok(worker) => workers.push(worker),
            Err(e) => {
                // Already queued failures precede this startup error.
                let mut primary = None;
                for event in rx.try_iter() {
                    if let Some(e) = event_error(event) {
                        primary = Some(e);
                        break;
                    }
                }
                notify_failure(
                    &mut reply,
                    primary.unwrap_or_else(|| {
                        ExecError::new("EXEC_WORKER", format!("cannot start child worker: {e}"))
                    }),
                );
                let reaped = kill_and_reap(&mut child);
                let mut joined = true;
                for worker in workers {
                    joined &= worker.join().is_ok();
                }
                slot.safe_to_release = reaped && joined;
                return;
            }
        }
    }
    drop(tx);
    let (result, safe) = coordinate(&mut child, rx, workers, &mut reply);
    slot.safe_to_release = safe;
    drop(slot);
    if let Some(reply) = reply {
        let _ = reply.send(result);
    }
}

fn event_error(event: Event) -> Option<ExecError> {
    match event {
        Event::Stdin(r) => r.err(),
        Event::Stdout(r) | Event::Stderr(r) => r.err(),
    }
}

fn start_worker(
    tx: Sender<Event>,
    task: Box<dyn FnOnce() -> Event + Send>,
) -> io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("neo-js-exec".into())
        .spawn(move || {
            let event = std::panic::catch_unwind(std::panic::AssertUnwindSafe(task))
                .unwrap_or_else(|_| {
                    Event::Stdin(Err(ExecError::new("EXEC_WORKER", "child worker panicked")))
                });
            let _ = tx.send(event);
        })
}

trait ChildControl {
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>>;
    fn kill(&mut self) -> io::Result<()>;
    fn wait(&mut self) -> io::Result<ExitStatus>;
}
impl ChildControl for Child {
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        Child::try_wait(self)
    }
    fn kill(&mut self) -> io::Result<()> {
        Child::kill(self)
    }
    fn wait(&mut self) -> io::Result<ExitStatus> {
        Child::wait(self)
    }
}

fn coordinate(
    child: &mut impl ChildControl,
    rx: Receiver<Event>,
    workers: Vec<JoinHandle<()>>,
    reply: &mut Option<Reply>,
) -> (Result<serde_json::Value, ExecError>, bool) {
    let mut status = None;
    let mut stdout = None;
    let mut stderr = None;
    let mut completed = 0;
    let mut failure = None;
    let mut pending_wait_error = None;
    loop {
        // Queued events are observed before polling the child and once more
        // after a wait failure, so already available worker failures win.
        while let Ok(event) = rx.try_recv() {
            completed += 1;
            match event {
                Event::Stdin(Err(e)) | Event::Stdout(Err(e)) | Event::Stderr(Err(e)) => {
                    failure = Some(e);
                    break;
                }
                Event::Stdin(Ok(())) => {}
                Event::Stdout(Ok(bytes)) => stdout = Some(bytes),
                Event::Stderr(Ok(bytes)) => stderr = Some(bytes),
            }
        }
        if failure.is_none() {
            failure = pending_wait_error.take();
        }
        if let Some(error) = failure.take() {
            notify_failure(reply, error);
            let reaped = status.is_some() || kill_and_reap(child);
            let mut joined = true;
            for worker in workers {
                joined &= worker.join().is_ok();
            }
            return (
                Err(ExecError::new("EXEC_WORKER", "failure already reported")),
                reaped && joined,
            );
        }
        if completed == workers.len() && status.is_some() {
            break;
        }
        if status.is_none() {
            match child.try_wait() {
                Ok(value) => status = value,
                Err(e) => {
                    pending_wait_error = Some(ExecError::new(
                        "EXEC_WAIT",
                        format!("cannot wait for child: {e}"),
                    ));
                    continue;
                }
            }
        }
        if completed < workers.len() {
            // Preserve the event in an explicit pending value by processing it
            // here; events already in the channel are FIFO.
            match rx.recv_timeout(Duration::from_millis(10)) {
                Ok(event) => {
                    completed += 1;
                    match event {
                        Event::Stdin(Err(e)) | Event::Stdout(Err(e)) | Event::Stderr(Err(e)) => {
                            failure = Some(e)
                        }
                        Event::Stdin(Ok(())) => {}
                        Event::Stdout(Ok(bytes)) => stdout = Some(bytes),
                        Event::Stderr(Ok(bytes)) => stderr = Some(bytes),
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    failure = Some(ExecError::new(
                        "EXEC_WORKER",
                        "child worker channel disconnected",
                    ))
                }
            }
            // A fatal event already observed must not be replaced by queued ones.
            if let Some(error) = failure.take() {
                notify_failure(reply, error);
                let reaped = status.is_some() || kill_and_reap(child);
                let mut joined = true;
                for worker in workers {
                    joined &= worker.join().is_ok();
                }
                return (
                    Err(ExecError::new("EXEC_WORKER", "failure already reported")),
                    reaped && joined,
                );
            }
        } else {
            thread::sleep(Duration::from_millis(10));
        }
    }
    let mut joined = true;
    for worker in workers {
        joined &= worker.join().is_ok();
    }
    if !joined {
        return (
            Err(ExecError::new(
                "EXEC_WORKER",
                "child worker join failed; cleanup state unknown",
            )),
            false,
        );
    }
    let status = status.expect("completed child has a status");
    (
        Ok(
            serde_json::json!({"code": status.code(), "success": status.success(), "stdout": stdout.expect("stdout worker completed"), "stderr": stderr.expect("stderr worker completed")}),
        ),
        true,
    )
}

fn write_input(mut writer: impl Write, input: &[u8]) -> Result<(), ExecError> {
    match writer.write_all(input) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(e) => Err(ExecError::new(
            "EXEC_IO",
            format!("cannot write child stdin: {e}"),
        )),
    }
}
fn read_output(reader: impl Read, stream: &str) -> Result<Vec<u8>, ExecError> {
    let mut bytes = Vec::new();
    reader
        .take(OUTPUT_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| ExecError::new("EXEC_IO", format!("cannot read child {stream}: {e}")))?;
    if bytes.len() as u64 > OUTPUT_LIMIT {
        return Err(ExecError::new(
            "EXEC_IO",
            format!("child {stream} exceeds output limit"),
        ));
    }
    Ok(bytes)
}
fn kill_and_reap(child: &mut impl ChildControl) -> bool {
    let _ = child.kill();
    child.wait().is_ok()
}

// ---------------------------------------------------------------------------
// `neo:exec.spawn`: owned, synchronous child processes.
//
// Each spawned child is owned by one supervisor thread. The supervisor reaps
// the direct child as soon as it exits (so an idle handle never leaves a
// zombie), owns the child's stdin handle, and answers one synchronous
// `wait`, `communicate`, or `kill` call before exiting. Two capture threads
// drain stdout and stderr concurrently so a child cannot block on a full pipe
// while JavaScript is working on a different handle. Readers keep a clone of
// the slot reservation, so a cleanup that cannot be confirmed holds the slot
// for the lifetime of the lingering thread, exactly like the one-shot `exec`
// supervisor.
//
// A handle owns its slot until one terminal call consumes the handle, or until
// the runtime is dropped. That bounds outstanding handles (and their cached
// output) by the process-wide slot limit. Terminal results are cached in the
// JavaScript wrapper, so repeated calls do not need a second Rust call.
// ---------------------------------------------------------------------------

/// Runtime-local registry of spawned-child supervisors, keyed by handle id.
pub(crate) struct ExecState {
    next_id: u64,
    handles: HashMap<u64, Sender<SpawnCommand>>,
}

impl ExecState {
    pub(crate) fn new() -> Self {
        Self {
            next_id: 1,
            handles: HashMap::new(),
        }
    }
}

/// One terminal capture result. `complete` is true when both output streams
/// reached end of file before the result was produced; `kill` may return a
/// snapshot with `complete` false when a descendant still holds a pipe.
struct SpawnResult {
    code: Option<i32>,
    success: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    complete: bool,
}

impl SpawnResult {
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "code": self.code,
            "success": self.success,
            "stdout": self.stdout,
            "stderr": self.stderr,
            "complete": self.complete,
        })
    }
}

enum SpawnCommand {
    Wait {
        input: Option<Vec<u8>>,
        reply: Sender<Result<SpawnResult, ExecError>>,
    },
    Kill {
        reply: Sender<Result<SpawnResult, ExecError>>,
    },
    Abort {
        reply: Sender<()>,
    },
}

/// Shared slot reservation. The counter is released when the last clone drops,
/// which may be a capture thread that outlives the supervisor.
struct PermitInner {
    safe_to_release: AtomicBool,
}

impl Drop for PermitInner {
    fn drop(&mut self) {
        if self.safe_to_release.load(Ordering::Acquire) {
            SLOTS.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

#[derive(Clone)]
struct Permit(Arc<PermitInner>);

impl Permit {
    fn reserve() -> Result<Self, ExecError> {
        reserve_counter(&SLOTS, SLOT_LIMIT)?;
        Ok(Self(Arc::new(PermitInner {
            safe_to_release: AtomicBool::new(true),
        })))
    }

    /// Retention is required when the child could not be reaped: releasing the
    /// slot would assert a cleanup that was not proven.
    fn disarm(&self) {
        self.0.safe_to_release.store(false, Ordering::Release);
    }

    fn arm(&self) {
        self.0.safe_to_release.store(true, Ordering::Release);
    }
}

#[derive(Default)]
struct CaptureState {
    bytes: Vec<u8>,
    overflow: bool,
    done: bool,
    error: Option<ExecError>,
}

struct Capture {
    stream: &'static str,
    state: Mutex<CaptureState>,
}

impl Capture {
    fn new(stream: &'static str) -> Arc<Self> {
        Arc::new(Self {
            stream,
            state: Mutex::new(CaptureState::default()),
        })
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn capture_done(capture: &Capture) -> bool {
    lock(&capture.state).done
}

fn capture_failure(captures: &[Arc<Capture>; 2]) -> Option<ExecError> {
    for capture in captures {
        let state = lock(&capture.state);
        if state.overflow {
            return Some(ExecError::new(
                "EXEC_IO",
                format!("child {} exceeds output limit", capture.stream),
            ));
        }
        if let Some(error) = &state.error {
            return Some(error.clone());
        }
    }
    None
}

fn capture_bytes(capture: &Capture) -> Vec<u8> {
    std::mem::take(&mut lock(&capture.state).bytes)
}

/// Drain one stream to end of file. Bytes beyond the harness limit are read and
/// discarded, so a verbose child never blocks on a pipe; `wait` and
/// `communicate` report the overflow, while `kill` returns a snapshot marked
/// incomplete. A read error is recorded for the terminal call.
fn drain_capture(mut reader: impl Read, capture: Arc<Capture>, permit: Permit) {
    let _permit = permit;
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let mut buffer = [0_u8; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => return Ok(()),
                Ok(count) => {
                    let mut state = lock(&capture.state);
                    let remaining = OUTPUT_LIMIT as usize - state.bytes.len();
                    let kept = remaining.min(count);
                    state.bytes.extend_from_slice(&buffer[..kept]);
                    if kept < count {
                        state.overflow = true;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    return Err(ExecError::new(
                        "EXEC_IO",
                        format!("cannot read child {}: {error}", capture.stream),
                    ));
                }
            }
        }
    }));
    let error = match outcome {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(error),
        Err(_) => Some(ExecError::new(
            "EXEC_WORKER",
            "child capture worker panicked",
        )),
    };
    let mut state = lock(&capture.state);
    state.done = true;
    if error.is_some() {
        state.error = error;
    }
}

#[derive(Default)]
struct WriterState {
    done: bool,
    error: Option<ExecError>,
}

struct Writer {
    state: Arc<Mutex<WriterState>>,
    worker: JoinHandle<()>,
}

fn start_writer(stdin: ChildStdin, input: Vec<u8>, permit: Permit) -> io::Result<Writer> {
    let state = Arc::new(Mutex::new(WriterState::default()));
    let writer_state = Arc::clone(&state);
    let worker = thread::Builder::new()
        .name("neo-js-spawn-stdin".into())
        .spawn(move || {
            let _permit = permit;
            let outcome = catch_unwind(AssertUnwindSafe(|| write_input(stdin, &input)));
            let mut state = lock(&writer_state);
            state.done = true;
            match outcome {
                Ok(Ok(())) => {}
                Ok(Err(error)) => state.error = Some(error),
                Err(_) => {
                    state.error =
                        Some(ExecError::new("EXEC_WORKER", "child stdin worker panicked"));
                }
            }
        })?;
    Ok(Writer { state, worker })
}

/// Narrow process-control surface used by the spawn coordinator. It is also
/// what the injected fault tests implement, so kill/reap ordering can be
/// exercised without a real OS process.
trait SpawnChild {
    fn kill(&mut self) -> io::Result<()>;
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>>;
    fn wait(&mut self) -> io::Result<ExitStatus>;
}

impl SpawnChild for Child {
    fn kill(&mut self) -> io::Result<()> {
        Child::kill(self)
    }

    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        Child::try_wait(self)
    }

    fn wait(&mut self) -> io::Result<ExitStatus> {
        Child::wait(self)
    }
}

enum ReapMode {
    /// Wait for the status. Used by `kill`, which reports it to the caller.
    Blocking,
    /// Never wait: a failure path must report promptly even if the child is
    /// still running or cannot be reaped. The supervisor continues polling.
    NonBlocking,
}

enum KillOutcome {
    Reaped(ExitStatus),
    /// Termination was requested but not confirmed. The caller gets this
    /// error; the supervisor keeps ownership and the slot until it reaps.
    Pending(ExecError),
}

/// Attempt to terminate and reap the direct child.
///
/// A failed `kill` never falls through to a blocking `wait`: on Unix that can
/// wait forever for a child that is still running. A nonblocking `try_wait`
/// distinguishes "already gone" from "still running or unknown".
fn terminate_child(child: &mut impl SpawnChild, mode: ReapMode) -> KillOutcome {
    match child.kill() {
        Ok(()) => match mode {
            ReapMode::Blocking => match child.wait() {
                Ok(status) => KillOutcome::Reaped(status),
                Err(error) => KillOutcome::Pending(ExecError::new(
                    "EXEC_WAIT",
                    format!(
                        "cannot reap child: {error}; cleanup pending; \
                         the direct child may survive"
                    ),
                )),
            },
            ReapMode::NonBlocking => match child.try_wait() {
                Ok(Some(status)) => KillOutcome::Reaped(status),
                Ok(None) => KillOutcome::Pending(ExecError::new(
                    "EXEC_KILL",
                    "kill sent; cleanup pending; the direct child may survive",
                )),
                Err(error) => KillOutcome::Pending(ExecError::new(
                    "EXEC_WAIT",
                    format!(
                        "cannot confirm child termination: {error}; cleanup unknown; \
                         the direct child may survive"
                    ),
                )),
            },
        },
        Err(kill_error) => match child.try_wait() {
            Ok(Some(status)) => KillOutcome::Reaped(status),
            Ok(None) => KillOutcome::Pending(ExecError::new(
                "EXEC_KILL",
                format!(
                    "cannot kill child: {kill_error}; cleanup pending; \
                     the direct child may survive"
                ),
            )),
            Err(error) => KillOutcome::Pending(ExecError::new(
                "EXEC_KILL",
                format!(
                    "cannot kill child: {kill_error}; cannot confirm termination: {error}; \
                     cleanup unknown; the direct child may survive"
                ),
            )),
        },
    }
}

/// Outcome of preparing to report a `wait`/`communicate` failure.
struct FailureReport {
    /// The primary error, with a cleanup qualifier appended when termination
    /// was not confirmed.
    error: ExecError,
    /// True when the supervisor must keep ownership and the slot and keep
    /// polling for the child to exit.
    pending: bool,
    status: Option<ExitStatus>,
}

/// Before reporting an IO/worker/wait failure, attempt a nonblocking stop so
/// the error can state whether the direct child is confirmed reaped. This
/// never blocks on a running child: if termination cannot be confirmed, the
/// error says so and cleanup continues in the background.
fn prepare_failure(
    child: &mut impl SpawnChild,
    status: Option<ExitStatus>,
    mut error: ExecError,
) -> FailureReport {
    if status.is_some() {
        return FailureReport {
            error,
            pending: false,
            status,
        };
    }
    match terminate_child(child, ReapMode::NonBlocking) {
        KillOutcome::Reaped(status) => FailureReport {
            error,
            pending: false,
            status: Some(status),
        },
        KillOutcome::Pending(cleanup) => {
            error.message.push_str("; ");
            error.message.push_str(&cleanup.message);
            FailureReport {
                error,
                pending: true,
                status: None,
            }
        }
    }
}

/// Owns the direct child and all workers until a terminal call or abort.
struct Supervisor {
    child: Child,
    status: Option<ExitStatus>,
    stdin: Option<ChildStdin>,
    captures: [Arc<Capture>; 2],
    workers: Vec<JoinHandle<()>>,
    /// Taken on a fully clean terminal outcome so the slot is released before
    /// the caller observes the result. `None` after release.
    permit: Option<Permit>,
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        if self.status.is_none() {
            // Never block this thread on a kill failure. If termination is
            // not confirmed, retain the slot permanently rather than claim a
            // cleanup that did not happen.
            match terminate_child(&mut self.child, ReapMode::NonBlocking) {
                KillOutcome::Reaped(status) => self.status = Some(status),
                KillOutcome::Pending(_) => {
                    if let Some(permit) = &self.permit {
                        permit.disarm();
                    }
                }
            }
        }
    }
}

/// Reap the direct child while the handle is idle, then wait for one command.
fn supervise_spawn(mut supervisor: Supervisor, receiver: Receiver<SpawnCommand>) {
    loop {
        if supervisor.status.is_none() {
            if let Ok(status) = supervisor.child.try_wait() {
                supervisor.status = status;
            }
        }
        match receiver.recv_timeout(Duration::from_millis(10)) {
            Ok(SpawnCommand::Wait { input, reply }) => {
                run_wait(supervisor, input, reply);
                return;
            }
            Ok(SpawnCommand::Kill { reply }) => {
                run_kill(supervisor, reply);
                return;
            }
            Ok(SpawnCommand::Abort { reply }) => {
                run_abort(supervisor, reply);
                return;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                // The runtime dropped the registry without an explicit abort.
                run_abort(supervisor, mpsc::channel().0);
                return;
            }
        }
    }
}

fn run_wait(
    mut supervisor: Supervisor,
    input: Option<Vec<u8>>,
    reply: Sender<Result<SpawnResult, ExecError>>,
) {
    let mut writer = None;
    match input {
        Some(bytes) if !bytes.is_empty() => {
            let Some(stdin) = supervisor.stdin.take() else {
                let status = supervisor.status;
                fail_wait(
                    supervisor,
                    status,
                    ExecError::new(
                        "EXEC_STATE",
                        "child stdin was already closed; no input was written",
                    ),
                    reply,
                );
                return;
            };
            let permit = supervisor
                .permit
                .as_ref()
                .expect("a live supervisor always retains its slot")
                .clone();
            match start_writer(stdin, bytes, permit) {
                Ok(started) => writer = Some(started),
                Err(error) => {
                    let status = supervisor.status;
                    fail_wait(
                        supervisor,
                        status,
                        ExecError::new(
                            "EXEC_WORKER",
                            format!("cannot start child stdin worker: {error}"),
                        ),
                        reply,
                    );
                    return;
                }
            }
        }
        _ => {
            // Closing stdin is what makes a child that reads stdin finish.
            supervisor.stdin.take();
        }
    }

    let mut status = supervisor.status;
    loop {
        if status.is_none() {
            match supervisor.child.try_wait() {
                Ok(value) => status = value,
                Err(error) => {
                    fail_wait(
                        supervisor,
                        status,
                        ExecError::new("EXEC_WAIT", format!("cannot wait for child: {error}")),
                        reply,
                    );
                    return;
                }
            }
        }
        if let Some(error) = capture_failure(&supervisor.captures) {
            fail_wait(supervisor, status, error, reply);
            return;
        }
        let writer_error = writer
            .as_ref()
            .and_then(|started| lock(&started.state).error.clone());
        if let Some(error) = writer_error {
            drop(writer.take());
            fail_wait(supervisor, status, error, reply);
            return;
        }
        let captures_done = supervisor
            .captures
            .iter()
            .all(|capture| capture_done(capture));
        let writer_done = writer
            .as_ref()
            .is_none_or(|started| lock(&started.state).done);
        if status.is_some() && captures_done && writer_done {
            supervisor.status = status;
            let mut joined = true;
            for worker in supervisor.workers.drain(..) {
                joined &= worker.join().is_ok();
            }
            if let Some(started) = writer.take() {
                joined &= started.worker.join().is_ok();
            }
            if !joined {
                fail_wait(
                    supervisor,
                    status,
                    ExecError::new(
                        "EXEC_WORKER",
                        "child worker join failed; cleanup state unknown",
                    ),
                    reply,
                );
                return;
            }
            let result = SpawnResult {
                code: status.and_then(|status| status.code()),
                success: status.is_some_and(|status| status.success()),
                stdout: capture_bytes(&supervisor.captures[0]),
                stderr: capture_bytes(&supervisor.captures[1]),
                complete: true,
            };
            // All workers were joined, so this is the last reservation clone.
            supervisor.permit.take();
            let _ = reply.send(Ok(result));
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// Report a failure after attempting a nonblocking stop. If termination was
/// confirmed, the error is sent without a pending qualifier. Otherwise the
/// error says cleanup is pending/unknown and a background supervisor retains
/// ownership and the slot until the child is reaped.
fn fail_wait(
    mut supervisor: Supervisor,
    status: Option<ExitStatus>,
    primary: ExecError,
    reply: Sender<Result<SpawnResult, ExecError>>,
) {
    supervisor.status = status;
    let report = prepare_failure(&mut supervisor.child, supervisor.status, primary);
    supervisor.status = report.status;
    let _ = reply.send(Err(report.error));
    if report.pending {
        cleanup_pending(supervisor);
    }
}

/// Keep the direct child and its slot while polling for exit. Runs only after
/// the caller has been told that cleanup is pending/unknown, so it must not
/// send another reply or block a runtime operation.
fn cleanup_pending(mut supervisor: Supervisor) {
    supervisor.stdin.take();
    loop {
        match supervisor.child.try_wait() {
            Ok(Some(status)) => {
                supervisor.status = Some(status);
                return;
            }
            Ok(None) => {}
            Err(_) => {}
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn run_kill(mut supervisor: Supervisor, reply: Sender<Result<SpawnResult, ExecError>>) {
    // Kill before closing stdin: a child that reads stdin would otherwise see
    // EOF and could exit successfully before the signal is delivered. A
    // successful signal is followed by a blocking reap so the result carries
    // a status; a kill failure never falls through to a blocking wait.
    if supervisor.status.is_none() {
        match terminate_child(&mut supervisor.child, ReapMode::Blocking) {
            KillOutcome::Reaped(status) => supervisor.status = Some(status),
            KillOutcome::Pending(error) => {
                // The caller is told cleanup is pending; keep ownership and
                // the slot and poll instead of waiting on a live child.
                let _ = reply.send(Err(error));
                cleanup_pending(supervisor);
                return;
            }
        }
    }
    supervisor.stdin.take();
    // Snapshot without waiting for EOF: a descendant may hold a pipe forever.
    let complete = supervisor.captures.iter().all(|capture| {
        let state = lock(&capture.state);
        state.done && !state.overflow && state.error.is_none()
    });
    let result = SpawnResult {
        code: supervisor.status.and_then(|status| status.code()),
        success: supervisor.status.is_some_and(|status| status.success()),
        stdout: capture_bytes(&supervisor.captures[0]),
        stderr: capture_bytes(&supervisor.captures[1]),
        complete,
    };
    if complete {
        // Both streams reached EOF, so the capture threads are finished. Join
        // them and release the slot before the caller observes the result.
        for worker in supervisor.workers.drain(..) {
            let _ = worker.join();
        }
        supervisor.permit.take();
    }
    let _ = reply.send(Ok(result));
}

fn run_abort(mut supervisor: Supervisor, reply: Sender<()>) {
    // Abort does not need the exit status, so use a nonblocking stop: the
    // runtime drop must not wait on a live child. If termination is not
    // confirmed, acknowledge and keep polling in the background.
    if supervisor.status.is_none() {
        match terminate_child(&mut supervisor.child, ReapMode::NonBlocking) {
            KillOutcome::Reaped(status) => supervisor.status = Some(status),
            KillOutcome::Pending(_) => {
                let _ = reply.send(());
                cleanup_pending(supervisor);
                return;
            }
        }
    }
    supervisor.stdin.take();
    let _ = reply.send(());
}

fn error_response(error: ExecError) -> String {
    serde_json::json!({"error": {"code": error.code, "message": error.message}}).to_string()
}

fn result_response(result: Result<SpawnResult, ExecError>) -> String {
    match result {
        Ok(result) => serde_json::json!({"result": result.to_json()}).to_string(),
        Err(error) => error_response(error),
    }
}

/// Launch a child and return its handle id.
pub(crate) fn spawn(
    state: &Rc<RefCell<ExecState>>,
    program: &str,
    args: &[String],
    directory: Option<&std::path::Path>,
) -> String {
    match spawn_inner(state, program, args, directory) {
        Ok(id) => serde_json::json!({"result": {"id": id}}).to_string(),
        Err(error) => error_response(error),
    }
}

fn spawn_inner(
    state: &Rc<RefCell<ExecState>>,
    program: &str,
    args: &[String],
    directory: Option<&std::path::Path>,
) -> Result<u64, ExecError> {
    let Some(directory) = directory else {
        return Err(ExecError::new(
            "EXEC_CWD",
            "the runtime working directory is unavailable",
        ));
    };
    check_program(program).map_err(|error| ExecError::new("EXEC_UNSUPPORTED", error))?;
    let permit = Permit::reserve()?;
    let program = program.to_owned();
    let args = args.to_owned();
    let directory = directory.to_path_buf();
    let (sender, receiver) = mpsc::channel();
    let (ready_tx, ready_rx) = mpsc::channel();
    // Allocate the owner before spawning: failed thread creation cannot drop a child.
    thread::Builder::new()
        .name("neo-js-spawn-supervisor".into())
        .spawn(move || start_spawn_owner(program, args, directory, permit, receiver, ready_tx))
        .map_err(|error| {
            ExecError::new(
                "EXEC_WORKER",
                format!("cannot start child supervisor; no child spawned: {error}"),
            )
        })?;
    ready_rx.recv().unwrap_or_else(|_| {
        Err(ExecError::new(
            "EXEC_WORKER",
            "child supervisor stopped unexpectedly; cleanup state unknown; child may survive",
        ))
    })?;
    let mut state = state
        .try_borrow_mut()
        .map_err(|_| ExecError::new("EXEC_STATE", "process state is unavailable"))?;
    let id = state.next_id;
    state.next_id = state.next_id.wrapping_add(1).max(1);
    state.handles.insert(id, sender);
    Ok(id)
}

// Startup and every failure after Command::spawn remain on the preallocated owner.
fn start_spawn_owner(
    program: String,
    args: Vec<String>,
    directory: std::path::PathBuf,
    permit: Permit,
    receiver: Receiver<SpawnCommand>,
    ready: Sender<Result<(), ExecError>>,
) {
    start_spawn_owner_with_fault(
        program,
        args,
        directory,
        permit,
        receiver,
        ready,
        StartupFault::None,
    );
}

#[derive(Clone, Copy)]
enum StartupFault {
    None,
    #[cfg(test)]
    MissingPipe,
    #[cfg(test)]
    Capture(usize),
}

fn start_spawn_owner_with_fault(
    program: String,
    args: Vec<String>,
    directory: std::path::PathBuf,
    permit: Permit,
    receiver: Receiver<SpawnCommand>,
    ready: Sender<Result<(), ExecError>>,
    fault: StartupFault,
) {
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let _ = ready.send(Err(ExecError::new(
                "EXEC_SPAWN",
                format!("cannot start child: {error}"),
            )));
            return;
        }
    };
    // If startup panics before Supervisor takes ownership, do not release a
    // reservation for a child whose cleanup cannot be confirmed.
    permit.disarm();
    #[cfg(test)]
    if matches!(fault, StartupFault::MissingPipe) {
        child.stdout.take();
    }
    #[cfg(not(test))]
    let _ = fault;
    let pipes = (child.stdin.take(), child.stdout.take(), child.stderr.take());
    let (Some(stdin), Some(stdout), Some(stderr)) = pipes else {
        startup_failure(
            child,
            permit,
            ready,
            ExecError::new("EXEC_IO", "child pipes unavailable"),
        );
        return;
    };
    let stdout_capture = Capture::new("stdout");
    let stderr_capture = Capture::new("stderr");
    let mut workers = Vec::with_capacity(2);
    for (index, (reader, capture)) in [
        (
            Box::new(stdout) as Box<dyn Read + Send>,
            Arc::clone(&stdout_capture),
        ),
        (
            Box::new(stderr) as Box<dyn Read + Send>,
            Arc::clone(&stderr_capture),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        #[cfg(test)]
        let injected = matches!(fault, StartupFault::Capture(n) if n == index);
        #[cfg(not(test))]
        let injected = {
            let _ = index;
            false
        };
        let result = if injected {
            Err(io::Error::other("injected capture startup failure"))
        } else {
            thread::Builder::new()
                .name("neo-js-spawn-capture".into())
                .spawn({
                    let permit = permit.clone();
                    move || drain_capture(reader, capture, permit)
                })
        };
        match result {
            Ok(worker) => workers.push(worker),
            Err(error) => {
                drop(stdin);
                startup_failure(
                    child,
                    permit,
                    ready,
                    ExecError::new(
                        "EXEC_WORKER",
                        format!("cannot start child capture worker: {error}"),
                    ),
                );
                return;
            }
        }
    }
    let supervisor = Supervisor {
        child,
        status: None,
        stdin: Some(stdin),
        captures: [stdout_capture, stderr_capture],
        workers,
        permit: Some(permit),
    };
    if let Some(permit) = &supervisor.permit {
        permit.arm();
    }
    if ready.send(Ok(())).is_ok() {
        supervise_spawn(supervisor, receiver);
    } else {
        run_abort(supervisor, mpsc::channel().0);
    }
}

fn startup_failure(
    mut child: impl SpawnChild,
    permit: Permit,
    ready: Sender<Result<(), ExecError>>,
    primary: ExecError,
) {
    let report = prepare_failure(&mut child, None, primary);
    let _ = ready.send(Err(report.error));
    if report.pending {
        loop {
            if matches!(SpawnChild::try_wait(&mut child), Ok(Some(_))) {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
    permit.arm();
    drop(permit);
}

/// Run one synchronous terminal call against a spawned handle.
pub(crate) fn call(
    state: &Rc<RefCell<ExecState>>,
    id: u64,
    action: &str,
    input: Vec<u8>,
) -> String {
    let sender = {
        let state = match state.try_borrow() {
            Ok(state) => state,
            Err(_) => {
                return error_response(ExecError::new(
                    "EXEC_STATE",
                    "process state is unavailable",
                ));
            }
        };
        match state.handles.get(&id) {
            Some(sender) => sender.clone(),
            None => {
                return error_response(ExecError::new(
                    "EXEC_HANDLE",
                    "the process handle is unknown or already consumed",
                ));
            }
        }
    };
    let (reply, response) = mpsc::channel();
    let command = match action {
        "wait" => SpawnCommand::Wait { input: None, reply },
        "communicate" => SpawnCommand::Wait {
            input: Some(input),
            reply,
        },
        "kill" => SpawnCommand::Kill { reply },
        _ => {
            return error_response(ExecError::new(
                "EXEC_STATE",
                "unknown process handle action",
            ));
        }
    };
    if sender.send(command).is_err() {
        if let Ok(mut state) = state.try_borrow_mut() {
            state.handles.remove(&id);
        }
        return error_response(ExecError::new(
            "EXEC_WORKER",
            "process supervisor stopped unexpectedly; cleanup state unknown; the direct child may survive",
        ));
    }
    let result = response.recv();
    // A handle answers exactly one terminal call. The JavaScript wrapper keeps
    // the answer so repeated calls do not need a live supervisor.
    if let Ok(mut state) = state.try_borrow_mut() {
        state.handles.remove(&id);
    }
    match result {
        Ok(result) => result_response(result),
        Err(_) => error_response(ExecError::new(
            "EXEC_WORKER",
            "process supervisor stopped unexpectedly; cleanup state unknown; the direct child may survive",
        )),
    }
}

/// Abort every outstanding handle. Called while the runtime is being dropped,
/// so the direct child is asked to terminate before this returns or the shared
/// acknowledgement budget expires.
const ABORT_BUDGET: Duration = Duration::from_secs(10);

pub(crate) fn abort_all(state: &Rc<RefCell<ExecState>>) {
    abort_all_with_budget(state, ABORT_BUDGET);
}

/// Send aborts to every handle first, then wait for acknowledgements under one
/// shared deadline. A supervisor that does not acknowledge within the budget
/// keeps running in the background; it is not waited on serially.
fn abort_all_with_budget(state: &Rc<RefCell<ExecState>>, budget: Duration) {
    let senders: Vec<Sender<SpawnCommand>> = {
        let Ok(mut state) = state.try_borrow_mut() else {
            return;
        };
        state.handles.drain().map(|(_, sender)| sender).collect()
    };
    // One shared acknowledgement channel, so a slow supervisor cannot make
    // the caller wait on the others in sender order.
    let (acknowledge, acknowledged) = mpsc::channel();
    let mut expected = 0;
    for sender in senders {
        if sender
            .send(SpawnCommand::Abort {
                reply: acknowledge.clone(),
            })
            .is_ok()
        {
            expected += 1;
        }
    }
    drop(acknowledge);
    let deadline = Instant::now() + budget;
    let mut received = 0;
    while received < expected {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        if acknowledged.recv_timeout(remaining).is_err() {
            break;
        }
        received += 1;
    }
}

/// Windows can dispatch `.bat` and `.cmd` programs through `cmd.exe`, and its
/// command search path can resolve an extensionless name to one of those files.
/// Require an explicit `.exe` or `.com` so the program string always names a
/// directly executable image. An explicit shell such as `cmd.exe` is allowed
/// and retains normal shell semantics.
///
/// This is compiled on every platform so its policy can be tested without a
/// Windows host; it is only enforced on Windows.
#[cfg_attr(not(windows), allow(dead_code))]
fn windows_program_policy(program: &str) -> Result<(), String> {
    let extension = std::path::Path::new(program)
        .extension()
        .and_then(|extension| extension.to_str());
    if extension.is_some_and(|extension| {
        extension.eq_ignore_ascii_case("exe") || extension.eq_ignore_ascii_case("com")
    }) {
        return Ok(());
    }
    Err(
        "on Windows, neo:exec requires an explicit .exe or .com program; \
         invoke cmd.exe or powershell.exe explicitly to use shell features"
            .to_owned(),
    )
}

#[cfg(windows)]
fn check_program(program: &str) -> Result<(), String> {
    windows_program_policy(program)
}

#[cfg(not(windows))]
fn check_program(_program: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::windows_program_policy;

    #[test]
    fn windows_policy_requires_explicit_exe_or_com_programs() {
        assert!(windows_program_policy("cmd.exe").is_ok());
        assert!(windows_program_policy("CMD.EXE").is_ok());
        assert!(windows_program_policy(r"C:\Windows\System32\cmd.exe").is_ok());
        assert!(windows_program_policy("command.com").is_ok());
        assert!(windows_program_policy("child.cmd").is_err());
        assert!(windows_program_policy("child.bat").is_err());
        assert!(windows_program_policy("child.BAT").is_err());
        assert!(windows_program_policy("cmd").is_err());
        assert!(windows_program_policy(r"C:\Windows\System32\cmd").is_err());
        assert!(windows_program_policy("cmd.exe.").is_err());
        assert!(windows_program_policy("cmd.exe ").is_err());
        assert!(
            windows_program_policy("child.bat")
                .unwrap_err()
                .contains(".exe")
        );
    }
}

#[cfg(test)]
mod fault_tests {
    use super::*;
    use std::sync::Arc;

    struct FaultChild {
        fail_wait: bool,
        reap_ok: bool,
        block: Option<Receiver<()>>,
        calls: Arc<AtomicUsize>,
        event_on_wait: Option<Sender<Event>>,
    }
    impl ChildControl for FaultChild {
        fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
            if let Some(tx) = self.event_on_wait.take() {
                tx.send(Event::Stdin(Err(ExecError::new(
                    "EXEC_IO",
                    "queued during wait",
                ))))
                .unwrap();
            }
            if self.fail_wait {
                Err(io::Error::other("injected wait"))
            } else {
                Ok(None)
            }
        }
        fn kill(&mut self) -> io::Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some(rx) = self.block.take() {
                rx.recv().unwrap();
            }
            Ok(())
        }
        fn wait(&mut self) -> io::Result<ExitStatus> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.reap_ok {
                #[cfg(unix)]
                {
                    use std::os::unix::process::ExitStatusExt;
                    return Ok(ExitStatus::from_raw(0));
                }
                #[cfg(windows)]
                {
                    use std::os::windows::process::ExitStatusExt;
                    return Ok(ExitStatus::from_raw(0));
                }
            }
            Err(io::Error::other("injected reap"))
        }
    }
    struct BadIo(io::ErrorKind);
    impl Read for BadIo {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::from(self.0))
        }
    }
    impl Write for BadIo {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(self.0))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    // Faults and blocked cleanup cannot be reliably induced through a real OS pipe.
    #[test]
    fn io_fault_is_notified_before_blocked_cleanup_and_reservation_is_held() {
        assert!(write_input(BadIo(io::ErrorKind::BrokenPipe), b"x").is_ok());
        assert_eq!(
            write_input(BadIo(io::ErrorKind::Other), b"x")
                .unwrap_err()
                .code,
            "EXEC_IO"
        );
        let error = read_output(BadIo(io::ErrorKind::Other), "stdout").unwrap_err();
        let counter = Box::leak(Box::new(AtomicUsize::new(0)));
        let slot = Slot::reserve(counter, 1).unwrap();
        let (unblock, blocked) = mpsc::channel();
        let (reply, received) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let worker = start_worker(event_tx, Box::new(move || Event::Stdin(Err(error)))).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let mut child = FaultChild {
            fail_wait: false,
            reap_ok: true,
            block: Some(blocked),
            calls: calls.clone(),
            event_on_wait: None,
        };
        let owner = thread::spawn(move || {
            let mut reply = Some(reply);
            let (_, safe) = coordinate(&mut child, event_rx, vec![worker], &mut reply);
            let mut slot = slot;
            slot.safe_to_release = safe;
            drop(slot);
        });
        let error = received
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap_err();
        assert_eq!(error.code, "EXEC_IO");
        assert!(error.message.contains("cleanup pending or unknown"));
        assert!(Slot::reserve(counter, 1).is_err());
        unblock.send(()).unwrap();
        owner.join().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }
    // Ordering is tested with injected events instead of nondeterministic OS failures.
    #[test]
    fn queued_worker_error_wins_over_wait_error_even_when_wait_enqueues_it() {
        for during_wait in [false, true] {
            let (tx, rx) = mpsc::channel();
            if !during_wait {
                tx.send(Event::Stdin(Err(ExecError::new("EXEC_IO", "queued"))))
                    .unwrap();
            }
            let mut child = FaultChild {
                fail_wait: true,
                reap_ok: false,
                block: None,
                calls: Arc::new(AtomicUsize::new(0)),
                event_on_wait: during_wait.then_some(tx.clone()),
            };
            let (reply, received) = mpsc::channel();
            let worker = thread::spawn(|| {});
            let _ = coordinate(&mut child, rx, vec![worker], &mut Some(reply));
            assert_eq!(received.recv().unwrap().unwrap_err().code, "EXEC_IO");
        }
    }
    // Supervisor startup failure is not deterministic in normal operation.
    #[test]
    fn supervisor_start_failure_releases_reservation_before_spawn() {
        let counter = Box::leak(Box::new(AtomicUsize::new(0)));
        let slot = Slot::reserve(counter, 1).unwrap();
        let (tx, _) = mpsc::channel();
        let error = start_supervisor(slot, tx, Box::new(|_, _| panic!("must not run")), |_| {
            Err(io::Error::other("injected spawn"))
        })
        .unwrap_err();
        assert_eq!(error.code, "EXEC_WORKER");
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }
    // Panics and wait failures require injection to exercise deterministically.
    #[test]
    fn panic_and_wait_failures_are_reported() {
        for wait_failure in [false, true] {
            let (tx, rx) = mpsc::channel();
            let worker = if wait_failure {
                thread::spawn(move || drop(tx))
            } else {
                start_worker(tx, Box::new(|| panic!("injected"))).unwrap()
            };
            let mut child = FaultChild {
                fail_wait: wait_failure,
                reap_ok: false,
                block: None,
                calls: Arc::new(AtomicUsize::new(0)),
                event_on_wait: None,
            };
            let (reply, received) = mpsc::channel();
            let _ = coordinate(&mut child, rx, vec![worker], &mut Some(reply));
            assert_eq!(
                received.recv().unwrap().unwrap_err().code,
                if wait_failure {
                    "EXEC_WAIT"
                } else {
                    "EXEC_WORKER"
                }
            );
        }
    }
}

#[cfg(all(test, any(unix, windows)))]
mod spawn_fault_tests {
    use super::*;
    use std::{
        sync::{
            Arc, Barrier,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        time::Instant,
    };

    fn reaped_status() -> ExitStatus {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            ExitStatus::from_raw(0)
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::ExitStatusExt;
            ExitStatus::from_raw(0)
        }
    }

    enum WaitBehavior {
        Status,
        Error(io::ErrorKind),
        Block(mpsc::Receiver<()>),
    }

    enum TryWaitBehavior {
        Running,
        Exited,
        Error(io::ErrorKind),
    }

    struct FakeChild {
        kill_result: Option<io::ErrorKind>,
        try_wait: TryWaitBehavior,
        wait: WaitBehavior,
        kill_calls: Arc<AtomicUsize>,
        wait_calls: Arc<AtomicUsize>,
    }

    impl FakeChild {
        fn new(
            kill_result: Option<io::ErrorKind>,
            try_wait: TryWaitBehavior,
            wait: WaitBehavior,
        ) -> Self {
            Self {
                kill_result,
                try_wait,
                wait,
                kill_calls: Arc::new(AtomicUsize::new(0)),
                wait_calls: Arc::new(AtomicUsize::new(0)),
            }
        }
    }

    impl SpawnChild for FakeChild {
        fn kill(&mut self) -> io::Result<()> {
            self.kill_calls.fetch_add(1, Ordering::SeqCst);
            match self.kill_result {
                Some(kind) => Err(io::Error::from(kind)),
                None => Ok(()),
            }
        }

        fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
            match self.try_wait {
                TryWaitBehavior::Running => Ok(None),
                TryWaitBehavior::Exited => Ok(Some(reaped_status())),
                TryWaitBehavior::Error(kind) => Err(io::Error::from(kind)),
            }
        }

        fn wait(&mut self) -> io::Result<ExitStatus> {
            self.wait_calls.fetch_add(1, Ordering::SeqCst);
            match &mut self.wait {
                WaitBehavior::Status => Ok(reaped_status()),
                WaitBehavior::Error(kind) => Err(io::Error::from(*kind)),
                WaitBehavior::Block(release) => {
                    let _ = release.recv();
                    Ok(reaped_status())
                }
            }
        }
    }

    #[test]
    fn startup_failure_reports_before_reap_and_retains_permit() {
        struct DelayedChild(Arc<AtomicBool>);
        impl SpawnChild for DelayedChild {
            fn kill(&mut self) -> io::Result<()> {
                Err(io::Error::other("injected kill failure"))
            }
            fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
                Ok(self.0.load(Ordering::SeqCst).then(reaped_status))
            }
            fn wait(&mut self) -> io::Result<ExitStatus> {
                panic!("startup failure must not block in wait")
            }
        }
        let permit = Permit::reserve().unwrap();
        let weak = Arc::downgrade(&permit.0);
        let exited = Arc::new(AtomicBool::new(false));
        let child = DelayedChild(Arc::clone(&exited));
        let (tx, rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            startup_failure(
                child,
                permit,
                tx,
                ExecError::new("EXEC_IO", "injected missing pipe"),
            );
        });
        let error = rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap_err();
        assert_eq!(error.code, "EXEC_IO");
        assert!(error.message.contains("injected missing pipe"));
        assert!(error.message.contains("cleanup pending"));
        assert!(weak.upgrade().is_some());
        exited.store(true, Ordering::SeqCst);
        worker.join().unwrap();
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn terminal_disconnects_report_unknown_cleanup_without_leaking_panic_text() {
        for panic_after_command in [false, true] {
            let state = Rc::new(RefCell::new(ExecState::new()));
            let (sender, receiver) = mpsc::channel();
            state.borrow_mut().handles.insert(1, sender);
            let permit = Permit::reserve().unwrap();
            let weak = Arc::downgrade(&permit.0);
            let owner_permit = permit.clone();
            let owner = thread::spawn(move || {
                if panic_after_command {
                    let _command = receiver.recv().unwrap();
                    owner_permit.disarm();
                    panic!("private injected panic detail");
                }
                owner_permit.disarm();
                panic!("private injected panic detail");
            });
            if !panic_after_command {
                while !owner.is_finished() {
                    thread::yield_now();
                }
            }
            let response: serde_json::Value =
                serde_json::from_str(&call(&state, 1, "wait", Vec::new())).unwrap();
            assert!(owner.join().is_err());
            assert_eq!(response["error"]["code"], "EXEC_WORKER");
            assert_eq!(
                response["error"]["message"],
                "process supervisor stopped unexpectedly; cleanup state unknown; the direct child may survive"
            );
            assert!(
                !response
                    .to_string()
                    .contains("private injected panic detail")
            );
            assert!(!state.borrow().handles.contains_key(&1));
            assert!(weak.upgrade().is_some());
            assert!(!permit.0.safe_to_release.load(Ordering::Acquire));
            // This test has no actual child; restore the synthetic reservation.
            permit.arm();
            drop(permit);
            assert!(weak.upgrade().is_none());
        }
    }

    #[test]
    fn actual_startup_failure_paths_report_and_retain_owner_until_cleanup() {
        for fault in [
            StartupFault::MissingPipe,
            StartupFault::Capture(0),
            StartupFault::Capture(1),
        ] {
            let executable = std::env::current_exe().unwrap();
            let permit = Permit::reserve().unwrap();
            let weak = Arc::downgrade(&permit.0);
            let (ready, response) = mpsc::channel();
            let (_sender, receiver) = mpsc::channel();
            let worker = thread::spawn(move || {
                start_spawn_owner_with_fault(
                    executable.to_string_lossy().into_owned(),
                    vec!["--list".into()],
                    std::env::current_dir().unwrap(),
                    permit,
                    receiver,
                    ready,
                    fault,
                );
            });
            let error = response
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .unwrap_err();
            match fault {
                StartupFault::MissingPipe => {
                    assert_eq!(error.code, "EXEC_IO");
                    assert!(error.message.contains("child pipes unavailable"));
                }
                StartupFault::Capture(_) => {
                    assert_eq!(error.code, "EXEC_WORKER");
                    assert!(error.message.contains("cannot start child capture worker"));
                }
                StartupFault::None => unreachable!(),
            }
            worker.join().unwrap();
            // Capture threads can finish slightly after the supervisor.
            let deadline = Instant::now() + Duration::from_secs(5);
            while weak.upgrade().is_some() && Instant::now() < deadline {
                thread::yield_now();
            }
            assert!(
                weak.upgrade().is_none(),
                "permit retained after clean startup failure"
            );
        }
    }

    // A kill failure must not fall through to a blocking wait on a child that
    // is still running. The fake's wait blocks, so this test hangs (and is
    // failed by the watchdog) if the implementation regresses.
    #[test]
    fn kill_failure_reports_pending_without_blocking_on_wait() {
        let (release, blocked) = mpsc::channel();
        let wait_calls = Arc::new(AtomicUsize::new(0));
        let mut child = FakeChild {
            kill_result: Some(io::ErrorKind::PermissionDenied),
            try_wait: TryWaitBehavior::Running,
            wait: WaitBehavior::Block(blocked),
            kill_calls: Arc::new(AtomicUsize::new(0)),
            wait_calls: Arc::clone(&wait_calls),
        };
        let (result_tx, result_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let outcome = terminate_child(&mut child, ReapMode::Blocking);
            let _ = result_tx.send(outcome);
        });
        let outcome = match result_rx.recv_timeout(Duration::from_secs(2)) {
            Ok(outcome) => outcome,
            Err(_) => {
                let _ = release.send(());
                panic!("kill failure blocked on child.wait");
            }
        };
        worker.join().unwrap();
        match outcome {
            KillOutcome::Pending(error) => {
                assert_eq!(error.code, "EXEC_KILL");
                assert!(error.message.contains("cleanup pending"));
                assert!(error.message.contains("may survive"));
            }
            KillOutcome::Reaped(_) => panic!("kill failure was reported as reaped"),
        }
        assert_eq!(wait_calls.load(Ordering::SeqCst), 0);
    }

    // A kill failure is not a reap failure if the child is already gone.
    #[test]
    fn kill_failure_after_exit_is_reported_as_reaped() {
        let mut child = FakeChild::new(
            Some(io::ErrorKind::Other),
            TryWaitBehavior::Exited,
            WaitBehavior::Status,
        );
        match terminate_child(&mut child, ReapMode::Blocking) {
            KillOutcome::Reaped(_) => {}
            KillOutcome::Pending(error) => panic!("unexpected pending: {}", error.message),
        }
        assert_eq!(child.wait_calls.load(Ordering::SeqCst), 0);
    }

    // If the kill fails and the state cannot even be checked, the error must
    // say cleanup is unknown rather than claiming a reaped child.
    #[test]
    fn kill_failure_with_unconfirmable_state_reports_unknown() {
        let mut child = FakeChild::new(
            Some(io::ErrorKind::PermissionDenied),
            TryWaitBehavior::Error(io::ErrorKind::Other),
            WaitBehavior::Status,
        );
        match terminate_child(&mut child, ReapMode::Blocking) {
            KillOutcome::Pending(error) => {
                assert_eq!(error.code, "EXEC_KILL");
                assert!(error.message.contains("cleanup unknown"));
                assert!(error.message.contains("may survive"));
            }
            KillOutcome::Reaped(_) => panic!("unknown state was reported as reaped"),
        }
        assert_eq!(child.wait_calls.load(Ordering::SeqCst), 0);
    }

    // A successful signal whose reap fails is pending, not an unconditional
    // reaped result.
    #[test]
    fn reap_failure_reports_pending() {
        let mut child = FakeChild::new(
            None,
            TryWaitBehavior::Running,
            WaitBehavior::Error(io::ErrorKind::Other),
        );
        match terminate_child(&mut child, ReapMode::Blocking) {
            KillOutcome::Pending(error) => {
                assert_eq!(error.code, "EXEC_WAIT");
                assert!(error.message.contains("cleanup pending"));
            }
            KillOutcome::Reaped(_) => panic!("reap failure was reported as reaped"),
        }
        assert_eq!(child.wait_calls.load(Ordering::SeqCst), 1);
    }

    // A nonblocking stop cannot claim the child is reaped just because the
    // signal was sent successfully.
    #[test]
    fn nonblocking_stop_reports_pending_until_reaped() {
        let mut child = FakeChild::new(None, TryWaitBehavior::Running, WaitBehavior::Status);
        match terminate_child(&mut child, ReapMode::NonBlocking) {
            KillOutcome::Pending(error) => {
                assert_eq!(error.code, "EXEC_KILL");
                assert!(error.message.contains("cleanup pending"));
            }
            KillOutcome::Reaped(_) => panic!("unconfirmed stop was reported as reaped"),
        }
        assert_eq!(child.wait_calls.load(Ordering::SeqCst), 0);
    }

    // Injected IO, worker, and wait failures keep their primary code and only
    // add the pending qualifier when cleanup was not confirmed.
    #[test]
    fn failure_report_preserves_primary_code_and_marks_pending_cleanup() {
        for primary_code in ["EXEC_IO", "EXEC_WORKER", "EXEC_WAIT"] {
            let mut child = FakeChild::new(
                Some(io::ErrorKind::PermissionDenied),
                TryWaitBehavior::Running,
                WaitBehavior::Status,
            );
            let report = prepare_failure(
                &mut child,
                None,
                ExecError::new(primary_code, "primary failure"),
            );
            assert!(report.pending);
            assert!(report.status.is_none());
            assert_eq!(report.error.code, primary_code);
            assert!(report.error.message.starts_with("primary failure"));
            assert!(report.error.message.contains("cleanup pending"));
            assert_eq!(child.wait_calls.load(Ordering::SeqCst), 0);
        }
    }

    #[test]
    fn failure_report_omits_pending_qualifier_when_reaped() {
        let mut child = FakeChild::new(
            None,
            TryWaitBehavior::Exited,
            WaitBehavior::Error(io::ErrorKind::Other),
        );
        let report = prepare_failure(
            &mut child,
            None,
            ExecError::new("EXEC_IO", "primary failure"),
        );
        assert!(!report.pending);
        assert!(report.status.is_some());
        assert_eq!(report.error.code, "EXEC_IO");
        assert_eq!(report.error.message, "primary failure");
        assert_eq!(child.wait_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn known_status_needs_no_cleanup() {
        let mut child = FakeChild::new(None, TryWaitBehavior::Running, WaitBehavior::Status);
        let report = prepare_failure(
            &mut child,
            Some(reaped_status()),
            ExecError::new("EXEC_WORKER", "primary failure"),
        );
        assert!(!report.pending);
        assert!(report.status.is_some());
        assert_eq!(report.error.code, "EXEC_WORKER");
        assert_eq!(report.error.message, "primary failure");
        assert_eq!(child.kill_calls.load(Ordering::SeqCst), 0);
    }

    // Multiple supervisors that never acknowledge must be sent their abort
    // before the first budget wait, and the total wait is one shared budget,
    // not one budget per supervisor.
    #[test]
    fn abort_all_sends_every_abort_before_a_shared_budget() {
        let (ready_tx, ready_rx) = mpsc::channel();
        let (received_tx, received_rx) = mpsc::channel();
        let release = Arc::new(Barrier::new(4));
        let times = Arc::new(Mutex::new(Vec::new()));
        let mut state = ExecState::new();
        let mut workers = Vec::new();
        for id in 0..3 {
            let (sender, receiver) = mpsc::channel();
            state.handles.insert(id, sender);
            let ready_tx = ready_tx.clone();
            let received_tx = received_tx.clone();
            let release = Arc::clone(&release);
            let times = Arc::clone(&times);
            workers.push(thread::spawn(move || {
                let _ = ready_tx.send(());
                if let Ok(SpawnCommand::Abort { reply }) = receiver.recv() {
                    lock(&times).push(Instant::now());
                    let _ = received_tx.send(());
                    // Hold the reply open so the acknowledgement cannot arrive
                    // during the abort budget.
                    release.wait();
                    drop(reply);
                }
            }));
        }
        drop(ready_tx);
        drop(received_tx);
        for _ in 0..3 {
            ready_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("a fake supervisor did not start");
        }

        let start = Instant::now();
        abort_all_with_budget(&Rc::new(RefCell::new(state)), Duration::from_millis(300));
        let total = start.elapsed();
        let received = {
            let mut count = 0;
            while received_rx.recv_timeout(Duration::from_millis(1)).is_ok() {
                count += 1;
            }
            count
        };
        assert_eq!(received, 3, "not every supervisor received an abort");
        release.wait();
        for worker in workers {
            worker.join().unwrap();
        }

        let offsets: Vec<Duration> = lock(&times)
            .iter()
            .map(|received| received.saturating_duration_since(start))
            .collect();
        assert_eq!(offsets.len(), 3);
        assert!(
            offsets
                .iter()
                .all(|offset| *offset < Duration::from_millis(150)),
            "aborts were sent one at a time: {offsets:?}"
        );
        assert!(
            total < Duration::from_millis(600),
            "abort budget was not shared: {total:?}"
        );
    }
}
