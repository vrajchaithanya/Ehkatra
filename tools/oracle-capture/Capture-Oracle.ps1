<#
.SYNOPSIS
Capture Excel conformance vectors from the locally installed Microsoft Excel.

.DESCRIPTION
The oracle harness of ADR-024 / docs/32. For every function in the shipped
catalogue (crates/usk-formula/src/functions.rs) it drives real Excel over COM
across an edge-case grid -- types, coercions, boundary values, error inputs,
blanks -- and records input to output vectors as JSON under vectors/.

The premise, from docs/32: "the documentation lies and the binary doesn't." So
this tool never asserts what Excel should do. It asks Excel and writes down the
answer, with enough provenance that the answer stays falsifiable.

Host isolation (DP-S5): refuses to run while the user has Excel open unless
-Force, never quits an instance it did not start, restores every application
setting it changes, and writes only under tools/oracle-capture.

.PARAMETER Mode
  Oracle       drive real Excel (default)
  SpecDerived  emit the same file format from the grids' documented-behaviour
               `expect` blocks, marked oracle:false. For proving the pipeline on
               a machine without Excel -- never for gating conformance.

.PARAMETER Functions
Capture only these function names.

.PARAMETER Date1904
Capture under the 1904 date system. Output goes to vectors-1904/ so it cannot be
confused with the 1900 corpus.

.PARAMETER Force
Proceed even if Excel is already running (accepts the settings churn).

.EXAMPLE
powershell -ExecutionPolicy Bypass -File .\Capture-Oracle.ps1

