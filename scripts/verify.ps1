#!/usr/bin/env pwsh
<#
.SYNOPSIS
    The single definition of "done" for this repository.

.DESCRIPTION
    AGENTS.md states intent; this script is what actually fails. Every gate here
    is mechanical, so "I followed the rules" is never an argument.

    Usage:
        pwsh -File scripts/verify.ps1          # static gates + fast unit tests
        pwsh -File scripts/verify.ps1 -Full    # adds the ignored blargg ROM suite

    `testing/` is gitignored, so a GitHub checkout has no harness: the script
    then drops `--all-features` and runs only the tests that live under `src/`.
    CI therefore proves "it compiles, passes every lint, and the public-API
    tests hold"; a local run with the harness proves the emulator core too.
    Exit code 0 means every gate that applies to this checkout passed.
#>
[CmdletBinding()]
param(
    [switch]$Full
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repo = Split-Path -Parent $PSScriptRoot
$harness = Test-Path (Join-Path $repo 'testing')
$failures = [System.Collections.Generic.List[string]]::new()

function Invoke-Stage {
    param([string]$Name, [scriptblock]$Body)
    Write-Host "== $Name" -ForegroundColor Cyan
    $global:LASTEXITCODE = 0
    & $Body
    if ($LASTEXITCODE -ne 0) {
        $failures.Add($Name)
        Write-Host "   FAILED (exit $LASTEXITCODE)" -ForegroundColor Red
    }
}

function Get-SourceFiles {
    $roots = @('src', 'testing') |
        ForEach-Object { Join-Path $repo $_ } |
        Where-Object { Test-Path -LiteralPath $_ }
    foreach ($root in $roots) {
        Get-ChildItem -Path $root -Recurse -Filter '*.rs' -File
    }
}

# Inline suppressions are forbidden: a deviation is an `#[expect(lint, reason)]`
# so the compiler re-opens the question as soon as the lint stops firing.
function Test-NoInlineAllow {
    $hits = Get-SourceFiles | Select-String -Pattern '^\s*#!?\[\s*allow\s*\('
    if ($hits) {
        $hits | ForEach-Object { Write-Host "   $($_.Path):$($_.LineNumber): $($_.Line.Trim())" }
        Write-Host "   use #[expect(lint, reason = `"...`")] instead" -ForegroundColor Yellow
        $global:LASTEXITCODE = 1
    }
}

# Unfinished work is not allowed to enter the tree; `clippy::todo` covers the
# macros, this covers the comments that tend to outlive them.
function Test-NoMarkerComments {
    $hits = Get-SourceFiles | Select-String -Pattern '//.*\b(TODO|FIXME|HACK|XXX)\b'
    if ($hits) {
        $hits | ForEach-Object { Write-Host "   $($_.Path):$($_.LineNumber): $($_.Line.Trim())" }
        $global:LASTEXITCODE = 1
    }
}

Push-Location $repo
try {
    Invoke-Stage 'fmt' {
        if ($harness) {
            cargo fmt --all -- --check
        } else {
            # rustfmt resolves `#[path]` modules even when the `cfg` that guards
            # them is off, so without `testing/` `cargo fmt` fails on a missing
            # file. check each published file on its own instead.
            $files = @(Get-SourceFiles | ForEach-Object { $_.FullName })
            if ($files.Count -gt 0) {
                rustfmt --check --edition 2024 --config skip_children=true @files
            }
        }
    }

    Invoke-Stage 'clippy (all targets)' {
        if ($harness) {
            cargo clippy --all-targets --all-features -- -D warnings
        } else {
            cargo clippy --all-targets -- -D warnings
        }
    }

    Invoke-Stage 'policy: no inline #[allow]' { Test-NoInlineAllow }
    Invoke-Stage 'policy: no TODO/FIXME markers' { Test-NoMarkerComments }

    if ($harness) {
        Invoke-Stage 'tests (fast)' { cargo test --all-features }

        if ($Full) {
            Invoke-Stage 'tests (blargg ROM suite, release)' {
                cargo test --release --all-features -- --ignored
            }
        }
    } else {
        Write-Host '== tests: harness absent; running only the in-crate tests under src/' -ForegroundColor Yellow
        Invoke-Stage 'tests (in-crate)' { cargo test }
    }

    Write-Host ''
    if ($failures.Count -gt 0) {
        Write-Host "NOT DONE: $($failures -join ', ')" -ForegroundColor Red
        exit 1
    }
    Write-Host 'DONE: all gates passed' -ForegroundColor Green
    exit 0
}
finally {
    Pop-Location
}
