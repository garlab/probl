// Highlighting for Probl in CodeMirror: keywords, numbers with their
// percentages and dice, strings, comments and pragmas.

import { StreamLanguage } from '@codemirror/language';

const keywords = new Set([
  'and', 'break', 'chance', 'continue', 'div', 'else', 'enum', 'fn', 'for', 'if', 'import', 'in',
  'let', 'loop', 'match', 'mod', 'not', 'observe', 'or', 'repeat', 'report', 'return', 'simulate',
  'type', 'var', 'while', 'with',
  // Only keywords in their places, but always highlighted.
  'as', 'by', 'from', 'to',
]);
const atoms = new Set(['true', 'false']);
const types = new Set(['int', 'float', 'prob', 'bool', 'str', 'date', 'list', 'map', 'bag', 'dist']);

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

export const probl = StreamLanguage.define({
  name: 'probl',
  startState: () => ({ string: false }),
  token(stream, state) {
    if (state.string) return string(stream, state);
    if (stream.eatSpace()) return null;
    if (stream.match('#')) {
      stream.skipToEnd();
      return 'comment';
    }
    if (stream.match(/^@[A-Za-z_]+/)) return 'meta';
    if (stream.match(/^\d*d\d+\b/)) return 'number';
    if (stream.match(/^\d[\d_]*(\.[\d_]+)?(e[+-]?\d+)?%?/)) return 'number';
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
    stream.next();
    return null;
  },
  languageData: { commentTokens: { line: '#' } },
});
