# OpenSuite Architecture — Multi-Role Executive Review

**Subject:** Proposed cross-platform Office suite (Rust core + WASM, CRDT-native model, OOXML fidelity, self-host + SaaS, API/MCP-first)
**Review date:** 2026-08-07
**Reviewers (simulated):** Founder · CTO · CBO · COO · CISO · Architect · Principal Engineer

---

## 0. Headline

| Dimension | Rating | One-line verdict |
|---|---|---|
| Architectural direction | **7.5 / 10** | The bones are good — genuinely better structural choices than Microsoft made. |
| Technical feasibility as stated | **3.5 / 10** | Several load-bearing claims are not deliverable as written. |
| Business viability as stated | **3 / 10** | No wedge, no distribution, no pricing. "Beat MS Office" is not a strategy. |
| Security depth | **5 / 10** | Correct vocabulary, missing the hard parts (key management, legal hold, parser isolation). |
| Operational readiness | **3 / 10** | On-prem-at-scale, upgrades, and CRDT schema evolution are treated as free. They are not. |
| **Composite** | **≈ 5.5 / 10** | A strong architecture sketch, not yet an executable plan. |

The single most important correction: **this document describes a product, not a company.** The technology strategy is defensible. The reason every general-purpose Office competitor has failed is not that they built worse software — LibreOffice, WPS, OnlyOffice, Zoho, Collabora all built respectable software. They failed on distribution and on the asymptote of compatibility. Nothing in the current proposal addresses either.

---

## 1. Founder — Rating 4 / 10

### What's right
The instinct that a from-scratch, agent-native, sovereignty-friendly suite is a real opening is correct. Two genuine tailwinds exist right now: European and Indian public-sector pressure to de-Microsoft, and the fact that no incumbent has a document substrate that AI agents can drive natively. Those are real, and they are the only two reasons this company could exist.

### Where it fails
The proposal commits to building Word, Excel and PowerPoint at parity, simultaneously, before generating a dollar. That is a 100–200 engineer, four-to-seven year, nine-figure program with no revenue milestone in it. Founders do not get to make that bet; only incumbents do.

Beating Office is a distribution problem wearing a technology costume. Office's moat is the Microsoft 365 bundle (Teams, Entra ID, Exchange, Intune sold as one line item), thirty years of accumulated macros, templates and financial models, and the muscle memory of roughly a billion people. A better kerning engine moves none of that. Google Workspace is the only meaningful winner in this category in twenty-five years, and it won by being free and riding Gmail, Chrome and the education market — not by being better at documents.

The "MS Office lags here" table is largely accurate but mostly lists *irritations*, not *purchase drivers*. CIOs do not migrate 40,000 seats because pagination differs between web and desktop. They migrate for cost, sovereignty, or a mandate.

### The gaps
There is no ideal customer profile, no wedge product, no pricing model, no distribution channel, no partner or reseller motion, and no articulated reason a buyer switches *this quarter*. There is no open-source vs. proprietary decision, which is a strategic fork that determines whether the community becomes leverage or becomes the competitor who forks you. There is no funding sequence tied to technical milestones.

### Recommendation
Pick one wedge and sequence hard. The two candidates, in order of my conviction:

**Wedge A — the agent-native spreadsheet.** Ship `doc-grid` alone, first, as the workbook that agents and APIs can actually drive end-to-end. Sell to fintech, ops and analytics teams who are currently gluing Python to Excel and losing. This is a twelve-to-eighteen month v1, not five years, and it has a paying buyer today.

**Wedge B — sovereign document infrastructure.** Lead with self-hosted, air-gapped, E2EE and full audit, sold to EU/India public sector, defense, and regulated healthcare. Slower sales cycles, but the buyer is politically motivated and Microsoft cannot follow.

Both beat "all three apps, everywhere, eventually." Word and PowerPoint should be years two and three, funded by the wedge.

---

## 2. CTO — Rating 6 / 10 on direction, 3 / 10 on plan realism

### What's right
One engine compiled to native and WASM is the correct structural bet, and it is precisely the mistake Microsoft cannot undo. The command bus that makes UI, API and MCP the same surface is excellent and rare. Codec-at-the-boundary with the internal model decoupled from OOXML is the right call. If this architecture is executed, cross-platform behavioural consistency really would be a differentiator no incumbent can match without a rewrite.

