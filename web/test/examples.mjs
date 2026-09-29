// Every example must print in WebAssembly exactly what `probl run` prints
// natively: the same numbers on every platform (docs/semantics.md, section 14).
//
//   cargo build -p probl-wasm --target wasm32-unknown-unknown --profile wasm
//   cargo build --release -p probl-cli
//   node web/test/examples.mjs

import { execFileSync } from 'node:child_process';
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
for (const example of cases) {
  const start = performance.now();
  const answer = probl.run({ source: example.source, files: example.files });
  const time = performance.now() - start;
  const expected = execFileSync(cli, ['run', example.path], { cwd: root, encoding: 'utf8' });
  const actual = answer.output === undefined ? JSON.stringify(answer.error) : `${answer.output}\n`;
  const same = actual === expected;
  failed += !same;
  console.log(`${same ? 'same' : 'DIFFERENT'}  ${example.name.padEnd(24)} ${time.toFixed(0).padStart(6)} ms`);
  if (!same) {
    console.log(`--- probl run\n${expected}--- WebAssembly\n${actual}`);
  }
}
console.log(`memory: ${(probl.memoryBytes() / 1048576).toFixed(0)} MiB`);
process.exit(failed ? 1 : 0);
