// Package the already tested browser distribution. JSON/base64 avoids archive
// extraction semantics; consumers accept an exact file allowlist.
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { resolve, join } from 'node:path';
import { pathToFileURL } from 'node:url';
const [tag, output] = process.argv.slice(2);
if (!/^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(tag ?? '') || !output) throw new Error('Usage: bun ci/browser.mjs vX.Y.Z OUTPUT');
const root = resolve(import.meta.dir, '..');
// All of web/dist, or the release fails here, before it's immutable. The
// playground's probl.wasm is the engine's, packaged once.
const playground = ['index.html', 'guide.html', 'app.js', 'worker.js', 'style.css'];
const built = readdirSync(join(root, 'web/dist')).sort();
if (built.join() !== [...playground, 'probl.wasm'].sort().join()) throw new Error(`web/dist has ${built.join(', ')}: update the bundle's files, and its consumers'`);
// The reference results' date, for the examples that depend on it.
const today = '2026-09-29';
const commit = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim();
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const { load } = await import(pathToFileURL(join(root, 'web/src/probl.js')));
const wasm = readFileSync(join(root, 'web/dist/probl.wasm'));
const engine = await load(wasm);
if (`v${engine.version()}` !== tag) throw new Error('WASM version differs from release tag');
const cases = engine.examples().map(example => {
  const output = execFileSync(join(root, 'target/release/probl'), ['run', `examples/${example.name}.probl`, '--today', today], { cwd: root, encoding: 'utf8' });
  const result = engine.run({ source: example.source, files: example.files, today });
  if (`${result.output}\n` !== output) throw new Error(`WASM/native mismatch: ${example.name}`);
  return { ...example, today, output };
});
const bytes = {
  'engine/probl.wasm': wasm,
  'engine/probl.js': readFileSync(join(root, 'web/src/probl.js')),
  'engine/probl-lang.js': readFileSync(join(root, 'web/src/probl-lang.js')),
  'cases.json': Buffer.from(JSON.stringify(cases)),
};
for (const name of playground) bytes[`playground/${name}`] = readFileSync(join(root, `web/dist/${name}`));
for (const name of ['CHANGELOG.md', 'LICENSE-MIT', 'LICENSE-APACHE']) bytes[name] = readFileSync(join(root, name));
const files = Object.fromEntries(Object.entries(bytes).map(([name, data]) => [name, { sha256: hash(data), base64: data.toString('base64') }]));
const bundle = Buffer.from(JSON.stringify({ schema: 1, tag, commit, files }) + '\n');
const name = `probl-${tag.slice(1)}-browser.json`;
mkdirSync(output, { recursive: true });
writeFileSync(join(output, name), bundle);
writeFileSync(join(output, `${name}.sha256`), `${hash(bundle)}  ${name}\n`);
console.log(`Packaged ${name} (${bundle.length} bytes) from ${commit}`);
