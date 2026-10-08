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
cases.push({ name: 'api-contracts', path: 'web/test/api-contracts.probl', source: await readFile(`${root}/web/test/api-contracts.probl`, 'utf8') });
cases.push({ name: 'ordering', path: 'web/test/ordering.probl', source: await readFile(`${root}/web/test/ordering.probl`, 'utf8') });
cases.push({ name: 'statistics', path: 'web/test/statistics.probl', source: await readFile(`${root}/web/test/statistics.probl`, 'utf8') });
cases.push({ name: 'analytic', path: 'web/test/analytic.probl', source: await readFile(`${root}/web/test/analytic.probl`, 'utf8') });
cases.push({ name: 'math', path: 'web/test/math.probl', source: await readFile(`${root}/web/test/math.probl`, 'utf8') });
cases.push({ name: 'text', path: 'web/test/text.probl', source: await readFile(`${root}/web/test/text.probl`, 'utf8') });
cases.push({ name: 'dates', path: 'web/test/dates.probl', source: await readFile(`${root}/web/test/dates.probl`, 'utf8') });
cases.push({ name: 'reporting', path: 'web/test/reporting.probl', source: await readFile(`${root}/web/test/reporting.probl`, 'utf8') });
cases.push({ name: 'types', path: 'web/test/types.probl', source: await readFile(`${root}/web/test/types.probl`, 'utf8') });
cases.push({ name: 'catching', path: 'web/test/catching.probl', source: await readFile(`${root}/web/test/catching.probl`, 'utf8') });
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
    'report [2,1].sort((a,b) -> { let r ~ d1; a-b })',
    'report [2,1].sort_desc((a,b) -> { observe true; a-b })',
    'report [1].map(x -> { var deck = bag([1]); deck.take() })',
    'report [1].map(x -> { score 50%; x })',
    'report [1].map(x -> if 30% { x } else { x })',
    'report [1].reduce((a, x) -> chance { 50% => a, else => a }, 0)',
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

// Integer conversions reject fractions and invalid index types in both engines.
for (const mode of ['enumerate', 'sample']) {
  for (const source of [
    'let x=1.4; let n:int=x; report n',
    'let x=1.0000000000000002; let n:int=x; report n',
    'let xs=[10,20]; report xs.get(1.4,-1)',
    'report [].get("wrong",-1)',
    'report slice([10,20],0,1.4)',
    'report factorial(1.4)',
    'report bit_length(prob(1))',
    'report roll(1.4,d1)',
    'report date(2026,1,1).add_months(1.4)',
    'let n:dist[int]=one_of([1.0,1.4]); report n',
    'let m:map[int,str]=[1:"int",1.0:"float"]; report m',
    'let m:map[prob,str]=[1:"int",1.0:"float"]; report m',
  ]) {
    const result = probl.run({ source, mode, ...(mode === 'sample' ? { runs: 10, seed: 19 } : {}) });
    assert.equal(result.error?.kind, 'language', source);
    assert.match(result.error?.message, /int|collision/, source);
  }
  const result = probl.run({
    source: 'let x=1.0; let n:int=x; report typeof n == "int" and typeof x == "float" and [10,20].get(x,-1)==20',
    mode, ...(mode === 'sample' ? { runs: 10, seed: 19 } : {}),
  });
  assert.equal(result.error, undefined);
  assert.match(result.output, /100(?:\.0+)?%/);
}
console.log('same  checked integer conversion in both modes');

// Sorting and quantiles reject unordered values and invalid callbacks uniformly.
for (const mode of ['enumerate', 'sample']) {
  for (const source of [
    'report [complex(1)].sort()',
    'report [complex(2,3),complex(1,4)].sort()',
    'report [{x:1}].sort_desc()',
    'report [].sort(0)',
    'report [1].sort(x->0)',
    'report [2,1].sort((a,b)->a<b)',
    'report [2,1].sort((a,b)->d1)',
    'report [2,1].sort((a,b)->1/0)',
    'report quantile(one_of([1,"a"]),50%)',
    'report quantile(one_of([{x:1}]),0%)',
    'report quantile(one_of([complex(1)]),100%)',
  ]) {
    const result = probl.run({ source, mode, ...(mode === 'sample' ? { runs: 10, seed: 19 } : {}) });
    assert.equal(result.error?.kind, 'language', source);
  }
  const result = probl.run({
    source: 'let xs=[complex(2,3),complex(1,4)]; report xs.sort((a,b)->real(a)-real(b))==[complex(1,4),complex(2,3)]',
    mode, ...(mode === 'sample' ? { runs: 10, seed: 19 } : {}),
  });
  assert.equal(result.error, undefined);
  assert.match(result.output, /100(?:\.0+)?%/);
}
console.log('same  comparator sorting and ordered quantiles in both modes');

