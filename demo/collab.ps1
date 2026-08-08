# demo/collab.ps1 -- BOOTSTRAP row 10's visible proof, on Windows.
#
# Starts the relay on loopback 7423, runs two peers that edit the SAME workbook
# concurrently (Alice inserts a row in the middle of the data Bob is writing
# into), and prints both replicas' state hashes. The demo passes when the two
# hashes are identical: same op set, independently merged, one state.
#
# Two terminals, if you want to watch it live:
#   Terminal 1:  cargo run --release --bin ehkatra-relay
#   Terminal 2:  cargo run --release --bin ehkatra-peer -- 1 alice
#   Terminal 3:  cargo run --release --bin ehkatra-peer -- 2 bob
#
# This script does all three for you and checks the result.

$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
$env:PATH = "$env:USERPROFILE\.cargo\bin;" + $env:PATH

Write-Host "building..." -ForegroundColor Cyan
cargo build --release --bin ehkatra-relay --bin ehkatra-peer
if ($LASTEXITCODE -ne 0) { throw "build failed" }

$relayExe = ".\target\release\ehkatra-relay.exe"
$peerExe = ".\target\release\ehkatra-peer.exe"
$out = Join-Path $PSScriptRoot 'out'
New-Item -ItemType Directory -Force $out | Out-Null

Write-Host "`nstarting relay on 127.0.0.1:7423" -ForegroundColor Cyan
$relay = Start-Process -FilePath $relayExe -PassThru -NoNewWindow `
    -RedirectStandardOutput "$out\relay.log" -RedirectStandardError "$out\relay.err"
Start-Sleep -Milliseconds 400

try {
    Write-Host "starting two peers" -ForegroundColor Cyan
    $alice = Start-Process -FilePath $peerExe -ArgumentList '1', 'alice' -PassThru -NoNewWindow `
        -RedirectStandardOutput "$out\alice.log" -RedirectStandardError "$out\alice.err"
    $bob = Start-Process -FilePath $peerExe -ArgumentList '2', 'bob' -PassThru -NoNewWindow `
        -RedirectStandardOutput "$out\bob.log" -RedirectStandardError "$out\bob.err"

    $alice.WaitForExit(30000) | Out-Null
    $bob.WaitForExit(30000) | Out-Null

    Write-Host "`n--- alice ---" -ForegroundColor Yellow
    Get-Content "$out\alice.log"
    Write-Host "`n--- bob ---" -ForegroundColor Yellow
    Get-Content "$out\bob.log"

    $hashes = @(Get-Content "$out\alice.log", "$out\bob.log" |
        Select-String -Pattern '^STATE HASH \d+: (.+)$' |
        ForEach-Object { $_.Matches[0].Groups[1].Value })

    Write-Host ""
    if ($hashes.Count -ne 2) {
        Write-Host "DEMO FAILED -- expected two state hashes, got $($hashes.Count)" -ForegroundColor Red
        exit 1
    }
    if ($hashes[0] -ne $hashes[1]) {
        Write-Host "DEMO FAILED -- replicas diverged" -ForegroundColor Red
        Write-Host "  alice: $($hashes[0])"
        Write-Host "  bob:   $($hashes[1])"
        exit 1
    }
    Write-Host "DEMO PASS -- both replicas converged" -ForegroundColor Green
    Write-Host "  state hash: $($hashes[0])"
}
finally {
    if ($relay -and -not $relay.HasExited) { Stop-Process -Id $relay.Id -Force }
}
