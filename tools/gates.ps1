# gates.ps1 — run the whole CLAUDE.md gate set locally, in CI order.
# Solo rule (docs/07 §2): if a check is not one command, it does not get run.
#   pwsh -File tools/gates.ps1
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
$env:PATH = "$env:USERPROFILE\.cargo\bin;" + $env:PATH
# ehkatra-store compiles SQLite from C (ADR-031). On this host the compiler
# lives inside the workspace; on CI it is the runner's own cc. Sourcing this is
# a no-op when .toolchain\ is absent, so the gate set stays one command either
# way (docs/07: a check that is not one command does not get run).
. (Join-Path $PSScriptRoot 'cc-env.ps1')

function Step($name, [scriptblock]$body) {
    Write-Host "`n=== $name ===" -ForegroundColor Cyan
    & $body
    if ($LASTEXITCODE -ne 0) { throw "GATE FAILED: $name" }
}

Step 'Format (DP-C1)'            { cargo fmt --all -- --check }
Step 'Clippy (DP-C1)'            { cargo clippy --workspace --all-targets -- -D warnings }
Step 'Tests (DP-C4)'             { cargo test --workspace }
Step 'no_std kernel (DP-A3)'     { cargo build -p usk-types -p usk-oplog -p usk-state -p usk-formula -p usk-calc -p usk-reduce -p usk-sync -p usk-recover -p usk-json -p usk-csv -p usk-zip -p usk-xml -p usk-xlsx -p usk-mcp --target wasm32-wasip1 }
Step 'Complexity budget (DP-S2)' { node tools/dep-budget.mjs }

# Supply chain (DP-E8). For eight sessions this ran only on CI - which meant in
# practice it ran nowhere, and it failed silently on its first two pushes.
# `cargo-deny` builds here once the in-workspace MinGW `dlltool` is ahead of
# rustup's stub on PATH (D-078), so the check now happens *before* a push
# instead of after one.
Write-Host "`n=== Supply chain (DP-E8) ===" -ForegroundColor Cyan
$denyBin = Join-Path $env:USERPROFILE '.cargo\bin\cargo-deny.exe'
if (Test-Path $denyBin) {
    $mingw = Join-Path (Split-Path $PSScriptRoot -Parent) '.toolchain\mingw64\bin'
    if (Test-Path $mingw) { $env:PATH = "$mingw;" + $env:PATH }
    cargo deny check
    if ($LASTEXITCODE -ne 0) { throw 'GATE FAILED: Supply chain (cargo-deny)' }
} else {
    Write-Host '  SKIP  cargo-deny not installed (cargo install cargo-deny --locked)' -ForegroundColor Yellow
    Write-Host '        CI still enforces this on every push.' -ForegroundColor Yellow
}

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
    @{ Name = 'host service reference (DP-S5)'; Pattern = 'postgres://|:5432|localhost:3000|:8080'; Path = 'crates', 'ehkatra-cli', 'ehkatra-relay', 'tools' },
    # Row 10 introduced the first listening socket in the project. DP-S5 says
    # loopback-only, so "loopback-only" becomes a gate rather than a promise.
    @{ Name = 'listener outside loopback (DP-S5)'; Pattern = '0\.0\.0\.0|Ipv4Addr::UNSPECIFIED|\[::\]'; Path = 'crates', 'ehkatra-cli', 'ehkatra-relay', 'tools' }
)
foreach ($g in $greps) {
    $hits = Get-ChildItem -Recurse -Include *.rs, *.toml -Path $g.Path |
        Where-Object { $_.FullName -notmatch '\\target\\' } |
        Select-String -Pattern $g.Pattern
    if ($hits) { $hits; throw "GATE FAILED: $($g.Name)" }
    Write-Host "  PASS  $($g.Name)"
}

Write-Host "`nALL GATES GREEN" -ForegroundColor Green
