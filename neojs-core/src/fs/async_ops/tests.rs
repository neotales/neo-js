//! The worker barrier is private so these integration-style owner-turn
//! tests can prove ordering without a slow device or a timing assumption.
//!
//! These are the allowed unit-test exception from
//! `code-ergonomics.md § Testing: integration first`: each asserts a critical
//! concurrency path (admission, cancellation, panic containment, or teardown)
//! that the public API cannot reach deterministically, because the private
//! gate is the only way to hold a worker at a known point.

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

#[test]
fn a_text_write_uses_the_cwd_captured_at_the_call() {
    let old = tempfile::tempdir().unwrap();
    let new = tempfile::tempdir().unwrap();
    let js = runtime();
    let (started_tx, started) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    js.ops.gates.borrow_mut().push_back(Gate(Box::new(move |_| {
        started_tx.send(()).unwrap();
        release_rx.recv_timeout(TIMEOUT).unwrap();
        Ok(())
    })));
    let controller = thread::spawn(move || {
        started.recv_timeout(TIMEOUT).unwrap();
        release.send(()).unwrap();
    });
    let result = js.run_module(
        "write-text-cwd.js",
        &format!(
            r#"import {{chdir}} from 'neo:process';
            import {{writeText, readText}} from 'neo:fs';
            chdir({old});
            const pending = writeText('out.txt', 'old directory');
            chdir({new});
            await pending;
            chdir({old});
            if (readText('out.txt') !== 'old directory')
                throw Error('text write did not use the captured cwd');"#,
            old = literal(old.path()),
            new = literal(new.path()),
        ),
    );
    controller.join().unwrap();
    result.unwrap();
    assert_eq!(
        fs::read_to_string(old.path().join("out.txt")).unwrap(),
        "old directory"
    );
    assert!(!new.path().join("out.txt").exists());
}

#[test]
fn outstanding_text_writes_share_the_operation_bound() {
    let dir = tempfile::tempdir().unwrap();
    let paths: Vec<_> = (0..=MAX_PENDING)
        .map(|index| dir.path().join(format!("out-{index}.txt")))
        .collect();
    let literals: Vec<_> = paths.iter().map(|path| literal(path)).collect();
    let path_array = format!("[{}]", literals.join(","));
    let js = runtime();
    js.run_module(
        "write-text-saturation.js",
        &format!(
            r#"import {{writeText, FsError}} from 'neo:fs';
            const paths = {paths};
            const writes = [];
            for (let i = 0; i < {limit}; i++) writes.push(writeText(paths[i], 'text-' + i));
            try {{
                await writeText(paths[{limit}], 'overflow');
                throw Error('admitted one text write beyond the bound');
            }} catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_LIMIT') throw error;
            }}
            for (const result of await Promise.all(writes)) {{
                if (result !== undefined) throw Error('writeText resolved with a value');
            }}
            await writeText(paths[{limit}], 'recovered');
            if (fs.readText(paths[{limit}]) !== 'recovered') throw Error('slot did not recover');"#,
            paths = path_array,
            limit = MAX_PENDING,
        ),
    )
    .unwrap();
    for (index, path) in paths.iter().take(MAX_PENDING).enumerate() {
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            format!("text-{index}"),
            "{}",
            path.display()
        );
    }
    assert_eq!(
        fs::read_to_string(&paths[MAX_PENDING]).unwrap(),
        "recovered"
    );
}

#[test]
fn failing_turn_cancels_a_text_write_after_its_file_effect() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.txt");
    let second = dir.path().join("second.txt");
    let queued = dir.path().join("queued.txt");
    let js = runtime();
    let (started_tx, started) = mpsc::channel();
    let (joined_tx, joined) = mpsc::channel();
    let (queued_tx, queued_started) = mpsc::channel();
    let mut release = Vec::new();
    {
        // Each of the two running workers pauses only after `write_bytes_at`
        // returned, so the file already contains the new text when the wait
        // begins.
        let mut write_text_gates = js.ops.write_text_gates.borrow_mut();
        for (target, expected) in [(first.clone(), "first"), (second.clone(), "second")] {
            let (send, wait) = mpsc::channel::<()>();
            release.push(send);
            let started_tx = started_tx.clone();
            let joined_tx = joined_tx.clone();
            write_text_gates.push_back(WriteTextGate(Box::new(move |cancel| {
                assert_eq!(
                    fs::read_to_string(&target).unwrap(),
                    expected,
                    "gate ran before the text write effect"
                );
                started_tx.send(()).unwrap();
                assert_eq!(
                    wait.recv_timeout(TIMEOUT),
                    Err(mpsc::RecvTimeoutError::Disconnected)
                );
                joined_tx.send(cancel.requested()).unwrap();
                Ok(())
            })));
        }
        // A no-op pre-execute gate occupies each running job. The third gate
        // owns the senders and is attached to the queued job; discarding that
        // request during shutdown disconnects both post-effect waits.
        let mut gates = js.ops.gates.borrow_mut();
        for _ in 0..2 {
            gates.push_back(Gate(Box::new(|_| Ok(()))));
        }
        gates.push_back(Gate(Box::new(move |_| {
            drop(release);
            queued_tx.send(()).unwrap();
            Ok(())
        })));
    }
    js.context.with(|ctx| {
        let wait = Function::new(ctx.clone(), move || {
            started.recv_timeout(TIMEOUT).unwrap();
            started.recv_timeout(TIMEOUT).unwrap();
        })
        .unwrap();
        ctx.globals().set("finishTextWrites", wait).unwrap();
    });
    let result = js.run_module(
        "cancel-write-text-effect.js",
        &format!(
            "globalThis.cancelledTextWrite = fs.writeText({first}, 'first');
             fs.writeText({second}, 'second'); fs.writeText({queued}, 'queued');
             finishTextWrites(); throw Error('entry failed');",
            first = literal(&first),
            second = literal(&second),
            queued = literal(&queued),
        ),
    );
    assert!(result.unwrap_err().contains("entry failed"));
    assert_eq!(joined.try_recv(), Ok(true));
    assert_eq!(joined.try_recv(), Ok(true));
    assert_eq!(
        queued_started.try_recv(),
        Err(mpsc::TryRecvError::Disconnected)
    );
    // The two started text writes are real side effects and were not rolled
    // back. The queued write was never dispatched.
    assert_eq!(fs::read_to_string(&first).unwrap(), "first");
    assert_eq!(fs::read_to_string(&second).unwrap(), "second");
    assert!(!queued.exists());
    assert!(js.ops.active.borrow().is_none());
    js.context.with(|ctx| {
        let promise: Promise = ctx.globals().get("cancelledTextWrite").unwrap();
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
fn outstanding_metadata_reads_share_the_operation_bound() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("fixture");
    fs::write(&file, b"abc").unwrap();
    let js = runtime();
    let (started_tx, started) = mpsc::channel();
    let (progress, progress_rx) = std::sync::mpsc::channel();
    js.context.with(|ctx| {
        let progress = Function::new(ctx.clone(), move |phase: u8| {
            progress.send(phase).unwrap();
        })
        .unwrap();
        ctx.globals().set("progress", progress).unwrap();
    });
    let (release_first_tx, release_first) = mpsc::channel::<()>();
    let (release_second_tx, release_second) = mpsc::channel::<()>();
    {
        let mut gates = js.ops.gates.borrow_mut();
        for wait in [release_first, release_second] {
            let started_tx = started_tx.clone();
            gates.push_back(Gate(Box::new(move |_| {
                started_tx.send(()).unwrap();
                wait.recv_timeout(TIMEOUT).unwrap();
                Ok(())
            })));
        }
    }
    drop(started_tx);
    let controller = thread::spawn(move || {
        started.recv_timeout(TIMEOUT).unwrap();
        started.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(progress_rx.recv_timeout(TIMEOUT).unwrap(), 1);
        release_first_tx.send(()).unwrap();
        release_second_tx.send(()).unwrap();
    });
    let result = js.run_module(
        "stat-saturation.js",
        &format!(
            r#"import {{stat, FsError}} from 'neo:fs';
            const file = {file};
            const reads = [];
            for (let i = 0; i < {limit}; i++) reads.push(stat(file));
            try {{
                await stat(file);
                throw Error('admitted one metadata read beyond the bound');
            }} catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_LIMIT') throw error;
            }}
            progress(1);
            for (const info of await Promise.all(reads)) {{
                if (info.size !== '3' || !info.isFile) throw Error('missing metadata');
            }}
            if ((await stat(file)).size !== '3') throw Error('slot did not recover');"#,
            file = literal(&file),
            limit = MAX_PENDING,
        ),
    );
    controller.join().unwrap();
    result.unwrap();
}

