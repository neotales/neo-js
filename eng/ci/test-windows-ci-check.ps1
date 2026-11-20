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
    path-qualified wrapper command, a missing --target, a cargo alias, a
    floating action ref, extra `release`/`workflow_call` triggers, column-zero
    comments hiding a following trigger or permission, quoted or unquoted
    job-level permissions, quoted permission keys, `id-token: write`, and
    `contents: write` permissions, and a missing registry test source.

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
