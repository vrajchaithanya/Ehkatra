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

**Session 32 in one paragraph.** **A CJK user can now *read* what they typed.**
Session 31 delivered the composition and drew it as three identical boxes; this
session paid **TD-79** with a font fallback chain (**D-125**), which is TD-79's
option (b) — the one PROGRESS.md recommended and the one that needs no ADR,
because it *narrows* docs/31's determinism promise and writes the narrowing down
rather than retiring it. `demo/editing-ime.png` now shows `にほん` mid-composition,
underlined, in **Yu Gothic**, where session 31's frame could only honestly show
Latin.

**The dependency was measured before it was chosen, and the measurement was the
session's first act.** `fontdb` and `font-kit` were each added with
`default-features = false` and the shell closure read against the 246/280
baseline: **`fontdb` + `fs` is 3 crates (249/280), `fontdb` with defaults is 5,
`font-kit` is 19** (W-DEPS-FALLBACK). The three are `fontdb`, `slotmap` and
`tinyvec` — `ttf-parser`, `log` and `libm` were already in the closure via
`rustybuzz`, which is why the honest number is this small. `memmap` is off
deliberately: it maps font files another process can rewrite underneath us and
saves nothing on a database consulted a handful of times per process. **31 crates
of headroom remain for accesskit**, which is now the constraint to watch.

**What the promise says now, stated precisely, because a weakened invariant
nobody wrote down is how invariants die.** Every codepoint the bundled face
covers is still laid out from bundled metrics — that is all Latin, which is
every benchmark, every corpus file and every demo frame in this repo. The line
box is bundled *unconditionally*: `ascent` and `line_height` come from slot 0
whatever a cell contains, so a row's height never depends on the scripts in it.
The **order of preference is bundled** — `text::PREFERRED` is a constant in the
binary, consulted before anything is enumerated, so the host's enumeration order
is never the tie-breaker and two machines with the same families installed
choose the same face. And the resolution is **recorded**: `Run::faces` names the
slots, `TextEngine::face_name` names the faces, `--script` prints them. What is
no longer true — a run that leaves the bundled face is not metric-deterministic
across hosts — is filed as **TD-81** rather than left to be discovered.

**Three of the four new tests are host-independent, and that was the design
problem.** The obvious test ("`にほん` renders as three different glyphs") passes
here and would be a lie on a container with no CJK fonts. So the mechanism is
proven with a *second bundled* face — `DejaVu Sans Mono`, which the same crate
already ships: `two_faces_do_not_share_an_atlas_entry_for_the_same_glyph_id`
asks both faces for glyph 50 and asserts they get different atlas rects, which
is precisely what the **old** `(glyph, device_px)` key would have failed. That
key change is the part of this work most likely to have been wrong silently —
one face serving another's pictures errors nowhere and looks like a font bug.
The kana test remains, with both branches asserted: three distinct glyphs on a
host that has a CJK face, and an explicit *reported* shortfall (`Run::unresolved`)
on one that does not. Neither branch is a skip.

**The number that did not move, and the one that did.** W-KEYSTROKE was
re-measured because fallback puts a coverage lookup on every character of every
shaped run: **1.76–2.30 ms p50 typing, 1.79–3.12 ms p50 composing**, against 16
ms and against session 31's 1.77 / 1.64–2.03 — unmoved inside this host's stated
±30%, and free for a structural reason rather than a lucky one (the bundled
face answers from its own `cmap` before the fallback map is consulted at all,
which `latin_is_shaped_entirely_from_the_bundled_face_and_says_so` asserts by
asserting **no system font was ever enumerated**). The one that did move is new:
**the first codepoint the bundled face cannot draw costs 238–412 ms**, which is
15–25× the 16 ms keystroke budget, once per process, 22–49 µs thereafter.
**Profiled rather than guessed, and the profile decided the fix**: the
enumeration is 203–321 ms of it — `fontdb` reads and parses all 379 face files
on this host to learn their names — and the pick is the small remainder. So the
work item is *move the enumeration off the frame*, not *search more cleverly*.
That is **TD-80**, and it is deliberately not in this session: warming needs a
background thread and a decision about where it starts, and stacking that
unverified on the fallback layer this session just proved is what DP-C4 forbids.
Tests 549 → 552 (kernel 438 unchanged, shell 111 → 114). Gates green, replay
hashes unmoved — a shell-only change touches no encoding.

