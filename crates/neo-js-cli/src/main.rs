use std::{
    io::Write,
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::Parser;
use neojs_core::{NeoJs, Permissions, RuntimeOptions};

#[derive(Parser)]
#[command(
    name = "neo-js",
    version,
    about = "Run JavaScript with the NeoTales QuickJS host"
)]
struct Args {
    /// Grant the host user's filesystem authority, including deletion, renaming, and replacement.
    #[arg(long)]
    allow_fs: bool,
    /// Permit scripts to read environment variables.
    #[arg(long)]
    allow_env: bool,
    /// Permit scripts to read the host standard input through neo:process.
    /// This is the narrow alternative to --allow-run; --allow-run also grants
    /// stdin because it already permits arbitrary child programs. Either flag
    /// also makes neo:process importable for its stdout/stderr writers,
    /// runtime-local working directory, and host process elevation status.
    /// Reads are synchronous and can block until input or end of file.
    #[arg(long)]
    allow_stdio: bool,
    /// Permit synchronous program execution with the current user's privileges.
    #[arg(long)]
    allow_run: bool,
    /// Permit synchronous Windows Registry reads, writes, and deletes.
    #[arg(long)]
    allow_registry: bool,
    /// Permit synchronous HTTP and HTTPS downloads through neo:fetch. This is
    /// broad outbound network authority, not a sandbox or SSRF protection;
    /// downloading a file also requires --allow-fs.
    #[arg(long)]
    allow_net: bool,
    /// Permit synchronous in-memory databases through neo:sqlite.
    #[arg(long)]
    allow_sqlite: bool,
    /// Print embedded third-party source notices and exit. The text covers
    /// adapted source code, not every dependency. No script is read and no
    /// permission is required.
    #[arg(long, conflicts_with = "script")]
    third_party_notices: bool,
    /// JavaScript or TypeScript module to execute. Required unless
    /// --third-party-notices is used. The CLI always reads this entry file; a
    /// `.ts` or `.mts` entry is stripped before evaluation.
    #[arg(required_unless_present = "third_party_notices")]
    script: Option<PathBuf>,
}

fn main() -> ExitCode {
    let args = Args::parse();

    if args.third_party_notices {
        return print_third_party_notices();
    }

    let Some(script) = args.script.as_deref() else {
        // Clap enforces this unless `--third-party-notices` is present, which
        // returned above. Keep a usage failure instead of panicking if that
        // invariant ever changes.
        eprintln!("neo-js: a script is required unless --third-party-notices is used");
        return ExitCode::from(2);
    };

    match execute(script, &args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(errors) => {
            for error in errors {
                eprintln!("neo-js: {error}");
            }
            ExitCode::FAILURE
        }
    }
}

/// Write the embedded notices to standard output.
///
/// This does not construct a runtime, read a script, or consult any permission.
fn print_third_party_notices() -> ExitCode {
    let mut stdout = std::io::stdout().lock();
    match stdout
        .write_all(neojs_core::third_party_notices().as_bytes())
        .and_then(|()| stdout.flush())
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("neo-js: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Run the entry module, then any tests it registered with `neo:test`.
///
/// Tests registered before an ordinary module error still run. An abandoned
/// runtime refuses to execute them and reports `NEO_MODULE_ABORTED` alongside
/// the module error, rather than running code with unfinished work.
fn execute(requested_script: &Path, args: &Args) -> Result<(), Vec<String>> {
    let script =
        std::fs::canonicalize(requested_script).map_err(|error| vec![error.to_string()])?;
    let source = std::fs::read_to_string(&script).map_err(|error| vec![error.to_string()])?;
    let permissions = Permissions {
        filesystem: args.allow_fs,
        environment: args.allow_env,
        stdio: args.allow_stdio,
        process: args.allow_run,
        registry: args.allow_registry,
        network: args.allow_net,
        sqlite: args.allow_sqlite,
    };
    // Compose the optional crates explicitly. Core links none of them: this
    // registration only makes the exact `neo:os`, `neo:sqlite`, and scheduler
    // names available, and the permission above still gates every SQLite
    // import. `neo:os` needs no permission flag and installs the `os` global
    // exactly when the runtime admits the module. The scheduler generators
    // need no permission flag and install no global.
    let options = neojs_os::with_os(RuntimeOptions::new())
        .and_then(neojs_sqlite::with_sqlite)
        .and_then(neojs_schedulers::with_schedulers)
        .map_err(|error| vec![error.to_string()])?;
    let js =
        NeoJs::new_with_options(permissions, options).map_err(|error| vec![error.to_string()])?;
    let identity = script
        .to_str()
        .ok_or_else(|| vec!["canonical script path is not valid UTF-8".to_owned()])?;
    let name = requested_script
        .to_str()
        .ok_or_else(|| vec!["script path is not valid UTF-8".to_owned()])?;

    let mut errors = Vec::new();
    if let Err(error) = js.run_module_with_identity(name, identity, &source) {
        errors.push(error);
    }
    match js.run_pending_tests() {
        Ok(Some(report)) if !report.all_passed() => errors.push(format!(
            "{} of {} test(s) failed",
            report.failed, report.total
        )),
        Ok(_) => {}
        Err(error) => errors.push(error),
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}
