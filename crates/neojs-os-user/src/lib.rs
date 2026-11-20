//! Optional synchronous account and process identity for `neojs-core`.
//!
//! Register [`with_os_user`], grant [`neojs_core::Permissions::user`], and
//! admit [`MODULE_NAME`] in any explicit allowlist. This extension does not
//! register `neo:os` or install globals. Account information is sensitive;
//! only grant access to trusted scripts.
//!
//! ```
//! use neojs_core::{NeoJs, Permissions, RuntimeOptions};
//! use neojs_os_user::{MODULE_NAME, with_os_user};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let options = with_os_user(RuntimeOptions::new())?.module_names([MODULE_NAME])?;
//! let runtime = NeoJs::new_with_options(
//!     Permissions { user: true, ..Permissions::default() },
//!     options,
//! )?;
//! runtime.run_module("main.js", "import 'neo:os/user';")?;
//! # Ok(())
//! # }
//! ```

use std::any::Any;

use neojs_core::rquickjs;
use neojs_core::rquickjs::{
    Array, Ctx, Error, Function, Module, Object, Result, Value,
    module::{Declarations, Declared, Exports, ModuleDef},
    object::Property,
    qjs,
};
use neojs_core::{
    ExtensionDescriptor, ExtensionPermission, ExtensionRegistrationError, RuntimeOptions,
};

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix;

/// Exact module name for use in extension-aware runtime allowlists.
pub const MODULE_NAME: &str = "neo:os/user";

/// Describe the identity extension. The canonical module must pass both the
/// allowlist and user permission before identity state is initialized.
/// Adding other module names to the descriptor does not create aliases.
pub fn os_user_extension() -> ExtensionDescriptor {
    ExtensionDescriptor::new("neojs-os-user", load)
        .with_module(MODULE_NAME, ExtensionPermission::User)
        .with_init(init)
}

/// Register the identity extension without granting permissions or changing
/// the allowlist. Duplicate registration is an error.
pub fn with_os_user(
    options: RuntimeOptions,
) -> std::result::Result<RuntimeOptions, ExtensionRegistrationError> {
    options.with_extension(os_user_extension())
}

struct UserSession;

fn init(ctx: &Ctx<'_>, allowed: &[&str]) -> Result<Box<dyn Any>> {
    // Core runs init if any registered name is admitted. Only the canonical
    // name has the identity permission gate, so an appended name is not enough.
    if !allowed.contains(&MODULE_NAME) {
        return Ok(Box::new(()));
    }
    prepare(ctx)?;
    Ok(Box::new(UserSession))
}

fn load<'js>(ctx: &Ctx<'js>, state: &mut dyn Any, name: &str) -> Result<Module<'js, Declared>> {
    if name != MODULE_NAME {
        return Err(Error::new_loading_message(
            name,
            "identity module does not support aliases",
        ));
    }
    if !state.is::<UserSession>() {
        return Err(Error::new_loading_message(
            name,
            "identity module was not initialized",
        ));
    }
    Module::declare_def::<UserModule, _>(ctx.clone(), name)
}

struct Account {
    uid: u32,
    primary_gid: u32,
    username: String,
    home_dir: String,
}

