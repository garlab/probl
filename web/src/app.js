// The Probl playground: an editor, and Probl's engine in WebAssembly,
// running in workers so the page stays responsive (docs/playground-plan.md).
//
// Two workers share the compiled module: the checker checks the program as
// it's edited, and the runner runs it. Stopping a run ends the runner's
// worker, and a new one takes its place.

import { indentWithTab, isolateHistory } from '@codemirror/commands';
import { syntaxHighlighting } from '@codemirror/language';
import { linter, lintGutter, setDiagnostics } from '@codemirror/lint';
import { Compartment } from '@codemirror/state';
import { EditorView, keymap } from '@codemirror/view';
import { basicSetup } from 'codemirror';
import { intelligence, setSymbols } from './intel.js';
import { highlighter, probl } from './probl-lang.js';

/** Seconds a run may take before it's stopped, unless the options say not to. */
const TIME_LIMIT = 30;
const STORED = 'probl-playground-source';
const SPLIT = 'probl-playground-split';

const $ = (id) => document.getElementById(id);
/** The modifier key for shortcuts: ⌘ on Apple's systems, Ctrl elsewhere. */
const modKey = /Mac|iPhone|iPad/.test(navigator.platform) ? '⌘' : 'Ctrl';
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

function remember(value, key = STORED) {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Private windows and blocked storage: nothing to remember.
  }
}

