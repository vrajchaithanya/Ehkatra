# PROGRESS.md — Ehkatra build log (current state)

History for sessions 1–19 lives in [`archive/PROGRESS-sessions-01-19.md`](archive/PROGRESS-sessions-01-19.md).
This file stays short on purpose: it is what a human returning after days away
reads first, so it holds the handoff, the current state, and the next action —
not the story of how they came about. That is what the archive and the
registers (docs/43 decisions, docs/44 debt, MEASUREMENTS.md numbers) are for.

---

## HANDOFF — read this first

**Where the tree is:** v0.1 is complete and tagged `v0.1.0`. Q1 is done. The
compatibility-debt push has **met its target: W-ORACLE is 90.4%**
(1,235 / 1,366), up from 74.2% two sessions ago. What is in flight now is the
**structural** debt around the tile image, then Q2's shell.

**Sessions 20–21 in one paragraph.** Paid **ten** debt items — TD-47, TD-33,
TD-14, TD-35, TD-34, TD-32, TD-51, TD-54, TD-52, TD-53 — and implemented
`ERROR.TYPE`, moving W-ORACLE **74.2% → 90.4%**, +221 cases, **past the ≥90%
target**. Every unit is committed and pushed with CI green. Nineteen functions
reached 100%: `DATE`, `DAY`, `MONTH`, `YEAR`, `WEEKDAY`, `VLOOKUP`, `HLOOKUP`,
`XLOOKUP`, `MATCH`, `FIND`, `SEARCH`, `IF`, `IFERROR`, `IFNA`, `NA`, `AND`,
`OR`, `NOT`, `XOR` — plus the whole `__compat_literal_parser` corpus. Tests
323 → 346. The replay hashes never moved, which is the evidence that none of it
touched the op algebra.

**The through-line, worth reading before anything else.** Six of the ten were
paid by finding that the *documented* rule and the *implemented* rule differ,
and only the oracle could say which one is Excel. Four of those were the
**inverse** of the natural implementation: approximate lookup is a binary search
whose answer on unsorted data no reasonable linear scan reproduces (D-106);
criteria coerce across the text boundary where lookups do not (D-107); the
15-digit truncation happens on the source text before conversion, and truncates
rather than rounds (D-108); a condition reads `"TRUE"` and refuses `"1"`, where
the ordinary coercion does exactly the opposite (D-110). ADR-024's premise —
the binary is the spec — earned its keep, and the same should be expected of
what is left.

**The remaining 131 failures.** No single cheap cluster is left — the target
is met and what remains needs real work. In order of size: **TD-36 (`TEXT()`,
~28)** needs a number-format grammar, which is a language of its own and the
largest single item in the register; it also unblocks the residual
`__compat_1900_leap` / `__compat_serial_boundary` cases, which fail on `TEXT`,
`DATEVALUE` and `EOMONTH` rather than on date arithmetic. Then **TD-16**
(implicit intersection, which also unblocks **TD-50**), **TD-15** (the five
remaining float `near` cases), **TD-55** (3, Unicode case mapping) and
**TD-49** (locale date text, which needs a locale model rather than a fix).

**Per-item detail is in the registers, not here.** D-104 (TD-47), D-105
(TD-33), D-106 (TD-14/TD-35), D-107 (TD-34), D-108 (TD-32), D-109
(TD-51/TD-54) and D-110 (TD-52/TD-53) in `docs/43`; the paid rows and their residues in `docs/44`; every
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
- **Tests:** 352, all passing.
- **Replay hashes:** oplog `c79fa533…` · state `b58d5505…` (unchanged by the
  date work — it is additive to the op algebra).
- **Dependency budget:** kernel direct 1/5 · kernel closure 10/12 · workspace
  closure 29/40.
- **W-ORACLE:** **90.4%** overall (1,235 / 1,366) — **the ≥90% target is met**.
- **Open structural debt:** TD-46 (the tile image is built, tested, fuzz-clean
  and measured — and still not the snapshot body), and with it TD-45, TD-31 and
  TD-24's residual, all of which close together.

---

## NEXT ACTION

**The container half of ADR-036 — make `snapshots.body` the image.** The kernel
half is built and proven; this is what actually closes **TD-45** (7.86 s cold
open), **TD-31** (307 MB container) and **TD-24's residual**. Concretely:
`Snapshot::build` writes `State::write_image_with(&WinnerStamps::from_log(log))`
instead of the compacted op set; `verify` decodes and re-hashes instead of
replaying; `decode_body` and the salvage path follow. **D-101 records the two
traps**, including that `Watermark` gaps do not survive the stored encoding
without a `user_version` bump. Budget a session: it touches 18 container, 3
crash and 15 recovery tests, and those tests are the guarantee TD-30 was closed
to buy. Re-measure **W-OPEN-1M** and **W-TILE-10M** afterwards — 153.2 MB is a
projection, and a projection is not a measurement.

**The rest of step 3 is gated and should stay closed** (D-112): TD-17 and TD-44
have triggers that measurement shows are not live, and TD-37 is blocked on
packaging rather than effort.

**TD-46: the ADR is written (ADR-036) and its kernel half is built** — stamp
sidecar, `State::apply_tail`, and the refusal of a tail that predates the image,
with the loser-equivalence test ADR-036 named as load-bearing. **What remains is
the container half**: `Snapshot::build` still writes the compacted op set, so
TD-45, TD-31 and TD-24's residual are still open. D-101 records the two traps
waiting there, including that `Watermark` gaps do not survive the stored
encoding without a `user_version` bump. Superseded context: D-102 settled the encoding (delta-varint,
3.1 B/cell, 153.2 MB at 10M — passes A-001); D-103 states the remaining fork,
which is an ADR-005 question about where stamps come from, with both options
measured. The recommendation on record is (b), reconstruction at snapshot time,
streamed per tile band. Do not implement (a) without the ADR — it retires a
frozen ADR's central claim.
