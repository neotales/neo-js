//! Owner-side promise table and bounded, Rust-only filesystem operations.
//!
//! The owner thread validates and snapshots every input before a job reaches a
//! worker. Workers receive only owned Rust data; no JavaScript context, value,
//! or promise handle crosses the thread boundary. Completion delivery and all
//! promise settlement happen again on the owner thread.

use std::{
    cell::RefCell, collections::BTreeMap, num::NonZeroUsize, path::PathBuf, rc::Rc, time::Duration,
};

use rquickjs::{
    Ctx, Function, Object, Persistent, Promise, TypedArray, Undefined, Value, object::Property,
};

use super::{
    Code, DirEntryInfo, FileInfo, FsError, JsError, JsResult, JsString, MAX_LIST_ENTRIES,
    MAX_LIST_NAME_BYTES, MAX_LIST_TOTAL_BYTES, MAX_PATH_BYTES, MAX_READ_BYTES, MAX_WRITE_BYTES,
    WorkingDirectory, bounded_copy, checked_js_path, copy_file_at_until, entries_json,
    file_info_at, file_info_value, mkdir_at, open_regular_source, read_bounded_until,
    read_dir_at_until, remove_at, rename_at, resolve, resolve_pair, resolve_path,
    snapshot_copy_paths, snapshot_write_text, validate_entry_path, validate_path, worker,
    write_bytes_at,
};

/// Maximum outstanding filesystem operations, including undelivered
/// completions.
const MAX_PENDING: usize = 4;
const WORKERS: usize = 2;
/// Largest owned payload one job may hold: a bounded read result or a
/// bounded write input.
const MAX_OPERATION_BYTES: usize = if MAX_READ_BYTES > MAX_WRITE_BYTES {
    MAX_READ_BYTES
} else {
    MAX_WRITE_BYTES
};
/// Admission charge for the owned metadata snapshot. The three flags, a
/// 20-digit size, and a 20-digit time fit comfortably; this is an accounting
/// bound, not a separate allocation.
const STAT_RESULT_BYTES: usize = 128;
/// Admission charge for the owned listing a worker can return. Entry names are
/// already capped by `MAX_LIST_TOTAL_BYTES`; 128 bytes per entry covers the
/// `String` header, flags, padding, vector slack, and allocator overhead. The
/// owner-side parsed array is neither part of this charge nor covered by a
/// memory bound.
const MAX_LIST_OPERATION_BYTES: usize = MAX_LIST_TOTAL_BYTES + MAX_LIST_ENTRIES * 128;
const LIMITS: worker::Limits = worker::Limits {
    workers: NonZeroUsize::new(WORKERS).unwrap(),
    queued: NonZeroUsize::new(MAX_PENDING).unwrap(),
    pending: NonZeroUsize::new(MAX_PENDING).unwrap(),
    bytes: MAX_PENDING * (MAX_OPERATION_BYTES + MAX_PATH_BYTES),
};

/// Owned work for one worker. A test gate may block the operation
/// deterministically without a slow device or a timing assumption.
struct Job {
    kind: JobKind,
    #[cfg(test)]
    gate: Option<Gate>,
    #[cfg(test)]
    read_dir_gate: Option<ReadDirGate>,
    #[cfg(test)]
    remove_gate: Option<RemoveGate>,
    #[cfg(test)]
    write_text_gate: Option<WriteTextGate>,
    #[cfg(test)]
    rename_gate: Option<RenameGate>,
    #[cfg(test)]
    copy_gate: Option<CopyGate>,
}

enum JobKind {
    Read {
        path: PathBuf,
    },
    Write {
        path: PathBuf,
        data: Vec<u8>,
        overwrite: bool,
    },
    WriteText {
        path: PathBuf,
        data: Vec<u8>,
    },
    Stat {
        path: PathBuf,
        follow: bool,
    },
    ReadDir {
        path: PathBuf,
    },
    Mkdir {
        path: PathBuf,
        recursive: bool,
    },
    Remove {
        path: PathBuf,
        recursive: bool,
    },
    Rename {
        source: PathBuf,
        destination: PathBuf,
    },
    Copy {
        source: PathBuf,
        destination: PathBuf,
    },
}

