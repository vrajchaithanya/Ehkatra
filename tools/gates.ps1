# gates.ps1 — run the whole CLAUDE.md gate set locally, in CI order.
# Solo rule (docs/07 §2): if a check is not one command, it does not get run.
#   pwsh -File tools/gates.ps1
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
$env:PATH = "$env:USERPROFILE\.cargo\bin;" + $env:PATH

function Step($name, [scriptblock]$body) {
    Write-Host "`n=== $name ===" -ForegroundColor Cyan
    & $body
    if ($LASTEXITCODE -ne 0) { throw "GATE FAILED: $name" }
}

Step 'Format (DP-C1)'            { cargo fmt --all -- --check }
Step 'Clippy (DP-C1)'            { cargo clippy --workspace --all-targets -- -D warnings }
Step 'Tests (DP-C4)'             { cargo test --workspace }
Step 'no_std kernel (DP-A3)'     { cargo build -p usk-types -p usk-oplog -p usk-state -p usk-formula -p usk-calc -p usk-reduce --target wasm32-wasip1 }
Step 'Complexity budget (DP-S2)' { node tools/dep-budget.mjs }

Write-Host "`n=== Differential replay (DP-A2) ===" -ForegroundColor Cyan
cargo build --release -p replay-check
if ($LASTEXITCODE -ne 0) { throw 'GATE FAILED: replay-check native build' }
cargo build --release -p replay-check --target wasm32-wasip1
if ($LASTEXITCODE -ne 0) { throw 'GATE FAILED: replay-check wasm build' }
$native = & '.\target\release\replay-check.exe'
$wasm = & node --no-warnings tools/run-wasi.mjs target/wasm32-wasip1/release/replay-check.wasm
if (($native -join "`n") -ne ($wasm -join "`n")) {
    Write-Host "native:`n$($native -join "`n")`nwasm:`n$($wasm -join "`n")"
    throw 'GATE FAILED: native and wasm32 hashes differ'
}
$native | ForEach-Object { Write-Host "  $_" }
Write-Host 'determinism gate PASS'

Write-Host "`n=== Kernel purity / host isolation greps ===" -ForegroundColor Cyan
$kernelSrc = Get-ChildItem -Directory 'crates' | ForEach-Object { Join-Path $_.FullName 'src' }
# Kernel *sources* must be no_std; integration tests are ordinary std binaries
# that link the kernel, so they are deliberately out of scope (CI greps
# `crates/*/src` for the same reason).
$greps = @(
    @{ Name = 'std leaked into kernel (DP-A3)'; Pattern = 'use std::'; Path = $kernelSrc },
    @{ Name = 'target_os cfg in kernel (DP-C2)'; Pattern = 'cfg\(target_os'; Path = $kernelSrc },
    @{ Name = 'host service reference (DP-S5)'; Pattern = 'postgres://|:5432|localhost:3000|:8080'; Path = 'crates', 'ehkatra-cli', 'tools' }
)
foreach ($g in $greps) {
    $hits = Get-ChildItem -Recurse -Include *.rs, *.toml -Path $g.Path |
        Where-Object { $_.FullName -notmatch '\\target\\' } |
        Select-String -Pattern $g.Pattern
    if ($hits) { $hits; throw "GATE FAILED: $($g.Name)" }
    Write-Host "  PASS  $($g.Name)"
}

Write-Host "`nALL GATES GREEN" -ForegroundColor Green
