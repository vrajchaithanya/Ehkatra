<#
    build-loop.ps1 - autonomous Ehkatra build loop

    Runs Claude Code headless against build-prompt.md, one fresh session per
    iteration (PROGRESS.md is the memory between them, exactly as CLAUDE.md's
    handoff contract intends). After each session the full gate suite runs; the
    loop stops the moment a session leaves the tree red, so a bad session cannot
    compound into three.

    Usage:
      .\build-loop.ps1                    # 4 sessions, opus
      .\build-loop.ps1 -Sessions 8        # run overnight
      .\build-loop.ps1 -Model sonnet      # cheaper per session
      .\build-loop.ps1 -WhatIf            # show what would run, run nothing

    Logs land in .logs\ (one file per session, plus loop.log for the summary).
    Ctrl+C is safe between sessions; mid-session it leaves the tree wherever
    that session got to, which PROGRESS.md will describe.
#>
[CmdletBinding(SupportsShouldProcess = $true)]
param(
    [int]$Sessions = 4,
    [string]$Model = 'opus',
    [string]$PromptFile = 'build-prompt.md',
    [switch]$SkipGates,
    [switch]$NoCommit
)

$ErrorActionPreference = 'Stop'
$repo = $PSScriptRoot
Set-Location $repo

# A non-interactive shell does not inherit rustup's PATH entry, so `cargo` is
# absent unless we put it there. The claude process and everything it spawns
# inherit this, which is what makes `cargo test` work inside a headless session.
$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
if ((Test-Path $cargoBin) -and ($env:PATH -notlike "*$cargoBin*")) {
    $env:PATH = "$cargoBin;$env:PATH"
}

$claude = Get-Command claude -ErrorAction SilentlyContinue
if (-not $claude) { throw "claude CLI not found on PATH. Install it, or run: & `"$env:USERPROFILE\.local\bin\claude.exe`" --version" }

$promptPath = Join-Path $repo $PromptFile
if (-not (Test-Path $promptPath)) { throw "Prompt file not found: $promptPath" }
$prompt = Get-Content $promptPath -Raw

$logDir = Join-Path $repo '.logs'
if (-not (Test-Path $logDir)) { New-Item -ItemType Directory -Path $logDir | Out-Null }
$loopLog = Join-Path $logDir 'loop.log'

function Write-Loop([string]$Message) {
    $line = "[{0}] {1}" -f (Get-Date -Format 'yyyy-MM-dd HH:mm:ss'), $Message
    Write-Host $line
    Add-Content -Path $loopLog -Value $line
}

function Get-SessionSubject {
    # The session's own one-line summary is the best commit subject there is:
    # PROGRESS.md entries open "**Session N in one paragraph.** **<claim>** ..."
    $progress = Join-Path $repo 'PROGRESS.md'
    if (Test-Path $progress) {
        $hit = Select-String -Path $progress -Pattern '^\*\*Session\s+(\d+)' | Select-Object -First 1
        if ($hit) {
            $num = $hit.Matches[0].Groups[1].Value
            # The claim is the second bolded span; fall back to the rest of the line.
            $claim = [regex]::Match($hit.Line, '^\*\*Session\s+\d+[^*]*\*\*\s*\*\*(.+?)\*\*')
            if ($claim.Success) { $text = $claim.Groups[1].Value }
            else { $text = $hit.Line -replace '^\*\*Session\s+\d+[^*]*\*\*\s*', '' }
            $text = $text -replace '\*\*', '' -replace '`', ''
            $text = ($text -replace '\s+', ' ').Trim(' ', '-', '.', ',')
            if ($text.Length -gt 64) { $text = $text.Substring(0, 64).TrimEnd() + '...' }
            if ($text) { return "build(session ${num}): $text" }
            return "build(session ${num}): gates green"
        }
    }
    return "build: autonomous session, all gates green ($(Get-Date -Format 'yyyy-MM-dd HH:mm'))"
}

Write-Loop "=== loop start: $Sessions session(s), model=$Model, repo=$repo ==="

for ($i = 1; $i -le $Sessions; $i++) {
    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
    $sessionLog = Join-Path $logDir "session-$stamp.log"
    Write-Loop "--- session $i/$Sessions starting -> $sessionLog"

    if ($PSCmdlet.ShouldProcess("session $i", "claude -p (model $Model)")) {
        $started = Get-Date

        # bypassPermissions is what makes this unattended: the session must not
        # stop to ask. The guardrails are CLAUDE.md's hard boundaries (no git,
        # workspace confinement) plus the gate check below, not a prompt.
        # $null on stdin: claude -p waits 3s for piped input otherwise, and in a
        # detached window with no console stdin that wait is worth removing.
        $null | & $claude.Source `
            --print `
            --model $Model `
            --permission-mode bypassPermissions `
            --verbose `
            $prompt | Tee-Object -FilePath $sessionLog

        $claudeExit = $LASTEXITCODE
        $elapsed = [int]((Get-Date) - $started).TotalMinutes
        Write-Loop "session $i finished in ${elapsed}m (claude exit $claudeExit)"

        if ($claudeExit -ne 0) {
            Write-Loop "STOPPING: claude exited non-zero. See $sessionLog"
            break
        }

        if (-not $SkipGates) {
            $gateLog = Join-Path $logDir "gates-$stamp.log"
            Write-Loop "running gate suite -> $gateLog"
            & powershell -ExecutionPolicy Bypass -File (Join-Path $repo 'tools\gates.ps1') |
                Tee-Object -FilePath $gateLog | Out-Null
            $gateExit = $LASTEXITCODE

            if ($gateExit -ne 0) {
                Write-Loop "STOPPING: gates RED after session $i. Tree needs a human. See $gateLog"
                break
            }
            Write-Loop "gates green after session $i"

            # The one place git runs (CLAUDE.md, user decision 2026-08-11): a
            # script, between sessions, on a tree that just passed every gate.
            # Sessions themselves still never touch version control.
            if (-not $NoCommit) {
                $subject = Get-SessionSubject
                & git add -A
                & git commit -m $subject --quiet
                if ($LASTEXITCODE -eq 0) {
                    Write-Loop "committed: $subject"
                    & git push --quiet
                    if ($LASTEXITCODE -eq 0) { Write-Loop "pushed to origin/main (CI will run)" }
                    else { Write-Loop "WARNING: push failed - the commit is safe locally; push by hand" }
                } else {
                    Write-Loop "nothing to commit after session $i"
                }
            }
        }
    }
}

Write-Loop "=== loop end ==="
Write-Loop "next action is the top of PROGRESS.md's NEXT ACTION section:"
if (Test-Path (Join-Path $repo 'PROGRESS.md')) {
    Get-Content (Join-Path $repo 'PROGRESS.md') -Tail 25 | ForEach-Object { Write-Host "  $_" }
}
