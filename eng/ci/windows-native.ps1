#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Run the native Windows x64 MSVC gate for neo-js.

.DESCRIPTION
    This is the reproducible form of .github/workflows/windows-native.yml. It
    checks that the host is a 64-bit Windows x64 process with the pinned MSVC
    Rust toolchain and the x86_64-pc-windows-msvc standard library, refuses to
    start with stale registry fixtures, runs the Windows registry and process
    suites, and compiles every workspace test target.

    Every cargo command passes --target x86_64-pc-windows-msvc explicitly. The
    rustc host check alone is not treated as proof that cargo would build for
    that target.

    The registry tests create uniquely named keys under
    HKCU\Software\neo-js-tests-* only. They never write to HKLM, HKEY_USERS,
    or an existing user key. A final read-only check fails the run if any
    fixture key was left behind.

.PARAMETER FullWorkspace
    Also run `cargo test --workspace` for the explicit Windows target. This is
    opt-in because Windows-only tests have not been executed before; the first
    full run is expected to find failures rather than certify them.

.PARAMETER CheckOnly
    Run the host, target, fixture, and source preflight checks, then exit
    without invoking cargo. This is also the fail-closed probe used by
    eng/ci/check-windows-ci.ps1 on non-Windows hosts.

.EXAMPLE
    pwsh -File eng/ci/windows-native.ps1

.EXAMPLE
    pwsh -File eng/ci/windows-native.ps1 -FullWorkspace

.EXAMPLE
    pwsh -File eng/ci/windows-native.ps1 -CheckOnly
#>
[CmdletBinding()]
param(
    [switch]$FullWorkspace,
    [switch]$CheckOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$Target = 'x86_64-pc-windows-msvc'
$registryTestsRan = $false
$failed = $false

function Invoke-Cargo {
    param(
        [Parameter(Mandatory = $true)]
        [string[]]$Arguments
    )

    $targetIndices = @()
    for ($index = 0; $index -lt $Arguments.Count; $index++) {
        if ($Arguments[$index] -eq '--target') {
            $targetIndices += $index
        }
    }
    if ($targetIndices.Count -ne 1) {
        throw "cargo invocation must contain exactly one --target: cargo $($Arguments -join ' ')"
    }
    $targetIndex = $targetIndices[0]
    if ($targetIndex + 1 -ge $Arguments.Count -or $Arguments[$targetIndex + 1] -ne $Target) {
        throw "cargo invocation must target $Target`: cargo $($Arguments -join ' ')"
    }

    Write-Host "==> cargo +1.87.0 $($Arguments -join ' ')"
    & cargo '+1.87.0' @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "cargo +1.87.0 failed with exit code $LASTEXITCODE`: cargo $($Arguments -join ' ')"
    }
}

function Get-RegistryFixtures {
    $software = Get-Item -LiteralPath 'HKCU:\Software' -ErrorAction Stop
    foreach ($child in $software.GetSubKeyNames()) {
        if ($child -like 'neo-js-tests-*') {
            "HKCU:\Software\$child"
        }
    }
}

