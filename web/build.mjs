// Build the playground into web/dist: the WebAssembly module, and the page's
// scripts bundled with esbuild. `--serve` also serves it on port 8000.

import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync, rmSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import * as esbuild from 'esbuild';

const web = fileURLToPath(new URL('.', import.meta.url));
const root = fileURLToPath(new URL('..', import.meta.url));
const dist = `${web}dist`;

execFileSync('cargo', ['build', '-q', '-p', 'probl-wasm', '--target', 'wasm32-unknown-unknown', '--profile', 'wasm'], {
  cwd: root,
  stdio: 'inherit',
});
rmSync(dist, { recursive: true, force: true });
mkdirSync(dist);
copyFileSync(`${root}target/wasm32-unknown-unknown/wasm/probl_wasm.wasm`, `${dist}/probl.wasm`);
copyFileSync(`${web}index.html`, `${dist}/index.html`);
copyFileSync(`${web}src/style.css`, `${dist}/style.css`);
await esbuild.build({
  entryPoints: [`${web}src/app.js`, `${web}src/worker.js`],
  outdir: dist,
  bundle: true,
  minify: true,
  format: 'iife',
  target: 'es2022',
  logLevel: 'warning',
});

if (process.argv.includes('--serve')) {
  const context = await esbuild.context({});
  const { port } = await context.serve({ servedir: dist, port: 8000 });
  console.log(`http://localhost:${port}`);
}