### Where it fails

**The WASM claims do not hold.** `wasm32` has a hard 4 GB address space, and Memory64 support is still uneven — notably weak in Safari, which is the *only* engine available on iOS. The stated "10M+ rows in the browser" is not deliverable on that substrate. You will need a server-side compute path for large workbooks, and the moment you add that, two headline claims break simultaneously: "the identical engine runs everywhere" and "the server never sees plaintext." That contradiction is currently unacknowledged and it propagates through the entire design.

**Mobile is not the same artifact.** iOS `WKWebView` runs JITless for third-party apps, so WASM there is materially slower than desktop — typically two to five times. A realistic mobile path is a native Rust compile via UniFFI with a native or lightly-wrapped UI, which means mobile diverges from the web shell. That needs to be an explicit, budgeted decision rather than a footnote.

**CRDT-for-spreadsheets is research-grade risk.** CRDTs for rich text are proven in production (Yjs, Automerge). CRDTs for spreadsheet semantics are not. The unsolved parts are concurrent row/column insertion and deletion with range-reference rebasing (two users concurrently insert a row and a `SUM(A1:A10)` must mean the right thing on both replicas), tombstone growth on multi-million-cell grids, and deterministic recalculation across replicas in the presence of volatile functions. This is the highest-variance item in the entire plan and it is currently stated as a settled decision.

**Accessibility is absent and it is a hard blocker.** A custom canvas renderer means you own the entire accessibility tree, screen-reader integration, keyboard navigation model, focus management, high-contrast and reflow behaviour. Under Section 508, EN 301 549 and the European Accessibility Act, a missing or weak VPAT eliminates you from public-sector and most large-enterprise procurement outright — which is exactly the market the sovereignty wedge targets. This cannot be retrofitted; it has to be a P0 constraint on the render architecture.

### Recommendation
Spend the first eight to ten weeks on three de-risking spikes before committing a line of product code: a CRDT spreadsheet spike (100k cells, concurrent structural edits, measured tombstone growth and merge convergence); a WASM performance and memory spike on Safari/iOS with a real workbook; and an accessibility spike proving a canvas-rendered grid can be driven by NVDA, JAWS and VoiceOver. If any of the three fails, the architecture changes materially — better to learn that now than in month eighteen.

---

## 3. CBO — Rating 3 / 10

Commercial strategy is effectively absent from the document, so this rating reflects a gap rather than a flaw.

Compatibility deserves special comment because it is commercially treacherous. It is not a feature; it is table stakes, and it is asymptotic. Ninety-nine percent fidelity still means the one board deck that matters renders wrong, and that single incident ends the pilot. Every competitor has died on this hill. The commercial answer is not "achieve 100%" — it is to *choose document classes where fidelity risk is low* (internal ops workbooks, generated reports, collaborative drafts) and avoid ones where it is fatal (client-facing decks, legal contracts with tracked changes) until fidelity is proven at scale.

Missing entirely: pricing architecture and per-seat economics, the open-core versus proprietary decision and its licence, channel and systems-integrator strategy (which is how sovereign deals are actually won), a migration services offering — enterprises have millions of legacy files and will pay someone to move them — competitive positioning against the realistic incumbent set (Google Workspace, OnlyOffice, Collabora, Zoho, WPS, not just Microsoft), and an analyst and procurement-readiness plan.

One overlooked asset: the MCP server is potentially a *distribution* strategy, not just a feature. "The document layer AI agents can drive" is a story that gets you into accounts that would never take a call about an Office alternative.

---

## 4. COO — Rating 3 / 10

The claim that self-hosted and SaaS are "the same artifact, so on-prem is never second-class" is technically elegant and operationally naive. The same binary does not mean the same operational burden. Supporting on-premises deployments at even a few hundred customers means multiple concurrent versions in the field, customer-specific environments you cannot reproduce, upgrade windows you do not control, and support engineers debugging blind. That is a permanent cost centre, and it needs an LTS policy, a supported-version window, and a support tier model priced accordingly — none of which exist yet.

The sharpest operational landmine is **CRDT schema evolution**. When the document model changes in version 12, there are offline replicas sitting on laptops running version 9 that will eventually reconnect and merge. You need forward and backward compatibility rules in the CRDT itself, a migration protocol, and a defined maximum staleness window, designed in from day one. Retrofitting this is close to impossible.