#[test]
fn metadata_reads_share_the_bound_with_reads_and_writes() {
    let dir = tempfile::tempdir().unwrap();
    let existing = dir.path().join("existing");
    let written = dir.path().join("written");
    fs::write(&existing, [42, 7]).unwrap();
    let js = runtime();
    let (started_tx, started) = mpsc::channel();
    let (progress, progress_rx) = std::sync::mpsc::channel();
    js.context.with(|ctx| {
        let progress = Function::new(ctx.clone(), move |phase: u8| {
            progress.send(phase).unwrap();
        })
        .unwrap();
        ctx.globals().set("progress", progress).unwrap();
    });
    let (release_read_tx, release_read) = mpsc::channel::<()>();
    let (release_stat_tx, release_stat) = mpsc::channel::<()>();
    {
        let mut gates = js.ops.gates.borrow_mut();
        gates.push_back(Gate(Box::new({
            let started_tx = started_tx.clone();
            move |_| {
                started_tx.send(()).unwrap();
                release_read.recv_timeout(TIMEOUT).unwrap();
                Ok(())
            }
        })));
        gates.push_back(Gate(Box::new(move |_| {
            started_tx.send(()).unwrap();
            release_stat.recv_timeout(TIMEOUT).unwrap();
            Ok(())
        })));
    }
    let controller = thread::spawn(move || {
        started.recv_timeout(TIMEOUT).unwrap();
        started.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(progress_rx.recv_timeout(TIMEOUT).unwrap(), 1);
        release_read_tx.send(()).unwrap();
        release_stat_tx.send(()).unwrap();
    });
    let result = js.run_module(
        "stat-mixed-saturation.js",
        &format!(
            r#"import {{readFile, writeFile, stat, readFileSync, FsError}} from 'neo:fs';
            const existing = {existing};
            const written = {written};
            const read = readFile(existing);
            const firstInfo = stat(existing);
            const write = writeFile(written, new Uint8Array([5]));
            const secondInfo = stat(existing);
            try {{
                await stat(existing);
                throw Error('admitted work beyond the shared bound');
            }} catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_LIMIT') throw error;
            }}
            progress(1);
            const [bytes, first, wrote, second] = await Promise.all([read, firstInfo, write, secondInfo]);
            if (bytes[0] !== 42 || first.size !== '2' || second.size !== '2') {{
                throw Error('accepted metadata work did not finish');
            }}
            if (wrote !== undefined) throw Error('write resolved with a value');
            if (readFileSync(written)[0] !== 5) throw Error('accepted write did not land');
            if ((await stat(existing)).size !== '2') throw Error('metadata admission did not recover');"#,
            existing = literal(&existing),
            written = literal(&written),
        ),
    );
    controller.join().unwrap();
    result.unwrap();
    assert_eq!(fs::read(&written).unwrap(), [5]);
}

#[test]
fn a_panicking_metadata_read_is_reported_as_io_and_leaves_the_worker_usable() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("fixture");
    fs::write(&file, b"abc").unwrap();
    let js = runtime();
    js.ops.gates.borrow_mut().push_back(Gate(Box::new(|_| {
        panic!("injected worker panic");
    })));
    js.run_module(
        "panic-stat.js",
        &format!(
            r#"import {{stat, FsError}} from 'neo:fs';
            try {{
                await stat({file});
                throw Error('panicking metadata read fulfilled');
            }} catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_IO') throw error;
            }}
            if ((await stat({file})).size !== '3') throw Error('worker did not recover');"#,
            file = literal(&file),
        ),
    )
    .unwrap();
}

#[test]
fn failing_turn_cancels_pending_metadata_reads() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("fixture");
    fs::write(&file, b"abc").unwrap();
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
        ctx.globals().set("finishStats", wait).unwrap();
    });
    let result = js.run_module(
        "cancel-stats.js",
        &format!(
            "globalThis.cancelledStat = fs.stat({file});
             fs.stat({file}); fs.stat({file});
             finishStats(); throw Error('entry failed');",
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
        let promise: Promise = ctx.globals().get("cancelledStat").unwrap();
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
fn a_directory_read_completes_while_first_worker_is_blocked() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.txt"), b"a").unwrap();
    fs::write(dir.path().join("b.txt"), b"b").unwrap();
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
        "read-dir-progress.js",
        &format!(
            r#"import {{readDir}} from 'neo:fs';
            const dir = {dir};
            let done = false;
            const first = readDir(dir).then((entries) => {{
                done = true;
                if (entries.length !== 2 || entries[0].name !== 'a.txt') {{
                    throw Error('first listing wrong');
                }}
            }});
            await Promise.resolve();
            if (done) throw Error('first directory read finished before release');
            progress(1);
            const second = await readDir(dir);
            if (done) throw Error('first finished before its worker was released');
            if (second.length !== 2 || second[1].name !== 'b.txt') {{
                throw Error('second listing did not finish independently');
            }}
            progress(2);
            await first;"#,
            dir = literal(dir.path()),
        ),
    );
    controller.join().unwrap();
    result.unwrap();
}

#[test]
fn outstanding_directory_reads_share_the_operation_bound() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.txt"), b"a").unwrap();
    fs::write(dir.path().join("b.txt"), b"b").unwrap();
    let js = runtime();
    let (started_tx, started) = mpsc::channel();
    let (progress, progress_rx) = std::sync::mpsc::channel();
    js.context.with(|ctx| {
        let progress = Function::new(ctx.clone(), move |phase: u8| {
            progress.send(phase).unwrap();
        })
        .unwrap();
        ctx.globals().set("progress", progress).unwrap();
    });
    let (release_first_tx, release_first) = mpsc::channel::<()>();
    let (release_second_tx, release_second) = mpsc::channel::<()>();
    {
        let mut gates = js.ops.gates.borrow_mut();
        for wait in [release_first, release_second] {
            let started_tx = started_tx.clone();
            gates.push_back(Gate(Box::new(move |_| {
                started_tx.send(()).unwrap();
                wait.recv_timeout(TIMEOUT).unwrap();
                Ok(())
            })));
        }
    }
    drop(started_tx);
    let controller = thread::spawn(move || {
        started.recv_timeout(TIMEOUT).unwrap();
        started.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(progress_rx.recv_timeout(TIMEOUT).unwrap(), 1);
        release_first_tx.send(()).unwrap();
        release_second_tx.send(()).unwrap();
    });
    let result = js.run_module(
        "read-dir-saturation.js",
        &format!(
            r#"import {{readDir, FsError}} from 'neo:fs';
            const dir = {dir};
            const reads = [];
            for (let i = 0; i < {limit}; i++) reads.push(readDir(dir));
            try {{
                await readDir(dir);
                throw Error('admitted one directory read beyond the bound');
            }} catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_LIMIT') throw error;
            }}
            progress(1);
            for (const entries of await Promise.all(reads)) {{
                if (entries.length !== 2 || entries[1].name !== 'b.txt') {{
                    throw Error('missing directory entries');
                }}
            }}
            if ((await readDir(dir)).length !== 2) throw Error('slot did not recover');"#,
            dir = literal(dir.path()),
            limit = MAX_PENDING,
        ),
    );
    controller.join().unwrap();
    result.unwrap();
}

