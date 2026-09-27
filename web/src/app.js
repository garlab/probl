// The Probl playground: an editor, and Probl's engine in WebAssembly,
// running in workers so the page stays responsive (docs/playground-plan.md).
//
// Two workers share the compiled module: the checker checks the program as
// it's edited, and the runner runs it. Stopping a run ends the runner's
// worker, and a new one takes its place.

import { indentWithTab } from '@codemirror/commands';
import { HighlightStyle, syntaxHighlighting } from '@codemirror/language';
import { linter, lintGutter, setDiagnostics } from '@codemirror/lint';
import { EditorView, keymap } from '@codemirror/view';
import { tags } from '@lezer/highlight';
import { basicSetup } from 'codemirror';
import { probl } from './probl-lang.js';

/** Seconds a run may take before it's stopped, unless the options say not to. */
const TIME_LIMIT = 30;
const STORED = 'probl-playground-source';

const $ = (id) => document.getElementById(id);
const format = new Intl.NumberFormat('en');

// ── Workers ──────────────────────────────────────────────────────────────

/** A worker running the module, which can be replaced. */
class Engine {
  constructor(module, onMessage) {
    this.module = module;
    this.onMessage = onMessage;
    this.start();
  }

  start() {
    this.worker = new Worker('worker.js');
    this.ready = new Promise((resolve) => {
      this.worker.onmessage = ({ data }) => (data.type === 'ready' ? resolve(data) : this.onMessage(data));
    });
    this.worker.onerror = (event) => this.onMessage({ type: 'crash', message: event.message });
    this.worker.postMessage({ type: 'init', module: this.module });
  }

  restart() {
    this.worker.terminate();
    this.start();
  }

  send(message) {
    this.worker.postMessage(message);
  }
}

// ── Sharing ──────────────────────────────────────────────────────────────

async function squeeze(text, stream) {
  const piped = new Blob([text]).stream().pipeThrough(stream);
  return new Uint8Array(await new Response(piped).arrayBuffer());
}

async function encode(source) {
  const bytes = await squeeze(new TextEncoder().encode(source), new CompressionStream('deflate-raw'));
  let binary = '';
  for (const b of bytes) binary += String.fromCharCode(b);
  return btoa(binary).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '');
}

async function decode(code) {
  const binary = atob(code.replaceAll('-', '+').replaceAll('_', '/'));
  const bytes = Uint8Array.from(binary, (c) => c.charCodeAt(0));
  return new TextDecoder().decode(await squeeze(bytes, new DecompressionStream('deflate-raw')));
}

function remember(source) {
  try {
    localStorage.setItem(STORED, source);
  } catch {
    // Private windows and blocked storage: nothing to remember.
  }
}

function remembered() {
  try {
    return localStorage.getItem(STORED);
  } catch {
    return null;
  }
}

// ── The page ─────────────────────────────────────────────────────────────

const highlight = HighlightStyle.define([
  { tag: tags.keyword, color: 'var(--keyword)' },
  { tag: [tags.number, tags.atom, tags.bool], color: 'var(--number)' },
  { tag: tags.string, color: 'var(--string)' },
  { tag: tags.comment, color: 'var(--comment)', fontStyle: 'italic' },
  { tag: tags.typeName, color: 'var(--type)' },
  { tag: [tags.function(tags.variableName)], color: 'var(--function)' },
  { tag: tags.meta, color: 'var(--meta)' },
  { tag: tags.operator, color: 'var(--operator)' },
]);

const theme = EditorView.theme({
  '&': { height: '100%', color: 'var(--text)', backgroundColor: 'var(--editor)' },
  '.cm-content': { caretColor: 'var(--text)' },
  '.cm-gutters': { backgroundColor: 'var(--editor)', color: 'var(--faint)', border: 'none' },
  '.cm-activeLine, .cm-activeLineGutter': { backgroundColor: 'var(--active)' },
  '&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection': {
    backgroundColor: 'var(--selection) !important',
  },
  '.cm-tooltip': { backgroundColor: 'var(--panel)', color: 'var(--text)', border: '1px solid var(--line)' },
});

