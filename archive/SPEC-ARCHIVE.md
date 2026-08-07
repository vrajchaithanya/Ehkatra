# The Grid Platform — Reference Architecture Specification

**Codename:** `gridkernel`
**Version:** 1.0-RC (proposed for freeze)
**Authors:** Chief Architect · Distinguished Engineer · Spreadsheet Domain Expert · Distributed Systems Expert · Performance Engineer · API Designer · AI Platform Architect · Principal Product Engineer
**Horizon:** 20-year reference implementation
**Status:** Complete rewrite. Supersedes DOC-GRID-DESIGN.md and all prior grid documents.

---

## Table of Contents

- §1 Critical evaluation of the prior design
- §2 Architecture principles and invariants
- §3 System overview and component topology
- §4 Universal Spreadsheet Kernel (USK)
- §5 Platform Abstraction Layer (PAL)
- §6 Identity, addressing, and the infinite virtual grid
- §7 Storage engine: tiles, memory, caching
- §8 The value system
- §9 Formula engine
- §10 Dependency graph and calculation engine
- §11 Distributed execution
- §12 Collaboration: CRDT, presence, offline sync
- §13 Undo/redo architecture
- §14 Versioning, history, snapshots, recovery, autosave
- §15 Rendering engine: virtual scrolling, GPU, mobile-first
- §16 Accessibility
- §17 Internationalization
- §18 Feature layer: the full spreadsheet capability set
- §19 Import/export and format compatibility
- §20 Data connectivity: SQL, REST, GraphQL, streaming
- §21 Security architecture
- §22 Enterprise governance: multi-tenancy, RBAC/ABAC, audit, encryption, signatures
- §23 Plugin architecture and extension SDK
- §24 API surfaces: REST, gRPC, WebSocket
- §25 MCP surface
- §26 AI-native architecture
- §27 Observability, telemetry, diagnostics
- §28 Performance engineering and benchmarks
- §29 Testing strategy
- §30 Deployment architecture
- §31 Implementation roadmap
- §32 Architecture Decision Records (index)

---

# §1 Critical Evaluation of the Prior Design

Before replacing the prior design, we state precisely what survives, what is corrected, and what was missing. A rewrite that cannot articulate this is fashion, not engineering.

**Survives (was correct):** identity-based addressing with A1 as a view; tile-based sparse storage with promotion-on-conflict CRDT metadata; formula groups + range edges for the dependency graph; spill as derived overlay; errors as values with provenance; ops-as-truth with a versioned reducer; SQL-first MCP; preview-before-apply for agents; `Decimal128`; the compat/strict coercion split.

**Corrected (was under-designed):**

1. *The kernel had no formal state machine.* "Ops are truth" was asserted without defining the op algebra, its commutation laws, or its serialization. §4 defines the algebra formally.
2. *No platform abstraction layer existed.* "Compiles everywhere" is not a design; §5 defines the exact seam between the kernel and each host.
3. *The grid was bounded.* 1,048,576 × 16,384 was inherited from Excel unquestioned. §6 designs an infinite virtual grid with Excel's bounds as a compatibility *viewport*, not a kernel limit.
4. *Calculation was single-node.* 10M rows in a browser was "solved" by a server compute path that was never designed. §11 designs distributed execution properly: partitioned calculation with deterministic merge.
5. *Rendering was a paragraph.* §15 is a full retained-scene, GPU-accelerated, mobile-first rendering architecture.
6. *History was an op log with compaction.* That conflates three different products — undo, version history, and disaster recovery — which have different granularities, retention, and access patterns. §14 separates them.
7. *The feature layer (pivots, charts, validation, protection, printing) was a cut list, not a design.* §18 designs each as a kernel-native construct with a defined projection, so deferral is a scheduling decision rather than an architectural unknown.
8. *AI was a tool catalog.* An AI-native platform requires AI-legible substrate — semantic annotations, intent metadata, explanation channels — designed into the kernel, not adapters bolted onto an engine that was designed for pixels. §26.

**Was missing entirely:** gRPC and WebSocket API design; observability; plugin SDK; multi-tenancy; digital signatures; ODS/Parquet/Arrow; streaming data; Goal Seek/Solver/Scenario Manager; Flash Fill; themes; shapes/images; freeze/split panes; slicers/timelines; printing; embedded-target constraints; formal deployment topology.

**Verdict on prior document: 6.5/10 as a design sketch, 3/10 as a reference specification.** What follows is the reference specification.

---

# §2 Architecture Principles and Invariants

Principles are cheap; invariants are checkable. Each principle below carries at least one machine-verifiable invariant, enforced in CI. A principle without an enforcement mechanism is a poster.

**P1 — One kernel, everywhere.** The USK is a single `no_std`-compatible Rust library with zero platform dependencies. *Invariant I1:* the kernel crate graph contains no dependency on `std::fs`, `std::net`, `std::time`, `std::thread`, or any OS binding; enforced by `cargo-deny` graph rules and a `no_std + alloc` CI build.

**P2 — Determinism is sacred.** Identical op logs produce bit-identical state on every platform, forever. *Invariant I2:* differential replay of the full op corpus across `x86_64`, `aarch64`, `wasm32`, and `riscv64` produces identical BLAKE3 state hashes; a gate on every merge.

**P3 — Ops are the only truth.** All mutation flows through the op log; there is no side door — not for import, not for AI, not for admin repair. *Invariant I3:* the state type's mutating methods are `pub(crate)` to the op applier module alone; enforced by visibility + a lint.

**P4 — Everything is addressable, everything is explainable.** Every value, format, and derived artifact can answer *why am I this way* — the op that set it, the formula that computed it, the rule that formatted it, the actor that caused it. *Invariant I4:* the explanation API achieves 100% coverage over model constructs; a conformance test enumerates them.

**P5 — API-first, and the UI is just a client.** The UI holds no capability the API lacks. *Invariant I5:* the UI emits only public `Command` values; CI diffs the UI's command usage against the published API schema.

**P6 — AI-first, human-sovereign.** Every AI operation is previewable, explainable, attributable, and reversible as a unit. *Invariant I6:* every op group whose actor is an agent principal carries `intent`, `preview_hash`, and `explanation_ref` metadata; the relay rejects agent groups without them.

**P7 — Offline-first; the server is a peer with privileges,** not an authority over content. Any replica can operate indefinitely disconnected within the published staleness window.

**P8 — Secure by construction.** Parsing is sandboxed; formulas have no ambient authority; plugins are capability-scoped; encryption is layered per §21–22. *Invariant I8:* the fuzz corpus and adversarial op corpus run continuously; no release with an open crash.

**P9 — Horizontally scalable, vertically humble.** Every server component is stateless or partitioned; a Raspberry Pi can host a workbook and a 10,000-node cluster can host a tenant of millions.

**P10 — Observable by default, private by design.** Every subsystem emits structured traces; no telemetry ever contains cell content. *Invariant I10:* telemetry schema is typed; a content-taint lint forbids `Value` types from crossing into telemetry encoders.

**P11 — Backward compatible for 20 years.** Op semantics are immutable once shipped; new behavior is a new op type; unknown ops are preserved and re-transmitted (kernel forward-preservation rule). Files written today open in 2046.

**P12 — Plugin-first.** First-party features use the same extension points as third parties wherever feasible (charts, functions, connectors, panels are all plugins; the formula core and CRDT are not).

---

# §3 System Overview and Component Topology

## 3.1 The component map

```
╔══════════════════════════ CLIENTS ══════════════════════════╗
║  Browser (WASM)   Desktop (Tauri)   Mobile (native+UniFFI)  ║
║  CLI/headless     Embedded (no_std host)   3P apps (SDK)    ║
║        │                │                    │              ║
║  ┌─────┴────────────────┴────────────────────┴───────────┐  ║
║  │              RENDERING & INTERACTION (§15)             │  ║
║  │  scene graph · GPU compositor · virtual scroll · a11y  │  ║
║  └───────────────────────────┬───────────────────────────┘  ║
║  ┌───────────────────────────┴───────────────────────────┐  ║
║  │        UNIVERSAL SPREADSHEET KERNEL (USK, §4)          │  ║
║  │                                                        │  ║
║  │  command api → reducer → op log → state (CRDT)         │  ║
║  │  grid store (§7) · value system (§8) · formulas (§9)   │  ║
║  │  dep graph & calc (§10) · features (§18) · explain     │  ║
║  └───────────────────────────┬───────────────────────────┘  ║
║  ┌───────────────────────────┴───────────────────────────┐  ║
║  │         PLATFORM ABSTRACTION LAYER (PAL, §5)           │  ║
║  │  clock · entropy · storage · net · gpu · fonts · a11y  │  ║
║  └────────────────────────────────────────────────────────┘  ║
╚══════════════════════════════║══════════════════════════════╝
                        sync protocol (§12)
╔══════════════════════════════║═══════════ SERVER PLANE ═════╗
║  ┌──────────┐ ┌───────────┐ ┌┴──────────┐ ┌──────────────┐  ║
║  │ Gateway  │ │  Relay    │ │ Doc Store │ │ Calc Fleet   │  ║
║  │ REST/gRPC│ │ ws fanout │ │ ops+snaps │ │ (§11)        │  ║
║  │ /WS/MCP  │ │ presence  │ │ tiered    │ │ headless USK │  ║
║  └────┬─────┘ └─────┬─────┘ └────┬──────┘ └──────┬───────┘  ║
║  ┌────┴─────┐ ┌─────┴─────┐ ┌────┴──────┐ ┌──────┴───────┐  ║
║  │ AuthN/Z  │ │ Connector │ │ Search &  │ │ AI Plane     │  ║
║  │ RBAC/ABAC│ │ Service   │ │ Index     │ │ (§26)        │  ║
║  │ (§22)    │ │ (§20)     │ │           │ │              │  ║
║  └──────────┘ └───────────┘ └───────────┘ └──────────────┘  ║
║  Import/Export Fleet (sandboxed, §19) · Audit Chain (§22)   ║
║  Observability (§27) · Admin & Governance Plane (§22)       ║
╚══════════════════════════════════════════════════════════════╝
```

## 3.2 The one-sentence data flow

A gesture, API call, or AI proposal becomes a `Command`; the versioned reducer compiles it, once, at the authoring replica, into `Ops`; ops append to the local log, apply to local state, propagate via the relay to peers and to the server's headless kernel; every consumer — screen, calc fleet, search index, audit chain, AI plane — is a fold over the same log.

