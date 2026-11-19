use crate::{NeoJs, Permissions};

use super::{MAX_TRACKED_PENDING, MAX_TRACKED_REJECTIONS, Rejections};

fn runtime_without_automatic_gc() -> NeoJs {
    let js = NeoJs::new(Permissions::default()).unwrap();
    js._runtime.set_gc_threshold(usize::MAX);
    js
}

const UNREACHABLE_CYCLE: &str = "(() => {
    const promise = new Promise(() => {});
    promise.self = promise;
    return new WeakRef(promise);
})()";

#[test]
fn unreachable_pending_cycle_does_not_stall_or_poison() {
    let js = runtime_without_automatic_gc();
    js.run_module(
        "cycle.js",
        &format!(
            "globalThis.weakCycle = {UNREACHABLE_CYCLE};
             if (!weakCycle.deref()) throw Error('automatic GC invalidated the fixture');"
        ),
    )
    .unwrap();
    js.run_module(
        "after-cycle.js",
        "if (weakCycle.deref()) throw Error('cycle survived');",
    )
    .unwrap();
}

#[test]
fn out_of_turn_cycles_are_collected_but_live_work_is_refused() {
    for use_eval in [false, true] {
        let js = runtime_without_automatic_gc();
        let setup = format!(
            "globalThis.weakCycle = {UNREACHABLE_CYCLE};
             if (!weakCycle.deref()) throw Error('automatic GC invalidated the fixture');
             'ok'"
        );
        if use_eval {
            assert_eq!(js.eval(&setup).unwrap(), "ok");
        } else {
            js.run(&setup).unwrap();
        }
        js.run_module(
            "after-script.js",
            "if (weakCycle.deref()) throw Error('cycle survived');",
        )
        .unwrap();
        js.run(
            "globalThis.liveCycle = new Promise(() => {});
             liveCycle.self = liveCycle;",
        )
        .unwrap();
        let error = js.run_module("after-live-script.js", "").unwrap_err();
        assert!(error.contains("NEO_MODULE_DIRTY"), "{error}");
    }
}

#[test]
fn pending_cap_collects_cycles_but_refuses_a_live_overflow() {
    for (retain, expected) in [(false, None), (true, Some("NEO_MODULE_LIMIT"))] {
        let js = runtime_without_automatic_gc();
        let result = js.run_module(
            "capacity.js",
            &format!(
                "const retained = [];
                 for (let i = 0; i <= {MAX_TRACKED_PENDING}; i++) {{
                     const promise = new Promise(() => {{}});
                     promise.self = promise;
                     if ({retain}) retained.push(promise);
                 }}"
            ),
        );
        if let Some(expected) = expected {
            let error = result.unwrap_err();
            assert!(error.contains(expected), "{error}");
        } else {
            result.unwrap();
            js.run_module("after-capacity.js", "").unwrap();
        }
    }
}

#[test]
fn cycle_collection_jobs_stay_with_the_turn_that_created_them() {
    let js = runtime_without_automatic_gc();
    js.run_module(
        "cycle-finalizer.js",
        "let resume;
         const awaited = new Promise(resolve => { resume = resolve; });
         globalThis.registry = new FinalizationRegistry(() => resume(42));
         (() => {
             const cycle = new Promise(() => {});
             cycle.self = cycle;
             registry.register(cycle, 0);
         })();
         if (await awaited !== 42) throw Error('wrong cleanup value');",
    )
    .unwrap();
    js.run_module("after-finalizer.js", "").unwrap();

    let js = runtime_without_automatic_gc();
    js.run(
        "globalThis.registry = new FinalizationRegistry(() => { globalThis.escaped = true; });
         (() => {
             const cycle = new Promise(() => {});
             cycle.self = cycle;
             registry.register(cycle, 0);
         })();",
    )
    .unwrap();
    let error = js
        .run_module("must-refuse-queued-cleanup.js", "")
        .unwrap_err();
    assert!(error.contains("NEO_MODULE_DIRTY"), "{error}");
    js.context
        .with(|ctx| assert!(!ctx.globals().contains_key("escaped").unwrap()));
}