/// A worker result that still needs an owner-side settlement.
enum Output {
    Read(Vec<u8>),
    Write,
    Stat(FileInfo),
    ReadDir(Vec<DirEntryInfo>),
    Mkdir,
    Remove,
    Rename,
    Copy,
}

type OperationResult = Result<Output, FsError>;
type Pool = worker::Pool<Job, OperationResult>;

/// Owned Rust data is all that reaches this thread.
fn execute(job: Job, cancellation: &worker::Cancellation) -> OperationResult {
    #[cfg(test)]
    if let Some(Gate(gate)) = job.gate {
        gate(cancellation)?;
    }
    if cancellation.requested() {
        return Err(FsError::new(Code::Cancelled));
    }
    match job.kind {
        JobKind::Read { path } => {
            let (file, metadata) = open_regular_source(&path)?;
            let length = usize::try_from(metadata.len()).map_err(|_| FsError::new(Code::Limit))?;
            read_bounded_until(file, length, MAX_READ_BYTES, || cancellation.requested())
                .map(Output::Read)
        }
        JobKind::Write {
            path,
            data,
            overwrite,
        } => {
            write_bytes_at(&path, &data, overwrite, MAX_WRITE_BYTES)?;
            Ok(Output::Write)
        }
        JobKind::WriteText { path, data } => {
            write_bytes_at(&path, &data, true, MAX_WRITE_BYTES)?;
            // Test-only point after the text write effect. A turn failure
            // here proves teardown joins a worker that has already created,
            // truncated, or written the file, so nothing is rolled back.
            #[cfg(test)]
            if let Some(WriteTextGate(gate)) = job.write_text_gate {
                gate(cancellation)?;
            }
            Ok(Output::Write)
        }
        JobKind::Stat { path, follow } => file_info_at(&path, follow).map(Output::Stat),
        JobKind::ReadDir { path } => {
            #[cfg(test)]
            let mut read_dir_gate = job.read_dir_gate;
            read_dir_at_until(
                &path,
                MAX_LIST_ENTRIES,
                MAX_LIST_NAME_BYTES,
                MAX_LIST_TOTAL_BYTES,
                || cancellation.requested(),
                #[cfg(test)]
                |entry| match read_dir_gate.as_mut() {
                    Some(ReadDirGate(gate)) => gate(entry, cancellation),
                    None => Ok(()),
                },
            )
            .map(Output::ReadDir)
        }
        JobKind::Mkdir { path, recursive } => {
            mkdir_at(&path, recursive)?;
            Ok(Output::Mkdir)
        }
        JobKind::Remove { path, recursive } => {
            remove_at(&path, recursive)?;
            // Test-only point after the deletion effect has happened. A turn
            // failure here proves teardown joins a worker that has already
            // changed the filesystem, so nothing is rolled back.
            #[cfg(test)]
            if let Some(RemoveGate(gate)) = job.remove_gate {
                gate(cancellation)?;
            }
            Ok(Output::Remove)
        }
        JobKind::Rename {
            source,
            destination,
        } => {
            rename_at(&source, &destination)?;
            // Test-only point after the rename effect. A turn failure here
            // proves teardown joins a worker that has already moved the
            // entry, so nothing is rolled back.
            #[cfg(test)]
            if let Some(RenameGate(gate)) = job.rename_gate {
                gate(cancellation)?;
            }
            Ok(Output::Rename)
        }
        JobKind::Copy {
            source,
            destination,
        } => {
            copy_file_at_until(&source, &destination, || cancellation.requested())?;
            // Test-only point after the copy effect. A turn failure here
            // proves teardown joins a worker that has already created and
            // filled the destination, so nothing is rolled back.
            #[cfg(test)]
            if let Some(CopyGate(gate)) = job.copy_gate {
                gate(cancellation)?;
            }
            Ok(Output::Copy)
        }
    }
}

/// Which value a completion resolves with. The owner records this before the
/// job is submitted, so no result type needs to carry a JavaScript value back.
#[derive(Clone, Copy)]
enum SettlementKind {
    Read,
    Write,
    Stat,
    ReadDir,
    Mkdir,
    Remove,
    Rename,
    Copy,
}

struct Settlement {
    kind: SettlementKind,
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

