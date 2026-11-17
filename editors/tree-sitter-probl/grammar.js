// Syntax follows probl-syntax's lexer and Pratt parser. Generate with
// tree-sitter-cli 0.27.1; generated C is shipped to consumers.

const PREC = { or: 1, and: 2, not: 3, cmp: 4, range: 5, add: 6, mul: 7, neg: 8, pow: 9, typeof: 10, postfix: 11 };
const nl = $ => repeat($._newline);
const continued = ($, text) => seq(text, nl($));
const bracket = ($, text) => alias($[{
  '(': '_lparen', ')': '_rparen', '[': '_lbracket', ']': '_rbracket', '{': '_lbrace', '}': '_rbrace',
}[text]], text);
const commaSep = ($, rule) => optional(seq(rule, repeat(seq(continued($, ','), rule)), optional(continued($, ','))));
const braceSep = ($, rule) => seq(nl($), optional(seq(rule, nl($), repeat(seq(continued($, ','), rule, nl($))), optional(continued($, ',')))));
const parens = ($, rule) => seq(bracket($, '('), rule, bracket($, ')'));
const braces = ($, rule) => seq(bracket($, '{'), rule, bracket($, '}'));
const brackets = ($, rule) => seq(bracket($, '['), rule, bracket($, ']'));

// Explicit tiers enforce the compiler's non-associative comparisons/ranges.
// Prefix operators can occur in any operand position, as in the Pratt parser;
// their own binding power determines how much of the following input they take.
function expressionRules(header) {
  const name = part => part === 'expression' && header ? '_header_expression' : `_${part}${header ? '_header' : ''}`;
  const ref = ($, part) => $[name(part)];
  const node = ($, part, kind = part) => alias(ref($, part), $[`${kind}_expression`]);
  const rules = {};
  const rule = (part, body) => { rules[name(part)] = body; };
  rule('expression', $ => choice(ref($, 'or'), node($, 'lambda')));
  for (const [tier, next, power, operator] of [
    ['or', 'and', PREC.or, 'or'], ['and', 'comparison', PREC.and, 'and'],
    ['sum', 'product', PREC.add, choice('+', '-')],
    ['product', 'power', PREC.mul, choice('*', '/', 'div', 'mod')],
  ]) {
    rule(tier, $ => choice(ref($, next), node($, `${tier}_binary`, 'binary')));
    rule(`${tier}_binary`, $ => prec.left(power, seq(field('left', ref($, tier)),
      field('operator', operator), nl($), field('right', ref($, next)))));
  }
  rule('comparison', $ => choice(ref($, 'range'), node($, 'comparison_binary', 'binary')));
  rule('comparison_binary', $ => prec.left(PREC.cmp, seq(field('left', ref($, 'range')),
    field('operator', choice('==', '!=', '<', '<=', '>', '>=', 'in', $.not_in_operator)),
    nl($), field('right', ref($, 'range')))));
  rule('range', $ => choice(ref($, 'sum'), node($, 'range_binary', 'binary')));
  rule('range_binary', $ => prec.left(PREC.range, seq(field('left', ref($, 'sum')),
    // The contextual `to` is an identifier in the lexer, so it cannot continue a line.
    choice(seq(field('operator', choice('..', '..<')), nl($)), field('operator', 'to')),
    field('right', ref($, 'sum')))));
  rule('power', $ => choice(ref($, 'prefix'), node($, 'power_binary', 'binary')));
  rule('power_binary', $ => prec.right(PREC.pow, seq(field('left', ref($, 'prefix')),
    field('operator', '^'), nl($), field('right', ref($, 'power')))));
  rule('prefix', $ => choice(ref($, 'postfix'), node($, 'unary')));
  rule('unary', $ => choice(
    ...[[PREC.not, 'not', 'comparison'], [PREC.neg, '-', 'power'],
      [PREC.typeof, choice('typeof', '~'), 'prefix']].map(([power, operator, operand]) =>
      prec.right(power, seq(field('operator', operator), nl($), field('operand', ref($, operand))))),
  ));
  rule('postfix', $ => choice(ref($, 'primary'),
    ...['call', 'field', 'index', 'with'].map(part => node($, part))));
  rule('call', $ => prec.left(PREC.postfix, seq(field('function', ref($, 'postfix')), $.arguments)));
  rule('field', $ => prec.left(PREC.postfix, seq(field('value', ref($, 'postfix')),
    continued($, '.'), field('property', $._name))));
  rule('index', $ => prec.left(PREC.postfix, seq(field('value', ref($, 'postfix')),
    brackets($, field('index', $._expression)))));
  rule('with', $ => prec.left(PREC.postfix, seq(field('value', ref($, 'postfix')),
    continued($, 'with'), $.record_fields)));
  rule('lambda', $ => prec.right(seq(field('parameters', choice($._name, $.lambda_parameters)),
    continued($, '->'), field('body', ref($, 'expression')))));
  rule('primary', $ => choice($._name, $._literal, $.parenthesized_expression, $.list, $.map,
    $.if_expression, $.try_expression, $.chance_expression, $.match_expression, $.simulate_expression,
    ...(header ? [] : [$.record, $.typed_record, $.block])));
  return rules;
}

