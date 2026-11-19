---
title: "Host permissions and module access"
description: "CLI flags, module allowlists, and the host authority they grant."
---

`Permissions` controls host operations, while `RuntimeOptions` can restrict
which `neo:` modules the runtime constructs. These controls are independent.
An explicit module allowlist does not remove authority granted by a permission
through a different path:

- `neo:hash.digestFile` reads files with filesystem permission even if `neo:fs`
  is not allowed.
- `neo:fetch.download` writes files when filesystem and network permissions
  are granted even if `neo:fs` is not allowed.
- Filesystem permission enables relative module imports independently of the
  `neo:fs` module.
- Process permission lets `neo:exec` launch programs with the host user's
  authority, regardless of other permission flags or module selections.
- `neo:path` can use the runtime cwd as an implicit base in the host dialect.
  This fallback requires filesystem permission with `neo:fs` admitted,
  or process or stdio permission with `neo:process` admitted. See the
  [path guide](/guides/modules/path/#runtime-working-directory).
- `console` is present in every runtime.

neo-js is not a sandbox. For untrusted scripts, isolate the process with
operating-system controls.

## CLI flags

The CLI reads the entry script regardless of `--allow-fs`. That flag grants
scripts the host user's filesystem authority, including reading, writing,
deleting, renaming, and replacing files or directories as permitted by the
OS. It is not a sandbox; grant it only to scripts you trust. `--allow-env`
permits the script to read any environment variable. Neither capability is
enabled by default.

`--allow-registry` permits the synchronous `neo:winregistry` module. On
Windows this is full read, write, and delete authority under the current
process token, not a sandbox; a script can change any key or value the user can,
including keys that run programs at sign-in. Grant it only to scripts you
trust. On a non-Windows host the module can be imported with this permission,
but `isAvailable()` is `false` and every operation reports that registry access
is unsupported. Without the flag the module and all of its functions are
absent.

`--allow-run` permits the synchronous `neo:exec` module, including its
owned-child `spawn` handles. The child process runs with the current user's
full host authority: it can read files, read the environment, and use the
network even when `--allow-fs` and `--allow-env` are absent. Enable this flag
only for scripts you trust.

`--allow-stdio` permits scripts to read the host's standard input through
`neo:process`. That consumes input the embedder or an interactive user may be
using, so it is an explicit opt-in; reads are synchronous and can block until
input or end of file. `--allow-run` also grants this access, because it already
permits arbitrary child programs under the host user's authority. `--allow-stdio`
is the narrower choice when only standard I/O is needed. `--allow-run` or
`--allow-stdio` makes `neo:process` importable for its stdout and stderr writers
and its runtime-local working directory; without one of them the module is
absent.

`--allow-net` permits the synchronous `neo:fetch` module. It grants broad
outbound HTTP and HTTPS authority under the current user's network access,
including loopback, link-local, private, and cloud-metadata addresses. It is
not a sandbox, allowlist, or SSRF defense; a redirect may change the host,
port, or scheme, including an HTTPS-to-HTTP downgrade. Downloading a file also
requires `--allow-fs`, because the module writes the destination; without it
`download` throws `FETCH_PERMISSION` before any file or network activity. Enable `--allow-net` only for scripts
you trust with the user's network reachability.

## Conditional exports and globals

`neo:hash` is importable without a permission flag. Without `--allow-fs` the module does not export `digestFile`; `digest` remains available. Without `--allow-net`, `neo:fetch` is not importable. With `--allow-net` but not `--allow-fs`, `neo:fetch` is importable but `download` throws `FETCH_PERMISSION`.

The globals `os`, `env`, and `fs` are present only if the allowlist admits their matching modules. `env` and `fs` also require their corresponding permissions. `os` needs no flag but discloses bounded host OS metadata. `console` always exists. See [Public host globals](/api/globals/) for signatures.

Other built-in modules require no permission flag. The allowlist still applies to every module. TypeScript does not check runtime permissions or allowlists.

## Scheduler allowlists

None of the scheduler modules requires a permission flag. An explicit
allowlist checks each exact name independently:

| Import specifier            | Rust allowlist entry              |
| --------------------------- | --------------------------------- |
| `neo:os/schedulers/systemd` | `RuntimeModule::SchedulerSystemd` |
| `neo:os/schedulers/windows` | `RuntimeModule::SchedulerWindows` |
| `neo:scheduler`             | `RuntimeModule::Scheduler`        |

`RuntimeModule::Os` admits `neo:os`, not its scheduler specifiers. The
deprecated `neo:scheduler` exposes both generators through the legacy name
only. It does not admit either platform-specific import.
