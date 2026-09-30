// Build the playground into web/dist: the WebAssembly module, the page's
// scripts bundled with Bun, and the guide. `bun run serve` (serve.mjs)
// serves it, and builds it again when its sources change.
//
// Everything is made before web/dist is replaced, so a build that fails
// leaves the last one in place.

import { copyFileSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { basename } from 'node:path';
import { fileURLToPath } from 'node:url';
import { marked } from 'marked';
import { highlight } from './src/probl-lang.js';
import { load } from './src/probl.js';

const web = fileURLToPath(new URL('.', import.meta.url));
const root = fileURLToPath(new URL('..', import.meta.url));
const dist = `${web}dist`;

const cargo = Bun.spawnSync(
  ['cargo', 'build', '-q', '-p', 'probl-wasm', '--target', 'wasm32-unknown-unknown', '--profile', 'wasm'],
  { cwd: root, stdio: ['inherit', 'inherit', 'inherit'] },
);
if (!cargo.success) process.exit(1);
const wasm = readFileSync(`${root}target/wasm32-unknown-unknown/wasm/probl_wasm.wasm`);
const bundle = await Bun.build({
  entrypoints: [`${web}src/app.js`, `${web}src/worker.js`],
  minify: true,
  // Scripts, not modules: the page loads app.js with a <script>, and it
  // starts worker.js as a classic worker.
  format: 'iife',
  target: 'browser',
});

// The guide: the language overview, as HTML. Its complete programs, as the
// module itself checks them, get a button to run them.
const probl = await load(wasm);
const escape = (text) => text.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;');
// Section anchors as GitHub makes them, which the overview's links use.
const slug = (text) =>
  text
    .toLowerCase()
    .replace(/[^\p{L}\p{N}\s_-]/gu, '')
    .trim()
    .replace(/\s+/g, '-');
// The top-level sections, for a list of contents after the opening note.
const sections = [];
marked.use({
  renderer: {
    heading({ tokens, depth, text }) {
      const inner = this.parser.parseInline(tokens);
      if (depth === 2) sections.push(`<li><a href="#${slug(text)}">${inner}</a></li>`);
      return `<h${depth} id="${slug(text)}">${inner}</h${depth}>\n`;
    },
    code({ text, lang }) {
      if (lang !== 'probl') return `<pre><code>${escape(text)}</code></pre>\n`;
      const html = highlight(text, escape, (part, classes) => `<span class="${classes}">${escape(part)}</span>`);
      const pre = `<pre><code>${html.join('')}</code></pre>`;
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
const overview = marked.parse(readFileSync(`${root}docs/language-overview.md`, 'utf8'));
const contents = `<nav class="contents" aria-label="Contents"><p>Contents</p><ul>${sections.join('')}</ul></nav>\n`;
const guide = overview.replace('</blockquote>\n', `</blockquote>\n${contents}`);

rmSync(dist, { recursive: true, force: true });
mkdirSync(dist);
writeFileSync(`${dist}/probl.wasm`, wasm);
copyFileSync(`${web}index.html`, `${dist}/index.html`);
copyFileSync(`${web}src/style.css`, `${dist}/style.css`);
for (const output of bundle.outputs) await Bun.write(`${dist}/${basename(output.path)}`, output);
writeFileSync(`${dist}/guide.html`, guide);