function remembered(key = STORED) {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

// ── The page ─────────────────────────────────────────────────────────────

const theme = EditorView.theme({
  '&': { height: '100%', color: 'var(--text)', backgroundColor: 'var(--editor)' },
  // The cursor CodeMirror draws in place of the browser's, in the text's
  // color: its own is black, which the dark background hides.
  '.cm-cursor, .cm-dropCursor': { borderLeftColor: 'var(--text)', borderLeftWidth: '2px', marginLeft: '-1px' },
  '.cm-gutters': { backgroundColor: 'var(--editor)', color: 'var(--faint)', border: 'none' },
  '.cm-activeLine, .cm-activeLineGutter': { backgroundColor: 'var(--active)' },
  '&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection': {
    backgroundColor: 'var(--selection) !important',
  },
  '.cm-tooltip': { backgroundColor: 'var(--panel)', color: 'var(--text)', border: '1px solid var(--line)' },
});

// Whether the page is dark, which CodeMirror's own styles follow: its
// search panel, search matches and the like.
const darkScheme = window.matchMedia('(prefers-color-scheme: dark)');
const scheme = new Compartment();

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
  const { examples, docs, version } = await checker.ready;
  $('version').textContent = `Probl ${version}`;
  const intel = intelligence(
    docs,
    async (source) => {
      const answer = await check(source);
      return answer.type === 'done' ? answer.result.symbols : null;
    },
    (name) => showReference(name),
  );

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
      const source = view.state.doc.toString();
      const answer = await check(source);
      if (answer.type !== 'done') return [];
      // Names, for the document as it still is. While it doesn't parse, the
      // last ones stay, following the edits.
      const { symbols } = answer.result;
      if (symbols && view.state.doc.toString() === source) view.dispatch({ effects: setSymbols.of(symbols) });
      return answer.result.diagnostics.map((d) => toEditor(d, view.state.doc));
    },
    { delay: 300 },
  );

  let saving = null;
  let edited = false;
  const userEdits = ['input', 'delete', 'move', 'undo', 'redo'];
  const view = new EditorView({
    parent: $('editor'),
    extensions: [
      intel.extensions,
      basicSetup,
      probl,
      probl.data.of({ autocomplete: intel.complete }),
      syntaxHighlighting(highlighter),
      theme,
      scheme.of(EditorView.darkTheme.of(darkScheme.matches)),
      lint,
      lintGutter(),
      keymap.of([
        { key: 'Mod-Enter', run: () => (start(), true) },
        { key: 'Escape', run: () => (running ? (stop('Stopped.'), true) : false) },
        indentWithTab,
      ]),
      EditorView.updateListener.of((update) => {
        if (!update.docChanged) return;
        // Only what the person changes is kept for next time: a program
        // loaded from a link, the menu or the guide isn't theirs.
        edited ||= update.transactions.some((tr) => userEdits.some((event) => tr.isUserEvent(event)));
        clearTimeout(saving);
        saving = setTimeout(() => {
          const source = update.state.doc.toString();
          if (edited) remember(source);
          edited = false;
          followEdits(source);
        }, 500);
      }),
    ],
  });

  darkScheme.addEventListener('change', () => {
    view.dispatch({ effects: scheme.reconfigure(EditorView.darkTheme.of(darkScheme.matches)) });
  });

  // A new program is a step of its own in the history, so that undo takes
  // back that and no more.
  const setSource = (source) => {
    view.dispatch({
      changes: { from: 0, to: view.state.doc.length, insert: source },
      annotations: isolateHistory.of('full'),
    });
  };

  // The program the address links to (`#code=` or `#example=`), until it's
  // edited.
  let linked = null;

  /** Put a program in the editor. If it replaces a program of the person's
   * own, the note for the status line says how to get it back. */
  function load(source) {
    const before = view.state.doc.toString();
    setSource(source);
    const own = before.trim() !== '' && before !== source && before !== linked && !examples.some((e) => e.source === before);
    return own ? `Your program was replaced: ${modKey}+Z brings it back.` : '';
  }

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
    const note = load(example.source);
    linked = example.source;
    history.replaceState(null, '', `#example=${example.name}`);
    clearOutput(note);
    view.focus();
  });

  /** After edits: the menu names the example the program is, if it is one,
   * and a link to another program leaves the address, so that reloading
   * keeps the edits. */
  function followEdits(source) {
    select.value = examples.find((e) => e.source === source)?.name ?? '';
    if (linked !== null && source !== linked) {
      history.replaceState(null, '', location.pathname + location.search);
      linked = null;
    }
  }

  // ── Running ──
  let running = null;
  // Runs that finished, were stopped or crashed: tests wait on it.
  let finished = 0;

  function clearOutput(note = '') {
    $('printed').textContent = '';
    $('printed').hidden = true;
    $('result').textContent = '';
    $('result').className = '';
    $('status').textContent = note;
    $('progress').hidden = true;
    $('empty').hidden = false;
  }

  /** A run's output, as `probl run` prints it. Each line is laid out so that
   * a report too long for the pane wraps at its ` · ` separators, under its
   * values: the text itself doesn't change. */
  function showOutput(text) {
    const lines = text.split('\n');
    $('result').replaceChildren(
      ...lines.map((line, i) => {
        const row = document.createElement('span');
        row.className = 'line';
        // A report's label, and the spaces that line its values up.
        const label = /^\s*\S.*?\s{2,}(?=\S)/.exec(line)?.[0] ?? '';
        row.style.setProperty('--hang', `${label.length}ch`);
        row.append(label);
        line
          .slice(label.length)
          .split(' · ')
          .forEach((part, j) => {
            // A wrapped line starts with its separator.
            const item = document.createElement('span');
            item.className = 'item';
            item.textContent = j ? `· ${part}` : part;
            row.append(...(j ? [' ', item] : [item]));
          });
        if (i < lines.length - 1) row.append('\n');
        return row;
      }),
    );
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

  /** Run the program. `note` is added to the status line when it's done. */
  function start(note = '') {
    if (running) stop(null);
    showPane('output');
    clearOutput();
    $('empty').hidden = true;
    const id = ++lastId;
    const started = performance.now();
    const limited = $('limited').checked;
    running = {
      id,
      started,
      note,
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
      const { note } = running;
      finish();
      show(data.result, elapsed, note);
    }
  }

  function show(result, elapsed, note) {
    const time = elapsed < 1 ? `${Math.round(elapsed * 1000)} ms` : `${elapsed.toFixed(1)} s`;
    const status = (text) => {
      $('status').textContent = note ? `${text} ${note}` : text;
    };
    if (result.output !== undefined) {
      showOutput(result.output);
      const s = result.stats;
      const parts = [`Done in ${time}`];
      if (s.runs === undefined) parts.push(`${format.format(s.world_steps)} world-steps`);
      if (s.solved_loops) parts.push(`${s.solved_loops} ${s.solved_loops === 1 ? 'loop' : 'loops'} solved`);
      if (s.solved_calls) parts.push(`${s.solved_calls} recursive ${s.solved_calls === 1 ? 'call' : 'calls'} solved`);
      status(`${parts.join(' · ')}.`);
      return;
    }
    const error = result.error;
    $('result').className = 'error';
    $('result').textContent = error?.rendered ?? error?.message ?? 'Something went wrong.';
    const limit = error?.kind === 'limit' ? ' The playground’s limits are lower than the command line’s.' : '';
    status(`Failed after ${time}.${limit}`);
    if (error?.from !== undefined) {
      const doc = view.state.doc;
      const diagnostics = result.diagnostics.filter((d) => d.severity === 'warning').map((d) => toEditor(d, doc));
      view.dispatch(setDiagnostics(view.state, [...diagnostics, toEditor(error, doc)]));
    }
  }

  $('run').addEventListener('click', () => start());
  $('stop').addEventListener('click', () => stop('Stopped.'));
  $('mode').addEventListener('change', () => {
    $('sampling').hidden = $('mode').value !== 'sample';
  });

  // ── Tabs: the output, the guide and the reference ──
  const tabs = [...document.querySelectorAll('[role="tab"]')];
  function showPane(name) {
    for (const tab of tabs) {
      const selected = tab.dataset.pane === name;
      tab.setAttribute('aria-selected', String(selected));
      tab.tabIndex = selected ? 0 : -1;
      $(tab.dataset.pane).hidden = !selected;
    }
    if (name === 'guide') loadGuide();
    if (name === 'reference') $('search').focus();
  }
  for (const tab of tabs) tab.addEventListener('click', () => showPane(tab.dataset.pane));
  // The arrow keys, Home and End move between the tabs.
  document.querySelector('.tabs').addEventListener('keydown', (event) => {
    const at = tabs.indexOf(document.activeElement);
    const next = { ArrowRight: at + 1, ArrowLeft: at - 1, Home: 0, End: tabs.length - 1 }[event.key];
    if (at < 0 || next === undefined) return;
    event.preventDefault();
    const tab = tabs[(next + tabs.length) % tabs.length];
    showPane(tab.dataset.pane);
    tab.focus();
  });

  // ── The divider between the editor and the side panel ──
  const divider = $('divider');
  const main = document.querySelector('main');
  /** Give the editor `share` of the width, from 20% to 80%. */
  function setShare(share) {
    const clamped = Math.round(Math.min(80, Math.max(20, share)));
    main.style.setProperty('--share', `${clamped}%`);
    divider.setAttribute('aria-valuenow', String(clamped));
    return clamped;
  }
  let share = setShare(Number(remembered(SPLIT)) || 52);
  divider.addEventListener('pointerdown', (event) => {
    event.preventDefault();
    divider.classList.add('dragging');
    const move = (e) => {
      const { left, width } = main.getBoundingClientRect();
      share = setShare(((e.clientX - left) / width) * 100);
    };
    const up = () => {
      divider.classList.remove('dragging');
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
      remember(String(share), SPLIT);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
  });
  divider.addEventListener('keydown', (event) => {
    const step = { ArrowLeft: -5, ArrowRight: 5 }[event.key];
    if (step === undefined) return;
    event.preventDefault();
    share = setShare(share + step);
    remember(String(share), SPLIT);
  });
  divider.addEventListener('dblclick', () => {
    share = setShare(52);
    remember(String(share), SPLIT);
  });

  // The reference, from the module's documentation.
  const inline = (text) => {
    const p = document.createElement('p');
    for (const [i, part] of text.split('`').entries()) {
      if (i % 2) {
        const code = document.createElement('code');
        code.textContent = part;
        p.append(code);
      } else {
        p.append(part);
      }
    }
    return p;
  };
  const entry = (item) => {
    const div = document.createElement('div');
    div.className = 'entry';
    div.dataset.names = item.name;
    div.dataset.search = `${item.name} ${item.signature} ${item.summary}`.toLowerCase();
    const head = document.createElement('code');
    head.className = 'signature';
    head.textContent = item.signature;
    div.append(head, inline(item.summary));
    return div;
  };
  // Keywords written together, like `if` and `else`, share an entry.
  const keywords = [];
  for (const k of docs.keywords) {
    const same = keywords.find((e) => e.signature === k.signature);
    if (same) same.name += ` ${k.name}`;
    else keywords.push({ ...k });
  }
  const groups = [['Keywords', keywords], ['Reading data', [docs.read]]];
  for (const category of ['Distributions', 'Questions about distributions', 'Collections', 'Math', 'Text', 'Dates']) {
    groups.push([category, docs.builtins.filter((b) => b.category === category)]);
  }
  for (const [title, items] of groups) {
    const section = document.createElement('section');
    const h = document.createElement('h3');
    h.textContent = title;
    section.append(h, ...items.map(entry));
    $('entries').append(section);
  }
  $('search').addEventListener('input', () => {
    const query = $('search').value.trim().toLowerCase();
    for (const section of $('entries').children) {
      let any = false;
      for (const e of section.querySelectorAll('.entry')) {
        e.hidden = query !== '' && !e.dataset.search.includes(query);
        e.classList.remove('current');
        any ||= !e.hidden;
      }
      section.hidden = !any;
    }
  });

  /** Show a built-in's or keyword's entry, from the editor. */
  function showReference(name) {
    showPane('reference');
    $('search').value = name;
    $('search').dispatchEvent(new Event('input'));
    const entry = [...$('entries').querySelectorAll('.entry')].find((e) => e.dataset.names.split(' ').includes(name));
    entry?.classList.add('current');
    entry?.scrollIntoView({ block: 'nearest' });
  }

  // The guide: the language overview, loaded when first shown. Its
  // complete programs can be run in the editor.
  let guide = null;
  function loadGuide() {
    guide ??= fetch('guide.html')
      .then((r) => r.text())
      .then((html) => {
        $('guide').innerHTML = html;
      })
      .catch(() => {
        $('guide').textContent = 'The guide couldn’t be loaded.';
      });
  }
  $('guide').addEventListener('click', (event) => {
    const button = event.target.closest('button.try');
    if (button) {
      const note = load(button.previousElementSibling.textContent);
      linked = null;
      history.replaceState(null, '', location.pathname + location.search);
      start(note);
      return;
    }
    // Links within the guide scroll it, without touching the address.
    const anchor = event.target.closest('a[href^="#"]');
    if (anchor && !anchor.getAttribute('href').startsWith('#example=')) {
      event.preventDefault();
      document.getElementById(decodeURIComponent(anchor.getAttribute('href').slice(1)))?.scrollIntoView();
    }
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

  // ── The program in the link: a shared one, or an example ──
  async function fromLink() {
    const hash = new URLSearchParams(location.hash.slice(1));
    if (hash.has('code')) {
      try {
        return await decode(hash.get('code'));
      } catch {
        $('status').textContent = 'The link’s program couldn’t be read.';
        return null;
      }
    }
    const named = examples.find((e) => e.name === hash.get('example'));
    if (!named) return null;
    select.value = named.name;
    return named.source;
  }
  window.addEventListener('hashchange', async () => {
    const source = await fromLink();
    if (source !== null) {
      const note = load(source);
      linked = source;
      clearOutput(note);
      showPane('output');
    }
  });

  // The first program: from the link, then as left, then the tour.
  linked = await fromLink();
  const first = linked ?? remembered() ?? examples[0].source;
  setSource(first);
  select.value = examples.find((e) => e.source === first)?.name ?? '';
  view.focus();

  // The hint shown until a run: its shortcut, and its links to the tabs.
  for (const key of document.querySelectorAll('kbd.mod')) key.textContent = modKey;
  for (const link of document.querySelectorAll('[data-show]')) {
    link.addEventListener('click', () => showPane(link.dataset.show));
  }

  // For tests and for the curious.
  window.playground = {
    view,
    examples,
    getSource: () => view.state.doc.toString(),
    setSource,
    run: () => start(),
    stop: () => stop('Stopped.'),
    isRunning: () => running !== null,
    finished: () => finished,
  };
  document.body.dataset.ready = 'true';
}

main().catch((error) => {
  $('status').textContent = `The playground couldn’t start: ${error.message}`;
});
