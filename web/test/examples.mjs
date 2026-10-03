// Every example must print in WebAssembly exactly what `probl run` prints
// natively: the same numbers on every platform (docs/semantics.md, section 14).
//
//   cargo build -p probl-wasm --target wasm32-unknown-unknown --profile wasm
//   cargo build --release -p probl-cli
//   bun web/test/examples.mjs

import { execFileSync } from 'node:child_process';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { load } from '../src/probl.js';

const root = fileURLToPath(new URL('../..', import.meta.url));
const wasm = await readFile(`${root}/target/wasm32-unknown-unknown/wasm/probl_wasm.wasm`);
const cli = `${root}/target/release/probl`;

const probl = await load(wasm);
let failed = 0;
const cases = probl.examples().map((e) => ({ ...e, path: `examples/${e.name}.probl` }));
cases.push({ name: 'math', path: 'web/test/math.probl', source: await readFile(`${root}/web/test/math.probl`, 'utf8') });
cases.push({ name: 'text', path: 'web/test/text.probl', source: await readFile(`${root}/web/test/text.probl`, 'utf8') });
cases.push({ name: 'dates', path: 'web/test/dates.probl', source: await readFile(`${root}/web/test/dates.probl`, 'utf8') });
cases.push({ name: 'reporting', path: 'web/test/reporting.probl', source: await readFile(`${root}/web/test/reporting.probl`, 'utf8') });
cases.push({ name: 'types', path: 'web/test/types.probl', source: await readFile(`${root}/web/test/types.probl`, 'utf8') });
cases.push({
  name: 'integers', path: 'web/test/integers.probl',
  source: await readFile(`${root}/web/test/integers.probl`, 'utf8'),
  files: Object.fromEntries(await Promise.all(['json', 'csv', 'txt'].map(async (ext) => [
    `data/integers.${ext}`, await readFile(`${root}/web/test/data/integers.${ext}`, 'utf8'),
  ]))),
});
for (const example of cases) {
  const start = performance.now();
  const answer = probl.run({ source: example.source, today: '2026-09-29', files: example.files });
  const time = performance.now() - start;
  const expected = execFileSync(cli, ['run', example.path, '--today', '2026-09-29'], { cwd: root, encoding: 'utf8' });
  const actual = answer.output === undefined ? JSON.stringify(answer.error) : `${answer.output}\n`;
  const same = actual === expected;
  failed += !same;
  console.log(`${same ? 'same' : 'DIFFERENT'}  ${example.name.padEnd(24)} ${time.toFixed(0).padStart(6)} ms`);
  if (!same) {
    console.log(`--- probl run\n${expected}--- WebAssembly\n${actual}`);
  }
}

// Callback effects must fail through the WASM API in either mode too.
for (const mode of ['enumerate', 'sample']) {
  for (const source of [
    'report [1, 2].map(x -> { let r ~ d6; x + r })',
    'report [1].filter(x -> { observe true; true })',
    'report [1].count(x -> { let r ~ d1; true })',
    'report [1].map(x -> ~d6)',
    'report [1].map(x -> { var deck = bag([1]); deck.take() })',
    'report [1].map(x -> { score 50%; x })',
    'report [1].map(x -> if 30% { x } else { x })',
    'report [1].reduce(0, (a, x) -> chance { 50% => a, else => a })',
  ]) {
    const result = probl.run({ source, mode, ...(mode === 'sample' ? { runs: 10, seed: 19 } : {}) });
    assert.equal(result.error?.kind, 'language');
    assert.match(result.error?.message, /can't branch on chances, draw values or observe/);
  }
}
console.log('same  callback effect restrictions in both modes');

// Numbers, probabilities and distribution recipes are not boolean events.
for (const mode of ['enumerate', 'sample']) {
  for (const source of [
    'let p = 133%\nif p { report true }',
    'score true\nreport true',
    'let e = 5%\nobserve e\nreport e',
    'let x = 3d8\nobserve x > 10\nreport x',
    'let rate = -33%\nreport bernoulli(rate)',
    'report prob(d6)',
    'report prob(150%)',
    'report one_of([true: prob(33%), false: 67%])',
  ]) {
    const result = probl.run({ source, mode, ...(mode === 'sample' ? { runs: 10, seed: 19 } : {}) });
    assert.equal(result.error?.kind, 'language', source);
  }
}
console.log('same  explicit conversions and observation restrictions in both modes');

// Crossing UTC midnight between executions must change the default snapshot,
// while repeated references, calls and simulations in one execution share it.
const RealDate = globalThis.Date;
let clockReads = 0;
try {
  globalThis.Date = class extends RealDate {
    constructor(...args) {
      if (args.length) super(...args);
      else super(['2024-02-29T23:59:59.999Z', '2024-03-01T00:00:00.000Z'][clockReads++]);
    }
  };
  const source = 'fn anchor() { today }\nreport today\nreport anchor()\nreport simulate { let n ~ d6; today + n - n }';
  const first = probl.run({ source });
  assert.equal(first.today, '2024-02-29');
  assert.equal(first.output.match(/2024-02-29/g)?.length, 3);
  assert.equal(clockReads, 1);
  const second = probl.run({ source });
  assert.equal(second.today, '2024-03-01');
  assert.equal(second.output.match(/2024-03-01/g)?.length, 3);
  assert.equal(clockReads, 2);
  const replay = probl.run({ source, today: first.today });
  assert.equal(replay.output, first.output);
  assert.equal(clockReads, 2);
  assert.ok(probl.run({ source, today: null }).error);
  assert.equal(clockReads, 2);
} finally {
  globalThis.Date = RealDate;
}
console.log('same  execution-date snapshot and replay');
console.log(`memory: ${(probl.memoryBytes() / 1048576).toFixed(0)} MiB`);
process.exit(failed ? 1 : 0);