for (const mode of ['enumerate', 'sample']) {
  for (const source of [
    '@epsilon 0.1\nlet d=simulate {var n=0; while 50% {n+=1}; n}; report cdf(d,200)',
    '@epsilon 0.1\nlet d=simulate {var n=0; while 50% {n+=1}; n}; report pmf(one_of([d,uniform(100,101)]),0)',
    'report P(geometric(50%)>1)',
    'report [1:"a"][1.0]',
    'report maximum([complex(1)])',
    'report "abc".get(0.5,"?")',
    'report mean(1..0)',
    'report mean(lognormal(1000,1))',
    'report variance(normal(0,1e308))',
    'report quantile(normal(0,1),100%)',
    'report pdf(beta(0.5,1),0)',
    'report prob(1.0000000000000002)',
  ]) {
    const result = probl.run({ source, mode, ...(mode === 'sample' ? { runs: 10, seed: 19 } : {}) });
    assert.equal(result.error?.kind, 'language', source);
  }
}
console.log('same  remaining API contract validation in both modes');

// The old unary comparison/top-n forms are now compile errors.
for (const source of ['report min(d6)', 'report max([1,2])', 'report highest([1,2])']) {
  assert.ok(probl.run({source}).error, source);
}
for (const source of [
  'report maximum([d6,d8])',
  'report maximum(normal(0,1))',
  'report minimum(geometric(50%))',
  'report maximum(d6,(a,b)->a-b)',
  'fn cmp(a,b){let x ~ d1; a-b}; report [1,2].maximum(cmp)',
]) {
  assert.equal(probl.run({source}).error?.kind, 'language', source);
}

// Optional seeds and named defaults preserve errors and callback restrictions.
for (const mode of ['enumerate', 'sample']) {
  for (const source of [
    'report [].reduce(max)',
    'report [1,2].filter(x->false).reduce(max)',
    'report [].reduce(0,max)',
    'report [].reduce(abs,0)',
    'report [1,2].reduce((a,b)->{let r ~ d1; a+b})',
    'report [].maximum(abs,default:0)',
    'report maximum(normal(0,1),default:0)',
    'report minimum(geometric(50%),default:0)',
    'report [1].maximum(default:1/0)',
    'let f=maximum; report f([],unknown:0)',
  ]) {
    assert.equal(probl.run({source, mode, ...(mode === 'sample' ? {runs:10,seed:19} : {})}).error?.kind, 'language', source);
  }
}
for (const source of [
  'report maximum([],default:0,default:1)',
  'report maximum([],default:0,(a,b)->a-b)',
  'report maximum(default:0)',
  'report maximum([],fallback:0)',
]) {
  assert.ok(probl.run({source}).error, source);
}
console.log('same  optional reduction seeds and extrema defaults in both modes');

// Queries cannot silently turn a scalar or entire list into a point mass.
for (const mode of ['enumerate', 'sample']) {
  for (const source of [
    'report mean(pi)',
    'report median(pi)',
    'let x = if 50% { e } else { pi }\nprint(x, mean(x), median(x))',
    'let x ~ uniform(0, 2)\nreport mean(x)',
    'let x ~ uniform(0, 2)\nreport P(x > 1)',
    'report P(true)',
    'report P(prob(30%))',
    'report median(["aa","bb"])',
    'report median(one_of(["aa"]))',
    'report median_low(pi)',
    'report median_high(pi)',
    'report median_low([])',
    'report median_high([complex(1)])',
    'report median([complex(1, 2)])',
    'report median([])',
    'report mean([])',
    'report mean([d6, d8])',
  ]) {
    const result = probl.run({ source, mode, ...(mode === 'sample' ? { runs: 10, seed: 19 } : {}) });
    assert.equal(result.error?.kind, 'language', source);
  }
  const result = probl.run({
    source: 'report median([1,2,3,4]) as "middle"\nreport mean([1,1,4]) as "average"',
    mode, ...(mode === 'sample' ? { runs: 10, seed: 19 } : {}),
  });
  assert.equal(result.error, undefined);
  assert.match(result.output, /middle\s+2\.5/);
  assert.match(result.output, /average\s+2/);
}
console.log('same  statistics require populations and aggregate lists in both modes');

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
