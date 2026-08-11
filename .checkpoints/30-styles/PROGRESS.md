# PROGRESS.md — Ehkatra build log (current state)

History for sessions 1–19 lives in [`archive/PROGRESS-sessions-01-19.md`](archive/PROGRESS-sessions-01-19.md).
This file stays short on purpose: it is what a human returning after days away
reads first, so it holds the handoff, the current state, and the next action —
not the story of how they came about. That is what the archive and the
registers (docs/43 decisions, docs/44 debt, MEASUREMENTS.md numbers) are for.

---

## HANDOFF — read this first

**Where the tree is: Ehkatra is a spreadsheet you can use.** A window opens, you
click a cell, you type `=SUM(A1:A5)`, it computes; you copy a range, paste it
somewhere else and the formulas move with it; you drag the fill handle and the
series continues. **TD-60, TD-61 and TD-64 are paid** — the three rows that
stood between "a renderer with evidence" and "a product". Q1 is done, v0.1 is
tagged, W-ORACLE is 90.4%. **And it exports:** session 29 adds XLSX *write*
with a published round-trip fidelity number — 100.0% of the modelled surface
over the whole corpus, verified against real Excel (W-XLSX-WRITE).

**Session 29 in one paragraph.** XLSX **write** exists and the project has its
first write-fidelity number: **100.0%** — 49/49 modelled cells identical over
read → write → re-read across all 20 corpus files, and **20/20 written files
open in real Excel** (COM, 21/21 cell checks, no orphan EXCEL.EXE).
`usk_zip::write` emits STORED-only containers — **D-121**: zero new
dependencies, no second codec to be wrong about, 2.06× the deflated input,
filed as TD-72 — and `usk_xlsx::write` emits the reader's whole modelled
surface: values on the canonical decimal rendering, formulas with their cached
values, number formats at both indirection levels, shared strings (**D-122**).
Everything the format cannot carry is a **named loss**, never a silent drop
(**D-123**): NaN/±inf → `#NUM!`, `Decimal` → its exact digits with the *type*
recorded lost (TD-12's XLSX-round-trip trigger fired, and that is the answer
on record), `#CIRC!`/`#SPILL!` → `#N/A`. The defect of the session was found
by the **Excel oracle and by no test of ours**: a bare `#SPILL!` error cell —
literal or formula-cached — makes Excel refuse the *entire container*, while
our own reader round-tripped it at 100%; the prime suspects (`f64::MAX`, a
5e-324 subnormal) were innocent. Two halves of one codebase agreeing proves
only that they agree, which is why the fidelity number is published *with* the
Excel validation. TD-04 paid. Tests 494 → 504. Gates green, replay hashes
unmoved.

**Session 28 in one paragraph.** Paid **TD-71** — the tile store speaks
positions at range granularity (**D-120**). Profiled first, and for the first
time in four sessions **the register's named cause survived the measurement —
mostly**: experiment 4 rebuilt `TileStore::locate`'s three `BTreeMap` lookups
outside the store with identical content and probe pattern and priced them at
**~90–110 ns of a ~195–270 ns read** — the largest single term, but only just
over half, with `Presence::rank`'s popcount walk most of the rest. The fix
took both halves: `TileStore::read_rect` resolves a range's column slots once
per run, each row's slot once per row, each tile once per (row, column band),
and continues ranks incrementally while the in-tile index ascends;
`Grid::read_rect` (default body = the old per-cell loop, and normative) hands
the rectangle down from the evaluator, and `EngineGrid`'s override overlays
computed results at one binary search per column. Range reads went **~206 →
64–78 ns/cell (≈3×), flat at 500k rows**; the wide profile shape went **4.30 →
2.73–2.94 µs/formula** at 100k and **4.46–4.55 → 2.87** at 500k. W-CHAIN-100K
unchanged at 45–57 ms — correctly, since its reads are single-cell and mostly
hit computed results. Identity-based single-cell reads untouched; no defects
found. Tests 492 → 494. Gates green, replay hashes unmoved.