Push-Location $repoRoot
try {
    if ($env:OS -ne 'Windows_NT') {
        throw "this gate must run on Windows, not $([System.Environment]::OSVersion.VersionString)"
    }

    # RuntimeInformation describes the OS and this process. PROCESSOR_ARCHITECTURE
    # is printed as a diagnostic only: it is not evidence of the physical CPU
    # when a process is emulated.
    $processArch = [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString()
    $osArch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
    Write-Host "Process architecture: $processArch"
    Write-Host "OS architecture: $osArch"
    Write-Host "PROCESSOR_ARCHITECTURE: $env:PROCESSOR_ARCHITECTURE (diagnostic only)"
    if ($processArch -ne 'X64') {
        throw "expected a 64-bit x64 PowerShell process, got $processArch"
    }
    if ($osArch -ne 'X64') {
        throw "expected an x64 Windows OS, got $osArch; x64 emulation on a non-x64 OS is not accepted by this gate"
    }

    $rustc = (& rustc '+1.87.0' -vV) -join "`n"
    Write-Host $rustc
    if ($LASTEXITCODE -ne 0) {
        throw "rustc -vV failed with exit code $LASTEXITCODE"
    }
    if ($rustc -notmatch 'release: 1\.87\.0\b') {
        throw "expected Rust 1.87.0, got:`n$rustc"
    }
    if ($rustc -notmatch 'host: x86_64-pc-windows-msvc\b') {
        throw "expected the native x86_64-pc-windows-msvc host, got:`n$rustc"
    }

    $targetLibdir = (& rustc '+1.87.0' '--print' 'target-libdir' '--target' $Target) -join "`n"
    if ($LASTEXITCODE -ne 0) {
        throw "rustc --print target-libdir failed with exit code $LASTEXITCODE"
    }
    $targetLibdir = $targetLibdir.Trim()
    if (-not (Test-Path -LiteralPath $targetLibdir -PathType Container)) {
        throw "the $Target standard library is not installed at $targetLibdir"
    }
    Write-Host "Target library: $targetLibdir"

    $registrySource = Get-Content -LiteralPath 'neojs-core/tests/winregistry_windows.rs' -Raw
    if ($registrySource -notmatch 'Software\\\\neo-js-tests-') {
        throw "the Windows registry fixture no longer reserves HKCU\\Software\\neo-js-tests-* keys"
    }
    foreach ($forbidden in @('HKLM', 'HKEY_LOCAL_MACHINE', 'HKEY_USERS')) {
        if ($registrySource -match $forbidden) {
            throw "registry test source mentions $forbidden; fixtures must stay under HKCU"
        }
    }

    $stale = @(Get-RegistryFixtures)
    if ($stale.Count -gt 0) {
        throw "stale registry fixtures exist before the run:`n$($stale -join "`n")"
    }

    if ($CheckOnly) {
        Write-Host "Preflight only: host, $Target, fixture, and source checks passed; no tests run."
    }
    else {
        $registryTestsRan = $true
        Invoke-Cargo @('test', '--locked', '--target', $Target, '-p', 'neojs-core', '--test', 'winregistry', '--', '--test-threads=1')

        Invoke-Cargo @('test', '--locked', '--target', $Target, '-p', 'neojs-core', '--test', 'winregistry_windows', '--', '--test-threads=1')

        Invoke-Cargo @('test', '--locked', '--target', $Target, '-p', 'neojs-core', '--test', 'process')
        Invoke-Cargo @('test', '--locked', '--target', $Target, '-p', 'neojs', '--test', 'process')
        Invoke-Cargo @('test', '--locked', '--target', $Target, '-p', 'neojs', '--test', 'process_stdio')
        Invoke-Cargo @('test', '--locked', '--target', $Target, '-p', 'neojs-os', '--test', 'os')

        Invoke-Cargo @('test', '--workspace', '--locked', '--target', $Target, '--no-run')

        if ($FullWorkspace) {
            Invoke-Cargo @('test', '--workspace', '--locked', '--target', $Target, '--no-fail-fast')
        }
        else {
            Write-Host "Skipping full workspace test execution; pass -FullWorkspace to enable it."
        }
    }
}
catch {
    [Console]::Error.WriteLine("windows-native gate failed: $_")
    $failed = $true
}
finally {
    if ($registryTestsRan) {
        $leftovers = @(Get-RegistryFixtures)
        if ($leftovers.Count -gt 0) {
            [Console]::Error.WriteLine("registry cleanup left fixture keys behind:`n$($leftovers -join "`n")")
            $failed = $true
        }
        else {
            Write-Host "Registry cleanup check passed: no neo-js-tests-* keys remain."
        }
    }
    Pop-Location
}

if ($failed) {
    exit 1
}
exit 0