**Session 31 in one paragraph.** **A CJK user can now type into a cell** — the
first item on the last session's NEXT ACTION list, and the one ADR-039 named as
*absent rather than left to be found*. Native composition, end to end
(**D-124**): `set_ime_allowed` at window creation, `WindowEvent::Ime` forwarded
verbatim, and `set_ime_cursor_area` fed the caret rectangle so the candidate
list appears under the composing text instead of in the window's corner. docs/33
says composition is *"never reimplemented"* and this session takes that
literally — winit's `Ime` events **are** TSF on Windows and NSTextInputClient on
macOS, so the shell writes no composition logic, no candidate list and no
conversion state, and the whole feature cost **zero crates** (closure 246/280,
unchanged). What the shell *does* own is three rules, and each of them exists to
prevent a specific wrong outcome. **A preedit lives on `Editor::preedit`, never
in `Editor::text`** — the cell is written from `text` alone, so an interrupted
composition cannot put unconfirmed characters into a document; `Editor::display`
splices the two for drawing only, and both the cell overlay and the formula bar
read that one spliced string so they cannot disagree about what is being typed.
**A composition opens the editor**, because the keystrokes that start one are
swallowed by the input method and no key event ever arrives — an implementation
that only updated an already-open editor would drop a Japanese user's first word
and look, from outside, exactly like a dead keyboard. **While composing, every
intent but `Cancel` is refused**: any key that reaches a composing editor either
leaked past the IME or was declined by it, and a `Backspace` there would delete
a *committed* character the user cannot see the caret in front of, while on a
backend that keeps delivering `KeyboardInput` during composition an `Insert`
would type every keystroke twice. Escape peels one layer at a time — first the
composition, then the edit.

**The defect this session found is in the font, not in the code, and the IME
work is what exposed it.** The plumbing is right and the pixels are wrong:
`DejaVu Sans` carries no CJK block, so `にほん` shapes to three `.notdef` boxes.
Advances and cluster offsets are correct — the caret lands in the right place —
but the user cannot read what they typed. Filed as **TD-79** with the proof
attached rather than the assertion: the test shapes the three kana and asserts
all three sample the **same atlas rect**, since three distinct characters
rendering identically is `.notdef` three times, with three Latin letters as the
control that the check is capable of failing. It is expected to fail the day
font fallback lands, and that is the signal to close TD-79. The honest
consequence: docs/48's *"IME validated by native JP/CN/KR typists"* stays
**unchecked**, and TD-79 blocks it — validating an IME whose output is boxes
would test nothing. `demo/editing-ime.png` therefore composes `nihao`, which is
what a pinyin IME shows before committing 你好 and what this font can draw.

**Numbers.** A composition update is a keystroke and carries docs/31's same
16 ms budget, so W-KEYSTROKE gained a third series: **1.64–2.03 ms p50,
1.69–3.19 ms p95** over four runs on M1, against 16 ms — indistinguishable from
typing a character (1.44–1.71 ms p50), which is the shape the code predicts and
therefore the one worth checking, since a preedit does the same `String` work
and the same repaint with no reducer, no fold and no recalculation. Had it come
out *slower*, the splice in `Editor::display` would have been the suspect. Tests
**98 → 111** shell (+13), kernel 438 unchanged, total **549**. Replay hashes
**unmoved** — nothing in this session touches the kernel, the op algebra or any
encoding, which is exactly what a shell-only change should look like.

**Session 30 in one paragraph.** **Cell styles**, and the first new op type
since the taxonomy was sealed — which made the *design* the load-bearing work
and the code its consequence (**ADR-041**, written before a line was
implemented). Four decisions, each permanent under DP-A5. **The addressing unit
is an identity rectangle**, not a cell: `StyleTarget { rows, cols }` where a
span is `All` or `Between(id, id)`, so a whole-column format is **one op and
one rule**, and it survives structural edits by identity rather than by
position — a row inserted above leaves it alone, a row inserted *inside* joins
it, a deleted endpoint re-anchors inward, all inherited from the rule
`usk_calc::refs::Axis::resolve` already implements. `AxisSpan::All` is not
sugar: a column format applies to rows that **do not exist yet**, which no pair
of endpoints can name. **The register is `(cell, facet)`**, not `(cell)`: one
facet per op, resolved per slot independently, so two actors turning one cell
bold and yellow at the same lamport **both win** — the case a single style blob
per cell loses silently, which is the one outcome ADR-006 forbids. **The facet
vocabulary is extensible without a new op type** — `tag ‖ u32 len ‖ body`, and
an unknown tag becomes `StyleFacet::Unknown`, the `Payload::Opaque` pattern one
level down: byte-exact, hashed as the author hashed it, applied to nothing.
Without it, every future facet would cost a permanent op tag. **State holds
rules plus an interner**, never a styled cell. Undo is exact rather than
approximate, which only the rectangle model allows: the prior resolution over a
target is itself a set of rectangles, so undo clears the target and replays the
overlapping prior rules clipped to it, in original stamp order — and blocks
whole when a *later* foreign rule intersects (`ApplyReport::blocked`).

