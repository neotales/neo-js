#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Static and fail-closed checks for the native Windows CI files.

.DESCRIPTION
    This runs on any host with PowerShell 7. It checks the workflow text, the
    action pins, and the PowerShell AST of eng/ci/windows-native.ps1. It then
    runs windows-native.ps1 -CheckOnly and asserts that the preflight fails
    closed on a non-Windows host and succeeds on Windows.

    It deliberately does not need a YAML module. Trigger and permission blocks
    are parsed with a strict indentation contract; comments and blank lines do
    not end a mapping, and any non-root permissions key is rejected. Action
    SHAs are checked against the values documented in eng/ci/README.md. There
    are no external dependencies or installs.

    The cargo check enumerates CommandAst nodes. It allows exactly one literal
    cargo command: `& cargo '+1.87.0' @Arguments` inside Invoke-Cargo. It
    rejects direct cargo calls, dynamic command expressions, shell/dynamic
    wrappers, and cargo aliases/functions. This is a source/AST guarantee; it
    does not attest the cargo binary on PATH or cargo's subprocesses.
#>
[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$workflowPath = Join-Path $repoRoot '.github/workflows/windows-native.yml'
$scriptPath = Join-Path $repoRoot 'eng/ci/windows-native.ps1'
$registryTestPath = Join-Path $repoRoot 'neojs-core/tests/winregistry_windows.rs'
$failures = [System.Collections.Generic.List[string]]::new()

function Add-Failure {
    param([Parameter(Mandatory = $true)][string]$Message)
    [void]$failures.Add($Message)
}

function Get-ExecutableBasename {
    param([Parameter(Mandatory = $true)][string]$Name)
    $value = $Name.Trim().Trim('"', "'")
    $value = $value -replace '\\', '/'
    $lastSlash = $value.LastIndexOf('/')
    if ($lastSlash -ge 0) {
        $value = $value.Substring($lastSlash + 1)
    }
    return $value.TrimEnd('.', ' ').ToLowerInvariant()
}

function Get-YamlTopLevelBlock {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][AllowEmptyCollection()][string[]]$Lines,
        [Parameter(Mandatory = $true)][string]$Key
    )

    $pattern = '^' + [regex]::Escape($Key) + ':(?<inline>.*)$'
    $start = -1
    $inline = $null
    $occurrences = 0
    for ($index = 0; $index -lt $Lines.Count; $index++) {
        if ($Lines[$index] -match $pattern) {
            $occurrences++
            if ($start -lt 0) {
                $start = $index
                $inline = $Matches.inline.Trim()
            }
        }
    }
    if ($occurrences -gt 1) {
        Add-Failure "workflow has duplicate top-level '${Key}:' keys ($occurrences)"
    }
    if ($start -lt 0) {
        return $null
    }

    $block = [System.Collections.Generic.List[string]]::new()
    for ($index = $start + 1; $index -lt $Lines.Count; $index++) {
        $line = $Lines[$index]
        $trimmed = $line.Trim()
        # YAML comments and blank lines do not end a mapping, even at column 0.
        if ($trimmed -eq '' -or $trimmed.StartsWith('#')) {
            $block.Add($line)
            continue
        }
        if ($line -match '^\S') {
            break
        }
        $block.Add($line)
    }
    return [pscustomobject]@{
        Inline = $inline
        Lines  = $block.ToArray()
    }
}

function Get-YamlChildEntries {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][AllowEmptyCollection()][string[]]$Lines,
        [Parameter(Mandatory = $true)][int]$Indent,
        [Parameter(Mandatory = $true)][string]$Context
    )

    $entries = @()
    foreach ($line in $Lines) {
        $trimmed = $line.Trim()
        if ($trimmed -eq '' -or $trimmed.StartsWith('#')) {
            continue
        }
        $leading = $line.Length - $line.TrimStart().Length
        if ($leading -ne $Indent) {
            continue
        }
        if ($trimmed -notmatch '^(?<key>"(?:[^"]*)"|''(?:[^'']*)''|[A-Za-z0-9_.-]+)\s*:(?<value>.*)$') {
            Add-Failure "$Context has an unparsable key line: $line"
            continue
        }
        $rawKey = $Matches.key
        $key = $rawKey.Trim('"', "'")
        $value = $Matches.value.Trim()
        $entries += [pscustomobject]@{
            Key    = $key
            Value  = $value
            Line   = $line
            Quoted = ($rawKey.StartsWith('"') -or $rawKey.StartsWith("'"))
        }
    }
    return $entries
}

