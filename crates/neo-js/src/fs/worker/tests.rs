//! These behavioral tests exercise private worker lifecycle and admission
//! branches that have no public runtime path yet.

use super::*;
use std::{
    cell::RefCell,
    panic::panic_any,
    sync::mpsc::{self, Receiver, Sender},
};

const TIMEOUT: Duration = Duration::from_secs(10);

fn limits(workers: usize, queued: usize, pending: usize, bytes: usize) -> Limits {
    Limits {
        workers: NonZeroUsize::new(workers).unwrap(),
        queued: NonZeroUsize::new(queued).unwrap(),
        pending: NonZeroUsize::new(pending).unwrap(),
        bytes,
    }
}

fn submit<T, U>(pool: &mut Pool<T, U>, id: u64, bytes: usize, input: T) {
    pool.reserve(bytes).unwrap().send(id, input);
}

struct Job {
    started: Sender<thread::ThreadId>,
    release: Receiver<()>,
    result: u64,
}

fn gated_job(result: u64) -> (Job, Receiver<thread::ThreadId>, Sender<()>) {
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    (
        Job {
            started: started_tx,
            release: release_rx,
            result,
        },
        started_rx,
        release_tx,
    )
}

fn execute_gated(job: Job, _: &Cancellation) -> u64 {
    job.started.send(thread::current().id()).unwrap();
    // The controller owns all deadlines. On controller failure its sender
    // drops before the pool joins, so the worker cannot strand teardown.
    let _ = job.release.recv();
    job.result
}

#[test]
fn concurrent_workers_and_queue_saturation() {
    let mut pool = Pool::new(limits(2, 1, 8, 100), execute_gated).unwrap();
    let (first, first_started, first_release) = gated_job(10);
    submit(&mut pool, 1, 1, first);
    let worker_a = first_started.recv_timeout(TIMEOUT).unwrap();
    let (second, second_started, second_release) = gated_job(20);
    submit(&mut pool, 2, 1, second);
    let worker_b = second_started.recv_timeout(TIMEOUT).unwrap();
    assert_ne!(worker_a, worker_b);
    assert_ne!(worker_a, thread::current().id());
    assert_ne!(worker_b, thread::current().id());

    let (third, third_started, third_release) = gated_job(30);
    submit(&mut pool, 3, 1, third);
    assert_eq!(pool.reserve(1).err(), Some(AdmitError::Full));
    assert_eq!(pool.receive(Some(Duration::ZERO)), None);

    second_release.send(()).unwrap();
    assert_eq!(
        pool.receive(Some(TIMEOUT)),
        Some(Completion {
            id: 2,
            outcome: Outcome::Finished(20),
        })
    );
    assert_eq!(third_started.recv_timeout(TIMEOUT).unwrap(), worker_b);
    third_release.send(()).unwrap();
    assert_eq!(pool.receive(Some(TIMEOUT)).unwrap().id, 3);
    first_release.send(()).unwrap();
    assert_eq!(pool.receive(Some(TIMEOUT)).unwrap().id, 1);
    assert_eq!(pool.receive(Some(Duration::ZERO)), None);
}

enum Echo {
    Bytes(Vec<u8>),
    Signal(Sender<()>),
}

fn echo(input: Echo, _: &Cancellation) -> Vec<u8> {
    match input {
        Echo::Bytes(bytes) => bytes,
        Echo::Signal(started) => {
            started.send(()).unwrap();
            Vec::new()
        }
    }
}

#[test]
fn undelivered_completions_hold_pending_and_byte_charges() {
    let mut pool = Pool::new(limits(1, 3, 2, 5), echo).unwrap();
    assert_eq!(pool.reserve(6).err(), Some(AdmitError::TooLarge));
    // An owner can fail snapshot validation and drop admission without leaks.
    {
        let _abandoned = pool.reserve(5).unwrap();
    }

    submit(&mut pool, 1, 5, Echo::Bytes(vec![42; 5]));
    let (started, running) = mpsc::channel();
    submit(&mut pool, 2, 0, Echo::Signal(started));
    // Starting the second job proves the first completion is queued.
    running.recv_timeout(TIMEOUT).unwrap();
    assert_eq!(pool.reserve(0).err(), Some(AdmitError::Full));
    let first = pool.receive(Some(TIMEOUT)).unwrap();
    assert_eq!(first.id, 1);
    assert_eq!(first.outcome, Outcome::Finished(vec![42; 5]));
    submit(&mut pool, 3, 5, Echo::Bytes(vec![7; 5]));
    let second = pool.receive(Some(TIMEOUT)).unwrap();
    assert_eq!(second.id, 2);
    // There is now a job slot but the byte charge still blocks admission.
    assert_eq!(pool.reserve(1).err(), Some(AdmitError::Full));
    assert_eq!(
        pool.receive(Some(TIMEOUT)).unwrap().outcome,
        Outcome::Finished(vec![7; 5])
    );
    assert!(pool.reserve(5).is_ok());
}

