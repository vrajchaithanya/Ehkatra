# 33 — Cross-Platform & Desktop Strategy
Status: Approved · Owner: Chief Architect · Normative: yes · New for desktop-first mandate (ADR-027/028)

## Priority order
**Windows and macOS are the product.** Linux: kernel + server first-class (the server plane runs on it); desktop preserved through the PAL seam, shipped only on demonstrated demand. Web: future target — the wgpu scene graph maps to WebGPU and the kernel to wasm32 (kept honest by the permanent wasm32 differential-replay gate), but no web product before desktop earns its users. Mobile: Horizon 3 via UniFFI.

## Shell architecture (ADR-028)
Native windowed app: **winit + wgpu** for the document surface (our renderer is the craft, docs/31), with **platform adapters** for everything users judge as "native": menus (Win32 menu bar / NSMenu), dialogs (IFileDialog / NSSavePanel), title bar & window chrome per HIG, drag-and-drop, file associations, jump lists / dock menus, notifications, quick-look/thumbnail providers, Spotlight/Windows Search metadata indexing of workbook names+sheet names (never cell content without opt-in). *Not* a webview: Excel-grade latency, memory, and IME behavior are the reasons users stay on desktop; a webview forfeits all three.

## Platform integration contracts (each has an acceptance checklist in docs/48)
**IME:** native composition via the in-cell editor overlay (TSF on Windows, NSTextInputClient on macOS) — never reimplemented. **Accessibility:** one kernel a11y tree → accesskit → UIA (Windows: Narrator/JAWS/NVDA) and NSAccessibility (VoiceOver); grid semantics native ("B4, Revenue March, $12,400, formula, 1 comment"); Excel-parity keyboard map; WCAG 2.2 AA + VPAT before first enterprise sale. **Displays:** per-monitor DPI, fractional scaling, ProMotion/120 Hz, mixed-DPI window drag. **Power:** background calc throttles on battery; presence heartbeats coalesce. **Sync-managed folders** (OneDrive/iCloud/Dropbox): the container detects sync-manager presence, uses safe-copy + advisory locks, and warns on concurrent-open-via-cloud rather than corrupting (a real-world failure mode Excel handles badly; we handle it explicitly).

## The portability discipline
Kernel purity (`no_std`, PAL-only OS access) is enforced by CI, which means "add a platform" = "implement 10 traits + a shell." That claim is kept true by the wasm32 gate and the Linux server plane — two live non-desktop targets exercising the seam continuously. Platform-specific code lives only in `shell/` and `pal/` trees; a grep for `#[cfg(target_os)]` outside them fails CI.