struct ProcessIds {
    real_uid: u32,
    effective_uid: u32,
    real_gid: u32,
    effective_gid: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UserFailure {
    #[cfg(any(test, target_os = "linux", target_os = "macos"))]
    NotFound,
    #[cfg(any(test, target_os = "linux", target_os = "macos"))]
    Encoding,
    #[cfg(any(test, target_os = "linux", target_os = "macos"))]
    Limit,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    Lookup,
    #[cfg(any(test, not(any(target_os = "linux", target_os = "macos"))))]
    Unsupported,
}

impl UserFailure {
    fn throw(self, ctx: &Ctx<'_>, class: ErrorClass) -> Error {
        let (code, message) = match self {
            #[cfg(any(test, target_os = "linux", target_os = "macos"))]
            Self::NotFound => ("USER_NOT_FOUND", "the real user's account was not found"),
            #[cfg(any(test, target_os = "linux", target_os = "macos"))]
            Self::Encoding => ("USER_ENCODING", "an account field is not valid UTF-8"),
            #[cfg(any(test, target_os = "linux", target_os = "macos"))]
            Self::Limit => ("USER_LIMIT", "an identity lookup exceeded a resource limit"),
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            Self::Lookup => ("USER_LOOKUP", "the operating system identity lookup failed"),
            #[cfg(any(test, not(any(target_os = "linux", target_os = "macos"))))]
            Self::Unsupported => (
                "USER_UNSUPPORTED",
                "identity lookup is unsupported on this platform",
            ),
        };
        // SAFETY: class came from a native Error in this context. The live
        // context owns the returned object, handed to Value only on success.
        let raw = unsafe { qjs::JS_NewObjectClass(ctx.as_raw().as_ptr(), class.0) };
        if unsafe { qjs::JS_IsException(raw) } {
            return Error::Exception;
        }
        let value = unsafe { Value::from_raw(ctx.clone(), raw) };
        let object = value
            .as_object()
            .expect("JS_NewObjectClass creates an object");
        // Omit the stack on the returned Error. QuickJS may still invoke its
        // ordinary throw-time stack hook; the error contains only fixed data.
        // Do not call user constructors or inherited property setters here.
        if let Err(error) = object
            .prop("stack", rquickjs::Undefined)
            .and_then(|()| object.prop("name", "Error"))
            .and_then(|()| object.prop("message", Property::from(message)))
            .and_then(|()| object.prop("code", Property::from(code).enumerable()))
        {
            return error;
        }
        ctx.throw(value)
    }
}

#[cfg(test)]
mod tests {
    use rquickjs::{Context, Module, Runtime};

    use super::*;

    #[test]
    fn alias_admission_does_not_initialize_identity_state() {
        let runtime = Runtime::new().unwrap();
        let context = Context::full(&runtime).unwrap();
        context.with(|ctx| {
            let state = init(&ctx, &["neo:identity-alias"]).unwrap();
            assert!(!state.is::<UserSession>());
            assert!(ctx.userdata::<ErrorClass>().is_none());
        });
    }

    struct Synthetic;

    impl Backend for Synthetic {
        fn current_user() -> Lookup<Account> {
            Ok(Account {
                uid: 4001,
                primary_gid: 301,
                username: "synthetic-user".into(),
                home_dir: "/synthetic/profile".into(),
            })
        }

        fn home_dir() -> Lookup<String> {
            Ok("/synthetic/profile".into())
        }

        fn process_ids() -> Lookup<ProcessIds> {
            Ok(ProcessIds {
                real_uid: 4001,
                effective_uid: u32::MAX - 1,
                real_gid: 302,
                effective_gid: 303,
            })
        }

        fn account_group_ids() -> Lookup<Vec<u32>> {
            Ok(vec![301, u32::MAX])
        }
    }

    struct Failures;

    impl Backend for Failures {
        fn current_user() -> Lookup<Account> {
            Err(UserFailure::NotFound)
        }

        fn home_dir() -> Lookup<String> {
            Err(UserFailure::Encoding)
        }

        fn process_ids() -> Lookup<ProcessIds> {
            Err(UserFailure::Unsupported)
        }

        fn account_group_ids() -> Lookup<Vec<u32>> {
            Err(UserFailure::Limit)
        }
    }

    struct MockModule<B>(std::marker::PhantomData<B>);

    impl<B: Backend> ModuleDef for MockModule<B> {
        fn declare<'js>(decl: &Declarations<'js>) -> Result<()> {
            declare(decl)
        }

        fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
            evaluate::<B>(ctx, exports)
        }
    }

    fn run<B: Backend>(source: &str) {
        use rquickjs::CatchResultExt;
        let runtime = Runtime::new().unwrap();
        let context = Context::full(&runtime).unwrap();
        context.with(|ctx| {
            prepare(&ctx).unwrap();
            let (module, evaluation) =
                Module::declare_def::<MockModule<B>, _>(ctx.clone(), "test-backend")
                    .unwrap()
                    .eval()
                    .unwrap();
            evaluation.finish::<()>().catch(&ctx).unwrap();
            ctx.globals()
                .set("api", module.namespace().unwrap())
                .unwrap();
            ctx.eval::<(), _>(source).catch(&ctx).unwrap();
        });
    }

    #[test]
    fn sync_exports_keep_account_and_process_fields_distinct() {
        run::<Synthetic>(
            r#"
            const { currentUserSync, homeDirSync, processIdsSync, accountGroupIdsSync } = api;
            const fail = () => { throw Error("synthetic identity mismatch"); };
            const trap = () => { throw Error("unexpected inherited setter"); };
            Object.defineProperty(Array.prototype, "0", {set: trap});
            Object.defineProperty(Object.prototype, "uid", {set: trap});
            const account = currentUserSync();
            const ids = processIdsSync();
            const groups = accountGroupIdsSync();
            if (Object.getPrototypeOf(account) !== null || Object.getPrototypeOf(ids) !== null) fail();
            if (Object.keys(account).sort().join() !== "homeDir,primaryGid,uid,username") fail();
            if (Object.keys(ids).sort().join() !== "effectiveGid,effectiveUid,realGid,realUid") fail();
            if (account.uid !== ids.realUid || ids.realUid !== 4001 || ids.effectiveUid !== 4294967294) fail();
            if (ids.realGid !== 302 || ids.effectiveGid !== 303 || account.primaryGid !== 301) fail();
            if (account.username !== "synthetic-user" || account.homeDir !== homeDirSync()) fail();
            if (account.homeDir !== "/synthetic/profile") fail();
            if (!Array.isArray(groups) || groups.length !== 2 || groups[0] !== 301 || groups[1] !== 4294967295) fail();
            groups[0] = 1;
            account.uid = 1;
            if (accountGroupIdsSync()[0] !== 301 || currentUserSync().uid !== 4001) fail();
        "#,
        );
    }

    #[test]
    fn errors_are_real_errors_with_fixed_codes_messages_and_no_stack() {
        run::<Failures>(
            r#"
            const NativeError = Error;
            let hooks = 0;
            const stackTrap = () => "synthetic stack hook output";
            const propertyTrap = () => { hooks++; };
            Error.prepareStackTrace = stackTrap;
            Object.defineProperty(Error.prototype, "code", {set: propertyTrap});
            Object.defineProperty(Error.prototype, "message", {set: propertyTrap});
            globalThis.Error = propertyTrap;
            const expected = [
                [api.currentUserSync, "USER_NOT_FOUND", "the real user's account was not found"],
                [api.homeDirSync, "USER_ENCODING", "an account field is not valid UTF-8"],
                [api.processIdsSync, "USER_UNSUPPORTED", "identity lookup is unsupported on this platform"],
                [api.accountGroupIdsSync, "USER_LIMIT", "an identity lookup exceeded a resource limit"],
            ];
            for (const [call, code, message] of expected) {
                let caught = false;
                try { call(); } catch (error) {
                    if (!(error instanceof NativeError) || error.code !== code || error.message !== message ||
                        error.stack !== undefined || !Object.hasOwn(error, "code")) {
                        throw "identity error mismatch";
                    }
                    caught = true;
                }
                if (!caught) throw "identity lookup unexpectedly succeeded";
            }
            if (hooks !== 0) throw "unexpected error-construction hook count: " + hooks;
        "#,
        );
    }
}

#[derive(Clone, Copy)]
struct ErrorClass(qjs::JSClassID);

// SAFETY: contains a numeric class ID only, never a borrowed JS value.
unsafe impl<'js> rquickjs::JsLifetime<'js> for ErrorClass {
    type Changed<'to> = Self;
}

/// Remember the native Error class in the fresh realm without looking up host identity.
fn prepare(ctx: &Ctx<'_>) -> Result<()> {
    let template = rquickjs::Exception::from_message(ctx.clone(), "")?;
    // SAFETY: the native Error is live in this context.
    let class = unsafe { qjs::JS_GetClassID(template.as_object().as_raw()) };
    ctx.store_userdata(ErrorClass(class))
        .map_err(|_| Error::Unknown)?;
    Ok(())
}

