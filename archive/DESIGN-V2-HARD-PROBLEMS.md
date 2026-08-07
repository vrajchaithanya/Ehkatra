# OpenSuite — Design v2: The Hard Problems, Solved

**Supersedes:** the corresponding sections of the v1 architecture proposal
**Scope:** the three areas the review flagged as under-designed — the architect's five load-bearing questions, the security problems compressed into single clauses, and the text-layout/i18n underestimate
**Status:** proposed for freeze

---

## 0. What changed

| Problem | v1 said | v2 says |
|---|---|---|
| Command bus vs CRDT | Both exist | Ops are truth; Commands compile to Ops **once**, at the author |
| Collaborative undo | Not mentioned | Selective undo via inverse synthesis against *current* state; structural undo is conflict-aware |
| Recalc determinism | Not mentioned | Three-tier value lattice + explicit Calculation Authority; volatiles are **materialized**, not evaluated |
| Compound documents | Not mentioned | Universal `Embed` node + projection trait + ACL-aware render cache |
| API/schema versioning | "stable API" | Three independent version axes + mandatory op forward-preservation + compaction-time rewriter |
| Key management | One clause | MLS (RFC 9420) group per document; devices as leaves; escrow as a *visible member* |
| E2EE vs compliance | Contradiction | Three explicit tiers with an honest capability matrix; Compliance Principal under HSM dual control |
| Parser safety | "fuzzing" | Process/instance isolation, IR-only output, schema revalidation, resource caps |
| Opaque blob preservation | "preserve everything" | Four-class ingest policy; **active content is quarantined, never re-emitted by default** |
| Text layout | "harfbuzz, better than Word" | 10-stage pipeline, `icu4x` + `rustybuzz`, explicit Word-compat profiles, metrics-not-pixels determinism |
| IME / accessibility | Absent | Hybrid rendering: canvas visuals + live shadow accessibility tree (`accesskit` on native) |
| i18n | Absent | Three shipping tiers, each with a test corpus gate |

---

# PART I — The Architect's Five Questions

## I.1 Command bus vs CRDT: which is the source of truth?

### The resolution

**Operations are the source of truth. Commands are an authoring language that compiles to Operations. Compilation happens exactly once, on the originating replica.**

Three strictly separated layers:

**Layer 1 — Intent (`Command`).** User-level and semantic: `InsertRowsBelow { sheet, anchor, count }`, `SetCellFormula { ref, text }`, `ApplyStyle { range, style }`. This is what the UI emits, what the REST API accepts, and what MCP tools expose. One vocabulary, three consumers.

**Layer 2 — Reducer (pure function).** `reduce_vN(cmd, &Snapshot) -> Vec<Op>`. This is the *only* place non-determinism could enter, so it is fenced:

- The reducer crate has a deny-by-default dependency allowlist and a custom lint that bans `SystemTime`, `Instant`, `rand`, `HashMap`/`HashSet` iteration, locale-sensitive formatting, and floating-point values in control-flow predicates.
- All entropy is *injected*, never ambient: the command carries `actor_id`, a Lamport timestamp, and a seed derived from `hash(actor_id, lamport)`.
- Reducer versions are immutable and retained forever in the binary, exactly like consensus rules in a replicated log. A command records the reducer version that produced its ops.

**Layer 3 — Operations (`Op`).** Immutable, commutative, causally ordered. Document state is `fold(ops)` — a pure CRDT merge with no reducer involvement.

### Why this eliminates the divergence risk

The dangerous failure mode in v1 was two engine versions reducing the same command differently and silently diverging. That is now impossible by construction: **remote replicas never see Commands.** They receive Ops. Reduction is a local, one-time compilation step, so no two engines ever need to agree on it.

The reducer still needs versioning for four cases: local undo replay, deterministic testing, server-side reduction of API/MCP commands, and the Calculation Authority. For the server case, the server publishes a `min_reducer_version`; clients below it are read-only until they upgrade. Reads are never gated.

### Enforcement

A CI **differential replay gate**: take the full op-log corpus, replay it on `x86_64`, `aarch64`, and `wasm32`, and assert an identical 256-bit state hash. Any divergence fails the build. This single test protects the deepest invariant in the system.

---

## I.2 Collaborative undo

### The problem

Undo cannot be "apply the inverse op." If Alice types "hello" and Bob then bolds it, Alice's undo must remove her text while Bob's bold survives on whatever remains.

### The design: selective undo by inverse synthesis against current state

