<#
.SYNOPSIS
Validate a captured vector corpus without Excel.

.DESCRIPTION
A corpus is only useful if a consumer can trust its shape. This checks the
invariants a Rust conformance test will rely on, so a malformed capture fails
here rather than as a confusing deserialisation error later:

  * every file declares ehkatra.oracle.vectors/1 and names its function
  * case ids are unique within a file
  * every observation carries a `kind` from the closed set, with the field that
    kind requires (number+number_r17, text, logical, error)
  * every number_r17 is canonical -- Parse then re-format returns the same text
    -- and the JSON `number` field is byte-identical to it in the raw file. This
    is the check that matters most: it proves the corpus did not lose the low
    bits of a double on its way through JSON.

    Note the comparison is done against the raw file text, not against the
    deserialised object. Windows PowerShell's ConvertFrom-Json parses a
    non-integer JSON number into System.Decimal, and .NET's Decimal-to-Double
    conversion is not correctly rounded -- it moves values like
    0.10000000000000009 by one ULP. Comparing parsed numbers would therefore
    report six false failures on a corpus that is bit-exact. That hazard is the
    reason number_r17 exists as a string at all: a consumer must read it, not
    the JSON number, unless its parser is known to be correctly rounding.
  * an absent observation is explained by observed_status, never silently null
  * the index agrees with the files it indexes

Exit code 0 when clean, 1 otherwise. Runs on any machine; no COM, no Excel.

.EXAMPLE
powershell -ExecutionPolicy Bypass -File .\Verify-Vectors.ps1
#>
[CmdletBinding()]
param([string] $VectorDir)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$Root = Split-Path -Parent $MyInvocation.MyCommand.Path
if (-not $VectorDir) { $VectorDir = Join-Path $Root 'vectors' }
$inv = [System.Globalization.CultureInfo]::InvariantCulture

$problems = @()
$stats = @{
    Files = 0; Cases = 0; Rejected = 0; Rewritten = 0
    Kinds = @{}; Tags = @{}; Groups = @{}
}
$validKinds = @('number', 'text', 'logical', 'blank', 'error')

$files = @(Get-ChildItem -Path $VectorDir -Filter '*.json' |
           Where-Object { $_.Name -ne '_index.json' } | Sort-Object Name)
if ($files.Count -eq 0) { throw "No vector files under $VectorDir" }

