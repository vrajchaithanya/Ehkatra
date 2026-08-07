# 04 — Domain Model
Status: Approved · Owner: Chief Architect · Normative: yes

## Entities and identity

| Entity | Identity | Nature |
|---|---|---|
| Workbook | `DocId` (uuid v7) | root aggregate; an op log |
| Sheet | `SheetId` | element of workbook's sheet order sequence |
| Row / Column | `RowId` / `ColId` | dense order identifiers (Fugue-family); never reused |
| Cell | `(SheetId, RowId, ColId)` | an intersection, not an object — no independent identity |
| Range | identity interval + `AnchorMode` | the reference primitive; A1 text is a *view* |
| Table | `TableId` | named region + column map (ColId ↔ name ↔ type contract) |
| Name | scoped string → target | workbook- or sheet-scoped |
| Style | interned flyweight id | value object, deduplicated |
| Comment thread | `ThreadId` anchored to cell | rich-text sequence |
| Version | `(DocId, watermark)` + optional name | immutable ref into the log |
| Principal | `ActorId` | human, service, or agent; every op attributes to one |
| Op / Op group | `(ActorId, Counter)` / `GroupId` | the atoms; groups = undo/audit units |

## Value lattice
`Blank · Bool · Number(f64) · Decimal(d128) · Text · Date · DateTime · Duration · Error(kind, origin-trace) · Array · Reference · Rich(entity) · Lambda(closure)` — coercion per mode (`compat`/`strict`); display formatting is a pure function, never feeds back into stored values.

## Invariants (checkable)
1. Every mutation is an op; every op has an actor; every op belongs to exactly one group.
2. Identity order is total and stable per axis; A1 = f(identity order) is derivable, never stored as truth.
3. A formula's references bind to identities at edit time (reducer); evaluation never performs address arithmetic.
4. Deleted identities tombstone; they never reincarnate; compaction may collect them only past the staleness window (180 days).
5. Errors propagate as values; `Error` carries origin; no evaluation path throws.
6. Spilled cells hold no stored value; spill is recomputed projection (ownership conflicts are therefore impossible).

## Ubiquitous language
Use *op* (not event), *group* (not transaction), *watermark* (not revision number), *materialize* (volatile → stored value), *promotion* (tile → per-cell CRDT metadata), *profile* (`compat`/`native` behavior selection). The glossary (46) is the arbiter; PRs using drifted terms fail review.