**Session 27 in one paragraph.** Fixed the thing TD-20's row was hiding
(**D-119**). `BandIndex::stab` narrowed a query to a row band and then scanned
**every** rectangle a candidate group owned, discarding the narrowing it had
just done — a **defect, not debt**, and one that was invisible until TD-66 made
a group's rectangle count large enough to see. A band now holds the rectangles
crossing it. The gapped corpus at 500,000 rows went **11.33 → 4.5–4.9 µs per
formula** and is now indistinguishable from the dense one; the 1M-row corpus's
full recalculation went **15.3 → 3.6–3.9 s**. Also closed a real coverage gap:
`BAND` is 256 rows and **every existing `usk-calc` test used a sheet that fits
in band 0**, so the multi-band arithmetic had never been exercised. Tests
489 → 492.

**Session 26 in one paragraph.** Paid **TD-23** — calc results are slot-indexed
by derived position instead of keyed by identity (**D-118**). W-CHAIN-100K's
full recalculation went **112–138 → 41–47 ms**, below the 53.0 ms the register
records as the figure its regression started from. The work was *profiled
first*, with `tools/recalc-profile`, because there is no profiler on this host
and TD-66 had just shown what guessing costs. **The measurement corrected the
guess**: session 25 said the cost was the result map growing with the sheet;
the map cost 1.2 µs per *formula* and nothing per *read*. Two further
bottlenecks came out of the same harness with numbers attached — `State::cell`
at 214–322 ns (**TD-71**) and, for gapped sheets, `stab` scanning a group's read
rectangles (**TD-20**, which now carries a measurement instead of a prediction).
Tests 489, unchanged — this was a rewrite under an existing suite.

**Session 25 in one paragraph.** Paid **TD-68** (a reference followed by `(` is
a call — the rule the parser's `Ident` arm always applied, now applied to
`CellRef` too) and the hard half of **TD-66**. The quadratic in the graph build
was `extent_of`: it accumulated a group's read rectangles one at a time and
linearly scanned everything accumulated so far looking for a merge, which is
O(n) when the column is unbroken and **O(n²) the moment it has gaps**. One sort
and one sweep. **1,000,000 rows: 218 s → 5.36 s.** The experiment that found it
varied exactly one thing — a `dense` corpus with a formula in every row has 50%
*more* formulas and builds six times faster, which said the formula count was
not the cost. Tests 483 → 489.

**And a flattering number that was noise, for the third time this quarter.** The
first A/B showed full recalc halving; re-running the old algorithm *warm* gave
the same 112–138 ms as the new one. `extent_of` is not on the recalculation
path, so a halving there needed an explanation, and the explanation was that it
had not happened.

**Session 24 in one paragraph.** Paid **TD-64** — copy, cut, paste and the fill
handle (**ADR-040**). Three things are worth carrying forward. The dependency
was *measured* before it was taken and the measurement changed the answer:
`arboard` is 28 crates with default features and 7 without, because `image-data`
drags in an entire image codec stack for a flavour a spreadsheet never writes —
267/280 would have spent the accesskit budget on a JPEG decoder and nothing
would have said so. Moving a formula's references went into the **kernel**
(`usk_formula::translate`), because copy/paste, fill-drag and the future
`fill_range` verb all ask the same question and each inventing an answer would
guarantee they disagree. And a test written for that module found a **latent
kernel defect**: the lexer reads `LOG10` as a cell reference, which is harmless
only because no function in the current set has a digit in its name (TD-68).
Tests 429 → 483.

