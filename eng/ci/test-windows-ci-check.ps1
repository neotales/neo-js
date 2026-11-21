#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Mutation self-tests for eng/ci/check-windows-ci.ps1.

.DESCRIPTION
    Copies the workflow, scripts, and the Windows registry test source to a
    temporary directory, applies one mutation per fixture, and asserts that
    check-windows-ci.ps1 rejects it with the expected diagnostic. The baseline
    copy must be accepted.

    The registry test source is included because windows-native.ps1 -CheckOnly
    reads it on Windows and check-windows-ci.ps1 now checks it on every host.
    That makes the missing-source fixture a platform-independent simulation of
    the Windows-specific dependency.

    Covered bypasses: direct cargo, Windows path-qualified cargo, Windows and
    Unix path-qualified shells, an environment-expanded cargo command, a
    path-qualified wrapper command, a missing --target, and missing,
    duplicated, no-run, list, wrong-package, conditional, outside-else, or
    uninvoked-function neojs-core filesystem and validation invocations, a
    cargo alias, a floating action ref, policy-rejected `with:` blocks on
    dtolnay including quoted keys, extra `release`/`workflow_call` triggers,
    column-zero comments
    hiding a following trigger or permission, quoted or unquoted job-level
    permissions, quoted permission keys, `id-token: write`, `contents: write`,
    and a missing registry test source.

    This is a source/AST check, not a sandbox. It does not attest the cargo
    binary on PATH or anything cargo/rustc/build scripts execute at runtime.
#>
[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$sourceFiles = @(
    '.github/workflows/windows-native.yml',
    'eng/ci/windows-native.ps1',
    'eng/ci/check-windows-ci.ps1',
    'neojs-core/tests/winregistry_windows.rs'
)
$tempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ('neojs-windows-ci-selftest-' + [guid]::NewGuid().ToString('N'))
$pwshPath = (Get-Process -Id $PID).Path
$failures = [System.Collections.Generic.List[string]]::new()

function New-Fixture {
    param([Parameter(Mandatory = $true)][string]$Name)

    $root = Join-Path $tempRoot $Name
    foreach ($relative in $sourceFiles) {
        $source = Join-Path $repoRoot $relative
        $destination = Join-Path $root $relative
        $directory = Split-Path -Parent $destination
        New-Item -ItemType Directory -Path $directory -Force | Out-Null
        Copy-Item -LiteralPath $source -Destination $destination
    }
    return $root
}

function Invoke-Check {
    param([Parameter(Mandatory = $true)][string]$Root)

    $check = Join-Path $Root 'eng/ci/check-windows-ci.ps1'
    $output = & $pwshPath -NoProfile -NonInteractive -File $check 2>&1
    return [pscustomobject]@{
        ExitCode = $LASTEXITCODE
        Output   = (($output | Out-String).Trim())
    }
}

function Replace-First {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Find,
        [Parameter(Mandatory = $true)][string]$Replacement
    )

    $content = Get-Content -LiteralPath $Path -Raw
    $index = $content.IndexOf($Find, [System.StringComparison]::Ordinal)
    if ($index -lt 0) {
        throw "fixture setup failed: '$Find' not found in $Path"
    }
    $updated = $content.Substring(0, $index) + $Replacement + $content.Substring($index + $Find.Length)
    Set-Content -LiteralPath $Path -Value $updated -NoNewline
}

function Insert-AfterLine {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Line,
        [Parameter(Mandatory = $true)][string]$Insert
    )

    Replace-First -Path $Path -Find $Line -Replacement ($Line + "`n" + $Insert)
}

function Assert-Accepted {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$Expected
    )

    try {
        $result = Invoke-Check $Root
    }
    catch {
        [void]$failures.Add("$Name`: check threw: $_")
        return
    }
    if ($result.ExitCode -ne 0) {
        [void]$failures.Add("$Name`: baseline was rejected: $($result.Output)")
        return
    }
    if (-not $result.Output.Contains($Expected)) {
        [void]$failures.Add("$Name`: baseline did not print '$Expected': $($result.Output)")
        return
    }
    Write-Host "  accepted: $Name (matched: $Expected)"
}

