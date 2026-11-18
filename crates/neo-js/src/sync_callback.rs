use rquickjs::{Array, Ctx, Function, Object, Result, Value, qjs};

fn class_id(value: &Value<'_>) -> qjs::JSClassID {
    // SAFETY: `value` holds a live QuickJS handle; this public API only reads its class.
    unsafe { qjs::JS_GetClassID(value.as_raw()) }
}

/// Build a private guard in the fresh realm for `neo:test` and `neo:assert`.
/// Compare public QuickJS class IDs from genuine specimens, not hard-coded
/// internal enum values, mutable prototypes, or user-visible tags.
pub(crate) fn guard<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    let samples: Array = ctx.eval(
        "[(async function () {}), (function* () {}), (async function* () {}),
          (function* () {})(), (async function* () {})()]",
    )?;
    let sample_class = |index| samples.get::<Value>(index).map(|value| class_id(&value));
    let function_classes = [sample_class(0)?, sample_class(1)?, sample_class(2)?];
    let generator_classes = [sample_class(3)?, sample_class(4)?];
    let invalid_callback = Function::new(ctx.clone(), move |value: Value<'js>| {
        value.is_proxy() || function_classes.contains(&class_id(&value))
    })?;
    let return_brand = Function::new(ctx.clone(), move |value: Value<'js>| {
        if value.is_promise() {
            // The caller reports a synchronous failure. Do not leak the
            // rejection reason, enqueue reactions, or drain any pending jobs.
            // SAFETY: Both live handles belong to the same context and `value`
            // was checked to be a native Promise.
            unsafe { qjs::JS_PromiseMarkAsHandled(value.ctx().as_raw().as_ptr(), value.as_raw()) };
            'p'
        } else if generator_classes.contains(&class_id(&value)) {
            'g'
        } else {
            's'
        }
    })?;
    let is_proxy = Function::new(ctx.clone(), |value: Value<'js>| value.is_proxy())?;
    let factory: Function = ctx.eval(
        r#"(function (invalidCallback, returnBrand, isProxy) {
            const ownDescriptor = Object.getOwnPropertyDescriptor;
            const own = Object.hasOwn;
            const prototypeOf = Object.getPrototypeOf;
            function unsupportedReturn(value) {
                if (value === null || (typeof value !== 'object' && typeof value !== 'function'))
                    return null;
                const brand = returnBrand(value);
                if (brand === 'p') return 'a Promise';
                if (brand === 'g') return 'a generator';
                try {
                    for (let object = value; object !== null; object = prototypeOf(object)) {
                        if (isProxy(object)) return 'a value with a proxy in its then lookup';
                        const descriptor = ownDescriptor(object, 'then');
                        if (descriptor === undefined) continue;
                        if (!own(descriptor, 'value')) return 'a value with a then accessor';
                        return typeof descriptor.value === 'function' ? 'a thenable' : null;
                    }
                } catch (_) {
                    return 'a value whose then property could not be inspected';
                }
                return null;
            }
            return { invalidCallback, unsupportedReturn };
        })"#,
    )?;
    factory.call((invalid_callback, return_brand, is_proxy))
}
