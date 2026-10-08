// Highlighting for Probl in CodeMirror: keywords, numbers with their
// percentages and dice, strings, comments and pragmas. It also indents: a
// line inside brackets is one level in from the line that opened them.

import { StreamLanguage } from '@codemirror/language';
import { highlightCode, tagHighlighter, tags } from '@lezer/highlight';

const keywords = new Set([
  'and', 'break', 'catch', 'chance', 'continue', 'div', 'else', 'enum', 'fn', 'for', 'if', 'import', 'in',
  'let', 'loop', 'match', 'mod', 'not', 'observe', 'or', 'repeat', 'report', 'return', 'score', 'simulate',
  'try', 'type', 'typeof', 'var', 'while', 'with',
  // Only keywords in their places, but always highlighted.
  'as', 'by', 'from', 'to',
]);
const atoms = new Set(['true', 'false']);
const types = new Set(['int', 'float', 'complex', 'prob', 'bool', 'str', 'date', 'list', 'map', 'bag', 'dist']);

/** The rest of a string, up to its closing quote or the end of the line. */
function string(stream, state) {
  while (!stream.eol()) {
    const c = stream.next();
    if (c === '\\') stream.next();
    else if (c === '"') {
      state.string = false;
      break;
    }
  }
  return 'string';
}

const opening = new Set(['(', '[', '{']);
const closing = new Set([')', ']', '}']);

export const probl = StreamLanguage.define({
  name: 'probl',
  // `open`: for each bracket not yet closed, innermost last, the
  // indentation of the line it's on.
  startState: () => ({ string: false, open: [] }),
  copyState: (state) => ({ string: state.string, open: state.open.slice() }),
  token(stream, state) {
    if (state.string) return string(stream, state);
    if (stream.eatSpace()) return null;
    if (stream.match('#')) {
      stream.skipToEnd();
      return 'comment';
    }
    if (stream.match(/^@[A-Za-z_]+/)) return 'meta';
    if (stream.match(/^0(?:[bB][01](?:_?[01])*|[xX][\da-fA-F](?:_?[\da-fA-F])*)\b/)) return 'number';
    if (stream.match(/^\d*d\d+\b/)) return 'number';
    if (stream.match(/^\d[\d_]*(\.[\d_]+)?([eE][+-]?\d[\d_]*)?%?/)) return 'number';
    if (stream.peek() === '"') {
      stream.next();
      state.string = true;
      return string(stream, state);
    }
    if (stream.match(/^[A-Za-z_][A-Za-z0-9_]*/)) {
      const word = stream.current();
      if (keywords.has(word)) return 'keyword';
      if (atoms.has(word)) return 'atom';
      if (types.has(word) || /^[A-Z]/.test(word)) return 'typeName';
      if (stream.match(/^\s*\(/, false)) return 'variableName.function';
      return 'variableName';
    }
    if (stream.match(/^(->|=>|==|!=|<=|>=|\.\.<|\.\.|\+=|-=|\*=|\/=|[-+*/<>=~])/)) return 'operator';
    const c = stream.next();
    if (opening.has(c)) state.open.push(stream.indentation());
    else if (closing.has(c)) state.open.pop();
    return null;
  },
  indent(state, textAfter, cx) {
    if (!state.open.length) return 0;
    const opener = state.open[state.open.length - 1];
    // A line that starts by closing the bracket lines up with its opener's.
    return closing.has(textAfter[0]) ? opener : opener + cx.unit;
  },
  languageData: {
    commentTokens: { line: '#' },
    // Typing a closing bracket at the start of a line indents it again.
    indentOnInput: /^\s*[)\]}]$/,
    // `'` isn't a quote in Probl.
    closeBrackets: { brackets: ['(', '[', '{', '"'] },
  },
});

/** Classes for Probl's tokens, which the page's style colors: in the
 * editor, the guide's programs and the descriptions of names. */
export const highlighter = tagHighlighter([
  { tag: tags.keyword, class: 'tok-keyword' },
  { tag: [tags.number, tags.atom, tags.bool], class: 'tok-number' },
  { tag: tags.string, class: 'tok-string' },
  { tag: tags.comment, class: 'tok-comment' },
  { tag: tags.typeName, class: 'tok-type' },
  { tag: tags.function(tags.variableName), class: 'tok-function' },
  { tag: tags.meta, class: 'tok-meta' },
  { tag: tags.operator, class: 'tok-operator' },
]);

/** Probl code, highlighted: `text(part)` and `span(part, classes)` build
 * the output, as HTML or as elements. */
export function highlight(code, text, span) {
  const out = [];
  highlightCode(
    code,
    probl.parser.parse(code),
    highlighter,
    (part, classes) => out.push(classes ? span(part, classes) : text(part)),
    () => out.push(text('\n')),
  );
  return out;
}