.EXAMPLE
powershell -ExecutionPolicy Bypass -File .\Capture-Oracle.ps1 -Functions SUM,ROUND -Verbose
#>
[CmdletBinding()]
param(
    [ValidateSet('Oracle', 'SpecDerived')] [string] $Mode = 'Oracle',
    [string[]] $Functions,
    [string]   $GridDir,
    [string]   $OutDir,
    [switch]   $Date1904,
    [switch]   $Force,
    [switch]   $NoDisplayText
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$HarnessVersion = '1.0.0'
$Root = Split-Path -Parent $MyInvocation.MyCommand.Path

. (Join-Path $Root 'lib\OracleJson.ps1')
. (Join-Path $Root 'lib\OracleCom.ps1')
. (Join-Path $Root 'lib\OracleEval.ps1')
. (Join-Path $Root 'lib\OracleGrid.ps1')

if (-not $GridDir) { $GridDir = Join-Path $Root 'grids' }
if (-not $OutDir) {
    if ($Date1904) { $OutDir = Join-Path $Root 'vectors-1904' } else { $OutDir = Join-Path $Root 'vectors' }
}
if (-not (Test-Path $OutDir)) { [void](New-Item -ItemType Directory -Path $OutDir) }

$capturedUtc = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')

# `powershell -File script.ps1 -Functions SUM,ABS` hands the shell one literal
# string rather than an array, because -File suppresses PowerShell's own
# argument parsing. Split here so both invocation styles behave the same.
if ($Functions) {
    $Functions = @($Functions | ForEach-Object { $_ -split ',' } |
                   ForEach-Object { $_.Trim() } | Where-Object { $_ })
}

Write-Host "oracle-capture $HarnessVersion  mode=$Mode  grids=$GridDir"
$grids = Import-OracleGrids -GridDir $GridDir -Only $Functions
# Never $grids.Count: PowerShell resolves a dictionary member against the keys
# first, and the catalogue contains a function named COUNT, so $grids.Count
# silently returns that function's grid entry instead of the number of entries.
$gridNames = @($grids.Keys)
$totalCases = 0
foreach ($k in $gridNames) {
    foreach ($b in $grids[$k].Blocks) { $totalCases += $b.Cases.Count }
}
Write-Host "loaded $($gridNames.Count) function grids, $totalCases cases"

# --------------------------------------------------------------- provenance

$session = $null
$prov = [ordered]@{}
if ($Mode -eq 'Oracle') {
    $session = Open-OracleExcel -Force:$Force -Date1904:$Date1904
    $prov = Get-OracleExcelProvenance -Session $session
    if ($session.Borrowed) {
        Write-Warning 'Attached to an already-running Excel instance (-Force). It will not be quit.'
    }
    Write-Host "excel $($prov['excel_version']) build $($prov['excel_build'])  date system $($prov['date_system'])"
} else {
    $prov['source']      = 'spec-derived'
    $prov['oracle']      = $false
    $prov['warning']     = 'NOT ORACLE-CAPTURED. Documented behaviour transcribed by hand; docs/32 says the documentation lies. Do not gate conformance on this.'
    $prov['date_system'] = $(if ($Date1904) { '1904' } else { '1900' })
    $prov['os']          = [string][System.Environment]::OSVersion
    $prov['powershell']  = [string]$PSVersionTable.PSVersion
}
$prov['harness_version']   = $HarnessVersion
$prov['captured_utc']      = $capturedUtc
$prov['probe_column_width']= $script:ProbeColWidth
$prov['catalogue_source']  = 'crates/usk-formula/src/functions.rs CATALOGUE'

# ------------------------------------------------------------------ capture

$summary   = @()
$divergent = @()
$failed    = @()
$emitted   = 0

try {
    foreach ($name in $gridNames) {
        $fn = $grids[$name]
        $cases = @()
        $seq = 0
        $blockIndex = 0

        foreach ($block in $fn.Blocks) {
            $blockIndex++
            $observations = $null
            if ($Mode -eq 'Oracle') {
                try {
                    $observations = Invoke-OracleBlock -Session $session `
                        -Fixture $block.Fixture -Cases $block.Cases `
                        -CaptureDisplayText:(-not $NoDisplayText)
                } catch {
                    $failed += "$name block $blockIndex : $($_.Exception.Message)"
                    Write-Warning "$name block $blockIndex failed: $($_.Exception.Message)"
                    continue
                }
            }

            # The fixture travels with the block so a vector is reproducible
            # without the grid file -- the corpus must stand alone.
            $fixtureOut = @()
            foreach ($f in $block.Fixture) {
                $fo = [ordered]@{}
                $fo['ref'] = [string]$f['ref']
                foreach ($k in 'value', 'text', 'codepoints', 'formula', 'format') {
                    if ($f.Contains($k)) { $fo[$k] = $f[$k] }
                }
                if ($f.Contains('blank')) { $fo['blank'] = $true }
                $fixtureOut += ,$fo
            }

            for ($i = 0; $i -lt $block.Cases.Count; $i++) {
                $c = $block.Cases[$i]
                $seq++
                $case = [ordered]@{}
                if ($c.Contains('id')) {
                    $case['id'] = [string]$c['id']
                } else {
                    $case['id'] = ('{0}/{1:d4}' -f $name, $seq)
                }
                $case['formula'] = [string]$c['formula']
                $case['block']   = $blockIndex
                if ($block.Label) { $case['block_label'] = $block.Label }
                if ($fixtureOut.Count -gt 0) { $case['fixture'] = $fixtureOut }
                if ($c.Contains('tags'))  { $case['tags'] = @($c['tags']) }
                if ($c.Contains('note'))  { $case['note'] = [string]$c['note'] }

                if ($Mode -eq 'Oracle') {
                    $obs = $observations[$i]
                    if ($obs['stored'] -and $obs['stored'] -cne $case['formula']) {
                        # Excel rewrote what we wrote: _xlfn prefixes for
                        # functions this build does not know natively, or an
                        # implicit-intersection @. A conformance test must compare
                        # against what was actually evaluated.
                        $case['stored_formula'] = $obs['stored']
                    }
                    if ($obs['write_error']) {
                        # Excel would not accept the formula at all. That is a
                        # real observation about the parser, so it is recorded as
                        # one -- but there is no result value, and inventing a
                        # placeholder would put a lie in the corpus.
                        $case['observed'] = $null
                        $case['observed_status'] = 'rejected-by-excel'
                        $case['reject_reason']   = $obs['write_error']
                    } else {
                        $case['observed'] = ConvertTo-OracleExpect -Obs $obs
                    }

                    if ($c.Contains('expect') -and $case['observed']) {
                        $verdict = Test-OracleExpectation -Expect $c['expect'] -Observed $case['observed']
                        $case['spec_expect'] = $c['expect']
                        $case['spec_agreement'] = $verdict
                        if ($verdict -ne 'agree') {
                            $divergent += "$($case['id'])  $($case['formula'])  --  $verdict"
                        }
                    }
                } else {
                    if ($c.Contains('expect')) {
                        $e = [ordered]@{}
                        foreach ($k in @($c['expect'].Keys)) { $e[$k] = $c['expect'][$k] }
                        $case['observed'] = $e
                        $case['observed_status'] = 'spec-derived'
                    } else {
                        $case['observed'] = $null
                        $case['observed_status'] = 'uncaptured'
                    }
                }

                $cases += ,$case
            }
        }

        if ($cases.Count -eq 0) { continue }

        $doc = [ordered]@{}
        $doc['schema']     = 'ehkatra.oracle.vectors/1'
        $doc['function']   = $name
        $doc['group']      = $fn.Group
        if ($fn.Doc) { $doc['doc'] = $fn.Doc }
        $doc['grid_source']= $fn.SourceFile
        $doc['provenance'] = $prov
        $doc['case_count'] = $cases.Count
        $doc['cases']      = $cases

        $file = Join-Path $OutDir ("{0}.json" -f $name)
        Write-OracleJsonFile -Path $file -InputObject $doc
        $emitted += $cases.Count
        $summary += ,@{ Function = $name; Cases = $cases.Count; Group = $fn.Group }
        Write-Verbose "$name : $($cases.Count) vectors"
    }
} finally {
    if ($session) { Close-OracleExcel -Session $session }
}

# -------------------------------------------------------------------- index

$index = [ordered]@{}
$index['schema']      = 'ehkatra.oracle.index/1'
$index['provenance']  = $prov
$index['function_count'] = $summary.Count
$index['vector_count']   = $emitted
$fnList = @()
foreach ($s in ($summary | Sort-Object { $_.Function })) {
    $e = [ordered]@{}
    $e['function'] = $s.Function
    $e['group']    = $s.Group
    $e['cases']    = $s.Cases
    $e['file']     = "$($s.Function).json"
    $fnList += ,$e
}
$index['functions'] = $fnList
$index['spec_divergences'] = @($divergent)
$index['block_failures']   = @($failed)
Write-OracleJsonFile -Path (Join-Path $OutDir '_index.json') -InputObject $index

Write-Host ''
Write-Host "wrote $emitted vectors across $($summary.Count) functions to $OutDir"
if ($divergent.Count -gt 0) {
    Write-Host ''
    Write-Host "$($divergent.Count) case(s) where real Excel diverges from documented behaviour:" -ForegroundColor Yellow
    foreach ($d in $divergent) { Write-Host "  $d" -ForegroundColor Yellow }
}
if ($failed.Count -gt 0) {
    Write-Host ''
    Write-Host "$($failed.Count) block failure(s):" -ForegroundColor Red
    foreach ($f in $failed) { Write-Host "  $f" -ForegroundColor Red }
    exit 1
}
