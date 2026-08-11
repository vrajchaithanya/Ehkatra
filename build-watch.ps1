<#
    build-watch.ps1 - see what the autonomous loop is doing right now.

    `claude --print` writes its transcript only when a session ENDS, so a
    session log sits at 0 bytes for the whole 25-60 minutes it is working.
    That is normal and is not a hang. This shows the signals that do move:
    the claude process's CPU, files it has touched, and the loop's own log.

    Usage:  .\build-watch.ps1          # one look
            .\build-watch.ps1 -Follow  # refresh every 60s until Ctrl+C
#>
param(
    [switch]$Follow,
    [int]$IntervalSeconds = 60,
    [int]$RecentMinutes = 15
)

$repo = $PSScriptRoot

function Show-State {
    Clear-Host
    Write-Host "=== Ehkatra build loop @ $(Get-Date -Format 'HH:mm:ss') ===" -ForegroundColor Cyan

    $loopLog = Join-Path $repo '.logs\loop.log'
    if (Test-Path $loopLog) {
        Write-Host "`n-- loop.log (last 6) --" -ForegroundColor Yellow
        Get-Content $loopLog -Tail 6
    } else {
        Write-Host "`nno .logs\loop.log - loop has not been started" -ForegroundColor DarkGray
    }

    Write-Host "`n-- claude processes over 200MB (a working session) --" -ForegroundColor Yellow
    # Only processes young enough to be this loop's - the desktop app leaves
    # long-lived claude processes around that are nothing to do with the build.
    $busy = Get-Process claude -ErrorAction SilentlyContinue |
        Where-Object { $_.WorkingSet64 -gt 200MB -and $_.StartTime -gt (Get-Date).AddHours(-3) } |
        Sort-Object StartTime -Descending
    if ($busy) {
        $busy | ForEach-Object {
            $mins = [int]((Get-Date) - $_.StartTime).TotalMinutes
            "  PID {0}  running {1}m  CPU {2}s  {3}MB" -f $_.Id, $mins, [math]::Round($_.CPU, 0), [math]::Round($_.WorkingSet64 / 1MB)
        }
    } else {
        Write-Host "  none - between sessions, or the loop has finished" -ForegroundColor DarkGray
    }

    Write-Host "`n-- repo files touched in the last $RecentMinutes min --" -ForegroundColor Yellow
    $cutoff = (Get-Date).AddMinutes(-$RecentMinutes)
    $touched = Get-ChildItem -Path $repo -Recurse -File -Include *.rs, *.md, *.toml, *.ps1 -ErrorAction SilentlyContinue |
        Where-Object { $_.LastWriteTime -gt $cutoff -and $_.FullName -notlike '*\target\*' -and $_.FullName -notlike '*\.logs\*' } |
        Sort-Object LastWriteTime -Descending | Select-Object -First 15
    if ($touched) {
        $touched | ForEach-Object { "  {0}  {1}" -f $_.LastWriteTime.ToString('HH:mm:ss'), $_.FullName.Replace("$repo\", '') }
    } else {
        Write-Host "  none yet - a session reads and runs the gate suite before it writes anything" -ForegroundColor DarkGray
    }

    Write-Host "`n-- newest session entry in PROGRESS.md --" -ForegroundColor Yellow
    $progress = Join-Path $repo 'PROGRESS.md'
    if (Test-Path $progress) {
        $last = Select-String -Path $progress -Pattern '(^#+\s*Session\s|^\*\*Session\s\d+)' | Select-Object -First 1  # newest first: entries are prepended
        if ($last) { "  line $($last.LineNumber): $($last.Line.Trim())" } else { "  no session headings found" }
    }

    $gate = Get-ChildItem (Join-Path $repo '.logs\gates-*.log') -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if ($gate) {
        $verdict = Select-String -Path $gate.FullName -Pattern 'ALL GATES GREEN' -Quiet
        Write-Host "`n-- last gate run: $($gate.Name) --" -ForegroundColor Yellow
        if ($verdict) { Write-Host "  ALL GATES GREEN" -ForegroundColor Green }
        else { Write-Host "  NOT GREEN - read $($gate.FullName)" -ForegroundColor Red }
    }
}

if ($Follow) {
    while ($true) { Show-State; Write-Host "`n(refreshing every ${IntervalSeconds}s - Ctrl+C to stop)" -ForegroundColor DarkGray; Start-Sleep -Seconds $IntervalSeconds }
} else {
    Show-State
}