Also missing: an organisational design and hiring plan (Rust systems engineers, typography and text-layout specialists, and formula-engine people are scarce and expensive — this shapes both burn and location strategy), a cost model and unit economics for the collaboration relay and storage, backup and disaster recovery for CRDT operation logs including point-in-time restore semantics, a customer-onboarding and bulk-migration toolchain, and an incident-management and status-communication practice.

---

## 5. CISO — Rating 5 / 10

The security section uses the right vocabulary and gets several instincts right — WASM-sandboxed plugins instead of ambient-authority macros is genuinely better than VBA, and capability-based permissions, SBOM and signed releases are correct defaults. But every hard problem is compressed into a single clause.

### The E2EE contradiction
End-to-end encryption is stated as a feature and then quietly contradicted by server-side search indexing, thumbnail generation, headless rendering and conversion, and an AI gateway. All of those require plaintext. This needs an explicit tiered model: a standard tier with server-side keys and full features, and a confidential tier with E2EE and a *documented, accepted* loss of server-side capability.

More seriously, E2EE collides head-on with mandatory enterprise controls: eDiscovery, legal hold, DLP scanning, and regulatory retention. In regulated industries these are not negotiable, and "the server cannot read it" is a compliance failure, not a feature. The answer is usually organisational key escrow with a break-glass audit trail — which must be designed, disclosed and defensible, not improvised.

Key management itself is unspecified and it is the hardest part of the whole system: device enrolment and attestation, key rotation, member removal with post-compromise security (a user removed from a document must not decrypt future operations), account recovery without a plaintext-accessible backdoor, and multi-device sync. This is where E2EE projects fail.

### The parser surface
Office file parsers are historically the single richest source of critical CVEs in the category. Rust's memory safety removes a large class of these, but not logic bugs, zip-bombs, XXE, billion-laughs, or resource exhaustion. Fuzzing in CI is necessary and insufficient — parsing should run in a separate sandboxed process or WASM instance with hard memory and CPU limits, per document.

The proposal's "preserve unknown OOXML elements as opaque blobs for lossless round-trip" is a security landmine as written. You would be faithfully storing and re-emitting arbitrary content, potentially including OLE objects, embedded executables and macro streams. This needs an explicit policy: sanitise and quarantine active content, never re-emit executable payloads, and make the preservation guarantee explicitly exclude active content.

Also absent: legacy binary formats (`.doc`, `.xls`, `.ppt`) are not mentioned at all, yet enterprise archives are full of them and their parsers are far more dangerous than OOXML. A formal threat model with documented trust boundaries. Multi-tenant isolation design for the collaboration relay. Supply-chain hardening beyond SBOM — `cargo-deny`, dependency vendoring, provenance attestation, and a policy on the very large transitive Rust and npm dependency surface. FIPS 140-3 validated crypto, which US federal procurement requires regardless of how good your non-validated crypto is. GDPR data-residency mechanics and a DPIA. A vulnerability disclosure policy, bug bounty, and penetration-testing cadence. Incident response runbooks.

---

## 6. Architect — Rating 7 / 10

This is the strongest section of the proposal. Layer separation is clean, the command bus is genuinely elegant, the codec boundary is right, and the retained-mode render tree with pluggable backends is the correct shape.

Five design questions are unresolved and each is load-bearing.

**Command bus versus CRDT — which is the source of truth?** There are two overlapping change mechanisms in the design. Commands must reduce deterministically to CRDT operations, and that reduction must be identical on every replica and every engine version. Version skew here produces silent divergence, which is the worst possible failure mode. This contract needs to be specified before anything else is built.

**Collaborative undo.** In a multi-user CRDT, undo is not "apply the inverse operation." It must be intention-preserving and scoped to the user's own operations, without clobbering concurrent edits by others. This is a well-known hard problem and it is not mentioned.

**Recalculation determinism.** Volatile functions (`NOW`, `RAND`, `RANDBETWEEN`), external data connections and iterative calculation cannot converge across replicas by construction. You need a defined evaluation authority — either a designated calculating replica, or volatile results materialised as ordinary values in the CRDT. Pick one explicitly.

