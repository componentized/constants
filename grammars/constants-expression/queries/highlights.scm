; Highlights for expressions generating the items of a stream

(parameter (identifier) @variable.parameter)
(let_declaration pattern: (identifier) @variable)
(method_call method: (identifier) @function.method.call)
["$init" "$call" (index)] @variable.builtin
(field_index) @property
(builtin_field field: (identifier) @variable.parameter)
(primitive_type) @type.builtin
(parameter "_" @variable.parameter.builtin)
(identifier) @variable

(integer) @number
(float) @number.float
(boolean) @boolean

["if" "else"] @keyword.conditional
["let" (mutable)] @keyword
"while" @keyword.repeat
"as" @keyword.operator

(binary_expression operator: _ @operator)
(assignment_statement operator: _ @operator)
(assignment_statement left: (identifier) @variable)
(unary_expression operator: _ @operator)

["|" "||"] @punctuation.bracket
["(" ")" "{" "}" "[" "]"] @punctuation.bracket
["," ";" "." ":"] @punctuation.delimiter
"=" @operator