#[test]
fn directory_reads_share_the_bound_with_reads_and_writes() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("file.bin");
    let written = dir.path().join("written.bin");
    let listing = dir.path().join("listing");
    fs::create_dir(&listing).unwrap();
    fs::write(&file, [42, 7]).unwrap();
    fs::write(listing.join("a.txt"), b"a").unwrap();
    fs::write(listing.join("b.txt"), b"b").unwrap();
    let js = runtime();
    let (started_tx, started) = mpsc::channel();
    let (progress, progress_rx) = std::sync::mpsc::channel();
    js.context.with(|ctx| {
        let progress = Function::new(ctx.clone(), move |phase: u8| {
            progress.send(phase).unwrap();
        })
        .unwrap();
        ctx.globals().set("progress", progress).unwrap();
    });
    let (release_read_tx, release_read) = mpsc::channel::<()>();
    let (release_dir_tx, release_dir) = mpsc::channel::<()>();
    {
        let mut gates = js.ops.gates.borrow_mut();
        gates.push_back(Gate(Box::new({
            let started_tx = started_tx.clone();
            move |_| {
                started_tx.send(()).unwrap();
                release_read.recv_timeout(TIMEOUT).unwrap();
                Ok(())
            }
        })));
        gates.push_back(Gate(Box::new(move |_| {
            started_tx.send(()).unwrap();
            release_dir.recv_timeout(TIMEOUT).unwrap();
            Ok(())
        })));
    }
    let controller = thread::spawn(move || {
        started.recv_timeout(TIMEOUT).unwrap();
        started.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(progress_rx.recv_timeout(TIMEOUT).unwrap(), 1);
        release_read_tx.send(()).unwrap();
        release_dir_tx.send(()).unwrap();
    });
    let result = js.run_module(
        "read-dir-mixed-saturation.js",
        &format!(
            r#"import {{readFile, writeFile, readDir, readFileSync, FsError}} from 'neo:fs';
            const listing = {listing};
            const file = {file};
            const written = {written};
            const read = readFile(file);
            const firstListing = readDir(listing);
            const write = writeFile(written, new Uint8Array([5]));
            const secondListing = readDir(listing);
            try {{
                await readDir(listing);
                throw Error('admitted work beyond the shared bound');
            }} catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_LIMIT') throw error;
            }}
            progress(1);
            const settled = await Promise.all([read, firstListing, write, secondListing]);
            if (settled[0][0] !== 42 || settled[1].length !== 2 || settled[3].length !== 2) {{
                throw Error('accepted work did not finish');
            }}
            if (settled[2] !== undefined) throw Error('write resolved with a value');
            if (readFileSync(written)[0] !== 5) throw Error('accepted write did not land');
            if ((await readDir(listing)).length !== 2) throw Error('directory admission did not recover');"#,
            listing = literal(&listing),
            file = literal(&file),
            written = literal(&written),
        ),
    );
    controller.join().unwrap();
    result.unwrap();
    assert_eq!(fs::read(&written).unwrap(), [5]);
}

#[test]
fn a_panicking_directory_read_is_reported_as_io_and_leaves_the_worker_usable() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.txt"), b"a").unwrap();
    let js = runtime();
    js.ops.gates.borrow_mut().push_back(Gate(Box::new(|_| {
        panic!("injected worker panic");
    })));
    js.run_module(
        "panic-read-dir.js",
        &format!(
            r#"import {{readDir, FsError}} from 'neo:fs';
            try {{
                await readDir({dir});
                throw Error('panicking directory read fulfilled');
            }} catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_IO') throw error;
            }}
            if ((await readDir({dir})).length !== 1) throw Error('worker did not recover');"#,
            dir = literal(dir.path()),
        ),
    )
    .unwrap();
}

#[test]
fn failing_turn_cancels_pending_directory_reads() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.txt"), b"a").unwrap();
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
        ctx.globals().set("finishListings", wait).unwrap();
    });
    let result = js.run_module(
        "cancel-read-dir.js",
        &format!(
            "globalThis.cancelledListing = fs.readDir({dir});
             fs.readDir({dir}); fs.readDir({dir});
             finishListings(); throw Error('entry failed');",
            dir = literal(dir.path()),
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
        let promise: Promise = ctx.globals().get("cancelledListing").unwrap();
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
fn cancellation_stops_started_directory_enumeration_before_the_next_entry() {
    use std::sync::{Arc, Mutex};

    let dir = tempfile::tempdir().unwrap();
    for name in ["gate.txt", "marker-1.txt", "marker-2.txt"] {
        fs::write(dir.path().join(name), b"x").unwrap();
    }
    let js = runtime();
    let visited = Arc::new(Mutex::new(Vec::new()));
    let (started_tx, started) = mpsc::channel();
    let (release_entry, wait_entry) = mpsc::channel::<()>();
    let (release_second, wait_second) = mpsc::channel::<()>();
    let (again_tx, again) = mpsc::channel::<String>();
    let (third_tx, third_started) = mpsc::channel();
    {
        let visited = Arc::clone(&visited);
        let mut wait_entry = Some(wait_entry);
        let mut first = true;
        js.ops
            .read_dir_gates
            .borrow_mut()
            .push_back(ReadDirGate(Box::new(move |entry, cancel| {
                visited.lock().unwrap().push(entry.name.clone());
                if first {
                    first = false;
                    started_tx.send(()).unwrap();
                    // The senders live in the queued third job. They are
                    // dropped only after shutdown sets cancellation and
                    // discards the queue, so this is a cancellation barrier,
                    // not a delay.
                    assert_eq!(
                        wait_entry.take().unwrap().recv_timeout(TIMEOUT),
                        Err(mpsc::RecvTimeoutError::Disconnected)
                    );
                    assert!(cancel.requested());
                } else {
                    let _ = again_tx.send(entry.name.clone());
                }
                Ok(())
            })));
        let release = vec![release_entry, release_second];
        let mut gates = js.ops.gates.borrow_mut();
        gates.push_back(Gate(Box::new(|_| Ok(()))));
        gates.push_back(Gate(Box::new(move |cancel| {
            assert_eq!(
                wait_second.recv_timeout(TIMEOUT),
                Err(mpsc::RecvTimeoutError::Disconnected)
            );
            assert!(cancel.requested());
            Ok(())
        })));
        gates.push_back(Gate(Box::new(move |_| {
            drop(release);
            third_tx.send(()).unwrap();
            Ok(())
        })));
    }
    js.context.with(|ctx| {
        let wait = Function::new(ctx.clone(), move || {
            started.recv_timeout(TIMEOUT).unwrap();
        })
        .unwrap();
        ctx.globals().set("finishListing", wait).unwrap();
    });
    let result = js.run_module(
        "cancel-started-read-dir.js",
        &format!(
            "globalThis.cancelledListing = fs.readDir({dir});
             fs.readDir({dir}); fs.readDir({dir});
             finishListing(); throw Error('entry failed');",
            dir = literal(dir.path()),
        ),
    );
    assert!(result.unwrap_err().contains("entry failed"));
    assert_eq!(visited.lock().unwrap().len(), 1);
    assert_eq!(
        again.try_recv(),
        Err(mpsc::TryRecvError::Disconnected),
        "enumeration visited another entry after cancellation"
    );
    assert_eq!(
        third_started.try_recv(),
        Err(mpsc::TryRecvError::Disconnected)
    );
    assert!(js.ops.active.borrow().is_none());
    js.context.with(|ctx| {
        let promise: Promise = ctx.globals().get("cancelledListing").unwrap();
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
fn a_directory_creation_completes_while_first_worker_is_blocked() {
    let dir = tempfile::tempdir().unwrap();
    let first_dir = dir.path().join("first");
    let second_dir = dir.path().join("second");
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
        "mkdir-progress.js",
        &format!(
            r#"import {{mkdir}} from 'neo:fs';
            let done = false;
            const first = mkdir({first_dir}).then(() => {{
                done = true;
            }});
            await Promise.resolve();
            if (done) throw Error('first creation finished before release');
            progress(1);
            await mkdir({second_dir});
            if (done) throw Error('first finished before its worker was released');
            progress(2);
            await first;"#,
            first_dir = literal(&first_dir),
            second_dir = literal(&second_dir),
        ),
    );
    controller.join().unwrap();
    result.unwrap();
    assert!(first_dir.is_dir());
    assert!(second_dir.is_dir());
}

#[test]
fn outstanding_directory_creations_share_the_operation_bound() {
    let dir = tempfile::tempdir().unwrap();
    let paths: Vec<_> = (0..=MAX_PENDING)
        .map(|index| dir.path().join(format!("out-{index}")))
        .collect();
    let literals: Vec<_> = paths.iter().map(|path| literal(path)).collect();
    let path_array = format!("[{}]", literals.join(","));
    let js = runtime();
    let (started_tx, started) = mpsc::channel();
    let (progress, progress_rx) = std::sync::mpsc::channel();
    js.context.with(|ctx| {
        let progress = Function::new(ctx.clone(), move |phase: u8| {
            progress.send(phase).unwrap();
        })
        .unwrap();
        ctx.globals().set("progress", progress).unwrap();
    });
    let (release_first_tx, release_first) = mpsc::channel::<()>();
    let (release_second_tx, release_second) = mpsc::channel::<()>();
    {
        let mut gates = js.ops.gates.borrow_mut();
        for wait in [release_first, release_second] {
            let started_tx = started_tx.clone();
            gates.push_back(Gate(Box::new(move |_| {
                started_tx.send(()).unwrap();
                wait.recv_timeout(TIMEOUT).unwrap();
                Ok(())
            })));
        }
    }
    drop(started_tx);
    let controller = thread::spawn(move || {
        started.recv_timeout(TIMEOUT).unwrap();
        started.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(progress_rx.recv_timeout(TIMEOUT).unwrap(), 1);
        release_first_tx.send(()).unwrap();
        release_second_tx.send(()).unwrap();
    });
    let result = js.run_module(
        "mkdir-saturation.js",
        &format!(
            r#"import {{mkdir, FsError}} from 'neo:fs';
            const paths = {paths};
            const creations = [];
            for (let i = 0; i < {limit}; i++) creations.push(mkdir(paths[i]));
            try {{
                await mkdir(paths[{limit}]);
                throw Error('admitted one creation beyond the bound');
            }} catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_LIMIT') throw error;
            }}
            progress(1);
            for (const result of await Promise.all(creations)) {{
                if (result !== undefined) throw Error('mkdir resolved with a value');
            }}
            await mkdir(paths[{limit}]);
            if (fs.readDirSync(paths[0]).length !== 0) throw Error('created directory not empty');"#,
            paths = path_array,
            limit = MAX_PENDING,
        ),
    );
    controller.join().unwrap();
    result.unwrap();
    for path in &paths {
        assert!(path.is_dir(), "missing {}", path.display());
    }
}

#[test]
fn directory_creations_share_the_bound_with_reads_and_writes() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("file.bin");
    let written = dir.path().join("written.bin");
    let first_dir = dir.path().join("first");
    let second_dir = dir.path().join("second");
    let rejected_dir = dir.path().join("rejected");
    fs::write(&file, [42, 7]).unwrap();
    let js = runtime();
    let (started_tx, started) = mpsc::channel();
    let (progress, progress_rx) = std::sync::mpsc::channel();
    js.context.with(|ctx| {
        let progress = Function::new(ctx.clone(), move |phase: u8| {
            progress.send(phase).unwrap();
        })
        .unwrap();
        ctx.globals().set("progress", progress).unwrap();
    });
    let (release_read_tx, release_read) = mpsc::channel::<()>();
    let (release_mkdir_tx, release_mkdir) = mpsc::channel::<()>();
    {
        let mut gates = js.ops.gates.borrow_mut();
        gates.push_back(Gate(Box::new({
            let started_tx = started_tx.clone();
            move |_| {
                started_tx.send(()).unwrap();
                release_read.recv_timeout(TIMEOUT).unwrap();
                Ok(())
            }
        })));
        gates.push_back(Gate(Box::new(move |_| {
            started_tx.send(()).unwrap();
            release_mkdir.recv_timeout(TIMEOUT).unwrap();
            Ok(())
        })));
    }
    let controller = thread::spawn(move || {
        started.recv_timeout(TIMEOUT).unwrap();
        started.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(progress_rx.recv_timeout(TIMEOUT).unwrap(), 1);
        release_read_tx.send(()).unwrap();
        release_mkdir_tx.send(()).unwrap();
    });
    let result = js.run_module(
        "mkdir-mixed-saturation.js",
        &format!(
            r#"import {{readFile, writeFile, mkdir, FsError}} from 'neo:fs';
            const read = readFile({file});
            const first = mkdir({first_dir});
            const write = writeFile({written}, new Uint8Array([5]));
            const second = mkdir({second_dir});
            try {{
                await mkdir({rejected_dir});
                throw Error('admitted work beyond the shared bound');
            }} catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_LIMIT') throw error;
            }}
            progress(1);
            const [bytes, createdFirst, wrote, createdSecond] =
                await Promise.all([read, first, write, second]);
            if (bytes[0] !== 42 || createdFirst !== undefined || createdSecond !== undefined) {{
                throw Error('accepted work resolved with the wrong value');
            }}
            if (wrote !== undefined) throw Error('write resolved with a value');
            if (fs.readFileSync({written})[0] !== 5) throw Error('accepted write did not land');
            await mkdir({rejected_dir});
            if (!fs.exists({rejected_dir})) throw Error('creation admission did not recover');"#,
            file = literal(&file),
            written = literal(&written),
            first_dir = literal(&first_dir),
            second_dir = literal(&second_dir),
            rejected_dir = literal(&rejected_dir),
        ),
    );
    controller.join().unwrap();
    result.unwrap();
    assert_eq!(fs::read(&written).unwrap(), [5]);
    assert!(first_dir.is_dir() && second_dir.is_dir() && rejected_dir.is_dir());
}

