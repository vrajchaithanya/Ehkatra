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

const KERNEL = ['usk-types', 'usk-oplog', 'usk-state', 'usk-formula', 'usk-calc'];
const KERNEL_DIRECT_MAX = 5; // DP-S2 as written in docs/07 §3
const KERNEL_CLOSURE_MAX = 12; // D-035; today 10 (blake3's build+SIMD support crates)
const WORKSPACE_CLOSURE_MAX = 40; // DP-S2

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

report('kernel direct deps (DP-S2)', kernelDirect, KERNEL_DIRECT_MAX);
report('kernel dep closure (D-035)', kernelClosure, KERNEL_CLOSURE_MAX);
report('workspace dep closure (DP-S2)', workspaceClosure, WORKSPACE_CLOSURE_MAX);

if (failed) {
  console.error('\ndep-budget: complexity budget exceeded — raising a ceiling requires an ADR.');
  process.exit(1);
}
