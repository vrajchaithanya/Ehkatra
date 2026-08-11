Session start per CLAUDE.md. You are an autonomous build session. Never ask
questions; never wait. Take recorded decisions from docs\43-decision-register.md,
or best-engineering-judgment defaults and record them. No git, ever. Nothing
outside C:\Users\velag\Desktop\Ehkatra.

1. Read CLAUDE.md, then the last ~120 lines of PROGRESS.md, then BOOTSTRAP.md,
   docs\06-design-principles.md and docs\07-solo-operating-model.md. Read the
   module doc(s) for the work you are about to do.

2. Verify the baseline before trusting it: run
   `powershell -ExecutionPolicy Bypass -File tools\gates.ps1`
   If any gate is red, repair it FIRST and record the repair in PROGRESS.md.

3. Do the first item in PROGRESS.md's NEXT ACTION list. Depth with proof beats
   breadth without it: if the item is larger than one session, land a coherent
   part of it, prove that part, and hand the rest off precisely. Scope down
   rather than leave two unverified layers stacked (DP-C4).

Rules that outrank convenience:
  - PROFILE BEFORE BUILDING A PERFORMANCE FIX. This project is five-for-five on
    the register's named cause not being the measured cause. If the measurement
    contradicts the debt entry, record that and stop — that is a successful
    session, not a failed one.
  - A number without a workload id (docs\38) is invalid. New numbers land in
    MEASUREMENTS.md with machine context. "M1" means Machine 1, the Windows
    i7-10750H host defined at the top of MEASUREMENTS.md.
  - New op types are permanent (DP-A5). Adding one needs an ADR before code.
  - Never claim a fidelity or conformance number our own code alone produced
    where a real-Excel oracle is available (D-123).
  - Checkpoint into .checkpoints\<nn>-<short-name>\ before risky multi-file
    work; keep the last 3 checkpoint folders.

4. Finish green. Run the full gate suite and do not stop until it prints
   ALL GATES GREEN. If the replay hashes move, they must move for a reason
   docs\29 requires, and you must say so explicitly in PROGRESS.md and
   MEASUREMENTS.md. Never leave the tree red — restore from the checkpoint if
   you cannot reach green.

5. Record: append a session entry to PROGRESS.md in the established narrative
   style (what was built and why, defects found and how they were found, the
   numbers, test delta, gates status), and update CURRENT STATE and NEXT ACTION.
   Durable decisions go in docs\43, new debt in docs\44, new numbers in
   MEASUREMENTS.md. End the entry with the exact next action so the next
   session — which remembers nothing — continues seamlessly.
