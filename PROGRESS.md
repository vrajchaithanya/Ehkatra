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

**Session 35 in one paragraph.** **TD-84 is paid: a JP or CN typist can see
which clause they are converting.** `Editor::display` now returns a
**`Composition`** — the whole composition's span *and* the focused clause's
sub-span — and `scene.rs` draws the focused clause as a wash **behind** the glyph
run while the underline stays exactly as it was (**D-128**). `demo/ime-jp.png` at
step 7 is the picture of the fix where it used to be the picture of the defect:
`晴れ` shaded, `今日は` clear, one underline across all five characters. Cost:
**zero crates, zero op tags, no kernel surface**; replay hashes unmoved
(`a1b35c1a…` / `b95f1632…`), which is what a display-layer fix must do. Tests
560 → **563** (kernel 438 unchanged, shell 122 → 125). Gates green.

**The decision that mattered was not the plumbing — it was what the second span
*means*.** Widening a return value is mechanical. The choice that would have been
made by accident inside the fix is whether the underline **moves** to the focused
clause or **stays** whole. Moving it is tempting: one span, no new token. It is
wrong, and worse than the defect. The underline means *"this is a proposal and not
yet your text"*, and that is true of the unfocused clauses too — so underlining
only the focus would tell a typist that `今日は` had already been committed while
they were still converting `晴れ`. That trades a **missing** signal for a **false**
one. So the composition carries two spans, which is also what the platform IMEs
actually draw, and `Composition` is a named struct rather than a second
`Option<(usize, usize)>` in the tuple because two same-typed span options side by
side are exactly where the wrong two get swapped.

**Quad order is the whole difference between a highlight and a censor bar, and it
is invisible in code review.** The same quad — same rect, same colour — pushed
*after* `push_run` covers the very text the user is choosing a candidate for.
Nothing about either quad tells you which one you wrote; only its index in the
list does. So it has its own assertion (`first < glyph`) rather than a comment,
and the assertion was checked against a control: moving the push after the run
fails it with *"the wash is at 102 and the first glyph at 96"*.

**Session 34's inverted test did its job, which is the part worth keeping.**
`a_multi_clause_conversion_moves_the_caret_but_cannot_show_the_focus` was written
to assert the *defect* — underline `(0, 15)` in both conversion states —
specifically so that paying TD-84 would have to break it. It broke. It is now
`…shows_which_clause_the_arrow_keys_are_on`, and it still asserts the unchanged
half: the underline is `(0, 15)` in both states. **Both new tests were run against
negative controls before being believed**, because a test written after the fix
passes by construction: substituting `span` for `focus` in the scene fails at the
first assertion (*"a caret must not shade anything"*), and the ordering control
above. That is the same discipline session 28 arrived at the hard way — a test
that passes both ways is not a regression test.

**The two frames with *no* wash are the other half of the evidence.**
`demo/ime-cn.png` and `demo/ime-kr.png` show an underline and no highlight, which
is correct and not an omission: Microsoft Pinyin reported a caret rather than a
range at every step of this sequence, and **Korean has no conversion phase at
all**. All 19 CN and KR steps report `focus: none`, so Korean is the control
proving the wash appears *only* where an input method asked for one. A highlight
that showed up in Hangul would be a new defect, and nothing would have caught it.

**The platform's range gets the same distrust its caret already got.** The offsets
are the IME's, so `focus` is clamped to the composition and **normalised** if
reported end-first — a reversed range is a focused clause, not an empty one, and
the scene's `right > left` check would have silently dropped it. Four cases are
asserted: past the end, reversed, both-past-the-end (which collapses to a caret
and therefore to *no* focus rather than a zero-width highlight), and absent.