    fn finish(self, ctx: &Ctx<'_>, result: OperationResult) -> JsResult<()> {
        let output = match result {
            Ok(output) => output,
            Err(error) => return self.reject(ctx, error.code),
        };
        match (self.kind, output) {
            (SettlementKind::Read, Output::Read(bytes)) => self.finish_read(ctx, bytes),
            (SettlementKind::Write, Output::Write) => self.finish_void(ctx),
            (SettlementKind::Mkdir, Output::Mkdir) => self.finish_void(ctx),
            (SettlementKind::Remove, Output::Remove) => self.finish_void(ctx),
            (SettlementKind::Rename, Output::Rename) => self.finish_void(ctx),
            (SettlementKind::Copy, Output::Copy) => self.finish_void(ctx),
            (SettlementKind::Stat, Output::Stat(info)) => self.finish_stat(ctx, info),
            (SettlementKind::ReadDir, Output::ReadDir(entries)) => {
                self.finish_read_dir(ctx, entries)
            }
            // A mismatched completion is an internal failure. Reject instead
            // of resolving a promise with a value the caller did not request.
            _ => self.reject(ctx, Code::Io),
        }
    }

    fn finish_read(self, ctx: &Ctx<'_>, bytes: Vec<u8>) -> JsResult<()> {
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

    /// Resolve an operation that has no value: a write, a directory
    /// creation, or a deletion.
    fn finish_void(self, ctx: &Ctx<'_>) -> JsResult<()> {
        self.resolve.restore(ctx)?.call::<_, ()>((Undefined,))
    }

    /// Settle with an owned metadata snapshot. The value has no prototype, so
    /// promise resolution cannot invoke a script-defined `then`.
    fn finish_stat(self, ctx: &Ctx<'_>, info: FileInfo) -> JsResult<()> {
        let resolve = self.resolve.clone().restore(ctx)?;
        let value = match file_info_value(ctx, &info) {
            Ok(value) => value,
            Err(JsError::Exception) => return self.reject.restore(ctx)?.call((ctx.catch(),)),
            Err(error) => return Err(error),
        };
        resolve.call::<_, ()>((value,))
    }

    /// Settle with a directory listing parsed on the owner. QuickJS's JSON
    /// parser creates own array indices without running script. The own `then`
    /// guards only this initial Promise resolution; it is removed immediately
    /// after and is not general thenable assimilation, so later `.then` calls
    /// observe the normal `Array.prototype`.
    fn finish_read_dir(self, ctx: &Ctx<'_>, entries: Vec<DirEntryInfo>) -> JsResult<()> {
        let resolve = self.resolve.clone().restore(ctx)?;
        let value = match ctx.json_parse(entries_json(entries)) {
            Ok(value) => value,
            Err(JsError::Exception) => return self.reject.restore(ctx)?.call((ctx.catch(),)),
            Err(error) => return Err(error),
        };
        let Some(object) = value.as_object().cloned() else {
            return self.reject(ctx, Code::Io);
        };
        if let Err(error) = object.prop("then", Property::from(Undefined).configurable()) {
            return if matches!(error, JsError::Exception) {
                self.reject.restore(ctx)?.call((ctx.catch(),))
            } else {
                Err(error)
            };
        }
        resolve.call::<_, ()>((value,))?;
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
/// borrowed. An inactive driver cannot allocate a pool or dispatch work.
#[derive(Clone, Default)]
pub(crate) struct FsDriver {
    active: Rc<RefCell<Option<Active>>>,
    #[cfg(test)]
    gates: Rc<RefCell<std::collections::VecDeque<Gate>>>,
    #[cfg(test)]
    read_dir_gates: Rc<RefCell<std::collections::VecDeque<ReadDirGate>>>,
    #[cfg(test)]
    remove_gates: Rc<RefCell<std::collections::VecDeque<RemoveGate>>>,
    #[cfg(test)]
    write_text_gates: Rc<RefCell<std::collections::VecDeque<WriteTextGate>>>,
    #[cfg(test)]
    rename_gates: Rc<RefCell<std::collections::VecDeque<RenameGate>>>,
    #[cfg(test)]
    copy_gates: Rc<RefCell<std::collections::VecDeque<CopyGate>>>,
}

impl FsDriver {
    /// Begin a managed module evaluation. The guard always stops its workers.
    pub(crate) fn begin(&self) -> FsTurn<'_> {
        assert!(self.active.borrow().is_none());
        *self.active.borrow_mut() = Some(Active::default());
        FsTurn(self)
    }

    fn submit<'js>(
        &self,
        ctx: &Ctx<'js>,
        bytes: usize,
        settlement_kind: SettlementKind,
        make_error: Function<'js>,
        make_job: impl FnOnce() -> Result<JobKind, FsError>,
    ) -> JsResult<Promise<'js>> {
        // Allocate JS handles before borrowing the table. No job is submitted
        // until the corresponding settlement is fully owned by the table.
        let (promise, resolve, reject) = Promise::new(ctx)?;
        let settlement = Settlement {
            kind: settlement_kind,
            resolve: Persistent::save(ctx, resolve),
            reject: Persistent::save(ctx, reject),
            make_error: Persistent::save(ctx, make_error),
        };
        let result = self.dispatch(bytes, settlement, make_job);
        if let Err((code, settlement)) = result {
            settlement.reject(ctx, code)?;
        }
        Ok(promise)
    }

