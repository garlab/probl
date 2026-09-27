// What the editor knows about the program: its names, from the compiler
// (`check` in crates/probl-wasm), and the built-ins' and keywords'
// documentation. It gives completion, descriptions on hover, going to a
// definition with Cmd-click, Ctrl-click or F12, and highlights of the
// name under the cursor.

import { closeCompletion, completionStatus, selectedCompletion } from '@codemirror/autocomplete';
import { syntaxTree } from '@codemirror/language';
import { Prec, StateEffect, StateField } from '@codemirror/state';
import { Decoration, EditorView, ViewPlugin, hoverTooltip, keymap, showTooltip } from '@codemirror/view';

/** New symbols from the compiler, for the document as it is now. */
export const setSymbols = StateEffect.define();

/** The program's names: `{definitions, references, functions}`, with
 * positions mapped through the edits made since they were computed, so
 * they stay useful while the program doesn't parse. */
export const symbolsField = StateField.define({
  create: () => null,
  update(symbols, tr) {
    for (const effect of tr.effects) {
      if (effect.is(setSymbols)) return effect.value;
    }
    return symbols && tr.docChanged ? mapSymbols(symbols, tr.changes) : symbols;
  },
});

function mapSymbols(symbols, changes) {
  const at = (pos, assoc = -1) => changes.mapPos(pos, assoc);
  // A name whose own text was edited, or replaced with the rest of the
  // program, is unknown until the next check. Scopes and functions, which
  // contain the edits, follow them.
  const edited = (from, to) => changes.touchesRange(from, to) !== false;
  const definitions = symbols.definitions.map((d) => {
    if (d.gone) return d;
    return edited(d.from, d.to)
      ? { ...d, from: -1, to: -1, scope: [-1, -1], gone: true }
      : { ...d, from: at(d.from), to: at(d.to, 1), scope: [at(d.scope[0]), at(d.scope[1], 1)] };
  });
  return {
    definitions,
    references: symbols.references
      .filter(([from, to, def]) => !edited(from, to) && !definitions[def].gone)
      .map(([from, to, def]) => [at(from), at(to, 1), def]),
    functions: symbols.functions.map(([from, to]) => [at(from), at(to, 1)]),
  };
}

/** The definition of the name at `pos`, used or declared there. */
function definitionAt(state, pos) {
  const symbols = state.field(symbolsField, false);
  if (!symbols) return null;
  const reference = symbols.references.find(([from, to]) => from <= pos && pos <= to);
  if (reference) return { ...symbols.definitions[reference[2]], index: reference[2] };
  const index = symbols.definitions.findIndex((d) => d.from <= pos && pos <= d.to);
  return index < 0 ? null : { ...symbols.definitions[index], index };
}

/** Whether `pos` is in a comment or a string, where words aren't names:
 * `side` says whether it's the text before `pos` (-1) or after it (1). */
function inText(state, pos, side) {
  const { name } = syntaxTree(state).resolveInner(pos, side);
  return name === 'comment' || name === 'string';
}

/** The call whose arguments `pos` is among: `{name, from, paren, index,
 * method}`, with where its function's name and its `(` are, which argument
 * (from 0), and whether it's called as a method, `xs.f(y)`. Brackets and
 * commas in strings and comments don't count. */
function callAt(state, pos) {
  const start = Math.max(0, pos - 2000);
  const text = state.sliceDoc(start, pos);
  const skipped = [];
  syntaxTree(state).iterate({
    from: start,
    to: pos,
    enter: (node) => {
      if (node.name === 'comment' || node.name === 'string') skipped.push([node.from, node.to]);
    },
  });
  let depth = 0;
  let index = 0;
  for (let i = pos - 1, s = skipped.length - 1; i >= start; i -= 1) {
    while (s >= 0 && skipped[s][0] > i) s -= 1;
    if (s >= 0 && i < skipped[s][1]) {
      i = skipped[s][0];
      continue;
    }
    const c = text[i - start];
    if (c === ')' || c === ']' || c === '}') depth += 1;
    else if (c === ',' && depth === 0) index += 1;
    else if (c === '(' || c === '[' || c === '{') {
      if (depth > 0) {
        depth -= 1;
        continue;
      }
      const name = c === '(' && /([A-Za-z_]\w*)\s*$/.exec(text.slice(0, i - start));
      if (!name) return null;
      const from = i - name[0].length;
      return { name: name[1], from, paren: i, index, method: state.sliceDoc(from - 1, from) === '.' };
    }
  }
  return null;
}

