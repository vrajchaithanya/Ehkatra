# 03 — Architecture Overview & System Context
Status: Approved · Owner: Chief Architect · Normative: yes

## System context

```
        ┌────────────┐   OIDC    ┌─────────────┐
        │ Identity   │◄─────────►│             │        ┌──────────────┐
        │ Provider   │           │   SERVER    │◄──────►│ Customer     │
        └────────────┘           │   PLANE     │  SQL/  │ data systems │
┌──────────────┐  WS(ops)+HTTPS  │ relay·API·  │  REST  └──────────────┘
│ DESKTOP APP  │◄───────────────►│ MCP·calc-   │
│ Win / macOS  │                 │ authority·  │        ┌──────────────┐
│ (full kernel)│                 │ audit·store │◄──────►│ SIEM / audit │
└──────────────┘                 └──────┬──────┘        └──────────────┘
┌──────────────┐    MCP/HTTPS          │
│ AI agents &  │◄──────────────────────┘
│ integrations │     (server MCP; desktop exposes local MCP too)
└──────────────┘
```

The desktop app embeds the **full kernel** — it is never a thin client. The server plane runs the **same kernel headless** plus services. Offline desktop is fully functional; the server adds collaboration, API/MCP hosting, external data, and governance.

## Layering (normative dependency direction)

```
Shell (Win/macOS window, menus, dialogs, file assoc, updater)
  → Presentation (wgpu scene renderer · a11y tree · IME host · input)
    → Command API (one vocabulary: UI = REST = MCP)
      → Reducer (versioned, pure, compiles Command → Ops once, at author)
        → Kernel state (CRDT model · tiles · formulas · calc · explain)
          → PAL (clock, entropy, storage, net, compute, gpu, fonts, a11y, ime, securestore)
```

Rules: no layer reaches around the one below it; state mutates only via the op applier (visibility-enforced); the kernel is `no_std + alloc` with zero OS dependencies; wall-clock time and randomness are injected, never sampled.

## The one invariant that organizes everything
**The op log is the single source of truth.** State, sync, undo, version history, audit, AI attribution, and caches (all watermarked folds) derive from it. Any feature that cannot be expressed as ops + folds is redesigned until it can.

## Component inventory → owning doc
Kernel/PAL → 10 · Workbook model, undo → 11 · Formula → 12 · Calc → 13 · Storage → 14 · CRDT/sync/presence/offline → 15 · Snapshots/recovery/autosave → 16 · APIs → 20 · MCP → 21 · AI plane → 22 · Plugin SDK → 23 · Import/export → 24 · Desktop shell & platform integration → 33 · Server plane deployment → 36.
