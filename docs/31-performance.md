# 31 — Performance Architecture & Budgets
Status: Approved · Owner: Performance Engineer · Normative: yes (budgets are CI gates) · Carved from SPEC §15, §28

## Rendering architecture (desktop-first)
Retained scene graph → wgpu compositor (D3D12 on Windows, Metal on macOS): glyph-atlas text, instanced fills/borders/gridlines, damage-rect repaint only. View model is a watermarked fold (docs/14 caching law) — the render loop never walks the document. Numeric fast path (pre-shaped digit runs per style) ≈ halves paint cost in dense sheets. Shaping: rustybuzz + **bundled fonts only for layout metrics** (26.6 fixed-point) — layout determinism is metric-determinism; pixels may differ per rasterizer (DirectWrite/CoreText raster is fine; their *metrics* are not consulted). Virtual scroll: identity-anchored position (viewport never teleports under concurrent edits), order-statistic tree pixel↔identity O(log n), velocity prefetch. In-cell editor = native text field overlay at caret (IME correctness); a11y via accesskit (docs/33).

## The budget table (single source; all other docs link here) — p95, reference hw (mid-2023 laptops), CI-gated
| Metric | Target | Evidence |
|---|---|---|
| Keystroke→paint, 10k sheet | <16 ms | A-004, Q1 |
| Keystroke→paint incl. 10k-cell recalc | <50 ms | A-004, Q1 |
| Scroll frame | <8.3 ms, zero jank p99 | Q2 (renderer) |
| Cold open 1M-cell workbook (skeleton+viewport) | <1.5 s | Q1 |
| Full recalc 100k dependent cells (8-core) | <200 ms | A-003, Q1 |
| Incremental recalc, single edit | <8 ms | Q1 |
| `query` 1M-row grouped aggregate | <500 ms | Q1 (DataFusion) |
| Sync propagation same-region | <150 ms | Q2 |
| Local op durability | <250 ms | Q1 |
| Memory, 10M numeric cells | <400 MB | A-001/A-002, Q1 |
| Desktop cold launch → blank workbook | <1.0 s | Q2 |
| Battery, 1 h active editing (MBP/x86 laptop) | <8% | Q3 |

## Method
Budgets before code; benchmark suite = micro (op apply, tile codec, interval stab, shaping cache) + macro (calc corpus + real-workbook suite + adversarial: deep chains, volatile storms, spill lattices) + soak (24 h collab fuzz) + competitive harness (same ops scripted against Excel/Sheets, published honestly). Any gate breach blocks release; >5% macro regression needs sign-off + debt entry. Every number in this table carries an evidence tag: **measured** (link to run) or **target** (assumption id) — the tag *is* the honesty mechanism (Evidence Maturity, docs/47).
