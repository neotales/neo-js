# Native Windows CI

This directory owns the native Windows x64 MSVC gate for neo-js. The workflow
is `.github/workflows/windows-native.yml`; the script it calls is
`eng/ci/windows-native.ps1`, with static checks in
`eng/ci/check-windows-ci.ps1` and mutation self-tests in
`eng/ci/test-windows-ci-check.ps1`. The same checks can be reproduced on a
Windows machine without GitHub Actions.

> **Status: dispatched, not yet passing.** Two dispatches are recorded and both
> failed; no native gate has passed. Run
> [44841666909](https://github.com/neotales/neo-js/actions/runs/44841666909) at
> `72ed60b` ran the registry suites (`8` and `9` tests),
> `neojs-core --test process` (`9`), and `neojs --test process` (`19`) green,
> then failed `neojs/tests/process_stdio.rs` with `4 passed, 1 failed`:
> `cwd_and_chdir_are_runtime_local_and_coordinate_with_filesystem` compared the
> initial `cwd()` text to `std::fs::canonicalize` of the working directory. The
> gate stopped there, so the `neojs-os` suite and the workspace `--no-run`
> compile were not reached. The earlier run
> [44836480297](https://github.com/neotales/neo-js/actions/runs/44836480297)
> also failed. The current correction for the `process_stdio` failure has not
> run on Windows. Nothing here is evidence that the Windows build or runtime
> passes.

## Runner environment

The repository is public (`neotales/neo-js`). The workflow targets
`windows-latest`, a GitHub-hosted standard runner, and does not use a
self-hosted or Linux runner label.

The only trigger is `workflow_dispatch`. A `pull_request` trigger is
deliberately not active yet: a hosted Windows runner was reached in both
recorded dispatches, but no dispatch has passed the gate, so a PR trigger would
queue a job on every pull request before one has. Enable it after one
successful manual dispatch, in a reviewed change.

Public repositories are normally eligible for GitHub-hosted standard runners,
but eligibility for this organization, Actions enablement, spending limits, and
concurrency limits are all outside the repository checkout. Both recorded
dispatches reached a hosted Windows runner; that does not establish eligibility
or billing beyond those runs, and neither passed the gate. If hosted Windows
becomes unavailable, the manual run will queue or fail; it will not silently
fall back to a Linux runner.

The separate self-hosted runner pool advertises only `linux-amd64` and
`ubuntu-latest`. It is not used by this workflow.

## Pinned inputs

Actions are pinned to full commit SHAs. The ref after the SHA is the upstream
tag or branch it was resolved from.

| Action                   | Ref       | Commit SHA                                 | Source                                                                      |
| ------------------------ | --------- | ------------------------------------------ | --------------------------------------------------------------------------- |
| `actions/checkout`       | `v7.0.1`  | `3d3c42e5aac5ba805825da76410c181273ba90b1` | `git ls-remote https://github.com/actions/checkout.git refs/tags/v7.0.1`    |
| `dtolnay/rust-toolchain` | `1.87.0`  | `4b54d2ecfeb0fc349341b7929a580d09d9e2479b` | `git ls-remote https://github.com/dtolnay/rust-toolchain.git refs/heads/1.87.0` |
| `Swatinem/rust-cache`    | `v2.9.3`  | `d1527c8f67ef9af357290475d4af4b0fab8f12e0` | `git ls-remote https://github.com/Swatinem/rust-cache.git 'refs/tags/v2.9.3*'` (peeled tag) |

`dtolnay/rust-toolchain`'s `1.87.0` ref is a branch whose `action.yml`
hardcodes `toolchain: 1.87.0` and declares `targets`, `target`, and `components`
inputs. Only `toolchain` is unsupported. The workflow intentionally passes no
`with:` block at all because the pinned ref already selects 1.87.0 and this gate
needs no other inputs. The static guard enforces that workflow policy by failing
closed on block, inline, or quoted `with:` forms; it is not claiming that
`targets`, `target`, or `components` are invalid. Pinning the commit also makes
the workflow independent of later branch movement.
`Swatinem/rust-cache`'s `v2.9.3` tag is annotated: the tag object is
`2201cc584a476d1b508be977bbe4b7d1c7a52e9c` and it points at the commit in the
table. `actions/checkout`'s `v7.0.1` ref points directly at its commit. These
were checked on 2026-11-20 with `git ls-remote` and the GitHub API.

Rust is pinned to `1.87.0`, the workspace MSRV in `Cargo.toml`. The script
invokes `cargo +1.87.0` and `rustc +1.87.0`, so rustup must have that toolchain
installed; the workflow action installs it. Every cargo invocation passes
`--target x86_64-pc-windows-msvc` explicitly and uses `--locked`, so
`Cargo.lock` decides dependency versions. No `@latest`, floating channel, or
unversioned tool is used. The rustc host check is not treated as proof that
cargo would select that target.

## What runs

```powershell
pwsh -File eng/ci/windows-native.ps1
```

The script fails unless the host reports:

- Windows (`$env:OS -eq 'Windows_NT'`)
- An x64 OS and an x64 PowerShell process, read through
  `RuntimeInformation.OSArchitecture` and `ProcessArchitecture`. An x64 process
  emulated on a non-x64 OS is rejected. `PROCESSOR_ARCHITECTURE` is printed for
  diagnostics but is not used as evidence of the physical CPU.
- `rustc -vV` release `1.87.0` and host `x86_64-pc-windows-msvc`
- The `x86_64-pc-windows-msvc` standard library is installed at the path
  returned by `rustc --print target-libdir --target x86_64-pc-windows-msvc`

It then runs, in order. Every command includes the explicit target:

| Command                                                                                                                        | Windows tests |
| ------------------------------------------------------------------------------------------------------------------------------ | ------------- |
| `cargo +1.87.0 test --locked --target x86_64-pc-windows-msvc -p neojs-core --test winregistry -- --test-threads=1`              | 8             |
| `cargo +1.87.0 test --locked --target x86_64-pc-windows-msvc -p neojs-core --test winregistry_windows -- --test-threads=1`      | 9             |
| `cargo +1.87.0 test --locked --target x86_64-pc-windows-msvc -p neojs-core --test process`                                      | 9             |
| `cargo +1.87.0 test --locked --target x86_64-pc-windows-msvc -p neojs --test process`                                           | 19            |
| `cargo +1.87.0 test --locked --target x86_64-pc-windows-msvc -p neojs --test process_stdio`                                     | 6             |
| `cargo +1.87.0 test --locked --target x86_64-pc-windows-msvc -p neojs-os --test os`                                             | 7             |
| `cargo +1.87.0 test --workspace --locked --target x86_64-pc-windows-msvc --no-run`                                              | compiles all workspace test targets, runs none |

The counts are source-level counts after applying `cfg` gates; they are not a
claim that those tests passed. `neojs/tests/process.rs` contains 22 tests, of
which three are `#[cfg(unix)]`, so 19 run on Windows. The targeted suites total
58 Windows tests.

The full workspace suite is opt-in:

```powershell
pwsh -File eng/ci/windows-native.ps1 -FullWorkspace
```

It is also available from the workflow's `workflow_dispatch` input
`full_workspace_tests`. It is off by default because the full workspace suite
has never run on Windows, and only some targeted suites have: the observed run
stopped before the `neojs-os` suite, which neither observed run reached. The
first full run is expected to expose further failures, and turning that on for
every pull request would conflate "the native build works" with "every
unverified Windows behavior is correct".

Compiling the workspace compiles the native MSVC dependencies, including the
bundled QuickJS C sources used by `rquickjs`, the bundled SQLite C sources used
by `rusqlite`, and `windows-sys` bindings. This is a native x64 MSVC build, not
a GNU cross-compile.

## Static checks

`eng/ci/check-windows-ci.ps1` runs its static checks on any host with
PowerShell 7. On Windows it also runs the nested `-CheckOnly` probe, which
requires the pinned Rust toolchain and MSVC target as described below. It
checks:

- the workflow trigger and permissions with a strict indentation contract, not
  loose regexes: the `on` block must contain exactly `workflow_dispatch`, and
  the `permissions` block must contain exactly `contents: read`. Column-zero
  comments and blank lines do not end either block, because they do not end a
  YAML mapping. Extra triggers, extra permissions, job-level `permissions:`
  keys, quoted permission keys (including `"contents"`), and inline scalar
  blocks are rejected;
- the runner label, timeout, concurrency, and `persist-credentials`;
- that all three action references are 40-hex commit SHAs matching the table
  above;
- that the workflow does not reference `secrets.*`;
- that the pinned `dtolnay/rust-toolchain` step defines no `with:` at all,
  whether block, inline, quoted key, or nested quoted `toolchain`. The action
  supports `targets`, `target`, and `components`, but this workflow needs none
  of them and the pinned ref selects 1.87.0; the guard rejects any `with:` by
  policy, not because those inputs are unsupported;
- the PowerShell AST of `windows-native.ps1`: it enumerates command
  invocations, not lines. It normalizes Windows `\` and Unix `/` separators and
  compares the executable basename case-insensitively. Exactly one literal
  `cargo` command is allowed: `& cargo '+1.87.0' @Arguments` inside
  `Invoke-Cargo`. Direct `cargo`, `Cargo`, `cargo.exe`, path-qualified
  `C:\tools\cargo.exe` or `/usr/bin/cargo`, path-qualified shells such as
  `C:\Windows\System32\cmd.exe` or `/bin/sh`, dynamic command expressions such
  as `& $tool` or `& ('cargo')`, shell/dynamic wrappers, and explicit cargo
  aliases or functions are rejected. A path-qualified cargo is rejected even
  when it replaces the wrapper;
- every `Invoke-Cargo` call must contain exactly one `--target` followed by
  `$Target`;
- `neojs-core/tests/winregistry_windows.rs` exists, still reserves
  `HKCU\Software\neo-js-tests-*`, and does not mention `HKLM`,
  `HKEY_LOCAL_MACHINE`, or `HKEY_USERS`. This is the platform-independent
  check for the Windows-specific dependency that `-CheckOnly` reads;
- the fail-closed probe `windows-native.ps1 -CheckOnly`: nonzero on a
  non-Windows host, zero on Windows.

On Windows, `-CheckOnly` is not fully host-independent: it requires rustup
toolchain `1.87.0` and the `x86_64-pc-windows-msvc` standard library installed.
Without them it fails closed. The self-test baseline on Windows therefore
needs the same toolchain as the workflow.

The guarantee is source-level. The YAML checks are a strict contract for this
workflow's block structure, not a general YAML parser; they do not handle every
legal YAML form. The guard also does not attest which `cargo` binary is found on
`PATH`, what that binary or its build scripts do, or code generated at run time.
It is not a sandbox.

Run it locally with:

```powershell
pwsh -File eng/ci/check-windows-ci.ps1
```

The workflow runs the same check after installing Rust and before the tests,
then runs the mutation self-test before the native gate.

`eng/ci/test-windows-ci-check.ps1` is the checked-in mutation self-test. It
copies the workflow, scripts, and `neojs-core/tests/winregistry_windows.rs` to
an isolated fixture root and asserts that the baseline is accepted while
mutations are rejected with their specific diagnostics. The registry source is
copied because `windows-native.ps1 -CheckOnly` reads it on Windows; the
missing-source fixture is therefore a platform-independent simulation of that
dependency. Rejected mutations:

- a direct `cargo test`;
- Windows path-qualified cargo (`C:\tools\cargo.exe`);
- Windows path-qualified `cmd.exe`;
- Unix path-qualified `/bin/sh`;
- environment-expanded cargo (`& "$env:CARGO"`);
- a path-qualified cargo replacing the wrapper command;
- a removed `--target`;
- a cargo alias;
- a floating action ref;
- a policy-rejected block `with: toolchain: 1.87.0` on
  `dtolnay/rust-toolchain` (the `toolchain` input itself is also unsupported);
- a policy-rejected block `with:` containing quoted `"toolchain": "1.87.0"`;
- a policy-rejected inline `with: { "toolchain": "1.87.0" }`;
- a policy-rejected quoted `"with": { "toolchain": "1.87.0" }`;
- a `release` trigger;
- a `workflow_call` trigger;
- a column-zero comment hiding a following `release` trigger;
- a column-zero comment hiding a following `'id-token': write` permission;
- a job-level `permissions: write-all`;
- a quoted job-level `'permissions': write-all`;
- a quoted `"contents": read` permission key;
- a `contents: write` permission;
- a missing registry test source.

The self-test removes only its own generated root under the OS temp directory,
after checking that the path is still under that temp directory and has the
`neojs-windows-ci-selftest-*` prefix. It does not remove anything else.

Run it with:

```powershell
pwsh -File eng/ci/test-windows-ci-check.ps1
```

## Registry safety

`neojs-core/tests/winregistry_windows.rs` reserves a unique key under
`HKCU\Software\neo-js-tests-<pid>-<nanos>-<id>` for every test and deletes only
the exact names it created, without recursive deletion. The script enforces
three properties around that design:

1. The test source still contains the `Software\neo-js-tests-` reservation.
2. The test source does not mention `HKLM`, `HKEY_LOCAL_MACHINE`, or
   `HKEY_USERS`.
3. There are no matching `neo-js-tests-*` keys before the run, and none after
   it. A leftover key fails the job instead of being reused or ignored.

The check after the run executes even when a test fails. On a GitHub-hosted
runner the whole VM is discarded afterwards, but the check still catches a
fixture that leaked within the run.

No user-supplied secrets, personal access tokens, or vault values are
referenced: the workflow references no `secrets.*`, uses no
`pull_request_target`, and has `permissions: contents: read`. `actions/checkout`
does receive the automatically supplied, repository-scoped `GITHUB_TOKEN` to
fetch the commit; `persist-credentials: false` only stops checkout from writing
that token into the git configuration for later steps. It does not mean no
credential is used. The cache step is written to skip fork pull requests if a
PR trigger is enabled later, because that code is untrusted.

## Enabling pull requests

After one successful manual dispatch has confirmed that the hosted Windows
runner exists, add the PR trigger in a separate reviewed change:

```yaml
on:
  pull_request:
  workflow_dispatch:
```

Do not add it before the manual run. A `windows-latest` job with no available
runner queues rather than fails fast, so an unproven PR trigger turns every
pull request into a stuck check.

## Failure modes and limits

- **No hosted Windows runner:** the job queues or fails. This cannot be
  distinguished from a slow runner without repository administration access.
- **Actions disabled or over budget:** the workflow does not start. There is no
  fallback to the Linux self-hosted pool.
- **`windows-latest` image drift:** the label floats. Once the first run
  succeeds, pin the runner image (`windows-2025` or the then-current stable
  image) in a separate change.
- **MSRV drift:** `Cargo.lock` and `Cargo.toml` are pinned, but a transitive
  crate can still declare a newer `rust-version` when the lock is regenerated.
  The `rustc -vV` guard catches the toolchain, not a bad lock.
- **Missing Windows standard library:** `rustc --print target-libdir` and the
  `Test-Path` check fail before cargo runs if the 1.87.0 MSVC target was not
  installed.
- **Wrong OS/process architecture:** the `RuntimeInformation` checks reject a
  32-bit process and an x64 process emulated on a non-x64 OS before any test
  runs. They do not assert the physical CPU model.
- **Full workspace failures:** expected on the first opt-in run. They are
  findings about Windows behavior, not evidence that the targeted gate is
  wrong.
- **No formatting, clippy, or docs gate:** this workflow is deliberately
  scoped to native build and runtime verification. Adding a Windows clippy gate
  before the first successful run would mix new lint findings with unknown
  runtime failures.
- **No native MSVC build or run was possible on the Linux development host,
  and the Rust 1.87 Windows target is not installed there.** The workflow and
  script are syntax-checked and the Linux suites are run locally, but only CI
  can prove the native Windows path.

## Changing the pin

1. Resolve the new ref with `git ls-remote`, and for annotated tags dereference
   with `^{}`. Do not copy a SHA from a blog post or a floating branch name.
2. Update the table above and the `uses:` line together.
3. Keep `persist-credentials: false`, `permissions: contents: read`, the
   timeout/concurrency block, and the explicit `--target` on every cargo
   command.
4. Re-check that no step needs a user secret. If it does, stop: this persona
   does not read one, and the value comes from a store at run time.