Each Command produces an `UndoGroup { id, actor, ops: [OpId] }`. The undo stack is a per-actor, per-document stack of group ids — never a stack of state snapshots.

Undo of group *G* computes `inverse(G, current_state)`, **not** `inverse(G, state_at_G)`. That distinction is the whole design. Per CRDT type:

**Sequence (text).** Undo of an insert marks those specific character identities deleted. Because characters carry unique ids (RGA/Fugue-style), concurrent insertions interleaved among them are untouched. Undo of a delete un-tombstones exactly those identities.

**Register (cell value, style property).** Undo of `set(v_new)` restores `v_old` **only if** the currently-winning value is still the one my op wrote. If another actor has since written `v_other`, my undo becomes a no-op on that register — their intent wins. This matches user expectation and matches Google Docs behaviour.

**Map (cell existence, object properties).** Same rule as Register.

**Structural (row/column insert, slide insert).** Undoing a row insertion would delete the row *and everything others put in it*. Policy: **structural undo is blocked and downgraded when it would destroy another actor's operations.** The user is told "Bob added data to this row — undo will clear only your changes," and the operation is narrowed accordingly. Silent data loss is never acceptable, and a prompt is better than a surprise.

**Redo** requires no separate machinery. The undo emits ops forming a new group tagged `is_undo_of: G`; redo is the undo of that group. Consequently the entire undo/redo history lives in the op log and is auditable and replayable.

**Persistence.** The stack persists per `(actor, document)` with a bounded depth (200 groups) and a staleness window. Beyond that, users go to version history instead. This bound is what keeps undo metadata from growing without limit.

---

## I.3 Recalculation determinism

Volatile functions, external data, and iterative calculation cannot converge across replicas by construction. The fix is to stop pretending they are formulas.

### Three-tier value lattice

**Tier 1 — Pure formulas.** `SUM`, `VLOOKUP`, `IF`, and the rest. Every replica evaluates locally and identically. Bit-determinism requirements:

- IEEE-754 binary64 with strict evaluation order. FMA contraction disabled (`-ffp-contract=off`), no fast-math, no auto-vectorised reduction reordering.
- Reductions use a **specified** algorithm — Neumaier compensated summation with a fixed traversal order — chosen because it is simultaneously more accurate than naive summation *and* order-pinned. Excel's `SUM` inaccuracies become a compat-flag behaviour, not our default.
- Ties in `SORT`/`RANK` break on cell identity, never on memory or hash order.
- **No locale inside the engine.** Number and date parsing are locale-free at the model layer; locale is strictly a display concern. This also permanently fixes Excel's regional-CSV and auto-date-mangling class of bugs.

**Tier 2 — Volatile functions** (`NOW`, `TODAY`, `RAND`, `RANDBETWEEN`, `RANDARRAY`, and the dynamic forms of `OFFSET`/`INDIRECT`). These are **not evaluated ambiently**. Each call site is an impure node with a `VolatileBinding` stored *in the CRDT*: `{ value, computed_at, computed_by, policy }`.

Recalculation is an explicit, attributed **event** — user-triggered, on-open per policy, or scheduled — that writes new materialized values as ops. All replicas converge because they read a stored value rather than evaluating. `RAND()` uses a seeded PRNG whose seed lives in the op, so results are reproducible.

This is strictly better than Excel, where a volatile-heavy workbook silently gives different numbers to different people at different times with no record. Here every recalculation is versioned, attributed, and reproducible.

**Tier 3 — External data** (web queries, database connections, live feeds). Always executed by a designated **Calculation Authority** — a server-side worker holding the connector credentials — never by a client. Results land as materialized values with provenance. Clients never hold connector secrets, which is also the fix for the formula-exfiltration threat class in Part II.

**Iterative / circular calculation** is opt-in only, executed exclusively by the Calculation Authority with pinned iteration count and epsilon, result materialized.

### Calculation Authority election

The authority is the server when online and permitted. Otherwise an ephemeral lease is held by the connected editor with the lowest actor id. The lease is advisory: because results are materialized and attributed, a split-brain produces two writes to a volatile register, which the CRDT resolves by last-writer-wins with actor-id tiebreak. **Convergent under all partitions, never divergent.**

In Strict E2EE mode the server cannot be the authority, so a client always holds the lease and external data connectors are unavailable. That is a documented capability difference, not a bug.

---

## I.4 Compound documents

A chart inside a report, a live range inside a slide, a linked table that updates. This is where document architectures historically collapse — OLE being the canonical warning.