async function main() {
  const module = await WebAssembly.compileStreaming(fetch('probl.wasm'));
  let lastId = 0;
  const waiting = new Map();
  const checker = new Engine(module, (data) => {
    waiting.get(data.id)?.(data);
    waiting.delete(data.id);
    if (data.type === 'crash') checker.restart();
  });
  const runner = new Engine(module, onRun);
  const { examples, version } = await checker.ready;
  $('version').textContent = `Probl ${version}`;

  // The examples' data, which any program may read.
  const files = Object.assign({}, ...examples.map((e) => e.files));

  /** The program's diagnostics, from the checker. */
  function check(source) {
    const id = ++lastId;
    return new Promise((resolve) => {
      waiting.set(id, resolve);
      checker.send({ type: 'check', id, source });
    });
  }

  const toEditor = (d, doc) => {
    const from = Math.min(d.from ?? 0, doc.length);
    const to = Math.min(Math.max(d.to ?? from, from), doc.length);
    const extra = [...(d.notes ?? []), d.help ? `help: ${d.help}` : null].filter(Boolean);
    return {
      from,
      to,
      severity: d.severity === 'warning' ? 'warning' : 'error',
      message: [d.message, ...extra].join('\n'),
    };
  };

  const lint = linter(
    async (view) => {
      const answer = await check(view.state.doc.toString());
      if (answer.type !== 'done') return [];
      return answer.result.diagnostics.map((d) => toEditor(d, view.state.doc));
    },
    { delay: 300 },
  );

  let saving = null;
  const view = new EditorView({
    parent: $('editor'),
    extensions: [
      basicSetup,
      probl,
      syntaxHighlighting(highlight),
      theme,
      lint,
      lintGutter(),
      keymap.of([
        { key: 'Mod-Enter', run: () => (start(), true) },
        { key: 'Escape', run: () => (running ? (stop('Stopped.'), true) : false) },
        indentWithTab,
      ]),
      EditorView.updateListener.of((update) => {
        if (!update.docChanged) return;
        clearTimeout(saving);
        saving = setTimeout(() => remember(update.state.doc.toString()), 500);
      }),
    ],
  });

  const setSource = (source) => {
    view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: source } });
  };

  // ── Examples ──
  const select = $('examples');
  for (const example of examples) {
    const option = document.createElement('option');
    option.value = example.name;
    option.textContent = `${example.name.slice(0, 2)} · ${example.title}`;
    select.append(option);
  }
  select.addEventListener('change', () => {
    const example = examples.find((e) => e.name === select.value);
    if (!example) return;
    setSource(example.source);
    history.replaceState(null, '', `#example=${example.name}`);
    clearOutput();
    view.focus();
  });

  // ── Running ──
  let running = null;
  // Runs that finished, were stopped or crashed: tests wait on it.
  let finished = 0;

  function clearOutput() {
    $('printed').textContent = '';
    $('printed').hidden = true;
    $('result').textContent = '';
    $('result').className = '';
    $('status').textContent = '';
    $('progress').hidden = true;
  }

  function request() {
    const r = { source: view.state.doc.toString(), files };
    const mode = $('mode').value;
    if (mode !== 'program') r.mode = mode;
    if (mode === 'sample') {
      const runs = parseInt($('runs').value, 10);
      const seed = parseInt($('seed').value, 10);
      if (runs > 0) r.runs = runs;
      if (seed >= 0) r.seed = seed;
    }
    r.conjugate = $('conjugate').checked;
    return r;
  }

  function start() {
    if (running) stop(null);
    clearOutput();
    const id = ++lastId;
    const started = performance.now();
    const limited = $('limited').checked;
    running = {
      id,
      started,
      lines: [],
      ticker: setInterval(() => {
        const seconds = (performance.now() - started) / 1000;
        if (limited && seconds > TIME_LIMIT) {
          stop(`Stopped after ${TIME_LIMIT} seconds: the playground stops long runs, unless you turn that off in the options.`);
          return;
        }
        if (!running.progress) $('status').textContent = `Running… ${seconds.toFixed(1)} s`;
      }, 100),
    };
    $('run').disabled = true;
    $('stop').disabled = false;
    $('status').textContent = 'Running…';
    runner.send({ type: 'run', id, request: request() });
  }

  function finish() {
    clearInterval(running.ticker);
    running = null;
    finished += 1;
    $('run').disabled = false;
    $('stop').disabled = true;
    $('progress').hidden = true;
  }

  /** Stop the run by replacing the runner's worker. */
  function stop(message) {
    if (!running) return;
    finish();
    runner.restart();
    if (message) $('status').textContent = message;
  }

  function onRun(data) {
    if (!running || data.id !== running.id) {
      if (data.type === 'crash') runner.restart();
      return;
    }
    const elapsed = (performance.now() - running.started) / 1000;
    if (data.type === 'print') {
      running.lines.push(data.line);
      $('printed').hidden = false;
      $('printed').textContent = running.lines.join('\n');
    } else if (data.type === 'progress') {
      running.progress = true;
      $('progress').hidden = false;
      $('progress').max = data.total;
      $('progress').value = data.done;
      $('status').textContent = `Running… ${format.format(data.done)} of ${format.format(data.total)} runs`;
    } else if (data.type === 'crash') {
      finish();
      runner.restart();
      $('result').className = 'error';
      $('result').textContent = `The engine crashed: ${data.message}\n\nThis may be the playground's limits (memory, or calls nested too deep), or a bug in Probl.`;
      $('status').textContent = `Crashed after ${elapsed.toFixed(1)} s.`;
    } else if (data.type === 'done') {
      finish();
      show(data.result, elapsed);
    }
  }

  function show(result, elapsed) {
    const time = elapsed < 1 ? `${Math.round(elapsed * 1000)} ms` : `${elapsed.toFixed(1)} s`;
    if (result.output !== undefined) {
      $('result').textContent = result.output;
      const s = result.stats;
      const parts = [`Done in ${time}`];
      if (s.runs === undefined) parts.push(`${format.format(s.world_steps)} world-steps`);
      if (s.solved_loops) parts.push(`${s.solved_loops} ${s.solved_loops === 1 ? 'loop' : 'loops'} solved`);
      if (s.solved_calls) parts.push(`${s.solved_calls} recursive ${s.solved_calls === 1 ? 'call' : 'calls'} solved`);
      $('status').textContent = `${parts.join(' · ')}.`;
      return;
    }
    const error = result.error;
    $('result').className = 'error';
    $('result').textContent = error?.rendered ?? error?.message ?? 'Something went wrong.';
    const limit = error?.kind === 'limit' ? ' The playground’s limits are lower than the command line’s.' : '';
    $('status').textContent = `Failed after ${time}.${limit}`;
    if (error?.from !== undefined) {
      const doc = view.state.doc;
      const diagnostics = result.diagnostics.filter((d) => d.severity === 'warning').map((d) => toEditor(d, doc));
      view.dispatch(setDiagnostics(view.state, [...diagnostics, toEditor(error, doc)]));
    }
  }

  $('run').addEventListener('click', start);
  $('stop').addEventListener('click', () => stop('Stopped.'));
  $('mode').addEventListener('change', () => {
    $('sampling').hidden = $('mode').value !== 'sample';
  });

  // ── Sharing ──
  $('share').addEventListener('click', async () => {
    const code = await encode(view.state.doc.toString());
    history.replaceState(null, '', `#code=${code}`);
    try {
      await navigator.clipboard.writeText(location.href);
      $('status').textContent = 'Link copied. It holds the program itself: nothing is stored anywhere.';
    } catch {
      $('status').textContent = 'The link is in the address bar. It holds the program itself: nothing is stored anywhere.';
    }
  });

  // ── The first program: from the link, then as left, then the tour ──
  const hash = new URLSearchParams(location.hash.slice(1));
  let first = null;
  if (hash.has('code')) {
    try {
      first = await decode(hash.get('code'));
    } catch {
      $('status').textContent = 'The link’s program couldn’t be read.';
    }
  }
  const named = examples.find((e) => e.name === hash.get('example'));
  if (first === null && named) {
    first = named.source;
    select.value = named.name;
  }
  first ??= remembered() ?? examples[0].source;
  setSource(first);
  view.focus();

  // For tests and for the curious.
  window.playground = {
    examples,
    getSource: () => view.state.doc.toString(),
    setSource,
    run: start,
    stop: () => stop('Stopped.'),
    isRunning: () => running !== null,
    finished: () => finished,
  };
  document.body.dataset.ready = 'true';
}

main().catch((error) => {
  $('status').textContent = `The playground couldn’t start: ${error.message}`;
});