#[test]
fn a_panicking_directory_creation_is_reported_as_io_and_leaves_the_worker_usable() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("created");
    let js = runtime();
    js.ops.gates.borrow_mut().push_back(Gate(Box::new(|_| {
        panic!("injected worker panic");
    })));
    js.run_module(
        "panic-mkdir.js",
        &format!(
            r#"import {{mkdir, FsError}} from 'neo:fs';
            try {{
                await mkdir({target});
                throw Error('panicking creation fulfilled');
            }} catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_IO') throw error;
            }}
            await mkdir({target});
            if (!fs.exists({target})) throw Error('worker did not recover');"#,
            target = literal(&target),
        ),
    )
    .unwrap();
    assert!(target.is_dir());
}

#[test]
fn failing_turn_cancels_pending_directory_creations() {
    let dir = tempfile::tempdir().unwrap();
    let targets: Vec<_> = (0..3)
        .map(|index| dir.path().join(format!("cancelled-{index}")))
        .collect();
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
        ctx.globals().set("finishCreations", wait).unwrap();
    });
    let result = js.run_module(
        "cancel-mkdir.js",
        &format!(
            "globalThis.cancelledMkdir = fs.mkdir({first});
             fs.mkdir({second}); fs.mkdir({third});
             finishCreations(); throw Error('entry failed');",
            first = literal(&targets[0]),
            second = literal(&targets[1]),
            third = literal(&targets[2]),
        ),
    );
    assert!(result.unwrap_err().contains("entry failed"));
    assert_eq!(joined.try_recv(), Ok(true));
    assert_eq!(joined.try_recv(), Ok(true));
    assert_eq!(
        third_started.try_recv(),
        Err(mpsc::TryRecvError::Disconnected)
    );
    assert!(targets.iter().all(|path| !path.exists()));
    assert!(js.ops.active.borrow().is_none());
    js.context.with(|ctx| {
        let promise: Promise = ctx.globals().get("cancelledMkdir").unwrap();
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
fn a_removal_completes_while_first_worker_is_blocked() {
    let dir = tempfile::tempdir().unwrap();
    let first_path = dir.path().join("first");
    let second_path = dir.path().join("second");
    fs::write(&first_path, b"first").unwrap();
    fs::write(&second_path, b"second").unwrap();
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
        "remove-progress.js",
        &format!(
            r#"import {{remove, exists}} from 'neo:fs';
            let done = false;
            const first = remove({first_path}).then(() => {{
                done = true;
                if (exists({first_path})) throw Error('first removal left the file');
            }});
            await Promise.resolve();
            if (done) throw Error('first removal finished before release');
            progress(1);
            await remove({second_path});
            if (done) throw Error('first finished before its worker was released');
            progress(2);
            await first;"#,
            first_path = literal(&first_path),
            second_path = literal(&second_path),
        ),
    );
    controller.join().unwrap();
    result.unwrap();
    assert!(!first_path.exists());
    assert!(!second_path.exists());
}

#[test]
fn outstanding_removals_share_the_operation_bound() {
    let dir = tempfile::tempdir().unwrap();
    let paths: Vec<_> = (0..=MAX_PENDING)
        .map(|index| dir.path().join(format!("out-{index}")))
        .collect();
    for path in &paths {
        fs::write(path, b"x").unwrap();
    }
    let literals: Vec<_> = paths.iter().map(|path| literal(path)).collect();
    let path_array = format!("[{}]", literals.join(","));
    let js = runtime();
    let (started_tx, started) = mpsc::channel();
    let (progress, progress_rx) = std::sync::mpsc::channel();
    js.context.with(|ctx| {
        let progress = Function::new(ctx.clone(), move |phase: u8| {
            progress.send(phase).unwrap();
        })
        .unwrap();
        ctx.globals().set("progress", progress).unwrap();
    });
    let (release_first_tx, release_first) = mpsc::channel::<()>();
    let (release_second_tx, release_second) = mpsc::channel::<()>();
    {
        let mut gates = js.ops.gates.borrow_mut();
        for wait in [release_first, release_second] {
            let started_tx = started_tx.clone();
            gates.push_back(Gate(Box::new(move |_| {
                started_tx.send(()).unwrap();
                wait.recv_timeout(TIMEOUT).unwrap();
                Ok(())
            })));
        }
    }
    drop(started_tx);
    let controller = thread::spawn(move || {
        started.recv_timeout(TIMEOUT).unwrap();
        started.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(progress_rx.recv_timeout(TIMEOUT).unwrap(), 1);
        release_first_tx.send(()).unwrap();
        release_second_tx.send(()).unwrap();
    });
    let result = js.run_module(
        "remove-saturation.js",
        &format!(
            r#"import {{remove, exists, FsError}} from 'neo:fs';
            const paths = {paths};
            const removals = [];
            for (let i = 0; i < {limit}; i++) removals.push(remove(paths[i]));
            try {{
                await remove(paths[{limit}]);
                throw Error('admitted one removal beyond the bound');
            }} catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_LIMIT') throw error;
            }}
            progress(1);
            for (const result of await Promise.all(removals)) {{
                if (result !== undefined) throw Error('remove resolved with a value');
            }}
            await remove(paths[{limit}]);
            if (exists(paths[0]) || exists(paths[{limit}])) {{
                throw Error('accepted removal left a file');
            }}"#,
            paths = path_array,
            limit = MAX_PENDING,
        ),
    );
    controller.join().unwrap();
    result.unwrap();
    for path in &paths {
        assert!(!path.exists(), "still present: {}", path.display());
    }
}