function Assert-Rejected {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string[]]$Expected
    )

    try {
        $result = Invoke-Check $Root
    }
    catch {
        [void]$failures.Add("$Name`: check threw: $_")
        return
    }
    if ($result.ExitCode -eq 0) {
        [void]$failures.Add("$Name`: check unexpectedly passed")
        return
    }
    foreach ($diagnostic in $Expected) {
        if (-not $result.Output.Contains($diagnostic)) {
            [void]$failures.Add("$Name`: missing expected diagnostic '$diagnostic': $($result.Output)")
            return
        }
    }
    Write-Host "  rejected: $Name"
}

function Remove-OwnedFixtureRoot {
    if (-not (Test-Path -LiteralPath $tempRoot)) {
        return
    }
    $resolved = [System.IO.Path]::GetFullPath($tempRoot)
    $tempBase = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath())
    if (-not $tempBase.EndsWith([System.IO.Path]::DirectorySeparatorChar)) {
        $tempBase += [System.IO.Path]::DirectorySeparatorChar
    }
    $isUnderTemp = $resolved.StartsWith($tempBase, [System.StringComparison]::OrdinalIgnoreCase)
    $isOwnedName = (Split-Path -Leaf $resolved) -like 'neojs-windows-ci-selftest-*'
    if (-not ($isUnderTemp -and $isOwnedName)) {
        [Console]::Error.WriteLine("refusing to remove non-owned fixture root: $resolved")
        return
    }
    Remove-Item -LiteralPath $resolved -Recurse -Force
    Write-Host "removed owned fixture root: $resolved"
}

