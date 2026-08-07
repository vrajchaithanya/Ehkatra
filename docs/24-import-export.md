# 24 — Import/Export & Formats
Status: Approved · Owner: Compatibility Engineer · Normative: yes · Carved from SPEC §19

## Sandbox rule (no exceptions)
All parsing/serialization runs in an isolated subprocess (desktop: sandboxed child with job-object/sandbox-exec confinement; server: sandboxed fleet): seccomp/platform equivalent, no network, memory/CPU/wall caps, fresh process per document, **IR-only output revalidated against schema** by the host. Zip: streaming, entry/size/ratio caps (100:1). XML: DTD/external entities disabled, depth/node caps.

## Active content policy
Four-class ingest: inert markup → preserve verbatim; inert binary → preserve, decode only in sandbox; **active** (vbaProject, OLE, ActiveX, DDE) → quarantine to encrypted vault, never executed, never re-emitted by default (re-emission = permissioned, audited, org-deniable); network-fetching (external links, remote images, INDIRECT-to-URL) → neutralized to data, fetch requires user action + egress allowlist. The fidelity promise is stated with this carve-out up front.

## Format matrix (H1)
XLSX in/out (semantic-lossless round-trip; unknown parts preserved; fidelity = published per-release number over the corpus); CSV/TSV in/out (streaming; type-inference **preview before commit** — the gene-name bug is a surfaced decision, never silent; formula-injection neutralization on both import and export per OWASP); clipboard (HTML/plain/native — treated as a format boundary with the same sandbox, because paste is an attack channel). H2: ODS in/out, XLS/XLSB in (tightest sandbox), Parquet/Arrow, JSON/XML mapped, PDF out (print pipeline), HTML/Markdown. Native container ≠ interchange: the working file is the SQLite container (docs/14); XLSX is a boundary projection.

## Fidelity engineering
Corpus: thousands of real-world workbooks (licensed + donated + synthetic), round-tripped per release; gates: semantic diff + rendered-layout diff; per-file fidelity reports on legacy imports; the number is *published* — fidelity is a measured product attribute, not a hope.
