// Build the playground into web/dist: the WebAssembly module, the page's
// scripts bundled with esbuild, and the guide. `--serve` also serves it on
// port 8000.

import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import * as esbuild from 'esbuild';
import { marked } from 'marked';
import { load } from './src/probl.js';

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

// The guide: the language overview, as HTML. Its complete programs, as the
// module itself checks them, get a button to run them.
const probl = await load(readFileSync(`${dist}/probl.wasm`));
const escape = (text) => text.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;');
// Section anchors as GitHub makes them, which the overview's links use.
const slug = (text) =>
  text
    .toLowerCase()
    .replace(/[^\p{L}\p{N}\s_-]/gu, '')
    .trim()
    .replace(/\s+/g, '-');
marked.use({
  renderer: {
    heading({ tokens, depth, text }) {
      return `<h${depth} id="${slug(text)}">${this.parser.parseInline(tokens)}</h${depth}>\n`;
    },
    code({ text, lang }) {
      const pre = `<pre><code>${escape(text)}</code></pre>`;
      if (lang !== 'probl') return `${pre}\n`;
      const runs = !probl.check(text).diagnostics.some((d) => d.severity === 'error');
      return `<figure>${pre}${runs ? '<button class="try">Run in the editor</button>' : ''}</figure>\n`;
    },
    link({ href, tokens }) {
      const inner = this.parser.parseInline(tokens);
      if (href.startsWith('#')) return `<a href="${href}">${inner}</a>`;
      const example = /examples\/(\w+)\.probl$/.exec(href);
      if (example) return `<a href="#example=${example[1]}">${inner}</a>`;
      // The other documents aren't part of the playground.
      return inner;
    },
  },
});
const overview = readFileSync(`${root}docs/language-overview.md`, 'utf8');
writeFileSync(`${dist}/guide.html`, marked.parse(overview));

if (process.argv.includes('--serve')) {
  const context = await esbuild.context({});
  const { port } = await context.serve({ servedir: dist, port: 8000 });
  console.log(`http://localhost:${port}`);
}
