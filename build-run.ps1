<#
    build-run.ps1 — run the Ehkatra autonomous build until every phase in
    BUILD-PHASES.md is DONE. Silent until completion; survives laptop
    restarts, network drops, and usage-limit windows.

    Start:                  .\build-run.ps1
    Stop (stay stopped):    .\build-run.ps1 -Stop
    Status:                 .\build-watch.ps1   (or read .logs\run-state.json)

    Resilience model:
    - Laptop restart: the logon task 'Ehkatra-build-guardian' calls this
      script with -Resume. It relaunches ONLY if run-state.json says the run
      is 'active'; a stopped/complete/blocked run is never resurrected.
      A session killed mid-work is repaired by the next session's start
      protocol (gates first, repair forward).
    - Network/usage outage: claude exiting non-zero retries with backoff
      (5m, 15m, 30m, then hourly) for up to ~24h per outage, then blocks.
    - Red gates: up to 2 dedicated repair sessions; still red -> status
      'blocked' and stop. Correctness beats continuity: a red tree must not
      compound unattended.
    - Only after ALL GATES GREEN does anything get committed and pushed.
    - Completion: final commit 'ALL PHASES COMPLETE', state 'complete',
      best-effort desktop notification. BUILD-COMPLETE.md is the report.
#>
[CmdletBinding()]
param(
    [int]$MaxSessions = 60,
    [string]$Model = 'opus',
    [switch]$Resume,
    [switch]$Stop
)

$ErrorActionPreference = 'Continue'
$repo = $PSScriptRoot; Set-Location $repo
$logDir = Join-Path $repo '.logs'
if (-not (Test-Path $logDir)) { New-Item -ItemType Directory $logDir | Out-Null }
$loopLog   = Join-Path $logDir 'loop.log'
$stateFile = Join-Path $logDir 'run-state.json'

function Log([string]$m) {
    $l = "[{0}] {1}" -f (Get-Date -Format 'yyyy-MM-dd HH:mm:ss'), $m
    Write-Host $l; Add-Content -Path $loopLog -Value $l
}
function Get-RunState { if (Test-Path $stateFile) { Get-Content $stateFile -Raw | ConvertFrom-Json } }
function Set-RunState($s) { $s | ConvertTo-Json | Set-Content $stateFile }

if ($Stop) {
    $s = Get-RunState
    if ($s) { $s.status = 'stopped'; Set-RunState $s }
    Log "=== run STOPPED by user (-Stop). Guardian will not relaunch. Kill any live session window by closing it. ==="
    return
}

# Single instance: if another build-run is alive, leave quietly.
$twin = Get-WmiObject Win32_Process -Filter "Name='powershell.exe'" |
    Where-Object { $_.CommandLine -like '*-File*build-run.ps1*' -and $_.CommandLine -notlike '*-Stop*' `
        -and $_.CommandLine -notlike '*-Command*' -and $_.ProcessId -ne $PID }
if ($twin) { Log "another build-run is active (PID $($twin[0].ProcessId)) - exiting"; return }

if ($Resume) {
    $s = Get-RunState
    if (-not $s -or $s.status -ne 'active') { return }   # nothing to resume - guardian exits silently
    Log "=== guardian RESUME after restart: run from $($s.startedAt), $($s.sessionsDone) sessions done ==="
} else {
    $s = [pscustomobject]@{ status = 'active'; startedAt = (Get-Date -Format s); sessionsDone = 0 }
    Set-RunState $s
    Log "=== run START: until all BUILD-PHASES.md phases DONE (cap $MaxSessions sessions), model=$Model ==="
}

# Non-interactive shells do not inherit rustup's PATH entry.
$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
if ((Test-Path $cargoBin) -and ($env:PATH -notlike "*$cargoBin*")) { $env:PATH = "$cargoBin;$env:PATH" }

$claudeCmd = Get-Command claude -ErrorAction SilentlyContinue
if ($claudeCmd) { $claudeExe = $claudeCmd.Source }
elseif (Test-Path "$env:USERPROFILE\.local\bin\claude.exe") { $claudeExe = "$env:USERPROFILE\.local\bin\claude.exe" }
else { Log "FATAL: claude CLI not found"; $s.status = 'blocked'; Set-RunState $s; return }

$prompt = Get-Content (Join-Path $repo 'build-prompt.md') -Raw
$repairPrompt = "The gate suite (powershell -ExecutionPolicy Bypass -File tools\gates.ps1) is RED in this repo. " +
    "Per CLAUDE.md session-start protocol: diagnose and fix ONLY what is red - no new features, no phase work. " +
    "Read PROGRESS.md's latest entry for what the previous session was doing (it may have been killed mid-work " +
    "by a restart). Repair forward to a green state, record the repair in PROGRESS.md, and end when the full " +
    "gate suite prints ALL GATES GREEN."

function Test-AllPhasesDone {
    -not (Select-String -Path (Join-Path $repo 'BUILD-PHASES.md') -Pattern 'Status:\s*PENDING' -Quiet)
}

function Invoke-Gates([string]$tag) {
    $gl = Join-Path $logDir "gates-$tag.log"
    & powershell -ExecutionPolicy Bypass -File (Join-Path $repo 'tools\gates.ps1') *> $gl
    return (Select-String -Path $gl -Pattern 'ALL GATES GREEN' -Quiet)
}

function Get-SessionSubject {
    $progress = Join-Path $repo 'PROGRESS.md'
    if (Test-Path $progress) {
        $hit = Select-String -Path $progress -Pattern '^\*\*Session\s+(\d+)' | Select-Object -First 1
        if ($hit) {
            $num = $hit.Matches[0].Groups[1].Value
            $claim = [regex]::Match($hit.Line, '^\*\*Session\s+\d+[^*]*\*\*\s*\*\*(.+?)\*\*')
            if ($claim.Success) { $text = $claim.Groups[1].Value }
            else { $text = $hit.Line -replace '^\*\*Session\s+\d+[^*]*\*\*\s*', '' }
            $text = ($text -replace '\*\*', '' -replace '`', '' -replace '\s+', ' ').Trim(' ', '-', '.', ',')
            if ($text.Length -gt 64) { $text = $text.Substring(0, 64).TrimEnd() + '...' }
            if ($text) { return "build(session ${num}): $text" }
        }
    }
    return "build: autonomous session, gates green ($(Get-Date -Format 'yyyy-MM-dd HH:mm'))"
}

