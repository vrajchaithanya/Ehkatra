// dep-budget — enforces DP-S2 (docs/07 §3), the complexity budget. The rule is
// the single most important solo constraint, so it is a gate, not a poster.
//
// Two budgets, because they measure different debts and docs/07 only ever
// stated the first (see D-035):
//   * DIRECT   — crates you chose. Each is a decision you must justify.
//   * CLOSURE  — every external crate that actually compiles into the kernel,
//                including build-scripts. This is the real supply-chain surface,
//                and it is the number an attacker cares about.
//
// Usage: node tools/dep-budget.mjs
import { execFileSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { join } from 'node:path';

// Every `no_std` crate under `crates/`. The list was missing `usk-sync` and
// `usk-recover` from the sessions that added them — harmless while both carry
// zero dependencies, but a budget that does not list a crate is a budget that
// would not notice the day it grew one.
const KERNEL = [
  'usk-types',
  'usk-oplog',
  'usk-state',
  'usk-formula',
  'usk-calc',
  'usk-reduce',
  'usk-sync',
  'usk-recover',
  'usk-json',
  'usk-csv',
  'usk-zip',
  'usk-xml',
  'usk-xlsx',
  'usk-mcp',
];
const KERNEL_DIRECT_MAX = 5; // DP-S2 as written in docs/07 §3
const KERNEL_CLOSURE_MAX = 12; // D-035; today 10 (blake3's build+SIMD support crates)
const WORKSPACE_CLOSURE_MAX = 40; // DP-S2 — the NON-shell workspace.

// The shell is budgeted separately (ADR-037). DP-S2's intent is "one of each
// hard thing" and "what a solo maintainer can carry"; a GPU stack is one hard
// thing, bought whole from upstream, behind the platform boundary DP-C2's gate
// already enforces. Counting it against the kernel's neighbours would say
// nothing useful about either.
//
// The shell is a **separate workspace** with its own lockfile (ADR-037
// Amendment 1, D-116), so it is measured by resolving that workspace rather
// than by filtering this one. As a member it pushed the kernel closure from 10
// to 13 through Cargo's global version unification; from outside, it cannot
// reach the kernel's graph at all.
const SHELL_WORKSPACE = 'shell';
// Set from the measurement, not from a guess (D-115). `winit` + `wgpu` plus the
// registry closure of the kernel crates the shell depends on measured **230**
// (MEASUREMENTS.md). The ceiling is **280**: the measured 230 plus ~50 for the
// platform adapters docs/33 already names and Q2 still owes — accesskit for the
// a11y tree, and the file-dialog and menu adapters — and nothing else.
//
// Deliberately not generous beyond that. DP-S2's purpose is to make growth
// visible and deliberate, so the next raise should cost an ADR; but a ceiling
// set so tight that the first planned adapter breaches it would make the gate a
// nuisance rather than a control, and nuisances get raised without thought.
const SHELL_CLOSURE_MAX = 280;

const meta = JSON.parse(
  execFileSync('cargo', ['metadata', '--format-version', '1'], {
    encoding: 'utf8',
    maxBuffer: 64 * 1024 * 1024,
  }),
);

const workspaceIds = new Set(meta.workspace_members);
const byId = new Map(meta.packages.map((p) => [p.id, p]));
const nodes = new Map(meta.resolve.nodes.map((n) => [n.id, n]));
const memberByName = new Map(
  meta.packages.filter((p) => workspaceIds.has(p.id)).map((p) => [p.name, p]),
);

/** Non-dev dependency edges out of `id`; dev-deps never ship, so they never count. */
function shippingDeps(id) {
  return (nodes.get(id)?.deps ?? []).filter((dep) => {
    const kinds = dep.dep_kinds.map((k) => k.kind);
    // `null` is cargo's encoding for a normal dependency.
    return kinds.includes(null) || kinds.includes('build');
  });
}

/** External (non-workspace) packages transitively reachable from `rootId`. */
function externalClosure(rootId) {
  const seen = new Set();
  const out = new Set();
  const stack = [rootId];
  while (stack.length) {
    const id = stack.pop();
    if (seen.has(id)) continue;
    seen.add(id);
    if (!workspaceIds.has(id)) out.add(byId.get(id).name);
    for (const dep of shippingDeps(id)) stack.push(dep.pkg);
  }
  return out;
}

let failed = false;
const report = (label, set, max) => {
  const ok = set.size <= max;
  if (!ok) failed = true;
  console.log(
    `${ok ? 'PASS' : 'FAIL'}  ${label}: ${set.size}/${max}  [${[...set].sort().join(', ')}]`,
  );
};

const kernelDirect = new Set();
const kernelClosure = new Set();
for (const name of KERNEL) {
  const pkg = memberByName.get(name);
  if (!pkg) {
    console.error(`dep-budget: kernel crate '${name}' not found in workspace`);
    failed = true;
    continue;
  }
  for (const dep of shippingDeps(pkg.id)) {
    if (!workspaceIds.has(dep.pkg)) kernelDirect.add(byId.get(dep.pkg).name);
  }
  for (const d of externalClosure(pkg.id)) kernelClosure.add(d);
}

const workspaceClosure = new Set();
for (const id of workspaceIds) for (const d of externalClosure(id)) workspaceClosure.add(d);

/** External packages the shell's own workspace resolves to. */
function shellClosureOf(dir) {
  if (!existsSync(join(dir, 'Cargo.toml'))) return null;
  const meta = JSON.parse(
    execFileSync('cargo', ['metadata', '--format-version', '1'], {
      cwd: dir,
      encoding: 'utf8',
      maxBuffer: 64 * 1024 * 1024,
    }),
  );
  const members = new Set(meta.workspace_members);
  // Path dependencies on the kernel are workspace-*local* to this repo even
  // though they are not members of the shell's workspace, so they are excluded
  // by source: a registry source is a crate we did not write.
  return new Set(
    meta.packages.filter((p) => !members.has(p.id) && p.source).map((p) => p.name),
  );
}

const shellClosure = shellClosureOf(SHELL_WORKSPACE);

report('kernel direct deps (DP-S2)', kernelDirect, KERNEL_DIRECT_MAX);
report('kernel dep closure (D-035)', kernelClosure, KERNEL_CLOSURE_MAX);
report('workspace dep closure, non-shell (DP-S2)', workspaceClosure, WORKSPACE_CLOSURE_MAX);

if (shellClosure === null) {
  console.log('INFO  shell workspace absent — nothing to budget');
} else {
  // Names are not listed: 200 of them is a wall of text nobody reads, and the
  // number plus the ceiling is what the gate is for. `cargo tree` in `shell/`
  // answers "which ones" when that is the actual question.
  const ok = shellClosure.size <= SHELL_CLOSURE_MAX;
  if (!ok) failed = true;
  console.log(
    `${ok ? 'PASS' : 'FAIL'}  shell dep closure (ADR-037): ` +
      `${shellClosure.size}/${SHELL_CLOSURE_MAX}  [separate workspace, D-116]`,
  );
}

if (failed) {
  console.error('\ndep-budget: complexity budget exceeded — raising a ceiling requires an ADR.');
  process.exit(1);
}