type Lookup<T> = std::result::Result<T, UserFailure>;

trait Backend: 'static {
    fn current_user() -> Lookup<Account>;
    fn home_dir() -> Lookup<String>;
    fn process_ids() -> Lookup<ProcessIds>;
    fn account_group_ids() -> Lookup<Vec<u32>>;
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
use unix::System;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
struct System;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
impl Backend for System {
    fn current_user() -> Lookup<Account> {
        Err(UserFailure::Unsupported)
    }

    fn home_dir() -> Lookup<String> {
        Err(UserFailure::Unsupported)
    }

    fn process_ids() -> Lookup<ProcessIds> {
        Err(UserFailure::Unsupported)
    }

    fn account_group_ids() -> Lookup<Vec<u32>> {
        Err(UserFailure::Unsupported)
    }
}

/// Native module instantiated by the loader only after both capability gates.
struct UserModule;

impl ModuleDef for UserModule {
    fn declare<'js>(decl: &Declarations<'js>) -> Result<()> {
        declare(decl)
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        evaluate::<System>(ctx, exports)
    }
}

const EXPORTS: [&str; 4] = [
    "currentUserSync",
    "homeDirSync",
    "processIdsSync",
    "accountGroupIdsSync",
];

fn declare<'js>(decl: &Declarations<'js>) -> Result<()> {
    for name in EXPORTS {
        decl.declare(name)?;
    }
    Ok(())
}

fn evaluate<'js, B: Backend>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
    // Convert UID/GID values through f64 explicitly: rquickjs's u32 conversion
    // can round-trip large unsigned IDs through a negative i32.
    let class = *ctx.userdata::<ErrorClass>().ok_or(Error::Unknown)?;
    let [current_user, home_dir, process_ids, account_group_ids] = EXPORTS;
    let account = Function::new(ctx.clone(), move |ctx: Ctx<'js>| {
        let account = B::current_user().map_err(|error| error.throw(&ctx, class))?;
        let value = Object::new_proto(ctx, None)?;
        value.set("uid", f64::from(account.uid))?;
        value.set("primaryGid", f64::from(account.primary_gid))?;
        value.set("username", account.username)?;
        value.set("homeDir", account.home_dir)?;
        Ok::<_, Error>(value)
    })?;
    let home = Function::new(ctx.clone(), move |ctx: Ctx<'js>| {
        B::home_dir().map_err(|error| error.throw(&ctx, class))
    })?;
    let ids = Function::new(ctx.clone(), move |ctx: Ctx<'js>| {
        let ids = B::process_ids().map_err(|error| error.throw(&ctx, class))?;
        let value = Object::new_proto(ctx, None)?;
        value.set("realUid", f64::from(ids.real_uid))?;
        value.set("effectiveUid", f64::from(ids.effective_uid))?;
        value.set("realGid", f64::from(ids.real_gid))?;
        value.set("effectiveGid", f64::from(ids.effective_gid))?;
        Ok::<_, Error>(value)
    })?;
    let groups = Function::new(ctx.clone(), move |ctx: Ctx<'js>| {
        let groups = B::account_group_ids().map_err(|error| error.throw(&ctx, class))?;
        let value = Array::new(ctx)?;
        for (index, group) in groups.into_iter().enumerate() {
            // Defining own data properties avoids inherited index setters.
            value.as_object().prop(
                index as u32,
                Property::from(f64::from(group))
                    .enumerable()
                    .writable()
                    .configurable(),
            )?;
        }
        Ok::<_, Error>(value)
    })?;

    exports.export(current_user, account.with_name(current_user)?)?;
    exports.export(home_dir, home.with_name(home_dir)?)?;
    exports.export(process_ids, ids.with_name(process_ids)?)?;
    exports.export(account_group_ids, groups.with_name(account_group_ids)?)?;
    Ok(())
}