#[test]
fn removals_share_the_bound_with_reads_and_writes() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("file.bin");
    let written = dir.path().join("written.bin");
    let first_path = dir.path().join("first");
    let second_path = dir.path().join("second");
    let rejected_path = dir.path().join("rejected");
    fs::write(&file, [42, 7]).unwrap();
    fs::write(&first_path, b"first").unwrap();
    fs::write(&second_path, b"second").unwrap();
    fs::write(&rejected_path, b"rejected").unwrap();
    let js = runtime();
    let (started_tx, started) = mpsc::channel();
    let (progress, progress_rx) = std::sync::mpsc::channel();
    js.context.with(|ctx| {
        let progress = Function::new(ctx.clone(), move |phase: u8| {
            progress.send(phase).unwrap();
        })
        .unwrap();
        ctx.globals().set("progress", progress).unwrap();
    });
    let (release_read_tx, release_read) = mpsc::channel::<()>();
    let (release_remove_tx, release_remove) = mpsc::channel::<()>();
    {
        let mut gates = js.ops.gates.borrow_mut();
        gates.push_back(Gate(Box::new({
            let started_tx = started_tx.clone();
            move |_| {
                started_tx.send(()).unwrap();
                release_read.recv_timeout(TIMEOUT).unwrap();
                Ok(())
            }
        })));
        gates.push_back(Gate(Box::new(move |_| {
            started_tx.send(()).unwrap();
            release_remove.recv_timeout(TIMEOUT).unwrap();
            Ok(())
        })));
    }
    let controller = thread::spawn(move || {
        started.recv_timeout(TIMEOUT).unwrap();
        started.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(progress_rx.recv_timeout(TIMEOUT).unwrap(), 1);
        release_read_tx.send(()).unwrap();
        release_remove_tx.send(()).unwrap();
    });
    let result = js.run_module(
        "remove-mixed-saturation.js",
        &format!(
            r#"import {{readFile, writeFile, remove, exists, FsError}} from 'neo:fs';
            const read = readFile({file});
            const first = remove({first_path});
            const write = writeFile({written}, new Uint8Array([5]));
            const second = remove({second_path});
            try {{
                await remove({rejected_path});
                throw Error('admitted work beyond the shared bound');
            }} catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_LIMIT') throw error;
            }}
            progress(1);
            const [bytes, removedFirst, wrote, removedSecond] =
                await Promise.all([read, first, write, second]);
            if (bytes[0] !== 42 || removedFirst !== undefined || removedSecond !== undefined) {{
                throw Error('accepted work resolved with the wrong value');
            }}
            if (wrote !== undefined) throw Error('write resolved with a value');
            if (fs.readFileSync({written})[0] !== 5) throw Error('accepted write did not land');
            if (exists({first_path}) || exists({second_path})) {{
                throw Error('accepted removal left a file');
            }}
            await remove({rejected_path});
            if (exists({rejected_path})) throw Error('removal admission did not recover');"#,
            file = literal(&file),
            written = literal(&written),
            first_path = literal(&first_path),
            second_path = literal(&second_path),
            rejected_path = literal(&rejected_path),
        ),
    );
    controller.join().unwrap();
    result.unwrap();
    assert_eq!(fs::read(&written).unwrap(), [5]);
    assert!(!first_path.exists() && !second_path.exists() && !rejected_path.exists());
}

#[test]
fn a_panicking_removal_is_reported_as_io_and_leaves_the_worker_usable() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("removed");
    fs::write(&target, b"x").unwrap();
    let js = runtime();
    js.ops.gates.borrow_mut().push_back(Gate(Box::new(|_| {
        panic!("injected worker panic");
    })));
    js.run_module(
        "panic-remove.js",
        &format!(
            r#"import {{remove, exists, FsError}} from 'neo:fs';
            try {{
                await remove({target});
                throw Error('panicking removal fulfilled');
            }} catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_IO') throw error;
            }}
            if (!exists({target})) throw Error('panicking removal deleted the file');
            await remove({target});
            if (exists({target})) throw Error('worker did not recover');"#,
            target = literal(&target),
        ),
    )
    .unwrap();
    assert!(!target.exists());
}

