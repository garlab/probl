// Compare a saved pre-follow-up WASM module with the current build, including
// exact text/report-document replay. Run from the repository root:
// bun crates/probl-bench/examples/multivariate_normal/followups.mjs BEFORE.wasm
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {load} from '../../../../web/src/probl.js';

assert.ok(process.argv[2], 'pass the saved baseline WASM module path');
const root = new URL('../../../../', import.meta.url);
const modules = {
  before: await readFile(process.argv[2]),
  after: await readFile(new URL('target/wasm32-unknown-unknown/wasm/probl_wasm.wasm', root)),
};
for (const [name, collection] of [
  ['multivariate_fixed', 'moments'],
  ['multivariate_changing', 'moments'],
  ['multivariate_shared', 'exact'],
  ['retirement_balance', 'exact'],
  ['retirement_balance', 'moments'],
  ['retirement_balance', 'quantiles'],
]) {
  const source = await readFile(new URL(`benches/${name}.probl`, root), 'utf8');
  const request = {source, runs:10000, seed:42, today:'2026-10-24', report_collections:{0:collection}};
  let baseline;
  for (const build of ['before', 'after', 'after', 'before']) {
    const api = await load(modules[build]);
    const warmup = api.run({...request, runs:200});
    assert.equal(warmup.error, undefined, JSON.stringify(warmup.error));
    const start = performance.now();
    const result = api.run(request);
    const milliseconds = performance.now() - start;
    assert.equal(result.error, undefined, JSON.stringify(result.error));
    assert.equal(result.reports_error, undefined, JSON.stringify(result.reports_error));
    const answer = {output:result.output, reports:result.reports};
    baseline ??= answer;
    assert.deepEqual(answer, baseline, `${name}/${collection}/${build}: answer changed`);
    console.log(JSON.stringify({name, collection, build, milliseconds, wasm_memory_bytes:api.memoryBytes()}));
  }
}