**Session 23 in one paragraph.** Built the window and the editing surface as one
unit (**ADR-039**), because they are one unit: an edit is what makes recalc
incremental, and formula cells were blank because the shell never ran
`usk-calc`. The shape is three separations — an application model that owns no
window, a keymap that is a pure table, and the reducer as the only way anything
reaches `State`. That is not tidiness: it is why clicking a cell, typing a
formula, watching a precedent recalculate and keeping the cursor on its identity
across a row insert are all asserted **without a display**. Then paid TD-24's
residual with `State::apply_tip` (**D-117**), because with an editing surface the
old "re-fold the whole log on every read" became a cost per keystroke. Tests
363 → 429. Gates green. Replay hashes unmoved.

**The through-line, and it is the same one as last session.** *Look at the
output.* Four defects were found this session and **none of them by a test**:

* the formula column rendered `#REF!`, because the seeded corpus wrote
  `SetFormula` with an empty binding list — a formula carries the *identities*
  its references resolve to, and the A1 text is the display of that binding
  (DP-A6). The renderer was telling the truth about an unbound formula;
* once bound, the column was **still blank on a freshly opened workbook**.
  `Engine::build` builds the graph and evaluates nothing. Every editing test
  passed, because the first formula a test types is a structural change that
  forces a full recalc and fills the sheet in behind it — the tests only ever
  looked *after* an edit;
* the first W-PRESENT run reported p99 10.2 ms and read as a budget breach. It
  was the 120 Hz refresh interval: under `Fifo`, `get_current_texture` blocks,
  so timing the whole present measures the display and not the renderer;
* the shell had **no CI at all**, because it is a separate workspace (D-116) and
  `cargo fmt --all`, `cargo clippy --workspace` and `cargo test --workspace` all
  stop at the kernel's members.

The fifth was found by a test, and by the right one: three of the relay's
convergence tests went red the first time `apply_tip` existed. Slot interning
happens on row *insert*, not on first write; the incremental applier interned
only on write, so two rows inserted at the tip and written in the opposite order
**swapped slots** — and slot order is the order the state hash folds in. Worth
keeping: the first regression test written for it **did not reproduce it**, and
was rewritten until it failed with the fix removed. A test that passes both ways
is not a regression test.

---

## CURRENT STATE

- **Gates:** ALL GREEN via one command — `powershell -File tools\gates.ps1`.
  Shell compat · fmt · clippy `-D warnings` · tests · no_std wasm32 kernel build
  · dep budget · **shell fmt + clippy + tests** (new) · supply chain · differential
  replay native == wasm32 · purity/host-isolation greps.
- **Tests: 504** — 406 kernel + 98 shell. All passing.
- **Replay hashes:** oplog `c79fa533…` · state `b58d5505…` — **unchanged**, which
  is the evidence that `apply_tip` and the slot-intern fix are additive to the
  op algebra rather than a change to it.
- **Dependency budget:** kernel direct 1/5 · kernel closure 10/12 · workspace
  closure 29/40 · **shell closure 246/280** (the editing surface cost zero
  crates; the clipboard cost 7 — see W-DEPS-CLIPBOARD).
- **W-ORACLE:** 90.4% (1,235 / 1,366).
- **The numbers that are new** (MEASUREMENTS.md):
  · **W-PRESENT** — presented frame cost p50 **2.15 ms**, p99 **4.10 ms** against
    docs/31's 8.3 ms; 299 of 300 frames inside it. Cold launch **39.8 ms** / 1.0 s.
  · **W-KEYSTROKE** — keystroke→paint on the 10k sheet docs/31 names: **1.77 ms
    p50** against 16 ms. Was 7.05 ms before `apply_tip`; 60,000 cells went
    **25.31 → 2.74 ms**.
  · **W-OPEN-SHELL** — open by phase at 100,000 rows: skeleton+viewport (what
    the 1.5 s budget names) **377–524 ms**, graph build **269–434 ms**, full
    recalc **423–457 ms**. The graph build's quadratic is **fixed** — 1M rows
    went **218 s → 5.36 s**. Read single figures from this host as ±30%.
  · **W-RECALC-PROFILE (TD-71)** — range reads through the tile store now cost
    **64–78 ns/cell** against ~206 before (flat in sheet size); wide-range
    recalc **4.30 → 2.73–2.94 µs/formula** at 100k rows, **4.46–4.55 → 2.87**
    at 500k. W-CHAIN-100K unchanged at 45–57 ms, which is the correct shape:
    its reads are single-cell and mostly hit computed results.
  · **W-XLSX-WRITE (session 29)** — write fidelity **100.0%** (49/49 modelled
    cells, 20/20 corpus files) over read → write → re-read; **20/20 written
    files open in real Excel** (COM, 21/21 checks); corpus write 524–885 µs;
    output 2.06× input (stored entries, D-121/TD-72); 0 cell losses, 4 source
    parts dropped **by name** (vba quarantine + chart/drawing/theme).