## 3.3 What is deliberately NOT in the architecture

No microservice decomposition of the kernel (the kernel is a library, embedded in whatever process needs it — this is the deepest structural advantage over server-oriented competitors); no dual write paths (import is ops, repair is ops); no client-specific servers (one protocol, all clients); no ORM between the op log and storage (the log *is* the storage format).

---

# §4 Universal Spreadsheet Kernel (USK)

## 4.1 Kernel anatomy

The USK is a workspace of crates with a strict dependency direction (each layer may depend only on layers above it in this list):

```
usk-types        value types, ids, errors, intervals          no_std
usk-oplog        op algebra, encoding, causal order, hashing  no_std
usk-state        CRDT state, tile store, apply/merge          no_std + alloc
usk-formula      parser, AST, analyzer, evaluator             no_std + alloc
usk-calc         dep graph, scheduler, incremental engine     no_std + alloc (+rayon w/ std)
usk-features     tables, validation, cond-format, pivots...   no_std + alloc
usk-reduce       command vocabulary, versioned reducers       no_std + alloc
usk-explain      provenance, tracing, why-engine              no_std + alloc
usk-project      render-tree & a11y-tree projection           no_std + alloc
usk-sync         sync protocol state machines                 no_std + alloc
usk (facade)     stable C ABI + WASM ABI + UniFFI bindings    per-target
```