**Deliberately not measured, and stated so rather than left implied.** The fix
adds at most **one quad** to a composing frame that already carries ~1,500.
Re-publishing W-KEYSTROKE's composing figure would have produced a number
indistinguishable from 1.68–2.10 ms, and session 33 already paid for the lesson
that an indistinguishable number is not evidence. **docs/48's IME item is still
◑ half closed** — paying a debt row that the mechanical half *found* does not make
the perceptual half any more closed (D-127). What changed on the 7-item checklist
is item 4's shape, not the count: from *"we draw no clause emphasis at all"* to
*"we draw one, and whether a blue wash is the right one for your writing system
needs a reader"*. Six of seven untouched.

**Session 34 in one paragraph.** **docs/48's IME item is half closed, and the
half that closed found two defects.** No native typist exists here and one is
not coming, so the first thing written was the *judgement* rather than the code
(**D-127**): a native typist is the oracle for this item exactly as Excel is the
oracle for conformance (D-123), so the box **cannot be ticked** — what is
available is to make the un-ticked part small, specific and named. The item
splits. The mechanical half — *do we handle the event shape each script's input
method actually produces* — is a property of our code alone and is now closed:
`--ime` (**W-IME-SCRIPTS**) replays JP, CN and KR composition sequences through
the real `App` and checks **29/29 steps**, each against the four things a user
can see. The perceptual half is published as a **7-item checklist** in
MEASUREMENTS.md rather than absorbed. Tests 556 → **560** (kernel 438 unchanged,
shell 118 → 122). Gates green; replay hashes unmoved — shell-only, no encoding.

**Three scripts, because three *shapes*.** A fourth kana test would have proved
one shape a fourth time; these three are chosen for how they differ. **JP** types
kana per keystroke and then *converts*, replacing the whole composition with
kanji and reporting the focused clause as a **selection range**. **CN**
(Microsoft Pinyin) is **ASCII for six of its nine steps** and becomes Han only at
partial conversion, so a face seam opens *inside* a live composition with the
caret sitting on it. **KR** composes **one syllable at a time**, so the keystroke
that starts the next syllable *commits* the previous one and `Commit` interleaves
with `Preedit` inside a single word — the shape most likely to break an editor,
and the one nothing in this repo had ever exercised. All three pass, all three
commit their cell, all three have a frame: `demo/ime-{jp,cn,kr}.png`.

**The step check is what makes it evidence rather than a demo.** Each step
asserts the display string, the caret's byte offset in it, the underline span,
and **that the cell is still blank** — the last being the invariant no screenshot
shows and the one that matters most, since a document that acquired text from an
unconfirmed composition is wrong in a way nobody notices until it ships. The
same tables run in the suite through `App::open_detached` with no GPU and no
assertion about a glyph, a face or a pixel, so a host with no CJK font installed
still proves the semantics; the driver adds only what a headless suite cannot
have — the frames and the resolved face. A fourth test abandons **every script at
every one of its steps** and asserts the cell stayed blank, because "run it to
the end" is not what a user who changes their mind does.

**Defect 1 — TD-83: Chinese is drawn by a Japanese face.** `中文` resolves to
`Yu Gothic`. `PREFERRED` is ordered by *coverage breadth*, Yu Gothic is 7th and
covers these codepoints, so `Microsoft YaHei` at 10th is never reached. Han
unification means the characters are correct and the **shapes** are not — to a
native reader that is the difference between their language and a foreign font.
**The alternative cause was measured and ruled out, not argued away**:
`TextEngine::resolve` reuses an already-loaded face before consulting the
database, so `中` could have inherited the kana's face for a reason having
nothing to do with `PREFERRED`. So the driver reports the face **in session** and
the face a **completely fresh engine** picks alone — they are identical for all
three scripts, so it is the preference order. `한글` reaching `Malgun Gothic` at
position **12** is the control that makes the reading safe: the list *is* walked
that far when it must be, so nothing is truncating the search. Korean is right by
accident, because no Japanese face covers Hangul.

