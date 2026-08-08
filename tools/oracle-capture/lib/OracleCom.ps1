# OracleCom.ps1 -- Excel COM session lifecycle, with host isolation.
#
# DP-S5 (docs/07) forbids touching anything of the user's. Excel's COM server is
# registered multi-use, so `New-Object -ComObject Excel.Application` can hand
# back the instance the user is already working in. Quitting that instance, or
# setting DisplayAlerts = $false on it, would silently discard their unsaved
# work. So: detect a pre-existing instance, refuse by default, and never Quit an
# instance we did not start. Every global we change is captured and restored.

Set-StrictMode -Version Latest

# Excel enumerations, spelled out so no interop assembly is required.
$script:XlCalculationManual    = -4135
$script:XlCalculationAutomatic = -4105

function Get-RunningExcel {
    <#
      .SYNOPSIS
      The user's Excel instance, or $null. Never creates one.
    #>
    try {
        return [System.Runtime.InteropServices.Marshal]::GetActiveObject('Excel.Application')
    } catch {
        return $null
    }
}

function Open-OracleExcel {
    <#
      .SYNOPSIS
      Start a private, invisible Excel instance and an empty workbook.
      .PARAMETER Force
      Proceed even when the user already has Excel open. The session still
      refuses to Quit a borrowed instance, but a shared instance means our
      global settings churn is visible to the user -- hence opt-in.
      .PARAMETER Date1904
      Capture under the 1904 date system instead of 1900. The workbook-level
      switch docs/32 calls out as part of the compat catalogue.
      .OUTPUTS
      A session hashtable to hand to the other functions in this file.
    #>
    param(
        [switch] $Force,
        [switch] $Date1904
    )

    $borrowed = Get-RunningExcel
    if ($borrowed -and -not $Force) {
        throw ("Excel is already running. Capturing would mutate application-wide " +
               "settings on the instance holding your workbooks (DP-S5). Close Excel " +
               "and re-run, or pass -Force to accept that.")
    }

    $xl = New-Object -ComObject Excel.Application
    $startedByUs = -not $borrowed

    # Capture before mutating so Close-OracleExcel can put it all back.
    $saved = @{
        Visible        = $xl.Visible
        DisplayAlerts  = $xl.DisplayAlerts
        ScreenUpdating = $xl.ScreenUpdating
        Calculation    = $null   # only meaningful once a workbook exists
        EnableEvents   = $xl.EnableEvents
    }

    $xl.Visible        = $false
    $xl.DisplayAlerts  = $false
    $xl.ScreenUpdating = $false
    $xl.EnableEvents   = $false   # no add-in or VBA hook may observe the capture

    $wb = $xl.Workbooks.Add()
    $saved.Calculation = $xl.Calculation
    # Manual calculation with an explicit CalculateFull per block: automatic mode
    # recalculates on every single write, which is both slow and -- after the
    # sheet is cleared between blocks -- prone to reading a stale dependency.
    $xl.Calculation = $script:XlCalculationManual

    $wb.Date1904 = [bool]$Date1904
    $ws = $wb.Worksheets.Item(1)

    return @{
        App         = $xl
        Book        = $wb
        Sheet       = $ws
        StartedByUs = $startedByUs
        Borrowed    = [bool]$borrowed
        Saved       = $saved
        Date1904    = [bool]$Date1904
    }
}

function Get-OracleExcelProvenance {
    <#
      .SYNOPSIS
      Everything about this Excel that could change an answer.
      .DESCRIPTION
      A vector without its capture environment is an anecdote. Version, build,
      calculation engine version, date system and the locale separators all
      change results or formula parsing, so they travel with the corpus.
    #>
    param([Parameter(Mandatory = $true)] $Session)
    $xl = $Session.App
    $wb = $Session.Book

    # xlListSeparator = 5, xlDecimalSeparator = 3, xlThousandsSeparator = 4,
    # xlCountryCode = 1, xlDateOrder = 32 (Application.International).
    $listSep = ''
    $decSep  = ''
    $thouSep = ''
    try { $listSep = [string]$xl.International(5) } catch {}
    try { $decSep  = [string]$xl.International(3) } catch {}
    try { $thouSep = [string]$xl.International(4) } catch {}

    $langId = $null
    try { $langId = [int]$xl.LanguageSettings.LanguageID(1) } catch {}

    $calcVersion = $null
    try { $calcVersion = [string]$wb.CalculationVersion } catch {}

    $os = ''
    try { $os = (Get-CimInstance Win32_OperatingSystem).Caption + ' ' +
                (Get-CimInstance Win32_OperatingSystem).Version } catch { $os = [string][System.Environment]::OSVersion }

    $prov = [ordered]@{}
    $prov['source']             = 'excel-com'
    $prov['oracle']             = $true
    $prov['excel_version']      = [string]$xl.Version
    $prov['excel_build']        = [string]$xl.Build
    $prov['excel_product']      = [string]$xl.Name
    $prov['excel_calc_version'] = $calcVersion
    $prov['date_system']        = $(if ($Session.Date1904) { '1904' } else { '1900' })
    $prov['ui_language_id']     = $langId
    $prov['list_separator']     = $listSep
    $prov['decimal_separator']  = $decSep
    $prov['thousands_separator']= $thouSep
    $prov['os']                 = $os
    $prov['powershell']         = [string]$PSVersionTable.PSVersion
    return $prov
}

function Close-OracleExcel {
    <#
      .SYNOPSIS
      Discard the scratch workbook and restore everything we touched.
      .DESCRIPTION
      Quit happens only for an instance we created. For a borrowed instance we
      close our own workbook and restore the globals, leaving the user's session
      exactly as we found it.
    #>
    param([Parameter(Mandatory = $true)] $Session)

    $xl = $Session.App
    try { if ($Session.Book) { $Session.Book.Close($false) } } catch {}

    try {
        $xl.Calculation    = $Session.Saved.Calculation
    } catch {
        # Calculation is only settable while a workbook is open; if ours was the
        # last one, Excel rejects the restore. Harmless -- a fresh workbook in a
        # fresh instance defaults to automatic.
    }
    try { $xl.EnableEvents   = $Session.Saved.EnableEvents   } catch {}
    try { $xl.ScreenUpdating = $Session.Saved.ScreenUpdating } catch {}
    try { $xl.DisplayAlerts  = $Session.Saved.DisplayAlerts  } catch {}
    try { $xl.Visible        = $Session.Saved.Visible        } catch {}

    if ($Session.StartedByUs) {
        try { $xl.Quit() } catch {}
    }

    foreach ($k in 'Sheet', 'Book', 'App') {
        if ($Session[$k]) {
            try { [void][System.Runtime.InteropServices.Marshal]::ReleaseComObject($Session[$k]) } catch {}
        }
        $Session[$k] = $null
    }
    [System.GC]::Collect()
    [System.GC]::WaitForPendingFinalizers()
}
