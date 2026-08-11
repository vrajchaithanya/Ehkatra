# gates.ps1 -- run the whole CLAUDE.md gate set locally, in CI order.
# Solo rule (docs/07 section 2): if a check is not one command, it does not get run.
#
#   powershell -File tools\gates.ps1     # Windows PowerShell 5.1 -- supported
#   pwsh -File tools/gates.ps1           # PowerShell 7+          -- supported
#
# SHELL COMPATIBILITY (TD-47, D-104). Windows PowerShell 5.1 turns a native
# command's stderr into an ErrorRecord *only when that stderr is merged into the
# PowerShell pipeline* -- by a stderr-into-stdout redirection operator, or by
# piping it into a cmdlet. Under `$ErrorActionPreference = 'Stop'` that
# ErrorRecord is terminating, so one such redirection anywhere below would make
# a green `cargo clippy` abort the run purely because cargo prints
# "Finished ..." to stderr. This script therefore never merges a native
# command's stderr into the pipeline; it lets stderr reach the console and
# judges every native command by `$LASTEXITCODE` alone. Two gates below keep it
# that way -- a runtime probe and a static grep -- because the trap is one
# character away and invisible until it fires.
#
# ASCII only, for the reason cc-env.ps1 states: 5.1 reads a BOM-less .ps1 as
# ANSI, so a stray em dash is a parse error rather than a typo.
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

# --------------------------------------------------------- shell compatibility
# Runs first and costs milliseconds, so an incompatible shell is named in two
# seconds rather than three minutes into a cargo build. It proves the property
# the rest of the script depends on -- that a native command writing to stderr
# and exiting 0 does not derail the run -- instead of assuming it.
Write-Host "`n=== Shell compatibility (TD-47) ===" -ForegroundColor Cyan
Write-Host "  host: PowerShell $($PSVersionTable.PSVersion) ($($PSVersionTable.PSEdition))"
cmd /c "echo gates-stderr-probe 1>&2"
if ($LASTEXITCODE -ne 0) {
    throw "GATE FAILED: a native command that writes to stderr and exits 0 did not survive this shell"
}
Write-Host '  PASS  native stderr does not derail the run'

# The static half. The probe proves this script is safe as written; this proves
# it stays that way, since re-arming the trap takes one redirection operator.
# The pattern is assembled from pieces so that this gate does not match its own
# source and fail on itself.
$mergePattern = '2' + '>' + '&' + '1'
$merges = Get-ChildItem -Recurse -Include *.ps1 -Path $PSScriptRoot |
    Select-String -SimpleMatch -Pattern $mergePattern
if ($merges) {
    $merges
    throw 'GATE FAILED: a .ps1 under tools\ merges native stderr into the pipeline (TD-47); judge native commands by $LASTEXITCODE instead'
}
Write-Host '  PASS  no .ps1 under tools\ merges native stderr into the pipeline'

Step 'Format (DP-C1)'            { cargo fmt --all -- --check }
Step 'Clippy (DP-C1)'            { cargo clippy --workspace --all-targets -- -D warnings }
Step 'Tests (DP-C4)'             { cargo test --workspace }
Step 'no_std kernel (DP-A3)'     { cargo build -p usk-types -p usk-oplog -p usk-state -p usk-formula -p usk-calc -p usk-reduce -p usk-sync -p usk-recover -p usk-json -p usk-csv -p usk-zip -p usk-xml -p usk-xlsx -p usk-mcp -p usk-view --target wasm32-wasip1 }
Step 'Complexity budget (DP-S2)' { node tools/dep-budget.mjs }

# The shell is a separate workspace (D-116), so *none* of the three gates above
# reaches it: `cargo fmt --all`, `cargo clippy --workspace` and
# `cargo test --workspace` all stop at this workspace's members. It therefore
# gets the same three explicitly.
#
# Until the editing surface landed this was a bare `cargo check`, justified by
# "it has no tests yet". It has 54 now, and a gate that compiles code without
# running its tests is a gate in name only. The mingw `dlltool` must be ahead of
# rustup's stub on PATH for this to link (D-078), which is why the supply-chain
# step's PATH edit is hoisted above it.
$mingw = Join-Path (Split-Path $PSScriptRoot -Parent) '.toolchain\mingw64\bin'
if (Test-Path $mingw) { $env:PATH = "$mingw;" + $env:PATH }
if (Test-Path 'shell\Cargo.toml') {
    Step 'Shell format (DP-C1)' { cargo fmt --all --manifest-path shell/Cargo.toml -- --check }
    Step 'Shell clippy (DP-C1)' { cargo clippy --manifest-path shell/Cargo.toml --all-targets -- -D warnings }
    Step 'Shell tests (DP-C4)'  { cargo test --manifest-path shell/Cargo.toml }
}

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