foreach ($f in $files) {
    $stats.Files++
    $raw = Get-Content -Raw -Path $f.FullName
    $d = $raw | ConvertFrom-Json

    # Byte-level precision check, done on the file text so no JSON number parser
    # sits between the corpus and the assertion. The writer emits `number` and
    # `number_r17` adjacently, and they must be the same characters.
    foreach ($m in [regex]::Matches($raw, '"number":\s*([^,\r\n]+),\s*\r?\n\s*"number_r17":\s*"([^"]+)"')) {
        $jsonNum = $m.Groups[1].Value.Trim()
        $r17     = $m.Groups[2].Value
        if ($jsonNum -cne $r17) {
            $problems += "$($f.Name): JSON number '$jsonNum' differs from number_r17 '$r17'"
        }
        $parsed = 0.0
        if (-not [double]::TryParse($r17, [Globalization.NumberStyles]::Float, $inv, [ref]$parsed)) {
            $problems += "$($f.Name): number_r17 '$r17' does not parse as a double"
        } elseif ($parsed.ToString('R', $inv) -cne $r17) {
            $problems += ("$($f.Name): number_r17 '$r17' is not canonical; " +
                          "re-formatting gives '$($parsed.ToString('R',$inv))'")
        }
    }

    if ($d.schema -ne 'ehkatra.oracle.vectors/1') {
        $problems += "$($f.Name): schema is '$($d.schema)'"
    }
    if (-not $d.function) { $problems += "$($f.Name): no function name" }
    if ($d.function -and ($f.BaseName -ne $d.function)) {
        $problems += "$($f.Name): filename does not match function '$($d.function)'"
    }
    if (-not $d.provenance) { $problems += "$($f.Name): no provenance" }
    if ($d.case_count -ne @($d.cases).Count) {
        $problems += "$($f.Name): case_count $($d.case_count) but $(@($d.cases).Count) cases"
    }
    if ($d.group) {
        if (-not $stats.Groups.ContainsKey($d.group)) { $stats.Groups[$d.group] = 0 }
        $stats.Groups[$d.group] += @($d.cases).Count
    }

    $seen = @{}
    foreach ($c in @($d.cases)) {
        $stats.Cases++
        if (-not $c.id) { $problems += "$($f.Name): a case has no id"; continue }
        if ($seen.ContainsKey($c.id)) { $problems += "$($f.Name): duplicate id $($c.id)" }
        $seen[$c.id] = $true
        if (-not $c.formula) { $problems += "$($c.id): no formula" }
        if ($c.formula -and -not $c.formula.StartsWith('=')) {
            $problems += "$($c.id): formula does not start with '=' : $($c.formula)"
        }
        if ($c.PSObject.Properties.Name -contains 'stored_formula') { $stats.Rewritten++ }
        foreach ($t in @($c.tags)) {
            if (-not $stats.Tags.ContainsKey($t)) { $stats.Tags[$t] = 0 }
            $stats.Tags[$t]++
        }

        $o = $c.observed
        if ($null -eq $o) {
            # An unobserved case must say why. A silent null is the one thing a
            # consuming test cannot distinguish from a harness bug.
            $status = $null
            if ($c.PSObject.Properties.Name -contains 'observed_status') { $status = $c.observed_status }
            if (-not $status) {
                $problems += "$($c.id): observed is null with no observed_status"
            } elseif ($status -eq 'rejected-by-excel') {
                $stats.Rejected++
                if (-not $c.reject_reason) { $problems += "$($c.id): rejected with no reason" }
            }
            continue
        }

        $kind = $o.kind
        if ($validKinds -notcontains $kind) {
            $problems += "$($c.id): kind '$kind' is not one of $($validKinds -join '/')"
            continue
        }
        if (-not $stats.Kinds.ContainsKey($kind)) { $stats.Kinds[$kind] = 0 }
        $stats.Kinds[$kind]++

        $has = $o.PSObject.Properties.Name
        switch ($kind) {
            'number' {
                # Presence only. The precision assertion is the raw-text check
                # above, which does not route the value through this shell's
                # lossy JSON number parsing.
                if ($has -notcontains 'number' -or $has -notcontains 'number_r17') {
                    $problems += "$($c.id): kind number without number/number_r17"
                }
            }
            'text'    { if ($has -notcontains 'text')    { $problems += "$($c.id): kind text without text" } }
            'logical' { if ($has -notcontains 'logical') { $problems += "$($c.id): kind logical without logical" } }
            'error'   {
                if ($has -notcontains 'error') { $problems += "$($c.id): kind error without error" }
                elseif ($o.error -notmatch '^#') { $problems += "$($c.id): error '$($o.error)' is not an Excel error name" }
            }
        }
        if ($has -notcontains 'general_text') {
            $problems += "$($c.id): no general_text"
        }
    }
}

# The index must describe the corpus it sits next to.
$indexPath = Join-Path $VectorDir '_index.json'
if (Test-Path $indexPath) {
    $idx = Get-Content -Raw -Path $indexPath | ConvertFrom-Json
    if ($idx.vector_count -ne $stats.Cases) {
        $problems += "_index.json: vector_count $($idx.vector_count) but $($stats.Cases) cases on disk"
    }
    if (@($idx.functions).Count -ne $stats.Files) {
        $problems += "_index.json: lists $(@($idx.functions).Count) functions but $($stats.Files) files on disk"
    }
} else {
    $problems += 'no _index.json'
}

Write-Host "corpus: $($stats.Files) files, $($stats.Cases) vectors"
Write-Host "  kinds     : $(($stats.Kinds.GetEnumerator() | Sort-Object Name | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join '  ')"
Write-Host "  groups    : $(($stats.Groups.GetEnumerator() | Sort-Object Name | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join '  ')"
Write-Host "  rejected  : $($stats.Rejected) (Excel's parser refused the formula)"
Write-Host "  rewritten : $($stats.Rewritten) (Excel stored something other than what was written)"
Write-Host "  tags      : $($stats.Tags.Keys.Count) distinct"

if ($problems.Count -gt 0) {
    Write-Host ''
    Write-Host "$($problems.Count) problem(s):" -ForegroundColor Red
    foreach ($p in $problems) { Write-Host "  $p" -ForegroundColor Red }
    exit 1
}
Write-Host ''
Write-Host 'corpus is well-formed' -ForegroundColor Green
