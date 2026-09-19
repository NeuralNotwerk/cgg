
const PREC = {
    PROPERTY_BLOCK: 2,
    MULTILINE_EXPRESSION: 1,
};

module.exports = grammar({
    name: "kivy",

    externals: ($) => [
        $._indent,
        $._dedent,
        $._indent_error,
        $._section_end,
    ],

    extras: ($) => [
        /[ \t\f]+/,
    ],

    conflicts: ($) => [
        [
            $.multiline_property,
            $.property_block,
            $.empty_property,
        ],
        [$.inline_property, $.empty_property],
        [$.property_name, $._expression_part],
        [$.source_file, $.section],
        [$.section, $._section_body_item],
        [$.section, $._section_body_item, $._expression_part],
        [$._section_body_item, $._expression_part],
    ],

    word: ($) => $.identifier,

    rules: {
        source_file: ($) => repeat(choice(
            $._newline,
            $.directive,
            $.section,
            $.comment,
            $.rule,
            $.widget,
            $.indentation_error,
        )),

        _newline: () => /\r?\n/,

        comment: () => token(seq("#", /[^\r\n]*/)),

        section: ($) => prec.right(2, seq(
            repeat(seq(
                field("comment", alias(
                    $.comment,
                    $.section_leading_comment,
                )),
                $._newline,
            )),
            field("marker", $.section_marker),
            field("name", $.section_name),
            repeat(field(
                "comment",
                $.section_continuation,
            )),
            field("fold", $.section_fold),
        )),

        section_fold: ($) => seq(
            repeat($._newline),
            choice(
                field("body", $.section_body),
                $._section_end,
            ),
        ),

        section_body: ($) => seq(
            $._section_body_item,
            repeat(choice(
                $._newline,
                $._section_body_item,
            )),
            $._section_end,
        ),

        section_continuation: () => token(prec(2,
            /\r?\n[ \t]*#[^\r\n]*/,
        )),

        section_marker: () => token(prec(2,
            /#[ \t]*section:[ \t]*/,
        )),

        section_name: () => token(
            /[^ \t\r\n](?:[^\r\n]*[^ \t\r\n])?/,
        ),

        directive: () => token(prec(2, seq("#:", /[^\r\n]*/))),

        rule: ($) => seq(
            "<",
            commaSep1($.selector),
            ">",
            ":",
            field("body", $.block),
        ),

        selector: ($) => seq(
            field("name", $.class_name),
            optional(seq(
                "@",
                field("base", $.class_name),
                repeat(seq(
                    "+",
                    field("base", $.class_name),
                )),
            )),
        ),

        widget: ($) => seq(
            field("name", $.class_name),
            ":",
            field("body", $.block),
        ),

        block: ($) => seq(
            $._newline,
            repeat($._newline),
            $._indent,
            repeat(choice(
                $._newline,
                $._body_item,
            )),
            $._dedent,
        ),

        _body_item: ($) => choice(
            $.section,
            $._section_body_item,
        ),

        _section_body_item: ($) => choice(
            $.directive,
            $.rule,
            $.widget,
            $.canvas_block,
            $.comment,
            $.property,
            $.indentation_error,
        ),

        indentation_error: ($) => $._indent_error,

        canvas_block: ($) => seq(
            field("name", $.canvas_name),
            ":",
            field("body", $.block),
        ),

        canvas_name: () => choice(
            "canvas",
            "canvas.before",
            "canvas.after",
        ),

        property: ($) => choice(
            $.inline_property,
            $.multiline_property,
            $.property_block,
            $.empty_property,
        ),

        inline_property: ($) => prec.dynamic(
            PREC.PROPERTY_BLOCK + 1,
            seq(
                optional("-"),
                field("name", $.property_name),
                ":",
                field("value", $.expression),
            ),
        ),

        multiline_property: ($) => prec.dynamic(
            PREC.MULTILINE_EXPRESSION,
            seq(
                optional("-"),
                field("name", $.property_name),
                ":",
                field("value", $.multiline_expression),
            ),
        ),

        multiline_expression: ($) => seq(
            $._newline,
            repeat($._newline),
            $._indent,
            $.expression,
            repeat($._newline),
            $._dedent,
        ),

        property_block: ($) => prec.dynamic(
            PREC.PROPERTY_BLOCK,
            seq(
                optional("-"),
                field("name", $.property_name),
                ":",
                field("body", $.block),
            ),
        ),

        empty_property: ($) => seq(
            optional("-"),
            field("name", $.property_name),
            ":",
        ),

        property_name: ($) => seq(
            $.identifier,
            repeat(seq(
                ".",
                $.identifier,
            )),
        ),

        expression: ($) => prec.right(
            repeat1($._expression_part),
        ),

        _expression_part: ($) => choice(
            $.identifier,
            $.number,
            $.string,
            $.operator,
            $.punctuation,
            $.parenthesized_expression,
            $.list_expression,
            $.dictionary_expression,
            $.line_continuation,
            $.comment,
        ),

        parenthesized_expression: ($) => seq(
            "(",
            repeat(choice(
                $._newline,
                $._expression_part,
            )),
            ")",
        ),

        list_expression: ($) => seq(
            "[",
            repeat(choice(
                $._newline,
                $._expression_part,
            )),
            "]",
        ),

        dictionary_expression: ($) => seq(
            "{",
            repeat(choice(
                $._newline,
                $._expression_part,
            )),
            "}",
        ),

        line_continuation: () => token(/\\\r?\n/),

        class_name: () => /[A-Z_][A-Za-z0-9_]*/,

        identifier: () => /[A-Za-z_][A-Za-z0-9_]*/,

        number: () => token(choice(
            /0[xX](?:_?[0-9a-fA-F])+/,
            /0[oO](?:_?[0-7])+/,
            /0[bB](?:_?[01])+/,
            /(?:\d(?:_?\d)*)?\.\d(?:_?\d)*(?:[eE][+-]?\d(?:_?\d)*)?/,
            /\d(?:_?\d)*\.(?:[eE][+-]?\d(?:_?\d)*)?/,
            /\d(?:_?\d)*[eE][+-]?\d(?:_?\d)*/,
            /\d(?:_?\d)*/,
        )),

        string: () => token(choice(
            seq(
                optional(/[rRuUbBfF]{1,2}/),
                /"(?:[^"\\\r\n]|\\.)*"/,
            ),
            seq(
                optional(/[rRuUbBfF]{1,2}/),
                /'(?:[^'\\\r\n]|\\.)*'/,
            ),
            seq(
                optional(/[rRuUbBfF]{1,2}/),
                '"""',
                repeat(choice(
                    /[^"\\]+/,
                    /\\[\s\S]/,
                    /"[^"]/,
                    /""[^"]/,
                )),
                '"""',
            ),
            seq(
                optional(/[rRuUbBfF]{1,2}/),
                "'''",
                repeat(choice(
                    /[^'\\]+/,
                    /\\[\s\S]/,
                    /'[^']/,
                    /''[^']/,
                )),
                "'''",
            ),
        )),

        operator: () => token(choice(
            "**",
            "//",
            "<<",
            ">>",
            "<=",
            ">=",
            "==",
            "!=",
            ":=",
            "->",
            "+",
            "-",
            "*",
            "/",
            "%",
            "&",
            "|",
            "^",
            "~",
            "<",
            ">",
        )),

        punctuation: () => choice(
            ".",
            ",",
            ";",
            ":",
            "@",
            "=",
        ),
    },
});

function commaSep1(rule) {
    return seq(
        rule,
        repeat(seq(",", rule)),
    );
}

