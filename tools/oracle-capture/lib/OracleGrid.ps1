# OracleGrid.ps1 -- load and normalise the edge-case input grids.
#
# Grids are PowerShell data files (.psd1) rather than JSON for one reason: they
# are ~700 hand-authored Excel formulas, and `'=UPPER("abc")'` is readable where
# `"=UPPER(\"abc\")"` is not. Import-PowerShellDataFile parses literals only and
# executes nothing, so a grid file cannot run code (DP-S5: a data file that can
# execute is not a data file).
#
# Grid file shape:
#
#   @{
#     schema  = 'ehkatra.oracle.grid/1'
#     group   = 'math'
#     fixture = @( ... )            # optional, shared by every function below
#     functions = @(
#       @{
#         name    = 'SUM'
#         doc     = 'why these cases exist'
#         fixture = @( ... )        # optional, appended to the group fixture
#         cases   = @( @{ formula = '=SUM(A1:A5)'; tags = @('basic') } )
#         blocks  = @( ... )        # alternative to fixture+cases when a
#                                   # function needs several distinct fixtures
#       }
#     )
#   }

Set-StrictMode -Version Latest

function Import-OracleGrid {
    param([Parameter(Mandatory = $true)] [string] $Path)

    $data = Import-PowerShellDataFile -Path $Path
    if (-not $data.Contains('schema') -or $data['schema'] -ne 'ehkatra.oracle.grid/1') {
        throw "$Path : missing or unknown schema (expected 'ehkatra.oracle.grid/1')"
    }
    if (-not $data.Contains('functions')) {
        throw "$Path : no 'functions' key"
    }

    $groupFixture = @()
    if ($data.Contains('fixture')) { $groupFixture = @($data['fixture']) }
    $group = 'ungrouped'
    if ($data.Contains('group')) { $group = [string]$data['group'] }

    $result = @()
    foreach ($fn in @($data['functions'])) {
        if (-not $fn.Contains('name')) { throw "$Path : a function entry has no 'name'" }
        $name = [string]$fn['name']

        $fnFixture = @()
        if ($fn.Contains('fixture')) { $fnFixture = @($fn['fixture']) }

        $blocks = @()
        if ($fn.Contains('blocks')) {
            foreach ($b in @($fn['blocks'])) {
                $bf = @()
                if ($b.Contains('fixture')) { $bf = @($b['fixture']) }
                if (-not $b.Contains('cases')) { throw "$Path/$name : a block has no 'cases'" }
                $blocks += ,@{
                    Fixture = @($groupFixture + $fnFixture + $bf)
                    Cases   = @($b['cases'])
                    Label   = $(if ($b.Contains('label')) { [string]$b['label'] } else { $null })
                }
            }
        }
        if ($fn.Contains('cases')) {
            $blocks += ,@{
                Fixture = @($groupFixture + $fnFixture)
                Cases   = @($fn['cases'])
                Label   = $null
            }
        }
        if ($blocks.Count -eq 0) { throw "$Path/$name : neither 'cases' nor 'blocks'" }

        foreach ($b in $blocks) {
            foreach ($c in $b.Cases) {
                if (-not $c.Contains('formula')) {
                    throw "$Path/$name : a case has no 'formula'"
                }
            }
        }

        $result += ,@{
            Name       = $name
            Group      = $group
            Doc        = $(if ($fn.Contains('doc')) { [string]$fn['doc'] } else { $null })
            SourceFile = [System.IO.Path]::GetFileName($Path)
            Blocks     = $blocks
        }
    }
    return $result
}

function Import-OracleGrids {
    <#
      .SYNOPSIS
      Load every grid file, merging repeated function names across files.
      .DESCRIPTION
      A function may legitimately appear in two grid files -- SUM has aggregation
      cases in the maths grid and accumulation-order cases in the compat
      catalogue -- and both belong in the same output file. Blocks concatenate in
      file order so the vector ids are stable across runs.
    #>
    param(
        [Parameter(Mandatory = $true)] [string] $GridDir,
        [string[]] $Only
    )

    $files = @(Get-ChildItem -Path $GridDir -Filter '*.psd1' | Sort-Object Name)
    if ($files.Count -eq 0) { throw "No .psd1 grid files under $GridDir" }

    $byName = [ordered]@{}
    foreach ($f in $files) {
        foreach ($fn in (Import-OracleGrid -Path $f.FullName)) {
            if ($Only -and ($Only -notcontains $fn.Name)) { continue }
            if ($byName.Contains($fn.Name)) {
                $existing = $byName[$fn.Name]
                $existing.Blocks = @($existing.Blocks + $fn.Blocks)
                if (-not $existing.Doc) { $existing.Doc = $fn.Doc }
                $existing.SourceFile = "$($existing.SourceFile), $($fn.SourceFile)"
            } else {
                $byName[$fn.Name] = $fn
            }
        }
    }
    return $byName
}

function Test-OracleExpectation {
    <#
      .SYNOPSIS
      Compare a spec-derived expectation against what Excel actually did.
      .DESCRIPTION
      docs/32 states the position plainly: the documentation lies and the binary
      does not. So an `expect` in a grid is never authoritative -- it is a claim
      about documented behaviour, and this function's job is to find where that
      claim and the oracle part company. Those divergences are the corpus's most
      valuable output, because each one is a place our engine would have been
      wrong had we trusted the docs.
      .OUTPUTS
      'agree', or a string describing the divergence.
    #>
    param(
        [Parameter(Mandatory = $true)] $Expect,
        [Parameter(Mandatory = $true)] $Observed
    )
    $diffs = @()
    foreach ($key in @($Expect.Keys)) {
        $want = $Expect[$key]
        if (-not $Observed.Contains($key)) {
            $diffs += "$key : expected '$want', observed field absent"
            continue
        }
        $got = $Observed[$key]
        $same = $false
        if ($key -eq 'number') {
            # Compare as exact doubles: a vector that says 0.3 when Excel said
            # 0.30000000000000004 is the bug we are hunting, not a rounding nit.
            $same = ([double]$want) -eq ([double]$got)
        } elseif ($want -is [bool] -or $got -is [bool]) {
            $same = ([bool]$want) -eq ([bool]$got)
        } else {
            $same = ([string]$want) -ceq ([string]$got)
        }
        if (-not $same) { $diffs += "$key : expected '$want', observed '$got'" }
    }
    if ($diffs.Count -eq 0) { return 'agree' }
    return ($diffs -join '; ')
}
