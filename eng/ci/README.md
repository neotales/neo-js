# Native Windows CI

This directory owns the native Windows x64 MSVC gate for neo-js. The workflow
is `.github/workflows/windows-native.yml`; the script it calls is
`eng/ci/windows-native.ps1`, with static checks in
`eng/ci/check-windows-ci.ps1` and mutation self-tests in
`eng/ci/test-windows-ci-check.ps1`. The same checks can be reproduced on a
Windows machine without GitHub Actions.

> **Status: the targeted 101-test gate passed at `89b966c`; this proposed
> 102-test revision has not been dispatched on native Windows.** Run
> [44964164697](https://github.com/neotales/neo-js/actions/runs/44964164697) at
> `89b966c1382513518cb022f8007d4f9984b24ded` passed the targeted gate: the two
> registry suites (`8` and `9` tests), `neojs-core --test process` (`9`),
> `neojs-core --test fs` (`42`), the filtered validation unit test (`1`),
> `neojs --test process` (`19`), `neojs --test process_stdio` (`6`), and
> `neojs-os --test os` (`7`), followed by the workspace `--no-run` compile.
> This revision adds a second exact filtered pure unit test
> (`windows_copy_snapshot_validates_original_and_resolved_operands`), which
> would select `102` tests; it has not been dispatched, so there is no run
> result for it.
>
> Earlier successes: run
> [44956915195](https://github.com/neotales/neo-js/actions/runs/44956915195) at
> `cb685999` passed `100` tests (including `41` `neojs-core --test fs` tests
> and the one filtered validation unit test); run
> [44949569504](https://github.com/neotales/neo-js/actions/runs/44949569504) at
> `c23a21b` passed `99`; and run
> [44945036146](https://github.com/neotales/neo-js/actions/runs/44945036146) at
> `1cc7384` passed `97`. The full workspace test execution was not enabled.
>
> Earlier history: run
> [44845110559](https://github.com/neotales/neo-js/actions/runs/44845110559) at
> `51f3994` passed the earlier 58-test gate before the filesystem extension.
> Run [44936430027](https://github.com/neotales/neo-js/actions/runs/44936430027)
> at `2703526` failed in `neojs-core --test fs` before any `mkdir` test passed,
> and run [44943444444](https://github.com/neotales/neo-js/actions/runs/44943444444)
> at `8475b21` reported `FS_ALREADY_EXISTS` for the async recursive `mkdir`
> below an intermediate regular file. Those failures led to the fixed-label
> diagnostics described below. Reaching a hosted runner does not establish
> organization eligibility or billing beyond the runs recorded here.
>
> The filesystem suite is `42` Windows-selected integration tests, plus two
> exact filtered pure unit tests. The `101`-test run at `89b966c` executed the
> filesystem suite and the first of those filters. This revision adds the
> second filter, so the proposed gate selects `102` tests at source level;
> because it has not been dispatched, none of that is evidence the second
> filter passed on Windows.

## Runner environment

The repository is public (`neotales/neo-js`). The workflow targets
`windows-latest`, a GitHub-hosted standard runner, and does not use a
self-hosted or Linux runner label.

The only trigger is `workflow_dispatch`. The targeted gate has successful
manual dispatches, most recently run `44964164697` at `89b966c` (`101` tests),
but this proposed `102`-test revision has not been dispatched and a
`pull_request` trigger remains a separate reviewed change. Enabling it before
the current revision passes would queue the extended gate on every pull request
without current evidence.

Public repositories are normally eligible for GitHub-hosted standard runners,
but eligibility for this organization, Actions enablement, spending limits, and
concurrency limits are all outside the repository checkout. The recorded
dispatches reached a hosted Windows runner; that does not establish
eligibility or billing beyond those runs. If hosted Windows becomes unavailable, the manual
run will queue or fail; it will not silently fall back to a Linux runner.

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
| `cargo +1.87.0 test --locked --target x86_64-pc-windows-msvc -p neojs-core --test fs`                                           | 42            |
| `cargo +1.87.0 test --locked --target x86_64-pc-windows-msvc -p neojs-core --lib -- fs::tests::windows_copy_paths_are_rejected_before_io --exact` | 1 selected    |
| `cargo +1.87.0 test --locked --target x86_64-pc-windows-msvc -p neojs-core --lib -- fs::tests::windows_copy_snapshot_validates_original_and_resolved_operands --exact` | 1 selected    |
| `cargo +1.87.0 test --locked --target x86_64-pc-windows-msvc -p neojs --test process`                                           | 19            |
| `cargo +1.87.0 test --locked --target x86_64-pc-windows-msvc -p neojs --test process_stdio`                                     | 6             |
| `cargo +1.87.0 test --locked --target x86_64-pc-windows-msvc -p neojs-os --test os`                                             | 7             |
| `cargo +1.87.0 test --workspace --locked --target x86_64-pc-windows-msvc --no-run`                                              | compiles all workspace test targets, runs none |

Neither filtered unit-test row is accepted just because Cargo exits zero: for
each one the script captures that invocation's output and requires the libtest
summary `running 1 test` and `1 passed; 0 failed`, plus the exact test name. A
filter that selects zero tests, or more than one, fails the gate.

The counts are source-level counts after applying `cfg` gates; they are not a
claim that those tests passed. `neojs/tests/process.rs` contains 22 tests, of
which three are `#[cfg(unix)]`, so 19 run on Windows. `neojs-core/tests/fs.rs`
contains 57 tests; 15 are `#[cfg(unix)]`, so 42 are selected on Windows. The
42 include five Windows-only integration tests: one uses only owned
`tempfile` fixtures to check `stat`, `lstat`, `readDir`, and their sync forms
for regular files and directories; one creates a final file symlink (a reparse
point), verifies that `stat` follows it while `lstat` and `readDir` report the
entry as a link, and fails closed if the process lacks
`SeCreateSymbolicLinkPrivilege` or Developer Mode; one covers `mkdir` with a
missing parent and with an intermediate regular file in sync, async, and
recursive forms and logs the observed missing-parent and intermediate-file
codes with fixed labels before asserting them; the non-recursive
intermediate-file result is restricted to `FS_NOT_FOUND` or
`FS_NOT_DIRECTORY`, and the recursive result also accepts the observed
`FS_ALREADY_EXISTS`; one copies owned tempfile fixtures with both `copyFile`
and `copyFileSync` and checks no-clobber; and one is a gated basic round trip.
The previous Windows-only integration test that passed
root, namespace, UNC, device, and stream names to `copyFile` was removed; those
cases now run in the pure validation unit test
`fs::tests::windows_copy_paths_are_rejected_before_io`. A second no-I/O test,
`fs::tests::windows_copy_snapshot_validates_original_and_resolved_operands`,
drives the production `snapshot_copy_paths` helper for both original and
resolved operands. The native gate invokes each test as its own exact
`--lib -- ... --exact` filter and requires the one-test libtest summary for
each. Both tests parse paths only and perform no filesystem I/O. No junction is
created and no privileged directory link or host path outside the temp
directory is touched. Across its invocations the targeted gate selects 102
tests at source level (`8 + 9 + 9 + 42 + 1 + 1 + 19 + 6 + 7`). That is a
source-level count, not a pass count, and this proposed revision has not run
on Windows.

The filesystem suite invocation does not add `--nocapture`. The Windows `mkdir`
diagnostic writes its sanitized fixed-label codes directly to the process
stderr, so they are visible even when libtest captures ordinary test output; no
path or arbitrary error property is written, and the guard's exact `--test fs`
argument shape is unchanged.

The full workspace suite is opt-in:

```powershell
pwsh -File eng/ci/windows-native.ps1 -FullWorkspace
```

It is also available from the workflow's `workflow_dispatch` input
`full_workspace_tests`. It is off by default because the full workspace suite
has never run on Windows; the targeted gate passed `100` tests at `cb685999`
and `101` tests at `89b966c` (run `44964164697`). This proposed `102`-test
revision has not run on native Windows. The first full run is expected to
expose further failures, and turning that on for
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
  `$Target`; exactly eleven calls must exist. Exactly one call must have the
  complete `test --locked --target $Target -p neojs-core --test fs` argument
  shape, with no extra arguments such as `--list`, `--no-run`, or a filter.
  Exactly one call must have the complete
  `test --locked --target $Target -p neojs-core --lib -- fs::tests::windows_copy_paths_are_rejected_before_io --exact`
  shape, and exactly one call must have the complete
  `test --locked --target $Target -p neojs-core --lib -- fs::tests::windows_copy_snapshot_validates_original_and_resolved_operands --exact`
  shape. Any call that names the snapshot test must have exactly that shape:
  a second snapshot-named invocation with an extra argument such as
  `--ignored`, `--list`, `--no-run`, `--nocapture`, or a trailing filter is
  rejected even when one valid snapshot call exists and the total call count
  is unchanged. All three shapes must be direct statements in the `else` block
  of the `-CheckOnly` guard and must not be inside a function, script block,
  `if`, or loop. At runtime, each filtered unit test's captured output must
  report `running 1 test` and `1 passed; 0 failed`; a zero-test or multi-test
  match fails the gate;
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

The AST/YAML guarantee is source-level. The YAML checks are a strict contract
for this workflow's block structure, not a general YAML parser; they do not
handle every legal YAML form. The native gate separately checks the validator's
libtest summary, but neither check attests which `cargo` binary is found on
`PATH`, what that binary or its build scripts do, or code generated at run time.
They are not a sandbox.

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
- a removed `neojs-core --test fs` invocation;
- a duplicate `neojs-core --test fs` invocation;
- a `neojs-core --test fs` invocation with `--no-run`;
- a `neojs-core --test fs` invocation with `--list`;
- a `neojs-core --test fs` invocation with the wrong package;
- a `neojs-core --test fs` invocation under `if ($false)`;
- a `neojs-core --test fs` invocation moved outside the `CheckOnly` else;
- a `neojs-core --test fs` invocation inside an uninvoked function;
- a missing validation unit test invocation;
- a duplicate validation unit test invocation;
- a validation unit test invocation with `--no-run`;
- a validation unit test invocation with `--list`;
- a validation unit test invocation with a bogus filter;
- a validation unit test invocation with the wrong package;
- a validation unit test invocation under `if ($false)`;
- a validation unit test invocation moved outside the `CheckOnly` else;
- a validation unit test invocation inside an uninvoked function;
- a missing copy-snapshot unit test invocation;
- a duplicate copy-snapshot unit test invocation;
- a copy-snapshot unit test invocation with `--no-run`;
- a copy-snapshot unit test invocation with `--list`;
- a malformed duplicate copy-snapshot invocation with `--ignored`, replacing
  an unrelated call so the total remains eleven and one valid snapshot call
  remains; the self-test also asserts no call-count failure is reported;
- a malformed duplicate copy-snapshot invocation with `--nocapture`, replacing
  an unrelated call so the total remains eleven and one valid snapshot call
  remains; the self-test also asserts no call-count failure is reported;
- a copy-snapshot unit test invocation with a bogus filter;
- a copy-snapshot unit test invocation with the wrong package;
- a copy-snapshot unit test invocation under `if ($false)`;
- a copy-snapshot unit test invocation moved outside the `CheckOnly` else;
- a copy-snapshot unit test invocation inside an uninvoked function;
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

## Filesystem fixture safety

The filesystem integration tests that use fixtures create a fresh
`tempfile::TempDir` for each such test; a test that needs no fixture, such as
the permission/allowlist test, performs no filesystem operation and creates no
temp directory. Fixture tests operate only on paths below their own temp
directory. They do not write to the repository, the host's home directory, a
drive root, or an existing user file. Tests that use relative paths set the
runtime cwd to their own temp directory first; outside-managed-turn probes pass
absolute owned paths. Invalid-path operations therefore stay under an owned cwd
even if validation regresses. The Windows-only reparse test creates one final
file symlink inside its own temp directory. If the process lacks `SeCreateSymbolicLinkPrivilege` /
Developer Mode (Windows error 1314), or the filesystem does not support
symlinks, the test fails with that specific message; it does not print a
skipped result that could be mistaken for coverage. It never creates a
junction, an elevated directory link, or a link to an unowned path.

`tempfile::TempDir` attempts to remove its owned tree on `Drop`, but its `Drop`
implementation discards cleanup errors. A failed assertion therefore still
unwinds and attempts cleanup, and any cleanup failure can leave files only
under that test's own temp root. Nothing outside that root is deleted, and the
suite does not reuse a path after a cleanup failure. On a GitHub-hosted runner
the VM is discarded after the job.

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

After the current revision passes a manual dispatch, add the PR trigger in a
separate reviewed change:

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
- **`windows-latest` image drift:** the label floats. The 100-test run at
  `cb685999` and the 101-test run at `89b966c` succeeded on the floating image,
  but the image is not pinned. Pin it
  (`windows-2025` or the then-current stable image) in a separate change before
  treating a later result as comparable.
- **MSRV drift:** `Cargo.lock` and `Cargo.toml` are pinned, but a transitive
  crate can still declare a newer `rust-version` when the lock is regenerated.
  The `rustc -vV` guard catches the toolchain, not a bad lock.
- **Missing Windows standard library:** `rustc --print target-libdir` and the
  `Test-Path` check fail before cargo runs if the 1.87.0 MSVC target was not
  installed.
- **Wrong OS/process architecture:** the `RuntimeInformation` checks reject a
  32-bit process and an x64 process emulated on a non-x64 OS before any test
  runs. They do not assert the physical CPU model.
- **Symlink capability missing:** the Windows reparse-point test fails closed
  with the `SeCreateSymbolicLinkPrivilege`/Developer Mode or unsupported-
  filesystem message. It does not skip silently and does not count a skipped
  test as coverage. No junction is created.
- **Full workspace failures:** expected on the first opt-in run. They are
  findings about Windows behavior, not evidence that the targeted gate is
  wrong.
- **No formatting, clippy, or docs gate:** this workflow is deliberately
  scoped to native build and runtime verification. Adding a Windows clippy gate
  before the extended gate passes would mix new lint findings with unknown
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
