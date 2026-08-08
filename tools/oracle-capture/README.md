# oracle-capture

The Excel conformance oracle of ADR-024 / docs/32. Drives the locally installed
Microsoft Excel over COM across an edge-case grid and records input→output
vectors as JSON.

Full plan, findings and corpus format: **`docs/50-oracle-capture-plan.md`**.

## Run

Excel must be installed and **closed**.

```powershell
cd tools\oracle-capture
powershell -ExecutionPolicy Bypass -File .\Capture-Oracle.ps1      # capture
powershell -ExecutionPolicy Bypass -File .\Verify-Vectors.ps1      # validate (no Excel needed)
```

| Flag | Effect |
|---|---|
| `-Functions SUM,ROUND` | capture only these |
| `-Date1904` | capture under the 1904 date system, into `vectors-1904/` |
| `-Force` | proceed even if Excel is already running |
| `-NoDisplayText` | skip the per-cell `Range.Text` read (faster) |
| `-Mode SpecDerived` | emit the file format from grid `expect` blocks, `oracle: false`. Pipeline smoke test only — never gate conformance on it |
| `-Verbose` | per-function progress |

## Layout

```
Capture-Oracle.ps1     driver: load grids, open Excel, run blocks, emit JSON
Verify-Vectors.ps1     offline corpus validator, CI-safe
lib/OracleCom.ps1      COM session lifecycle + DP-S5 host isolation
lib/OracleEval.ps1     sheet layout, fixture seeding, block evaluation
lib/OracleGrid.ps1     .psd1 grid loading, normalisation, expect comparison
lib/OracleJson.ps1     deterministic JSON writer (round-trip doubles, stable key order)
grids/*.psd1           the edge-case input grids
vectors/*.json         captured corpus, 1900 date system  (one file per function)
vectors-1904/*.json    captured corpus, 1904 date system
```

## Adding cases

Edit or add a `grids/*.psd1`. A grid file is a single hashtable literal:

```powershell
@{
    schema = 'ehkatra.oracle.grid/1'
    group  = 'math'
    fixture = @(                       # optional, shared by every function below
        @{ ref = 'A1'; value = 1 }     # written via Value2: "123" becomes 123
        @{ ref = 'A2'; text  = '1E2' } # cell formatted as Text first, so it stays "1E2"
        @{ ref = 'A3'; formula = '=NA()' }          # the only way to seed an error
        @{ ref = 'A4'; blank = $true }
        @{ ref = 'A5'; codepoints = @(128512) }     # Unicode by code point, keeps grids ASCII
    )
    functions = @(
        @{
            name  = 'SUM'
            doc   = 'why these cases exist'
            cases = @(
                @{ formula = '=SUM(A1:A5)'; tags = @('range'); note = 'optional' }
                @{ formula = '=SUM(1,2)';   expect = @{ kind = 'number'; number = 3 } }
            )
            blocks = @(                # alternative to fixture+cases when a
                @{ label = '...'       # function needs several distinct fixtures
                   fixture = @( ... )
                   cases   = @( ... ) }
            )
        }
    )
}
```

Rules worth knowing before you author:

* **Fixtures live in `A1:H40`.** Probes go in column J with companions in K/L/M,
  so a fixture reference outside that region is rejected.
* **Keep grid files pure ASCII.** Build non-ASCII inputs with `UNICHAR()` or the
  `codepoints` field. A `.psd1` has no BOM and PowerShell will guess a codepage.
* **`expect` is never authoritative.** It is a claim about *documented* behaviour;
  the harness compares it to the oracle and lists disagreements in
  `_index.json` → `spec_divergences`. Finding one is a success.
* **Avoid formulas that spill.** A dynamic array cannot spill into the occupied
  companion columns, so it returns `#SPILL!` — captured faithfully, but noise.
* A function name may appear in several grid files; the blocks concatenate in
  filename order and land in one output file.

## Reading the corpus from Rust

Read **`number_r17`** (a string), not the JSON `number`, unless your parser is
known to be correctly rounding — `serde_json` is. `general_text` is Excel's
value→text coercion and is the field to assert display behaviour against;
`display_text` is column-width dependent and is informative only.

Check `observed_status` before `observed`: a case Excel's parser refused carries
`observed: null` with `observed_status: "rejected-by-excel"`. Check
`stored_formula` too — when present, Excel evaluated something other than
`formula`, and that rewrite is itself conformance data.

## Two traps this harness already fell into

Both cost real debugging time; both are load-bearing in the code.

1. **`Range.Value2` cannot be assigned a scalar Double, Boolean or String** from
   Windows PowerShell 5.1 — it throws "Specified cast is not valid" while
   happily accepting an `Int32`. A fixture written the obvious way silently
   loses every non-integer. `Set-OracleCellValue` wraps the value in a 1×1
   `object[,]` to take the array marshalling path instead.
2. **`Clear()` and `ClearContents()` return a Boolean.** Unvoided, that Boolean
   joins the calling function's output stream and corrupts its return value.

And one in the analysis rather than the capture: never write `$grids.Count` on a
dictionary whose keys come from the function catalogue. PowerShell resolves a
dictionary member against the keys first, and `COUNT` is a function name.
