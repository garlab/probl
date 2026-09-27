// Runs Probl's WebAssembly module off the page's thread. The page sends the
// compiled module, then requests; a run can be stopped only by ending the
// worker, which the page then replaces.
//
//   → {type: 'init', module}              ← {type: 'ready', examples, version}
//   → {type: 'check', id, source}         ← {type: 'done', id, result}
//   → {type: 'run', id, request}          ← {type: 'print', id, line}, {type: 'progress', id, done, total},
//                                           then {type: 'done', id, result, memory}
//   ← {type: 'crash', id, message}: the module stopped, and this worker is done.

import { Crash, load } from './probl.js';

let loaded = null;
let current = null;

self.onmessage = async ({ data }) => {
  if (data.type === 'init') {
    loaded = load(data.module, {
      onPrint: (line) => self.postMessage({ type: 'print', id: current, line }),
      onProgress: (done, total) => self.postMessage({ type: 'progress', id: current, done, total }),
    });
    const probl = await loaded;
    self.postMessage({ type: 'ready', examples: probl.examples(), version: probl.version() });
    return;
  }
  // Requests sent before the module was ready wait for it, in order.
  const probl = await loaded;
  current = data.id;
  try {
    const result = data.type === 'check' ? probl.check(data.source) : probl.run(data.request);
    self.postMessage({ type: 'done', id: data.id, result, memory: probl.memoryBytes() });
  } catch (error) {
    const message = error instanceof Crash ? error.message : String(error);
    self.postMessage({ type: 'crash', id: data.id, message });
  }
};
