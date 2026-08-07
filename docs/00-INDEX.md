# Grid Platform — Document Index

Every document carries a header: `Status: Draft | Approved | Superseded` · `Owner: <role>` · `Normative: yes/no`. Normative documents bind implementation; conflicts between documents are defects — file them.

## Governance core
| # | Document | Owner | Status |
|---|---|---|---|
| 01 | Vision | Product Architect | Approved |
| 02 | Scope & Non-Goals | Product Architect | Approved |
| 03 | Architecture Overview & System Context | Chief Architect | Approved |
| 04 | Domain Model | Chief Architect | Approved |
| 05 | Architecture Flow & Feature Comparison | Chief Architect | Approved |
| 06 | **Design Principles — the complete rule set (authority)** | Chief Architect | Approved |
| 07 | Solo Operating Model & Review (incl. DP-S rules) | You | Approved |
| 08 | Engineering Handbook (process, self-review, releases) | You | Approved |
| 25 | UX Architecture (five surfaces, interaction rules) | You | Approved |
| 26 | Data Model & Container Schema Specs | You | Approved |
| 27 | State Machine Specifications | You | Approved |
| 28 | Error Handling Model | You | Approved |
| 29 | Determinism Verification Guide | You | Approved |
| 37 | Threat Model (STRIDE per boundary) | You | Approved |
| 38 | Benchmark Specification (workload ids) | You | Approved |
| — | templates/ (ADR, RFC) | You | Living |

## Module architecture (normative)
| # | Document | Owner | Status |
|---|---|---|---|
| 10 | Kernel Architecture (USK) | Distinguished Eng | Approved |
| 11 | Workbook Model & Undo | Principal Eng | Approved |
| 12 | Formula Engine | Principal Eng | Approved |
| 13 | Calculation Engine | Principal Eng | Approved |
| 14 | Storage Engine | Principal Eng | Approved |
| 15 | CRDT & Synchronization | Distributed Systems | Approved |
| 16 | Snapshot, Recovery & Autosave | Distributed Systems | Approved |
| 20 | API Design (3 layers) | API Designer | Approved |
| 21 | MCP Architecture | AI Platform Architect | Approved |
| 22 | AI Architecture | AI Platform Architect | Approved |
| 23 | Plugin SDK | Principal Eng | Draft |
| 24 | Import/Export & Formats | Compatibility Eng | Approved |

## Cross-cutting strategy (normative)
| # | Document | Owner | Status |
|---|---|---|---|
| 30 | Security Architecture | Security Architect | Approved |
| 31 | Performance Architecture & Budgets | Performance Eng | Approved |
| 32 | Compatibility Strategy | Compatibility Eng | Approved |
| 33 | Cross-Platform & Desktop Strategy | Chief Architect | Approved |
| 34 | Build, Packaging & Release | Build Eng | Approved |
| 35 | Testing, Benchmark & Validation | QA Architect | Approved |
| 36 | Observability, DR & Operational Readiness | SRE | Approved |

## Governance registers
| # | Document | Status |
|---|---|---|
| 40 | Roadmap & Implementation Plan | Approved |
| 41 | Risk Register | Living |
| 42 | Assumption Register | Living |
| 43 | Decision Register & ADR index | Living |
| 44 | Technical Debt Register | Living |
| 45 | Non-Functional Requirements | Approved |
| 46 | Glossary | Living |
| 47 | Engineering Scorecard | Living |
| 48 | Production Readiness Checklist | Living |
| 49 | Traceability Matrix | Living |
| — | CHANGELOG | Living |
| — | ARCHITECTURE-REVIEW-MEMO | Record |

**Archived:** SPEC-ARCHIVE (GRID-ARCHITECTURE-SPEC.md, carve source), DESIGN-V2-HARD-PROBLEMS.md, DOC-GRID-DESIGN.md, ARCHITECTURE-REVIEW.md.