    fn submit_read<'js>(
        &self,
        ctx: &Ctx<'js>,
        path: PathBuf,
        make_error: Function<'js>,
    ) -> JsResult<Promise<'js>> {
        let bytes = MAX_READ_BYTES + path.as_os_str().as_encoded_bytes().len();
        self.submit(ctx, bytes, SettlementKind::Read, make_error, move || {
            Ok(JobKind::Read { path })
        })
    }

    fn submit_write<'js>(
        &self,
        ctx: &Ctx<'js>,
        path: PathBuf,
        data: &[u8],
        overwrite: bool,
        make_error: Function<'js>,
    ) -> JsResult<Promise<'js>> {
        let bytes = data.len() + path.as_os_str().as_encoded_bytes().len();
        self.submit(ctx, bytes, SettlementKind::Write, make_error, move || {
            // Admission succeeded before this owned copy, so a full pool
            // rejects without reserving another write-sized buffer.
            let data = bounded_copy(data, MAX_WRITE_BYTES)?;
            Ok(JobKind::Write {
                path,
                data,
                overwrite,
            })
        })
    }

    fn submit_write_text<'js>(
        &self,
        ctx: &Ctx<'js>,
        path: PathBuf,
        data: String,
        make_error: Function<'js>,
    ) -> JsResult<Promise<'js>> {
        let bytes = data.len() + path.as_os_str().as_encoded_bytes().len();
        self.submit(ctx, bytes, SettlementKind::Write, make_error, move || {
            // The snapshot already owns the bounded UTF-8 bytes, so admission
            // failure drops it without reserving a second write-sized buffer.
            Ok(JobKind::WriteText {
                path,
                data: data.into_bytes(),
            })
        })
    }

    fn submit_stat<'js>(
        &self,
        ctx: &Ctx<'js>,
        path: PathBuf,
        follow: bool,
        make_error: Function<'js>,
    ) -> JsResult<Promise<'js>> {
        let bytes = STAT_RESULT_BYTES + path.as_os_str().as_encoded_bytes().len();
        self.submit(ctx, bytes, SettlementKind::Stat, make_error, move || {
            Ok(JobKind::Stat { path, follow })
        })
    }

    fn submit_read_dir<'js>(
        &self,
        ctx: &Ctx<'js>,
        path: PathBuf,
        make_error: Function<'js>,
    ) -> JsResult<Promise<'js>> {
        let bytes = MAX_LIST_OPERATION_BYTES + path.as_os_str().as_encoded_bytes().len();
        self.submit(ctx, bytes, SettlementKind::ReadDir, make_error, move || {
            Ok(JobKind::ReadDir { path })
        })
    }

    fn submit_mkdir<'js>(
        &self,
        ctx: &Ctx<'js>,
        path: PathBuf,
        recursive: bool,
        make_error: Function<'js>,
    ) -> JsResult<Promise<'js>> {
        let bytes = path.as_os_str().as_encoded_bytes().len();
        self.submit(ctx, bytes, SettlementKind::Mkdir, make_error, move || {
            Ok(JobKind::Mkdir { path, recursive })
        })
    }

    fn submit_remove<'js>(
        &self,
        ctx: &Ctx<'js>,
        path: PathBuf,
        recursive: bool,
        make_error: Function<'js>,
    ) -> JsResult<Promise<'js>> {
        let bytes = path.as_os_str().as_encoded_bytes().len();
        self.submit(ctx, bytes, SettlementKind::Remove, make_error, move || {
            Ok(JobKind::Remove { path, recursive })
        })
    }

    fn submit_rename<'js>(
        &self,
        ctx: &Ctx<'js>,
        source: PathBuf,
        destination: PathBuf,
        make_error: Function<'js>,
    ) -> JsResult<Promise<'js>> {
        // Charge both resolved paths; a rename owns two pathnames.
        let bytes = source.as_os_str().as_encoded_bytes().len()
            + destination.as_os_str().as_encoded_bytes().len();
        self.submit(ctx, bytes, SettlementKind::Rename, make_error, move || {
            Ok(JobKind::Rename {
                source,
                destination,
            })
        })
    }

    fn submit_copy<'js>(
        &self,
        ctx: &Ctx<'js>,
        source: PathBuf,
        destination: PathBuf,
        make_error: Function<'js>,
    ) -> JsResult<Promise<'js>> {
        // Charge both resolved paths; a copy owns two pathnames.
        let bytes = source.as_os_str().as_encoded_bytes().len()
            + destination.as_os_str().as_encoded_bytes().len();
        self.submit(ctx, bytes, SettlementKind::Copy, make_error, move || {
            Ok(JobKind::Copy {
                source,
                destination,
            })
        })
    }

    fn dispatch(
        &self,
        bytes: usize,
        settlement: Settlement,
        make_job: impl FnOnce() -> Result<JobKind, FsError>,
    ) -> Result<(), (Code, Settlement)> {
        let mut active = self.active.borrow_mut();
        let Some(active) = active.as_mut() else {
            return Err((Code::AsyncOutsideTurn, settlement));
        };
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
        let kind = match make_job() {
            Ok(kind) => kind,
            Err(error) => return Err((error.code, settlement)),
        };
        active.next = next;
        active.pending.insert(id, settlement);
        #[cfg(test)]
        let read_dir_gate = if matches!(kind, JobKind::ReadDir { .. }) {
            self.read_dir_gates.borrow_mut().pop_front()
        } else {
            None
        };
        #[cfg(test)]
        let remove_gate = if matches!(kind, JobKind::Remove { .. }) {
            self.remove_gates.borrow_mut().pop_front()
        } else {
            None
        };
        #[cfg(test)]
        let write_text_gate = if matches!(kind, JobKind::WriteText { .. }) {
            self.write_text_gates.borrow_mut().pop_front()
        } else {
            None
        };
        #[cfg(test)]
        let rename_gate = if matches!(kind, JobKind::Rename { .. }) {
            self.rename_gates.borrow_mut().pop_front()
        } else {
            None
        };
        #[cfg(test)]
        let copy_gate = if matches!(kind, JobKind::Copy { .. }) {
            self.copy_gates.borrow_mut().pop_front()
        } else {
            None
        };
        admission.send(
            id,
            Job {
                kind,
                #[cfg(test)]
                gate: self.gates.borrow_mut().pop_front(),
                #[cfg(test)]
                read_dir_gate,
                #[cfg(test)]
                remove_gate,
                #[cfg(test)]
                write_text_gate,
                #[cfg(test)]
                rename_gate,
                #[cfg(test)]
                copy_gate,
            },
        );
        Ok(())
    }
}