`no_std + alloc` is not purism: it is what makes the *embedded* target (kiosks, industrial HMIs, an engine inside another application's process) real, and it is the enforcement mechanism for P1. Anything needing OS services declares a PAL trait (§5).

## 4.2 The op algebra, formally

An op is: `Op { id: OpId, deps: CausalDeps, target: Scope, payload: Payload, meta: Meta }` where `OpId = (ActorId: u128, Counter: u64)` and `CausalDeps` is a compressed vector-clock delta.

The payload taxonomy is closed per model version and **layered by CRDT type**, because "the spreadsheet" is not one CRDT but a composition of five, each with known merge semantics:

| Layer | CRDT type | Used for | Merge rule |
|---|---|---|---|
| Order | dense identity sequence (Fugue-family) | row order, column order, sheet order | interleaving-free ordered insert; tombstoned delete |
| Cell registers | MV-register w/ deterministic resolution | value, formula, per-cell format | concurrent writes → resolve LWW by (lamport, actor) but **retain losers** for conflict surfacing |
| Object maps | OR-map | tables, names, charts, validation rules, styles | add-wins observed-remove |
| Rich text | sequence CRDT | in-cell rich text, comments | standard text merge |
| Counters/sets | PN-counter, OR-set | presence, reactions, slicer state | classical |
| Blobs | content-addressed immutable store | images, themes, embedded fonts | no merge needed (immutable) |

The **retain-losers** decision on cell registers deserves its justification: pure LWW silently discards a concurrent human edit, which users experience as data loss even though it is "correct." We keep the losing write reachable for 30 days and surface it in the UI ("Bob's simultaneous edit to C4 — restore?") and via `get_conflicts` in the API. Google Sheets discards; Excel co-authoring discards; this is a visible trust win at negligible storage cost (conflicts are rare — the promotion machinery of §7 measures them).

**Commutation law (the correctness core):** for any two concurrent ops `a ∥ b`, `apply(apply(S,a),b) = apply(apply(S,b),a)`. This is proven per payload type with property tests over randomized interleavings (10⁴ per type per CI run, 10⁷ nightly) and, for the order CRDT, by reusing the published proofs of the Fugue family and testing our implementation against its reference vectors.

## 4.3 Encoding and hashing

Ops encode as canonical, deterministic CBOR (definite lengths, sorted map keys) — self-describing enough for 20-year archaeology, compact enough for the wire, and with exactly one valid encoding per op so that hashing is well-defined. Op batches compress with zstd + a trained dictionary (op streams are highly repetitive; expect 8–15×). The document state hash is a BLAKE3 Merkle tree over tiles and object maps, giving O(log n) incremental rehash per edit and cheap cross-replica divergence *localization* (find the differing tile in log time), not just detection.

## 4.4 Kernel ABI

The facade exposes three bindings over one core: a **stable C ABI** (the 20-year contract — every function versioned, caller-allocated buffers, no Rust types across the boundary); a **WASM ABI** with shared-linear-memory views for bulk data (range reads map memory, never serialize per-cell); and **UniFFI** bindings for Swift/Kotlin. The ABI is *command/query only* — no host ever touches state structs directly.

---

# §5 Platform Abstraction Layer (PAL)

The PAL is the complete, enumerated seam between the kernel and the world. The kernel declares traits; each host implements them. There are exactly eleven, and adding a twelfth requires an ADR.

| Trait | Contract | Notes |
|---|---|---|
| `Clock` | monotonic ticks + wall time *as injected data* | wall time is an input, never sampled inside the kernel (P2) |
| `Entropy` | seeded CSPRNG handed to the kernel at session start | ambient randomness is banned; all kernel randomness derives from op-carried seeds |
| `BlockStore` | get/put/delete content-addressed blocks | backends: OPFS/IndexedDB (web), file system (desktop/server), SQLite (mobile), raw flash (embedded) |
| `KeyValue` | small metadata, atomic swap | session state, caches |
| `Net` | open bidirectional stream, send/recv frames | WebSocket (web), QUIC (native), none (air-gapped/embedded) |
| `Compute` | spawn deterministic subtask, N-way parallel map | rayon (native), worker pool (web), single-thread fallback (embedded) |
| `Gpu` | submit scene, present, query caps | wgpu (native), WebGPU→Canvas2D fallback (web), framebuffer (embedded) |
| `FontSource` | resolve (family, script) → font data | bundled-first per determinism rule; OS fonts only behind explicit opt-in |
| `A11y` | push accessibility tree diff, receive AT actions | accesskit (native), shadow DOM (web) |
| `Ime` | composition events in, caret geometry out | host-native composition always |
| `SecureStore` | seal/unseal small secrets | Secure Enclave / TPM / StrongBox / keychain; software fallback with Argon2id |

**Host profiles** define which traits must be live: `Full` (all), `Server` (no Gpu/A11y/Ime), `Headless-CLI` (no Gpu/A11y/Ime/Net optional), `Embedded` (BlockStore+Clock+Entropy only — a grid engine in 400 KB for HMI/kiosk use, formulas evaluated on demand, no collaboration). The embedded profile is what makes "runs anywhere" a tested truth instead of a slogan: CI builds and runs the conformance suite on a `riscv64` `no_std` target with 16 MB of RAM.

---

# §6 Identity, Addressing, and the Infinite Virtual Grid

## 6.1 Identity space

Rows, columns, sheets, and objects carry permanent identities: `RowId`/`ColId` are **dense order identifiers** from an interleaving-free sequence CRDT (Fugue-family — chosen over naive fractional indexing because fractional indexing under adversarial or merely unlucky concurrent insertion degrades to unbounded identifier growth and permits interleaving anomalies; Fugue's tree positions are proven non-interleaving and length-bounded by insertion depth). `CellId = (SheetId, RowId, ColId)` — cells have no independent identity; they are intersections. This is a deliberate domain judgment: cell-level identity (as in some research CRDTs) makes structural operations (sort, move, fill) semantically incoherent for spreadsheet users, whose mental model is "rows and columns are things; cells are places."

## 6.2 The infinite virtual grid

The kernel imposes **no row or column limit**. The grid is a function `Identity → MaybeCell`, sparse by construction (§7); "size" is a property of *content* (the used range), not of the sheet. Consequences, each deliberate:

- Excel's 1,048,576 × 16,384 becomes a **compatibility viewport** applied at import/export and optionally enforced per-workbook for round-trip safety (`grid_bounds: Excel | Unbounded`). Exceeding Excel bounds in an `Unbounded` workbook is flagged at export time with a lossy-export warning, exactly like any other fidelity loss.
- A1 notation must extend: columns beyond `XFD` continue base-26 (`XFE`…); rows are arbitrary integers. R1C1 is unaffected.
- Display-address computation (identity order → ordinal) is served by an **order-statistic tree** over live identities per axis: O(log n) id→ordinal and ordinal→id, incrementally maintained. This structure is shared by rendering (scroll position → identities), the API (A1 parsing), and the calc engine (range enumeration), and is the single most performance-critical index in the kernel.
- "Select column" and "whole-column reference" (`A:A`) are interval references over the *axis*, not enumerations — `SUM(A:A)` folds over live tiles in the column's identity interval, cost proportional to content, not to 2⁶⁴.

## 6.3 References

As established and retained from the prior design: ranges are identity intervals with `AnchorMode`; Excel's insert/delete/shift semantics fall out structurally; endpoint deletion re-anchors inward; empty interval → `#REF!`. Three additions complete the model:

- **Cross-sheet and cross-workbook references** are `(DocRef, SheetId, Interval)`. Cross-*workbook* references resolve through the compound-document `Linked{pinned}` mechanism — snapshot by default, live opt-in — inheriting the kernel's security posture (no ambient fetch).
- **3-D references** (`Sheet1:Sheet3!A1`) are intervals over the *sheet order sequence*, merging like any other interval; inserting a sheet between endpoints includes it, matching Excel.
- **Reference rewriting on copy/fill** is a reducer-time transformation (relative anchors re-derive against the destination), so evaluation never performs address arithmetic — a property that both simplifies the evaluator and makes fills parallelizable.

---

# §7 Storage Engine

## 7.1 Tile store

The unit of storage, sync granularity, cache residency, and render fetch is the **tile**: 256 rows × 64 columns of one sheet, keyed by `(SheetId, RowBand, ColBand)` in identity space. Per-tile layout:

```
Tile {
  presence: Bitmap256x64,              // roaring-style compressed
  layout:   Homogeneous(TypeTag) | Mixed,
  values:   PackedF64 | PackedDecimal | PackedStrId | PackedBool
          | TaggedUnion,               // chosen by layout
  formulas: GroupRefs,                 // refs into FormulaGroup table (§10)
  formats:  StyleRuns,                 // run-length refs into style table
  crdt:     TileCausalSummary          // ~24 B typical
          | PromotedCells(map)         // per-cell metadata, concurrency only
  merkle:   Blake3Node,
}
```

Verified budget (10M-cell numeric workbook): values 80 MB, presence 1.2 MB, CRDT summaries ~15 KB → **~81 MB total** vs ~201 MB naive — the margin that makes wasm32 (practical ~2 GB) comfortable. Promotion to per-cell CRDT metadata occurs only on true concurrency and is measured; the Spike-1 gate requires <1% promotion under realistic multi-author load, and telemetry tracks the fleet-wide promotion rate permanently (a rising rate is an early-warning signal that tile granularity needs revisiting — an explicitly instrumented assumption, per P10).

## 7.2 Memory management

A workbook is a **working set over a tiered store**, never a loaded blob:

- **Tier 0** — hot tiles, decoded, in-memory, LRU-with-pin (viewport tiles, dirty tiles, and tiles in the active calc closure are pinned).
- **Tier 1** — warm tiles, encoded+compressed in memory (~10× smaller); decode on touch (~µs).
- **Tier 2** — cold tiles in the PAL `BlockStore` (disk/OPFS), content-addressed.
- **Tier 3** — remote (server doc store); fetched by need, prefetched by scroll-velocity and dependency-closure prediction.

Eviction runs against an explicit budget negotiated with the host (`MemoryBudget { soft, hard }` — e.g., 256 MB soft on mobile, 4 GB on desktop). On mobile memory-pressure signals, the engine sheds Tier 0→1→2 deterministically and can rebuild from Tier 2 + op tail after process death (§14). Strings are interned per-workbook with refcounted GC at compaction; styles are interned flyweights (a workbook has millions of styled cells and dozens of styles).

## 7.3 Caching (unified theory)

All caches in the platform obey one invariant: **caches are folds over the op log with a recorded watermark** — never independently mutable state. A cache entry is `(key, value, valid_at_version)`; invalidation is dirty-interval intersection against ops since the watermark. This single rule covers the shaping cache, computed-value cache, display-address cache, query-plan cache, render-tile cache, and the server's projection caches, and it is why the system can be aggressively cached *and* correct under concurrency — the question "is this cache stale" always has a mechanical answer.

---

# §8 The Value System

Retained from the prior design and completed. The lattice:

```
Value = Blank | Bool | Number(f64) | Decimal(Decimal128) | Text(StrId)
      | Date | DateTime(tz?) | Duration | Error(Kind, OriginTrace)
      | Array(dims, packed) | Reference(Range) | Rich(EntityRef) | Lambda(Closure)
```

Additions over the prior design:

- **`Rich(EntityRef)`** — typed entity values (stock, currency, geography, custom records from connectors §20, product entities from plugins §23). An entity is a schema-tagged record in the object map; the cell holds a reference; dot-access (`A1.Price`) is a formula operation. This is the substrate for "data types" à la modern Excel, and it is in the *kernel*, because retrofitting entities later would fracture the type system.
- **`Lambda(Closure)`** — lambdas are first-class values (assignable to names, passable to functions, storable in cells), with lexical capture over `LET` bindings and names. Closure capture is by *value snapshot* at definition — capture-by-reference over a mutable grid under concurrency is semantic quicksand; snapshot semantics are explainable, deterministic, and match how users reason about `LAMBDA` in Excel.
- **Number formats are a pure display function** `format(Value, FormatCode, Locale) → String`, implementing the full Excel format-code grammar (sections, conditions, colors, fractions, scaling, elapsed time). Formatting never alters the stored value (the class of bugs where display rounding feeds back into computation is banned by construction; comparison under the 15-digit compat rule happens in the *evaluator's* compat profile, not in the formatter).

Coercion (`compat` for imported, `strict` for native), `Decimal` promotion rules, date systems, and the enumerated Excel bug-compatibility catalog (1900 leap year, 15-digit display, ±date-system, SUM accumulation) carry forward unchanged from the prior design — they were correct.

---

# §9 Formula Engine

## 9.1 Pipeline

```
source text → lexer → parser (Pratt) → CST (lossless, for tooling)
           → AST → binder (names, tables, refs → identities)
           → analyzer (types, volatility, dependencies, spill shape)
           → plan (typed expression DAG, common-subexpression numbered)
           → evaluate (interpreter now; JIT slot reserved)
```

The **lossless CST** is not a luxury: it powers formula formatting/pretty-printing, refactoring (rename a table column → rewrite formulas preserving user whitespace/comments), precise error carets, and the AI plane's formula-explanation anchoring (§26). The **binder** is where text meets identity: `A1` binds to `(RowId, ColId)` under the current view; `Table1[Amount]` binds through the table's column map; undefined names bind to `#NAME?` thunks that rebind automatically if the name is later defined (live rebinding is an op-driven invalidation like any other, per §7.3).

## 9.2 Function architecture

Functions are registered declaratively with a rich signature — arity, per-arg type & coercion class, volatility tier, spill behavior, aggregation identity (for distributed fold, §11), differentiability slot (for Solver, §18.9), and a documentation/i18n block (function names localize; the *storage* form is canonical English, display form is locale, exactly like Excel):

Catalog organization: **Core-200** (v1: arithmetic, logical, text, date/time, lookup — `XLOOKUP`, `INDEX/MATCH` family, information, math/trig, aggregation incl. `SUMIFS` family, dynamic array set — `FILTER/SORT/UNIQUE/SEQUENCE/…`, `LET`, `LAMBDA` + helpers `MAP/REDUCE/SCAN/BYROW/BYCOL/MAKEARRAY/ISOMITTED`), **Extended-250** (statistical incl. distributions, financial incl. depreciation & securities, engineering incl. complex numbers & bessel, database `D*`, cube), **Compat tail** (legacy aliases: `CONCATENATE`, `FORECAST`, banker's rounding variants, etc.). Every function: conformance-tested against oracle vectors captured from real Excel (see §29), including error propagation, coercion, and profile-specific bugs.

**User-defined functions**, three ascending tiers sharing one namespace: named `LAMBDA`s (pure, sandboxless — they are formulas); **WASM plugin functions** (§23 — capability-scoped, deterministic-mode enforced: no clock/entropy/net imports unless declared volatile-external, in which case they execute only through the Calculation Authority like any Tier-3 function); server-side connector functions (§20). The volatility taxonomy from the kernel design (pure / volatile-materialized / external-authority) governs all three uniformly.

## 9.3 Spill engine

Spill is a derived overlay (retained: no ownership ambiguity, `#SPILL!` is computed, convergent under concurrency). Completed with: spill-range references (`A1#`) binding to the anchor's *current* spill extent as a dynamic dependency (dependents re-dirty when the extent changes shape — the dep graph records extent-edges distinctly from value-edges to avoid over-invalidation); implicit-intersection operator `@` for compat-mode formulas; and legacy CSE array import (imported `{=...}` arrays become modern dynamic arrays under a compat flag that preserves their fixed extent).

---

# §10 Dependency Graph and Calculation Engine

## 10.1 Graph representation

Nodes are **formula groups** (shared R1C1 pattern over an identity region — 1M filled cells ≈ hundreds of groups), single formulas, names, tables (as producers), volatile bindings, external bindings, and spill extents. Edges are **range-granular**, resolved through a per-sheet interval index (R-tree over identity space): "who reads what I wrote" is a log-time stab query, never a materialized cell-edge set (measured: ~0.1 MB grouped vs ~96 MB naive at 1M formulas).

## 10.2 Incremental engine

Edit → dirty-interval set → stab query → transitively mark dirty groups (with early cutoff: a recomputed node whose value is unchanged — by hash — stops propagation; in real workbooks early cutoff prunes the majority of recalc cascades, e.g. a changed input that doesn't change a rounded subtotal) → topological level assignment over the dirty subgraph (Pearce-Kelly incremental topo order, maintained not recomputed) → **level-parallel evaluation** via PAL `Compute` (rayon / workers), with per-group cost models steering work-stealing granularity (a 100k-row group SUM splits; a scalar formula doesn't).

Guarantees: **interruptible** (remote ops or user edits arriving mid-calc checkpoint the frontier; undirtied cells always hold last-consistent values — the UI never blocks on calc), **resumable**, **deterministic** (order-pinned reductions per kernel FP rules: Neumaier, no FMA contraction, row-major identity-order traversal), and **fair** (calc runs at background priority; keystroke-to-paint never waits on it — visible-viewport dirty cells are prioritized first, a user-perceivable ordering choice that matters more than total throughput).

Cycles: Tarjan SCC over the dirty subgraph; `#CIRC!` unless iterative calc is enabled (then: Calculation Authority only, pinned iteration/epsilon, materialized results — retained from kernel design).

## 10.3 Calc consistency model

Named explicitly (the prior design left it implicit): the platform provides **eventual calc consistency with bounded staleness and monotone convergence** — after op quiescence, all replicas' computed values converge to the same fixpoint (P2); during activity, a replica may briefly display values from different calc generations, but each cell is marked with its generation and the UI renders in-flight cells with a subtle recalc affordance rather than stale-as-fresh. API reads carry `calc_watermark` so programmatic consumers can demand `at_least(version)` or `converged` semantics per request — an honest, checkable contract instead of Excel's "calculation may be pending" mystery state.

---

# §11 Distributed Execution

For workbooks beyond single-node budgets (browser 2 GB practical; mobile less) and for server-side heavy compute (10⁸-cell models, Monte Carlo scenario sweeps, org-wide query federation §20).

## 11.1 Partitioned calculation

The server Calc Fleet runs headless USK instances. A large workbook's dependency graph is partitioned by **min-cut over the group graph** (METIS-family partitioning; spreadsheet dep graphs are overwhelmingly local — data flows down columns and across summary sheets, so cuts are small in practice). Each partition owns its tiles and evaluates its levels; cross-partition edges exchange **value frontiers** (compact `(range, values, generation)` messages) over the fleet's internal mesh, in generation lockstep per level to preserve determinism (P2 across machines, verified by the same differential-replay gate run fleet-wide).

Aggregation functions declare algebraic identities in their signatures (§9.2): `SUM/COUNT/MAX/MIN/AND/OR` are commutative-monoid folds → tree-reduced across partitions; order-sensitive reductions (per compat profile) fall back to owner-evaluates. Non-partitionable constructs (whole-graph SCCs, iterative calc) pin to one node — measured, logged, surfaced in diagnostics (§27) as a modeling-quality signal to the user ("this workbook has a 40k-cell circular cluster; here's where").

## 11.2 Client/server calc split

A thin client (mobile, embedded, or a browser viewing a 10⁸-cell model) may run in **projection mode**: it holds the op log tail + viewport tiles + a *computed-value subscription* — the calc fleet streams value frontiers for the subscribed ranges. Edits still author ops locally (offline-capable for data entry); full local calc resumes automatically when the working set fits. The mode switch is transparent, per-workbook, and surfaced in diagnostics — never a different product, always the same kernel deciding where folds run. In Strict-E2EE workspaces the fleet cannot participate (ciphertext ops); projection mode is unavailable and the capability matrix says so (§21).

---

# §12 Collaboration: CRDT Sync, Presence, Offline

## 12.1 Sync protocol

Replica ↔ relay, over PAL `Net` (WebSocket/QUIC), four message families: `HELLO` (auth, doc, vector-clock summary, wire/model/capability negotiation per kernel versioning rules), `OPS` (zstd batches, causally contiguous per actor), `NEED/GIVE` (anti-entropy: Merkle-tree comparison localizes divergent tiles in O(log n) — the same Merkle from §4.3, doing double duty), `SNAP` (compacted snapshot + watermark for fresh joins and stale replicas past the 180-day window, which then rebase unsynced local ops through `migrate_ops`). The relay is a **fanout and retention service, not a merge authority** — it never interprets payloads (and cannot, under E2EE tiers). Backpressure: per-actor token buckets (rate + bytes) enforced at the relay (tombstone-amplification defense, retained), with server-assigned batch quotas for import bursts.

## 12.2 Presence and awareness

Ephemeral (never in the op log): cursors, selections, viewport hints, typing indicators, "calculating…" states — a separate CRDT-free gossip channel with 30 s TTL, coalesced at the relay. Selection presence is identity-based (survives concurrent structural edits — Bob's highlight of "the March column" stays on that column when Alice inserts one before it, a small correctness detail users notice constantly in competitors).

## 12.3 Offline

Fully symmetric with online (P7): ops queue in the local `BlockStore`; reconnection replays with causal metadata so merge is ordinary CRDT merge (no "conflict resolution dialog" as a modal event — conflicts are the §4.2 retained-losers surfacing, asynchronous and non-blocking). The published contract: **180-day staleness window; beyond it, snapshot-rebase flow; local unsynced ops are never silently dropped** — if `migrate_ops` cannot rebase an op (removed capability), the op is exported to a user-visible "unmerged changes" ledger with cell-level content, because silently losing a user's offline week is the one unforgivable sync sin.

---

# §13 Undo/Redo Architecture

Retained core (selective undo by inverse-synthesis-against-current-state; per-type semantics; structural undo blocked-and-narrowed when it would destroy others' work; redo as undo-of-undo; the whole history in the op log, auditable). Completed with:

- **Range-compressed undo groups**: a 100k-cell paste stores `(op-id interval per actor-counter run)`, not 100k ids — O(runs), not O(cells).
- **Undo scopes**: per-user × per-workbook by default; *per-session* view scoping so two windows of one user don't fight; **agent groups are first-class scopes** — "undo everything agent-X did in this session" is one operation resolving to a set of labeled groups (P6).
- **Cross-feature atomicity**: a Command touching cells + a table definition + a chart (e.g., "convert range to table with chart") is one undo group spanning object maps and tiles — group boundaries are Command boundaries, always, which is why the reducer is the only place groups form.
- **The stack is durable** (survives process death via `KeyValue`), bounded (200 groups / 30 days), and its truncation is *explained in the UI* at the boundary ("older changes: see version history") — undo and history hand off to each other visibly, not confusingly (§14).

---

# §14 Versioning, History, Snapshots, Recovery, Autosave

Three products over one log, separated because their granularity, retention, and access patterns differ:

**Undo** (§13): seconds-to-days, op-group granular, per-user, in-session mental model.

**Version history**: the human-meaningful timeline. **Named versions** (explicit user/API act, immutable ref), **auto-milestones** (session boundaries, import events, pre-agent-batch checkpoints — the platform *always* auto-milestones before an agent `apply_edits`, making "restore to before the AI touched it" a guaranteed one-click even if the undo window lapsed), and **branches**: a branch is a fork of the op log at a version — same CRDT machinery, merge = op replay with ordinary concurrent-merge semantics + a preview diff (what-if modeling, month-end close workflows, agent sandboxes §26). History views are diffs computed from the log (cell diff, structural diff, object diff), attributed to actors, filterable by region/actor/time — "who changed this cell, when, from what" is a log query (P4), rendered as *blame view* per range.

**Snapshots & recovery**: compacted state images every N ops / T minutes (content-addressed, structurally-shared via tile Merkle identity — a snapshot after 100 edits shares ~all tiles with its predecessor, so snapshot cost is O(dirty), enabling *minutes-granular* point-in-time recovery at low storage cost). Recovery = nearest snapshot + op tail replay; RPO ≤ 1 s of acked ops (relay-durable), client-crash RPO = last local fsync (≤ 250 ms of typing); RTO = snapshot decode + tail, budgeted <5 s for 100 MB workbooks. Disaster restore, legal-hold export, and tenant migration all consume this same snapshot+log format — one recovery story, tested by continuous automated restore drills in production (§27), because an untested backup is a hope.

**Autosave** is not a feature; it is the absence of one. There is no dirty bit and no Save. Ops are durable locally within 250 ms (batched fsync) and relay-acked opportunistically. "Save As" maps to *named version* or *branch*; export (§19) is explicitly a *projection to a file*, not saving. The Save-button generation gets an affordance (⌘S → creates a named version, toast: "Version saved — this workbook also saves continuously"), because ripping out a 40-year habit without a bridge is product malpractice.

---

# §15 Rendering Engine

## 15.1 Architecture: retained scene, damage-driven, GPU-composited

```
state + view params
  → usk-project: VIEW MODEL (visible-window row/col metrics, cell display
     runs, format resolution, cond-format results, selection/presence layers)
  → SCENE GRAPH (retained): layers z-ordered:
     [grid-lines] [fills/cond-fills] [content: text runs, sparklines,
      entity chips] [borders] [objects: charts/images/shapes] [frozen panes]
      [selection] [presence] [editors/overlays: IME, comments, filter UI]
  → COMPOSITOR (PAL Gpu / wgpu / WebGPU): glyph-atlas text, instanced fills
     & borders & grid-lines, damage-rect repaint only
```

Text shaping via the kernel text stack (rustybuzz + bundled fonts, shaping cache keyed `(StrId, style, script)` — spreadsheet content is massively repetitive; expect >95% hit rates); numbers use a fast path (pre-shaped digit glyph runs per style — the majority of painted cells are numeric, and this single optimization roughly halves paint cost in dense sheets). Everything the compositor draws comes from the view model; the view model is a cache-with-watermark (§7.3), so damage = dirty-interval intersection — the render loop never walks the document.

## 15.2 Virtual scrolling over an infinite grid

Scroll position is `(anchor RowId/ColId, pixel offset)` — identity-based, so concurrent structural edits never teleport the viewport (ordinal-based scroll positions, as in every DOM-virtualized competitor, jump when rows are inserted above). Row/col pixel metrics come from the order-statistic tree augmented with cumulative extents: pixel→identity and identity→pixel in O(log n) with variable heights. Scrolling streams tiles (Tier 0 pin viewport ± velocity-scaled margin; prefetch direction-weighted); unresolved tiles render as content-shaped placeholders for ≤1 frame in practice. Budget: 120 Hz scroll on desktop-class, 60 Hz mid-tier mobile, zero layout jank ≥ p99 frames.

## 15.3 GPU strategy and fallback ladder

wgpu everywhere → WebGPU (browser) → Canvas2D fallback (feature-detected, same scene graph, CPU raster) → server-side raster (headless: PDF/print/thumbnails/screen-reader-free environments). The scene graph is the contract; backends are swappable (P1). GPU is used for *compositing and instancing*, not for layout (layout determinism lives in the kernel; pixels may differ per rasterizer — the metrics-not-pixels rule).

## 15.4 Mobile-first interaction model

Not a shrunk desktop: touch-first hit targets and a dedicated interaction layer — drag-handle selection with magnifier, momentum scroll with axis-lock, pinch zoom (re-layout at discrete zoom steps, GPU-scale between), a **context-aware input bar** (numeric keypad for numbers, formula bar with token-level autocomplete chips, date wheel for dates — driven by the column's inferred/declared type), row/col gesture grammar (tap header selects, drag resizes, long-press structural menu), and **projection mode** (§11.2) as the default for giant workbooks on mobile. Feature parity is *capability* parity (every Command reachable) not *chrome* parity (the ribbon is not ported; a command palette + contextual sheets are).

## 15.5 Freeze/split panes, in-cell editors

Frozen panes are additional scene-graph viewports with clamped scroll axes sharing the same view model (no duplicated state); splits likewise with independent scroll. The in-cell editor is a real host text field overlaid at caret geometry (IME correctness, §5 `Ime`), swapped invisibly with the painted cell — the one place DOM/native-widget rendering intrudes into the canvas world, by design (the hybrid a11y/IME architecture).

---

# §16 Accessibility

P0 constraint, not a layer. One **semantic accessibility tree** is projected by `usk-project` alongside the render tree — same source, two sinks — and delivered via PAL `A11y`: accesskit (Win UIA / macOS NSAccessibility / Linux AT-SPI / mobile) and a virtualized shadow DOM (web). The tree exposes grid semantics natively (row/col headers as labels, cell coordinates, formula presence, error state, table/region landmarks, spill provenance, comment threads) so a screen reader announces "B4, Revenue March, $12,400, formula, spilled from B1, 1 comment" — not "cell."

Commitments: WCAG 2.2 AA + EN 301 549 + Section 508 with a maintained VPAT; 100% keyboard reachability with Excel-compatible default keymap (muscle memory is an accessibility *and* migration feature) + full remapping; high-contrast and forced-colors honored by the theme engine (§18.6); 400% zoom reflow (structural zoom steps); reduced-motion honored by all animation; screen-reader CI via accesskit tree assertions + scripted NVDA/VoiceOver runs per release; a11y conformance is a *release gate* with the same standing as the performance budget. Collaboration a11y: presence and conflict surfacing have non-visual channels (live-region announcements, sound-optional).

---

# §17 Internationalization

Locale is a **display and input concern; the kernel is locale-free** (P2: no locale-sensitive behavior in evaluation or storage). The i18n layer (icu4x) owns: number/date/currency parsing and formatting per locale (including locale decimal/argument separators in the formula *editor* — `=SUM(1,5;2,5)` in German — parsed to canonical storage form); localized function names and error names (storage canonical, display localized, round-trip lossless); calendar systems beyond Gregorian (display-level; serial storage unchanged); RTL sheets (column order mirroring as a *view* transform — identity order is unchanged, which makes RTL a pure projection concern and keeps formulas/refs sane); CJK/complex-script text via the kernel text stack (grid text is Tier-1 scope: single-line + wrapped cells, no pagination — materially simpler than the document case); locale-aware collation for sort/filter (ICU collation, *recorded in the sort descriptor* so a sort is reproducible on every replica regardless of its UI locale); Lunar/fiscal calendar functions in Extended catalog. Shipping tiers with test-corpus gates: T1 Latin/Cyrillic/Greek + full bidi (Arabic/Hebrew UI + data) + horizontal CJK; T2 Indic + Thai/Lao/Khmer/Burmese breaking + vertical CJK text-in-cells.

---

# §18 Feature Layer

Every feature below is specified as: *model* (what lives in the CRDT object maps), *projection* (how it renders/computes), *ops* (how it changes), which is what makes each independently schedulable without architectural risk.

**18.1 Workbook & sheets.** Workbook = object map root: sheet sequence (order CRDT), names, styles, themes, protection, connections, metadata. Sheet ops: insert/delete/rename/move/hide/color/copy (copy = bulk op batch with identity remapping, streamed for large sheets).

**18.2 Tables.** Model: name, header/total rows, body interval, column list (ColId + name + type contract + calculated-column formula), style ref, filter/sort state. Projection: structured refs bind through the column map (survive reorder); calculated columns are formula groups auto-extended on row append; totals row binds aggregation per column. Tables are the query layer's relations (§20) and the AI plane's primary semantic unit (§26).

**18.3 Named ranges & named functions.** OR-map name → (interval | constant | formula | lambda), workbook- or sheet-scoped, with rename refactoring through the CST rewriter (§9.1).

**18.4 Data validation.** Rules are identity-anchored objects over intervals: type constraints, list (static or range-driven → dropdown), custom formula, input/error messages, severity (reject/warn/info). Evaluated in the calc engine as ordinary dependents (a validation rule is a formula group producing booleans) — so validation is incremental, parallel, explainable, and API-queryable (`validate` reports all current violations with provenance). Reject-severity is enforced at the *reducer* (the op never forms), warn/info at projection.

**18.5 Conditional formatting.** Rules = formula groups producing style deltas; projection merges deltas in rule priority order into style runs at view-model build. Color scales/data bars/icon sets are vector rules over range statistics (min/max/percentiles maintained incrementally as range aggregates in the dep graph — so a 100k-row color scale doesn't rescan on every edit).

**18.6 Rich formatting, themes, styles.** Interned style flyweights (font, fill, borders, alignment, number format, protection flags); named cell styles as style inheritance chains; **themes** = palette + font scheme + effects, referenced (never baked) by styles, so retheming is O(1) and OOXML theme round-trip is structural. Dark mode is a theme transform with author-intent preservation (explicit colors flagged vs theme-derived colors remapped).

**18.7 Comments, notes, rich text in cells.** Threaded comments = rich-text CRDT sequences anchored to CellId with @-mentions (principal refs) and resolve state; legacy notes = single rich-text blobs; in-cell rich text = sequence CRDT runs (the one place cells contain a nested CRDT).

**18.8 Hyperlinks, images, shapes, icons.** Links: cell attribute (URL/internal ref/mailto), *never auto-fetched* (§21). Images/shapes/icons: blob-store refs + object-map placement records (anchor modes: two-cell / one-cell / absolute — Excel's three), rendered as scene-graph objects; images support in-cell placement (`IMAGE()` returns an entity value). Shape geometry: a bounded vector model (preset geometries + freeform paths) compatible with DrawingML's preset set.

**18.9 Analysis tools.** *Goal Seek*: bounded secant/Brent search driving one input against one target — runs as a branch (§14) evaluated in the calc engine, applies as one Command with full provenance ("set by Goal Seek: target…"). *Solver*: an integration point, not a solver — the model (objective, variables, constraints) is a first-class object; execution delegates to pluggable engines (bundled LP/NLP for small problems; enterprise engines via connector) on the Calculation Authority; solutions apply as previewable op batches. *Scenario Manager*: scenarios are named branches over a declared changing-cell set with a comparison projection — subsumed by branch machinery rather than a parallel bespoke feature (one concept, taught once).

**18.10 Pivot tables.** Model: source (table/range/connection), field list (rows/cols/values/filters, aggregation per value field, calculated fields/items), layout options. Projection: a **materialized incremental aggregation** — the pivot engine maintains group trees over the source via the same dirty-interval machinery (source edit → affected group paths recompute, not full rebuild); output projects into a read-only grid region with drill-through (double-click → op-free query listing contributing rows). Pivot *charts* bind charts (18.11) to the pivot projection. Slicers/timelines: shared filter state objects (OR-set of selected members / date interval) that multiple pivots/tables/charts subscribe to — cross-filtering is dependency propagation, nothing special.

**18.11 Charts & sparklines.** Charts are *plugins on a first-party contract* (P12): a chart = declarative spec (type, series bound to intervals/table columns/pivot fields, axes, legends, styles) + a renderer plugin producing scene-graph vectors. Bundled: line/bar/column/area/pie/donut/scatter/bubble/combo/stock/waterfall/funnel/histogram/box/treemap/sunburst/map. Series data flows through the dep graph (a chart is a dependent; it re-renders incrementally like any cell). Sparklines: in-cell chart micro-specs, same pipeline, cell-attribute placement.

**18.12 Sort & filter.** Sort is a *reorder op batch* over the order CRDT (identity permutation — formulas anchored to rows travel with them, the classic broken-reference-after-sort bug is structurally impossible), recorded with its collation descriptor (§17). Filter is *view state* (hidden-row projection), per-view not per-document — two users can filter differently without fighting (Excel's shared-filter wars, solved) — with an opt-in "shared filter" mode that makes it document state when teams want it.

**18.13 Flash-Fill-style extraction.** A *local, deterministic* program-synthesis engine (FlashFill-family string-transformation DSL) proposing a derivation from examples — surfaced as: preview → accept as *values* or as a *generated formula* (the latter is the differentiator: the inference is explainable and lives on as a maintainable formula, not magic pasted text). Runs client-side; no AI plane dependency (works air-gapped); the AI plane can *also* propose richer derivations through the same preview contract (§26).

**18.14 Protection & digital signatures.** Sheet/workbook protection = capability masks in the object map (range-level edit permissions with principal lists — enforced at the *reducer* for local UX and at the *relay* for actual security; client-side-only enforcement is theater and is named as such in the model: protection ≠ security; §22 ACLs are security). Digital signatures: detached signatures over (content Merkle root + version ref) with X.509/eIDAS support — a signature attests a *version*; any subsequent op visibly invalidates it (the version-ref makes "signed then edited" mechanically detectable, unlike Excel's fragile signature model).

**18.15 Printing.** Print = a projection to paged media: print areas (interval sets), page setup (size/orientation/margins/scaling incl. fit-to-N), headers/footers with field codes, row/col repeat titles, page-break preview as an overlay layer, rendered by the layout engine to the server/local raster backend → PDF (§19) or platform print services via PAL. Pagination here is the *simple* case (grid, not flowed text) and shares the deterministic-metrics rule: identical breaks everywhere.

**18.16 Templates.** A template = a workbook snapshot + parameter manifest (typed placeholders, locked regions, setup wizard spec) instantiated by op replay with identity refresh — templates are data, not a format variant; org template galleries are governance objects (§22).

---

# §19 Import/Export and Format Compatibility

All parsing/serialization runs in the sandboxed import/export fleet (or local sandbox process) per the kernel security design: IR-only output, schema revalidation, resource caps, active-content quarantine (verbatim-preserve inert; strip-and-vault executable; never re-emit by default).

| Format | Direction | Fidelity contract |
|---|---|---|
| XLSX/XLSM | in/out | Semantic-lossless round-trip; unknown parts preserved; macros quarantined (XLSM in → XLSX-equivalent + vaulted vbaProject; re-emission is a permissioned, audited export option). Fidelity is a *published per-release number* over the corpus. |
| XLS/XLSB (legacy) | in only | Best-effort with per-file fidelity report; tightest sandbox (Firecracker one-shot for pathological files) |
| ODS | in/out | Full model mapping; the second first-class format (procurement requirement in exactly our sovereign wedge) |
| CSV/TSV | in/out | Streaming; import wizard with *type-inference preview* and strict-mode defaults (the gene-name bug is an import-time decision surfaced to the user, never silent) |
| Parquet/Arrow | in/out | Columnar tiles map near-directly; Arrow IPC is also the *zero-copy bulk API representation* (§24) — one columnar story from disk to wire to tile |
| JSON/XML | in/out | Schema-mapped (tables ↔ arrays of records); JSON Lines streaming |
| PDF | out | Print pipeline (§18.15); PDF/A archival profile |
| HTML | in/out | Out: semantic tables + inline styles (clipboard-grade). In: table extraction |
| Markdown | in/out | Pipe tables; out includes formula-as-comment option |
| Google Sheets | in | Via export formats; direct API import through connector (§20) |

Clipboard is a format boundary too and gets the same rigor: writes HTML+plain+native; reads with the import sandbox (pasted content is untrusted input — clipboard is a real attack channel).

---

# §20 Data Connectivity

## 20.1 Connections as governed objects

A `Connection` = provider + location + auth ref + refresh policy + scope, stored in the object map, credentialed via server-side secret store (client never holds connector secrets — kernel Tier-3 rule). All external data flows through the **Connector Service** on the Calculation Authority: egress-allowlisted, rate-limited, audited, schema-validated at the boundary, results materialized as ops with provenance `(connection, query, fetched_at, actor)` — refresh is an attributed event, reproducible and diffable like any edit.

## 20.2 Providers

SQL (Postgres/MySQL/SQLServer/Oracle/SQLite/Snowflake/BigQuery/Redshift/Databricks via a driver plugin contract); REST (OpenAPI-described: pagination strategies, auth schemes, JSONPath/JMESPath shaping); GraphQL (typed queries, persisted-query allowlists for governance); Files (S3/GCS/Azure/SFTP/HTTP with format layer §19); Streaming (Kafka/Kinesis/NATS/webhooks/SSE): a stream binds to a table as an **append or upsert flow** with watermarking, windowed retention policy, and rate-adaptive batching (ops batched to the grid at bounded frequency — a 10k-msg/s stream becomes ≤4 Hz of op batches over changed intervals; the grid is a *materialized view over the stream*, and the dep graph makes downstream formulas/charts live). Real-time dashboards fall out rather than being a product.

## 20.3 The query engine (both directions)

The in-platform SQL engine (agent/API-facing, §25) treats sheets/tables/pivots as relations over tile-columnar payloads (vectorized execution, predicate pushdown into presence bitmaps and homogeneous tiles). **Query federation** pushes down to connected sources where the connector supports it (a `query` joining a local table to a Snowflake table plans the remote fragment remotely) — with an explicit cost/row-limit governor and per-connection governance policy (§22).

---

# §21 Security Architecture

Inherits the platform kernel security design in full; grid-specific statement:

**Threat model additions (grid-specific):** formula-based exfiltration (killed structurally: no ambient network in evaluation — Tier-3 only, egress-allowlisted); malicious op injection by a permitted collaborator (schema+bounds validation on receive; adversarial-op fuzz corpus); pivot/query cross-ACL leakage (query engine enforces *row-level* read ACLs of the querying principal — a pivot or `query` result can never contain data its reader couldn't read directly; enforced in the planner, tested adversarially); clipboard/import injection (sandboxed paste, formula-injection guards on CSV import — leading `=`/`+`/`-`/`@` neutralization per OWASP CSV-injection guidance, applied on *export* too); denial-of-calc (per-actor calc-cost quotas; hostile formula patterns — deep recursion, combinatorial spills — hit evaluator resource governors with attributable kill reports).

**Encryption tiers** (Standard / Managed-E2EE with HSM-quorum Compliance Principal / Strict-E2EE) apply per workspace with the published capability matrix (fleet calc, server query, connectors, and search degrade per tier exactly as the matrix states — the grid adds: projection mode unavailable under Strict, client-side query only).

**Formula/plugin/AI sandboxing:** evaluator has no I/O by construction (`no_std` core is the enforcement); plugin functions run in WASM with declared capabilities (§23); AI plane operates through the same Command/preview gates as any client (§26) — the AI is *not* a privileged subsystem.

---

# §22 Enterprise Governance

**Multi-tenancy.** Tenant = hard isolation boundary: per-tenant encryption keys (KMS-wrapped, per-tenant rotation), partitioned storage prefixes, per-tenant relay lanes and calc-fleet quotas (noisy-neighbor isolation by scheduling class), optional dedicated-cell deployment (single-tenant fleet) and BYO-storage/BYO-KMS for sovereignty. Tenant metadata (users, groups, policies) lives in the control plane, never in workbooks.

**RBAC/ABAC.** Two composable layers: *roles* (viewer/commenter/editor/structure-editor/owner/admin — coarse, human-assignable) and *attribute policies* (CEL-expressed conditions over principal attrs, resource attrs, context: device posture, network, time, data classification — e.g., "ranges classified `payroll` are readable only by group `finance` on managed devices"). Enforcement points: relay (op admission — the *security* boundary), API gateway (request scope), query planner (row/range-level reads, §21), reducer (UX-level early denial). Range-level classification is a first-class attribute (18.14 protection is UX; this is security) and flows into exports (a classified range exports redacted unless the exporting principal clears policy — data doesn't launder through XLSX).

**Audit.** Every admissible event (op groups with actor/intent/device, auth events, permission changes, exports, AI actions with explanation refs, admin acts) → append-only hash-chained log, externally anchored (RFC 3161 / transparency log), tenant-queryable, SIEM-streamable (OCSF-shaped), with guaranteed-complete coverage tied to I3 (mutation without audit is impossible because mutation is ops and ops are audited).

**Retention/legal.** Legal hold pins snapshots+logs immutably per matter; retention policies schedule compaction/erasure; **crypto-erasure** implements right-to-be-forgotten for E2EE tiers (destroy the epoch keys covering the subject's data after tombstoning) — with the honest documentation that plaintext-tier erasure is deletion+compaction, provable via the audit chain.

---

# §23 Plugin Architecture & Extension SDK

**One extension mechanism, five extension points**, all WASM (Component Model) with capability manifests — first-party features ride the same rails (P12; charts and connectors are literally plugins in-repo):

1. **Functions** — custom formula functions (deterministic-mode default; declared-volatile → Authority execution).
2. **Connectors** — data providers implementing the §20 driver contract (server-side WASM, egress per manifest).
3. **Renderers** — chart types, cell renderers (entity chips, custom sparklines), producing scene-graph vectors (no raw pixel/DOM access — the scene contract is the sandbox).
4. **Panels & commands** — task panes and command-palette verbs via a declarative UI schema (rendered by the host shell in host-native widgets — plugins get *capability*-scoped UI, not a webview free-for-all; a webview escape hatch exists behind an org-admin allowlist because pragmatism, but the default path is declarative).
5. **Automation** — event-driven scripts (on-edit/on-open/on-schedule/on-webhook) replacing macros: WASM, capability-scoped, *previewable and undoable like agents* (an automation's op batches are labeled groups — the VBA malware model is not reproduced, and "what did this script change" is a first-class question).

SDK: TypeScript-first (compiled to WASM via componentize-js) + Rust + Python (componentized); local dev harness with a headless kernel + hot reload; a typed `usk-sdk` API mirroring the Command/query surface (plugins cannot bypass it — I3 applies); marketplace with org-level allowlisting, publisher signing, static capability review, and runtime resource quotas (CPU/memory/egress per invocation, enforced by the WASM runtime). Versioning: plugins pin an SDK major; the platform guarantees two-major support with deprecation telemetry.

---

# §24 API Architecture: Three First-Class Layers

## 24.1 The layer model

The platform exposes **three API layers, each a first-class citizen with its own audience, its own optimization target, and its own contract** — strictly stacked, so every higher layer is implemented in terms of the layers beneath it and can never express something the lower layers cannot:

```
Layer 2 — SEMANTIC APIs (AI-native, MCP)          audience: agents & copilots
   optimizes: context economy, safety, explainability
   implemented on: Layer 3 (query/bulk reads) + Layer 1 (edit primitives,
   composed into previewed, labeled command groups)
────────────────────────────────────────────────────────────────────────
Layer 3 — BULK APIs (high-performance data plane)  audience: data platforms, ETL
   optimizes: throughput, zero-copy, columnar alignment
   implemented on: direct tile-store access for reads (bypassing per-cell
   materialization, never bypassing ACLs) + Layer-1 command batches for writes
────────────────────────────────────────────────────────────────────────
Layer 1 — CELL-LEVEL APIs (developer control plane) audience: developers, plugins
   optimizes: precision, completeness, ergonomics
   implemented on: the Command vocabulary directly — Layer 1 IS the public
   face of the command bus (P5); nothing sits beneath it but the reducer
```

The layering discipline, stated as invariants: **(a)** every Layer-1 call desugars to exactly one Command (auditable 1:1); **(b)** every Layer-2 mutation is a composition of Layer-1 Commands in one labeled group (an agent action is always explainable as "these cell-level operations"); **(c)** Layer-3 writes compile to the same op batches a Layer-1 loop would produce — the fast path changes the *encoding and transport*, never the semantics, so a Parquet import and a million `write_cell` calls are distinguishable only by speed; **(d)** ACL, audit, undo-grouping, and version preconditions apply identically at all three layers — there is no privileged layer.

All three layers ride all three transports (REST for ergonomics and the long tail; gRPC for systems integration, bidirectional op streaming, and Arrow Flight; WebSocket for live subscription and low-latency interactive submission), with OpenAPI/protobuf schemas generated from the Command vocabulary so documentation cannot drift from the engine. Auth is uniform: OIDC/OAuth2; tokens carry tenant + principal + scope set (workbook/range-granular per §22); short-lived intent-declared agent tokens (P6); mTLS for fleet peers.

## 24.2 Layer 1 — Cell-Level Developer APIs

Full-precision control, nothing hidden. The catalog (each entry: the Command it desugars to, in REST + gRPC + SDK bindings):

**Cell & range:** `read_cell(ref, view)` / `read_range(ref, view, max_cells)` — views: `values|formulas|formatted|full|types`; `write_cell(ref, value)` / `write_range(ref, values[][])`; `get_formula(ref)` / `set_formula(ref, text)` (CST-validated, bind errors returned with carets); `clear(ref, scope: contents|formats|all)`.

**Structure:** `insert_row(at, count)` / `delete_row(at, count)` / `insert_column(at, count)` / `delete_column(at, count)` — positional args resolve to identities at the server against the caller's `expected_version`, so a concurrent insert elsewhere cannot displace the target; `merge_cells(range)` / `unmerge_cells(range)` (merged regions are anchor-cell + span attributes; merge with occupied non-anchor cells returns the would-be-lost values in the error, forcing the caller to decide).

**Formatting & annotations:** `get_style(ref)` / `set_style(ref, style_patch)` (patch semantics against the interned flyweights); `get_validation(ref)` / `set_validation(ref, rule)`; `get_comment(ref)` / `set_comment(ref, thread_op)` (append/resolve/delete thread operations).

**Clipboard semantics:** `copy(range) → clip_id` / `cut(range) → clip_id` / `paste(clip_id, target, mode: all|values|formulas|formats|transpose)` — clipboard objects are server-side value+formula+format snapshots with reference-rewrite performed at paste time by the reducer (relative-anchor arithmetic, exactly like interactive fill); `cut`+`paste` moves identity-anchored dependents (references follow the move, matching Excel).

**Session:** `undo(scope?)` / `redo(scope?)` — group-granular, per the §13 scoping rules; `get_history(since)`, `get_conflicts(range?)` (§4.2 retained losers).

Ergonomics & performance contract: single calls are optimized for latency (<20 ms server-side p95); every Layer-1 write accepts `If-Match`/`expected_version` + `Idempotency-Key`; a `batch[]` envelope turns any sequence of Layer-1 calls into one atomic labeled undo group with one round-trip — which is precisely what Layer 2's `apply_edits` uses. A naive client looping `write_cell` 10,000 times works correctly (rate-limited, each call auditable); the documentation and SDK actively steer that pattern to `batch[]` or Layer 3, but the platform never *breaks* the naive pattern — first-class means first-class.

## 24.3 Layer 3 — High-Performance Bulk APIs

The data-plane surface, columnar end-to-end (Arrow is simultaneously the wire format, the file format, and near-aligned with homogeneous tile payloads — §20.3, ADR-020 — so the fast path is genuinely zero-copy from tile to socket):

**Table I/O:** `read_table(name|range, projection?, predicate?) → Arrow stream` (predicate pushdown into presence bitmaps and homogeneous tiles); `write_table(name, arrow_stream, mode: replace|append|upsert(keys))` — upsert diffs against current rows and emits *minimal* op batches (unchanged rows produce no ops, keeping history and sync clean).

**Streaming:** `stream_rows(source, cursor)` / `stream_columns(source, cols, cursor)` — resumable NDJSON (REST) or Arrow Flight (gRPC) with tile-aligned cursors; `stream_changes(since)` — the op-log tail as a typed change feed (insert/update/delete/structure events with before/after images per subscriber option), the CDC surface for downstream systems.

**Batch mutation:** `batch_update(ops_spec)` — the high-throughput write envelope: columnar payloads + command descriptors, compiled server-side into op batches at ~10⁶ cells/s per core, one atomic group per call (or chunked-with-manifest for >10⁷-cell loads, each chunk atomic, the manifest recording the whole load for one-click revert).

**Dataset lifecycle:** `import_dataset(source, format: csv|parquet|arrow|json|xlsx, mapping, mode)` / `export_dataset(scope, format, options)` — async jobs on the sandboxed import/export fleet with progress resources; import produces a fidelity/type-inference report before commit (strict-mode: the report *is* the preview).

**Query execution:** `execute_sql(sql, params) → Arrow|JSON` — the §20.3 engine, read-only by default; write-back CTAS-style forms (`CREATE TABLE ... AS`) gated by scope `structure:write`; `execute_dataframe(plan)` — a Substrait-encoded logical plan (the lingua franca for dataframe engines: Polars/pandas/Spark clients compile to Substrait, the platform executes — no bespoke dataframe dialect to maintain); `execute_arrow(compute_spec)` — Arrow-native compute over ranges without SQL; `execute_parquet(path_or_upload, sql)` — federated query joining uploaded/lake Parquet against live workbook tables (the "join my sheet to the lake" workflow, one call).

Throughput budgets (CI-gated, reference hardware): `read_table` ≥ 2 GB/s per node sustained; `batch_update` ≥ 10⁶ cells/s/core; `import_dataset` 1 GB CSV < 30 s including inference; `stream_changes` fanout lag < 500 ms p95 at 1k subscribers.

## 24.4 Cross-layer routing

The platform actively routes callers to the right layer rather than punishing wrong-layer use: Layer-1 responses to large-range reads include a `hint: bulk` header with the equivalent Layer-3 call; the SDKs expose all three layers behind one client object with automatic promotion (a `range.values` read above a size threshold transparently switches to Arrow transport); Layer-2 tools *internally* select Layer 3 for reads and Layer-1 batches for writes, so agents inherit optimal routing without knowing the layers exist. Rate limits are layer-aware: per-call limits on Layer 1, per-byte/per-row limits on Layer 3, blast-radius policies on Layer 2 — each layer throttled in the dimension it can actually abuse.

---

# §25 MCP Surface (Layer 2 — Semantic, AI-Native)

The agent-facing contract, **implemented entirely on Layers 1 and 3** per §24.1: reads route through the query engine and bulk plane; mutations compose Layer-1 Commands into previewed, labeled, reversible groups. Agents *default* to this layer — it is what their tokens are scoped to unless a developer explicitly grants Layer-1/3 scopes — because it minimizes context, maximizes safety, and keeps every action explainable; but an agent with granted scopes may drop to Layer 1 for surgical work (the layers are capabilities, not castes). Design law: **schemas and answers, never grids; preview before mutation; everything attributable and reversible.** Tools (stable, versioned, with JSON-Schema I/O; workbook/sheet resources expose *descriptions*, bounded tokens):

**Orient:** `describe_workbook` (sheets, tables, names, connections, mode, version, health flags), `describe_sheet` (used range, header inference, per-column type/null/cardinality stats, 5 sample rows), `summarize` (natural-language workbook/sheet/range summary built from the semantic layer §26.1 + structure + recent history — bounded tokens at any scale), `search` (values/formulas/comments, scoped, ranked, located).

**Read/analyze:** `query` (read-only SQL, relations = tables/sheets/pivots, row+byte caps with explicit truncation, ACL-planned per §21), `read_range` (capped escape hatch), `explain_formula` (CST-anchored natural-structure explanation feed: parse tree + binding table + per-node values — the *substrate* for LLM explanation, not canned prose), `trace_error` (origin trace + propagation path + suggested probes), `find_dependencies` / `find_dependents` (range-granular graph queries, depth-limited), `impact_analysis` (given hypothetical edits → affected ranges/sheets/named-summaries, error deltas, chart/pivot invalidations — *without* applying; this is `preview_edits`' analysis half exposed for planning), `validate` (all current validation violations with provenance).

**Mutate:** `preview_edits` (scratch-branch application → impact report + preview_hash), `apply_edits` (atomic labeled group, `expected_version` precondition, auto-milestone before large/agent batches per §14, requires preview_hash above configurable blast-radius thresholds), `generate_formula` (NL intent + target context → candidate formulas *with explanations and test evaluations against sample rows* — returns candidates, never writes; writing is `apply_edits`), `suggest_cleanup` (data-quality profile → ranked issue list — mixed types, trailing whitespace, near-duplicate keys, broken references, inconsistent formats — each with a previewable fix batch; the AI-plane data-cleaning capability §26.3 exposed as a tool), `undo` / `redo` (group- and agent-session-scoped).

**Lifecycle/data:** `import` / `export` (sandbox fleet, async with progress resource), `stream_changes` (op-log tail subscription as MCP resource updates — agents observe without polling).

**Guardrails:** every response labels cell-derived text as untrusted data (prompt-injection posture); tool tokens are workbook- and range-scoped, short-lived, intent-declared; per-token rate and blast-radius policy (org-configurable: e.g., >500 changed cells requires preview; >10k requires human approval via an approval resource); all agent groups auto-milestoned and one-click reversible. These guardrails are *relay-enforced*, not tool-suggested (P6/I6).

---

# §26 AI-Native Architecture

AI-native means the *substrate* is AI-legible and AI operations are platform citizens — not a chat panel bolted to a grid.

**26.1 The semantic layer.** The kernel maintains, as ordinary CRDT objects: column semantic types (inferred + user-confirmed: "email", "ISO date", "USD amount", entity-typed), table/range descriptions, workbook intent docs, formula-group intents (optional user/AI-authored "this computes churn"), and unit annotations. This layer is (a) what `describe_*` serves, (b) what grounds NL→SQL/formula generation, (c) itself previewable/editable/versioned like all state. Inference runs locally (cheap classifiers over column stats); confirmation is human; the confidence state is explicit.

**26.2 The AI plane.** A server-side (or client-side, per tier) orchestration service: model-pluggable (BYO endpoint per tenant — sovereignty), tool-calling against the MCP surface *with the same tokens and guardrails as external agents* (the house AI is not privileged; it is dogfooding P6), with per-tenant policy (allowed models, data-egress rules — under E2EE tiers the plane runs client-side or in attested enclaves per the platform matrix).

**26.3 Capability inventory** (each = MCP composition + UX surface): formula generation (`generate_formula` candidates with sample-row test evals → user/agent picks → `apply_edits`); formula & dependency explanation (`explain_formula` feed → prose at user's level, hover-to-expand per node); error tracing (`trace_error` → guided fix flow with previewed candidate repairs); workbook summarization & auto-documentation (semantic layer + structure + history → living README object, regenerated on drift, diffable); data cleaning/normalization (profiling → issue list → previewed transform batches — each fix an explainable op group, never a silent "cleaned"); NL querying (NL→SQL against `query` with the semantic layer as schema context; answer carries the SQL for inspection — trust through transparency); chart recommendation (data-shape heuristics + intent → chart spec candidates, previewed); impact & scenario analysis (`impact_analysis` + branch-based scenarios: "model a 10% churn increase" → branch, edits, comparison projection, narrated deltas); AI edit preview/undo/audit (inherited platform-wide: preview_hash chain, labeled groups, auto-milestones, session-scoped undo, audit entries with explanation refs).

**26.4 Explainability contract.** Every AI action stores: intent (the ask), evidence (ranges/queries read — a *taint record* of what the model saw), proposal (the previewed diff), decision (who approved: human click, policy auto-approve, or agent-of-agent chain), and explanation ref (generated rationale). `why did this change` on any cell reaches this record through the op group (P4 closed over AI). This record is also the *safety* mechanism: the evidence taint record is what makes exfiltration-via-agent auditable.

---

# §27 Observability, Telemetry, Diagnostics

**Tracing:** OpenTelemetry end-to-end; a Command carries a trace context from gesture → reducer → relay → fleet → dependents' recalc → paint, so "why was that edit slow" is one distributed trace. Kernel emits spans through a PAL-shimmed no_std tracing facade.

**Metrics (SLO-bearing):** op admission latency, sync propagation p95, calc generation lag, tile-fetch latency per tier, promotion rate (§7.1's watched assumption), conflict rate, relay fanout depth, per-tenant quota utilization, fleet partition balance, crash-free session rate. Product telemetry (feature usage, function frequency — feeding the catalog roadmap) is schema-typed, consented, content-free (I10: `Value` types cannot reach telemetry encoders — a compile-time taint check).

**Diagnostics:** an in-product doctor (`Help → Diagnostics`): workbook health report (calc SCC clusters, volatile density, oversized styles, promotion hotspots, dep-graph depth — the "why is my workbook slow" answer Excel never gives, rendered actionably: "83% of recalc time is these 3 formula groups; consider…"); support bundles (traces + structure metadata, *content-free by construction*, user-inspectable before send); replay bundles for bug reports (op-log slice + state hash, redactable) — a reported calc bug arrives as a deterministic reproduction (P2 pays for itself in support economics).

---

# §28 Performance Engineering & Benchmarks

Budgets are CI gates on reference hardware (desktop-class, 2-gen-old mid-tier Android, wasm32 in headless Chromium; embedded budget separately):

| Metric (p95) | Target |
|---|---|
| Keystroke→paint (edit, 10k-cell sheet) | <16 ms |
| Keystroke→paint (edit triggering 10k-cell recalc) | <50 ms (viewport-priority ordering) |
| Scroll frame time | <8.3 ms desktop / <16.6 ms mobile, zero jank p99 |
| Cold open, skeleton+viewport, 1M-cell workbook | <1.5 s |
| Full recalc, 100k dependent cells | <200 ms (8-core) |
| Incremental recalc, single edit, deep chain (10k levels) | <30 ms |
| `query` grouped aggregate over 1M rows | <500 ms |
| Fleet recalc, 10⁸ cells, 32 partitions | <10 s |
| Sync propagation same-region | <150 ms |
| Op local durability | <250 ms |
| Memory, 10M numeric cells | <400 MB total working set |
| WASM bundle (core, compressed) | <12 MB |
| Embedded profile flash/RAM | <2 MB / <16 MB |
| Battery: 1 h active mobile editing | <8% device battery |

**Benchmark suite** (public, versioned — a competitive asset per se): micro (op apply, tile codec, shaping cache, interval-index stab), macro (LibreOffice calc-perf corpus + captured real-world anonymized-structure workbooks + adversarial: 10⁶-level chains, 10⁵ volatile cells, pathological spill lattices), soak (24 h collaborative fuzz at 50 actors), and competitive harness (same operations scripted against Excel/Sheets via their automation surfaces, published honestly). Regression policy: any budget breach blocks release; any >5% p95 regression on macro suite requires sign-off with a tracked debt entry.

---

# §29 Testing Strategy

The pyramid, with the unusual layers explicit:

1. **Oracle conformance** — function vectors *captured from real Excel* (a harness drives actual Excel through COM across versions, recording inputs/outputs/errors for the full catalog × edge-case grid; the captured corpus is the specification of compat profile). ODS/format fidelity similarly corpus-driven, published per release.
2. **Algebraic property tests** — CRDT commutation/convergence (10⁴ CI / 10⁷ nightly randomized interleavings, structural ops weighted), undo properties (undo∘do = identity on own scope), cache-watermark coherence, reference-rewrite round-trips.
3. **Differential replay** — full op corpus, 4 architectures, bit-identical state hashes (the P2 gate).
4. **Fuzzing, continuous** — format parsers (structure-aware, grammar-based), the op applier (adversarial logs), the formula parser, the query planner (SQLancer-style logic fuzzing: TLP/NoREC oracles against the query engine), API surfaces (schema fuzz).
5. **Simulation testing** — FoundationDB-style deterministic cluster simulation for the distributed plane: relay + fleet + flaky clients under scripted partitions/reorders/crashes, seeds replayable (the distributed-systems bugs live here, and only simulation finds them pre-production).
6. **Integration/E2E** — golden-workbook suites per feature, cross-client (WASM+desktop+mobile drivers), collaboration scenarios (recorded multi-actor scripts), a11y tree assertions + scripted screen-reader runs, MCP contract tests (every tool × truncation/error/permission paths).
7. **Performance** — §28 gates.
8. **Chaos & restore drills** — production-grade: continuous automated restore verification, relay-failover drills, fleet-partition kills.

Coverage philosophy: the op applier, reducer, formula evaluator, and codecs carry mutation-testing targets (≥85% mutants killed), because these are the modules where a silent bug corrupts user data forever.

---

# §30 Deployment Architecture

**Topologies (one artifact set, four shapes):** (1) *SaaS multi-tenant* — K8s: gateway (Envoy), relay (stateful sets, doc-sharded by consistent hash, Raft-replicated retention), doc store (Postgres metadata + S3-compatible blob/op storage), calc fleet (autoscaled headless pods, doc-affinity scheduling), connector/AI/import fleets (sandboxed node pools), per-region cells with tenant pinning (data residency). (2) *Dedicated cell* — same charts, single tenant. (3) *Self-hosted compact* — the whole server plane in one binary (embedded SQLite + local blob store + in-proc relay/fleet), Docker/VM, air-gap capable, offline-license capable; scales to ~200 concurrent editors on one box — and it is the *same code*, so on-prem is never a fork. (4) *Edge/embedded* — kernel-only integrations (no server plane).

**Upgrade discipline:** rolling with wire/model version negotiation (N−2), canary rings, self-hosted LTS channel (18-month support, quarterly security backports), compaction-rewriter migrations (never in-place mutation), and documented rollback (op-log formats are forward-preserved, so a rollback re-serves older snapshots + preserved ops — rollback is a first-class tested path, not a prayer).

---

# §31 Implementation Roadmap

**Phase 0 — Proof (10 wks):** the three gating spikes (CRDT-at-scale w/ promotion-rate measurement; wasm32/Safari memory+perf; canvas a11y with screen readers) + oracle-capture harness bootstrap. *Gate: all three green or architecture revisions issued.*

**Phase 1 — Kernel (2 quarters):** usk-types/oplog/state/reduce, tile store, order CRDT, differential-replay rig, Core-200 formula engine + dep graph + incremental calc, XLSX/CSV import-export in sandbox, C/WASM ABI. *Exit: headless CLI edits, calcs, round-trips real workbooks under all gates.*

**Phase 2 — Product alpha (2 quarters):** rendering engine + virtual scroll + web shell; collaboration (relay, sync, presence, offline); undo/history/snapshots; tables/validation/cond-format/sort/filter; REST+WS APIs; MCP orient+read tools. *Exit: 50-editor collaborative alpha, published fidelity number.*

**Phase 3 — Wedge GA (2 quarters):** full MCP surface + preview/apply guardrails; AI plane v1 (generation/explanation/NL-query); gRPC+Arrow Flight; connectors (SQL+REST+files); desktop shell; RBAC/ABAC+audit+SSO; self-hosted compact; a11y VPAT; charts core set. *Exit: paying design partners on the agent-native + sovereign wedges.*

**Phase 4 — Depth (ongoing):** pivots/slicers, Extended-250, mobile shells (native+UniFFI), streaming connectors, plugin SDK GA + marketplace, distributed fleet calc, E2EE tiers, printing/signatures, ODS/Parquet, i18n T2, Solver/scenarios, embedded profile hardening.

Sequencing law: nothing in Phase ≥2 may begin against an unproven Phase-0 assumption; budget gates apply from Phase 1 day one (retrofitted performance is a myth).

---

# §32 ADR Index

| ADR | Decision | Over | Because |
|---|---|---|---|
| 001 | Ops-as-truth; reducer compiles Commands once at author | dual command/op authority | divergence impossible by construction |
| 002 | Fugue-family order CRDT | fractional indexing | non-interleaving proof; bounded ids |
| 003 | Cells are intersections; identity lives on axes | cell-level identity | matches user structural mental model; keeps sort/move coherent |
| 004 | Infinite grid; Excel bounds as compat viewport | inheriting 1M×16k | 20-year horizon; bounds are a file-format property, not a kernel one |
| 005 | Tile store, per-tile CRDT metadata + promotion | per-cell metadata | 120 MB→15 KB; the wasm32 feasibility margin |
| 006 | Retain-losers MV-registers, 30-day conflict surfacing | pure LWW | perceived data loss is trust death; cost negligible |
| 007 | Formula groups + range edges + interval index | cell-granular dep graph | graph smaller than data; log-time stabs |
| 008 | Spill as derived overlay | materialized spill cells | no ownership ambiguity; convergent |
| 009 | Volatile materialization + Calculation Authority | ambient evaluation | replica convergence; audit; kills exfiltration channel |
| 010 | Decimal128 alongside f64; strict/compat coercion modes | f64-only | currency correctness; Excel-bug fidelity where needed |
| 011 | Lossless CST retained | AST-only | refactoring, carets, AI anchoring |
| 012 | Lambda closures capture by value snapshot | capture by reference | deterministic; explainable; matches user model |
| 013 | Canonical CBOR + BLAKE3 Merkle | protobuf/custom binary | one valid encoding; incremental hashing; archaeology-grade |
| 014 | no_std kernel + 11-trait PAL | std-everywhere + cfg | embedded target real; P1 machine-enforced |
| 015 | Undo/history/recovery as three products over one log | one history mechanism | different granularity/retention/access patterns |
| 016 | Filters are view state (opt-in shared) | document-state filters | ends shared-filter wars; matches intent |
| 017 | Sort = identity permutation op | value rewrite | references survive; sort is undoable/attributable |
| 018 | Pivots as incremental materialized aggregation | recompute-on-open | live pivots at 10⁶ rows; drill-through free |
| 019 | Charts/connectors/panels as first-party plugins | privileged internals | P12; SDK dogfooding; marketplace credibility |
| 020 | Arrow as bulk wire + file + tile alignment | JSON everywhere | zero-copy path disk↔wire↔memory |
| 021 | Scene-graph renderer + GPU compositing + hybrid a11y/IME overlay | DOM grid / pure canvas | 120 Hz + screen readers + IME all at once |
| 022 | Identity-anchored scroll position | ordinal scroll | viewport stability under concurrent edits |
| 023 | AI plane uses public MCP with standard guardrails | privileged AI subsystem | P6; auditability; no second security model |
| 024 | Oracle-captured Excel conformance corpus | spec-from-docs | the docs lie; the binary is the spec |
| 025 | Deterministic cluster simulation for distributed plane | integration tests only | reorder/partition bugs are unfindable otherwise |
| 026 | Relay-enforced agent guardrails | tool-layer suggestions | security at the admission boundary, not etiquette |

---

## Closing statement

This specification is complete in the sense that matters: every subsystem has a design, every design has a justification, every justification names its alternative, and every load-bearing assumption is either proven, gated by a Phase-0 spike, or instrumented for permanent measurement in production. The kernel is one library that runs from a 16 MB embedded target to a 32-partition calc fleet; the op log is the single source of truth for state, sync, undo, history, audit, and AI attribution; and the agent surface is not a bolt-on but the same command vocabulary every human client uses — which is the structural bet that makes this platform the reference implementation for the next twenty years of spreadsheets.
