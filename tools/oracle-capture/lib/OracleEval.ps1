# OracleEval.ps1 -- drive one fixture block through real Excel and read it back.
#
# Sheet layout, fixed so grid authors can write plain A1 references:
#
#   A1:H40   fixture region -- literal cells the probes reference
#   J1:J{n}  probe formulas, one per case
#   K1:K{n}  =IF(ISERROR(Jx),"",Jx&"")   General-format text of the result
#   L1:L{n}  type tag: error | blank | text | logical | number
#   M1:M{n}  =IFERROR(ERROR.TYPE(Jx),0)  error class, 0 when not an error
#
# K is the semantically load-bearing capture: `x&""` is Excel's value-to-text
# coercion, which is where the 15-significant-digit display rule lives (D-041,
# `compat_round_15`). It is not a formatting artefact of column width the way
# Range.Text is, so it is the column a conformance test should assert against.
#
# Occupying K also gives spill detection for free: a probe that returns a
# dynamic array cannot spill into an occupied cell, so Excel reports #SPILL!
# (ERROR.TYPE 9) rather than silently overwriting the companions.

Set-StrictMode -Version Latest

$script:FixtureFirstCol = 1    # A
$script:FixtureLastCol  = 8    # H
$script:FixtureLastRow  = 40
$script:ProbeCol        = 10   # J
$script:TextCol         = 11   # K
$script:TypeCol         = 12   # L
$script:ErrCol          = 13   # M
$script:ProbeColWidth   = 80   # fixed so Range.Text is reproducible

# ERROR.TYPE codes. 1-8 are the classic set; 9-14 arrived with dynamic arrays
# and linked data types. A code we do not know is reported as its number rather
# than guessed at.
$script:ErrorTypeNames = @{
    1  = '#NULL!'
    2  = '#DIV/0!'
    3  = '#VALUE!'
    4  = '#REF!'
    5  = '#NAME?'
    6  = '#NUM!'
    7  = '#N/A'
    8  = '#GETTING_DATA'
    9  = '#SPILL!'
    10 = '#CONNECT!'
    11 = '#BLOCKED!'
    12 = '#UNKNOWN!'
    13 = '#FIELD!'
    14 = '#CALC!'
}

# CVErr codes as they marshal through Value2. Kept as an independent cross-check
# on ERROR.TYPE: two disagreeing sources beat one trusted source.
$script:CvErrNames = @{
    -2146826288 = '#NULL!'
    -2146826281 = '#DIV/0!'
    -2146826273 = '#VALUE!'
    -2146826265 = '#REF!'
    -2146826259 = '#NAME?'
    -2146826252 = '#NUM!'
    -2146826246 = '#N/A'
    -2146826245 = '#GETTING_DATA'
}

function Reset-OracleSheet {
    param([Parameter(Mandatory = $true)] $Session)
    $ws = $Session.Sheet
    # Clear() and ClearContents() return a Boolean. Left unvoided, that Boolean
    # joins the calling function's output stream and corrupts its return value --
    # PowerShell's most expensive default.
    [void]$ws.Cells.Clear()
    $ws.Cells.NumberFormat = 'General'
    $ws.Columns.Item($script:ProbeCol).ColumnWidth = $script:ProbeColWidth
}

function Set-OracleCellValue {
    <#
      .SYNOPSIS
      Write one typed literal into a cell, exactly.
      .DESCRIPTION
      Windows PowerShell 5.1's COM adapter rejects a scalar Double, Boolean or
      String assigned to Range.Value2 ("Specified cast is not valid") while
      accepting an Int32, so a fixture written the obvious way silently loses
      every non-integer. Handing Value2 a 1x1 object[,] goes through the array
      path instead, which marshals every VARIANT type faithfully. Value2 (not
      Value) also keeps Excel from reinterpreting the write through the cell's
      number format.
    #>
    param(
        [Parameter(Mandatory = $true)] $Cell,
        [Parameter(Mandatory = $true)] [AllowNull()] $Value
    )
    $box = New-Object 'object[,]' 1, 1
    $box[0, 0] = $Value
    $Cell.Value2 = $box
}

