# PROGRESS-ORACLE.md

Live state for the **Excel conformance oracle** work stream (ADR-024, docs/32
§The oracle, assumption A-007). This file is owned by the parallel oracle
session and is deliberately separate from `PROGRESS.md`, which the main build
session owns.

Session 1 — 2026-08-08. Status: **green and checkpointed.** Nothing in progress,
nothing half-refactored.

Session 1b — 2026-08-08, same session, continued: cancellation sweep widened
across binade boundaries. The 8-ULP rule is now confirmed rather than inferred,
and a **second adjustment mechanism** was found inside `SUM`/`AVERAGE`. Corpus
1126 → **1236 vectors**. Still green.

---

## What exists now

`A-007 is closed.` The COM capture harness of ADR-024 is built, working, and has
produced its first corpus against real Excel.

| Artifact | State |
|---|---|
| `tools/oracle-capture/Capture-Oracle.ps1` | driver — grids → Excel → JSON. Working. |
| `tools/oracle-capture/Verify-Vectors.ps1` | offline corpus validator, no Excel needed. CI-ready. Clean. |
| `tools/oracle-capture/lib/*.ps1` | COM session + host isolation, block evaluation, grid loading, deterministic JSON writer |
| `tools/oracle-capture/grids/*.psd1` | 11 grid files, 1236 authored cases |
| `tools/oracle-capture/vectors/` | **1236 vectors, 80 functions**, 1900 date system, ~2.1 MB |
| `tools/oracle-capture/vectors-1904/` | **130 vectors, 7 functions**, 1904 date system |
| `docs/50-oracle-capture-plan.md` | the plan, the findings, the corpus format, the limits |
| `tools/oracle-capture/README.md` | how to run it and how to add cases |

Proof it works, reproducible in two commands:

```powershell
cd tools\oracle-capture
powershell -ExecutionPolicy Bypass -File .\Capture-Oracle.ps1
powershell -ExecutionPolicy Bypass -File .\Verify-Vectors.ps1
```

Last run: 1236 vectors across 80 functions; validator reports **corpus is
well-formed**; 0 block failures. Captured against Excel 16.0 build 20228
(Microsoft 365), calc engine 191029, Windows 11 26200.

Coverage: all 69 functions of `crates/usk-formula/src/functions.rs::CATALOGUE`,
plus 11 synthetic `__compat_*` entries for the enumerated bug catalogue of
docs/32. 719 numeric / 195 error / 170 text / 145 logical results, 50 edge-case
tags. Target for this session was 500+; delivered 1366 across both date systems.

Excel **was** installed and COM **did** work, so no fallback was needed. The
`-Mode SpecDerived` path is implemented anyway, for a machine without Excel; it
marks its output `oracle: false` and must never gate conformance.

---

## Findings that change the design

Full detail with the supporting vectors is in `docs/50-oracle-capture-plan.md`.
Summary, worst-first:

1. **TD-13 resolved, and D-041 was wrong as recorded.** `compat_final_adjust`'s
   `1e-15` *relative* threshold cannot reproduce Excel: `=1+1E-15-1` (relative
   residue 1.11e-15) is zeroed while `=1000000+1E-9-1000000` (relative residue
   1.05e-15) is not. Measured in **ULPs of the larger operand** the data is
   perfectly monotone — zeroed at ≤7 ULP, kept at ≥8. The rule is
   `|result| < 2^(e-49)` where `e = floor(log2(max(|op1|,|op2|)))` over the
   operands of the final add/subtract; strict, so exactly 8 ULP is kept.

   **Confirmed against binades, not just decades** (`grids/94-binade-sweep.psd1`).
   Inside a binade the ULP is constant, so a fixed ULP count has a relative
   residue that halves from the bottom to the top — which makes the two candidate
   rules cross over: operand 1.9 at 8 ULP (relative 9.35e-16) is **kept** while
   operand 1.0 at 7 ULP (relative 1.554e-15) is **zeroed**. A smaller relative
   residue kept, a larger one zeroed; no relative threshold reproduces that. Holds
   at both ends of six binades from 2^-10 to 2^49, 20 paired probes, no
   exceptions. The binade-crossing block (`G1 = 2-2^-52`) further pins it to the
   **larger** operand's binade. Corollary: `=2-G1` is `0`, so Excel cannot
   subtract two adjacent doubles.
2. **`compat_final_adjust` is positional.** It applies to a formula's *final*
   result, not to the subtraction operator. `=0.1+0.2-0.3` gives 0 but
   `=(0.1+0.2-0.3)` gives 5.55e-17 and `=1/(0.1+0.2-0.3)` gives 2^54. The zero is
   real once stored in a cell (`=1/A1` → `#DIV/0!`) but the residue survives
   inside a larger expression. Parentheses around the *subtraction* suppress it;
   parentheses around an *operand* do not (`=(H1)-A1` is still zeroed). Same three
   values differently associated differ: `=7*2^-52+A1-A1` → 0 but
   `=A1-A1+7*2^-52` → 1.554e-15.
2b. **There are two mechanisms, not one — `SUM` and `AVERAGE` adjust
   unconditionally.** Their zero survives nesting, unlike the operator's:
   `=1/SUM(A1,7*2^-52,-A1)` is `#DIV/0!` and `=SUM(A1,7*2^-52,-A1)*1E17` is `0`,
   while `=1/(A1+7*2^-52-A1)` keeps the residue and
   `=(A1+7*2^-52-A1)*1E17` is `155.43`. Same 8-ULP threshold, different activation
   condition. **An engine must implement both**; reproducing only the positional
   rule leaves `SUM` wrong in every nested position. This is the finding most
   likely to be missed, because it is invisible unless you nest the aggregate.