function Invoke-Commit {
    & git add -A 2>&1 | Out-Null
    & git commit -m (Get-SessionSubject) --quiet 2>&1 | Out-Null
    if ($LASTEXITCODE -eq 0) {
        Log "committed: $(& git log -1 --pretty=%s)"
        & git push --quiet 2>&1 | Out-Null
        if ($LASTEXITCODE -eq 0) { Log "pushed to origin/main" }
        else { Log "push failed (offline?) - commit is safe locally; will push with the next green" }
    } else { Log "nothing to commit" }
}

function Invoke-Session([string]$text, [string]$tag) {
    # Returns $true when a session ran to completion; retries transient
    # failures (network down, usage-limit window, expired token refresh)
    # with backoff for up to ~24h before giving up.
    $sl = Join-Path $logDir "session-$tag.log"
    $backoff = @(300, 900, 1800)
    for ($attempt = 0; $attempt -le 24; $attempt++) {
        if ($attempt -gt 0) {
            $wait = 3600; if ($attempt -le 3) { $wait = $backoff[$attempt - 1] }
            Log "claude failed (attempt $attempt) - waiting $([int]($wait/60))m (network/usage window?)"
            Start-Sleep -Seconds $wait
        }
        # PROMPT BY STDIN, NEVER AS AN ARGUMENT. PowerShell 5.1's native-command
        # argument marshalling truncates a string at its first embedded double
        # quote, silently. build-prompt.md is 3,109 chars with its first `"` at
        # index 1077, so passing $text as an argument delivered ~35% of the
        # prompt and dropped everything after it. Diagnosed 2026-08-21 on the
        # sibling ITSM runner, where the same construct was delivering 2,107 of
        # 16,519 chars and cost two days of stalled build time. Do not "tidy"
        # this back into the argument list.
        $text | & $claudeExe --print --model $Model --permission-mode bypassPermissions 2>&1 | Out-File $sl -Encoding utf8
        if ($LASTEXITCODE -eq 0) { return $true }
    }
    return $false
}

while ($s.sessionsDone -lt $MaxSessions -and -not (Test-AllPhasesDone)) {
    $tag = Get-Date -Format 'yyyyMMdd-HHmmss'
    Log "--- session $($s.sessionsDone + 1)/$MaxSessions starting -> session-$tag.log"
    if (-not (Invoke-Session $prompt $tag)) {
        Log "STOPPING: claude unreachable for ~24h. Fix connectivity/auth (run: claude) then rerun .\build-run.ps1"
        $s.status = 'blocked'; Set-RunState $s; return
    }
    $s.sessionsDone++; Set-RunState $s

    if (Invoke-Gates $tag) {
        Log "gates green after session $($s.sessionsDone)"
        Invoke-Commit
    } else {
        $fixed = $false
        foreach ($r in 1, 2) {
            Log "gates RED - repair session $r of 2"
            if (-not (Invoke-Session $repairPrompt "$tag-repair$r")) { break }
            if (Invoke-Gates "$tag-repair$r") { $fixed = $true; break }
        }
        if ($fixed) { Log "repaired - gates green"; Invoke-Commit }
        else {
            Log "STOPPING: gates still red after 2 repair sessions - a human needs to look. Nothing was committed."
            $s.status = 'blocked'; Set-RunState $s; return
        }
    }

    # A user stop issued while a session was running takes effect here.
    $cur = Get-RunState
    if ($cur.status -eq 'stopped') { Log "user stop honored between sessions"; return }
}

if (Test-AllPhasesDone) {
    & git add -A 2>&1 | Out-Null
    & git commit -m "build: ALL PHASES COMPLETE - see BUILD-COMPLETE.md" --quiet 2>&1 | Out-Null
    & git push --quiet 2>&1 | Out-Null
    $s.status = 'complete'; Set-RunState $s
    Log "=== ALL PHASES COMPLETE - report: BUILD-COMPLETE.md ==="
    try { & msg $env:USERNAME "Ehkatra build: ALL PHASES COMPLETE. Report: BUILD-COMPLETE.md" 2>&1 | Out-Null } catch {}
} else {
    $s.status = 'cap-reached'; Set-RunState $s
    Log "=== session cap $MaxSessions reached before all phases DONE - review PROGRESS.md, then rerun to continue ==="
}