**Compound documents.** A chart inside a document, a spreadsheet range embedded in a slide, a linked-and-updating table — this is where document architectures historically collapse. The kernel needs an embedding and linking model from the start; it cannot be added between `doc-text` and `doc-deck` later.

**Engine API versioning across independently updating shells.** The server updates continuously, the desktop app updates weekly, the mobile app is gated by app-store review, and self-hosted customers update annually. All must interoperate. That demands an explicit compatibility contract, negotiated capability levels, and a deprecation policy.

Two smaller notes. The WASM boundary is described as "typed commands" with no attention to serialisation cost — passing large ranges across the boundary per keystroke will dominate your latency budget, so plan for shared linear memory with zero-copy views rather than message passing. And the plugin model has no UI extension surface defined; plugins that need to render anything need a declarative or sandboxed-iframe story.

Finally: adopt an ADR (Architecture Decision Record) practice now. Every decision above will be revisited in year two by people who were not in the room.

---

## 7. Principal Engineer — Rating 6 / 10 on concept, 3 / 10 on stated claims

Specific technical objections, in descending order of how much they will hurt.

**"Byte-faithful round-trip" is not achievable and should be withdrawn.** OOXML is a zip of XML. Zip entry ordering, compression parameters, XML namespace prefix choices and attribute ordering all vary legitimately. The correct and defensible goal is *semantically lossless* round-trip, verified by semantic diff plus pixel diff, with unknown elements preserved. Promising byte-identical output creates a test suite you can never make green.