**Two defects, and neither was found by a test.** The first by the workload:
resolution measured **1,826–3,361 ns per cell per facet**, i.e. 2.4 ms for one
facet over a viewport against docs/31's **8.3 ms whole-frame** budget, with
four facets to go. The cause was not the linear scan the ADR knowingly traded
memory for — `covers` re-walked the axis `BTreeMap`s *inside* the per-rule
loop, paying two tree lookups **per rule** instead of two in total. Hoisted:
**112–195 ns/cell**, 15–26×, and flat in sheet size. That is TD-71's defect in
a different organ, found the same way, and **profile-before-fix is now
five-for-five**. The second by asking what the fidelity number actually covered:
the widened round-trip comparison scored **49/49, 100.0%** on the existing 20
corpus files — and would have whatever the style code did, because **not one of
those files carries a font, a fill or an alignment**. A percentage over a
surface the corpus does not exercise measures the writer against itself. The
corpus gained a 21st file, hand-written in Excel's shapes and deliberately
awkward in the four places a naive styles reader goes wrong (`<color>` lives in
both `<font>` and `<patternFill>`; `<xf>` lives in both `<cellStyleXfs>` and
`<cellXfs>`; fill 0/1 are the mandatory skeleton and are not formatting;
`<b val="0"/>` means *not* bold). Published number: **100.0%, 57/57 modelled
cells across 21 files.** A third trap was caught by reading rather than by
running: the replay generator minted opaque tags from `0x19`, which are now
*ours* — left alone it would have emitted its fallback for two values in
sixteen and covered DP-A5 preservation one-eighth less, while still printing a
hash. Base moved to `0x2B`.

**The replay hashes moved, and that is the correct outcome.** docs/29: a new op
type must join replay-check's generator or the gate silently stops covering it.
oplog `c79fa533…` → **`a1b35c1a…`**, state `b58d5505…` → **`b95f1632…`**, native
== wasm32 still holding. No existing op encoding moved — `0x10`–`0x18` are
byte-for-byte what they were. The image gained a styles section
(`IMAGE_VERSION` 3), because styles are hashed state and an image that dropped
them would rebuild a document `Snapshot::verify` refuses. Memory, measured
(**W-STYLE-COLUMN**): 64 column rules over a 262,144-row sheet cost **14,576 B
flat**, 0.00087 B per addressed cell, **0 tiles**, and cell-store bytes
*identical* to the same sheet unstyled — against a per-cell store's ~1.5 GB
floor. Tests **406 → 438** kernel (+32). Borders, theme colours and named
styles are **not** modelled and are filed as TD-75/76/77 with triggers; the
resolution scan is TD-78. Gates green.

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
- **Tests: 552** — 438 kernel + 114 shell. All passing.
- **Replay hashes:** oplog `a1b35c1ac5afa7b5…` · state `b95f16327e2e9e88…` —
  **unmoved in sessions 31 and 32** (a shell-only change touches no encoding), and
  **MOVED in session 30, legitimately and for the second time in the project's
  life.** ADR-041 added the first new op types since the taxonomy was sealed,
  and docs/29 requires a new op type to join replay-check's generator or the
  determinism gate silently stops covering it. The corpus is a different corpus,
  so the hashes differ by construction — exactly as they did in session 12 for
  `Payload::Opaque`. **No existing op encoding moved**: tags `0x10`–`0x18` are
  byte-for-byte what they were, `0x19`/`0x1A` are new, and native == wasm32
  still holds, which is the property the gate actually asserts. The previous
  reference was `c79fa533…` / `b58d5505…`; anything still quoting those is
  pre-session-30. See MEASUREMENTS.md §W-REPLAY-5K (session 30).