pub(crate) struct FsTurn<'a>(&'a FsDriver);

pub(crate) struct FsCompletion(Settlement, OperationResult);

impl FsCompletion {
    /// Settle on the owner, with neither the pool nor its owner table locked.
    pub(crate) fn deliver(self, ctx: &Ctx<'_>) -> JsResult<()> {
        self.0.finish(ctx, self.1)
    }
}

pub(crate) struct CancelledOperations(BTreeMap<u64, Settlement>);

impl CancelledOperations {
    /// Reject abandoned work without executing its reactions.
    pub(crate) fn reject(self, ctx: &Ctx<'_>) {
        for (_, settlement) in self.0 {
            if settlement.reject(ctx, Code::Cancelled).is_err() {
                ctx.catch();
            }
        }
    }
}

impl FsTurn<'_> {
    /// Count accepted operations still awaiting delivery on the owner.
    pub(crate) fn has_pending(&self) -> bool {
        !self.0.active.borrow().as_ref().unwrap().pending.is_empty()
    }

    /// Receive one completion, releasing its budget before settling it.
    /// Pass zero to poll while QuickJS has runnable jobs.
    pub(crate) fn receive(&self, timeout: Option<Duration>) -> Option<FsCompletion> {
        let mut active = self.0.active.borrow_mut();
        let active = active.as_mut().unwrap();
        let completion = active.pool.as_mut()?.receive(timeout)?;
        let result = match completion.outcome {
            worker::Outcome::Finished(result) => result,
            worker::Outcome::Panicked => Err(FsError::new(Code::Io)),
        };
        Some(FsCompletion(
            active.pending.remove(&completion.id).unwrap(),
            result,
        ))
    }

    /// Stop dispatch and join workers without entering QuickJS.
    pub(crate) fn cancel(&self) -> CancelledOperations {
        let Active { pool, pending, .. } = self.0.active.borrow_mut().take().unwrap();
        drop(pool);
        CancelledOperations(pending)
    }
}

