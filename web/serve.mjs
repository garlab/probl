// Serve the playground from web/dist on port 8000, build it again when its
// sources change, and reload the page when the new build is ready. The page
// keeps the program being edited across a reload.
//
//   bun run serve                # then open http://localhost:8000
//
// It watches the page's sources, the language overview (which makes the
// guide), and the crates and examples that make the WebAssembly module. A
// build that fails is reported here, and the last one stays served. A build
// that makes the same files as the last one, after a change to a test, say,
// doesn't reload the page.

import { readdirSync, readFileSync, watch } from 'node:fs';
import { join, normalize } from 'node:path';
import { fileURLToPath } from 'node:url';

const web = fileURLToPath(new URL('.', import.meta.url));
const root = fileURLToPath(new URL('..', import.meta.url));

/** Serve the files in `dir`, and its index.html for `/`, never cached.
 * `page` can change index.html as it's served, and `routes` adds others. */
export function serveFiles(dir, { port = 0, page = (html) => html, routes = {} } = {}) {
  const base = normalize(`${dir}/`);
  const headers = { 'Cache-Control': 'no-store' };
  return Bun.serve({
    port,
    routes,
    async fetch(request) {
      const notFound = () => new Response('Not found', { status: 404, headers });
      const { pathname } = new URL(request.url);
      let name;
      try {
        name = decodeURIComponent(pathname);
      } catch {
        return notFound();
      }
      const path = normalize(join(base, name, pathname.endsWith('/') ? 'index.html' : ''));
      const file = Bun.file(path);
      if (!path.startsWith(base) || !(await file.exists())) return notFound();
      if (path === `${base}index.html`) {
        return new Response(page(await file.text()), { headers: { ...headers, 'Content-Type': file.type } });
      }
      return new Response(file, { headers });
    },
  });
}

if (import.meta.main) {
  const dist = `${web}dist`;

  // ── Telling the page to reload ──
  // Each build that changes something gets a new name, which the page is
  // sent when it connects and after each such build. It reloads when the name
  // changes: after a build, or when this server is started again.
  const session = Date.now().toString(36);
  let builds = 0;
  const current = () => `${session}.${builds}`;
  const listeners = new Set();
  const encoder = new TextEncoder();
  const send = (listener) => listener.enqueue(encoder.encode(`data: ${current()}\n\n`));
  const reloader = `<script>
  // Reload when the playground is built again (bun run serve).
  {
    let built;
    new EventSource('/__reload').onmessage = ({ data }) => {
      if (built !== undefined && data !== built) location.reload();
      built = data;
    };
  }
</script>
`;

  // What a build made, to tell whether a new one changed anything.
  const digest = () => {
    const hash = new Bun.CryptoHasher('sha256');
    for (const name of readdirSync(dist).sort()) {
      hash.update(name);
      hash.update(readFileSync(join(dist, name)));
    }
    return hash.digest('hex');
  };

  // ── Building, one at a time ──
  // Changes made during a build wait for it, then build together.
  let building = false;
  let waiting = [];
  let made = null;
  async function build(why) {
    waiting.push(why);
    if (building) return;
    building = true;
    while (waiting.length) {
      const reasons = waiting.join(', ');
      waiting = [];
      console.log(`Building (${reasons})…`);
      const start = performance.now();
      const child = Bun.spawn([process.execPath, 'build.mjs'], { cwd: web, stdio: ['ignore', 'inherit', 'inherit'] });
      const ok = (await child.exited) === 0;
      const took = `${((performance.now() - start) / 1000).toFixed(1)} s`;
      if (!ok) {
        console.log(`The build failed after ${took}: the last one is still served.`);
        continue;
      }
      const now = digest();
      if (now === made) {
        console.log(`Built in ${took}: nothing changed.`);
        continue;
      }
      made = now;
      builds += 1;
      for (const listener of listeners) {
        try {
          send(listener);
        } catch {
          listeners.delete(listener);
        }
      }
      console.log(`Built in ${took}${listeners.size ? ': reloading the page.' : '.'}`);
    }
    building = false;
  }

  // ── Watching the sources ──
  // Directories, not files: an editor that saves by renaming a new file over
  // the old one would leave a watcher on the old file behind.
  const watched = [
    [web, false, (name) => name === 'index.html'],
    [`${web}src`, true],
    [`${root}docs`, false, (name) => name === 'language-overview.md'],
    [root, false, (name) => name === 'Cargo.toml' || name === 'Cargo.lock'],
    [`${root}crates`, true],
    [`${root}examples`, true],
  ];
  // Editors' temporary and backup files, and the like.
  const ignored = (name) => name.split('/').some((part) => part.startsWith('.')) || name.endsWith('~');
  let timer = null;
  const changed = new Set();
  for (const [dir, recursive, wanted = () => true] of watched) {
    watch(dir, { recursive }, (_, name) => {
      if (!name || ignored(name) || !wanted(name)) return;
      changed.add(join(dir, name).slice(root.length));
      // Editors change a file more than once as they save it.
      clearTimeout(timer);
      timer = setTimeout(() => {
        const why = [...changed].join(', ');
        changed.clear();
        build(why);
      }, 100);
    });
  }

  await build('starting');
  const server = serveFiles(dist, {
    port: 8000,
    page: (html) => html.replace('</head>', `${reloader}</head>`),
    routes: {
      '/__reload': (request, server) => {
        // Kept open for as long as the page is.
        server.timeout(request, 0);
        let listener;
        const stream = new ReadableStream({
          start(controller) {
            listener = controller;
            listeners.add(listener);
            send(listener);
          },
          cancel() {
            listeners.delete(listener);
          },
        });
        return new Response(stream, { headers: { 'Content-Type': 'text/event-stream', 'Cache-Control': 'no-store' } });
      },
    },
  });
  console.log(`http://localhost:${server.port}`);
}