- **Dependency budget:** kernel direct 1/5 · kernel closure 10/12 · workspace
  closure 29/40 · **shell closure 249/280** (the editing surface cost zero
  crates; the clipboard cost 7 — see W-DEPS-CLIPBOARD; IME cost zero, winit
  already carried the platform half; **font fallback cost 3** — `fontdb` with
  `default-features = false, features = ["fs"]`, chosen over `font-kit`'s 19,
  see W-DEPS-FALLBACK). **31 crates of headroom, and accesskit is the only
  remaining claim on them** — but ADR-037 earmarked ~50 for accesskit *plus* the
  file-dialog and menu adapters, so that trio no longer fits at its estimate and
  should be budgeted together rather than first-come.
- **W-ORACLE:** 90.4% (1,235 / 1,366).
- **The numbers that are new** (MEASUREMENTS.md):
  · **W-FALLBACK (session 32, TD-79/TD-80)** — the first codepoint the bundled
    face cannot draw costs **238–412 ms**, of which the system-font enumeration
    is **203–321 ms** (379 faces on M1); every one after it costs **22–49 µs**,
    and a Latin run costs 230–575 µs and enumerates nothing. Resolved face on
    M1: **Yu Gothic**. The first figure is 15–25× docs/31's 16 ms keystroke
    budget, once per process — **TD-80**, profiled before it was filed.
  · **W-DEPS-FALLBACK (session 32)** — `fontdb` + `fs` **+3 crates** (249/280),
    `fontdb` default +5, `font-kit` **+19**. Measured before choosing, and the
    measurement is why the answer is `fontdb`.
  · **W-PRESENT** — presented frame cost p50 **2.15 ms**, p99 **4.10 ms** against
    docs/31's 8.3 ms; 299 of 300 frames inside it. Cold launch **39.8 ms** / 1.0 s.
  · **W-KEYSTROKE** — keystroke→paint on the 10k sheet docs/31 names: **1.77 ms
    p50** against 16 ms. Was 7.05 ms before `apply_tip`; 60,000 cells went
    **25.31 → 2.74 ms**. **New in session 31:** the IME composition path on the
    same workload, **1.64–2.03 ms p50 / 1.69–3.19 ms p95** against the same
    16 ms — level with typing a character, which is correct: a preedit is a
    `String` and a repaint, with no reducer and no recalc behind it.
    **Re-measured in session 32** after fallback put a coverage lookup on every
    character: **1.76–2.30 ms p50 typing, 1.79–3.12 ms p50 composing** — unmoved
    inside this host's ±30%, because the bundled face answers from its own
    `cmap` before the fallback map is consulted at all.
  · **W-OPEN-SHELL** — open by phase at 100,000 rows: skeleton+viewport (what
    the 1.5 s budget names) **377–524 ms**, graph build **269–434 ms**, full
    recalc **423–457 ms**. The graph build's quadratic is **fixed** — 1M rows
    went **218 s → 5.36 s**. Read single figures from this host as ±30%.
  · **W-RECALC-PROFILE (TD-71)** — range reads through the tile store now cost
    **64–78 ns/cell** against ~206 before (flat in sheet size); wide-range
    recalc **4.30 → 2.73–2.94 µs/formula** at 100k rows, **4.46–4.55 → 2.87**
    at 500k. W-CHAIN-100K unchanged at 45–57 ms, which is the correct shape:
    its reads are single-cell and mostly hit computed results.
  · **W-XLSX-WRITE (session 29, re-run session 30)** — write fidelity
    **100.0%**, now **57/57 modelled cells over 21 corpus files** with font,
    fill and alignment added to the comparison key. The corpus grew a 21st file
    because the first 20 carry no styling at all, so the widened number would
    have been 100% whatever the style code did. Excel COM validation is
    session 29's (20/20 files) and was **not** re-run for the styled output —
    stated as a gap, and the first thing the next XLSX session should close.
  · **W-STYLE-COLUMN (session 30, ADR-041)** — 64 whole-column rules over a
    262,144-row sheet with no values in it: style state **14,576 B flat** across
    a 256× size range (**0.00087 B per addressed cell**), 64 rules, **1**
    interned value, **0 tiles**, and cell-store bytes identical to the same
    sheet unstyled. A per-cell store's floor would be ~1.5 GB. Facet resolution
    **112–195 ns/cell**, flat in sheet size — after a 15–26× fix the workload
    itself found (it measured 1,826–3,361 ns first).

