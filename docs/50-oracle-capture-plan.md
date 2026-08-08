# 50 — Excel Conformance Oracle: Capture Plan and First Findings
Status: Active · Owner: Compatibility Engineer · Normative: no (the *corpus* is normative; this document explains it)
Implements: ADR-024, docs/32 §The oracle · Closes: A-007 · Resolves: TD-13

## Why this exists

docs/32 states the compatibility posture in one line: *the documentation lies and
the binary doesn't*. Everything the engine currently believes about Excel comes
from Microsoft's prose, and `crates/usk-formula/src/functions.rs` says so in its
own header — the vectors in `tests/formulas.rs` "encode documented behaviour and
are marked as such" because the capture harness did not exist (A-007).

It exists now. `tools/oracle-capture/` drives the locally installed Excel over
COM across an edge-case grid and writes down what it actually does.

The first capture did not merely confirm the design. It **refuted a recorded
decision** (TD-13's threshold), **found a rule no design document mentions** (a
third 15-digit truncation, at parse time), and **cleared a suspected divergence**
that would otherwise have been "fixed" into a real one (string offsets). That is
the return on building an oracle instead of reading a manual.

## What was captured

| Corpus | Vectors | Functions | Scope |
|---|---|---|---|
| `tools/oracle-capture/vectors/` | **1236** | 80 | 1900 date system, the full shipped catalogue plus the compat catalogue |
| `tools/oracle-capture/vectors-1904/` | **130** | 7 | 1904 date system, date functions and the serial boundaries |

Environment: Excel 16.0 build 20228 (Microsoft 365), calc engine version 191029,
Windows 11 26200, UI language 16393, `.` decimal / `,` list separator. Every
vector file carries this provenance, because a vector without its environment is
an anecdote.

Coverage: all 69 functions in `functions.rs::CATALOGUE`, plus 11 synthetic
`__compat_*` entries for the enumerated bug catalogue. 719 numeric, 195 error,
170 text, 145 logical results. 50 distinct edge-case tags (`coercion`,
`boundary`, `error-input`, `blank`, `wildcard`, `astral`, `1900-leap`,
`literal-parser`, `cancellation`, `binade`, `discriminator`, `mechanism`, …).

`TODAY` and `NOW` are volatile, so no fixed vector can exist for them. They are
captured as structural invariants instead (`=TODAY()=INT(NOW())`,
`=NOW()-INT(NOW())>=0`), which are testable without a clock.

## Findings

### 1. TD-13 is resolved: the cancellation threshold is 8 ULP, not 1e-15 relative

D-041 split Excel's "15-digit quirk" into `compat_round_15` (display) and
`compat_final_adjust` (evaluation). TD-13 records that the latter's `1e-15`
*relative* threshold is documentation-derived and unvalidated.

It is worse than unvalidated — as stated it is **wrong**, and the corpus
contains the pair that proves it:

```
=1+1E-15-1             residue 1.1102e-15  relative 1.11e-15  ->  0      (zeroed)
=1000000+1E-9-1000000  residue 1.0477e-09  relative 1.05e-15  ->  kept
```

Two cases with essentially the same *relative* residue, opposite outcomes. No
relative threshold at any value reproduces both.

Measured in **ULPs of the larger operand**, the same data is perfectly monotone
across eight decades of operand magnitude (1e-6 … 1e15), with no exceptions in
28 sweep points:

| residue in ULP of operand | outcome |
|---|---|
| 1, 2, 5, 6, 7 | **zeroed** |
| 8, 9, 14, 18, 23, 45, 90, 225, 450, 4504, 45036 | kept |

> **The rule.** Let `e = floor(log2(max(|op1|, |op2|)))` over the two operands of
> the final add/subtract. The result is set to zero iff
> `|result| < 8 × 2^(e-52)`, i.e. `|result| < 2^(e-49)`. Strict inequality —
> exactly 8 ULP is kept.

Equivalently: zero the result when the top 49 bits of significance cancelled.

**Confirmed against binades** (`grids/94-binade-sweep.psd1`), which is the probe
that settles it. A decade sweep cannot separate "8 ULP" from "≈1.7e-15 relative",
because at powers of ten the two nearly coincide. Inside a binade the ULP is
constant, so a fixed ULP count has a relative residue that halves from the bottom
of the binade to the top — and the two candidate rules therefore **cross over**:

```
operand 1.0, 7 ULP   relative residue 1.5543e-15   ->  ZEROED
operand 1.9, 8 ULP   relative residue 9.3492e-16   ->  kept     <- SMALLER, yet kept
```

A smaller relative residue kept while a larger one is zeroed. No relative
threshold at any value reproduces that ordering. The result holds at **both ends
of six binades from 2^-10 to 2^49** — 20 paired probes, 7 ULP zeroed and 8 ULP
kept every single time, with relative residues ranging over 1.55e-15 … 7.77e-16
and interleaving freely across the two outcomes.

The boundary is exactly determined, not merely bracketed: `fl(a+b) - a` is always
an integral multiple of `ulp(a)` when both are in the same binade, so 7 and 8 ULP
are adjacent reachable values.

**It is the *larger* operand's binade, not the first operand's.** Constructed
where the two differ: with `G1 = 2-2^-52` (the largest double below 2, so `G1` is
in binade [1,2) while `G1 + k·2^-52` is in [2,4) with twice the ULP):

```
=G1+8*2^-52-G1    ->  0                       residue is 4.5 ULP of the sum
=G1+16*2^-52-G1   ->  3.7747582837255322E-15  residue is 8.5 ULP of the sum
=G2+8*2^-51-G2    ->  0                       same shape one binade up
=G2+16*2^-51-G2   ->  7.5495165674510645E-15
```

Counted against `G1` these would be 8 and 16 ULP — both at or above the
threshold, so both should be kept. Counted against the larger operand they are
4.5 and 8.5 ULP, which predicts the observed zero-then-keep. The threshold scales
with `max(|op1|, |op2|)`.

**Corollary worth stating on its own: Excel cannot subtract two adjacent
doubles.** `=2-G1` and `=4-G2` both return `0`, though the operands are distinct
representable values exactly one ULP apart. Any engine that returns the true
`2^-52` here diverges.

### 2. `compat_final_adjust` is *positional*, and the design's name for it is right

The adjustment is applied to a **formula's final result**, not to the subtraction
operator. Wrapping the identical expression in parentheses suppresses it:

```
=0.1+0.2-0.3        ->  0                        (adjusted)
=(0.1+0.2-0.3)      ->  5.5511151231257827E-17   (not adjusted)
=1/(0.1+0.2-0.3)    ->  18014398509481984        ( = 2^54; the residue is intact)
=ABS(0.1+0.2-0.3)>0 ->  TRUE
=(0.1+0.2-0.3)=0    ->  FALSE
```

Excel's stored formula representation keeps an explicit parenthesis token, so
`=(0.1+0.2-0.3)` does not end in an add/subtract token and the rule does not
fire. Consequences an implementation must reproduce:

* The zero is **real once stored**. A cell holding `=0.1+0.2-0.3` returns
  `#DIV/0!` for `=1/A1` — the adjusted zero is what gets written to the cell.
* The residue **survives inside a larger expression**. The same arithmetic as a
  sub-expression keeps its full IEEE residue.
* `=A2+A3-A4` and `=SUM(A2:A3)-A4` over cell references are both adjusted, so
  this is about the top-level operator, not about literals.
* `=(0.1+0.2-0.3)+0` is *not* adjusted — the final operator is an addition, but
  the operands are already tiny, so nothing cancelled. Both conditions are
  required.

**Parentheses around the subtraction suppress it; parentheses around an operand
do not.** Isolated with a 7-ULP residue, which the bare form zeroes:

```
=H1-A1        ->  0                        (H1 = 1+7*2^-52, A1 = 1)
=(H1-A1)      ->  1.5543122344752192E-15   enclosing parentheses suppress it
=((H1-A1))    ->  1.5543122344752192E-15
=-(H1-A1)     -> -1.5543122344752192E-15
=(H1)-A1      ->  0                        parenthesised operand: still adjusted
=H1-(A1)      ->  0
=1/(H1-A1)    ->  643371375338642.25       the residue is intact
```

Also positional in the other direction — the same three values, differently
associated, give different answers:

```
=7*2^-52+A1-A1   ->  0                        final op is the cancelling subtraction
=A1-A1+7*2^-52   ->  1.5543122344752192E-15   final op is an addition with nothing to cancel
```

### 2b. `SUM` and `AVERAGE` adjust *unconditionally* — there are two mechanisms, not one

The most consequential structural finding of the binade sweep. The operator rule
is positional, but the aggregates' is not: their adjusted zero survives being
nested inside another expression.

```
=1/(A1+7*2^-52-A1)            ->  643371375338642.25   operator: residue intact when nested
=1/SUM(A1,7*2^-52,-A1)        ->  #DIV/0!              SUM: the zero survives nesting
=1/AVERAGE(A1,7*2^-52,-A1)    ->  #DIV/0!

=(A1+7*2^-52-A1)=0            ->  FALSE
=SUM(A1,7*2^-52,-A1)=0        ->  TRUE

=(A1+7*2^-52-A1)*1E17         ->  155.43122344752192
=SUM(A1,7*2^-52,-A1)*1E17     ->  0
```

Nesting the operator form in `MAX`, `ABS` or `0+(…)` likewise keeps the residue,
so the operator's suppression is not specific to division. `SUM` applies the same
8-ULP threshold as the operator (`SUM(A1,7*2^-52,-A1)` → 0,
`SUM(A1,8*2^-52,-A1)` → kept), and it applies at both ends of a binade.
Order matters within `SUM`: `=SUM(A1,-A1,8*2^-52)` cancels first and is kept.

> **An implementation needs both rules.** A positional one for the `+`/`-`
> operators, applied to a formula's top-level result; and an unconditional one
> inside the accumulating aggregates, applied to their own return value in any
> context. D-041 describes one rule. There are two, with different activation
> conditions, and reproducing only the first leaves `SUM` wrong in every nested
> position.

Not established: whether `SUMPRODUCT` adjusts. The probe used
(`=1/SUMPRODUCT(A1,7*2^-52,-A1)`) multiplies its arguments rather than summing
them, so no cancellation occurs and the case is uninformative. It needs a
`SUMPRODUCT` over a range that genuinely cancels.

### 3. A third 15-digit rule, in the parser, that no design document mentions

D-041 says the quirk is two rules. It is three. Excel truncates a numeric
**literal to 15 significant digits at parse time**, destructively, before the
evaluator sees it. The harness caught this only because it records what Excel
*stored* alongside what Excel *computed*:

```
=123456789012345678   stored as  =123456789012345000
=9999999999999999     stored as  =9999999999999900
=9007199254740992     stored as  =9007199254740990   (2^53, exactly representable, still truncated)
```

The parser's exponent range is also narrower than a double's:

* `=1E307` parses. `=1E308`, `=1E+308`, `=1.5E308`, `=-1E308` are **rejected
  outright** — the top decade of the double range is unreachable from formula
  text, though `Range.Value2` accepts `1.7976931348623157E+308` without
  complaint.
* `=1E-308` is accepted and then **silently stored as `=0`** — a normal double
  erased at parse time. `=1E-310` and below are rejected.
* Negative zero is normalised away: `=-0.0` is stored as `=0`, and `=0*-1` does
  not produce one either (`=1/(0*-1)` is `#DIV/0!`, not `-inf`).

Rejection is recorded as a result, not a gap: those cases carry
`observed_status: "rejected-by-excel"` with the parser's own error. Our parser
has to match this, so it is conformance data.

### 4. `compat_round_15` confirmed as display-only — and the `=` operator has its own tolerance

```
=SUM(0.1,0.2)   value 0.30000000000000004   displays "0.3"
=1/3            value 0.33333333333333331   displays "0.333333333333333"
=4.35*100       value 434.99999999999994    displays "435"
=1E15+1         value 1000000000000001      displays "1000000000000000"
```

The value keeps every bit; only the rendering rounds. D-041's split is correct
and the two rules are genuinely independent — `=1E15+1` *displays* as
`1000000000000000` while `=1E15+1-1E15` still evaluates to `1`.

