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
(now **85.7%**, 1,171 / 1,366 cases — up from 74.2% this session), followed by
the structural debt around the tile image, then Q2's shell.

**Session 20 paid TD-33, the largest cluster.** Excel's two date systems are now
an explicit `DateSystem` on the evaluation context (**D-105**): +126 cases, and
the 1904 corpus went **20.0% → 87.7%** because the engine had had no 1904 mode
at all — every serial was off by the same 1,462, so nearly every case failed for
one reason. `DATE`, `DAY`, `MONTH`, `WEEKDAY` and `YEAR` are now **100% in both
corpora**. Six rules, none of which follows from the others, all measured from
the Excel COM capture rather than from documentation. Locale date text is
deliberately excluded and filed as **TD-49** — `"15/03/2024"` means different
days on differently-configured hosts, so the corpus cannot settle it.

**Session 20 also paid TD-14 and TD-35** (**D-106**): +31 cases, and `VLOOKUP`,
`HLOOKUP`, `XLOOKUP`, `MATCH`, `FIND` and `SEARCH` all reach **100%**.
Approximate match is the **binary search Excel actually runs** — over the
unsorted keys `30,10,50,10` it answers the row holding **10**, where a linear
"largest key ≤ needle" scan answers 30. TD-14's refusal was right until vectors
existed and wrong the moment they did; it is the clearest case in the corpus for
ADR-024's premise that the binary is the spec. Three `INDEX` cases still diverge
and all three are **TD-16**, implicit intersection — attributed there, not here.

**Session 20 closed TD-47 and found it was never true.** `tools/gates.ps1` was
recorded as requiring PowerShell 7 and aborting on this 5.1-only host. Measured:
it runs top to bottom in **one invocation, every gate green, exit 0**, under
Windows PowerShell 5.1.26100.8875 — bare, fully redirected, and dot-sourced.
Nothing needed fixing; what it needed was *running*. Details in **D-104**.

The 5.1 `NativeCommandError` mechanism is real and was reproduced, but it fires
only when a native command's stderr is **merged into the pipeline**, which
`gates.ps1` never does. So the trap was latent, one redirection operator away.
Two new gates now stand where the assumption was, and they run first:

- a runtime probe — a native command that writes to stderr and exits 0 must not
  derail the run;
- a static grep — no `.ps1` under `tools\` may merge native stderr into the
  pipeline.

**The defect underneath is the register, not the shell.** TD-47 is the second
row to assert an unverified host condition that measurement then refuted; TD-48
(filed in session 18 under TD-28's number) was the first. D-078's lesson was not
enough, because both rows named a clearing condition and neither was ever run.
D-104 strengthens it: **a debt row asserting something about this host is not
filed until the command demonstrating it has been executed and its output pasted
into the row.** Register IDs are append-only — a paid row is struck through and
keeps its number forever.

---

## CURRENT STATE

- **Gates:** ALL GREEN via one command — `powershell -File tools\gates.ps1`
  (also `pwsh -File tools/gates.ps1`). Shell compat · fmt · clippy `-D warnings`
  · tests · no_std wasm32 kernel build · dep budget · supply chain (cargo-deny)
  · differential replay native == wasm32 · purity/host-isolation greps.
- **Tests:** 335, all passing.
- **Replay hashes:** oplog `c79fa533…` · state `b58d5505…` (unchanged by the
  date work — it is additive to the op algebra).
- **Dependency budget:** kernel direct 1/5 · kernel closure 10/12 · workspace
  closure 29/40.
- **W-ORACLE:** **85.7%** overall (1,171 / 1,366).
- **Open structural debt:** TD-46 (the tile image is built, tested, fuzz-clean
  and measured — and still not the snapshot body), and with it TD-45, TD-31 and
  TD-24's residual, all of which close together.

---

## NEXT ACTION

**Compatibility debt, in the order docs/44's W-ORACLE table ranks it by measured
case count** — the ranking is the point of having a runner, so follow it rather
than re-guessing:

1. ~~TD-33 — date semantics~~ **PAID (session 20)**, +126 cases.
2. **TD-36 — `TEXT()`** (~28), unimplemented; needs the number-format grammar.
   Now the largest remaining cluster, and it also unblocks the residual
   `__compat_1900_leap` / `__compat_serial_boundary` cases that TD-33 left —
   they fail on `TEXT`, `DATEVALUE` and `EOMONTH`, not on date arithmetic.
3. ~~TD-14 — approximate-match lookup~~ **PAID (session 20)**, with ~~TD-35~~.
4. **TD-34 — the `COUNTIF`/`SUMIF` criteria sub-language** (~20). Next: it
   reuses the wildcard matcher TD-35 just landed, so it is the cheapest
   remaining cluster.
5. **TD-32 — `compat_parse_15`** (~14), parse-time literal truncation.
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