if (-not (Test-Path -LiteralPath $workflowPath)) {
    Add-Failure "missing workflow: $workflowPath"
}
if (-not (Test-Path -LiteralPath $scriptPath)) {
    Add-Failure "missing script: $scriptPath"
}
if (-not (Test-Path -LiteralPath $registryTestPath)) {
    Add-Failure "missing Windows registry test source: $registryTestPath"
}
else {
    $registrySource = Get-Content -LiteralPath $registryTestPath -Raw
    if ($registrySource -notmatch 'Software\\\\neo-js-tests-') {
        Add-Failure 'registry test source no longer reserves HKCU\Software\neo-js-tests-* keys'
    }
    foreach ($forbidden in @('HKLM', 'HKEY_LOCAL_MACHINE', 'HKEY_USERS')) {
        if ($registrySource -match $forbidden) {
            Add-Failure "registry test source mentions $forbidden; fixtures must stay under HKCU"
        }
    }
}

if ($failures.Count -eq 0) {
    $workflow = Get-Content -LiteralPath $workflowPath -Raw
    $workflowLines = Get-Content -LiteralPath $workflowPath

    $onBlock = Get-YamlTopLevelBlock -Lines $workflowLines -Key 'on'
    if ($null -eq $onBlock) {
        Add-Failure 'workflow must declare on: workflow_dispatch'
    }
    elseif ($onBlock.Inline -ne '') {
        Add-Failure "on: must be a block mapping with only workflow_dispatch, found: $($onBlock.Inline)"
    }
    else {
        $triggers = @(Get-YamlChildEntries -Lines $onBlock.Lines -Indent 2 -Context 'on')
        if ($triggers.Count -ne 1) {
            $names = ($triggers | ForEach-Object { $_.Key }) -join ', '
            Add-Failure "workflow must declare exactly one trigger (workflow_dispatch), found $($triggers.Count): $names"
        }
        elseif ($triggers[0].Key -ne 'workflow_dispatch') {
            Add-Failure "workflow trigger must be workflow_dispatch, found $($triggers[0].Key)"
        }
        elseif ($triggers[0].Value -ne '') {
            Add-Failure "workflow_dispatch must be a block mapping, found: $($triggers[0].Value)"
        }
    }

    foreach ($line in $workflowLines) {
        $trimmed = $line.Trim()
        if ($trimmed -eq '' -or $trimmed.StartsWith('#')) {
            continue
        }
        if ($line -match '^(?<indent>\s*)(?<key>"permissions"|''permissions''|permissions)\s*:') {
            if ($Matches.indent.Length -gt 0) {
                Add-Failure "job-level permissions are not allowed: $line"
            }
            elseif ($Matches.key -ne 'permissions') {
                Add-Failure 'top-level permissions key must be unquoted'
            }
        }
    }

    $permissionsBlock = Get-YamlTopLevelBlock -Lines $workflowLines -Key 'permissions'
    if ($null -eq $permissionsBlock) {
        Add-Failure 'workflow must declare permissions: contents: read'
    }
    elseif ($permissionsBlock.Inline -ne '') {
        Add-Failure "permissions: must be a block mapping, found: $($permissionsBlock.Inline)"
    }
    else {
        $permissions = @(Get-YamlChildEntries -Lines $permissionsBlock.Lines -Indent 2 -Context 'permissions')
        if ($permissions.Count -ne 1) {
            $names = ($permissions | ForEach-Object { $_.Key }) -join ', '
            Add-Failure "workflow permissions must be exactly contents: read, found $($permissions.Count): $names"
        }
        elseif ($permissions[0].Quoted) {
            Add-Failure "workflow permission key must be unquoted contents, found $($permissions[0].Line.Trim())"
        }
        elseif ($permissions[0].Key -ne 'contents') {
            Add-Failure "workflow permission key must be contents, found $($permissions[0].Key)"
        }
        elseif ($permissions[0].Value -ne 'read') {
            Add-Failure "workflow permission contents must be read, found $($permissions[0].Value)"
        }
    }
    if ($workflow -notmatch 'runs-on:\s*windows-latest') {
        Add-Failure 'workflow must use the GitHub-hosted windows-latest runner'
    }
    if ($workflow -match '(?m)^\s*runs-on:.*self-hosted') {
        Add-Failure 'workflow must not use a self-hosted runner'
    }
    if ($workflow -notmatch 'timeout-minutes:\s*60') {
        Add-Failure 'workflow must set a 60-minute job timeout'
    }
    if ($workflow -notmatch 'concurrency:') {
        Add-Failure 'workflow must set concurrency'
    }
    if ($workflow -notmatch 'persist-credentials:\s*false') {
        Add-Failure 'checkout must not persist credentials'
    }
    if ($workflow -match 'secrets\.') {
        Add-Failure 'workflow must not reference secrets.*'
    }
    if ($workflow -notmatch 'test-windows-ci-check\.ps1') {
        Add-Failure 'workflow must run the checked-in mutation self-test'
    }

    $expectedActions = @{
        # Verified 2026-11-20 with git ls-remote and the GitHub API; see README.
        'actions/checkout'       = '3d3c42e5aac5ba805825da76410c181273ba90b1'
        'dtolnay/rust-toolchain' = '4b54d2ecfeb0fc349341b7929a580d09d9e2479b'
        'Swatinem/rust-cache'    = 'd1527c8f67ef9af357290475d4af4b0fab8f12e0'
    }
    $uses = [regex]::Matches($workflow, '(?m)^\s*uses:\s*([^\s#]+)')
    if ($uses.Count -ne $expectedActions.Count) {
        Add-Failure "expected $($expectedActions.Count) pinned actions, found $($uses.Count)"
    }
    foreach ($match in $uses) {
        $entry = $match.Groups[1].Value
        $at = $entry.LastIndexOf('@')
        if ($at -le 0 -or $at -eq $entry.Length - 1) {
            Add-Failure "malformed action reference: $entry"
            continue
        }
        $name = $entry.Substring(0, $at)
        $sha = $entry.Substring($at + 1)
        if ($sha -notmatch '^[0-9a-f]{40}$') {
            Add-Failure "action $name is not pinned to a 40-hex commit SHA: $sha"
            continue
        }
        if (-not $expectedActions.ContainsKey($name)) {
            Add-Failure "unexpected action: $name"
            continue
        }
        if ($expectedActions[$name] -ne $sha) {
            Add-Failure "action $name SHA changed: expected $($expectedActions[$name]), found $sha"
        }
    }

    $tokens = $null
    $parseErrors = $null
    $ast = [System.Management.Automation.Language.Parser]::ParseFile($scriptPath, [ref]$tokens, [ref]$parseErrors)
    if ($parseErrors.Count -gt 0) {
        foreach ($error in $parseErrors) {
            Add-Failure "PowerShell parse error: $($error.ToString())"
        }
    }
    else {
        $source = Get-Content -LiteralPath $scriptPath -Raw
        if (-not $source.Contains("`$Target = 'x86_64-pc-windows-msvc'")) {
            Add-Failure 'windows-native.ps1 does not declare the explicit Windows MSVC target'
        }
        if ($source -notmatch '\[switch\]\$CheckOnly') {
            Add-Failure 'windows-native.ps1 must expose -CheckOnly for the fail-closed probe'
        }
        if (-not $source.Contains('cargo invocation must contain exactly one --target')) {
            Add-Failure 'Invoke-Cargo must keep its runtime --target assertion'
        }

        $functionDefinitions = $ast.FindAll({
                param($node)
                $node -is [System.Management.Automation.Language.FunctionDefinitionAst]
            }, $true)
        foreach ($definition in $functionDefinitions) {
            $functionName = $definition.Name -replace '^(global|script|local|private):', ''
            if ($functionName.ToLowerInvariant() -in @('cargo', 'cargo.exe')) {
                Add-Failure "windows-native.ps1 defines a cargo function: $($definition.Extent.Text)"
            }
        }

        $allCommands = $ast.FindAll({
                param($node)
                $node -is [System.Management.Automation.Language.CommandAst]
            }, $true)
        $cargoCalls = @()
        $forbiddenCommands = @(
            'invoke-expression', 'iex',
            'start-process', 'start-job', 'start-threadjob',
            'import-module', 'ipmo',
            'cmd', 'cmd.exe', 'powershell', 'powershell.exe', 'pwsh', 'pwsh.exe',
            'bash', 'sh', 'env', 'env.exe'
        )
        foreach ($command in $allCommands) {
            $firstElement = $command.CommandElements[0]
            if ($firstElement -is [System.Management.Automation.Language.VariableExpressionAst] -or
                $firstElement -is [System.Management.Automation.Language.ParenExpressionAst] -or
                $firstElement -is [System.Management.Automation.Language.ExpandableStringExpressionAst] -or
                $firstElement -is [System.Management.Automation.Language.ScriptBlockExpressionAst] -or
                $firstElement -is [System.Management.Automation.Language.SubExpressionAst] -or
                $firstElement -is [System.Management.Automation.Language.ArrayExpressionAst]) {
                Add-Failure "dynamic command invocation cannot be proven not to invoke cargo: $($command.Extent.Text)"
                continue
            }

            $commandName = $command.GetCommandName()
            if (-not $commandName) {
                if ($firstElement -is [System.Management.Automation.Language.StringConstantExpressionAst]) {
                    $commandName = $firstElement.Value
                }
                else {
                    $commandName = $firstElement.Extent.Text
                }
            }
            $normalizedName = Get-ExecutableBasename $commandName

            if ($forbiddenCommands -contains $normalizedName) {
                Add-Failure "forbidden dynamic/shell command in windows-native.ps1: $($command.Extent.Text)"
            }
            if ($normalizedName -in @('set-alias', 'new-alias', 'sal', 'nal') -and
                $command.Extent.Text -match '(?i)\bcargo(\.exe)?\b') {
                Add-Failure "windows-native.ps1 defines a cargo alias: $($command.Extent.Text)"
            }
            if ($normalizedName -in @('set-item', 'new-item') -and
                $command.Extent.Text -match '(?i)(alias|function):cargo(\.exe)?\b') {
                Add-Failure "windows-native.ps1 defines a cargo alias/function item: $($command.Extent.Text)"
            }
            if ($normalizedName -in @('cargo', 'cargo.exe')) {
                $cargoCalls += $command
            }
        }

        if ($cargoCalls.Count -ne 1) {
            $text = ($cargoCalls | ForEach-Object { $_.Extent.Text }) -join ' | '
            Add-Failure "expected exactly one literal cargo invocation (inside Invoke-Cargo), found $($cargoCalls.Count): $text"
        }
        else {
            $cargoCall = $cargoCalls[0]
            $parent = $cargoCall.Parent
            $containingFunction = $null
            while ($parent) {
                if ($parent -is [System.Management.Automation.Language.FunctionDefinitionAst]) {
                    $containingFunction = $parent.Name
                    break
                }
                $parent = $parent.Parent
            }
            if ($containingFunction -ne 'Invoke-Cargo') {
                Add-Failure "cargo invocation is not inside Invoke-Cargo: $($cargoCall.Extent.Text)"
            }
            if ($cargoCall.InvocationOperator -ne [System.Management.Automation.Language.TokenKind]::Ampersand) {
                Add-Failure "cargo invocation must use the call operator: $($cargoCall.Extent.Text)"
            }
            if ($cargoCall.CommandElements.Count -ne 3) {
                Add-Failure "cargo invocation must be exactly: cargo '+1.87.0' @Arguments; found $($cargoCall.Extent.Text)"
            }
            else {
                $commandElement = $cargoCall.CommandElements[0]
                $toolchainElement = $cargoCall.CommandElements[1]
                $argumentElement = $cargoCall.CommandElements[2]
                if ($commandElement -isnot [System.Management.Automation.Language.StringConstantExpressionAst] -or
                    $commandElement.Value -ne 'cargo') {
                    Add-Failure "cargo invocation command is not the literal 'cargo': $($cargoCall.Extent.Text)"
                }
                if ($toolchainElement -isnot [System.Management.Automation.Language.StringConstantExpressionAst] -or
                    $toolchainElement.Value -ne '+1.87.0') {
                    Add-Failure "cargo invocation does not select +1.87.0: $($cargoCall.Extent.Text)"
                }
                if ($argumentElement -isnot [System.Management.Automation.Language.VariableExpressionAst] -or
                    $argumentElement.VariablePath.UserPath -ne 'Arguments' -or
                    -not $argumentElement.Splatted) {
                    Add-Failure "cargo invocation does not splat @Arguments: $($cargoCall.Extent.Text)"
                }
            }
        }

        $invokeCargoCalls = $ast.FindAll({
                param($node)
                $node -is [System.Management.Automation.Language.CommandAst] -and
                $node.GetCommandName() -eq 'Invoke-Cargo'
            }, $true)
        if ($invokeCargoCalls.Count -ne 8) {
            Add-Failure "expected 8 Invoke-Cargo calls, found $($invokeCargoCalls.Count)"
        }
        foreach ($call in $invokeCargoCalls) {
            if ($call.CommandElements.Count -ne 2) {
                Add-Failure "Invoke-Cargo call is not an explicit array: $($call.Extent.Text)"
                continue
            }
            $argument = $call.CommandElements[1]
            if ($argument -is [System.Management.Automation.Language.ArrayExpressionAst]) {
                $statements = $argument.SubExpression.Statements
                if ($statements.Count -ne 1 -or
                    $statements[0].PipelineElements.Count -ne 1 -or
                    $statements[0].PipelineElements[0].Expression -isnot [System.Management.Automation.Language.ArrayLiteralAst]) {
                    Add-Failure "Invoke-Cargo call is not a literal array: $($call.Extent.Text)"
                    continue
                }
                $array = $statements[0].PipelineElements[0].Expression
            }
            elseif ($argument -is [System.Management.Automation.Language.ArrayLiteralAst]) {
                $array = $argument
            }
            else {
                Add-Failure "Invoke-Cargo call is not an explicit array: $($call.Extent.Text)"
                continue
            }
            $values = @()
            foreach ($element in $array.Elements) {
                if ($element -is [System.Management.Automation.Language.StringConstantExpressionAst]) {
                    $values += $element.Value
                }
                elseif ($element -is [System.Management.Automation.Language.VariableExpressionAst]) {
                    $values += '$' + $element.VariablePath.UserPath
                }
                else {
                    $values += $element.Extent.Text
                }
            }
            $targetIndices = @()
            for ($index = 0; $index -lt $values.Count; $index++) {
                if ($values[$index] -eq '--target') {
                    $targetIndices += $index
                }
            }
            if ($targetIndices.Count -ne 1) {
                Add-Failure "cargo call lacks exactly one --target: $($call.Extent.Text)"
                continue
            }
            $targetIndex = $targetIndices[0]
            if ($targetIndex + 1 -ge $values.Count -or $values[$targetIndex + 1] -ne '$Target') {
                Add-Failure "cargo call does not use `$Target after --target: $($call.Extent.Text)"
            }
        }
    }
}

