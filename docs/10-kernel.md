# 10 — Kernel Architecture (USK)
Status: Approved · Owner: Distinguished Engineer · Normative: yes · Carved from SPEC §4–5

## Crate layering (dependency direction downward only)
`usk-types → usk-oplog → usk-state → usk-formula → usk-calc → usk-features → usk-reduce → usk-explain → usk-project → usk-sync → usk (facade: C ABI + UniFFI)`. All `no_std + alloc`; a CI build enforces it (this is the mechanism that keeps the kernel portable to web/mobile later — ADR-014/027).

## Op algebra
`Op { id: (ActorId u128, Counter u64), deps: causal delta, target, payload, meta }`. Payloads are a closed taxonomy per model version, layered over five CRDT types: order sequences (rows/cols/sheets — Fugue-family, non-interleaving), MV-registers with deterministic LWW resolution **retaining losers 30 days** (conflict surfacing — ADR-006), OR-maps (objects), rich-text sequences (comments/in-cell), content-addressed immutable blobs. Commutation (`a∥b ⇒ apply order irrelevant`) is the correctness core: property-tested continuously, **TLA+ model-checked for the order CRDT and register** (Q1 deliverable, docs/35).

## Encoding & hashing
Canonical deterministic CBOR (one valid encoding per op); zstd + trained dictionary on batches; BLAKE3 Merkle tree over tiles/objects → incremental state hash, O(log n) divergence localization, structurally-shared snapshots. The state hash is the determinism gate (P2): differential replay across x86_64/aarch64/wasm32 must match bit-for-bit, every merge.

## Reducer contract
`reduce_vN(Command, &Snapshot) → Vec<Op>` — pure, versioned, immutable once shipped. Fenced by lints banning ambient time/randomness/hash-order iteration/locale. Remote replicas never see Commands — reduction happens once, at the author — so cross-version divergence is impossible by construction (ADR-001). Server publishes `min_reducer_version`; older clients go read-only, never wrong.

## PAL (10 traits for desktop-first; `Gpu`,`A11y`,`Ime` unused on server)
Clock · Entropy · BlockStore · KeyValue · Net · Compute · Gpu · FontSource · A11y · Ime · SecureStore. Windows/macOS implementations are the reference-quality ones (DPAPI/Keychain for SecureStore; DirectWrite/CoreText only for *enumeration* — layout metrics always come from bundled fonts, docs/31). Adding a trait requires an ADR.

## Forward compatibility (the 10-year rule)
Op semantics immutable; new behavior = new op type; unknown ops are preserved, causally ordered, hashed as opaque, and retransmitted. Ops declare `Cosmetic` (old clients keep editing) or `Structural` (document goes read-only with banner). Schema evolution = additive types + compaction-time rewriter; max offline staleness 180 days, then snapshot-rebase with a never-drop guarantee for local unsynced ops (unmergeable ops export to a user-visible ledger).
