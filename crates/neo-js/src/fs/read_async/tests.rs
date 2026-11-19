//! The worker barrier is private so these integration-style owner-turn
//! tests can prove ordering without a slow device or a timing assumption.

use std::{fs, sync::mpsc, thread};

use crate::{NeoJs, Permissions};

use super::*;

const TIMEOUT: Duration = Duration::from_secs(10);

struct WorkerExit(mpsc::Sender<bool>, bool);

impl Drop for WorkerExit {
    fn drop(&mut self) {
        let _ = self.0.send(self.1);
    }
}

thread_local! {
    static WORKER_EXIT: RefCell<Option<WorkerExit>> = const { RefCell::new(None) };
}

fn runtime() -> NeoJs {
    NeoJs::new(Permissions {
        filesystem: true,
        stdio: true,
        ..Permissions::default()
    })
    .unwrap()
}

fn literal(path: &std::path::Path) -> String {
    serde_json::to_string(path.to_str().unwrap()).unwrap()
}

#[test]
fn a_js_job_and_another_read_complete_while_first_worker_is_blocked() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("data");
    fs::write(&file, [42, 7]).unwrap();
    let js = runtime();
    let (started_tx, started) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    js.reads
        .gates
        .borrow_mut()
        .push_back(ReadGate(Box::new(move |_| {
            started_tx.send(()).unwrap();
            release_rx.recv_timeout(TIMEOUT).unwrap();
            Ok(())
        })));
    let (progress, progress_rx) = mpsc::channel();
    js.context.with(|ctx| {
        let progress = Function::new(ctx.clone(), move |phase: u8| {
            progress.send(phase).unwrap();
        })
        .unwrap();
        ctx.globals().set("progress", progress).unwrap();
    });
    let controller = thread::spawn(move || {
        started.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(progress_rx.recv_timeout(TIMEOUT).unwrap(), 1);
        assert_eq!(progress_rx.recv_timeout(TIMEOUT).unwrap(), 2);
        release.send(()).unwrap();
    });
    let result = js.run_module(
        "progress.js",
        &format!(
            r#"import {{readFile}} from 'neo:fs';
            let done = false;
            const first = readFile({file}).then(bytes => {{
                done = true;
                if (bytes[0] !== 42) throw Error('first read bytes');
            }});
            await Promise.resolve();
            if (done) throw Error('first finished before release');
            progress(1);
            const second = await fs.readFile({file});
            if (!(second instanceof Uint8Array) || second[1] !== 7 || done)
                throw Error('second did not finish independently');
            progress(2);
            await first;"#,
            file = literal(&file),
        ),
    );
    controller.join().unwrap();
    result.unwrap();
}

#[test]
fn outstanding_reads_are_bounded_until_delivered() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("fixture");
    fs::write(&file, b"ok").unwrap();
    let js = runtime();
    js.run_module(
        "saturation.js",
        &format!(
            r#"import {{readFile, FsError}} from 'neo:fs';
            const reads = [];
            for (let i = 0; i < {limit}; i++) reads.push(readFile({file}));
            try {{
                await readFile({file});
                throw Error('admitted one read beyond the bound');
            }} catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_LIMIT') throw error;
            }}
            for (const data of await Promise.all(reads)) {{
                if (data[0] !== 111) throw Error('missing response');
            }}
            if ((await readFile({file}))[1] !== 107) throw Error('slot did not recover');"#,
            file = literal(&file),
            limit = MAX_PENDING,
        ),
    )
    .unwrap();
}

