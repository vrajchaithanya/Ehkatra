# cc-env.ps1 -- point cc-rs at the in-workspace C toolchain, for this process only.
#
# TD-28 / D-073. The toolchain lives in `.toolchain\mingw64` inside the repo and
# is reached through environment variables scoped to whatever command sources
# this script. Deliberately NOT done: editing PATH, writing the registry,
# installing anything, or touching a file outside this folder (DP-S5).
#
#   . tools\cc-env.ps1        # dot-source, then run cargo in the same process
#
# The variables are the target-suffixed ones cc-rs looks for first, so they
# apply to `x86_64-pc-windows-gnu` builds only and cannot silently retarget a
# cross-compile (the wasm32 determinism gate builds no C at all).
#
# ASCII only, on purpose: Windows PowerShell 5.1 reads .ps1 as ANSI unless the
# file carries a BOM, so a stray em dash is a parse error rather than a typo.

$root = Split-Path $PSScriptRoot -Parent
$bin = Join-Path $root '.toolchain\mingw64\bin'

if (-not (Test-Path (Join-Path $bin 'gcc.exe'))) {
    Write-Host "no C toolchain in .toolchain\ - see docs/43 D-073 for the URL + SHA-256" -ForegroundColor Yellow
    return
}

$env:CC_x86_64_pc_windows_gnu = Join-Path $bin 'gcc.exe'
$env:AR_x86_64_pc_windows_gnu = Join-Path $bin 'ar.exe'
# The MinGW build is msvcrt-flavoured, matching what Rust's
# x86_64-pc-windows-gnu target links. A UCRT build would put C objects and Rust
# std on two different C runtimes: two heaps, one sqlite3_free.
$env:EHKATRA_CC_READY = '1'
