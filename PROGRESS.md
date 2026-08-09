# PROGRESS.md — Ehkatra build log (current state)

History for sessions 1–19 lives in [`archive/PROGRESS-sessions-01-19.md`](archive/PROGRESS-sessions-01-19.md).
This file stays short on purpose: it is what a human returning after days away
reads first, so it holds the handoff, the current state, and the next action —
not the story of how they came about. That is what the archive and the
registers (docs/43 decisions, docs/44 debt, MEASUREMENTS.md numbers) are for.

---

## HANDOFF — read this first

**Where the tree is:** v0.1 is complete and tagged `v0.1.0`. Q1 is done. The
work in flight is the compatibility-debt push toward **W-ORACLE ≥ 90%**
(now **89.3%**, 1,220 / 1,366 cases — up from 74.2% at the start of session 20),
followed by the structural debt around the tile image, then Q2's shell.

**Session 20 in one paragraph.** Paid **eight** debt items — TD-47, TD-33,
TD-14, TD-35, TD-34, TD-32, TD-51, TD-54 — moving W-ORACLE **74.2% → 89.3%**,
+206 cases against a 90% target. Every unit is committed and pushed with CI
green. Eleven functions reached 100% — `DATE`,
`DAY`, `MONTH`, `YEAR`, `WEEKDAY`, `VLOOKUP`, `HLOOKUP`, `XLOOKUP`, `MATCH`,
`FIND`, `SEARCH` — plus the whole `__compat_literal_parser` corpus. Tests
323 → 343. The replay hashes never moved, which is the evidence that none of it
touched the op algebra.

**The through-line, worth reading before the next cluster.** Five of the eight
were paid by finding that the *documented* rule and the *implemented* rule
differ, and only the oracle could say which one is Excel: approximate lookup is
a binary search whose answer on unsorted data no reasonable linear scan
reproduces (D-106); criteria coerce across the text boundary where lookups do
not (D-107); the 15-digit truncation happens on the source text before
conversion, and truncates rather than rounds (D-108). ADR-024's premise — the
binary is the spec — earned its keep this session, and the same should be
expected of what is left.

**The remaining 146 failures, and how close 90% is.** The session ends **0.7
points short of the 90% target** — about **10 cases**. They are already
identified: **TD-52** (~5, an omitted argument is a parse error rather than a
blank: `=IF(TRUE,,2)` is `0` in Excel), **TD-53** (~4, `IF`'s condition coerces
text logicals but not numeric text, and the engine does exactly the opposite),
and `ERROR.TYPE`, which is simply unimplemented. **Those three alone should
cross 90%**, and none needs a new subsystem. **TD-36 (`TEXT()`, ~28)** is the
largest single item after that and needs a number-format grammar — budget a
session for it rather than starting it late.

**Per-item detail is in the registers, not here.** D-104 (TD-47), D-105
(TD-33), D-106 (TD-14/TD-35), D-107 (TD-34), D-108 (TD-32) and D-109
(TD-51/TD-54) in `docs/43`; the paid rows and their residues in `docs/44`; every
number in MEASUREMENTS.md under W-ORACLE. This file keeps only what a returning
human needs before choosing what to do next.

**One process rule from this session, because it has now bitten twice.** TD-47
claimed `tools/gates.ps1` needed PowerShell 7 and aborted on this 5.1-only host.
Measured: it runs top to bottom, one invocation, every gate green, exit 0 —
bare, fully redirected, and dot-sourced. Nothing needed fixing; what it needed
was *running*. TD-48 (filed in session 18 under TD-28's number) was the same
defect. D-078's lesson was not enough, because both rows named a clearing
condition and neither was ever run, so **D-104 strengthens it: a debt row
asserting something about this host is not filed until the command
demonstrating it has been executed and its output pasted into the row.**
Register IDs are append-only — a paid row is struck through and keeps its number
forever, because a vacant-looking ID is what invited TD-28's reuse.

The 5.1 `NativeCommandError` mechanism is real and was reproduced, but it fires
only when a native command's stderr is merged into the pipeline, which
`gates.ps1` never does. Two gates now stand where the assumption was and run
first: a runtime probe, and a static grep refusing any stderr-into-pipeline
redirection in `tools\*.ps1`.

---

## CURRENT STATE

- **Gates:** ALL GREEN via one command — `powershell -File tools\gates.ps1`
  (also `pwsh -File tools/gates.ps1`). Shell compat · fmt · clippy `-D warnings`
  · tests · no_std wasm32 kernel build · dep budget · supply chain (cargo-deny)
  · differential replay native == wasm32 · purity/host-isolation greps.
- **Tests:** 343, all passing.
- **Replay hashes:** oplog `c79fa533…` · state `b58d5505…` (unchanged by the
  date work — it is additive to the op algebra).
- **Dependency budget:** kernel direct 1/5 · kernel closure 10/12 · workspace
  closure 29/40.
- **W-ORACLE:** **89.3%** overall (1,220 / 1,366) — 0.7 points off the 90% target.
- **Open structural debt:** TD-46 (the tile image is built, tested, fuzz-clean
  and measured — and still not the snapshot body), and with it TD-45, TD-31 and
  TD-24's residual, all of which close together.

---

## NEXT ACTION

**Compatibility debt, in the order docs/44's W-ORACLE table ranks it by measured
case count** — the ranking is the point of having a runner, so follow it rather
than re-guessing:

1. ~~TD-33 — date semantics~~ **PAID (session 20)**, +126 cases.
2. ~~TD-51 — direct arguments vs range cells~~ **PAID (session 20)**, with
   ~~TD-54~~. **Start with what is left of that pass: TD-52, TD-53 and
   `ERROR.TYPE`** — ~10 cases between them, all argument and error handling in
   functions already touched, and enough to cross **90%**.
2b. **TD-36 — `TEXT()`** (~28), unimplemented; needs the number-format grammar,
   which is a language of its own and the largest *single* item left. It also
   unblocks the residual `__compat_1900_leap` / `__compat_serial_boundary`
   cases TD-33 left — they fail on `TEXT`, `DATEVALUE` and `EOMONTH`, not on
   date arithmetic. Budget a session for it rather than starting it late.
3. ~~TD-14 — approximate-match lookup~~ **PAID (session 20)**, with ~~TD-35~~.
4. ~~TD-34 — the criteria sub-language~~ **PAID (session 20)**, +18 cases.
5. ~~TD-32 — `compat_parse_15`~~ **PAID (session 20)**, +16 cases.
6. **TD-16 — implicit intersection** — not in the original list, but it is now
   what holds the last three `INDEX` cases down, and it needs the dependency
   graph to supply the calling cell's position.

Re-run the conformance harness after each cluster and record every new number in
MEASUREMENTS.md against **W-ORACLE**. A number without its workload id is invalid
(docs/38).

**Still open and unchanged from session 19: the TD-46 ADR must be written before
any stamp plumbing is implemented.** D-102 settled the encoding (delta-varint,
3.1 B/cell, 153.2 MB at 10M — passes A-001); D-103 states the remaining fork,
which is an ADR-005 question about where stamps come from, with both options
measured. The recommendation on record is (b), reconstruction at snapshot time,
streamed per tile band. Do not implement (a) without the ADR — it retires a
frozen ADR's central claim.
