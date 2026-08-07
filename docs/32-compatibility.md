# 32 — Compatibility Strategy
Status: Approved · Owner: Compatibility Engineer · Normative: yes

## The posture
Compatibility is a **measured profile, not an identity**. `compat` profile reproduces Excel behavior including enumerated bugs (1900 leap-year, 15-digit display rounding, date systems, SUM accumulation, coercion rules); `native` profile is free to be better (Decimal currency, strict coercion, error provenance). Imported workbooks default `compat`; new workbooks default `native` with a per-workbook switch. Never conflate the two in one document silently.

## The oracle (ADR-024)
The conformance spec is **captured from real Excel via COM automation**: a Windows harness drives Excel across versions recording input→output vectors for the full function catalog × edge-case grid (types, errors, coercions, boundary values). The captured corpus is versioned, and is the test oracle — because the documentation lies and the binary doesn't. Capture starts Q1 week 3 and runs continuously. Google Sheets divergences are documented where users will hit them (import advisories), not chased.

## Fidelity engineering
XLSX round-trip: semantic-lossless with preservation (docs/24); corpus-gated per release; **the fidelity number is published** per release with the top regression list. Function conformance: published percentage against the oracle corpus. Both numbers are product features and marketing assets — honesty as strategy.

## Interop behaviors that make or break adoption
Clipboard fidelity with Excel (formats, formulas, tables — both directions, corpus-tested); file association + "open in Excel and back" round-trip stability (unknown-part preservation makes this survivable); keyboard parity (Excel default keymap, remappable — muscle memory is a migration feature); formula-text compatibility (localized separators, function-name localization with canonical storage).

## Versioned compatibility (our own)
Container files forward-open forever (docs/10 preservation rules); N−2 wire support; LTS desktop channel for enterprises (18-month, docs/34); deprecation policy per docs/20.