function Write-OracleFixture {
    <#
      .SYNOPSIS
      Seed the literal cells a block's probes reference.
      .DESCRIPTION
      Three ways to seed, because the difference between them is exactly what a
      coercion test measures:
        value   -- written through Value2, so "123" becomes the number 123
        text    -- cell pre-formatted as Text first, so "1E2" stays the string
                  "1E2" instead of being helpfully turned into 100
        formula -- written through Formula, the only way to seed an error cell
                  (=NA()) or a real date serial
    #>
    param(
        [Parameter(Mandatory = $true)] $Session,
        [Parameter(Mandatory = $true)] [AllowEmptyCollection()] [array] $Fixture
    )
    $ws = $Session.Sheet
    foreach ($f in $Fixture) {
        $ref = [string]$f.ref
        $cell = $ws.Range($ref)
        if ($cell.Column -lt $script:FixtureFirstCol -or
            $cell.Column -gt $script:FixtureLastCol -or
            $cell.Row    -gt $script:FixtureLastRow) {
            throw "Fixture cell $ref is outside the A1:H$($script:FixtureLastRow) fixture region"
        }
        if ($f.Contains('codepoints')) {
            # A string given as Unicode code points, so a grid file can seed
            # astral characters, combining marks and non-breaking spaces while
            # staying pure ASCII on disk. This path writes a real .NET string
            # through Value2, which is a genuinely different route into Excel
            # than a UNICHAR() formula -- and whether the two agree is itself a
            # question the corpus needs to answer (see LEN's astral cases).
            $sb = New-Object System.Text.StringBuilder
            foreach ($cp in @($f['codepoints'])) {
                $n = [int]$cp
                if ($n -ge 0xD800 -and $n -le 0xDFFF) {
                    # A lone surrogate code unit. ConvertFromUtf32 rejects these
                    # by design, but writing a pair of them explicitly is the
                    # point: it is how a grid expresses "the UTF-16 encoding of
                    # this character" as distinct from "this character", which is
                    # exactly the distinction the astral cases are testing.
                    [void]$sb.Append([char]$n)
                } else {
                    [void]$sb.Append([char]::ConvertFromUtf32($n))
                }
            }
            $cell.NumberFormat = '@'
            Set-OracleCellValue -Cell $cell -Value $sb.ToString()
        } elseif ($f.Contains('text')) {
            # Format as Text *before* writing: otherwise "1E2" becomes 100 and
            # "1/2/2024" becomes serial 45293, which is the gene-symbol mangling
            # this fixture exists to hold constant.
            $cell.NumberFormat = '@'
            Set-OracleCellValue -Cell $cell -Value ([string]$f['text'])
        } elseif ($f.Contains('formula')) {
            $cell.Formula = [string]$f['formula']
        } elseif ($f.Contains('value')) {
            Set-OracleCellValue -Cell $cell -Value $f['value']
        } elseif ($f.Contains('blank')) {
            [void]$cell.ClearContents()
        } else {
            throw "Fixture cell $ref has none of: value, text, formula, blank"
        }
        if ($f.Contains('format')) { $cell.NumberFormat = [string]$f['format'] }
        [void][System.Runtime.InteropServices.Marshal]::ReleaseComObject($cell)
    }
}

function Get-ColLetter {
    param([int] $Index)
    # 1 -> A. The sheet layout never reaches two letters, but be correct anyway.
    $s = ''
    $n = $Index
    while ($n -gt 0) {
        $r = ($n - 1) % 26
        $s = [char](65 + $r) + $s
        $n = [int](($n - 1) / 26)
    }
    return $s
}