---

## NEXT ACTION

**The editing surface works and is not finished. In order:**

1. **TD-80 — warm the font database off the frame.** This is the residual
   TD-79's fix created and it is *measured, profiled and scoped* (W-FALLBACK,
   D-125): the first codepoint the bundled face cannot draw costs **238–412 ms**
   inside `TextEngine::layout`, against docs/31's 16 ms keystroke→paint. The
   cause is not a guess — the enumeration is **203–321 ms** of it (379 face
   files read and parsed to learn their names) and the pick is the remainder, so
   the fix is to move the enumeration, not to make the search cleverer.

   **The shape, so the next session does not rediscover it.** Build the
   `fontdb::Database` on a background thread and hand it over a
   `std::sync::mpsc` channel; `TextEngine::resolve` takes the receiver and
   `recv()`s, which blocks exactly as today if the user is faster than the scan
   and is free otherwise. **The decision that matters is *where the thread
   starts*: `App::open`, not `TextEngine::new`** — 114 tests and every benchmark
   construct a `TextEngine`, and starting it there would have all of them spawn
   a 300 ms file scan they never use. Two numbers must be re-measured, not one:
   W-KEYSTROKE **and cold launch** (39.8 ms against docs/31's 1.0 s), because a
   background file scan competing with startup is the thing that could go wrong.
   TD-80 also names two residuals to fix or re-file while there: `pick`'s
   exhaustive last-resort pass re-reads every face on a host whose fonts are in
   none of `PREFERRED` (seconds, untested — M1 never reaches it), and the
   resolution is not shared between processes.

   **Then, and only then, attempt docs/48's *"IME validated by native JP/CN/KR
   typists"*** — TD-79 unblocked it, and a native typist's very first keystroke
   is the one that pays TD-80.
2. Then **accesskit tree v1** and the platform adapters (menus, dialogs, file
   association). **Read the headroom note in CURRENT STATE first**: fallback
   spent 3 of the 34 crates, leaving 31, and ADR-037's earmark was ~50 for
   accesskit *plus* the adapters. That estimate no longer fits and the three
   should be budgeted together — measure accesskit's closure with
   `default-features = false` before writing any of it, exactly as sessions 24
   and 32 did for `arboard` and `fontdb`, both times with the measurement
   changing the answer.
3. Then **validation / conditional formatting / sort / filter / tables**.
   **Styles are off this list** — done in session 30 through the op layer
   (**ADR-041**), with the round-trip re-published (W-XLSX-WRITE, 100.0% over
   57/57 cells and 21 files) and the memory property measured
   (W-STYLE-COLUMN). **XLSX write is off it too** (session 29). What remains of
   both is register debt with triggers: TD-72 deflate, TD-73 control
   characters, TD-74 unaccounted parts, and TD-75/76/77/78 for the style
   facets that were deliberately not modelled.

   **Read ADR-041 before starting any of the four**, because it decides the
   shape of all of them. The question it forces on each: *is this a facet, or a
   new object?* Formatting-like features that are a value on a cell are facets
   — cheap, no new op tag, no ADR. **Conditional formatting is not one**: it is
   a rule that *computes* a style from a formula, so it is a new object with a
   formula group behind it, and it needs its own ADR. Validation is the same
   shape (docs/11 already calls both "formula groups"). Sort is different again
   — docs/11 specifies it as an *identity permutation op*, which the axis
   already has the machinery for. Tables lean on named styles, which is TD-77.

   **The cheapest real work in this area, if a short session is wanted:**
   borders as facet tag `0x05` (TD-75). ADR-041 decision 3 exists precisely so
   this costs no op tag and no ADR — encoder, decoder, the `<borders>` table in
   the writer, and the reader's loss record. It is the one gap most likely to
   show up in the first customer file.

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
**Profile before building the fix** stands at four-for-four. Session 32 is the
fifth application and the first where the profile ran *before the debt row was
even written*: TD-80 was filed with `--fonts`'s split already in it, so the row
says "the enumeration is 203–321 of 238–412 ms" instead of "the first miss is
slow, probably the enumeration". That is the register entry the rule is for.

**Gated and should stay closed** (D-112): TD-17 and TD-44 have triggers that
measurement shows are not live; TD-37 is blocked on packaging rather than
effort. **TD-63** — confirm the bundled font's licence before the first
installer. TD-79 is paid without bundling a second face (D-125 took option (b),
not (a)), so no second bundled licence arrived; but **fallback loads *system*
faces at runtime, which is a distribution question of a different kind** and the
installer work should read TD-63 and D-125 together.
**TD-62** (no shaped-run cache) and **TD-58** (the axis rebuilds
prefix sums in O(n)) are filed and below their triggers; **TD-65** may turn out
to be TD-58 wearing a different hat, which is why it says the cause is
unattributed instead of guessing.

**TD-46's container half is still open.** `Snapshot::build` still writes the
compacted op set, so TD-24's snapshot residual and TD-57 stand. D-103 states the
remaining fork with both options measured; the recommendation on record is (b),
reconstruction at snapshot time, streamed per tile band. Do not implement (a)
without the ADR — it retires a frozen ADR's central claim.

---

### THE EXACT NEXT ACTION

**Pay TD-80 — move the font enumeration off the frame. The profile is already
done; do not redo it, and do not re-litigate the fallback design (D-125).**

1. Read **docs/44 TD-80**, **D-125**, and **MEASUREMENTS.md §W-FALLBACK** — in
   that order. W-FALLBACK already contains the split this work needs
   (enumeration 203–321 ms of a 238–412 ms first miss, 379 faces on M1), so the
   first thing to do is *not* another measurement. `--fonts` reproduces it in
   two seconds if you want to see it.
2. **The change, and its whole surface.** `TextEngine::resolve` in
   `shell/ehkatra-shell/src/text.rs` currently does
   `db.load_system_fonts()` inline the first time a codepoint misses. Replace
   `system: Option<fontdb::Database>` with a warm-up: a `std::thread::spawn` that
   builds the database and sends it down a `std::sync::mpsc` channel, and a
   `resolve` that `recv()`s it once. Blocking on `recv()` is exactly today's
   behaviour when the user is faster than the scan, so the change can only be
   neutral-or-better, never worse.
3. **The decision that matters, and it is not the threading.** The thread must
   start in **`App::open`** (`app.rs`), *not* in `TextEngine::new` — 114 tests
   and every benchmark construct a `TextEngine`, and starting it there makes all
   of them spawn a 300 ms file scan they never use. So `TextEngine` needs a
   `warm()` method the app calls, and the default must remain lazy.
4. **Two numbers must move or be shown not to, not one.**
   `ehkatra-shell --keystroke 10000` for W-KEYSTROKE, **and cold launch** from
   `--present` (39.8 ms against docs/31's 1.0 s), because a background file scan
   competing with startup is the failure mode this introduces. Also re-run
   `--fonts` — its `first miss` line should drop to the pick alone once warmed,
   and if it does not, the hand-off is wrong.
5. **The test that closes it** must prove the *ordering*, not the speed: a test
   asserting `App::open` leaves the engine warm — e.g. that a resolve after a
   short settle does not block — is timing-dependent and therefore a flaky test
   (DP-C5). Assert the structure instead: that `TextEngine::warm` is idempotent,
   that a `resolve` before warming still succeeds (the lazy path must survive),
   and that a warmed engine resolves without touching the lazy constructor.
6. TD-80 names two residuals to fix or deliberately re-file while you are in
   here: `pick`'s exhaustive last-resort pass re-reads every face on a host
   whose fonts are in none of `text::PREFERRED` (seconds, and untested because
   M1 never reaches it), and the resolution is not shared between processes.
7. Only then attempt docs/48's *"IME validated by native JP/CN/KR typists"*.
   TD-79 unblocked it — `demo/editing-ime.png` shows real kana now — and TD-80
   is the last thing between it and a first keystroke that does not stall.

Baseline to return to if anything goes wrong: `.checkpoints\32-fallback\` holds
`text.rs`, `scene.rs`, `app.rs`, `script.rs`, `Cargo.toml` and the shell
`Cargo.lock` as they were **before** this session's fallback work.

Baseline to return to if anything goes wrong: `.checkpoints\31-ime\` holds
`app.rs`, `scene.rs`, `window.rs`, `input.rs`, `script.rs` and `text.rs` as they
were **before** this session's IME work.
