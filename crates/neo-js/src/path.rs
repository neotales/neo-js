//! Private host binding for the lexical `neo:path` module.

use std::{cell::RefCell, rc::Rc};

use rquickjs::{
    Ctx, Error, Function, Object, Persistent, Result, Value,
    module::{Declarations, Exports, ModuleDef},
};

use crate::{
    Permissions,
    cwd::WorkingDirectory,
    modules::{AllowedModules, RuntimeModule},
};

const PATH_SOURCE: &str = include_str!("js/path.js");
const GLOB_SOURCE: &str = include_str!("js/path-glob.js");
const FILE_URL_SOURCE: &str = include_str!("js/path-url.js");
const EXPORTS: &[&str] = &[
    "posix",
    "win32",
    "join",
    "normalize",
    "basename",
    "dirname",
    "extname",
    "isAbsolute",
    "parse",
    "format",
    "common",
    "resolve",
    "relative",
    "toNamespacedPath",
    "toFileUrl",
    "fromFileUrl",
    "isGlob",
    "globToRegExp",
    "joinGlobs",
    "normalizeGlob",
    "match",
    "matchesGlob",
    "SEPARATOR",
    "DELIMITER",
    "SEPARATOR_PATTERN",
];

/// Exports constructed in the pristine realm and released before runtime drop.
#[derive(Default)]
pub(crate) struct PathState {
    pub(crate) api: Option<Persistent<Object<'static>>>,
}

// PathState stores no borrowed JS value, only a persistent handle whose
// lifetime is managed by NeoJs.
unsafe impl<'js> rquickjs::JsLifetime<'js> for PathState {
    type Changed<'to> = Self;
}

/// Build the path API and its realm-local file URL class before user code.
pub(crate) fn api<'js>(
    ctx: &Ctx<'js>,
    permissions: Permissions,
    allowed: &AllowedModules,
    cwd: Rc<RefCell<WorkingDirectory>>,
) -> Result<Object<'js>> {
    // Requiring a corresponding admitted module prevents `neo:path` from
    // reintroducing cwd access in an allowlist that excludes these capabilities.
    let can_read_cwd = (permissions.filesystem && allowed.allows(RuntimeModule::Fs))
        || ((permissions.process || permissions.stdio) && allowed.allows(RuntimeModule::Process));
    let read_cwd = Function::new(ctx.clone(), move |ctx: Ctx<'js>| -> Result<Value<'js>> {
        if !can_read_cwd {
            return Ok(Value::new_undefined(ctx.clone()));
        }
        match cwd.borrow().text() {
            Ok(path) => Ok(rquickjs::String::from_str(ctx.clone(), &path)?.into_value()),
            Err(_) => Ok(Value::new_null(ctx.clone())),
        }
    })?;
    // Use only the pure WHATWG host parser, not `Url`'s file path parser,
    // which loses empty path segments and drops the host before a drive.
    let parse_host = Function::new(ctx.clone(), |input: String| {
        if input.len() > 32 * 1024 {
            return None;
        }
        url::Host::parse(&input).ok().map(|host| host.to_string())
    })?;
    let factory: Function = ctx.eval(format!(
        "(function (hostWindows, readCwd, parseFileHost) {{ 'use strict'; \
         {GLOB_SOURCE}\n{FILE_URL_SOURCE}\n{PATH_SOURCE}\n}})"
    ))?;
    factory.call((cfg!(windows), read_cwd, parse_host))
}

/// The native export boundary; the cwd callback is not a module export.
pub(crate) struct PathModule;

impl ModuleDef for PathModule {
    fn declare<'js>(decl: &Declarations<'js>) -> Result<()> {
        for &name in EXPORTS {
            decl.declare(name)?;
        }
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        let state = ctx
            .userdata::<Rc<RefCell<PathState>>>()
            .ok_or(Error::Unknown)?;
        let api = state
            .borrow()
            .api
            .as_ref()
            .cloned()
            .ok_or(Error::Unknown)?
            .restore(ctx)?;
        for &name in EXPORTS {
            exports.export(name, api.get::<_, Value>(name)?)?;
        }
        Ok(())
    }
}