#[test]
fn caught_promise_finalization_stays_in_the_same_turn() {
    for (setup, tail) in [
        (
            "const onFinalize = () => { globalThis.cleaned = true; };",
            "",
        ),
        (
            "let onFinalize;
             globalThis.pending = new Promise(resolve => {
                 onFinalize = () => { globalThis.cleaned = true; resolve(); };
             });",
            "",
        ),
        (
            "let onFinalize;
             const pending = new Promise(resolve => {
                 onFinalize = () => { globalThis.cleaned = true; resolve(); };
             });",
            "await pending;",
        ),
    ] {
        let js = NeoJs::new(Permissions::default()).unwrap();
        // With weak tracking the Promise ordinarily dies earlier. Pin it for
        // this test until the driver's post-empty-queue checkpoint instead.
        let root = js.module_turns.quiescence_root.clone();
        js.context.with(|ctx| {
            let pin = rquickjs::Function::new(ctx.clone(), move |promise: rquickjs::Value<'_>| {
                *root.borrow_mut() =
                    Some(rquickjs::Persistent::save(promise.ctx(), promise.clone()));
            })
            .unwrap();
            ctx.globals().set("pinForCheckpoint", pin).unwrap();
        });
        js.run_module(
            "finalize.js",
            &format!(
                "{setup}
                 globalThis.registry = new FinalizationRegistry(onFinalize);
                 (() => {{
                     const promise = Promise.reject('caught');
                     registry.register(promise, 0);
                     promise.catch(() => {{}});
                     pinForCheckpoint(promise);
                 }})();
                 {tail}"
            ),
        )
        .unwrap_or_else(|error| panic!("{setup} {tail}: {error}"));
        js.run_module(
            "next.js",
            "if (!globalThis.cleaned) throw Error('cleanup escaped');",
        )
        .unwrap();
    }
}

#[test]
fn sequential_caught_reads_do_not_pin_rejection_reasons() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("buffer");
    // Track the actual read result and its backing buffer, without using
    // production-sized allocations or measuring RSS.
    std::fs::write(&file, vec![91; 256 * 1024]).unwrap();
    let file = serde_json::to_string(file.to_str().unwrap()).unwrap();
    let js = NeoJs::new(Permissions {
        filesystem: true,
        ..Permissions::default()
    })
    .unwrap();
    js.context.with(|ctx| {
        let collect =
            rquickjs::Function::new(ctx.clone(), |ctx: rquickjs::Ctx<'_>| ctx.run_gc()).unwrap();
        ctx.globals().set("collectForTest", collect).unwrap();
    });
    js.run_module(
        "caught-reads.js",
        &format!(
            "import {{ readFile }} from 'neo:fs';
             const weak = [];
             for (let i = 0; i < 16; i++) {{
                 await readFile({file}).then(bytes => {{ throw bytes; }}).catch(bytes => {{
                     if (bytes[0] !== 91) throw bytes;
                     weak.push(new WeakRef(bytes), new WeakRef(bytes.buffer));
                 }});
                 collectForTest();
                 if (weak.some(ref => ref.deref() !== undefined)) {{
                     throw Error('settled reasons were still strongly reachable');
                 }}
             }}"
        ),
    )
    .unwrap();
}

#[test]
fn a_collected_unhandled_rejection_still_counts_after_later_catches() {
    let js = NeoJs::new(Permissions::default()).unwrap();
    let error = js
        .run_module(
            "unhandled-collected.js",
            "let reason;
             (() => {
                 const bytes = new Uint8Array(256 * 1024);
                 reason = new WeakRef(bytes);
                 Promise.reject(bytes);
             })();
             for (let i = 0; i < 256; i++) {
                 const caught = Promise.reject(i);
                 await Promise.resolve();
                 await caught.catch(() => {});
             }
             if (reason.deref() !== undefined) throw Error('rejection reason was pinned');",
        )
        .unwrap_err();
    assert!(error.contains("NEO_UNHANDLED_REJECTION"), "{error}");
    js.run_module("after-unhandled.js", "").unwrap();
}

#[test]
fn reused_promise_identity_keeps_only_the_old_unhandled_rejection() {
    // Inject the exact hook sequence without depending on allocator reuse.
    let mut rejections = Rejections::default();
    let address = 1;
    rejections.on_init(address);
    rejections.update(address, false);
    rejections.on_init(address);
    rejections.update(address, false);
    rejections.update(address, true);
    assert!(rejections.has_unhandled());
    assert_eq!(rejections.collected, 1);
    assert!(rejections.promises.is_empty());
    for id in 2..=MAX_TRACKED_REJECTIONS {
        rejections.on_init(id);
        rejections.update(id, false);
    }
    assert!(!rejections.overflow);
    rejections.update(MAX_TRACKED_REJECTIONS + 1, false);
    assert!(rejections.overflow);
}

#[test]
fn weak_tracking_uses_no_script_controlled_intrinsics() {
    let js = runtime_without_automatic_gc();
    js.run_module(
        "tamper.js",
        "Object.defineProperty(WeakRef.prototype, 'deref', {
             get() { throw Error('host invoked script'); }
         });
         globalThis.WeakRef = new Proxy(WeakRef, {
             construct() { throw Error('host invoked script'); }
         });
         const caught = new Promise((_, reject) => {
             globalThis.rejectLater = reject;
         });
         const observed = caught.catch(() => {});
         await Promise.resolve();
         rejectLater('handled while pending');
         await observed;",
    )
    .unwrap();
    let error = js
        .run_module(
            "actually-pending.js",
            "globalThis.pending = new Promise(() => {}); pending.self = pending;",
        )
        .unwrap_err();
    assert!(error.contains("NEO_MODULE_STALLED"), "{error}");
}