**Defect 2 — TD-84: in a multi-clause conversion the focused clause is
invisible.** A converting IME reports the clause the arrow keys are on as a byte
**range**; `Editor::display` keeps the range's start and discards its end, so
`今日は晴れ` focused on `今日は` and the same string focused on `晴れ` draw
identically apart from the caret. Session 31 built this surface against `にほん`,
where a range and a caret are indistinguishable — which is why 13 IME tests and a
session of fallback work never saw it, and why the *second clause* was worth
scripting. Not fixed here: the session that found it had already landed the
driver that found it, and a scene change stacked on top would be the second
unverified layer (DP-C4). `demo/ime-jp.png` is the picture of the defect.

**Both defects are the argument for the shape of D-127.** Neither is reachable
by a stopwatch or by another kana test, and both were found *without* a typist —
which is the evidence that closing half the item on its own was worth doing
rather than waiting for a person who may never arrive.

**Session 33 in one paragraph.** **TD-80 is paid: a CJK user's first keystroke
no longer stalls the grid for a third of a second.** The enumeration is off the
frame — `App::open` starts a background thread that builds the
`fontdb::Database` and hands it over an `mpsc` channel (**D-126**) — and the
first non-Latin codepoint went **244–887 ms → 16.8–21.3 ms** on M1. The profile
from session 32 was right and was not redone: the enumeration was 203–321 ms of
the miss, so moving it was the whole fix, and the remainder is now the whole
cost. Tests 552 → 556 (kernel 438 unchanged, shell 114 → 118). Gates green,
replay hashes unmoved — a shell-only change touches no encoding.

**The decision that mattered was not the threading.** It was *where the thread
starts*, and PROGRESS.md had already named it: `App::open`, never
`TextEngine::new`, because 118 shell tests and every benchmark construct an
engine and would each have spawned a scan of several hundred font files for a
fallback they never use. What that leaves out — and what the handoff did not
say — is that `App::open_detached`, the constructor the *suite* uses, goes
through `App::open`. So the split is one layer lower: `App::open_cold` builds
the app, `App::open` is that plus one `warm()` line, and `open_detached` uses
`open_cold`. **Both halves are asserted**, and the second is the one that would
have rotted silently: `warm` in the shared constructor keeps every test passing
while quietly making the suite slower for a reason nothing names.

**`Option` was the wrong shape and that is the substance of D-126.**
`system: Option<Database>` cannot tell *"not built yet"* from *"a thread is
building it"* — the first must build inline, the second must wait — and
collapsing them is exactly how a warmed engine enumerates twice. So
`SystemFonts { Cold, Warming(Receiver), Ready(Database) }`. A miss arriving
mid-scan `recv()`s, which blocks precisely as the old inline build did, so the
path is **neutral or better and never worse**; a miss on a cold engine builds
inline, so the lazy path survives for every caller that is not an `App`. Thread
spawn failure and a dead thread both fall back to `Cold` rather than panic
(DP-A10): a shell that cannot start a thread must still draw kana.

**The tests are structural, and they were checked by being broken.** The
tempting test — warm, sleep, assert the resolve was fast — is a race dressed as
an assertion (DP-C5). Instead the engine ships two counters, `lazy_builds` and
`warm_spawns`, and the claims become numbers: *"the enumeration happens
elsewhere"* is `lazy_builds == 0`, *"warm is idempotent"* is `warm_spawns == 1`.
The second counter exists because a `warm` that spawned a second scan and handed
over the second database would look identical from outside and cost twice as
much. All three warm tests were then **re-run with the fix disabled and all
three fail** — session 28's lesson, that a test which passes both ways is not a
regression test.

**The measurement that would have been wrong, and the control that caught it.**
TD-80 required cold launch to be re-checked, since a background file scan
competing with startup is the failure mode it introduces. The warmed build
opens to first frame in **52.5–69.3 ms** against session 32's recorded **39.8
ms** — which reads as a regression caused by exactly the suspected thing. It is
not. A **control binary was built with the single `warm()` line removed** and
measured on the same host minutes later: **51.1–65.2 ms**. The distributions
overlap; the warm-up's cost to launch is not distinguishable from run-to-run
variance, and 39.8 ms was one sample from the low end of a distribution. Cold
launch on M1 is **~50–70 ms**, warmed or not, against docs/31's 1.0 s. W-PRESENT
and W-OPEN-SHELL are annotated so nothing still quotes 39.8 as *the* number.
W-KEYSTROKE was re-measured and is unmoved: **1.73–2.62 ms p50 typing, 1.68–2.10
ms composing** against 16 ms — the scan does not steal the frame beside it.