### The design: universal `Embed` node over a projection trait

Kernel primitives:

- Every document is a `Container` with a stable `DocId`.
- A node may be `Embed { target: ObjectRef, mode: EmbedMode, view: ViewSpec, cache: RenderCache }`.
- `ObjectRef = { doc_id, path, revision_pin: Option<Version> }` — may point inside the same document or into another.

`EmbedMode` is explicit and small:

- **`Owned`** — the target sub-document lives inside this document's own op log as a nested container. Chart data travels with the deck.
- **`Linked { pinned: Version }`** — a snapshot; refresh is an explicit user action. **This is the default for cross-document links**, because live links are both a stability and a security hazard.
- **`Linked { live }`** — updates flow automatically. Opt-in, cycle-checked, depth-capped.
- **`Opaque { blob }`** — an embed type we do not understand, preserved for round-trip and rendered from the source's supplied placeholder image. Never executed.

Four rules make this hold together:

1. **The container never reaches into the target's model.** It calls the target's projection API — `project(ObjectRef, ViewSpec) -> (RenderTree, DataFrame)` — defined as a trait in the shared kernel. So `doc-deck` depends on a trait, not on `doc-grid` internals. This is what preserves modularity as the suite grows.
2. **Nested op logs are namespaced, not federated.** An `Owned` embed's ops carry a container prefix within the parent's log. One log, one causal order, one sync stream — and therefore no distributed transaction problem.
3. **Render caches are ACL-aware.** If a viewer lacks read permission on a linked target, the cached projection is shown only when it was produced under an explicit "publish snapshot" grant; otherwise a redacted placeholder appears. This closes the long-standing "embedded object leaks data across a permission boundary" hole that affects both Office and Google Workspace.
4. **OOXML maps cleanly.** `chartSpace`, `oleObject` and `externalLink` map onto these modes; anything unrecognised becomes `Opaque`, subject to the active-content policy in Part II.

---

## I.5 Engine API versioning and CRDT schema evolution

The fleet updates at four different speeds: server continuously, desktop weekly, mobile gated by app review, self-hosted possibly annually. All must interoperate.

### Three independent version axes — never conflated

1. **`wire_version`** — sync protocol framing. Negotiated at handshake; support N−2.
2. **`model_version`** — CRDT schema. The dangerous one.
3. **`capability_set`** — feature flags, negotiated as a *set*, not a number.

### The forward-compatibility rule (the crux)

Every Op is a tagged, length-prefixed structure with **mandatory preservation**. An engine encountering an unknown op type must: preserve it verbatim, include it in causal ordering and in the state hash as an opaque node, and re-transmit it intact. This is protobuf unknown-field preservation, applied to a merge structure.

Each op declares a criticality:

- **`Cosmetic`** — unknown ops are ignored for rendering but preserved for sync. An old client can keep editing safely.
- **`Structural`** — an unknown op forces the document **read-only** with a clear "this document uses features from a newer version" banner. This is what prevents an old client from silently corrupting a document by editing around structure it cannot see.

Op type meanings are immutable. New behaviour is always a new op type.

### Schema evolution across offline replicas

Model changes are **additive op types only**, plus a **compaction-time rewriter**. When the server compacts history it rewrites deprecated op types into current forms and bumps the snapshot's `model_version`.

A replica older than the compaction watermark cannot merge directly. It fetches a fresh snapshot and rebases its local unsynced ops through `migrate_ops(old_v, new_v)`. Maximum offline staleness is a **published, enforced number — 180 days** — beyond which merge is refused and the rebase flow is mandatory. Publishing that number is what turns the COO's landmine into an ordinary supported operation.

### Deprecation policy

A capability may be removed only after two LTS cycles, telemetry showing under 0.1% usage, and an automatic rewriter existing in the compactor.

---

# PART II — Security, Designed Rather Than Named

## II.1 Key management

### Choice: MLS (RFC 9420) as the group key agreement layer

This is not the obvious choice, and the obvious choice is wrong. Most designs reach for "one document key, wrapped per user," which collapses on member removal: revocation requires re-encrypting everything, so in practice nobody does it and removed members retain cryptographic access.

MLS is built precisely for this: large, dynamic, asynchronous groups with forward secrecy and post-compromise security. Its tree-based agreement makes membership changes O(log n) rather than O(n), so a 5,000-member document is tractable. Removal is *cryptographic* — a removed member cannot decrypt subsequent epochs, no re-encryption campaign required.

### Concrete design