**Text layout is being underestimated by roughly an order of magnitude.** `rustybuzz` gives you shaping, not layout. Above it you still need line breaking (ICU/UAX #14), bidirectional text (UAX #9), hyphenation dictionaries per locale, vertical text and ruby annotation for CJK, complex-script handling for Indic and Arabic, and justification models that match Word's. The claim of "better typography than Word" cannot be made on day one; Word's line-breaking model is itself the compatibility target, and matching it is the work.

**Deterministic pagination has a licensing dependency nobody has costed.** Identical page breaks across devices requires identical font files, identical hinting and identical rasterisation everywhere. That means shipping your own font stack with a defined fallback chain, plus subsetting and embedding — and the metric-compatible substitutes for Calibri, Cambria, Times New Roman and Arial are a licensing question with real money attached.

**The unglamorous eighty percent is unaddressed.** IME composition for Chinese, Japanese and Korean over a custom canvas is notoriously painful and is a hard requirement for the Asian market. Add to that text selection semantics, spellcheck and autocorrect, RTL editing, clipboard interop with native apps in HTML/RTF/OOXML flavours, drag-and-drop, and printing. Collectively this is more engineering than the formula engine.

**Comment anchoring and tracked changes under concurrent editing** are each a research problem in their own right. A comment anchored to a text range must survive concurrent edits to that range on another replica. Tracked changes as a CRDT-native construct — rather than as an overlay — is genuinely novel work.

**History and compaction are in direct tension.** "Operation-level history forever" and "garbage-collect tombstones to control memory" cannot both be true. You need a defined model: hot recent operations, periodic compacted snapshots, cold archived history, and an explicit statement of what granularity survives compaction.

**No performance budgets exist.** Before writing code, fix numbers and treat them as CI gates: keystroke-to-paint latency (target under 16 ms at p95), cold document open time by size, recalculation time per 100k dependent cells, memory per open document, sync operation propagation latency, and WASM bundle size. Without these, performance regresses continuously and invisibly.

Also missing: PDF export fidelity specifics (PDF/A conformance, colour management, CMYK for print), observability and telemetry design including privacy-preserving crash reporting for a product whose selling point is that you cannot read customer data, and a formula-engine compatibility strategy for Excel's *bugs* — which are load-bearing in real models and must be reproduced behind a compatibility flag.

---

## 8. Consolidated gap register

Items absent or single-clause in the original proposal, ranked by risk to the programme.

| # | Gap | Owner | Risk |
|---|---|---|---|
| 1 | No commercial wedge, ICP, pricing or distribution strategy | Founder / CBO | Fatal |
| 2 | Accessibility (a11y tree, screen readers, VPAT, EAA/508) | CTO / Architect | Fatal for target market |
| 3 | CRDT-for-spreadsheet feasibility unproven | CTO / Principal | Fatal if wrong |
| 4 | E2EE contradicts server-side features and legal hold / DLP / eDiscovery | CISO | Severe |
| 5 | WASM 4 GB limit and Safari/iOS performance break headline claims | Principal | Severe |
| 6 | Internationalisation: IME, bidi, CJK, Indic, complex scripts | Principal | Severe |
| 7 | CRDT schema evolution and offline-replica migration | COO / Architect | Severe |
| 8 | Command-bus ↔ CRDT source-of-truth contract undefined | Architect | Severe |
| 9 | Key management: rotation, revocation, recovery, escrow | CISO | Severe |
| 10 | Compound documents (embedding, linking) not modelled | Architect | High |
| 11 | Collaborative undo, comment anchoring, tracked changes | Principal | High |
| 12 | On-prem support economics, LTS, upgrade tooling | COO | High |
| 13 | Font licensing and metric-compatible substitution | Principal / CBO | High |
| 14 | Legacy binary formats (.doc/.xls/.ppt) unaddressed | CTO / CISO | High |
| 15 | Formal threat model and trust boundaries | CISO | High |
| 16 | Open-source vs proprietary licence decision | Founder / CBO | High |
| 17 | Recalculation determinism for volatile functions | Architect | Medium |
| 18 | No performance budgets or CI performance gates | Principal | Medium |
| 19 | Parser process isolation; sanitisation of preserved blobs | CISO | Medium |
| 20 | Engine API versioning across independently-updating shells | Architect | Medium |
| 21 | Supply chain: cargo-deny, vendoring, SLSA provenance | CISO | Medium |
| 22 | Mobile native path (UniFFI) diverges from web shell | CTO | Medium |
| 23 | Migration tooling and services for legacy corpora | COO / CBO | Medium |
| 24 | Observability and privacy-preserving telemetry | Principal | Medium |
| 25 | Backup, DR and point-in-time restore for CRDT stores | COO | Medium |
| 26 | PDF/A, colour management, print fidelity | Principal | Low |
| 27 | Org design, hiring plan, unit economics | COO | Ongoing |
| 28 | ADR practice and decision governance | Architect | Ongoing |

---

## 9. Recommended changes before freeze

**Reframe the mission.** Replace "beat MS Office" with something defensible and sequenced — the agent-native, sovereign document substrate, starting with the spreadsheet. Word and PowerPoint follow, funded by the wedge.

**Insert a de-risking phase.** Ten weeks, three spikes, hard go/no-go criteria: CRDT spreadsheet convergence and tombstone growth at 100k+ cells with concurrent structural edits; WASM memory and performance on Safari/iOS with a realistic workbook; and screen-reader drivability of a canvas-rendered grid. The architecture is not frozen until these three report.

**Promote three concerns to P0 architectural constraints** rather than later workstreams: accessibility, internationalisation, and the offline-replica compatibility contract. All three are cheap to design in and effectively impossible to retrofit.

**Withdraw or soften four overclaims:** byte-faithful round-trip becomes semantically lossless; 10M rows in the browser becomes 10M rows with a defined server-compute path; better typography than Word becomes Word-compatible typography; and identical engine everywhere becomes identical engine with a documented mobile compilation target.

**Resolve the E2EE contradiction explicitly** with a two-tier model and a documented, honest capability matrix per tier — including the legal-hold and DLP position.

**Add five documents to the deliverable set:** `17-threat-model.md`, `18-accessibility.md`, `19-internationalization.md`, `20-performance-budgets.md`, and `21-adr/` with the first ten decisions recorded. And a `00-strategy.md` that precedes the vision document, containing the wedge, ICP, pricing and licence decision.

---

## 10. Closing assessment

The architecture is better than most funded attempts in this category, and several choices — the single engine, the command bus, the codec boundary — are ones Microsoft structurally cannot copy without a rewrite they will never authorise. That is real, and it is worth building on.

But the proposal currently reads as an engineering fantasy football team: every choice is individually defensible and the assembled whole is unbuildable by any organisation that does not already have five hundred engineers and a decade. The corrective is not to lower ambition on the architecture — it is to radically narrow the *scope of v1* while keeping the architecture intact, so that the good bones get built once, get paid for, and get extended.

Freeze the architecture. Do not freeze the scope until the three spikes report.
