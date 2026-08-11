<#
    build-autocommit.ps1 - a bridge, not a permanent tool.

    build-loop.ps1 now commits and pushes itself after each green gate, but a
    PowerShell script is read into memory when it launches: a loop that is
    ALREADY RUNNING cannot pick that up. This watches that running loop's
    log and does the commit on its behalf, so the current run is protected
    without restarting it and throwing away a session.

    Once the current run ends, this script has no job. build-loop.ps1 does it.

    The timing, stated plainly: the loop logs "gates green after session N" and
    starts session N+1 in the same breath. This polls every 15s, so it commits
    within ~25s of green. A session spends its first ~85s reading files and
    running the gate suite before it writes anything, so there is around a
    minute of margin - comfortable, but not infinite. It skips the commit if a
    source file has been modified in the last 20s, which is the observable
    signature of a session that has already started writing.

    Usage:  .\build-autocommit.ps1          # run in its own window
#>
param(
    [int]$PollSeconds = 15
)

$ErrorActionPreference = 'Continue'
$repo = $PSScriptRoot
Set-Location $repo
$loopLog = Join-Path $repo '.logs\loop.log'
$seen = @{}

function Get-SessionSubject {
    $progress = Join-Path $repo 'PROGRESS.md'
    if (Test-Path $progress) {
        $hit = Select-String -Path $progress -Pattern '^\*\*Session\s+(\d+)' | Select-Object -First 1
        if ($hit) {
            $num = $hit.Matches[0].Groups[1].Value
            $claim = [regex]::Match($hit.Line, '^\*\*Session\s+\d+[^*]*\*\*\s*\*\*(.+?)\*\*')
            if ($claim.Success) { $text = $claim.Groups[1].Value }
            else { $text = $hit.Line -replace '^\*\*Session\s+\d+[^*]*\*\*\s*', '' }
            $text = $text -replace '\*\*', '' -replace '`', ''
            $text = ($text -replace '\s+', ' ').Trim(' ', '-', '.', ',')
            if ($text.Length -gt 64) { $text = $text.Substring(0, 64).TrimEnd() + '...' }
            if ($text) { return "build(session ${num}): $text" }
        }
    }
    return "build: autonomous session, all gates green ($(Get-Date -Format 'yyyy-MM-dd HH:mm'))"
}

function Test-SessionWriting {
    $cut = (Get-Date).AddSeconds(-20)
    $recent = Get-ChildItem -Path $repo -Recurse -File -Include *.rs, *.toml, *.md -ErrorAction SilentlyContinue |
        Where-Object { $_.LastWriteTime -gt $cut -and $_.FullName -notlike '*\target\*' -and $_.FullName -notlike '*\.logs\*' } |
        Select-Object -First 1
    return [bool]$recent
}

function Invoke-Commit([string]$Tag) {
    if (Test-SessionWriting) {
        Write-Host "[$(Get-Date -Format 'HH:mm:ss')] $Tag - a session is already writing; skipping this window" -ForegroundColor Yellow
        return
    }
    $subject = Get-SessionSubject
    & git add -A
    & git commit -m $subject --quiet
    if ($LASTEXITCODE -eq 0) {
        Write-Host "[$(Get-Date -Format 'HH:mm:ss')] committed: $subject" -ForegroundColor Green
        & git push --quiet
        if ($LASTEXITCODE -eq 0) { Write-Host "[$(Get-Date -Format 'HH:mm:ss')] pushed to origin/main" -ForegroundColor Green }
        else { Write-Host "[$(Get-Date -Format 'HH:mm:ss')] push FAILED - commit is safe locally" -ForegroundColor Red }
    } else {
        Write-Host "[$(Get-Date -Format 'HH:mm:ss')] nothing to commit" -ForegroundColor DarkGray
    }
}

Write-Host "watching $loopLog every ${PollSeconds}s - Ctrl+C to stop" -ForegroundColor Cyan

# Mark the green gates already in the log as seen: their work is committed by
# the first commit this makes, and re-committing per historical line is noise.
if (Test-Path $loopLog) {
    Select-String -Path $loopLog -Pattern 'gates green after session (\d+)' |
        ForEach-Object { $seen[$_.Matches[0].Groups[1].Value] = $true }
    Write-Host "already-green sessions noted: $($seen.Keys -join ', ')" -ForegroundColor DarkGray
}

while ($true) {
    Start-Sleep -Seconds $PollSeconds
    if (-not (Test-Path $loopLog)) { continue }

    foreach ($m in (Select-String -Path $loopLog -Pattern 'gates green after session (\d+)')) {
        $n = $m.Matches[0].Groups[1].Value
        if (-not $seen.ContainsKey($n)) {
            $seen[$n] = $true
            Write-Host "[$(Get-Date -Format 'HH:mm:ss')] session $n went green" -ForegroundColor Cyan
            Invoke-Commit "session $n"
        }
    }

    if (Select-String -Path $loopLog -Pattern '=== loop end ===' -Quiet) {
        Write-Host "loop finished - final commit, then exiting" -ForegroundColor Cyan
        Invoke-Commit 'loop end'
        break
    }
}