/** Where a signature's parameters are, for its call of `name`:
 * `binomial(n: int, p: prob) -> dist[int]` has `n: int` and `p: prob`.
 * `method` says whether it's written as one, like `xs.push(x)`. */
function parametersOf(signature, name) {
  const at = signature.search(new RegExp(`\\b${name}\\(`));
  if (at < 0) return null;
  const params = [];
  const add = (from, to) => {
    while (signature[from] === ' ') from += 1;
    if (to > from) params.push([from, to]);
  };
  let from = at + name.length + 1;
  let depth = 0;
  for (let i = from; i < signature.length; i += 1) {
    const c = signature[i];
    if (c === '(' || c === '[' || c === '{') depth += 1;
    else if ((c === ')' || c === ']' || c === '}') && depth > 0) depth -= 1;
    else if (c === ',' && depth === 0) {
      add(from, i);
      from = i + 1;
    } else if (c === ')') {
      add(from, i);
      return { params, method: signature[at - 1] === '.' };
    }
  }
  return null;
}

/** Whether the person just typed one of `chars`. */
function typedOneOf(tr, chars) {
  let found = false;
  if (tr.isUserEvent('input.type')) {
    tr.changes.iterChanges((fromA, toA, fromB, toB, inserted) => {
      found ||= [...chars].some((c) => inserted.toString().includes(c));
    });
  }
  return found;
}

/** The identifier around `pos`. */
function wordAt(state, pos) {
  const line = state.doc.lineAt(pos);
  const text = line.text;
  let start = pos - line.from;
  let end = start;
  while (start > 0 && /\w/.test(text[start - 1])) start -= 1;
  while (end < text.length && /\w/.test(text[end])) end += 1;
  if (start === end || !/^[A-Za-z_]/.test(text[start])) return null;
  return { from: line.from + start, to: line.from + end, text: text.slice(start, end) };
}

const kinds = {
  variable: 'variable',
  parameter: 'variable',
  function: 'function',
  record: 'type',
  enum: 'enum',
  variant: 'constant',
  field: 'property',
};

/** How a definition is described: its declaration, what it is, and its
 * comments. */
function describeDefinition(d) {
  const what = {
    variable: d.mutable ? 'variable (var)' : 'variable',
    parameter: 'parameter',
    function: 'function',
    record: 'record type',
    enum: 'enum',
    variant: `variant of ${d.owner}`,
    field: `field of ${d.owner}`,
  }[d.kind];
  return { code: d.detail, what: `${what}, line ${d.line}`, doc: d.doc };
}

/** What a definition's declaration says after its name, shown beside it in
 * the completion list: `(days: int) -> int` for a function, `~ beta(2, 40)`
 * for a variable; or a field's type, or a variant's enum. */
function declaredAs(d) {
  const declared = new RegExp(`^(?:fn|let|var) ${d.name}\\b`).exec(d.detail);
  if (declared && (d.kind === 'function' || d.kind === 'variable')) return d.detail.slice(declared[0].length).trim();
  if (d.kind === 'field') return d.detail.slice(d.detail.indexOf(':') + 1).trim();
  if (d.kind === 'variant') return d.owner;
  return undefined;
}

/** A tooltip's or completion's content: code, then text. Backquoted words
 * in the text become code. */
function card({ code, what, doc }) {
  const dom = document.createElement('div');
  dom.className = 'cm-probl-card';
  if (code) {
    const pre = document.createElement('code');
    pre.className = 'cm-probl-signature';
    pre.textContent = code;
    dom.append(pre);
  }
  if (what) {
    const p = document.createElement('div');
    p.className = 'cm-probl-what';
    p.textContent = what;
    dom.append(p);
  }
  if (doc) {
    const p = document.createElement('div');
    p.className = 'cm-probl-doc';
    for (const [i, part] of doc.split('`').entries()) {
      if (i % 2) {
        const c = document.createElement('code');
        c.textContent = part;
        p.append(c);
      } else {
        p.append(part);
      }
    }
    dom.append(p);
  }
  return dom;
}

