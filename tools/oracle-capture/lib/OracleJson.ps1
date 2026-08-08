# OracleJson.ps1 -- a deterministic JSON writer for the oracle corpus.
#
# Why not ConvertTo-Json: the corpus is a *versioned* artifact that gets diffed
# across Excel builds, so byte-stability matters more than convenience. Windows
# PowerShell 5.1's ConvertTo-Json also renders doubles through the default "G15"
# path, which silently destroys the very thing we are here to capture -- the
# difference between 0.3 and 0.30000000000000004. This writer emits doubles with
# the round-trip "R" specifier under the invariant culture, and preserves key
# order from [ordered] dictionaries so a re-capture produces a minimal diff.

Set-StrictMode -Version Latest

$script:Inv = [System.Globalization.CultureInfo]::InvariantCulture

function Format-OracleDouble {
    <#
      .SYNOPSIS
      Round-trippable text for a double, as JSON would accept it.
      .DESCRIPTION
      "R" is the only .NET specifier that guarantees Parse(Format(x)) == x on
      every double. It is what makes a captured vector a fact rather than an
      approximation. Non-finite values cannot appear in JSON; Excel never
      produces them as cell values (it errors instead), so reaching that branch
      means the harness is wrong and the marker says so out loud.
    #>
    param([double] $Value)
    if ([double]::IsNaN($Value) -or [double]::IsInfinity($Value)) {
        return $null
    }
    $s = $Value.ToString('R', $script:Inv)
    # "R" can emit "1E+300"; JSON accepts that. It can also emit "6" for a whole
    # double, which is likewise valid JSON. No normalisation needed.
    return $s
}

function ConvertTo-OracleJsonString {
    param([string] $Value)
    $sb = New-Object System.Text.StringBuilder
    [void]$sb.Append('"')
    foreach ($ch in $Value.ToCharArray()) {
        $code = [int]$ch
        switch ($ch) {
            '"'  { [void]$sb.Append('\"');  continue }
            '\'  { [void]$sb.Append('\\');  continue }
            "`b" { [void]$sb.Append('\b');  continue }
            "`f" { [void]$sb.Append('\f');  continue }
            "`n" { [void]$sb.Append('\n');  continue }
            "`r" { [void]$sb.Append('\r');  continue }
            "`t" { [void]$sb.Append('\t');  continue }
            default {
                # Escape controls and anything above the BMP-safe ASCII range so
                # the file is pure-ASCII and cannot be corrupted by an encoding
                # mismatch on a later read. Text vectors deliberately contain
                # non-breaking spaces and CJK, and they must survive verbatim.
                if ($code -lt 0x20 -or $code -gt 0x7E) {
                    [void]$sb.Append('\u')
                    [void]$sb.Append($code.ToString('x4', $script:Inv))
                } else {
                    [void]$sb.Append($ch)
                }
            }
        }
    }
    [void]$sb.Append('"')
    return $sb.ToString()
}

function ConvertTo-OracleJson {
    <#
      .SYNOPSIS
      Serialise a value tree to stable, indented JSON.
      .PARAMETER InputObject
      Ordered dictionaries, hashtables, arrays, strings, numbers, booleans, $null.
      Hashtable keys are emitted sorted; [ordered] keys keep authored order.
    #>
    param(
        [Parameter(Mandatory = $true)] [AllowNull()] $InputObject,
        [int] $Indent = 0
    )
    $pad  = ' ' * (2 * $Indent)
    $pad1 = ' ' * (2 * ($Indent + 1))

    if ($null -eq $InputObject) { return 'null' }

    if ($InputObject -is [System.Collections.Specialized.OrderedDictionary] -or
        $InputObject -is [hashtable]) {
        $keys = @($InputObject.Keys)
        if ($InputObject -is [hashtable]) { $keys = @($keys | Sort-Object) }
        if ($keys.Count -eq 0) { return '{}' }
        $parts = @()
        foreach ($k in $keys) {
            $vs = ConvertTo-OracleJson -InputObject $InputObject[$k] -Indent ($Indent + 1)
            $parts += ('{0}{1}: {2}' -f $pad1, (ConvertTo-OracleJsonString ([string]$k)), $vs)
        }
        return "{`n" + ($parts -join ",`n") + "`n$pad}"
    }

    if ($InputObject -is [string]) { return ConvertTo-OracleJsonString $InputObject }
    if ($InputObject -is [bool])   { if ($InputObject) { return 'true' } else { return 'false' } }

    if ($InputObject -is [double] -or $InputObject -is [single] -or $InputObject -is [decimal]) {
        $d = [double]$InputObject
        $s = Format-OracleDouble -Value $d
        if ($null -eq $s) { return (ConvertTo-OracleJsonString ([string]$d)) }
        return $s
    }
    if ($InputObject -is [int] -or $InputObject -is [long] -or
        $InputObject -is [int16] -or $InputObject -is [byte]) {
        return ([long]$InputObject).ToString($script:Inv)
    }

    if ($InputObject -is [System.Collections.IEnumerable]) {
        $items = @($InputObject)
        if ($items.Count -eq 0) { return '[]' }
        $parts = @()
        foreach ($it in $items) {
            $parts += ($pad1 + (ConvertTo-OracleJson -InputObject $it -Indent ($Indent + 1)))
        }
        return "[`n" + ($parts -join ",`n") + "`n$pad]"
    }

    # Anything else is a harness bug; make it loud rather than silently stringly.
    throw "ConvertTo-OracleJson: unsupported type $($InputObject.GetType().FullName)"
}

function Write-OracleJsonFile {
    param(
        [Parameter(Mandatory = $true)] [string] $Path,
        [Parameter(Mandatory = $true)] $InputObject
    )
    $json = (ConvertTo-OracleJson -InputObject $InputObject) + "`n"
    # UTF-8 without BOM, LF line endings: the corpus is read by Rust tests and
    # diffed by git, and a BOM breaks naive serde readers.
    $bytes = [System.Text.UTF8Encoding]::new($false).GetBytes(($json -replace "`r`n", "`n"))
    [System.IO.File]::WriteAllBytes($Path, $bytes)
}
