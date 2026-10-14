// -------------------------------------------------------------------
// Project:    tree-sitter-smithy
// Author:     Simon Johnston <johntonskj@gmail.com>
// Version:    0.1.0
// Repository: https://github.com/johnstonskj/tree-sitter-smithy
// License:    MIT
// Copyright:  Copyright (c) 2023 Simon Johnston
//
// This grammar is based on the Smithy 2.0 specification at
// https://smithy.io/2.0/spec/idl.html
//
// For specific details on the translation from the Smithy BNF to this
// grammar see the "Grammar" section in README.org
//
// -------------------------------------------------------------------

const PREC_KEYWORD = 2
const PREC_BINDING = 1
const PREC_EOL = 0
const PREC_COMMENT = 0

function keyword(str) {
    return token(
        prec(
            PREC_KEYWORD,
            str
        )
    );
}

function seq_comma(rule) {
    return seq(
        rule,
        optional(',')
    );
}

function comma_repeat1(rule) {
    return seq(
        seq_comma(rule),
        repeat(
            seq_comma(rule)
        )
    );
}

function comma_repeat(rule) {
    return optional(
        comma_repeat1(rule)
    );
}

module.exports = grammar({
    name: 'smithy',

    word: $ => $._identifier,

    // -----------------------------------------------------------------------
    // Configuration
    // -----------------------------------------------------------------------

    supertypes: $ => [
        // think about this more: _simple_shape, _aggr_shape, _service_shape,
    ],

    // -----------------------------------------------------------------------
    // Whitespace
    // -----------------------------------------------------------------------

    extras: $ => [
        // One or more spaces or tabs
        /\s/,
        $.documentation_comment,
        $.line_comment
    ],

    // -----------------------------------------------------------------------
    // IDL
    // -----------------------------------------------------------------------

    rules: {

        // idl =
        //     [WS] ControlSection MetadataSection ShapeSection
        idl: $ => seq(
            optional($.control_section),
            optional($.metadata_section),
            optional($.shape_section)
        ),

        // -------------------------------------------------------------------
        // Whitespace
        // -------------------------------------------------------------------

        _eol: $ => token(
            prec(
                PREC_EOL,
                /(\n|\r\n)+/
            )
        ),

        // -------------------------------------------------------------------
        // Comments
        // -------------------------------------------------------------------

        // Comment =
        //     DocumentationComment / LineComment

        // DocumentationComment =
        //     "///" *NotNL NL
        documentation_comment: $ => token(
            prec(
                PREC_COMMENT,
                seq('///', /.*/)
            )
        ),

        // LineComment =
        //     "//" *NotNL NL
        line_comment: $ => token(
            prec(
                PREC_COMMENT, 
                seq('//', /.*/)
            )
        ),

        // -------------------------------------------------------------------
        // Control
        // -------------------------------------------------------------------

        // ControlSection =
        //     *(ControlStatement)
        control_section: $ => repeat1(
            $.control_statement
        ),

        // ControlStatement =
        //     "$" NodeObjectKey [SP] ":" [SP] NodeValue BR
        control_statement: $ => seq(
            '$',
            field('key', $.node_object_key),
            ':',
            field('value', $.node_value),
            $._eol
        ),

        // -------------------------------------------------------------------
        // Metadata
        // -------------------------------------------------------------------

        // MetadataSection =
        //     *(MetadataStatement)
        metadata_section: $ => repeat1(
            $.metadata_statement
        ),

        // MetadataStatement =
        //     %s"metadata" SP NodeObjectKey [SP] "=" [SP] NodeValue BR
        metadata_statement: $ => seq(
            keyword('metadata'),
            field('key', $.node_object_key),
            '=',
            field('value', $.node_value),
            $._eol
        ),

        // -------------------------------------------------------------------
        // Node values
        // -------------------------------------------------------------------

        // NodeValue =
        //     NodeArray / NodeObject / Number / NodeKeywords / NodeStringValue
        node_value: $ => choice(
            $.node_array,
            $.node_object,
            $.number,
            $.boolean,
            $.null,
            $.string_value
        ),

        // NodeArray =
        //     "[" [WS] *(NodeValue [WS]) "]"
        node_array: $ => seq(
            '[',
            repeat($.node_value),
            ']'
        ),

        // NodeObject =
        //     "{" [WS] [NodeObjectKvp *(WS NodeObjectKvp)] [WS] "}"
        node_object: $ => seq(
            '{',
            comma_repeat($.node_object_kvp),
            '}'
        ),

        // NodeObjectKvp =
        //     NodeObjectKey [WS] ":" [WS] NodeValue
        node_object_kvp: $ => prec(
            PREC_BINDING,
            seq(
                field('key', $.node_object_key),
                ':',
                field('value', $.node_value)
            )
        ),

        // NodeObjectKey =
        //     QuotedText / Identifier
        node_object_key: $ => choice(
            prec(
                PREC_BINDING,
                $.identifier
            ),
            $._quoted_text
        ),

        // Number = [Minus] Int [Frac] [Exp]
        // DecimalPoint = %x2E ; .
        // DigitOneToNine = %x31-39 ; 1-9
        // E = %x65 / %x45 ; e E
        // Exp = E [Minus / Plus] 1*DIGIT
        // Frac = DecimalPoint 1*DIGIT
        // Int = Zero / (DigitOneToNine *DIGIT)
        // Minus = %x2D ; -
        // Plus = %x2B ; +
        // Zero = %x30 ; 0
        number: $ => token(
            seq(
                optional('-'),
                choice(
                    '0',
                    /[1-9][0-9]*/
                ),
                optional(
                    /\.[0-9]+/
                ),
                optional(
                    /[eE][+\-]?[0-9]+/
                )
            )
        ),

        // NodeKeywords =
        //     %s"true" / %s"false" / %s"null"
        boolean: $ => choice(
            keyword('true'),
            keyword('false')
        ),

        null: $ => keyword('null'),

        // NodeStringValue =
        //     ShapeId / TextBlock / QuotedText
        string_value: $ => choice(
            $.shape_id,
            $.string
        ),

        string: $ => choice(
            $._text_block,
            $._quoted_text
        ),

        // TextBlock =
        //     ThreeDquotes [SP] NL *TextBlockContent ThreeDquotes
        // TextBlockContent =
        //     QuotedChar / (1*2DQUOTE 1*QuotedChar)
        // ThreeDquotes =
        //     DQUOTE DQUOTE DQUOTE
        _text_block: $ => seq(
            '"""',
            $._eol,
            repeat(
                choice($._quoted_char, /"["]?[^"]/)
            ),
            '"""'
        ),

        // QuotedText =
        //     DQUOTE *QuotedChar DQUOTE
        _quoted_text: $ => seq(
            '"',
            repeat($._quoted_char),
            '"'
        ),

        // QuotedChar =
        //     %x09        ; tab
        //     / %x20-21     ; space - "!"
        //     / %x23-5B     ; "#" - "["
        //     / %x5D-10FFFF ; "]"+
        //     / EscapedChar
        //     / NL
        // EscapedChar =
        //     Escape (Escape / DQUOTE / %s"b" / %s"f"
        //             / %s"n" / %s"r" / %s"t" / "/"
        //             / UnicodeEscape)
        // UnicodeEscape = %s"u" Hex Hex Hex Hex
        // Hex = DIGIT / %x41-46 / %x61-66
        // Escape = %x5C ; backslash
        _quoted_char: $ => /([^ --"\\])|(\\[\\"bBfFnNrRtT\/])|(\\[uU][0-9a-fA-F]{4})/,

        // -------------------------------------------------------------------
        // Shape IDs
        // -------------------------------------------------------------------

        // EXPANDED: ShapeId =
        //     (((Identifier *("." Identifier)) "#" Identifier) / Identifier)
        //         [("$" Identifier)]

        // ShapeId =
        //     RootShapeId [ShapeIdMember]
        shape_id: $ => seq(
            $._root_shape_id,
            optional($.shape_id_member),
        ),

        // RootShapeId =
        //     AbsoluteRootShapeId / Identifier
        _root_shape_id: $ => choice(
            $._absolute_root_shape_id,
            field('shape_id', $.identifier)
        ),

        // AbsoluteRootShapeId =
        //     Namespace "#" Identifier
        _absolute_root_shape_id: $ => seq(
            field('namespace', $.namespace),
            token.immediate('#'),
            field('shape_id', $.identifier)
        ),

        // Namespace =
        //     Identifier *("." Identifier)
        namespace: $ => seq(
            $._identifier,
            repeat(
                seq(
                    token.immediate('.'),
                    $._identifier
                )
            )
        ),

        // ShapeIdMember =
        //     "$" Identifier
        shape_id_member: $ => seq(
            token.immediate('$'),
            field('member_id', $.identifier)
        ),

        // Identifier =
        //     IdentifierStart *IdentifierChars
        // IdentifierStart =
        //     (1*"_" (ALPHA / DIGIT)) / ALPHA
        // IdentifierChars =
        //     ALPHA / DIGIT / "_"
        identifier: $ => $._identifier,

        _identifier: $ => token(
            /((_+[a-zA-Z0-9])|[a-zA-Z])[a-zA-Z0-9_]*/
        ),

        // -------------------------------------------------------------------
        // Shapes
        // -------------------------------------------------------------------

        // ShapeSection =
        //     [NamespaceStatement UseSection [ShapeStatements]]
        shape_section: $ => seq(
            field(
                'namespace',
                $.namespace_statement
            ),
            field(
                'uses',
                // UseSection =
                //     *(UseStatement)
                repeat(
                    $._use_statement
                )
            ),
            field(
                'shapes',
                optional(
                    $.shape_statements
                )
            )
        ),

        // NamespaceStatement =
        //     %s"namespace" SP Namespace BR
        namespace_statement: $ => seq(
            keyword("namespace"),
            $.namespace,
            $._eol
        ),

        // UseStatement =
        //     %s"use" SP AbsoluteRootShapeId BR
        _use_statement: $ => seq(
            keyword("use"),
            $.external_shape_id,
            $._eol
        ),

        external_shape_id: $ => $._absolute_root_shape_id,

        // ShapeStatements =
        //     ShapeOrApplyStatement *(BR ShapeOrApplyStatement)
        shape_statements: $ => repeat1(
            seq(
                $._shape_or_apply_statement,
                $._eol
            )
        ),

        // ShapeOrApplyStatement =
        //     ShapeStatement / ApplyStatement
        _shape_or_apply_statement: $ => choice(
            $.shape_statement,
            $.apply_statement
        ),

        // ShapeStatement =
        //     TraitStatements ShapeBody
        shape_statement: $ => seq(
            field('traits', optional($.applied_traits)),
            field('body', $._shape_body)
        ),

        // ShapeBody =
        //     SimpleShapeStatement
        //     / EnumShapeStatement / ListStatement / MapStatement
        //     / StructureStatement / UnionStatement
        //     / ServiceStatement / OperationStatement / ResourceStatement
        _shape_body: $ => choice(
            $.simple_shape_statement,
            $.enum_statement,
            $.list_statement,
            $.map_statement,
            $.structure_statement,
            $.union_statement,
            $.service_statement,
            $.operation_statement,
            $.resource_statement
        ),

        // SimpleShapeStatement =
        //     SimpleTypeName SP Identifier [Mixins]
        simple_shape_statement: $ => seq(
            field('type', $.simple_type_name),
            field('name', $.identifier),
            field('mixins', optional($.mixins))
        ),

        // SimpleTypeName =
        //     %s"blob" / %s"boolean" / %s"document" / %s"string"
        //     / %s"byte" / %s"short" / %s"integer" / %s"long"
        //     / %s"float" / %s"double" / %s"bigInteger"
        //     / %s"bigDecimal" / %s"timestamp"
        simple_type_name: $ => choice(
            keyword('blob'),
            keyword('boolean'),
            keyword('document'),
            keyword('string'),
            keyword('byte'),
            keyword('short'),
            keyword('integer'),
            keyword('long'),
            keyword('float'),
            keyword('double'),
            keyword('big_integer'),
            keyword('big_decimal'),
            keyword('timestamp')
        ),

        // Mixins =
        //     [SP] %s"with" [WS] "[" 1*([WS] ShapeId) [WS] "]"
        mixins: $ => seq(
            keyword('with'),
            '[',
            comma_repeat1($.shape_id),
            ']'
        ),

        // EnumShapeStatement =
        //     EnumTypeName SP Identifier [Mixins] [WS] EnumShapeMembers
        // EnumTypeName =
        //     %s"enum" / %s"intEnum"
        enum_statement: $ => seq(
            field(
                'type',
                choice(
                    $.type_enum,
                    $.type_int_enum
                )
            ),
            field('name', $.identifier),
            field('mixins', optional($.mixins)),
            field('members', $.enum_members)
        ),

        type_enum: $ => keyword('enum'),

        type_int_enum: $ => keyword('intEnum'),

        // EnumShapeMembers =
        //     "{" [WS] 1*(TraitStatements Identifier [ValueAssignment] [WS]) "}"
        enum_members: $ => seq(
            '{',
            comma_repeat1(
                $.enum_member
            ),
            '}'
        ),

        // ==> TraitStatements Identifier [ValueAssignment] [WS]
        enum_member: $ => seq(
            field('traits', optional($.applied_traits)),
            field('name', $.identifier),
            field('value', optional($.value_assignment))
        ),

        // ValueAssignment =
        //     [SP] "=" [SP] NodeValue BR
        value_assignment: $ => seq(
            '=',
            $.node_value,
        ),

        // ListStatement =
        //     %s"list" SP Identifier [Mixins] [WS] ListMembers
        list_statement: $ => seq(
            keyword('list'),
            field('name', $.identifier),
            field('mixins', optional($.mixins)),
            $._list_members),

        // ListMembers =
        //     "{" [WS] [ListMember] [WS] "}"
        _list_members: $ => seq(
            '{',
            $.list_member,
            '}'
        ),

        // ListMember =
        //     TraitStatements (ElidedListMember / ExplicitListMember)
        // ElidedListMember =
        //     %s"$member"
        // ExplicitListMember =
        //     %s"member" [SP] ":" [SP] ShapeId
        list_member: $ => seq(
            field('traits', optional($.applied_traits)),
            keyword('member'),
            field(
                'member_type',
                optional(
                    seq(
                        ':',
                        $.shape_id
                    )
                )
            )
        ),

        // MapStatement =
        //     %s"map" SP Identifier [Mixins] [WS] MapMembers
        map_statement: $ => seq(
            keyword('map'),
            field('name', $.identifier),
            field('mixins', optional($.mixins)),
            $._map_members
        ),

        // MapMembers =
        //     "{" [WS] [MapKey / MapValue / (MapKey WS MapValue)] [WS] "}"
        _map_members: $ => seq(
            '{',
            repeat(
                choice(
                    $.map_key,
                    $.map_value,
                    seq(
                        $.map_key,
                        $.map_value
                    )
                )
            ),
            '}'
        ),

        // MapKey =
        //     TraitStatements (ElidedMapKey / ExplicitMapKey)
        // ElidedMapKey =
        //     %s"$key"
        // ExplicitMapKey =
        //     %s"key" [SP] ":" [SP] ShapeId
        map_key: $ => seq(
            field('traits', optional($.applied_traits)),
            keyword('key'),
            field(
                'key_type',
                optional(
                    seq(
                        ':',
                        $.shape_id
                    )
                )
            )
        ),

        // MapValue =
        //     TraitStatements (ElidedMapValue / ExplicitMapValue)
        // ElidedMapValue =
        //     %s"$value"
        // ExplicitMapValue =
        //     %s"value" [SP] ":" [SP] ShapeId
        map_value: $ => seq(
            field('traits', repeat($.trait)),
            keyword('value'),
            field(
                'value_type',
                optional(
                    seq(
                        ':',
                        $.shape_id
                    )
                )
            )
        ),

        // StructureStatement =
        //     %s"structure" SP Identifier [StructureResource]
        //         [Mixins] [WS] StructureMembers
        structure_statement: $ => seq(
            keyword('structure'),
            field('name', $.identifier),
            field('resource', optional($.structure_resource)),
            field('mixins', optional($.mixins)),
            field('members', $.structure_members)
        ),

        // StructureResource =
        //     SP %s"for" SP ShapeId
        structure_resource: $ => seq(
            keyword('for'),
            $.shape_id
        ),

        // StructureMembers =
        //     "{" [WS] *(TraitStatements StructureMember [WS]) "}"
        structure_members: $ => seq(
            '{',
            comma_repeat($.structure_member),
            '}'
        ),

        // StructureMember =
        //     (ExplicitStructureMember / ElidedStructureMember) [ValueAssignment]
        // ExplicitStructureMember =
        //     Identifier [SP] ":" [SP] ShapeId
        // ElidedStructureMember =
        //     "$" Identifier
        structure_member: $ => seq(
            field('traits', repeat($.trait)),
            $._structure_or_union_member,
            field('value', optional($.value_assignment))
        ),

        // StructureMember =
        //     (ExplicitStructureMember / ElidedStructureMember)
        // ExplicitStructureMember =
        //     Identifier [SP] ":" [SP] ShapeId
        // ElidedStructureMember =
        //     "$" Identifier
        _structure_or_union_member: $ => seq(
            choice(
                seq(
                    field('name', $.identifier),
                    ':',
                    field('type', $.shape_id)
                ),
                seq(
                    '$',
                    field('name', $.identifier)
                )
            ),
        ),

        // UnionStatement =
        //     %s"union" SP Identifier [Mixins] [WS] UnionMembers
        union_statement: $ => seq(
            keyword('union'),
            field('name', $.identifier),
            field('mixins', optional($.mixins)),
            field('members', $.union_members)
        ),

        // UnionMembers =
        //     "{" [WS] *(TraitStatements UnionMember [WS]) "}"
        union_members: $ => seq(
            '{',
            comma_repeat($.union_member),
            '}'
        ),

        // UnionMember =
        //     (ExplicitStructureMember / ElidedStructureMember)
        union_member: $ => seq(
            field('traits', optional($.applied_traits)),
            $._structure_or_union_member
        ),

        // ServiceStatement =
        //     %s"service" SP Identifier [Mixins] [WS] NodeObject
        service_statement: $ => seq(
            keyword('service'),
            field('name', $.identifier),
            field('mixins', optional($.mixins)),
            field('members', $.node_object)
        ),

        // ResourceStatement =
        //     %s"resource" SP Identifier [Mixins] [WS] NodeObject
        resource_statement: $ => seq(
            keyword('resource'),
            field('name', $.identifier),
            field('mixins', optional($.mixins)),
            field('members', $.node_object)
        ),

        // OperationStatement =
        //     %s"operation" SP Identifier [Mixins] [WS] OperationBody
        operation_statement: $ => seq(
            keyword('operation'),
            field('name', $.identifier),
            field('mixins', optional($.mixins)),
            field('body', $.operation_body)
        ),

        // OperationBody =
        //     "{" [WS]
        //     *(OperationInput / OperationOutput / OperationErrors)
        //     [WS] "}"
        //     ; only one of each property can be specified.
        operation_body: $ => seq(
            '{',
            comma_repeat(
                seq(
                    // Note that this is missing from the Smithy specification!
                    field('traits', optional($.applied_traits)),
                    choice(
                        $.operation_input,
                        $.operation_output,
                        $.operation_errors
                    )
                )
            ),
            '}'
        ),

        // OperationInput =
        //     %s"input" [WS] (InlineStructure / (":" [WS] ShapeId)) WS
        operation_input: $ => seq(
            keyword('input'),
            field(
                'type',
                choice(
                    $.inline_structure,
                    seq(
                        ':',
                        $.shape_id
                    )
                )
            )
        ),

        // OperationOutput =
        //     %s"output" [WS] (InlineStructure / (":" [WS] ShapeId)) WS
        operation_output: $ => seq(
            keyword('output'),
            field(
                'type',
                choice(
                    $.inline_structure,
                    seq(
                        ':',
                        $.shape_id
                    )
                )
            )
        ),

        // OperationErrors =
        //     %s"errors" [WS] ":" [WS] "[" *([WS] Identifier) [WS] "]" WS
        operation_errors: $ => seq(
            keyword('errors'),
            ':',
            '[',
            comma_repeat($.identifier),
            ']'
        ),

        // InlineStructure =
        //     ":=" [WS] TraitStatements [StructureResource]
        //          [Mixins] [WS] StructureMembers
        inline_structure: $ => seq(
            ':=',
            field('traits', optional($.applied_traits)),
            field('resource', optional($.structure_resource)),
            field('mixins', optional($.mixins)),
            field('members', $.structure_members)
        ),

        // -------------------------------------------------------------------
        // Traits
        // -------------------------------------------------------------------

        // TraitStatements =
        //     *([WS] Trait) [WS]
        applied_traits: $ => repeat1(
            $.trait
        ),

        // Trait =
        //     "@" ShapeId [TraitBody]
        trait: $ => seq(
            '@',
            field('type', $.shape_id),
            field('body', optional($._trait_body))
        ),

        // TraitBody =
        //     "(" [WS] [TraitBodyValue] [WS] ")"
        _trait_body: $ => seq(
            '(',
            optional($._trait_body_value),
            ')'
        ),

        // TraitBodyValue =
        //     TraitStructure / NodeValue
        _trait_body_value: $ => choice(
            $.trait_structure,
            $.node_value
        ),

        // TraitStructure =
        //     TraitStructureKvp *([WS] TraitStructureKvp)
        trait_structure: $ => comma_repeat1(
            $.trait_structure_kvp
        ),

        // TraitStructureKvp =
        //     NodeObjectKey [WS] ":" [WS] NodeValue
        trait_structure_kvp: $ => seq(
            field('key', $.node_object_key),
            ':',
            field('value', $.node_value)
        ),

        // ApplyStatement =
        //     ApplyStatementSingular / ApplyStatementBlock
        // ApplyStatementSingular =
        //     %s"apply" SP ShapeId WS Trait
        // ApplyStatementBlock =
        //     %s"apply" SP ShapeId WS "{" TraitStatements "}"
        apply_statement: $ => seq(
            keyword('apply'),
            field('target', $.shape_id),
            field(
                'traits',
                choice(
                    $.trait,
                    seq(
                        '{',
                        comma_repeat($.trait),
                        '}'
                    )
                )
            )
        ),
    }
});
