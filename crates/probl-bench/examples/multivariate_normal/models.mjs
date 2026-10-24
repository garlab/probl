// Integrated models in fresh WASM modules; native output parity on every run.
// Build probl-cli (release) and probl-wasm (wasm profile) first, then:
// bun crates/probl-bench/examples/multivariate_normal/models.mjs
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {readFile} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
import {load} from '../../../../web/src/probl.js';

const root = new URL('../../../../', import.meta.url);
const wasm = await readFile(new URL('target/wasm32-unknown-unknown/wasm/probl_wasm.wasm', root));
const cli = fileURLToPath(new URL('target/release/probl', root));
// Alternate the two fixed-parameter formulations before the scaling cases.
for (const [name, runs] of [
  ['fixed',10000], ['shared',10000], ['shared',10000], ['fixed',10000],
  ['changing',10000], ['changing',10000], ['fixed',100000], ['changing',100000],
]) {
  const path = new URL(`benches/multivariate_${name}.probl`, root);
  const source = await readFile(path, 'utf8');
  const api = await load(wasm);
  const request = {source, runs, seed:42, today:'2026-10-24', report_collections:{0:'moments'}};
  const warmup = api.run({...request, runs:200});
  assert.equal(warmup.error, undefined, JSON.stringify(warmup.error));
  const start = performance.now();
  const result = api.run(request);
  const milliseconds = performance.now() - start;
  assert.equal(result.error, undefined, JSON.stringify(result.error));
  assert.equal(result.reports_error, undefined, JSON.stringify(result.reports_error));
  const native = execFileSync(cli, ['run',fileURLToPath(path),'--runs',String(runs),'--seed','42','--threads','1','--today','2026-10-24','--collect','0=moments'], {encoding:'utf8'});
  assert.equal(result.output.trimEnd(), native.trimEnd());
  console.log(JSON.stringify({name,runs,milliseconds,wasm_memory_bytes:api.memoryBytes()}));
}
