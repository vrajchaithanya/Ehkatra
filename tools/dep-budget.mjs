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
// Members whose name starts with this prefix are the shell.
const SHELL_PREFIX = 'ehkatra-shell';
// `null` means **unmeasured, and therefore unenforced**: ADR-037 requires the
// real closure to be measured and recorded in MEASUREMENTS.md before a ceiling
// is written here. D-115 is why — a number priced without measurement is what
// made the first stamp sidecar 21x too large. The gate reports the figure from
// the first day a shell crate exists, so the ceiling is set from evidence.
const SHELL_CLOSURE_MAX = null;

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

const isShell = (id) => (byId.get(id)?.name ?? '').startsWith(SHELL_PREFIX);

const workspaceClosure = new Set();
for (const id of workspaceIds) {
  if (isShell(id)) continue;
  for (const d of externalClosure(id)) workspaceClosure.add(d);
}

const shellClosure = new Set();
for (const id of workspaceIds) {
  if (!isShell(id)) continue;
  for (const d of externalClosure(id)) shellClosure.add(d);
}
// A crate the non-shell workspace already carries is not a cost the shell
// added, so the shell line measures what the shell *brought*.
for (const name of workspaceClosure) shellClosure.delete(name);

report('kernel direct deps (DP-S2)', kernelDirect, KERNEL_DIRECT_MAX);
report('kernel dep closure (D-035)', kernelClosure, KERNEL_CLOSURE_MAX);
report('workspace dep closure, non-shell (DP-S2)', workspaceClosure, WORKSPACE_CLOSURE_MAX);

if (SHELL_CLOSURE_MAX === null) {
  // Reported, not enforced. Saying which it is matters: a gate that prints a
  // number nobody set is not a gate, and pretending otherwise is how a budget
  // becomes decorative.
  console.log(
    `INFO  shell dep closure (ADR-037): ${shellClosure.size}, ceiling UNSET — ` +
      'measure and record in MEASUREMENTS.md before setting one',
  );
} else {
  report('shell dep closure (ADR-037)', shellClosure, SHELL_CLOSURE_MAX);
}

if (failed) {
  console.error('\ndep-budget: complexity budget exceeded — raising a ceiling requires an ADR.');
  process.exit(1);
}