**What is still owed is measured, not suspected — and no fix was built for it.**
16.8–21.3 ms is still **1.05–1.33× docs/31's 16 ms**. That is `pick` itself, now
that nothing stands in front of it: walking `PREFERRED` and loading the family
that answers (`Yu Gothic`, the 7th entry, a large CJK face). The **split inside
the pick is not measured**, and two different causes with two different fixes
are visible in the code — `pick` parses a candidate face to test coverage and
`resolve` then reads and parses the same face *again*, and rustybuzz's parse of
a multi-megabyte CJK file may simply cost this. Choosing between them without
profiling is how the last five performance rows went wrong. Filed as **TD-82**
with both candidates named and neither chosen. `pick`'s exhaustive last-resort
pass and the un-shared per-process resolution were carried into TD-82 rather
than fixed, and that is stated rather than quietly dropped.

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
- **Tests: 563** — 438 kernel + 125 shell. All passing.
- **Replay hashes:** oplog `a1b35c1ac5afa7b5…` · state `b95f16327e2e9e88…` —
  **unmoved in sessions 31, 32, 33, 34 and 35** (a shell-only change touches no encoding), and
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
- **docs/48 §Desktop quality, IME item: ◑ half closed** (D-127) — **still, after
  session 35**. The mechanical half is proven — W-IME-SCRIPTS, 29/29 steps over 3
  scripts, now including the focused clause at every step. The item **stays
  open**: a native typist is the oracle and has not run it, and the residue is a
  7-item checklist in MEASUREMENTS.md §W-IME-SCRIPTS, not a vague "needs
  testing". Paying TD-84 narrowed **item 4's question** ("we draw a clause
  emphasis; is a blue wash the right one for your writing system?") and closed
  none of the seven. Do not report this item as closed.
- **The numbers that are new** (MEASUREMENTS.md):
  · **W-IME-SCRIPTS (re-run session 35, TD-84 paid)** — **29/29 steps, 3/3
    cells**, unchanged, with a **focus** column added to every step. The JP
    conversion reports underline `(0, 15)` and focus `(0, 9)` → `(9, 15)` as the
    arrow key moves; all 19 CN and KR steps report `focus: none`, which is the
    control. Faces unchanged (JP/CN `Yu Gothic`, KR `Malgun Gothic`, 0
    unresolved) — **TD-83 is untouched and still open**. Frames re-published;
    `demo/ime-jp.png` is now the picture of the fix. **No latency number was
    published on purpose**: one extra quad in a ~1,500-quad frame is below this
    host's ±30% spread, and session 33's lesson is that an indistinguishable
    re-run is not evidence.
  · **W-IME-SCRIPTS (session 34, new workload — docs/38)** — the JP, CN and KR
    composition sequences replayed through the real `App`: **29/29 steps, 3/3
    cells**, 0 unresolved characters. Faces resolved on M1: JP `今日は晴れ` →
    `Yu Gothic`, CN `中文` → **`Yu Gothic`** (that is TD-83, not a typo), KR
    `한글` → `Malgun Gothic` — and the same three from a **fresh engine with no
    session history**, which is what rules out the loaded-face shortcut as the
    cause. Frames `demo/ime-{jp,cn,kr}.png`. Re-run: `ehkatra-shell --ime demo`.
  · **W-FALLBACK (session 33, TD-80 paid)** — the first codepoint the bundled
    face cannot draw now costs **16.8–21.3 ms**, down from **244–887 ms**: the
    enumeration (222–283 ms, 379 faces on M1) runs on a background thread that
    `App::open` starts, and the warmed engine builds **0** databases inline
    from **1** warm-up. What remains is the `pick` alone, still **1.05–1.33×**
    docs/31's 16 ms — **TD-82**, filed with the split inside it deliberately
    unmeasured and no fix built. Session 32's cold figures (238–412 ms, of
    which 203–321 enumeration) still stand as the *unwarmed* path, which is
    what a `TextEngine` without an `App` around it takes.
  · **Cold launch, re-measured against a control (session 33)** — **52.5–69.3 ms
    warmed vs 51.1–65.2 ms from a binary built without the warm call**, 5 runs
    each, overlapping. The warm-up is not distinguishable from run-to-run
    variance, and session 32's single **39.8 ms** was the low end of a
    distribution rather than the value. Read cold launch as **~50–70 ms on M1**
    against docs/31's 1.0 s. W-PRESENT and W-OPEN-SHELL carry the correction.
  · **W-KEYSTROKE (re-run session 33)** — **1.73–2.62 ms p50 typing, 1.68–2.10
    ms p50 composing**, against 16 ms. Unmoved with the font scan running
    beside it.
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

