; Specific names precede generic identifiers for consumers that use first match.
(function_declaration name: (identifier) @function)
(call_expression function: (identifier) @function)
(call_expression function: (field_expression property: (identifier) @function.method))
(type_identifier) @type
(parameter name: (identifier) @variable.parameter)
(pragma "@" @attribute name: (identifier) @attribute)
[(record_field name: (identifier) @property) (type_field name: (identifier) @property)
 (named_argument name: (identifier) @property) (field_expression property: (identifier) @property)
 (shorthand_field (identifier) @property)]
(enum_body (identifier) @constant)
(variant_pattern variant: (identifier) @constant)

["let" "var" "fn" "type" "enum" "import" "export" "from" "as" "by"
 "test" "expect" "for" "while" "repeat" "loop" (break_statement) (continue_statement) "return"
 "if" "else" "match" "chance" "try" "catch" "simulate" "observe" "score" "report"] @keyword
["and" "or" "not" "in" "div" "mod" "typeof" "with" "to"
 "=" "~" "+=" "-=" "*=" "/=" "==" "!=" "<" "<=" ">" ">="
 "+" "-" "*" "/" "^" ".." "..<" "..." "->" "=>" "|"] @operator
[(number) (percentage) (dice)] @number
(boolean) @constant.builtin
(temporal) @constant
(comment) @comment
; Never capture the whole string: interpolation children must retain their colors.
(string ["\"" (string_content)] @string)
(escape_sequence) @string.escape
(interpolation ["{" "}"] @punctuation.special)
["(" ")" "[" "]" "{" "}"] @punctuation.bracket
["," ";" ":" "."] @punctuation.delimiter
(identifier) @variable