Separately, the `=` operator carries its own tolerance:
`=0.1+0.2=0.3` is **TRUE**, while `=(0.1+0.2)-0.3=0` is **FALSE**. Comparison is
not subtraction-then-test-zero, and an engine that implements it that way will be
wrong on the single most common floating-point complaint users have.

### 5. String semantics are Unicode-scalar based — the engine is already right

The most consequential text question, because every `LEFT`/`RIGHT`/`MID`/`FIND`/
`SEARCH`/`REPLACE` offset depends on it. Common wisdom says Excel is UTF-16, which
would make `functions.rs`'s `chars().count()` wrong everywhere.

Three independent routes into Excel — a `UNICHAR()` formula, a cell seeded with
Unicode scalars, and a cell seeded with an explicit UTF-16 **surrogate pair** —
all agree:

```
=LEN(UNICHAR(128512))       -> 1
=LEN(A1)   (scalar-seeded)  -> 1
=LEN(B3)   (pair-seeded)    -> 1        Excel folds the pair into one unit
=EXACT(A1,B3)               -> TRUE
=LEFT(A2,1)                 -> the whole emoji, not half a surrogate
=FIND("x", emoji & "x")     -> 2        not 3
=UNICODE(A1)                -> 128512
```

**Excel counts Unicode scalar values, not UTF-16 code units.** `usk-formula`'s
char-based offsets are correct and need no change. Had this been "fixed" toward
the UTF-16 assumption, it would have introduced a divergence rather than removing
one. No normalisation is applied (`=EXACT(e-acute, e+combining-acute)` is FALSE).

### 6. Excel's 1900 calendar is internally inconsistent

The phantom leap day is reachable from every path — `DATE(1900,2,29)=60`,
`DAY(60)=29`, `TEXT(60,"yyyy-mm-dd")="1900-02-29"`,
`DATEVALUE("1900-02-29")=60`, `TEXT(DATE(1900,2,29),"dddd")="Wednesday"` — and
`DATE(1901,1,1)-DATE(1900,1,1)` is 366.

But `EOMONTH(DATE(1900,2,1),0)` returns **59**, i.e. 28 February. Excel
simultaneously holds that February 1900 has 29 days (serial 60 exists and is
named) and that it ends on the 28th. Both behaviours must be reproduced under
`Profile::Compat`; they cannot be derived from one another.

Also captured: `DATE` adds 1900 to any year below 1900, so `DATE(1899,12,31)` is
serial 693962 (year 3799) and `DATE(100,1,1)` is 2000-01-01. Month and day
arguments roll over rather than erroring.

### 7. Divergences from the current engine

Captured, not assumed. Each is a `Profile::Compat` bug with a vector attached.

| Vector | Excel | `functions.rs` today |
|---|---|---|
| `=ROUND(2.675,2)` | `2.68` | `f_round` does `floor(x*100+0.5)/100`; `2.675*100` is `267.49999999999997` → `2.67` |
| `=ROUND(1.005,2)` | `1.01` | same mechanism → `1.00` |
| `=FLOOR(2.5,0)` | `#DIV/0!` | `f_step` returns `0` for both FLOOR and CEILING |
| `=CEILING(2.5,0)` | `0` | matches |
| `=POWER(0,0)` | `#NUM!` | `powf` gives `1` |
| `=POWER(-8,1/3)` | `-1.9999999999999998` | `#NUM!` — Excel computes odd roots of negatives |
| `=LEN(TRIM(NBSP&"abc"&NBSP))` | `5` (NBSP survives) | Rust `str::trim()` strips U+00A0 → `3` |
| `=SEARCH("a*c","abxc")` | `1` | no wildcards → `#VALUE!` |
| `=SEARCH("~*","a*b")` | `2` | no tilde escape |
| `=VLOOKUP(35,A1:B5,2,TRUE)` | `"thirty"` | approximate match rejected → `#N/A` (a deliberate v0.1 choice; now measured) |
| `=MATCH(35,A1:A5,1)` | `3` | match type ≠ 0 rejected → `#N/A` |
| `=VLOOKUP("a*",E1:F5,2,FALSE)` | `1` | wildcards unsupported in keys |
| `=1<"1"` / `="1"<TRUE` | `TRUE` / `TRUE` | cross-type ordering by type tag — needs checking against `coerce::cmp` |
| `=COUNTBLANK(G4)` vs `=ISBLANK(G4)` | `1` vs `FALSE` | the `=""` cell is blank to one and not the other |

