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
    js.ops.gates.borrow_mut().push_back(Gate(Box::new(move |_| {
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
        let mut gates = js.ops.gates.borrow_mut();
        for _ in 0..2 {
            let (send, wait) = mpsc::channel::<()>();
            release.push(send);
            let started_tx = started_tx.clone();
            let joined_tx = joined_tx.clone();
            gates.push_back(Gate(Box::new(move |cancel| {
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
        gates.push_back(Gate(Box::new(move |_| {
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
    assert!(js.ops.active.borrow().is_none());
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
        let mut gates = js.ops.gates.borrow_mut();
        for wait in [wait, second_wait] {
            let started_tx = started_tx.clone();
            let exited_tx = exited_tx.clone();
            gates.push_back(Gate(Box::new(move |cancel| {
                started_tx.send(()).unwrap();
                assert_eq!(
                    wait.recv_timeout(TIMEOUT),
                    Err(mpsc::RecvTimeoutError::Disconnected)
                );
                WORKER_EXIT.set(Some(WorkerExit(exited_tx, cancel.requested())));
                Ok(())
            })));
        }
        gates.push_back(Gate(Box::new(move |_| {
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
    assert!(js.ops.active.borrow().is_none());
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

#[test]
fn a_js_job_and_a_read_complete_while_a_write_is_blocked() {
    let dir = tempfile::tempdir().unwrap();
    let existing = dir.path().join("existing");
    let written = dir.path().join("written");
    fs::write(&existing, [42, 7]).unwrap();
    let js = runtime();
    let (started_tx, started) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    js.ops.gates.borrow_mut().push_back(Gate(Box::new(move |_| {
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
        "write-progress.js",
        &format!(
            r#"import {{readFile, writeFile, readFileSync}} from 'neo:fs';
            let done = false;
            const first = writeFile({written}, new Uint8Array([5, 6, 7])).then(() => {{
                done = true;
            }});
            await Promise.resolve();
            if (done) throw Error('write finished before release');
            progress(1);
            const existing = await readFile({existing});
            if (existing[0] !== 42 || existing[1] !== 7)
                throw Error('independent read did not finish');
            progress(2);
            await first;
            const bytes = readFileSync({written});
            if (bytes.length !== 3 || bytes[2] !== 7) throw Error('write result missing');"#,
            existing = literal(&existing),
            written = literal(&written),
        ),
    );
    controller.join().unwrap();
    result.unwrap();
    assert_eq!(fs::read(&written).unwrap(), [5, 6, 7]);
}

#[test]
fn outstanding_writes_share_the_operation_bound() {
    let dir = tempfile::tempdir().unwrap();
    let js = runtime();
    let paths: Vec<_> = (0..=MAX_PENDING)
        .map(|index| dir.path().join(format!("out-{index}")))
        .collect();
    let literals: Vec<_> = paths.iter().map(|path| literal(path)).collect();
    let path_array = format!("[{}]", literals.join(","));
    js.run_module(
        "write-saturation.js",
        &format!(
            r#"import {{writeFile, FsError}} from 'neo:fs';
            const paths = {paths};
            const writes = [];
            for (let i = 0; i < {limit}; i++) writes.push(writeFile(paths[i], new Uint8Array([i])));
            try {{
                await writeFile(paths[{limit}], new Uint8Array([255]));
                throw Error('admitted one write beyond the bound');
            }} catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_LIMIT') throw error;
            }}
            await Promise.all(writes);
            await writeFile(paths[{limit}], new Uint8Array([9]));
            if (fs.readFileSync(paths[{limit}])[0] !== 9) throw Error('write slot did not recover');"#,
            paths = path_array,
            limit = MAX_PENDING,
        ),
    )
    .unwrap();
    for (index, path) in paths.iter().take(MAX_PENDING).enumerate() {
        assert_eq!(fs::read(path).unwrap(), [index as u8]);
    }
    assert_eq!(fs::read(&paths[MAX_PENDING]).unwrap(), [9]);
}

#[test]
fn combined_reads_and_writes_share_the_operation_bound() {
    let dir = tempfile::tempdir().unwrap();
    let readable = dir.path().join("input");
    let first_write = dir.path().join("out-1");
    let second_write = dir.path().join("out-2");
    let recovered = dir.path().join("recovered");
    fs::write(&readable, b"ok").unwrap();
    let js = runtime();
    let (started_tx, started) = mpsc::channel();
    let (release_first_tx, release_first) = mpsc::channel::<()>();
    let (release_second_tx, release_second) = mpsc::channel::<()>();
    {
        let mut gates = js.ops.gates.borrow_mut();
        gates.push_back(Gate(Box::new({
            let started_tx = started_tx.clone();
            move |_| {
                started_tx.send(()).unwrap();
                release_first.recv_timeout(TIMEOUT).unwrap();
                Ok(())
            }
        })));
        gates.push_back(Gate(Box::new({
            let started_tx = started_tx.clone();
            move |_| {
                started_tx.send(()).unwrap();
                release_second.recv_timeout(TIMEOUT).unwrap();
                Ok(())
            }
        })));
    }
    drop(started_tx);
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
        started.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(progress_rx.recv_timeout(TIMEOUT).unwrap(), 1);
        release_first_tx.send(()).unwrap();
        release_second_tx.send(()).unwrap();
    });
    let result = js.run_module(
        "mixed-saturation.js",
        &format!(
            r#"import {{readFile, writeFile, exists, FsError}} from 'neo:fs';
            const readable = {readable};
            const first = readFile(readable);
            const writeFirst = writeFile({first_write}, new Uint8Array([1]));
            const second = readFile(readable);
            const writeSecond = writeFile({second_write}, new Uint8Array([2]));
            for (const action of [
                () => readFile(readable),
                () => writeFile({recovered}, new Uint8Array([9])),
            ]) {{
                try {{
                    await action();
                    throw Error('admitted work beyond the shared bound');
                }} catch (error) {{
                    if (!(error instanceof FsError) || error.code !== 'FS_LIMIT') throw error;
                }}
            }}
            if (exists({recovered})) throw Error('rejected write created a file');
            progress(1);
            const settled = await Promise.all([first, writeFirst, second, writeSecond]);
            if (settled[0][0] !== 111 || settled[2][1] !== 107)
                throw Error('accepted reads did not finish');
            if (settled[1] !== undefined || settled[3] !== undefined)
                throw Error('accepted writes resolved with a value');
            if ((await readFile(readable))[0] !== 111) throw Error('read admission did not recover');
            await writeFile({recovered}, new Uint8Array([3]));"#,
            readable = literal(&readable),
            first_write = literal(&first_write),
            second_write = literal(&second_write),
            recovered = literal(&recovered),
        ),
    );
    controller.join().unwrap();
    result.unwrap();
    assert_eq!(fs::read(&first_write).unwrap(), [1]);
    assert_eq!(fs::read(&second_write).unwrap(), [2]);
    assert_eq!(fs::read(&recovered).unwrap(), [3]);
}

#[test]
fn failing_turn_cancels_pending_writes_before_refusing_later_work() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("fixture");
    let js = runtime();
    let (started_tx, started) = mpsc::channel();
    let (joined_tx, joined) = mpsc::channel();
    let (third_tx, third_started) = mpsc::channel();
    let mut release = Vec::new();
    {
        let mut gates = js.ops.gates.borrow_mut();
        for _ in 0..2 {
            let (send, wait) = mpsc::channel::<()>();
            release.push(send);
            let started_tx = started_tx.clone();
            let joined_tx = joined_tx.clone();
            gates.push_back(Gate(Box::new(move |cancel| {
                started_tx.send(()).unwrap();
                assert_eq!(
                    wait.recv_timeout(TIMEOUT),
                    Err(mpsc::RecvTimeoutError::Disconnected)
                );
                joined_tx.send(cancel.requested()).unwrap();
                Ok(())
            })));
        }
        gates.push_back(Gate(Box::new(move |_| {
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
        ctx.globals().set("finishWrite", wait).unwrap();
    });
    let result = js.run_module(
        "cancel-writes.js",
        &format!(
            "globalThis.cancelledWrite = fs.writeFile({file}, new Uint8Array([1]));
             fs.writeFile({file}, new Uint8Array([2]));
             fs.writeFile({file}, new Uint8Array([3]));
             finishWrite(); throw Error('entry failed');",
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
    assert!(js.ops.active.borrow().is_none());
    js.context.with(|ctx| {
        let promise: Promise = ctx.globals().get("cancelledWrite").unwrap();
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
fn a_panicking_write_is_reported_as_io_and_leaves_the_worker_usable() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("fixture");
    let js = runtime();
    js.ops.gates.borrow_mut().push_back(Gate(Box::new(|_| {
        panic!("injected worker panic");
    })));
    js.run_module(
        "panic-write.js",
        &format!(
            r#"import {{writeFile, readFileSync, FsError}} from 'neo:fs';
            try {{
                await writeFile({file}, new Uint8Array([1]));
                throw Error('panicking write fulfilled');
            }} catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_IO') throw error;
            }}
            await writeFile({file}, new Uint8Array([2]));
            if (readFileSync({file})[0] !== 2) throw Error('worker did not recover');"#,
            file = literal(&file),
        ),
    )
    .unwrap();
}

#[test]
fn a_write_uses_the_cwd_captured_before_option_reflection() {
    let old = tempfile::tempdir().unwrap();
    let new = tempfile::tempdir().unwrap();
    fs::write(old.path().join("out"), b"old").unwrap();
    let js = runtime();
    js.run_module(
        "write-cwd.js",
        &format!(
            r#"import {{chdir}} from 'neo:process';
            import {{writeFile, readFileSync}} from 'neo:fs';
            chdir({old});
            let trapped = false;
            const options = new Proxy({{}}, {{
                ownKeys() {{
                    trapped = true;
                    chdir({new});
                    return [];
                }},
            }});
            await writeFile('out', new Uint8Array([0x6e]), options);
            if (!trapped) throw Error('option trap did not run');
            chdir({old});
            if (readFileSync('out')[0] !== 0x6e) throw Error('write did not use old cwd');"#,
            old = literal(old.path()),
            new = literal(new.path()),
        ),
    )
    .unwrap();
    assert_eq!(fs::read(old.path().join("out")).unwrap(), b"n");
    assert!(!new.path().join("out").exists());
}