1. **TD-84 is paid (session 35, D-128)** — the focused clause is drawn.
   **TD-83 is now the head of this list, and it is *not* small.** It needs a
   language signal per run and **wants a D entry before any code**; do not "just
   reorder `PREFERRED`", which only moves the harm from Chinese readers to
   Japanese ones. It is in docs/44 with the measurement that found it, and that
   measurement is already done — do not redo it.
   *(docs/48's IME item itself is as closed as it gets without a person —
   D-127. Do not re-attempt the mechanical half; extend W-IME-SCRIPTS if a new
   shape appears, and read the 7-item checklist in MEASUREMENTS.md before
   claiming any part of the perceptual half.)*
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
their triggers. The shell's *non*-performance rows are now **TD-83** (Chinese
drawn by a Japanese face) and **TD-84** (the focused clause is invisible), both
filed in session 34 with the measurement that found them; TD-84 is the next
action and TD-83 needs a decision before code.

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
**Session 33 is the sixth, and it is the first time the rule paid off on the
way *out* rather than on the way in.** TD-80's fix went in exactly as the
profile predicted — the enumeration was the miss, so moving it was the whole
fix. The trap was in the *verification*: cold launch measured 52.5–69.3 ms
against session 32's recorded 39.8 ms, which is a textbook "the background scan
you just added is competing with startup". A control binary built without the
one `warm()` line measured 51.1–65.2 ms on the same host. The regression was not
there; the 39.8 ms was one sample. **Compare against a control run in the same
conditions, not against a recorded number** — a corollary the register did not
have before, and the reason W-PRESENT now says ~50–70 ms.
**Session 34 is the seventh, and it is the first time the rule applied to
something that is not a performance number at all.** TD-83's obvious cause is
`PREFERRED`'s order; its *other* cause — `resolve` reusing an already-loaded
face before it consults the database, so `中` inherits whatever the kana loaded —
is equally visible in the code and has a completely different fix. So the driver
reports the face **in session** and the face a **fresh engine** picks alone, and
the two being identical is what makes "it is the preference order" a measurement
instead of a reading. **The rule generalises: when a defect has two visible
causes, measure which one before naming it in the register**, whether or not
the defect is about time.

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

**Write the D entry for TD-83 — how the display layer learns what language a run
is in — and only then change anything. Do not "just move `Microsoft YaHei` up
`PREFERRED`": that is the whole trap, and it is why this row is a decision before
it is a fix.**