- **One MLS group per E2EE document.** Members are *devices*, not users; a user with three devices occupies three leaves bound to a user-level credential.
- **Ops are encrypted** with a per-epoch AEAD key derived from the MLS exporter secret. Epoch changes on every membership change, and on a schedule (24 hours or 100k ops, whichever first) so a passively compromised key stops being useful — automatic post-compromise security.
- **History on join is a policy, made explicit:** `NoHistory | FromEpoch(n) | FullHistory`. MLS gives forward secrecy by default, so `FullHistory` requires an existing member to re-wrap a `HistoryKeyBundle` to the joiner. That is an explicit, audited action — "who granted history to whom" is a logged fact, not an invisible default.
- **Device enrolment:** each device presents a key package signed by the user identity key, which is protected by the platform secure element (Secure Enclave, TPM, StrongBox) where available, and by an Argon2id-derived key otherwise. Adding a device requires either approval from an existing device via short-authentication-string comparison, or an org-admin attestation. Both are audited.
- **Recovery without a backdoor:** a Recovery Key issued at account creation (24-word mnemonic plus a printable PDF), optionally supplemented by Shamir social recovery (3-of-5 among designated colleagues or admins). Losing every device *and* the recovery key means permanent data loss, and the product must say so plainly rather than implying otherwise.

## II.2 The E2EE / compliance collision

E2EE as stated in v1 contradicted server-side search, thumbnails, headless rendering and the AI gateway — and collided head-on with eDiscovery, legal hold and DLP, which are non-negotiable in exactly the regulated markets the sovereignty wedge targets.

### Three tiers, chosen per workspace, with an honest capability matrix

| | **Standard** | **Managed E2EE** | **Strict E2EE** |
|---|---|---|---|
| Key holder | Server (envelope keys in KMS/HSM) | Clients + Compliance Principal | Clients only |
| Server reads plaintext | Yes | No | No |
| Search | Server-side full text | Client-side encrypted index; optional enclave indexer | Client-side index only |
| Server render / thumbnails / convert | Yes | No | No |
| AI features | Server-side | Client-side / BYO-key | Client-side only |
| External data connectors | Yes | Client-lease only | Client-lease only |
| DLP | Server-side, real-time | Client-side attested | Client-side attested |
| Legal hold | Native | Native (holds ciphertext) | Ciphertext retention only |
| eDiscovery production | Native | Via escrow, break-glass, quorum | **Not possible — documented** |

### Managed E2EE is the enterprise-viable middle, and the mechanism matters

The organisation is **a member of the MLS group**: a non-human "Compliance Principal" leaf whose private key lives in an HSM under dual control with M-of-N officer quorum. The properties that follow are the point:

- There is **no protocol backdoor** — it is an ordinary group member.
- It is **visible in the member list to every participant**. Access is transparent, not secret.
- Every use produces a **tamper-evident audit entry in a hash chain anchored externally** (transparency log or RFC 3161 timestamping authority), so the organisation can *prove* it did not snoop and employees can independently verify. That is a stronger position than any incumbent offers.
- Break-glass requires quorum approval plus a recorded legal basis.

### DLP under encryption: attestation, not inspection

Policy runs **client-side in the WASM sandbox** at the point of edit and export. The client evaluates signed policy bundles and emits *attestations* — signed assertions of the form "document X was scanned under policy v42; result: clean." The server sees attestations, never content. A client that will not attest is denied sync. Enforcement operates through availability rather than through reading, which is the only honest way to do DLP without plaintext.

### Search under encryption

Ship a per-user client-side encrypted index first: built locally, encrypted with a user index key, stored server-side as opaque shards, queried locally. Add a confidential-computing indexer later — an indexing worker that joins the MLS group inside an attested enclave (SEV-SNP or TDX), giving org-wide search without operator access. Sequence these; do not promise the second on day one.

## II.3 Parser isolation and the active-content policy

### Hostile-input zone

All format parsing — OOXML, ODF, legacy binary, images, fonts — runs in a **separate sandboxed process**, not merely a separate thread:

