// After building the example for release and wasm32-unknown-unknown:
// bun crates/probl-bench/examples/multivariate_normal/replay.mjs
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

const root = new URL('../../../../', import.meta.url);
const native = execFileSync(fileURLToPath(new URL('target/release/examples/multivariate-normal', root)), ['--replay'], {encoding:'utf8'});
const bytes = await readFile(new URL('target/wasm32-unknown-unknown/wasm/examples/multivariate-normal.wasm', root));
const { instance } = await WebAssembly.instantiate(bytes, {});
for (const line of native.trim().split('\n')) {
  const { seed, fingerprint } = JSON.parse(line);
  const actual = BigInt.asUintN(64, instance.exports.replay_fingerprint(seed)).toString(16).padStart(16, '0');
  assert.equal(actual, fingerprint, `native/WASM factor and draw replay, seed ${seed}`);
  console.log(`same  seed ${seed}: ${actual}`);
}