impl Drop for FsTurn<'_> {
    fn drop(&mut self) {
        let active = self.0.active.borrow_mut().take();
        drop(active);
    }
}

fn snapshot_absolute_path(
    cwd: &Rc<RefCell<WorkingDirectory>>,
    path: JsString<'_>,
) -> Result<PathBuf, FsError> {
    let path = checked_js_path(path, MAX_PATH_BYTES)?;
    let path = resolve(cwd, &path)?;
    if !path.is_absolute() {
        return Err(FsError::new(Code::Path));
    }
    validate_path(path.to_str().ok_or(FsError::new(Code::Encoding))?)?;
    Ok(path)
}

/// Snapshot an absolute path that must name a specific directory entry.
///
/// The wrapper already rejected roots, trailing separators, and final `.` or
/// `..` components before resolution. Re-checking the resolved path keeps that
/// rule load-bearing for deletion even if the private native callback is ever
/// reached by a different caller.
fn snapshot_absolute_entry_path(
    cwd: &Rc<RefCell<WorkingDirectory>>,
    path: JsString<'_>,
) -> Result<PathBuf, FsError> {
    let path = checked_js_path(path, MAX_PATH_BYTES)?;
    let path = validate_entry_path(&path)?;
    let path = resolve_path(&cwd.borrow(), path)?;
    if !path.is_absolute() {
        return Err(FsError::new(Code::Path));
    }
    Ok(path)
}

/// Snapshot two entry paths against one runtime working-directory snapshot.
///
/// Both operands are validated before either is resolved, matching the
/// synchronous form's error precedence. A later `chdir` cannot separate the
/// pair, because the same immutable cwd borrow resolves both.
fn snapshot_absolute_entry_pair(
    cwd: &Rc<RefCell<WorkingDirectory>>,
    source: JsString<'_>,
    destination: JsString<'_>,
) -> Result<(PathBuf, PathBuf), FsError> {
    let source = checked_js_path(source, MAX_PATH_BYTES)?;
    let source = validate_entry_path(&source)?;
    let destination = checked_js_path(destination, MAX_PATH_BYTES)?;
    let destination = validate_entry_path(&destination)?;
    let (source, destination) = resolve_pair(cwd, source, destination)?;
    if !source.is_absolute() || !destination.is_absolute() {
        return Err(FsError::new(Code::Path));
    }
    Ok((source, destination))
}

/// Snapshot a copy pair through the same pure path helper as the synchronous
/// form. Entry validation and cwd resolution stay in production code, so the
/// tests exercise this exact ordering rather than a copy of it.
fn snapshot_absolute_copy_pair(
    cwd: &Rc<RefCell<WorkingDirectory>>,
    source: JsString<'_>,
    destination: JsString<'_>,
) -> Result<(PathBuf, PathBuf), FsError> {
    let source = checked_js_path(source, MAX_PATH_BYTES)?;
    let source = validate_entry_path(&source)?;
    let destination = checked_js_path(destination, MAX_PATH_BYTES)?;
    let destination = validate_entry_path(&destination)?;
    snapshot_copy_paths(&cwd.borrow(), source, destination)
}