- seccomp-bpf syscall filter: no network, no filesystem beyond passed descriptors, no `exec`.
- No ambient capabilities; memory cap of 2× input size or 512 MB, whichever is greater; CPU and wall-clock caps; a fresh process per document.
- On WASM platforms the equivalent is a separate instance with its own linear memory and a capability-free import set.
- **The parser's only output is a serialized, schema-validated intermediate representation.** It never returns pointers and never invokes host callbacks. The host revalidates the IR against a strict schema before it reaches the model. A fully compromised parser can therefore at worst emit malformed-but-schema-valid IR.
- Zip handling is streaming with hard caps on entry count, uncompressed size, and compression ratio (reject beyond 100:1 — zip-bomb defence). XML parsing disables DTDs and external entities entirely and caps depth and node count, killing XXE and entity-expansion attacks at configuration level rather than by vigilance.
- Continuous structure-aware fuzzing with a grammar-based OOXML generator, seeded from the fidelity corpus, with a coverage gate in CI.

### The opaque-blob policy, corrected

v1's "preserve unknown elements verbatim for lossless round-trip" would have faithfully stored and re-emitted malware. Preservation is now **classified at ingest**:

| Class | Examples | Policy |
|---|---|---|
| **Inert markup** | Unknown attributes, unrecognised namespaces with no binary payload | Preserve verbatim, re-emit on export |
| **Inert binary** | Unknown image formats, embedded fonts | Preserve and re-emit; decode only inside the parser sandbox |
| **Active content** | `vbaProject.bin`, OLE objects, ActiveX controls, DDE links | **Quarantine.** Stripped from the live document into a separate encrypted store, never executed, never re-emitted by default. Export with active content is an explicit, permissioned, audited action that admin policy can forbid globally. |
| **Network-fetching** | `externalLink` URLs, remote image references, remote templates, external `INDIRECT` | Neutralised: the URL is preserved as data and never auto-fetched. Fetching requires user action plus an egress allowlist. |

The fidelity contract therefore reads: *we preserve everything, re-emit everything inert, and quarantine everything executable* — stated up front rather than discovered by a customer.

**Legacy binary formats** (`.doc`, `.xls`, `.ppt`) are import-only, through the same sandbox with a tighter resource budget, and are never an export target. For the most pathological cases, conversion runs one-shot in a Firecracker microVM.

## II.4 Threat model — the non-obvious entries

A full STRIDE-per-boundary document is a deliverable (`17-threat-model.md`), covering: untrusted file → parser, plugin → engine, client → sync server, tenant → tenant, operator → customer data, agent → document, and supply chain → build. Four threats deserve naming here because they are routinely missed:

**Malicious collaborator injecting hostile ops.** A permitted collaborator can send crafted operations that exploit a bug in another client's op applier. This is a genuinely novel attack surface that CRDT products consistently ignore. Mitigation: ops are schema-validated and bounds-checked *on receive*, and the applier is fuzzed against adversarial op logs as a first-class corpus.

**Formula-based exfiltration.** `WEBSERVICE`, `INDIRECT`, and image-from-URL are established Excel data-exfiltration channels. Mitigation is structural rather than heuristic: formulas have **no ambient network access, ever**. All external fetch flows through the Calculation Authority under an egress allowlist with audit — which falls out of the Tier 3 design in Part I.3.

**Agent confused-deputy via prompt injection.** An agent holding a broad token reads a document containing adversarial instructions and is induced to exfiltrate other documents. Neither Microsoft nor anyone else has a real answer yet, which makes it a differentiator worth solving properly: per-agent tokens are short-lived and *document-scoped*, intent is declared at mint time (which documents, read or write), egress is policy-controlled, and MCP tool responses explicitly mark document content as untrusted data rather than instruction.

**Tombstone amplification.** An attacker generates billions of tiny ops to exhaust storage and memory for every collaborator. Mitigation: per-actor operation rate limits and byte quotas enforced at the relay, plus scheduled server-side compaction.

## II.5 Supply chain, crypto, compliance

Dependency governance uses `cargo-deny` and `cargo-vet` with vendored dependencies, reproducible builds, SLSA Level 3 provenance, Sigstore signing, a CycloneDX SBOM per release, and — importantly — a hard cap on transitive dependency count in the core with a review gate on every addition. A memory-safe language does not save you from a compromised crate.

Cryptography uses a single audited implementation. `aws-lc-rs` is the recommendation specifically because its FIPS mode is backed by a FIPS 140-3 validated module, which is the practical route to US federal procurement; algorithm agility comes from a versioned cipher-suite identifier. No custom cryptography anywhere, under any deadline pressure.

Compliance work starts at day one, not at first audit: control mapping for SOC 2, ISO 27001 and ISO 27701; FIPS mode as a build flag; data residency as a first-class deployment concept with region-pinned storage and relay affinity; a DPIA template; a vulnerability disclosure policy and bug bounty from public beta; and penetration testing per major release.