0. **Build note, unchanged and still true.** A release build of the shell needs
   the in-repo MinGW on PATH or `libsqlite3-sys` fails with *"failed to find
   tool gcc.exe"*. `tools\gates.ps1` does this for you; a bare `cargo build
   --release` does not. From bash:
   `export PATH="/c/Users/velag/Desktop/Ehkatra/.toolchain/mingw64/bin:$PATH"`.
   Nothing is wrong when you see that error — the toolchain is just not on PATH.
   **Second note, learned in session 35:** the `--ime` driver writes into
   `demo/`, so run the built binary **from the repo root**
   (`./shell/target/release/ehkatra-shell.exe --ime demo`). `cargo run` from
   inside `shell/` fails with *"The system cannot find the path specified"* —
   that is the working directory, not a defect.
1. **Read TD-83 in docs/44 and MEASUREMENTS.md §W-IME-SCRIPTS first.** The cause
   is **already measured and the alternative already ruled out**: `中文` resolves
   to `Yu Gothic` both inside a Japanese session *and* on a completely fresh
   engine, so it is `PREFERRED`'s order and **not** `resolve`'s
   already-loaded-face shortcut. Do not re-measure this, and do not accept the
   shortcut as the cause.
2. **Why reordering is wrong, written down so it is not rediscovered.**
   `PREFERRED` is ordered by *coverage breadth*, which was the right question for
   TD-79 (draw something instead of a box) and is the wrong question now that the
   character is drawn. Yu Gothic (7th) covers the Han a Chinese user types, so
   YaHei (10th) is never reached. Putting YaHei first makes every Japanese reader
   see Chinese glyph forms instead. The list cannot be ordered correctly because
   **nothing in it knows what language the text is in** — that is the actual
   defect, and a reorder only picks a different victim.
3. **The decision the D entry has to make, and the constraint on it.** A
   **script/language signal per run**. Unicode script property is *not* enough
   (Han is Han). **DP-D5 says locale never enters storage**, so the signal is a
   display-layer input: the candidates on record in TD-83 are the document's or
   the user's locale as a display-layer input, or per-cell language tagging as
   XLSX itself carries. Whichever is chosen it is facet-shaped, and the entry
   should also say what happens to a run with **no** signal — today's `PREFERRED`
   walk is the obvious default, and saying so explicitly is part of the decision
   rather than a detail left to the code.
4. **Follow D-128's shape if the fix reaches `scene.rs` or `text.rs`.** What
   worked in session 35: decide what the new input *means* before plumbing it,
   name the thing rather than adding another same-typed field to a tuple, and
   **check every new test against a control** — a test written after the fix
   passes by construction. Both of session 35's controls earned their keep: one
   caught a wrong-field substitution, one caught draw order.
5. **TD-82 is still deliberately after TD-83.** 16.8–21.3 ms against a 16 ms
   budget on one keystroke per process. Item 3 of the typist checklist is what
   answers whether it is perceptible, and that answer has not arrived. If you do
   take it on: **profile the split inside `pick` before writing anything**
   (`covers` / `with_face_data` / `rustybuzz::Face::from_slice`, and the fact
   that `resolve` re-reads the same face `pick` just parsed).
6. **What not to redo.** W-IME-SCRIPTS covers three event shapes; adding a
   fourth kana variant proves nothing. Extend it only if a genuinely new *shape*
   appears (a Vietnamese Telex or a Thai reordering IME would be one). And
   nothing in this repo may report docs/48's IME item as closed — **paying TD-84
   did not close it.** D-127 fixes what "half closed" means, and MEASUREMENTS.md
   lists the seven questions still open, of which session 35 narrowed one and
   closed none.

Baseline to return to if anything goes wrong: `.checkpoints\35-imefocus\` holds
`app.rs`, `scene.rs`, `ime.rs`, `docs/43` and `docs/44` as they were **before**
this session's TD-84 work.

Baseline to return to if anything goes wrong: `.checkpoints\34-imescripts\`
holds `main.rs`, `app.rs`, `text.rs`, `docs/43` and `docs/44` as they were
**before** session 34's IME-script work. (`ime.rs` is new in session 34;
deleting it and its `mod ime;` line in `main.rs` reverts the whole change.)

Baseline to return to if anything goes wrong: `.checkpoints\33-warmfonts\` holds
`text.rs`, `app.rs` and `main.rs` as they were **before** session 33's
warm-up work.