---

## NEXT ACTION

**The editing surface works and is not finished. In order:**

1. **Native IME (docs/33).** The in-cell overlay is where TSF and
   NSTextInputClient attach — that is why docs/31 specifies an overlay at the
   caret — but the composition callbacks are not implemented, so a CJK user
   cannot type. Named in ADR-039 as absent rather than left to be found.
2. Then **accesskit tree v1** and the platform adapters (menus, dialogs, file
   association). **34** crates of shell headroom remain, earmarked for exactly
   these — the clipboard spent 7 of the 41 ADR-038 recorded, and accesskit is
   the item that will test what is left.
3. Then styles/validation/cond-format/sort/filter/tables. **XLSX write is off
   this list** — done in session 29 with its fidelity number published
   (W-XLSX-WRITE, 100.0% of the modelled surface, Excel-verified); what
   remains of it is register debt with triggers (TD-72 deflate, TD-73
   control characters, TD-74 unnamed unaccounted parts).

**TD-71 is paid** (session 28, D-120) and with it the read-path arc that began
with TD-23 is done: range reads resolve their slots once per run and cost
**64–78 ns/cell** where they cost ~206. What the profile says is largest *now*,
recorded so the next session does not have to rediscover it: within a range
read the remaining ~120 ns/cell is **evaluator-side** (operand materialisation,
the aggregate's own walk) — storage is no longer the bigger half. Single-cell
reads still pay `locate`'s three lookups by design; that is a new register row
if a single-cell-heavy workload ever approaches a budget, not a reopening. In
the shell, **TD-65** (frame cost grows with the document, unattributed) and
**TD-62** (no shaped-run cache) remain the filed performance rows, both below
their triggers.

**Four sessions of performance work, and the lesson held even when the register
was right:** TD-19 said parsing; it was `extent_of` (TD-66). TD-23 said the
result map grew with the sheet; the map cost per *formula* and nothing per
*read*. TD-20 said "not an R-tree"; it was the index discarding its own
narrowing. TD-71 said three tree lookups — and the profile *confirmed* it, at
just over half the read, which still changed the fix: the other half was
`Presence::rank`, so the rect read amortises ranks as well as lookups.
**Profile before building the fix** stands at four-for-four.

**Gated and should stay closed** (D-112): TD-17 and TD-44 have triggers that
measurement shows are not live; TD-37 is blocked on packaging rather than
effort. **TD-63** — confirm the bundled font's licence before the first
installer. **TD-62** (no shaped-run cache) and **TD-58** (the axis rebuilds
prefix sums in O(n)) are filed and below their triggers; **TD-65** may turn out
to be TD-58 wearing a different hat, which is why it says the cause is
unattributed instead of guessing.

**TD-46's container half is still open.** `Snapshot::build` still writes the
compacted op set, so TD-24's snapshot residual and TD-57 stand. D-103 states the
remaining fork with both options measured; the recommendation on record is (b),
reconstruction at snapshot time, streamed per tile band. Do not implement (a)
without the ADR — it retires a frozen ADR's central claim.