## How to run it

Excel must be installed and **closed** — the harness refuses to run against a
live instance (see Host isolation).

```powershell
cd tools\oracle-capture
powershell -ExecutionPolicy Bypass -File .\Capture-Oracle.ps1
powershell -ExecutionPolicy Bypass -File .\Verify-Vectors.ps1
```

`-Functions SUM,ROUND` narrows the run; `-Date1904` captures the alternate date
system into `vectors-1904/`; `-Force` proceeds against a running Excel;
`-Mode SpecDerived` emits the same file format from the grids' documented-
behaviour `expect` blocks with `oracle: false`, for a machine with no Excel — it
is a pipeline smoke test and must never gate conformance.

`Verify-Vectors.ps1` needs no Excel and belongs in CI: it checks the schema,
id uniqueness, kind/field agreement, and that every `number_r17` is canonical and
byte-identical to the JSON number.

### Host isolation (DP-S5)

Excel's COM server is registered multi-use, so `New-Object -ComObject
Excel.Application` can hand back the instance the user is working in. Setting
`DisplayAlerts = $false` on that instance and quitting it would silently discard
unsaved work. Therefore the harness:

* refuses to start when Excel is already running, unless `-Force`;
* never calls `Quit` on an instance it did not create;
* captures and restores `Visible`, `DisplayAlerts`, `ScreenUpdating`,
  `EnableEvents` and `Calculation`;
* sets `EnableEvents = $false` so no add-in or VBA hook observes the capture;
* writes only under `tools/oracle-capture/`, and never saves a workbook.

## Corpus format

One file per function, `vectors/<FUNCTION>.json`, schema
`ehkatra.oracle.vectors/1`. Synthetic catalogue entries are prefixed `__compat_`.

```json
{
  "schema": "ehkatra.oracle.vectors/1",
  "function": "SUM",
  "group": "math",
  "provenance": { "source": "excel-com", "oracle": true, "excel_version": "16.0",
                  "excel_build": "20228", "excel_calc_version": "191029",
                  "date_system": "1900", "captured_utc": "...", "...": "..." },
  "case_count": 20,
  "cases": [
    {
      "id": "SUM/0014",
      "formula": "=SUM(0.1,0.2)",
      "block": 1,
      "fixture": [ { "ref": "A1", "value": 1 }, { "ref": "B3", "text": "7" } ],
      "tags": ["precision"],
      "observed": {
        "kind": "number",
        "number": 0.30000000000000004,
        "number_r17": "0.30000000000000004",
        "general_text": "0.3",
        "display_text": "0.3"
      }
    }
  ]
}
```

Field notes, each of which exists because something would otherwise be lost:

* **`number_r17` is authoritative, `number` is a convenience.** Read the string
  unless your JSON parser is known to be correctly rounding. Windows
  PowerShell's `ConvertFrom-Json` routes non-integer numbers through
  `System.Decimal`, and .NET's Decimal→Double conversion is not correctly
  rounded — it moves values like `0.10000000000000009` by one ULP. `serde_json`
  is correct; `Verify-Vectors.ps1` compares raw file text for this reason.
* **`general_text`** is `x&""`, Excel's value→text coercion. This is where
  `compat_round_15` lives, and it is the field a display-conformance test should
  assert against. **`display_text`** is `Range.Text` at the recorded
  `probe_column_width` (80) and is width-dependent, so it is informative rather
  than normative.
* **`kind`** is a closed set: `number` · `text` · `logical` · `blank` · `error`.
  Errors carry both the name (`#DIV/0!`), the `ERROR.TYPE` code, and the raw
  `CVErr` integer — two independent sources, because one trusted source is how
  you get a corpus that is confidently wrong.
* **`stored_formula`** appears when Excel stored something other than what was
  written. 78 of 1126 cases. This is how finding 3 was made; do not drop it.
* **`observed_status`** explains an absent observation — `rejected-by-excel`
  (with `reject_reason`) or `spec-derived`. `observed: null` with no status is a
  harness bug and `Verify-Vectors.ps1` fails on it.