$exitCode = 0
try {
    $baselineRoot = New-Fixture -Name 'baseline'
    Assert-Accepted -Name 'baseline' -Root $baselineRoot -Expected 'windows-ci static checks: ok'

    $directCargoRoot = New-Fixture -Name 'direct-cargo'
    $directCargoScript = Join-Path $directCargoRoot 'eng/ci/windows-native.ps1'
    Insert-AfterLine -Path $directCargoScript -Line '        $registryTestsRan = $true' -Insert '        cargo test --locked --target $Target -p neojs-core --test process'
    Assert-Rejected -Name 'direct cargo' -Root $directCargoRoot -Expected @(
        'expected exactly one literal cargo invocation (inside Invoke-Cargo), found 2',
        'cargo test --locked --target $Target'
    )

    $windowsCargoRoot = New-Fixture -Name 'windows-path-cargo'
    $windowsCargoScript = Join-Path $windowsCargoRoot 'eng/ci/windows-native.ps1'
    Insert-AfterLine -Path $windowsCargoScript -Line '        $registryTestsRan = $true' -Insert '        & ''C:\tools\cargo.exe'' test --locked --target $Target'
    Assert-Rejected -Name 'Windows path-qualified cargo' -Root $windowsCargoRoot -Expected @(
        'expected exactly one literal cargo invocation (inside Invoke-Cargo), found 2',
        'C:\tools\cargo.exe'
    )

    $windowsShellRoot = New-Fixture -Name 'windows-path-shell'
    $windowsShellScript = Join-Path $windowsShellRoot 'eng/ci/windows-native.ps1'
    Insert-AfterLine -Path $windowsShellScript -Line '        $registryTestsRan = $true' -Insert '        & ''C:\Windows\System32\cmd.exe'' /c echo bypass'
    Assert-Rejected -Name 'Windows path-qualified shell' -Root $windowsShellRoot -Expected @(
        'forbidden dynamic/shell command in windows-native.ps1',
        'C:\Windows\System32\cmd.exe'
    )

    $unixShellRoot = New-Fixture -Name 'unix-path-shell'
    $unixShellScript = Join-Path $unixShellRoot 'eng/ci/windows-native.ps1'
    Insert-AfterLine -Path $unixShellScript -Line '        $registryTestsRan = $true' -Insert '        & ''/bin/sh'' -c ''echo bypass'''
    Assert-Rejected -Name 'Unix path-qualified shell' -Root $unixShellRoot -Expected @(
        'forbidden dynamic/shell command in windows-native.ps1',
        '/bin/sh'
    )

    $envCargoRoot = New-Fixture -Name 'environment-cargo'
    $envCargoScript = Join-Path $envCargoRoot 'eng/ci/windows-native.ps1'
    Insert-AfterLine -Path $envCargoScript -Line '        $registryTestsRan = $true' -Insert '        & "$env:CARGO" test --locked --target $Target'
    Assert-Rejected -Name 'environment-expanded cargo command' -Root $envCargoRoot -Expected @(
        'dynamic command invocation cannot be proven not to invoke cargo',
        '$env:CARGO'
    )

    $wrapperRoot = New-Fixture -Name 'path-qualified-wrapper'
    $wrapperScript = Join-Path $wrapperRoot 'eng/ci/windows-native.ps1'
    Replace-First -Path $wrapperScript -Find "& cargo '+1.87.0' @Arguments" -Replacement "& 'C:\tools\cargo.exe' '+1.87.0' @Arguments"
    Assert-Rejected -Name 'path-qualified wrapper command' -Root $wrapperRoot -Expected @(
        "cargo invocation command is not the literal 'cargo'",
        'C:\tools\cargo.exe'
    )

    $targetRoot = New-Fixture -Name 'missing-target'
    $targetScript = Join-Path $targetRoot 'eng/ci/windows-native.ps1'
    Replace-First -Path $targetScript -Find "'--target', `$Target, '-p', 'neojs-core', '--test', 'winregistry'," -Replacement "'-p', 'neojs-core', '--test', 'winregistry',"
    Assert-Rejected -Name 'missing --target' -Root $targetRoot -Expected @(
        'cargo call lacks exactly one --target',
        'winregistry'
    )

    $filesystemCall = "        Invoke-Cargo @('test', '--locked', '--target', `$Target, '-p', 'neojs-core', '--test', 'fs')"
    $validationCall = "        Invoke-Cargo @('test', '--locked', '--target', `$Target, '-p', 'neojs-core', '--lib', '--', 'fs::tests::windows_copy_paths_are_rejected_before_io', '--exact')"

    $missingFilesystemRoot = New-Fixture -Name 'missing-filesystem-suite'
    $missingFilesystemScript = Join-Path $missingFilesystemRoot 'eng/ci/windows-native.ps1'
    Replace-First -Path $missingFilesystemScript -Find $filesystemCall -Replacement '        # removed by mutation self-test'
    Assert-Rejected -Name 'missing neojs-core filesystem suite' -Root $missingFilesystemRoot -Expected @(
        'expected 10 Invoke-Cargo calls, found 9',
        'expected exactly one unconditional test -p neojs-core --test fs invocation with the exact argument shape, found 0'
    )

    $duplicateFilesystemRoot = New-Fixture -Name 'duplicate-filesystem-suite'
    $duplicateFilesystemScript = Join-Path $duplicateFilesystemRoot 'eng/ci/windows-native.ps1'
    Insert-AfterLine -Path $duplicateFilesystemScript -Line $filesystemCall -Insert $filesystemCall
    Assert-Rejected -Name 'duplicate neojs-core filesystem suite' -Root $duplicateFilesystemRoot -Expected @(
        'expected 10 Invoke-Cargo calls, found 11',
        'expected exactly one unconditional test -p neojs-core --test fs invocation with the exact argument shape, found 2'
    )

    $noRunFilesystemRoot = New-Fixture -Name 'no-run-filesystem-suite'
    $noRunFilesystemScript = Join-Path $noRunFilesystemRoot 'eng/ci/windows-native.ps1'
    Replace-First -Path $noRunFilesystemScript -Find "'-p', 'neojs-core', '--test', 'fs')" -Replacement "'-p', 'neojs-core', '--test', 'fs', '--no-run')"
    Assert-Rejected -Name 'no-run neojs-core filesystem suite' -Root $noRunFilesystemRoot -Expected @(
        'filesystem suite invocation must not use --no-run'
    )

    $listFilesystemRoot = New-Fixture -Name 'list-filesystem-suite'
    $listFilesystemScript = Join-Path $listFilesystemRoot 'eng/ci/windows-native.ps1'
    Replace-First -Path $listFilesystemScript -Find "'-p', 'neojs-core', '--test', 'fs')" -Replacement "'-p', 'neojs-core', '--test', 'fs', '--list')"
    Assert-Rejected -Name 'list neojs-core filesystem suite' -Root $listFilesystemRoot -Expected @(
        'filesystem suite invocation must not use --list',
        'expected exactly one unconditional test -p neojs-core --test fs invocation with the exact argument shape, found 0'
    )

    $wrongPackageRoot = New-Fixture -Name 'wrong-filesystem-package'
    $wrongPackageScript = Join-Path $wrongPackageRoot 'eng/ci/windows-native.ps1'
    Replace-First -Path $wrongPackageScript -Find "'-p', 'neojs-core', '--test', 'fs'" -Replacement "'-p', 'neojs-other', '--test', 'fs'"
    Assert-Rejected -Name 'wrong filesystem package' -Root $wrongPackageRoot -Expected @(
        'expected exactly one unconditional test -p neojs-core --test fs invocation with the exact argument shape, found 0'
    )

    $conditionalFilesystemRoot = New-Fixture -Name 'conditional-filesystem-suite'
    $conditionalFilesystemScript = Join-Path $conditionalFilesystemRoot 'eng/ci/windows-native.ps1'
    Replace-First -Path $conditionalFilesystemScript -Find $filesystemCall -Replacement ("        if (`$false) {`n" + $filesystemCall + "`n        }")
    Assert-Rejected -Name 'conditional neojs-core filesystem suite' -Root $conditionalFilesystemRoot -Expected @(
        'filesystem suite invocation must not be conditional',
        'filesystem suite invocation must be a direct statement in the CheckOnly else block'
    )

    $outsideFilesystemRoot = New-Fixture -Name 'outside-filesystem-suite'
    $outsideFilesystemScript = Join-Path $outsideFilesystemRoot 'eng/ci/windows-native.ps1'
    Replace-First -Path $outsideFilesystemScript -Find $filesystemCall -Replacement '        # moved outside by mutation self-test'
    Insert-AfterLine -Path $outsideFilesystemScript -Line 'Push-Location $repoRoot' -Insert $filesystemCall
    Assert-Rejected -Name 'outside neojs-core filesystem suite' -Root $outsideFilesystemRoot -Expected @(
        'filesystem suite invocation must be a direct statement in the CheckOnly else block'
    )

    $uninvokedFilesystemRoot = New-Fixture -Name 'uninvoked-filesystem-suite'
    $uninvokedFilesystemScript = Join-Path $uninvokedFilesystemRoot 'eng/ci/windows-native.ps1'
    Replace-First -Path $uninvokedFilesystemScript -Find $filesystemCall -Replacement ("        function Invoke-UnusedFilesystem {`n" + $filesystemCall + "`n        }")
    Assert-Rejected -Name 'uninvoked filesystem function' -Root $uninvokedFilesystemRoot -Expected @(
        'filesystem suite invocation must not be inside a function',
        'filesystem suite invocation must be a direct statement in the CheckOnly else block'
    )

    $missingValidationRoot = New-Fixture -Name 'missing-validation-unit-test'
    $missingValidationScript = Join-Path $missingValidationRoot 'eng/ci/windows-native.ps1'
    Replace-First -Path $missingValidationScript -Find $validationCall -Replacement '        # removed by mutation self-test'
    Assert-Rejected -Name 'missing validation unit test' -Root $missingValidationRoot -Expected @(
        'expected 10 Invoke-Cargo calls, found 9',
        'expected exactly one exact validation unit test invocation, found 0'
    )

    $duplicateValidationRoot = New-Fixture -Name 'duplicate-validation-unit-test'
    $duplicateValidationScript = Join-Path $duplicateValidationRoot 'eng/ci/windows-native.ps1'
    Insert-AfterLine -Path $duplicateValidationScript -Line $validationCall -Insert $validationCall
    Assert-Rejected -Name 'duplicate validation unit test' -Root $duplicateValidationRoot -Expected @(
        'expected 10 Invoke-Cargo calls, found 11',
        'expected exactly one exact validation unit test invocation, found 2'
    )

    $noRunValidationRoot = New-Fixture -Name 'no-run-validation-unit-test'
    $noRunValidationScript = Join-Path $noRunValidationRoot 'eng/ci/windows-native.ps1'
    Replace-First -Path $noRunValidationScript -Find "'--exact')" -Replacement "'--exact', '--no-run')"
    Assert-Rejected -Name 'no-run validation unit test' -Root $noRunValidationRoot -Expected @(
        'validation unit test invocation must not use --no-run',
        'expected exactly one exact validation unit test invocation, found 0'
    )

    $listValidationRoot = New-Fixture -Name 'list-validation-unit-test'
    $listValidationScript = Join-Path $listValidationRoot 'eng/ci/windows-native.ps1'
    Replace-First -Path $listValidationScript -Find "'--exact')" -Replacement "'--exact', '--list')"
    Assert-Rejected -Name 'list validation unit test' -Root $listValidationRoot -Expected @(
        'validation unit test invocation must not use --list',
        'expected exactly one exact validation unit test invocation, found 0'
    )

    $bogusValidationRoot = New-Fixture -Name 'bogus-validation-filter'
    $bogusValidationScript = Join-Path $bogusValidationRoot 'eng/ci/windows-native.ps1'
    Replace-First -Path $bogusValidationScript -Find 'fs::tests::windows_copy_paths_are_rejected_before_io' -Replacement 'fs::tests::bogus_validation_filter'
    Assert-Rejected -Name 'bogus validation filter' -Root $bogusValidationRoot -Expected @(
        'expected exactly one exact validation unit test invocation, found 0'
    )

    $validationWrongPackageRoot = New-Fixture -Name 'wrong-validation-package'
    $validationWrongPackageScript = Join-Path $validationWrongPackageRoot 'eng/ci/windows-native.ps1'
    Replace-First -Path $validationWrongPackageScript -Find "'-p', 'neojs-core', '--lib'" -Replacement "'-p', 'neojs-other', '--lib'"
    Assert-Rejected -Name 'wrong validation package' -Root $validationWrongPackageRoot -Expected @(
        'expected exactly one exact validation unit test invocation, found 0'
    )

    $conditionalValidationRoot = New-Fixture -Name 'conditional-validation-unit-test'
    $conditionalValidationScript = Join-Path $conditionalValidationRoot 'eng/ci/windows-native.ps1'
    Replace-First -Path $conditionalValidationScript -Find $validationCall -Replacement ("        if (`$false) {`n" + $validationCall + "`n        }")
    Assert-Rejected -Name 'conditional validation unit test' -Root $conditionalValidationRoot -Expected @(
        'validation unit test invocation must not be conditional',
        'validation unit test invocation must be a direct statement in the CheckOnly else block'
    )

    $outsideValidationRoot = New-Fixture -Name 'outside-validation-unit-test'
    $outsideValidationScript = Join-Path $outsideValidationRoot 'eng/ci/windows-native.ps1'
    Replace-First -Path $outsideValidationScript -Find $validationCall -Replacement '        # moved outside by mutation self-test'
    Insert-AfterLine -Path $outsideValidationScript -Line 'Push-Location $repoRoot' -Insert $validationCall
    Assert-Rejected -Name 'outside validation unit test' -Root $outsideValidationRoot -Expected @(
        'validation unit test invocation must be a direct statement in the CheckOnly else block'
    )

    $uninvokedValidationRoot = New-Fixture -Name 'uninvoked-validation-unit-test'
    $uninvokedValidationScript = Join-Path $uninvokedValidationRoot 'eng/ci/windows-native.ps1'
    Replace-First -Path $uninvokedValidationScript -Find $validationCall -Replacement ("        function Invoke-UnusedValidation {`n" + $validationCall + "`n        }")
    Assert-Rejected -Name 'uninvoked validation function' -Root $uninvokedValidationRoot -Expected @(
        'validation unit test invocation must not be inside a function',
        'validation unit test invocation must be a direct statement in the CheckOnly else block'
    )

    $aliasRoot = New-Fixture -Name 'cargo-alias'
    $aliasScript = Join-Path $aliasRoot 'eng/ci/windows-native.ps1'
    Insert-AfterLine -Path $aliasScript -Line 'Set-StrictMode -Version Latest' -Insert 'Set-Alias cargo ''Write-Host'''
    Assert-Rejected -Name 'cargo alias' -Root $aliasRoot -Expected @(
        'windows-native.ps1 defines a cargo alias',
        'Set-Alias cargo'
    )

    $pinRoot = New-Fixture -Name 'floating-action'
    $pinWorkflow = Join-Path $pinRoot '.github/workflows/windows-native.yml'
    Replace-First -Path $pinWorkflow -Find 'actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1' -Replacement 'actions/checkout@v7'
    Assert-Rejected -Name 'floating action ref' -Root $pinRoot -Expected @(
        'action actions/checkout is not pinned to a 40-hex commit SHA',
        'v7'
    )

    $toolchainInputRoot = New-Fixture -Name 'unsupported-toolchain-input'
    $toolchainInputWorkflow = Join-Path $toolchainInputRoot '.github/workflows/windows-native.yml'
    Insert-AfterLine -Path $toolchainInputWorkflow -Line '        uses: dtolnay/rust-toolchain@4b54d2ecfeb0fc349341b7929a580d09d9e2479b # 1.87.0' -Insert "        with:`n          toolchain: 1.87.0"
    Assert-Rejected -Name 'unsupported toolchain input block' -Root $toolchainInputRoot -Expected @(
        'dtolnay/rust-toolchain must not define with:',
        'toolchain: 1.87.0'
    )

    $quotedToolchainRoot = New-Fixture -Name 'unsupported-quoted-toolchain-input'
    $quotedToolchainWorkflow = Join-Path $quotedToolchainRoot '.github/workflows/windows-native.yml'
    Insert-AfterLine -Path $quotedToolchainWorkflow -Line '        uses: dtolnay/rust-toolchain@4b54d2ecfeb0fc349341b7929a580d09d9e2479b # 1.87.0' -Insert ('        with:' + "`n" + '          "toolchain": "1.87.0"')
    Assert-Rejected -Name 'unsupported quoted toolchain input block' -Root $quotedToolchainRoot -Expected @(
        'dtolnay/rust-toolchain must not define with:',
        '"toolchain": "1.87.0"'
    )

    $inlineToolchainRoot = New-Fixture -Name 'unsupported-inline-toolchain-input'
    $inlineToolchainWorkflow = Join-Path $inlineToolchainRoot '.github/workflows/windows-native.yml'
    Insert-AfterLine -Path $inlineToolchainWorkflow -Line '        uses: dtolnay/rust-toolchain@4b54d2ecfeb0fc349341b7929a580d09d9e2479b # 1.87.0' -Insert '        with: { "toolchain": "1.87.0" }'
    Assert-Rejected -Name 'unsupported inline quoted toolchain input' -Root $inlineToolchainRoot -Expected @(
        'dtolnay/rust-toolchain must not define with:',
        'with: { "toolchain": "1.87.0" }'
    )

    $quotedWithRoot = New-Fixture -Name 'quoted-with-key'
    $quotedWithWorkflow = Join-Path $quotedWithRoot '.github/workflows/windows-native.yml'
    Insert-AfterLine -Path $quotedWithWorkflow -Line '        uses: dtolnay/rust-toolchain@4b54d2ecfeb0fc349341b7929a580d09d9e2479b # 1.87.0' -Insert '        "with": { "toolchain": "1.87.0" }'
    Assert-Rejected -Name 'quoted with key' -Root $quotedWithRoot -Expected @(
        'dtolnay/rust-toolchain must not define with:',
        '"with": { "toolchain": "1.87.0" }'
    )

    $releaseRoot = New-Fixture -Name 'release-trigger'
    $releaseWorkflow = Join-Path $releaseRoot '.github/workflows/windows-native.yml'
    Insert-AfterLine -Path $releaseWorkflow -Line 'on:' -Insert '  release:'
    Assert-Rejected -Name 'release trigger' -Root $releaseRoot -Expected @(
        'workflow must declare exactly one trigger (workflow_dispatch), found 2',
        'release'
    )

    $workflowCallRoot = New-Fixture -Name 'workflow-call-trigger'
    $workflowCallWorkflow = Join-Path $workflowCallRoot '.github/workflows/windows-native.yml'
    Insert-AfterLine -Path $workflowCallWorkflow -Line 'on:' -Insert '  workflow_call:'
    Assert-Rejected -Name 'workflow_call trigger' -Root $workflowCallRoot -Expected @(
        'workflow must declare exactly one trigger (workflow_dispatch), found 2',
        'workflow_call'
    )

    $quotedPermissionRoot = New-Fixture -Name 'quoted-id-token-write'
    $quotedPermissionWorkflow = Join-Path $quotedPermissionRoot '.github/workflows/windows-native.yml'
    Insert-AfterLine -Path $quotedPermissionWorkflow -Line '  contents: read' -Insert "  'id-token': write"
    Assert-Rejected -Name 'quoted id-token write permission' -Root $quotedPermissionRoot -Expected @(
        'workflow permissions must be exactly contents: read, found 2',
        'id-token'
    )

    $commentReleaseRoot = New-Fixture -Name 'comment-then-release'
    $commentReleaseWorkflow = Join-Path $commentReleaseRoot '.github/workflows/windows-native.yml'
    Insert-AfterLine -Path $commentReleaseWorkflow -Line 'on:' -Insert "# column-zero comment`n  release:"
    Assert-Rejected -Name 'column-zero comment before release trigger' -Root $commentReleaseRoot -Expected @(
        'workflow must declare exactly one trigger (workflow_dispatch), found 2',
        'release'
    )

    $commentPermissionRoot = New-Fixture -Name 'comment-then-id-token'
    $commentPermissionWorkflow = Join-Path $commentPermissionRoot '.github/workflows/windows-native.yml'
    Insert-AfterLine -Path $commentPermissionWorkflow -Line '  contents: read' -Insert "# column-zero comment`n  'id-token': write"
    Assert-Rejected -Name 'column-zero comment before id-token permission' -Root $commentPermissionRoot -Expected @(
        'workflow permissions must be exactly contents: read, found 2',
        'id-token'
    )

    $jobPermissionRoot = New-Fixture -Name 'job-level-permissions'
    $jobPermissionWorkflow = Join-Path $jobPermissionRoot '.github/workflows/windows-native.yml'
    Insert-AfterLine -Path $jobPermissionWorkflow -Line '    timeout-minutes: 60' -Insert '    permissions: write-all'
    Assert-Rejected -Name 'job-level permissions' -Root $jobPermissionRoot -Expected @(
        'job-level permissions are not allowed',
        'permissions: write-all'
    )

    $quotedJobPermissionRoot = New-Fixture -Name 'quoted-job-level-permissions'
    $quotedJobPermissionWorkflow = Join-Path $quotedJobPermissionRoot '.github/workflows/windows-native.yml'
    Insert-AfterLine -Path $quotedJobPermissionWorkflow -Line '    timeout-minutes: 60' -Insert "    'permissions': write-all"
    Assert-Rejected -Name 'quoted job-level permissions' -Root $quotedJobPermissionRoot -Expected @(
        'job-level permissions are not allowed',
        "'permissions': write-all"
    )

    $quotedContentsRoot = New-Fixture -Name 'quoted-contents'
    $quotedContentsWorkflow = Join-Path $quotedContentsRoot '.github/workflows/windows-native.yml'
    Replace-First -Path $quotedContentsWorkflow -Find '  contents: read' -Replacement '  "contents": read'
    Assert-Rejected -Name 'quoted contents permission key' -Root $quotedContentsRoot -Expected @(
        'workflow permission key must be unquoted contents',
        '"contents": read'
    )

    $contentsWriteRoot = New-Fixture -Name 'contents-write'
    $contentsWriteWorkflow = Join-Path $contentsWriteRoot '.github/workflows/windows-native.yml'
    Replace-First -Path $contentsWriteWorkflow -Find '  contents: read' -Replacement '  contents: write'
    Assert-Rejected -Name 'contents write permission' -Root $contentsWriteRoot -Expected @(
        'workflow permission contents must be read, found write'
    )

    $missingRegistryRoot = New-Fixture -Name 'missing-registry-source'
    Remove-Item -LiteralPath (Join-Path $missingRegistryRoot 'neojs-core/tests/winregistry_windows.rs') -Force
    Assert-Rejected -Name 'missing registry test source' -Root $missingRegistryRoot -Expected @(
        'missing Windows registry test source'
    )

    if ($failures.Count -gt 0) {
        foreach ($failure in $failures) {
            [Console]::Error.WriteLine("windows-ci mutation self-test: $failure")
        }
        $exitCode = 1
    }
    else {
        Write-Host 'windows-ci mutation self-test: ok'
    }
}
finally {
    Remove-OwnedFixtureRoot
}

exit $exitCode