---

# PART III — Text Layout and Internationalisation

The v1 estimate was wrong by roughly an order of magnitude. The corrective principle: **do not build a text engine — build a layout orchestrator over battle-tested Unicode and shaping components, with an explicit Word-compatibility layer on top.**

## III.1 The ten-stage pipeline

```
Styled text runs (logical order)
 1. Unicode segmentation        grapheme / word / sentence — UAX #29
 2. Bidi resolution             UAX #9 — paragraph to runs by embedding level
 3. Script & font itemization   UAX #24 + fallback chain resolution
 4. Shaping                     rustybuzz — glyph ids and positions per item
 5. Line breaking               UAX #14 + dictionary/LSTM for Thai, Lao, Khmer, Burmese
 6. Justification & hyphenation greedy (Word-compat) or Knuth-Plass (optional)
 7. Bidi display reorder        UAX #9 rule L2, per line
 8. Complex layout              writing-mode, ruby, tate-chu-yoko, warichu
 9. Line stacking & pagination  Word-compatible line-height model
10. Floats & anchors            tables, floats, anchored objects, text wrap
 → Render tree (glyph runs + positioned boxes)
```

## III.2 Build versus buy, per stage

| Stage | Approach | Rationale |
|---|---|---|
| 1, 2, 5 | **`icu4x`** | Never implement Unicode algorithms yourself. `icu4x` matters specifically because it is designed for WASM size constraints with sliceable locale data — the older ICU was not viable in a browser bundle. This dependency is what makes the whole plan feasible. |
| 3 | Custom itemizer over `fontdb`/`fontique` | Fallback-chain policy is product-specific and is load-bearing for determinism. |
| 4 | **`rustybuzz` everywhere** | Shaping is solved; do not reimplement. Use the pure-Rust port on *all* platforms rather than native HarfBuzz on native ones, because deterministic pagination requires byte-identical shaping output. **Determinism beats speed here**; recover the cost with aggressive shaping caches. |
| 5 (dictionary languages) | `icu4x` dictionary and LSTM breakers | Thai, Lao, Khmer and Burmese have no inter-word spaces. There is no shortcut. |
| 6 | Both algorithms, profile-selected | Word compatibility is a *constraint*; typographic beauty is a *feature*. Never conflate them. |
| 8 | Custom, per CSS Writing Modes L4 and JLReq | Vertical Japanese is a genuine market requirement that no web-based editor currently does well — a real differentiator. |
| 9, 10 | Custom, Word-compatible | This is the compatibility work, and it is the bulk of it. |

## III.3 The Word-compatibility layer

Matching Word means reproducing behaviours documented in no specification:

- Word's line-height model — `w:spacing` with `auto`, `exact` and `atLeast`, and the ascent-plus-descent-plus-lineGap heuristics — including the behavioural differences between compatibility modes signalled by `w:compatSetting`.
- Word's justification, which is greedy rather than Knuth-Plass, with distinct inter-word and inter-character expansion rules, kashida for Arabic, and separate CJK behaviour.
- **Table autofit**, which is idiosyncratic and is the single largest source of visible fidelity loss in every competing product. It warrants a dedicated engineer.
- Anchored-object and text-wrap resolution ordering.
- Kinsoku shori — CJK line-break prohibition — with Word's specific default character classes.

The mechanism is a `compat_profile` enum (`Word2016 | Word2019 | Word365 | Native`) parameterising stages 5, 6, 9 and 10. Imported documents adopt the profile indicated by `w:compatSetting`; native documents use `Native`, which is free to be better. This is how you are simultaneously Word-compatible *and* better than Word without lying about either.

## III.4 Deterministic pagination — solved via metrics, not pixels

For page 7 to be page 7 on every device:

**Ship your own font stack.** Metric-compatible substitutes for the Microsoft core fonts are available under permissive licences: Carlito for Calibri, Caladea for Cambria, and the Liberation family for Arial, Times New Roman and Courier New. Beyond those, license a set deliberately. Full CJK and Indic coverage means Noto, which is open but approaches a gigabyte at full coverage — so aggressive subsetting and lazy fetch are architectural requirements, not optimisations. This is a real budget line.

**Make the fallback chain deterministic.** The resolved font for a given `(family request, script, codepoint)` must be identical on every platform. Therefore **layout never consults OS fonts by default** — it uses only bundled and document-embedded fonts. OS fonts are an explicit opt-in that marks the document "layout may vary." This is the single most important decision behind the WYSIWYG promise, and it is the one every competitor gets wrong.

