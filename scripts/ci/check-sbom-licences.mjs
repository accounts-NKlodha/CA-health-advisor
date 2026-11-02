// Licence check over a CycloneDX JSON SBOM (tracked work W3; docs/Licence_Register.md).
//
//   node scripts/ci/check-sbom-licences.mjs <sbom.json> <allowed licence>...
//
// Every component (nested ones too) must carry a licence id, or an SPDX expression of
// ids joined by OR, of which at least one is allowed. A component with no licence, or
// with AND / WITH terms not all allowed, fails. Prints each component and exits 1 on any
// failure. Plain Node, no packages: the CI job that runs it is the supply-chain check.
import { readFileSync } from 'node:fs';

const [file, ...allowedList] = process.argv.slice(2);
if (!file || allowedList.length === 0) {
  console.error('usage: check-sbom-licences.mjs <sbom.json> <allowed licence>...');
  process.exit(2);
}
const allowed = new Set(allowedList);

// "A OR (B AND C)" -> acceptable if any OR-branch has every AND-term allowed. Parentheses
// are only stripped at the edges; nested expressions beyond that fail closed.
export function expressionAllowed(expr, ok) {
  const text = expr.trim().replace(/^\((.*)\)$/, '$1');
  if (/[()]/.test(text)) return false;
  return text
    .split(/\s+OR\s+/)
    .some((branch) => branch.split(/\s+AND\s+/).every((term) => ok.has(term.trim())));
}

function licencesOf(c) {
  const out = [];
  for (const l of c.licenses ?? []) {
    if (l.expression) out.push(l.expression);
    else if (l.license?.id) out.push(l.license.id);
    else if (l.license?.name) out.push(l.license.name);
  }
  return out;
}

const sbom = JSON.parse(readFileSync(file, 'utf8'));
let failures = 0;
let count = 0;
function walk(components) {
  for (const c of components ?? []) {
    count += 1;
    const name = `${c.group ? `${c.group}/` : ''}${c.name}@${c.version}`;
    const lic = licencesOf(c);
    const ok = lic.length > 0 && lic.some((e) => expressionAllowed(e, allowed));
    console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}: ${lic.join(' | ') || '(no licence)'}`);
    if (!ok) failures += 1;
    walk(c.components);
  }
}
walk(sbom.components);
console.log(`${count} components, ${failures} not allowed`);
if (count === 0) {
  console.error('the SBOM lists no components');
  process.exit(1);
}
process.exit(failures === 0 ? 0 : 1);