module.exports = grammar({
  name: 'probl',
  word: $ => $.identifier,
  inline: $ => [$._name],
  reserved: {
    global: $ => [
      'and', $.break_statement, 'catch', 'chance', $.continue_statement, 'div', 'else', 'enum',
      'false', 'fn', 'for', 'if', 'import', 'in', 'let', 'loop', 'match', 'mod', 'not',
      'observe', 'or', 'repeat', 'report', 'return', 'score', 'simulate', 'true', 'try',
      'type', 'typeof', 'var', 'while', 'with', $.wildcard,
    ],
  },
  extras: $ => [/[ \t\r]/, $.comment, $._soft_newline],
  externals: $ => [
    $._newline, $._soft_newline, $._end,
    $._lparen, $._rparen, $._lbracket, $._rbracket, $._lbrace, $._rbrace,
    $._string_start, $._string_end,
    $._error_sentinel,
  ],
  conflicts: $ => [
    [$.lambda_parameters, $._primary],
    [$._separator, $.record],
    [$.try_expression],
    [$.if_expression],
  ],
  rules: {
    source_file: $ => repeat(choice($._separator, seq($._item, $._terminator))),
    _separator: $ => choice($._newline, ';'),
    _terminator: $ => choice($._separator, $._end),
    _item: $ => choice($.pragma, $.function_declaration, $.type_declaration, $.enum_declaration,
      $.import_declaration, $.export_declaration, $.test_declaration, $._statement),
    pragma: $ => seq(continued($, '@'), field('name', $._name), optional(field('value', $._expression))),
    export_declaration: $ => seq('export', choice($.function_declaration, $.type_declaration, $.enum_declaration, $.binding)),
    function_declaration: $ => seq('fn', field('name', $._name), $.parameters,
      optional(seq(continued($, ':'), field('return_type', $._type))), field('body', $.block)),
    parameters: $ => parens($, commaSep($, $.parameter)),
    parameter: $ => seq(field('name', $._name), optional(seq(continued($, ':'), field('type', $._type)))),
    type_declaration: $ => seq('type', field('name', $.type_identifier), continued($, '='), $._type),
    enum_declaration: $ => seq('enum', field('name', $.type_identifier), $.enum_body),
    enum_body: $ => braces($, braceSep($, $._name)),
    _type: $ => choice($.type_identifier, $.generic_type, $.record_type, $.union_type, $.parenthesized_type),
    type_identifier: $ => $._name,
    generic_type: $ => seq($.type_identifier, brackets($, seq($._type, repeat(seq(continued($, ','), $._type))))),
    record_type: $ => braces($, braceSep($, $.type_field)),
    type_field: $ => seq(field('name', $._name), continued($, ':'), field('type', $._type)),
    union_type: $ => prec.left(1, seq($._type, continued($, 'or'), $._type)),
    parenthesized_type: $ => parens($, $._type),
    import_declaration: $ => seq('import', optional(seq($.import_names, 'from')), field('path', $.string)),
    import_names: $ => braces($, braceSep($, $.import_name)),
    import_name: $ => seq($._name, optional(seq('as', field('alias', $._name)))),
    test_declaration: $ => seq('test', field('name', $.string), $.test_body),
    test_body: $ => braces($, repeat(choice($._separator, seq(choice($.expect_statement, $._statement), $._terminator)))),
    expect_statement: $ => prec(1, seq('expect', $._report_value)),

    _statement: $ => choice($.binding, $.assignment, $.for_statement, $.while_statement, $.repeat_statement,
      $.loop_statement, $.break_statement, $.continue_statement, $.return_statement, $.observe_statement,
      $.score_statement, $.report_statement, $.expression_statement),
    binding: $ => seq(choice('let', 'var'), field('pattern', $._pattern),
      optional(seq(continued($, ':'), field('type', $._type))),
      field('operator', choice('=', '~')), nl($), field('value', $._expression)),
    assignment: $ => seq(field('target', $._expression), field('operator', choice('=', '~', '+=', '-=', '*=', '/=')), nl($), field('value', $._expression)),
    for_statement: $ => seq('for', $._pattern, continued($, 'in'), field('value', $._header_expression), $.block),
    while_statement: $ => seq('while', field('condition', $._header_expression), $.block),
    repeat_statement: $ => seq('repeat', field('count', $._header_expression), $.block),
    loop_statement: $ => seq('loop', $.block),
    break_statement: _ => 'break',
    continue_statement: _ => 'continue',
    return_statement: $ => seq('return', optional($._expression)),
    observe_statement: $ => seq('observe', field('value', $._expression), optional(seq('from', field('distribution', $._expression)))),
    score_statement: $ => seq('score', $._expression),
    report_statement: $ => seq('report', $._report_value),
    _report_value: $ => seq(field('value', $._expression), optional(seq('by', field('key', $._expression))), optional(seq('as', field('label', $.string)))),
    expression_statement: $ => $._expression,
    block: $ => braces($, repeat(choice($._separator, seq($._statement, $._terminator)))),

    _pattern: $ => choice($._name, $.wildcard, $.variant_pattern, $.list_pattern, $.or_pattern,
      $.number, $.percentage, $.string, $.temporal, $.boolean, $.negative_pattern),
    wildcard: _ => '_',
    variant_pattern: $ => seq($.type_identifier, continued($, '.'), field('variant', $._name)),
    list_pattern: $ => brackets($, commaSep($, $._pattern)),
    or_pattern: $ => prec.left(1, seq($._pattern, continued($, '|'), $._pattern)),
    negative_pattern: $ => seq(continued($, '-'), choice($.number, $.percentage)),

    ...expressionRules(false),
    ...expressionRules(true),
    parenthesized_expression: $ => parens($, $._expression),
    lambda_parameters: $ => parens($, commaSep($, $._name)),
    arguments: $ => parens($, commaSep($, choice($._expression, $.named_argument, $.spread))),
    named_argument: $ => seq(field('name', $._name), continued($, ':'), field('value', $._expression)),
    spread: $ => seq('...', $._expression),
    list: $ => brackets($, commaSep($, choice($._expression, $.spread))),
    map: $ => brackets($, choice(':', seq($.map_entry, repeat(seq(continued($, ','), $.map_entry)), optional(continued($, ','))))),
    map_entry: $ => seq(field('key', $._expression), continued($, ':'), field('value', $._expression)),
    // `{ x }` is a block; an anonymous record needs a colon or comma.
    record: $ => braces($, seq(nl($), choice(
      seq($.record_field, nl($), repeat(seq(continued($, ','), $._record_field, nl($))), optional(continued($, ','))),
      seq($.shorthand_field, ',', braceSep($, $._record_field)),
    ))),
    typed_record: $ => seq(field('type', $.type_identifier), $.record_fields),
    record_fields: $ => braces($, braceSep($, $._record_field)),
    _record_field: $ => choice($.record_field, $.shorthand_field),
    record_field: $ => seq(field('name', $._name), continued($, ':'), field('value', $._expression)),
    shorthand_field: $ => $._name,
    if_expression: $ => seq('if', field('condition', $._header_expression), field('consequence', $.block),
      optional(seq(nl($), 'else', field('alternative', choice($.block, $.if_expression))))),
    try_expression: $ => seq('try', $.block, repeat1(seq(nl($), $.catch_clause))),
    catch_clause: $ => seq('catch', optional(field('fault', $.type_identifier)), $.block),
    simulate_expression: $ => seq('simulate', $.block),
    chance_expression: $ => seq('chance', $.chance_body),
    chance_body: $ => braces($, repeat(choice($._newline, ',', $.chance_arm))),
    chance_arm: $ => seq(field('weight', choice('else', $._expression)), continued($, '=>'), $._statement, choice($._newline, ',', $._end)),
    match_expression: $ => seq('match', field('value', $._header_expression), $.match_body),
    match_body: $ => braces($, repeat(choice($._newline, ',', $.match_arm))),
    match_arm: $ => seq($._pattern, optional(seq('if', field('guard', $._expression))), continued($, '=>'), $._statement, choice($._newline, ',', $._end)),

    // Contextual tokens are also available as names. This preserves calls like
    // `export(1)` and `test(2)` at item boundaries, where both interpretations
    // initially fit. Aliasing keeps those uses out of keyword queries.
    _name: $ => choice($.identifier, ...['as', 'by', 'from', 'to', 'export', 'test', 'expect'].map(word => alias(word, $.identifier))),
    not_in_operator: $ => seq(continued($, 'not'), 'in'),
    // d followed only by digits is always a die, including in a name position.
    identifier: _ => token(choice(/[A-Za-ce-z_][A-Za-z0-9_]*/, 'd', /d[A-Za-z_][A-Za-z0-9_]*/, /d[0-9]+[A-Za-z_][A-Za-z0-9_]*/)),
    _literal: $ => choice($.number, $.percentage, $.dice, $.boolean, $.temporal, $.string),
    number: _ => token(choice(
      /[0-9](_?[0-9])*(\.[0-9](_?[0-9])*)?([eE][+-]?[0-9](_?[0-9])*)?/,
      /0[bB][01](_?[01])*/, /0[xX][0-9a-fA-F](_?[0-9a-fA-F])*/,
    )),
    percentage: _ => /[0-9](_?[0-9])*(\.[0-9](_?[0-9])*)?([eE][+-]?[0-9](_?[0-9])*)?%/,
    // A bare d6 is lexed as a word: d6_0 is a name, whereas 2d6_0 is dice.
    dice: _ => token(choice(/d[0-9]+/, /[0-9](_?[0-9])*d[0-9](_?[0-9])*/)),
    boolean: _ => choice('true', 'false'),
    temporal: _ => /'[^'\r\n]*'/,
    string: $ => seq(alias($._string_start, '"'), repeat(choice($.string_content, $.escape_sequence, $.interpolation)), alias($._string_end, '"')),
    string_content: _ => token.immediate(prec(1, /[^"\\{}\n]+/)),
    escape_sequence: _ => token.immediate(/\\[ntr0\\"{}]/),
    interpolation: $ => seq(token.immediate('{'), $._expression, '}'),
    comment: _ => token(seq('#', /[^\n]*/)),
  },
});