#[test]
fn failing_turn_cancels_pending_removals() {
    let dir = tempfile::tempdir().unwrap();
    let targets: Vec<_> = (0..3)
        .map(|index| dir.path().join(format!("cancelled-{index}")))
        .collect();
    for path in &targets {
        fs::write(path, b"x").unwrap();
    }
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
        ctx.globals().set("finishRemovals", wait).unwrap();
    });
    let result = js.run_module(
        "cancel-remove.js",
        &format!(
            "globalThis.cancelledRemove = fs.remove({first});
             fs.remove({second}); fs.remove({third});
             finishRemovals(); throw Error('entry failed');",
            first = literal(&targets[0]),
            second = literal(&targets[1]),
            third = literal(&targets[2]),
        ),
    );
    assert!(result.unwrap_err().contains("entry failed"));
    assert_eq!(joined.try_recv(), Ok(true));
    assert_eq!(joined.try_recv(), Ok(true));
    assert_eq!(
        third_started.try_recv(),
        Err(mpsc::TryRecvError::Disconnected)
    );
    assert!(targets.iter().all(|path| path.exists()));
    assert!(js.ops.active.borrow().is_none());
    js.context.with(|ctx| {
        let promise: Promise = ctx.globals().get("cancelledRemove").unwrap();
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
fn failing_turn_cancels_a_removal_after_its_deletion_effect() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first");
    let second = dir.path().join("second");
    let queued = dir.path().join("queued");
    for path in [&first, &second, &queued] {
        fs::write(path, b"x").unwrap();
    }
    let js = runtime();
    let (started_tx, started) = mpsc::channel();
    let (joined_tx, joined) = mpsc::channel();
    let (queued_tx, queued_started) = mpsc::channel();
    let mut release = Vec::new();
    {
        // Each of the two running workers pauses only after `remove_at` has
        // returned, so the file is already gone when the wait begins.
        let mut remove_gates = js.ops.remove_gates.borrow_mut();
        for target in [first.clone(), second.clone()] {
            let (send, wait) = mpsc::channel::<()>();
            release.push(send);
            let started_tx = started_tx.clone();
            let joined_tx = joined_tx.clone();
            remove_gates.push_back(RemoveGate(Box::new(move |cancel| {
                assert!(!target.exists(), "gate ran before the deletion effect");
                started_tx.send(()).unwrap();
                assert_eq!(
                    wait.recv_timeout(TIMEOUT),
                    Err(mpsc::RecvTimeoutError::Disconnected)
                );
                joined_tx.send(cancel.requested()).unwrap();
                Ok(())
            })));
        }
        // A no-op pre-execute gate occupies each running job. The third gate
        // owns the senders and is attached to the queued job; discarding that
        // request during shutdown disconnects both post-effect waits.
        let mut gates = js.ops.gates.borrow_mut();
        for _ in 0..2 {
            gates.push_back(Gate(Box::new(|_| Ok(()))));
        }
        gates.push_back(Gate(Box::new(move |_| {
            drop(release);
            queued_tx.send(()).unwrap();
            Ok(())
        })));
    }
    js.context.with(|ctx| {
        let wait = Function::new(ctx.clone(), move || {
            started.recv_timeout(TIMEOUT).unwrap();
            started.recv_timeout(TIMEOUT).unwrap();
        })
        .unwrap();
        ctx.globals().set("finishRemovals", wait).unwrap();
    });
    let result = js.run_module(
        "cancel-remove-effect.js",
        &format!(
            "globalThis.cancelledRemoveEffect = fs.remove({first});
             fs.remove({second}); fs.remove({queued});
             finishRemovals(); throw Error('entry failed');",
            first = literal(&first),
            second = literal(&second),
            queued = literal(&queued),
        ),
    );
    assert!(result.unwrap_err().contains("entry failed"));
    assert_eq!(joined.try_recv(), Ok(true));
    assert_eq!(joined.try_recv(), Ok(true));
    assert_eq!(
        queued_started.try_recv(),
        Err(mpsc::TryRecvError::Disconnected)
    );
    // The two started deletions are real side effects and were not rolled back.
    assert!(!first.exists());
    assert!(!second.exists());
    // The queued deletion was never dispatched.
    assert!(queued.exists());
    assert!(js.ops.active.borrow().is_none());
    js.context.with(|ctx| {
        let promise: Promise = ctx.globals().get("cancelledRemoveEffect").unwrap();
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
fn a_rename_completes_while_first_worker_is_blocked() {
    let dir = tempfile::tempdir().unwrap();
    let first_source = dir.path().join("first-source");
    let first_destination = dir.path().join("first-destination");
    let second_source = dir.path().join("second-source");
    let second_destination = dir.path().join("second-destination");
    fs::write(&first_source, b"first").unwrap();
    fs::write(&second_source, b"second").unwrap();
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
        "rename-progress.js",
        &format!(
            r#"import {{rename}} from 'neo:fs';
            let done = false;
            const first = rename({first_source}, {first_destination}).then(() => {{
                done = true;
            }});
            await Promise.resolve();
            if (done) throw Error('first rename finished before release');
            progress(1);
            await rename({second_source}, {second_destination});
            if (done) throw Error('first finished before its worker was released');
            progress(2);
            await first;"#,
            first_source = literal(&first_source),
            first_destination = literal(&first_destination),
            second_source = literal(&second_source),
            second_destination = literal(&second_destination),
        ),
    );
    controller.join().unwrap();
    result.unwrap();
    assert!(!first_source.exists() && !second_source.exists());
    assert_eq!(fs::read(&first_destination).unwrap(), b"first");
    assert_eq!(fs::read(&second_destination).unwrap(), b"second");
}

#[test]
fn outstanding_renames_share_the_operation_bound() {
    let dir = tempfile::tempdir().unwrap();
    let sources: Vec<_> = (0..=MAX_PENDING)
        .map(|index| dir.path().join(format!("rename-source-{index}")))
        .collect();
    let destinations: Vec<_> = (0..=MAX_PENDING)
        .map(|index| dir.path().join(format!("rename-destination-{index}")))
        .collect();
    for source in &sources {
        fs::write(source, b"x").unwrap();
    }
    let source_literals: Vec<_> = sources.iter().map(|path| literal(path)).collect();
    let destination_literals: Vec<_> = destinations.iter().map(|path| literal(path)).collect();
    let source_array = format!("[{}]", source_literals.join(","));
    let destination_array = format!("[{}]", destination_literals.join(","));
    let js = runtime();
    let (started_tx, started) = mpsc::channel();
    let (progress, progress_rx) = std::sync::mpsc::channel();
    js.context.with(|ctx| {
        let progress = Function::new(ctx.clone(), move |phase: u8| {
            progress.send(phase).unwrap();
        })
        .unwrap();
        ctx.globals().set("progress", progress).unwrap();
    });
    let (release_first_tx, release_first) = mpsc::channel::<()>();
    let (release_second_tx, release_second) = mpsc::channel::<()>();
    {
        let mut gates = js.ops.gates.borrow_mut();
        for wait in [release_first, release_second] {
            let started_tx = started_tx.clone();
            gates.push_back(Gate(Box::new(move |_| {
                started_tx.send(()).unwrap();
                wait.recv_timeout(TIMEOUT).unwrap();
                Ok(())
            })));
        }
    }
    drop(started_tx);
    let controller = thread::spawn(move || {
        started.recv_timeout(TIMEOUT).unwrap();
        started.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(progress_rx.recv_timeout(TIMEOUT).unwrap(), 1);
        release_first_tx.send(()).unwrap();
        release_second_tx.send(()).unwrap();
    });
    let result = js.run_module(
        "rename-saturation.js",
        &format!(
            r#"import {{rename, exists, FsError}} from 'neo:fs';
            const sources = {sources};
            const destinations = {destinations};
            const renames = [];
            for (let i = 0; i < {limit}; i++) renames.push(rename(sources[i], destinations[i]));
            try {{
                await rename(sources[{limit}], destinations[{limit}]);
                throw Error('admitted one rename beyond the bound');
            }} catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_LIMIT') throw error;
            }}
            progress(1);
            for (const result of await Promise.all(renames)) {{
                if (result !== undefined) throw Error('rename resolved with a value');
            }}
            for (let i = 0; i < {limit}; i++) {{
                if (exists(sources[i]) || !exists(destinations[i])) {{
                    throw Error('accepted rename did not move the entry');
                }}
            }}
            await rename(sources[{limit}], destinations[{limit}]);
            if (exists(sources[{limit}]) || !exists(destinations[{limit}])) {{
                throw Error('rename admission did not recover');
            }}"#,
            sources = source_array,
            destinations = destination_array,
            limit = MAX_PENDING,
        ),
    );
    controller.join().unwrap();
    result.unwrap();
    for index in 0..=MAX_PENDING {
        assert!(!sources[index].exists());
        assert!(destinations[index].exists());
    }
}

#[test]
fn a_panicking_rename_is_reported_as_io_and_leaves_the_worker_usable() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("panic-source");
    let destination = dir.path().join("panic-destination");
    fs::write(&source, b"x").unwrap();
    let js = runtime();
    js.ops.gates.borrow_mut().push_back(Gate(Box::new(|_| {
        panic!("injected worker panic");
    })));
    js.run_module(
        "panic-rename.js",
        &format!(
            r#"import {{rename, exists, FsError}} from 'neo:fs';
            try {{
                await rename({source}, {destination});
                throw Error('panicking rename fulfilled');
            }} catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_IO') throw error;
            }}
            if (!exists({source}) || exists({destination})) {{
                throw Error('panicking rename moved the entry');
            }}
            await rename({source}, {destination});
            if (exists({source}) || !exists({destination})) {{
                throw Error('worker did not recover');
            }}"#,
            source = literal(&source),
            destination = literal(&destination),
        ),
    )
    .unwrap();
    assert!(!source.exists());
    assert!(destination.exists());
}