fn throw_argument<'js>(
    ctx: &Ctx<'js>,
    error: FsError,
    make_error: Function<'js>,
) -> JsResult<Promise<'js>> {
    let reason: Value = make_error.call((error.code(),))?;
    Err(ctx.throw(reason))
}

pub(super) fn install<'js>(
    ctx: &Ctx<'js>,
    native: &Object<'js>,
    driver: FsDriver,
    cwd: Rc<RefCell<WorkingDirectory>>,
) -> JsResult<()> {
    let check = driver.clone();
    native.set(
        "canAsync",
        Function::new(ctx.clone(), move || check.active.borrow().is_some())?,
    )?;

    let write_text_driver = driver.clone();
    let write_text_cwd = Rc::clone(&cwd);
    native.set(
        "writeText",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>,
                  path: JsString<'js>,
                  data: JsString<'js>,
                  make_error: Function<'js>| {
                // Snapshot and validate the text before resolving the path,
                // preserving the synchronous form's error precedence. Both
                // owned values are captured before dispatch.
                let result = snapshot_write_text(&data, MAX_WRITE_BYTES).and_then(|data| {
                    let path = snapshot_absolute_path(&write_text_cwd, path)?;
                    Ok((path, data))
                });
                match result {
                    Ok((path, data)) => {
                        write_text_driver.submit_write_text(&ctx, path, data, make_error)
                    }
                    Err(error) => throw_argument(&ctx, error, make_error),
                }
            },
        )?,
    )?;

    let read_driver = driver.clone();
    let read_cwd = Rc::clone(&cwd);
    native.set(
        "readFile",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, path: JsString<'js>, make_error: Function<'js>| {
                match snapshot_absolute_path(&read_cwd, path) {
                    Ok(path) => read_driver.submit_read(&ctx, path, make_error),
                    Err(error) => throw_argument(&ctx, error, make_error),
                }
            },
        )?,
    )?;

    let write_driver = driver.clone();
    let stat_base_cwd = Rc::clone(&cwd);
    native.set(
        "writeFile",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>,
                  path: JsString<'js>,
                  data: TypedArray<'js, u8>,
                  overwrite: bool,
                  make_error: Function<'js>| {
                // SAFETY: the private wrapper passes the fresh snapshot it
                // obtained from `copyData`, never the caller's original view.
                // The borrow ends before any JavaScript can run again.
                let result = snapshot_absolute_path(&cwd, path).and_then(|path| {
                    let bytes = unsafe { data.as_bytes() }.ok_or(FsError::new(Code::Io))?;
                    if bytes.len() > MAX_WRITE_BYTES {
                        return Err(FsError::new(Code::Limit));
                    }
                    Ok((path, bytes))
                });
                match result {
                    Ok((path, bytes)) => {
                        write_driver.submit_write(&ctx, path, bytes, overwrite, make_error)
                    }
                    Err(error) => throw_argument(&ctx, error, make_error),
                }
            },
        )?,
    )?;

    for (name, follow) in [("stat", true), ("lstat", false)] {
        let stat_driver = driver.clone();
        let stat_cwd = Rc::clone(&stat_base_cwd);
        native.set(
            name,
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'js>, path: JsString<'js>, make_error: Function<'js>| {
                    match snapshot_absolute_path(&stat_cwd, path) {
                        Ok(path) => stat_driver.submit_stat(&ctx, path, follow, make_error),
                        Err(error) => throw_argument(&ctx, error, make_error),
                    }
                },
            )?,
        )?;
    }

    let read_dir_driver = driver.clone();
    let read_dir_cwd = Rc::clone(&stat_base_cwd);
    native.set(
        "readDir",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, path: JsString<'js>, make_error: Function<'js>| {
                match snapshot_absolute_path(&read_dir_cwd, path) {
                    Ok(path) => read_dir_driver.submit_read_dir(&ctx, path, make_error),
                    Err(error) => throw_argument(&ctx, error, make_error),
                }
            },
        )?,
    )?;

    let mkdir_driver = driver.clone();
    let mkdir_cwd = Rc::clone(&stat_base_cwd);
    native.set(
        "mkdir",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>,
                  path: JsString<'js>,
                  recursive: bool,
                  make_error: Function<'js>| {
                match snapshot_absolute_path(&mkdir_cwd, path) {
                    Ok(path) => mkdir_driver.submit_mkdir(&ctx, path, recursive, make_error),
                    Err(error) => throw_argument(&ctx, error, make_error),
                }
            },
        )?,
    )?;

    let rename_driver = driver.clone();
    let rename_cwd = Rc::clone(&stat_base_cwd);
    native.set(
        "rename",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>,
                  source: JsString<'js>,
                  destination: JsString<'js>,
                  make_error: Function<'js>| {
                match snapshot_absolute_entry_pair(&rename_cwd, source, destination) {
                    Ok((source, destination)) => {
                        rename_driver.submit_rename(&ctx, source, destination, make_error)
                    }
                    Err(error) => throw_argument(&ctx, error, make_error),
                }
            },
        )?,
    )?;

    let copy_driver = driver.clone();
    let copy_cwd = Rc::clone(&stat_base_cwd);
    native.set(
        "copyFile",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>,
                  source: JsString<'js>,
                  destination: JsString<'js>,
                  make_error: Function<'js>| {
                match snapshot_absolute_copy_pair(&copy_cwd, source, destination) {
                    Ok((source, destination)) => {
                        copy_driver.submit_copy(&ctx, source, destination, make_error)
                    }
                    Err(error) => throw_argument(&ctx, error, make_error),
                }
            },
        )?,
    )?;

    let remove_driver = driver;
    let remove_cwd = stat_base_cwd;
    native.set(
        "remove",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>,
                  path: JsString<'js>,
                  recursive: bool,
                  make_error: Function<'js>| {
                match snapshot_absolute_entry_path(&remove_cwd, path) {
                    Ok(path) => remove_driver.submit_remove(&ctx, path, recursive, make_error),
                    Err(error) => throw_argument(&ctx, error, make_error),
                }
            },
        )?,
    )?;
    Ok(())
}