/** The editor's extensions, given the reference (`docs()` from the module),
 * `names(source)`, which gives a program's symbols, or null, and
 * `showReference(name)`, which shows a built-in's or keyword's entry. Put
 * them before `basicSetup`: Enter must reach them before the completion
 * list. */
export function intelligence(docs, names, showReference) {
  const builtins = new Map(docs.builtins.map((b) => [b.name, b]));
  builtins.set('read', docs.read);
  const keywords = new Map(docs.keywords.map((k) => [k.name, k]));

  const documented = (entry) => ({ code: entry.signature, doc: entry.summary });
  // A built-in's signature without its name, which the completion list
  // shows already.
  const parameters = (b) => (b.signature.startsWith(`${b.name}(`) ? b.signature.slice(b.name.length) : b.signature);

  // ── Completion ──
  async function complete(context) {
    if (inText(context.state, context.pos, -1)) return null;
    const word = context.matchBefore(/[A-Za-z_]\w*/);
    const from = word ? word.from : context.pos;
    const line = context.state.doc.lineAt(from);
    const before = context.state.sliceDoc(line.from, from);
    const dotted = /([A-Za-z_]\w*)?\.$/.exec(before);
    if (!word && !context.explicit && !dotted) return null;
    // Not while naming something new: a variable, a function or its
    // parameters, a type, or a loop's variable.
    if (/\b(let|var|fn|type|enum|for)\s+$|\bfn\s+\w+\s*\((.*,)?\s*$/.test(before)) return null;
    // The names as the program is now: the last check may be older.
    const symbols = (await names(context.state.doc.toString())) ?? context.state.field(symbolsField, false);
    const definitions = (symbols?.definitions ?? []).filter((d) => !d.gone);
    const options = [];
    const seen = new Set();
    const add = (option) => {
      if (seen.has(option.label)) return;
      seen.add(option.label);
      options.push(option);
    };
    const fromDefinition = (d, boost) => ({
      label: d.name,
      type: kinds[d.kind],
      detail: declaredAs(d),
      info: () => card(describeDefinition(d)),
      boost,
    });
    if (dotted) {
      const base = dotted[1];
      const enumType = definitions.find((d) => d.kind === 'enum' && d.name === base);
      if (enumType) {
        // (Without their enum's name beside them: it's just been typed.)
        definitions
          .filter((d) => d.kind === 'variant' && d.owner === base)
          .forEach((d) => add({ ...fromDefinition(d, 2), detail: undefined }));
        return { from, options, validFor: /^\w*$/ };
      }
      // Fields, and any function as a method: `x.f(y)` is `f(x, y)`.
      definitions.filter((d) => d.kind === 'field').forEach((d) => add(fromDefinition(d, 2)));
      definitions.filter((d) => d.kind === 'function').forEach((d) => add(fromDefinition(d, 1)));
      for (const b of docs.builtins) {
        add({ label: b.name, type: 'method', detail: parameters(b), info: () => card(documented(b)) });
      }
      return { from, options, validFor: /^\w*$/ };
    }
    if (symbols) {
      const inFunction = symbols.functions.some(([a, b]) => a <= from && from <= b);
      definitions
        .map((d, i) => ({ d, i }))
        .filter(
          ({ d }) =>
            d.kind !== 'field' && ((d.scope[0] <= from && from <= d.scope[1]) || (d.global && inFunction)),
        )
        .sort((a, b) => b.d.scope[0] - a.d.scope[0])
        .forEach(({ d }) => add(fromDefinition(d, d.kind === 'variable' || d.kind === 'parameter' ? 3 : 2)));
    }
    for (const [name, b] of builtins) {
      add({ label: name, type: 'function', detail: parameters(b), info: () => card(documented(b)) });
    }
    for (const [name, k] of keywords) {
      add({ label: name, type: 'keyword', info: () => card(documented(k)), boost: -1 });
    }
    return { from, options, validFor: /^\w*$/ };
  }

  // ── Hover ──
  const hover = hoverTooltip((view, pos, side) => {
    const word = wordAt(view.state, pos);
    if (!word || inText(view.state, pos, side)) return null;
    const def = definitionAt(view.state, pos);
    let content = null;
    if (def) content = describeDefinition(def);
    else if (builtins.has(word.text)) content = documented(builtins.get(word.text));
    else if (keywords.has(word.text)) content = documented(keywords.get(word.text));
    if (!content) return null;
    return { pos: word.from, end: word.to, above: true, create: () => ({ dom: card(content) }) };
  });

  /** The built-in or keyword at `pos`, which the reference documents. */
  function documentedAt(state, pos) {
    const word = wordAt(state, pos);
    if (!word || inText(state, pos, 1) || !(builtins.has(word.text) || keywords.has(word.text))) return null;
    return word;
  }

  // ── Going to a definition, or to the reference ──
  function goToDefinition(view, pos) {
    const def = definitionAt(view.state, pos);
    if (!def) {
      const word = documentedAt(view.state, pos);
      if (word) showReference(word.text);
      return word !== null;
    }
    view.dispatch({
      selection: { anchor: def.from, head: def.to },
      effects: EditorView.scrollIntoView(def.from, { y: 'center' }),
    });
    view.focus();
    return true;
  }

  // The name under the mouse while Cmd or Ctrl is held, underlined as a link.
  const setLink = StateEffect.define();
  const link = StateField.define({
    create: () => null,
    update(range, tr) {
      for (const effect of tr.effects) if (effect.is(setLink)) return effect.value;
      return range && tr.docChanged ? null : range;
    },
    provide: (field) =>
      EditorView.decorations.from(field, (range) =>
        range ? Decoration.set([Decoration.mark({ class: 'cm-probl-link' }).range(range.from, range.to)]) : Decoration.none,
      ),
  });
  const showLink = (view, event) => {
    let range = null;
    if (event.metaKey || event.ctrlKey) {
      const pos = view.posAtCoords({ x: event.clientX, y: event.clientY });
      const word = pos === null ? null : wordAt(view.state, pos);
      if (word && (definitionAt(view.state, pos) || documentedAt(view.state, pos))) range = { from: word.from, to: word.to };
    }
    const current = view.state.field(link);
    if (current?.from !== range?.from || current?.to !== range?.to) view.dispatch({ effects: setLink.of(range) });
  };

  // ── The name under the cursor, and its other uses ──
  const same = Decoration.mark({ class: 'cm-probl-same' });
  const highlights = EditorView.decorations.compute(['selection', symbolsField], (state) => {
    const symbols = state.field(symbolsField, false);
    const selection = state.selection.main;
    if (!symbols || !selection.empty) return Decoration.none;
    const def = definitionAt(state, selection.head);
    if (!def) return Decoration.none;
    const ranges = symbols.references.filter(([, , d]) => d === def.index).map(([from, to]) => [from, to]);
    ranges.push([def.from, def.to]);
    ranges.sort((a, b) => a[0] - b[0]);
    return Decoration.set(
      ranges.filter(([from, to]) => from < to).map(([from, to]) => same.range(from, to)),
      true,
    );
  });

  // ── A call's signature, while its arguments are typed ──
  // It shows when `(` or `,` is typed, or with Cmd-Shift-Space or
  // Ctrl-Shift-Space, and stays while the cursor is among that call's
  // arguments, until Escape.
  const askSignature = StateEffect.define();
  const hideSignature = StateEffect.define();

  /** What to show for a call: its signature, which parameter the argument
   * is, and the function's documentation. */
  function signatureFor(state, call) {
    const symbols = state.field(symbolsField, false);
    const own = symbols?.definitions.find((d) => d.kind === 'function' && !d.gone && d.name === call.name);
    const entry = own ? { signature: own.detail.replace(/^fn\s+/, ''), summary: own.doc } : builtins.get(call.name);
    const parsed = entry && parametersOf(entry.signature, call.name);
    if (!parsed) return null;
    // `xs.f(y)` is `f(xs, y)`: the first argument is before the dot.
    let index = call.index + (call.method && !parsed.method ? 1 : 0);
    if (index >= parsed.params.length) {
      index = parsed.params.findIndex(([a, b]) => entry.signature.slice(a, b) === '…');
    }
    return { ...call, signature: entry.signature, doc: entry.summary, param: parsed.params[index] ?? null };
  }

  // A call whose function isn't known yet, perhaps declared since the last
  // check, is `pending` until the checker's names come.
  const signature = StateField.define({
    create: () => null,
    update(value, tr) {
      if (tr.effects.some((e) => e.is(hideSignature))) return null;
      const asked = tr.effects.some((e) => e.is(askSignature));
      const named = value?.pending && tr.effects.some((e) => e.is(setSymbols));
      if (!tr.docChanged && !tr.selection && !asked && !named) return value;
      const { main } = tr.state.selection;
      const call = main.empty ? callAt(tr.state, main.head) : null;
      if (!call) return null;
      const kept = value !== null && tr.changes.mapPos(value.paren) === call.paren;
      if (!kept && !asked && !typedOneOf(tr, '(,')) return null;
      const same = kept && value.index === call.index && value.paren === call.paren && value.name === call.name;
      if (same && !value.pending) return value;
      return signatureFor(tr.state, call) ?? { ...call, pending: true };
    },
    provide: (field) =>
      showTooltip.from(field, (shown) =>
        shown && !shown.pending
          ? {
              pos: shown.from,
              above: true,
              create: () => {
                const dom = card({ doc: shown.doc });
                dom.classList.add('cm-probl-signature-help');
                const code = document.createElement('code');
                code.className = 'cm-probl-signature';
                if (shown.param) {
                  const [a, b] = shown.param;
                  const current = document.createElement('b');
                  current.textContent = shown.signature.slice(a, b);
                  code.append(shown.signature.slice(0, a), current, shown.signature.slice(b));
                } else {
                  code.textContent = shown.signature;
                }
                dom.prepend(code);
                return { dom };
              },
            }
          : null,
      ),
  });
  const namesForSignature = ViewPlugin.define((view) => {
    let asked = null;
    return {
      update(update) {
        const shown = update.state.field(signature);
        if (!shown?.pending || shown.paren === asked) return;
        asked = shown.paren;
        const source = update.state.doc.toString();
        names(source).then((symbols) => {
          if (symbols && view.state.doc.toString() === source) view.dispatch({ effects: setSymbols.of(symbols) });
        });
      },
    };
  });
  // Before the completion list's keys: its Escape also cancels a list still
  // being worked out, just after typing, so it would take the key while
  // nothing shows. A list that shows closes first.
  const signatureKeys = Prec.highest(
    keymap.of([
      {
        key: 'Mod-Shift-Space',
        run: (view) => (view.dispatch({ effects: askSignature.of(null) }), true),
      },
      {
        key: 'Escape',
        run: (view) => {
          if (!view.state.field(signature) || completionStatus(view.state) === 'active') return false;
          view.dispatch({ effects: hideSignature.of(null) });
          return true;
        },
      },
    ]),
  );

  // Enter takes a completion only when that changes something: with the
  // whole name typed, it starts a new line.
  const smartEnter = Prec.highest(
    keymap.of([
      {
        key: 'Enter',
        run(view) {
          if (completionStatus(view.state) !== 'active') return false;
          const { head } = view.state.selection.main;
          const typed = /\w*$/.exec(view.state.sliceDoc(view.state.doc.lineAt(head).from, head))[0];
          if (selectedCompletion(view.state)?.label === typed) closeCompletion(view);
          return false;
        },
      },
    ]),
  );

  return {
    complete,
    extensions: [
      smartEnter,
      symbolsField,
      signature,
      namesForSignature,
      signatureKeys,
      hover,
      link,
      highlights,
      keymap.of([{ key: 'F12', run: (view) => goToDefinition(view, view.state.selection.main.head) }]),
      EditorView.domEventHandlers({
        mousedown(event, view) {
          if (!(event.metaKey || event.ctrlKey) || event.button !== 0) return false;
          const pos = view.posAtCoords({ x: event.clientX, y: event.clientY });
          if (pos === null || !goToDefinition(view, pos)) return false;
          event.preventDefault();
          return true;
        },
        mousemove: (event, view) => showLink(view, event),
        keyup: (event, view) => {
          if (event.key === 'Meta' || event.key === 'Control') view.dispatch({ effects: setLink.of(null) });
          return false;
        },
        mouseleave: (event, view) => {
          if (view.state.field(link)) view.dispatch({ effects: setLink.of(null) });
          return false;
        },
      }),
      EditorView.theme({
        '.cm-probl-link': { textDecoration: 'underline', cursor: 'pointer', color: 'var(--accent)' },
        '.cm-probl-same': { backgroundColor: 'var(--same)', borderRadius: '2px' },
      }),
    ],
  };
}
