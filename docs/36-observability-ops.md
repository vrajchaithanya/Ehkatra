# 36 — Observability, DR & Operational Readiness
Status: Approved · Owner: SRE Lead · Normative: yes · Carved from SPEC §27, §30

## Observability
OpenTelemetry end-to-end: a Command carries trace context gesture → reducer → relay → calc → paint ("why was that edit slow" = one trace). Kernel emits via a no_std tracing facade. SLO metrics: op admission latency, sync propagation p95, calc generation lag, tile-fetch per tier, **promotion rate** (the watched assumption), conflict rate, crash-free session rate, restore-drill pass rate. Desktop telemetry: opt-in, schema-typed, **content-free by construction** (the taint lint: `Value` types cannot reach telemetry encoders); crash reports scrubbed (no cell content in minidumps — heap redaction pass before upload, user-inspectable before send).

## Diagnostics (support economics)
In-product doctor: workbook health (SCC clusters, volatile density, promotion hotspots, dep-depth — the "why is my workbook slow" answer, actionable); support bundles (structure metadata only, user-reviewed); **replay bundles**: a reported calc bug ships as op-log slice + state hash = deterministic reproduction (P2 pays for itself here).

## Server operations
SLOs: 99.9% API availability (GA), sync p95 <150 ms in-region; error budgets govern release pace. Runbooks (versioned in-repo): relay failover, doc-store degradation, poison-op quarantine (an op that crashes appliers is quarantined by hash + hotfix path — clients skip-and-report rather than crash-loop), key rotation, tenant migration. Incident practice: sev levels, paging, blameless postmortems with action-item tracking; status page with honest granularity.

## Disaster recovery
Server: continuous automated restore verification (a sampled workbook restored and hash-verified hourly); region failure = documented RTO 30 min / RPO ≤ 1 s acked (replicated relay writes); backup encryption keys escrowed separately from data keys. Desktop: container WAL + crash-injection tests (docs/16); "the laptop died" story = reinstall + open file + full history intact; "the file corrupted" story = salvage path with honest report.

## Operational readiness gate (before GA — checklist in docs/48)
On-call rotation staffed and drilled; runbooks exercised in game days; restore drills green 30 consecutive days; poison-op path tested in production-shape staging; support→engineering escalation with replay bundles proven on 10 real betas.