3. **A third 15-digit rule, in the parser.** Excel truncates numeric *literals*
   to 15 significant digits at parse time, destructively:
   `=123456789012345678` is stored as `=123456789012345000`. It also rejects
   literals at or beyond `1E308`, silently stores `1E-308` as `0`, and normalises
   negative zero away. Nothing in docs/12, docs/32 or docs/43 mentions this, and
   the engine does not implement it.
4. **String offsets are Unicode-scalar based, so the engine is already right.**
   Three independent routes agree that `LEN` of an astral character is 1, not 2,
   and that `LEFT(emoji&"x",1)` returns the whole emoji. `functions.rs`'s
   `chars().count()` needs no change — and "fixing" it toward the usual UTF-16
   assumption would have *created* a divergence.
5. **Excel's 1900 calendar contradicts itself.** `DATE(1900,2,29)=60` and
   `DAY(60)=29`, yet `EOMONTH(DATE(1900,2,1),0)=59` (28 February). Both must be
   reproduced under `Profile::Compat`; neither follows from the other.
6. **`=` has its own tolerance.** `=0.1+0.2=0.3` is TRUE while
   `=(0.1+0.2)-0.3=0` is FALSE. Comparison is not subtract-then-test-zero.
7. **14 measured divergences** between real Excel and `functions.rs` today,
   tabulated in docs/50 §7 — including `ROUND(2.675,2)`, `FLOOR(x,0)`,
   `POWER(0,0)`, `POWER(-8,1/3)`, TRIM vs non-breaking space, SEARCH wildcards,
   and the deliberate v0.1 approximate-match refusals (now measured rather than
   assumed).

---

## Next action, in order

The first item is the one to start with; it is a docs edit and needs no code.

1. **Amend `docs/43-decision-register.md`**: update D-041 with the 8-ULP rule
   against `max(|op1|,|op2|)`, the positional activation, and **the second
   unconditional mechanism inside `SUM`/`AVERAGE`** (findings 1, 2, 2b); close
   TD-13; and record the parse-time literal truncation (finding 3) as a new rule —
   it has no representation in the engine at all. *Not done by this session:
   `docs/43` is outside the parallel session's write scope.*
2. **Wire the corpus into `cargo test`.** A Rust harness that reads
   `tools/oracle-capture/vectors/*.json`, evaluates each case under
   `Profile::Compat`, and reports a conformance percentage. docs/32 makes that
   percentage a published product number, so it must exist before the number can
   be claimed. Read `number_r17` (the string), not the JSON `number`.
3. **Fix the divergences** in docs/50 §7, gated on these vectors.
4. **Capture more Excel builds.** One build is one data point; docs/32 wants a
   corpus across versions. The provenance block is already shaped for it.
5. **Two loose ends from the binade sweep**, both small: whether `SUMPRODUCT`,
   `SUBTOTAL` and `AGGREGATE` carry the unconditional adjustment (the probe used
   multiplies rather than sums, so it proved nothing — needs a range that
   genuinely cancels); and whether the threshold shifts for opposite-sign operands
   more than one binade apart.

---

## Notes for whoever picks this up

* **Host isolation is load-bearing, not decoration.** Excel's COM server is
  multi-use, so `New-Object -ComObject Excel.Application` can return the
  instance the user is working in — and `DisplayAlerts = $false` plus `Quit` on
  that instance discards their unsaved work. The harness refuses to run against a
  live Excel unless `-Force`, never quits an instance it did not start, and
  restores every application setting it touches. Do not relax this (DP-S5).
* **Two PowerShell traps are already worked around; keep the workarounds.**
  `Range.Value2` rejects a scalar Double/Boolean/String from PS 5.1 while
  accepting an `Int32` — so fixtures are written through a 1×1 `object[,]`.
  `Clear()`/`ClearContents()` return a Boolean that silently corrupts a
  function's return value unless voided. Both are commented at the call site.
* **`number_r17` exists because JSON number parsing is not trustworthy.**
  Windows PowerShell's `ConvertFrom-Json` routes non-integer numbers through
  `System.Decimal`, and .NET's Decimal→Double conversion is not correctly
  rounded — it shifts values like `0.10000000000000009` by one ULP. That
  produced six false failures in the validator's first version; it now compares
  raw file text. `serde_json` is correct, so Rust may read either field, but
  prefer the string.
* **`stored_formula` is where finding 3 came from.** The harness records what
  Excel *stored* as well as what it computed. Do not drop that field to save
  space; 78 of 1126 cases carry it and it is the only window onto the parser.
* **Grid files stay pure ASCII.** A `.psd1` has no BOM and PowerShell guesses a
  codepage; non-ASCII inputs are built with `UNICHAR()` or the `codepoints`
  fixture field so the input half of a vector cannot be corrupted.
* **The harness never asserts what Excel should do.** It asks and writes down the
  answer. A grid `expect` block is a claim about *documented* behaviour, and the
  harness reports where that claim and the oracle disagree — those divergences
  are the deliverable, not a defect.

## Scope discipline observed this session

Parallel-session constraints were respected in full: writes confined to
`tools/oracle-capture/`, `docs/50-oracle-capture-plan.md` and this file.
`crates/` was read only. No `cargo` invocation, no git command, no file touched
outside `C:\Users\velag\Desktop\Ehkatra`.

Consequently two things that *should* follow from findings 1–3 are **not** done
and are left as next actions above: `docs/43-decision-register.md` is not amended
and `MEASUREMENTS.md` records none of these numbers.