#[test]
fn failing_turn_cancels_and_joins_before_refusing_followon_execution() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("fixture");
    fs::write(&file, b"ok").unwrap();
    let js = runtime();
    let (started_tx, started) = mpsc::channel();
    let (joined_tx, joined) = mpsc::channel();
    let (third_tx, third_started) = mpsc::channel();
    let mut release = Vec::new();
    {
        let mut gates = js.reads.gates.borrow_mut();
        for _ in 0..2 {
            let (send, wait) = mpsc::channel::<()>();
            release.push(send);
            let started_tx = started_tx.clone();
            let joined_tx = joined_tx.clone();
            gates.push_back(ReadGate(Box::new(move |cancel| {
                started_tx.send(()).unwrap();
                // The queued third request owns both senders. It is discarded
                // after cancellation is set, while both workers are blocked.
                assert_eq!(
                    wait.recv_timeout(TIMEOUT),
                    Err(mpsc::RecvTimeoutError::Disconnected)
                );
                joined_tx.send(cancel.requested()).unwrap();
                Ok(())
            })));
        }
        gates.push_back(ReadGate(Box::new(move |_| {
            drop(release);
            third_tx.send(()).unwrap();
            Ok(())
        })));
    }
    js.context.with(|ctx| {
        let wait = Function::new(ctx.clone(), move || {
            started.recv_timeout(TIMEOUT).unwrap();
            started.recv_timeout(TIMEOUT).unwrap();
        })
        .unwrap();
        ctx.globals().set("finishRead", wait).unwrap();
    });
    let result = js.run_module(
        "cancel.js",
        &format!(
            "globalThis.cancelledRead = fs.readFile({file});
             fs.readFile({file}); fs.readFile({file}); finishRead(); throw Error('entry failed');",
            file = literal(&file),
        ),
    );
    assert!(result.unwrap_err().contains("entry failed"));
    assert_eq!(joined.try_recv(), Ok(true));
    assert_eq!(joined.try_recv(), Ok(true));
    assert_eq!(
        third_started.try_recv(),
        Err(mpsc::TryRecvError::Disconnected)
    );
    assert!(js.reads.active.borrow().is_none());
    // Inspect the promise directly here: normal embedder execution after an
    // abandoned turn is denied, so no canceled reaction can run later.
    js.context.with(|ctx| {
        let promise: Promise = ctx.globals().get("cancelledRead").unwrap();
        assert!(matches!(
            promise.result::<()>(),
            Some(Err(JsError::Exception))
        ));
        let reason = ctx.catch().into_object().unwrap();
        assert_eq!(reason.get::<_, String>("code").unwrap(), "FS_CANCELLED");
    });
    assert!(js.run("1").unwrap_err().contains("NEO_MODULE_ABORTED"));
    drop(js);
}

#[test]
fn an_owner_panic_after_admission_joins_workers_and_poisons_the_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("fixture");
    fs::write(&file, b"ok").unwrap();
    let js = runtime();
    let (started_tx, started) = mpsc::channel();
    let (exited_tx, exited) = mpsc::channel();
    let (release, wait) = mpsc::channel::<()>();
    let (second_release, second_wait) = mpsc::channel::<()>();
    {
        let mut gates = js.reads.gates.borrow_mut();
        for wait in [wait, second_wait] {
            let started_tx = started_tx.clone();
            let exited_tx = exited_tx.clone();
            gates.push_back(ReadGate(Box::new(move |cancel| {
                started_tx.send(()).unwrap();
                assert_eq!(
                    wait.recv_timeout(TIMEOUT),
                    Err(mpsc::RecvTimeoutError::Disconnected)
                );
                WORKER_EXIT.set(Some(WorkerExit(exited_tx, cancel.requested())));
                Ok(())
            })));
        }
        gates.push_back(ReadGate(Box::new(move |_| {
            drop((release, second_release));
            Ok(())
        })));
    }
    *js.before_drive.borrow_mut() = Some(Box::new(move || {
        started.recv_timeout(TIMEOUT).unwrap();
        started.recv_timeout(TIMEOUT).unwrap();
        panic!("injected owner panic");
    }));
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = js.run_module(
            "panic.js",
            &format!(
                "globalThis.pendingRead = fs.readFile({file}); fs.readFile({file}); fs.readFile({file});",
                file = literal(&file),
            ),
        );
    }))
    .unwrap_err();
    assert_eq!(panic.downcast_ref::<&str>(), Some(&"injected owner panic"));
    assert_eq!(exited.try_recv(), Ok(true));
    assert_eq!(exited.try_recv(), Ok(true));
    assert!(js.reads.active.borrow().is_none());
    js.context.with(|ctx| {
        let promise: Promise = ctx.globals().get("pendingRead").unwrap();
        assert!(promise.result::<()>().is_none());
    });
    for error in [
        js.eval("'no'").unwrap_err(),
        js.run("throw Error('must not run')").unwrap_err(),
        js.run_module("later.js", "").unwrap_err(),
        js.run_pending_tests().unwrap_err(),
    ] {
        assert!(error.contains("NEO_MODULE_ABORTED"));
    }
}