#[test]
fn worker_panic_with_panicking_payload_releases_capacity_and_preserves_worker() {
    struct PanicOnDrop {
        _notify: NotifyOnDrop,
    }

    impl Drop for PanicOnDrop {
        fn drop(&mut self) {
            panic!("intentional panic while dropping the caught payload");
        }
    }

    fn execute(input: Result<(), NotifyOnDrop>, _: &Cancellation) -> thread::ThreadId {
        if let Err(dropped) = input {
            panic_any(PanicOnDrop { _notify: dropped });
        }
        thread::current().id()
    }

    let (dropped, payload_dropped) = mpsc::channel();
    let mut pool = Pool::new(limits(1, 1, 1, 10), execute).unwrap();
    submit(&mut pool, 1, 10, Ok(()));
    let Some(Completion {
        outcome: Outcome::Finished(worker),
        ..
    }) = pool.receive(Some(TIMEOUT))
    else {
        panic!("expected the original worker ID");
    };
    submit(&mut pool, 2, 10, Err(NotifyOnDrop(dropped)));
    assert_eq!(pool.reserve(1).err(), Some(AdmitError::Full));
    assert_eq!(
        pool.receive(Some(TIMEOUT)),
        Some(Completion {
            id: 2,
            outcome: Outcome::Panicked,
        })
    );
    assert_eq!(payload_dropped.try_recv(), Ok(()));
    submit(&mut pool, 3, 10, Ok(()));
    assert_eq!(
        pool.receive(Some(TIMEOUT)),
        Some(Completion {
            id: 3,
            outcome: Outcome::Finished(worker),
        })
    );
}

// A dropped queued input opens the shutdown barrier. Merely starting a
// shutdown thread would not prove cancellation was visible to running work.
struct NotifyOnDrop(Sender<()>);

impl Drop for NotifyOnDrop {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

struct Discarded {
    _notify: NotifyOnDrop,
    panic: bool,
}

impl Drop for Discarded {
    fn drop(&mut self) {
        assert!(!self.panic, "intentional panic while discarding a message");
    }
}

thread_local! {
    static ON_WORKER_EXIT: RefCell<Option<NotifyOnDrop>> = const { RefCell::new(None) };
}

enum ShutdownJob {
    Complete(Discarded),
    Running {
        started: Sender<()>,
        queued_dropped: Receiver<()>,
        finished: Sender<bool>,
        result: Discarded,
        worker_exit: NotifyOnDrop,
    },
    Queued {
        _discarded: Discarded,
    },
}

fn execute_shutdown(job: ShutdownJob, cancel: &Cancellation) -> Discarded {
    match job {
        ShutdownJob::Complete(result) => result,
        ShutdownJob::Running {
            started,
            queued_dropped,
            finished,
            result,
            worker_exit,
        } => {
            ON_WORKER_EXIT.set(Some(worker_exit));
            started.send(()).unwrap();
            queued_dropped.recv_timeout(TIMEOUT).unwrap();
            finished.send(cancel.requested()).unwrap();
            result
        }
        ShutdownJob::Queued { .. } => panic!("shutdown must not run this request"),
    }
}

#[test]
fn drop_discards_pending_messages_signals_running_work_and_joins() {
    verify_drop(false);
}

#[test]
fn drop_during_unwind_contains_discard_panics_and_joins_running_worker() {
    verify_drop(true);
}

fn verify_drop(panicking: bool) {
    let mut pool = Pool::new(limits(1, 2, 4, 10), execute_shutdown).unwrap();
    let (ready_dropped, ready) = mpsc::channel();
    submit(
        &mut pool,
        1,
        0,
        ShutdownJob::Complete(Discarded {
            _notify: NotifyOnDrop(ready_dropped),
            panic: panicking,
        }),
    );
    let (started, running) = mpsc::channel();
    let (queued, queued_dropped) = mpsc::channel();
    let (finished, cancelled) = mpsc::channel();
    let (result_dropped, result) = mpsc::channel();
    let (exit, exited) = mpsc::channel();
    submit(
        &mut pool,
        2,
        0,
        ShutdownJob::Running {
            started,
            queued_dropped,
            finished,
            result: Discarded {
                _notify: NotifyOnDrop(result_dropped),
                panic: panicking,
            },
            worker_exit: NotifyOnDrop(exit),
        },
    );
    running.recv_timeout(TIMEOUT).unwrap();
    submit(
        &mut pool,
        3,
        0,
        ShutdownJob::Queued {
            _discarded: Discarded {
                _notify: NotifyOnDrop(queued),
                panic: panicking,
            },
        },
    );
    // Prove disposal continues after the first queued destructor panics.
    let (later_dropped, later) = mpsc::channel();
    submit(
        &mut pool,
        4,
        0,
        ShutdownJob::Queued {
            _discarded: Discarded {
                _notify: NotifyOnDrop(later_dropped),
                panic: panicking,
            },
        },
    );
    if panicking {
        let original = catch_unwind(AssertUnwindSafe(move || {
            let _pool = pool;
            panic!("original owner panic");
        }))
        .unwrap_err();
        assert_eq!(
            original.downcast_ref::<&str>(),
            Some(&"original owner panic")
        );
    } else {
        drop(pool);
    }
    // No blocking reads after drop: outputs must already be destroyed and
    // the running worker must have seen the cancellation before returning.
    assert_eq!(ready.try_recv(), Ok(()));
    assert_eq!(cancelled.try_recv(), Ok(true));
    assert_eq!(result.try_recv(), Ok(()));
    assert_eq!(exited.try_recv(), Ok(()));
    assert_eq!(later.try_recv(), Ok(()));
}

#[test]
fn idle_and_closed_pool_return_without_waiting() {
    let mut pool = Pool::new(limits(2, 1, 1, 0), |(), _| ()).unwrap();
    assert!(pool.receive(Some(TIMEOUT)).is_none());
    pool.shutdown();
    pool.shutdown();
    assert_eq!(pool.reserve(0).err(), Some(AdmitError::Closed));
    assert!(pool.receive(Some(TIMEOUT)).is_none());
}