#[test]
fn failing_turn_cancels_pending_renames() {
    let dir = tempfile::tempdir().unwrap();
    let pairs: Vec<_> = (0..3)
        .map(|index| {
            (
                dir.path().join(format!("cancel-source-{index}")),
                dir.path().join(format!("cancel-destination-{index}")),
            )
        })
        .collect();
    for (source, _) in &pairs {
        fs::write(source, b"x").unwrap();
    }
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
        ctx.globals().set("finishRenames", wait).unwrap();
    });
    let result = js.run_module(
        "cancel-rename.js",
        &format!(
            "globalThis.cancelledRename = fs.rename({first_source}, {first_destination});
             fs.rename({second_source}, {second_destination});
             fs.rename({third_source}, {third_destination});
             finishRenames(); throw Error('entry failed');",
            first_source = literal(&pairs[0].0),
            first_destination = literal(&pairs[0].1),
            second_source = literal(&pairs[1].0),
            second_destination = literal(&pairs[1].1),
            third_source = literal(&pairs[2].0),
            third_destination = literal(&pairs[2].1),
        ),
    );
    assert!(result.unwrap_err().contains("entry failed"));
    assert_eq!(joined.try_recv(), Ok(true));
    assert_eq!(joined.try_recv(), Ok(true));
    assert_eq!(
        third_started.try_recv(),
        Err(mpsc::TryRecvError::Disconnected)
    );
    assert!(pairs.iter().all(|(source, _)| source.exists()));
    assert!(pairs.iter().all(|(_, destination)| !destination.exists()));
    assert!(js.ops.active.borrow().is_none());
    js.context.with(|ctx| {
        let promise: Promise = ctx.globals().get("cancelledRename").unwrap();
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
fn failing_turn_cancels_a_rename_after_its_effect() {
    let dir = tempfile::tempdir().unwrap();
    let pairs: Vec<_> = (0..3)
        .map(|index| {
            (
                dir.path().join(format!("effect-source-{index}")),
                dir.path().join(format!("effect-destination-{index}")),
            )
        })
        .collect();
    for (source, _) in &pairs {
        fs::write(source, b"x").unwrap();
    }
    let js = runtime();
    let (started_tx, started) = mpsc::channel();
    let (joined_tx, joined) = mpsc::channel();
    let (queued_tx, queued_started) = mpsc::channel();
    let mut release = Vec::new();
    {
        // Each of the two running workers pauses only after `rename_at` has
        // returned, so the entry is already moved when the wait begins.
        let mut rename_gates = js.ops.rename_gates.borrow_mut();
        for (source, destination) in pairs.iter().take(2) {
            let source = source.clone();
            let destination = destination.clone();
            let (send, wait) = mpsc::channel::<()>();
            release.push(send);
            let started_tx = started_tx.clone();
            let joined_tx = joined_tx.clone();
            rename_gates.push_back(RenameGate(Box::new(move |cancel| {
                assert!(!source.exists(), "gate ran before the rename effect");
                assert!(destination.exists(), "gate ran before the rename effect");
                started_tx.send(()).unwrap();
                assert_eq!(
                    wait.recv_timeout(TIMEOUT),
                    Err(mpsc::RecvTimeoutError::Disconnected)
                );
                joined_tx.send(cancel.requested()).unwrap();
                Ok(())
            })));
        }
        // A no-op pre-execute gate occupies each running job. The third gate
        // owns the senders and is attached to the queued job; discarding that
        // request during shutdown disconnects both post-effect waits.
        let mut gates = js.ops.gates.borrow_mut();
        for _ in 0..2 {
            gates.push_back(Gate(Box::new(|_| Ok(()))));
        }
        gates.push_back(Gate(Box::new(move |_| {
            drop(release);
            queued_tx.send(()).unwrap();
            Ok(())
        })));
    }
    js.context.with(|ctx| {
        let wait = Function::new(ctx.clone(), move || {
            started.recv_timeout(TIMEOUT).unwrap();
            started.recv_timeout(TIMEOUT).unwrap();
        })
        .unwrap();
        ctx.globals().set("finishRenames", wait).unwrap();
    });
    let result = js.run_module(
        "cancel-rename-effect.js",
        &format!(
            "globalThis.cancelledRenameEffect = fs.rename({first_source}, {first_destination});
             fs.rename({second_source}, {second_destination});
             fs.rename({third_source}, {third_destination});
             finishRenames(); throw Error('entry failed');",
            first_source = literal(&pairs[0].0),
            first_destination = literal(&pairs[0].1),
            second_source = literal(&pairs[1].0),
            second_destination = literal(&pairs[1].1),
            third_source = literal(&pairs[2].0),
            third_destination = literal(&pairs[2].1),
        ),
    );
    assert!(result.unwrap_err().contains("entry failed"));
    assert_eq!(joined.try_recv(), Ok(true));
    assert_eq!(joined.try_recv(), Ok(true));
    assert_eq!(
        queued_started.try_recv(),
        Err(mpsc::TryRecvError::Disconnected)
    );
    // The two started renames are real side effects and were not rolled back.
    for (source, destination) in pairs.iter().take(2) {
        assert!(!source.exists());
        assert!(destination.exists());
    }
    // The queued rename was never dispatched.
    assert!(pairs[2].0.exists());
    assert!(!pairs[2].1.exists());
    assert!(js.ops.active.borrow().is_none());
    js.context.with(|ctx| {
        let promise: Promise = ctx.globals().get("cancelledRenameEffect").unwrap();
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
fn a_copy_completes_while_first_worker_is_blocked() {
    let dir = tempfile::tempdir().unwrap();
    let first_source = dir.path().join("first-source");
    let first_destination = dir.path().join("first-destination");
    let second_source = dir.path().join("second-source");
    let second_destination = dir.path().join("second-destination");
    fs::write(&first_source, b"first").unwrap();
    fs::write(&second_source, b"second").unwrap();
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
        "copy-progress.js",
        &format!(
            r#"import {{copyFile}} from 'neo:fs';
            let done = false;
            const first = copyFile({first_source}, {first_destination}).then(() => {{
                done = true;
            }});
            await Promise.resolve();
            if (done) throw Error('first copy finished before release');
            progress(1);
            await copyFile({second_source}, {second_destination});
            if (done) throw Error('first finished before its worker was released');
            progress(2);
            await first;"#,
            first_source = literal(&first_source),
            first_destination = literal(&first_destination),
            second_source = literal(&second_source),
            second_destination = literal(&second_destination),
        ),
    );
    controller.join().unwrap();
    result.unwrap();
    assert!(first_source.exists() && second_source.exists());
    assert_eq!(fs::read(&first_destination).unwrap(), b"first");
    assert_eq!(fs::read(&second_destination).unwrap(), b"second");
}

#[test]
fn outstanding_copies_share_the_operation_bound() {
    let dir = tempfile::tempdir().unwrap();
    let sources: Vec<_> = (0..=MAX_PENDING)
        .map(|index| dir.path().join(format!("copy-source-{index}")))
        .collect();
    let destinations: Vec<_> = (0..=MAX_PENDING)
        .map(|index| dir.path().join(format!("copy-destination-{index}")))
        .collect();
    for source in &sources {
        fs::write(source, b"x").unwrap();
    }
    let source_literals: Vec<_> = sources.iter().map(|path| literal(path)).collect();
    let destination_literals: Vec<_> = destinations.iter().map(|path| literal(path)).collect();
    let source_array = format!("[{}]", source_literals.join(","));
    let destination_array = format!("[{}]", destination_literals.join(","));
    let js = runtime();
    let (started_tx, started) = mpsc::channel();
    let (progress, progress_rx) = std::sync::mpsc::channel();
    js.context.with(|ctx| {
        let progress = Function::new(ctx.clone(), move |phase: u8| {
            progress.send(phase).unwrap();
        })
        .unwrap();
        ctx.globals().set("progress", progress).unwrap();
    });
    let (release_first_tx, release_first) = mpsc::channel::<()>();
    let (release_second_tx, release_second) = mpsc::channel::<()>();
    {
        let mut gates = js.ops.gates.borrow_mut();
        for wait in [release_first, release_second] {
            let started_tx = started_tx.clone();
            gates.push_back(Gate(Box::new(move |_| {
                started_tx.send(()).unwrap();
                wait.recv_timeout(TIMEOUT).unwrap();
                Ok(())
            })));
        }
    }
    drop(started_tx);
    let controller = thread::spawn(move || {
        started.recv_timeout(TIMEOUT).unwrap();
        started.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(progress_rx.recv_timeout(TIMEOUT).unwrap(), 1);
        release_first_tx.send(()).unwrap();
        release_second_tx.send(()).unwrap();
    });
    let result = js.run_module(
        "copy-saturation.js",
        &format!(
            r#"import {{copyFile, exists, FsError}} from 'neo:fs';
            const sources = {sources};
            const destinations = {destinations};
            const copies = [];
            for (let i = 0; i < {limit}; i++) copies.push(copyFile(sources[i], destinations[i]));
            try {{
                await copyFile(sources[{limit}], destinations[{limit}]);
                throw Error('admitted one copy beyond the bound');
            }} catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_LIMIT') throw error;
            }}
            progress(1);
            for (const result of await Promise.all(copies)) {{
                if (result !== undefined) throw Error('copy resolved with a value');
            }}
            for (let i = 0; i < {limit}; i++) {{
                if (!exists(sources[i]) || !exists(destinations[i])) {{
                    throw Error('accepted copy did not create the destination');
                }}
            }}
            await copyFile(sources[{limit}], destinations[{limit}]);
            if (!exists(destinations[{limit}])) {{
                throw Error('copy admission did not recover');
            }}"#,
            sources = source_array,
            destinations = destination_array,
            limit = MAX_PENDING,
        ),
    );
    controller.join().unwrap();
    result.unwrap();
    for index in 0..=MAX_PENDING {
        assert!(sources[index].exists());
        assert!(destinations[index].exists());
    }
}

#[test]
fn a_panicking_copy_is_reported_as_io_and_leaves_the_worker_usable() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("panic-source");
    let destination = dir.path().join("panic-destination");
    fs::write(&source, b"x").unwrap();
    let js = runtime();
    js.ops.gates.borrow_mut().push_back(Gate(Box::new(|_| {
        panic!("injected worker panic");
    })));
    js.run_module(
        "panic-copy.js",
        &format!(
            r#"import {{copyFile, exists, FsError}} from 'neo:fs';
            try {{
                await copyFile({source}, {destination});
                throw Error('panicking copy fulfilled');
            }} catch (error) {{
                if (!(error instanceof FsError) || error.code !== 'FS_IO') throw error;
            }}
            if (!exists({source}) || exists({destination})) {{
                throw Error('panicking copy created or removed an entry');
            }}
            await copyFile({source}, {destination});
            if (!exists({destination})) {{
                throw Error('worker did not recover');
            }}"#,
            source = literal(&source),
            destination = literal(&destination),
        ),
    )
    .unwrap();
    assert!(source.exists());
    assert_eq!(fs::read(&destination).unwrap(), b"x");
}

#[test]
fn failing_turn_cancels_pending_copies() {
    let dir = tempfile::tempdir().unwrap();
    let pairs: Vec<_> = (0..3)
        .map(|index| {
            (
                dir.path().join(format!("cancel-source-{index}")),
                dir.path().join(format!("cancel-destination-{index}")),
            )
        })
        .collect();
    for (source, _) in &pairs {
        fs::write(source, b"x").unwrap();
    }
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
        ctx.globals().set("finishCopies", wait).unwrap();
    });
    let result = js.run_module(
        "cancel-copy.js",
        &format!(
            "globalThis.cancelledCopy = fs.copyFile({first_source}, {first_destination});
             fs.copyFile({second_source}, {second_destination});
             fs.copyFile({third_source}, {third_destination});
             finishCopies(); throw Error('entry failed');",
            first_source = literal(&pairs[0].0),
            first_destination = literal(&pairs[0].1),
            second_source = literal(&pairs[1].0),
            second_destination = literal(&pairs[1].1),
            third_source = literal(&pairs[2].0),
            third_destination = literal(&pairs[2].1),
        ),
    );
    assert!(result.unwrap_err().contains("entry failed"));
    assert_eq!(joined.try_recv(), Ok(true));
    assert_eq!(joined.try_recv(), Ok(true));
    assert_eq!(
        third_started.try_recv(),
        Err(mpsc::TryRecvError::Disconnected)
    );
    assert!(pairs.iter().all(|(source, _)| source.exists()));
    assert!(pairs.iter().all(|(_, destination)| !destination.exists()));
    assert!(js.ops.active.borrow().is_none());
    js.context.with(|ctx| {
        let promise: Promise = ctx.globals().get("cancelledCopy").unwrap();
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
fn failing_turn_cancels_a_copy_after_its_effect() {
    let dir = tempfile::tempdir().unwrap();
    let pairs: Vec<_> = (0..3)
        .map(|index| {
            (
                dir.path().join(format!("effect-source-{index}")),
                dir.path().join(format!("effect-destination-{index}")),
            )
        })
        .collect();
    for (source, _) in &pairs {
        fs::write(source, b"x").unwrap();
    }
    let js = runtime();
    let (started_tx, started) = mpsc::channel();
    let (joined_tx, joined) = mpsc::channel();
    let (queued_tx, queued_started) = mpsc::channel();
    let mut release = Vec::new();
    {
        // Each of the two running workers pauses only after the copy has
        // returned, so the destination already exists when the wait begins.
        let mut copy_gates = js.ops.copy_gates.borrow_mut();
        for (source, destination) in pairs.iter().take(2) {
            let source = source.clone();
            let destination = destination.clone();
            let (send, wait) = mpsc::channel::<()>();
            release.push(send);
            let started_tx = started_tx.clone();
            let joined_tx = joined_tx.clone();
            copy_gates.push_back(CopyGate(Box::new(move |cancel| {
                assert!(source.exists(), "gate ran after the source disappeared");
                assert!(destination.exists(), "gate ran before the copy effect");
                started_tx.send(()).unwrap();
                assert_eq!(
                    wait.recv_timeout(TIMEOUT),
                    Err(mpsc::RecvTimeoutError::Disconnected)
                );
                joined_tx.send(cancel.requested()).unwrap();
                Ok(())
            })));
        }
        // A no-op pre-execute gate occupies each running job. The third gate
        // owns the senders and is attached to the queued job; discarding that
        // request during shutdown disconnects both post-effect waits.
        let mut gates = js.ops.gates.borrow_mut();
        for _ in 0..2 {
            gates.push_back(Gate(Box::new(|_| Ok(()))));
        }
        gates.push_back(Gate(Box::new(move |_| {
            drop(release);
            queued_tx.send(()).unwrap();
            Ok(())
        })));
    }
    js.context.with(|ctx| {
        let wait = Function::new(ctx.clone(), move || {
            started.recv_timeout(TIMEOUT).unwrap();
            started.recv_timeout(TIMEOUT).unwrap();
        })
        .unwrap();
        ctx.globals().set("finishCopies", wait).unwrap();
    });
    let result = js.run_module(
        "cancel-copy-effect.js",
        &format!(
            "globalThis.cancelledCopyEffect = fs.copyFile({first_source}, {first_destination});
             fs.copyFile({second_source}, {second_destination});
             fs.copyFile({third_source}, {third_destination});
             finishCopies(); throw Error('entry failed');",
            first_source = literal(&pairs[0].0),
            first_destination = literal(&pairs[0].1),
            second_source = literal(&pairs[1].0),
            second_destination = literal(&pairs[1].1),
            third_source = literal(&pairs[2].0),
            third_destination = literal(&pairs[2].1),
        ),
    );
    assert!(result.unwrap_err().contains("entry failed"));
    assert_eq!(joined.try_recv(), Ok(true));
    assert_eq!(joined.try_recv(), Ok(true));
    assert_eq!(
        queued_started.try_recv(),
        Err(mpsc::TryRecvError::Disconnected)
    );
    // The two started copies are real side effects and were not rolled back.
    for (source, destination) in pairs.iter().take(2) {
        assert!(source.exists());
        assert_eq!(fs::read(destination).unwrap(), b"x");
    }
    // The queued copy was never dispatched.
    assert!(pairs[2].0.exists());
    assert!(!pairs[2].1.exists());
    assert!(js.ops.active.borrow().is_none());
    js.context.with(|ctx| {
        let promise: Promise = ctx.globals().get("cancelledCopyEffect").unwrap();
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