**Separate layout determinism from rasterisation determinism.** Layout consumes font *metrics* — unhinted outlines scaled in fixed-point arithmetic at defined subpixel precision (1/64 px, matching FreeType's 26.6 convention) — never rasterised output. Pixels may legitimately differ across platforms; boxes and line breaks may not. This decoupling is what makes the promise achievable at all, and v1 conflated the two.

**Subset and embed on export.** On the web, fetch the metrics table before outlines so that layout never blocks on font downloads; load the visible subset first and the full face in the background.

## III.5 IME, accessibility, and the "boring 80%"

A custom canvas renderer is what kills most from-scratch editors, because it forfeits every piece of platform text integration at once.

### Hybrid rendering: canvas visuals, live shadow accessibility tree

Render visuals to canvas or WebGPU for speed and cross-platform consistency. **Simultaneously maintain a shadow DOM tree** positioned exactly over the canvas, containing real text nodes rendered transparent, with correct ARIA roles and geometry. This is the architecture Google Docs converged on after years of canvas migration, and it is correct.

What this buys, all of it otherwise unaffordable:

- **IME works natively.** A real editable element sits at the caret; the OS handles composition, candidate windows and preview underlines. Do not attempt to implement CJK composition yourself — it is a multi-year tarpit and users will notice every deviation.
- **Screen readers work**, because they read the shadow tree.
- **Native selection, spellcheck, autocorrect, dictation and find-in-page** can be delegated or coherently intercepted.
- **Browser translation, password managers and extensions** behave normally.

The cost is maintaining two trees; the mitigation is generating both from the *same* render tree — one source, two sinks — and virtualising the shadow tree to the viewport plus margin.

On native shells the equivalent is the platform accessibility API. Use **`accesskit`**, which abstracts UI Automation, AT-SPI and NSAccessibility behind one Rust interface — it exists precisely for this problem.

### Accessibility as a P0 constraint

Target WCAG 2.2 AA, EN 301 549 and Section 508, and produce a real VPAT before the first enterprise sale — without one, public-sector procurement eliminates you regardless of product quality. One accessibility tree is built by the kernel and fed to both `accesskit` and the shadow DOM.

Every command must be keyboard-reachable, with a remappable keymap whose defaults **match Word and Excel exactly** — muscle memory is a migration cost, and matching shortcuts is a feature, not imitation. High contrast, reduced motion, and 200%/400% zoom reflow are requirements. The CI matrix covers NVDA and JAWS on Windows, VoiceOver on macOS and iOS, TalkBack on Android, and Orca on Linux — automated via `accesskit` tree assertions where possible, manual per release otherwise.

### Internationalisation, staged honestly

**Tier 1 (v1):** Latin, Cyrillic, Greek; full bidi for Arabic and Hebrew; horizontal CJK. Roughly 85% of the addressable market.

**Tier 2 (v1.5):** Indic scripts — Devanagari, Bengali, Tamil, Telugu — which matter directly given the India sovereign angle; dictionary breaking for Thai, Lao, Khmer, Burmese; vertical CJK.

**Tier 3 (v2):** Ruby, tate-chu-yoko, warichu, Mongolian, full JLReq and CLReq conformance.

Each tier is a shipping gate with its own test corpus, not a blanket "we support Unicode."

## III.6 Revised effort estimate

The text stack alone is **8–12 engineer-years** to reach a credible Word replacement for business documents: roughly 3 years in the Word-compatibility layer, 2 in i18n Tiers 1–2, and approximately 1 in table autofit by itself. The v1 proposal implied a fraction of this.

This is the strongest argument for sequencing `doc-grid` first. A spreadsheet's text requirements are dramatically simpler — single-line cells, no pagination, no floats, no anchored objects — so the grid can ship and generate revenue while the text team does multi-year work.

---

# PART IV — Consequences for the Architecture

## IV.1 Revised layer diagram

```
┌──────────────────────────────────────────────────────────────┐
│ SHELLS   PWA │ Tauri │ iOS/Android (native, UniFFI) │ Headless│
├──────────────────────────────────────────────────────────────┤
│ PRESENTATION                                                  │
│   canvas/WebGPU visuals  ║  shadow a11y tree (accesskit/DOM)  │
│   IME host · keymap (Word/Excel-compatible defaults)          │
├──────────────────────────────────────────────────────────────┤
│ COMMAND API  — one vocabulary for UI, REST/GraphQL, and MCP   │
├──────────────────────────────────────────────────────────────┤
│ REDUCER (pure, versioned, lint-fenced)   Command → [Op]       │
├──────────────────────────────────────────────────────────────┤
│ CORE ENGINE (Rust)                                            │
│   doc-grid │ doc-text │ doc-deck   ── via projection trait    │
│   kernel: CRDT model · Embed/portal · undo groups · styles    │
│   text pipeline (icu4x · rustybuzz · font stack · compat)     │
│   formula engine (3-tier lattice) · render tree               │
├──────────────────────────────────────────────────────────────┤
│ CRYPTO LAYER   MLS group per doc · epoch keys · attestations  │
├──────────────────────────────────────────────────────────────┤
│ SYNC   op log · causal order · compaction+rewriter · quotas   │
├──────────────────────────────────────────────────────────────┤
│ PARSER SANDBOX (isolated process/instance) → validated IR     │
│   OOXML · ODF · legacy binary · images · fonts                │
├──────────────────────────────────────────────────────────────┤
│ SERVER   relay · storage · auth · Calculation Authority ·     │
│          MCP · compliance principal (HSM) · audit chain       │
└──────────────────────────────────────────────────────────────┘
```

Two structural changes from v1: the **parser sandbox is now a distinct layer** below the engine rather than a module inside it, and the **reducer is a named layer** between the command API and the model.

## IV.2 Claims, restated honestly

| v1 claim | v2 claim |
|---|---|
| Byte-faithful OOXML round-trip | **Semantically lossless** round-trip; inert content preserved verbatim; active content quarantined and disclosed |
| 10M+ rows in the browser | 10M+ rows with a **defined server-compute path**; browser handles what wasm32's 4 GB allows, with chunked virtual loading |
| Better typography than Word | **Word-compatible by profile, better in `Native` mode** — both true, neither overstated |
| Identical engine everywhere | Identical **core** everywhere; web and desktop via WASM/native, **mobile via native compile (UniFFI)** |
| WYSIWYG across devices | **Identical layout** across devices (metrics-determined); pixels may differ by rasteriser |
| E2EE | **Three tiers** with a published capability matrix and a documented compliance position |

## IV.3 Additional documents this design requires

Beyond the v1 list: `17-threat-model.md`, `18-accessibility.md`, `19-internationalization.md`, `20-performance-budgets.md`, `21-text-layout.md`, `22-crypto-and-key-management.md`, `23-compliance-tiers.md`, and `24-adr/` seeded with the decisions recorded above (ops-as-truth, MLS, rustybuzz-everywhere, canvas-plus-shadow-tree, no-OS-fonts, quarantine-active-content, 180-day staleness).

## IV.4 Performance budgets — now specified, as CI gates

| Metric | Target (p95) |
|---|---|
| Keystroke to paint | < 16 ms |
| Cold open, 1 MB document | < 800 ms |
| Cold open, 50 MB workbook | < 4 s |
| Recalculation, 100k dependent cells | < 200 ms |
| Sync operation propagation, same region | < 150 ms |
| Memory per open 1 MB document | < 60 MB |
| WASM core bundle, compressed | < 12 MB |
| Shaping cache hit rate, steady-state editing | > 95 % |

These become gates, not aspirations. Without them, performance regresses continuously and invisibly.

---

# PART V — What Remains Open

Three items are designed but unproven, and remain the go/no-go spikes:

**Spike 1 — CRDT spreadsheet at scale.** 100k+ cells, concurrent structural edits (row insert against a range-referencing formula), measured tombstone growth and compaction efficiency, verified merge convergence under adversarial interleaving. *Success criterion:* under 3× storage overhead versus raw data after compaction, and convergence across 10k randomised interleavings.

**Spike 2 — WASM memory and performance on Safari/iOS.** A realistic 50 MB workbook in the actual constrained environment. *Success criterion:* the performance budgets above met on an iPhone two generations old, or a clear determination that mobile requires the native path — which is an acceptable outcome, but must be known before committing.

**Spike 3 — Canvas accessibility.** A canvas-rendered grid and text region driven end-to-end by NVDA, JAWS and VoiceOver via the shadow tree. *Success criterion:* a blind operator completes a defined set of editing tasks unassisted.

Two further items are deliberately deferred rather than solved: the **confidential-computing search indexer** for Managed E2EE (ship the client-side index first), and **full JLReq/CLReq conformance** for Japanese and Chinese typography (i18n Tier 3).

Everything else in this document is designed and ready to specify.