if ($failures.Count -eq 0) {
    $pwshPath = (Get-Process -Id $PID).Path
    $probeArguments = @('-NoProfile', '-NonInteractive')
    if ($env:OS -eq 'Windows_NT') {
        $probeArguments += @('-ExecutionPolicy', 'Bypass')
    }
    $probeArguments += @('-File', $scriptPath, '-CheckOnly')
    $probeOutput = & $pwshPath @probeArguments 2>&1
    $probeExit = $LASTEXITCODE
    $probeText = ($probeOutput | Out-String)
    if ($env:OS -eq 'Windows_NT') {
        if ($probeExit -ne 0) {
            Add-Failure "windows-native.ps1 -CheckOnly failed on Windows (exit $probeExit): $probeText"
        }
        elseif ($probeText -notmatch 'Preflight only') {
            Add-Failure "windows-native.ps1 -CheckOnly did not report a successful preflight: $probeText"
        }
    }
    else {
        if ($probeExit -eq 0) {
            Add-Failure 'windows-native.ps1 -CheckOnly must fail closed on a non-Windows host'
        }
        elseif ($probeText -notmatch 'must run on Windows') {
            Add-Failure "non-Windows probe failed for the wrong reason: $probeText"
        }
    }
}

if ($failures.Count -gt 0) {
    foreach ($failure in $failures) {
        [Console]::Error.WriteLine("windows-ci check: $failure")
    }
    exit 1
}

Write-Host 'windows-ci static checks: ok'
exit 0