#[cfg(test)]
struct Gate(Box<GateFn>);

#[cfg(test)]
type GateFn = dyn FnOnce(&worker::Cancellation) -> Result<(), FsError> + Send;

/// Test-only pause after one enumerated entry, so cancellation is observed at
/// a known mid-traversal point rather than only before dispatch.
#[cfg(test)]
struct ReadDirGate(Box<ReadDirGateFn>);

#[cfg(test)]
type ReadDirGateFn = dyn FnMut(&DirEntryInfo, &worker::Cancellation) -> Result<(), FsError> + Send;

/// Test-only pause after `remove_at` has returned, so a deletion effect is
/// real before the gate blocks. Production builds have no such field or call.
#[cfg(test)]
struct RemoveGate(Box<RemoveGateFn>);

#[cfg(test)]
type RemoveGateFn = dyn FnOnce(&worker::Cancellation) -> Result<(), FsError> + Send;

/// Test-only pause after `write_bytes_at` has returned for a text write, so
/// the file effect is real before the gate blocks. Production builds have no
/// such field or call.
#[cfg(test)]
struct WriteTextGate(Box<WriteTextGateFn>);

#[cfg(test)]
type WriteTextGateFn = dyn FnOnce(&worker::Cancellation) -> Result<(), FsError> + Send;

/// Test-only pause after `rename_at` has returned, so the rename effect is
/// real before the gate blocks. Production builds have no such field or call.
#[cfg(test)]
struct RenameGate(Box<RenameGateFn>);

#[cfg(test)]
type RenameGateFn = dyn FnOnce(&worker::Cancellation) -> Result<(), FsError> + Send;

/// Test-only pause after `copy_file_at_until` has returned, so the copied
/// destination and its permissions are real before the gate blocks.
/// Production builds have no such field or call.
#[cfg(test)]
struct CopyGate(Box<CopyGateFn>);

#[cfg(test)]
type CopyGateFn = dyn FnOnce(&worker::Cancellation) -> Result<(), FsError> + Send;

#[cfg(test)]
mod tests;