function Invoke-OracleBlock {
    <#
      .SYNOPSIS
      Evaluate one block's cases in real Excel and return raw observations.
      .OUTPUTS
      One hashtable per case: formula as authored, formula as Excel stored it,
      the result kind, the exact double, the General-format text, the displayed
      text, and any error name.
    #>
    param(
        [Parameter(Mandatory = $true)] $Session,
        [Parameter(Mandatory = $true)] [AllowEmptyCollection()] [array] $Fixture,
        [Parameter(Mandatory = $true)] [array] $Cases,
        [switch] $CaptureDisplayText
    )

    $ws = $Session.Sheet
    Reset-OracleSheet -Session $Session
    Write-OracleFixture -Session $Session -Fixture $Fixture

    $n = $Cases.Count
    $probeL = Get-ColLetter $script:ProbeCol
    $textL  = Get-ColLetter $script:TextCol
    $typeL  = Get-ColLetter $script:TypeCol
    $errL   = Get-ColLetter $script:ErrCol

    # Build the probe block plus its three companion columns as one 2-D array.
    $grid = New-Object 'object[,]' $n, 4
    for ($i = 0; $i -lt $n; $i++) {
        $row = $i + 1
        $p = "$probeL$row"
        $grid[$i, 0] = [string]$Cases[$i].formula
        $grid[$i, 1] = "=IF(ISERROR($p),`"`",$p&`"`")"
        $grid[$i, 2] = "=IF(ISERROR($p),`"error`",IF(ISBLANK($p),`"blank`",IF(ISTEXT($p),`"text`",IF(ISLOGICAL($p),`"logical`",`"number`"))))"
        $grid[$i, 3] = "=IFERROR(ERROR.TYPE($p),0)"
    }

    $writeErrors = @{}
    $range = $ws.Range("$probeL`1:$errL$n")
    try {
        $range.Formula = $grid
    } catch {
        # A hand-authored grid will eventually contain something Excel refuses to
        # parse. Fall back to per-row writes so one bad formula costs one case
        # instead of the whole block, and record which one it was.
        for ($i = 0; $i -lt $n; $i++) {
            $row = $i + 1
            try {
                $ws.Range("$probeL$row").Formula = $grid[$i, 0]
            } catch {
                # Excel's *parser* refused the formula -- which is itself a
                # conformance fact, not a harness failure: numeric literals at
                # or beyond 1E+308, and at or below 1E-310, are unreachable
                # from formula text. The cell is left empty so the observation
                # comes back as a rejection rather than a fabricated value.
                $writeErrors[$row] = $_.Exception.Message
                [void]$ws.Range("$probeL$row").ClearContents()
            }
            $ws.Range("$textL$row").Formula = $grid[$i, 1]
            $ws.Range("$typeL$row").Formula = $grid[$i, 2]
            $ws.Range("$errL$row").Formula  = $grid[$i, 3]
        }
    }

    $Session.Book.Application.CalculateFull()

    $values   = $ws.Range("$probeL`1:$errL$n").Value2
    $stored   = $ws.Range("$probeL`1:$probeL$n").Formula

    $out = @()
    for ($i = 0; $i -lt $n; $i++) {
        $row = $i + 1
        $raw     = $values[$row, 1]
        $genText = $values[$row, 2]
        $tag     = [string]$values[$row, 3]
        $errCode = 0
        if ($null -ne $values[$row, 4]) { $errCode = [int][double]$values[$row, 4] }

        $storedFormula = $null
        if ($n -eq 1) { $storedFormula = [string]$stored } else { $storedFormula = [string]$stored[$row, 1] }

        $obs = [ordered]@{}
        $obs['row']            = $row
        $obs['authored']       = [string]$Cases[$i].formula
        $obs['stored']         = $storedFormula
        $obs['tag']            = $tag
        $obs['raw']            = $raw
        $obs['general_text']   = [string]$genText
        $obs['error_code']     = $errCode
        $obs['error_name']     = $null
        $obs['cverr']          = $null
        $obs['display_text']   = $null
        $obs['write_error']    = $null

        if ($writeErrors.ContainsKey($row)) { $obs['write_error'] = $writeErrors[$row] }

        if ($tag -eq 'error') {
            if ($script:ErrorTypeNames.ContainsKey($errCode)) {
                $obs['error_name'] = $script:ErrorTypeNames[$errCode]
            } else {
                $obs['error_name'] = "#ERRORTYPE($errCode)"
            }
            if ($raw -is [int]) {
                $obs['cverr'] = [int]$raw
                if ($script:CvErrNames.ContainsKey([int]$raw)) {
                    $obs['cverr_name'] = $script:CvErrNames[[int]$raw]
                }
            }
        }

        if ($CaptureDisplayText) {
            try { $obs['display_text'] = [string]$ws.Range("$probeL$row").Text } catch {}
        }

        $out += ,$obs
    }
    return $out
}

function ConvertTo-OracleExpect {
    <#
      .SYNOPSIS
      Turn one raw observation into the `observed` block a vector file carries.
      .DESCRIPTION
      `kind` is the discriminant a consuming test switches on. For numbers both
      the JSON double and its round-trip text are emitted: the text is
      authoritative, the number is for readers that parse doubles correctly.
    #>
    param([Parameter(Mandatory = $true)] $Obs)

    $o = [ordered]@{}
    switch ($Obs['tag']) {
        'error' {
            $o['kind']  = 'error'
            $o['error'] = $Obs['error_name']
            $o['error_type_code'] = $Obs['error_code']
            if ($null -ne $Obs['cverr']) { $o['cverr'] = $Obs['cverr'] }
        }
        'number' {
            $d = [double]$Obs['raw']
            $o['kind']      = 'number'
            $o['number']    = $d
            $o['number_r17']= (Format-OracleDouble -Value $d)
        }
        'logical' {
            $o['kind']    = 'logical'
            $o['logical'] = [bool]$Obs['raw']
        }
        'text' {
            $o['kind'] = 'text'
            $o['text'] = [string]$Obs['raw']
        }
        'blank' {
            $o['kind'] = 'blank'
        }
        default {
            $o['kind'] = 'unknown'
            $o['note'] = "unrecognised type tag '$($Obs['tag'])'"
        }
    }
    # Present for every kind: what Excel would splice into a string, and what it
    # paints on screen at the recorded column width.
    $o['general_text'] = $Obs['general_text']
    if ($null -ne $Obs['display_text']) { $o['display_text'] = $Obs['display_text'] }
    return $o
}