* **`fixture`** travels with every case, so a vector is reproducible without the
  grid file. The corpus must stand alone.
* A probe returning a dynamic array cannot spill, because the companion columns
  occupy the cells beside it — Excel reports `#SPILL!` instead of silently
  overwriting them. Spills are therefore visible, not corrupting.

## Grids

Input grids are `tools/oracle-capture/grids/*.psd1` — PowerShell data files, so
`'=UPPER("abc")'` stays readable where JSON would demand
`"=UPPER(\"abc\")"`, and `Import-PowerShellDataFile` parses literals only and
executes nothing. Grid files are kept **pure ASCII**; non-ASCII inputs are built
with `UNICHAR()` or the `codepoints` fixture field, so no encoding guess can
corrupt the input half of a vector.

| File | Contents |
|---|---|
| `10-math.psd1` | aggregation, arithmetic, the skip-vs-coerce split |
| `20-rounding.psd1` | ROUND family, CEILING/FLOOR asymmetry |
| `30-logic.psd1` | logicals, type predicates, the blank/`=""` boundary |
| `40-text.psd1` | text functions, Unicode, wildcards |
| `50-lookup.psd1` | lookups, the criteria sub-language |
| `60-dates.psd1` | dates, serial boundaries, volatiles |
| `90-compat-catalog.psd1` | display rounding, cancellation, 1900 leap, coercion, comparison, precision limits |
| `91-literal-parser.psd1` | finding 3 |
| `92-cancellation-sweep.psd1` | findings 1 and 2 — decade sweep, positional semantics |
| `93-unicode-length.psd1` | finding 5 |
| `94-binade-sweep.psd1` | findings 1, 2 and 2b confirmed against binades: the paired discriminator, binade crossing, direct subtraction, aggregates |

A grid case may carry an `expect` block of *documented* behaviour. It is never
authoritative — the harness compares it against the oracle and reports
divergences in `_index.json` under `spec_divergences`. Those divergences are the
point: each is a place the engine would have been wrong had it trusted the docs.

## Next actions

1. **Amend D-041 and close TD-13** in `docs/43-decision-register.md`. Three
   changes, all with vectors attached: the 8-ULP threshold against
   `max(|op1|,|op2|)` (finding 1), the positional activation for `+`/`-`
   (finding 2), and the **second, unconditional mechanism inside `SUM`/`AVERAGE`**
   (finding 2b) — D-041 currently describes one rule where there are two. Also
   record the parse-time literal truncation of finding 3 as a new rule
   (`compat_parse_15`?); it is unrepresented in the engine.
2. **Fix the divergences in §7** and gate them on these vectors.
3. **Wire the corpus into the test suite**: a Rust harness that reads
   `vectors/*.json`, evaluates each case under `Profile::Compat`, and reports a
   conformance percentage. docs/32 makes that percentage a published product
   number, so it needs to exist before the number can be claimed.
4. **Capture more Excel versions.** One build is one data point; docs/32 asks for
   a corpus across versions. The provenance block is already shaped for it.
5. **Extend the catalogue** as `functions.rs` grows — the grids are additive, and
   `CATALOGUE` is the checklist.
6. **Close the two loose ends the binade sweep left**: whether `SUMPRODUCT` (and
   `SUBTOTAL`, `AGGREGATE`) carry the unconditional adjustment, using a range that
   genuinely cancels; and whether the threshold shifts for operands of opposite
   sign whose magnitudes differ by more than one binade.

## Limits of this corpus, stated plainly

* **One Excel build.** 16.0.20228. Version-specific behaviour is invisible here.
* **One locale.** `.` decimal, `,` list separator, English function names.
  Localised formula text and separator handling are untested.
* **Values, not formats.** Number-format code grammar, styles and rendering are
  out of scope except where `TEXT()` makes them a value.
* **Scalars, not arrays.** Dynamic-array spilling, implicit intersection and
  array-formula semantics are only touched incidentally.
* **No workbook I/O.** XLSX round-trip fidelity (docs/24) is a separate corpus.
* **Formula-parse errors are sampled, not swept.** Argument-count and syntax
  rejection is captured only where it arose naturally.
