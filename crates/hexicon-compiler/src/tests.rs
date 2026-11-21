//! Tests for the compiler.
//!
//! The preprocessor's real test is the comparison with the reference
//! compiler over whole libraries (`tools/pp-compare.sh`). These pin down
//! the behavior that comparison established, on cases small enough to
//! read.

use std::collections::HashMap;

use crate::Tok;
use crate::pp::{Preprocessor, Severity, SourceFiles};

/// Source files held in memory.
#[derive(Default)]
struct Files(HashMap<String, Vec<u8>>);

impl Files {
    fn with(mut self, name: &str, text: &str) -> Self {
        self.0.insert(name.to_owned(), text.as_bytes().to_vec());
        self
    }
}

impl SourceFiles for Files {
    fn read(&mut self, name: &str) -> Option<Vec<u8>> {
        self.0.get(name).cloned()
    }

    fn exists(&mut self, name: &str) -> bool {
        self.0.contains_key(name)
    }

    // A character set of the tests' own: every byte is the letter after
    // it.
    fn decode(&mut self, charset: &str, bytes: &[u8]) -> Option<Vec<u8>> {
        (charset == "shifted").then(|| bytes.iter().map(|&c| c + 1).collect())
    }
}

/// The preprocessed lines of `main.t`, and the messages about it.
fn preprocess_files(mut files: Files) -> (Vec<String>, Vec<String>) {
    let mut pp = Preprocessor::new(&mut files);
    pp.set_test_report_mode(true);
    pp.add_include_path("inc");
    assert!(pp.set_source("main.t"));
    let mut lines = Vec::new();
    while let Some(line) = pp.next_line() {
        lines.push(String::from_utf8_lossy(&line.text).into_owned());
    }
    let messages = pp
        .take_diagnostics()
        .into_iter()
        .map(|problem| {
            let kind = match problem.severity {
                Severity::Error => "error",
                Severity::Fatal => "fatal error",
                Severity::Internal => "internal error",
                Severity::Warning => "warning",
                Severity::Message => "message",
            };
            format!(
                "{}({}): {kind}: {}",
                problem.file, problem.line, problem.message
            )
        })
        .collect();
    (lines, messages)
}

fn preprocess(text: &str) -> Vec<String> {
    let (lines, messages) = preprocess_files(Files::default().with("main.t", text));
    assert_eq!(messages, Vec::<String>::new());
    lines
}

#[test]
fn a_line_of_output_for_each_line_of_source() {
    assert_eq!(
        preprocess("#define N 3\nx = N; // three\ny = N /* still */ + 1;\n"),
        ["", "x = 3; ", "y = 3   + 1;"]
    );
}

#[test]
fn macros_with_arguments() {
    assert_eq!(
        preprocess("#define ADD(a, b) ((a) + (b))\nADD(1, ADD(2, 3))\nADD\n"),
        ["", "((1) + (((2) + (3))))", "ADD "]
    );
}

#[test]
fn a_macro_is_not_expanded_inside_its_own_expansion() {
    assert_eq!(
        preprocess("#define foo foo + bar\n#define bar foo\nfoo\n"),
        ["", "", "foo + foo"]
    );
}

#[test]
fn stringizing_and_pasting() {
    assert_eq!(
        preprocess(
            "#define STR(x) #x\n#define CHR(x) #@x\n#define GLUE(a, b) a##b\n\
             STR(a  \"b\"   c) CHR(its) GLUE(left, Right) GLUE('ab', 'cd')\n"
        ),
        ["", "", "", "\"a \\\"b\\\" c\" 'its' leftRight 'abcd'"]
    );
}

#[test]
fn variable_arguments() {
    let lines = preprocess(
        "#define ALL(first, rest...) first: rest\n\
         #define EACH(rest...) rest#foreach/[rest]/, /\n\
         #define COUNT(rest...) rest#argcount\n\
         #define OPT(rest...) rest#ifempty/none/ rest#ifnempty/some/\n\
         ALL(a, b, c)\nEACH(x, y, z)\nCOUNT() COUNT(p, q)\nOPT() OPT(1)\n",
    );
    assert_eq!(
        &lines[4..],
        ["a: b, c", "[x], [y], [z]", "0 2", "none   some"]
    );
}

#[test]
fn conditions() {
    assert_eq!(
        preprocess(
            "#define V 2\n#if V == 1\none\n#elif V == 2 && defined(V) && !defined W\ntwo\n\
             #else\nother\n#endif\n#ifndef V\nnot\n#endif\n#if 'abc' < 'abd'\nstr\n#endif\n"
        ),
        ["", "", "", "", "two", "", "", "", "", "", "", "", "str", ""]
    );
}

#[test]
fn a_string_that_runs_over_lines_is_joined_up() {
    // The lines of the string become one, and what follows the string
    // on its last line is a line of its own.
    assert_eq!(
        preprocess("say(\"one\n    two\n  three\"); next\nafter\n"),
        ["say(\"one two three\"", "); next", "after"]
    );
}

#[test]
fn macro_arguments_can_run_over_lines() {
    assert_eq!(
        preprocess("#define PAIR(a, b) <a|b>\nPAIR(one,\n     two) tail\nend\n"),
        ["", "<one|two>", " tail", "end"]
    );
}

#[test]
fn a_comment_to_end_of_line_inside_a_macro_call_loses_what_follows() {
    // The reference cuts the line short at the comment but goes on
    // adding the next line after where the comment was, out of sight.
    assert_eq!(
        preprocess("#define PAIR(a, b) <a|b>\nPAIR(one, // note\n     two) tail\n"),
        ["", "<one|>", " tail"]
    );
}

#[test]
fn includes_and_the_line_and_file_macros() {
    let files = Files::default()
        .with(
            "main.t",
            "#include \"local.h\"\n#include <system.h>\n__FILE__ __LINE__ LOCAL SYSTEM\n",
        )
        .with("local.h", "#pragma once\n#define LOCAL 1\n")
        .with("inc/system.h", "#define SYSTEM __FILE__\n");
    let (lines, messages) = preprocess_files(files);
    assert_eq!(messages, Vec::<String>::new());
    assert_eq!(lines, ["", "", "", "", "", "'main.t' 3 1 'main.t'"]);
}

#[test]
fn problems_are_reported_in_the_reference_wording() {
    let (lines, messages) = preprocess_files(Files::default().with(
        "main.t",
        "#define ONE(a) a\nONE(1, 2)\n#include \"missing.h\"\n#bogus\n#define ONE(b) b b\n#endif\n#if 1\n",
    ));
    assert_eq!(lines.len(), 7);
    assert_eq!(
        messages,
        [
            "main.t(2): error: too many arguments for macro \"ONE\"",
            "main.t(3): error: cannot open include file \"missing.h\"",
            "main.t(4): error: invalid preprocessor directive",
            "main.t(5): warning: redefinition of macro \"ONE\"",
            "main.t(6): error: #endif without #if",
            "main.t(8): error: #if without #endif (line 7, file main.t)",
        ]
    );
}

#[test]
fn a_file_says_what_character_set_it_is_in() {
    let mut files = Files::default();
    files.0.insert(
        "main.t".to_owned(),
        b"#charset \"iso-8859-1\"\n'caf\xe9'\n".to_vec(),
    );
    let (lines, messages) = preprocess_files(files);
    assert_eq!(messages, Vec::<String>::new());
    assert_eq!(lines, ["", "'caf\u{e9}'"]);

    // Without one, a file is ASCII, and other bytes are not characters.
    let mut files = Files::default();
    files.0.insert("main.t".to_owned(), b"'caf\xe9'\n".to_vec());
    let (lines, _) = preprocess_files(files);
    assert_eq!(lines, ["'caf\u{fffd}'"]);

    // A set the compiler does not know itself is asked for from outside.
    let (lines, messages) =
        preprocess_files(Files::default().with("main.t", "#charset \"shifted\"\nHAL\n"));
    assert_eq!(messages, Vec::<String>::new());
    assert_eq!(lines, ["\u{b}IBM\u{b}"]);

    // A set that is not known is an error, and the file is read as ASCII.
    let (lines, messages) =
        preprocess_files(Files::default().with("main.t", "#charset \"klingon\"\nx\n"));
    assert_eq!(lines, ["", "x"]);
    assert_eq!(messages, ["main.t(1): error: unable to load #charset"]);
}

// ---- the tokenizer ----

/// The tokens of `main.t`, as kind and text, and the messages about it.
fn tokens(text: &str) -> (Vec<(Tok, String)>, Vec<String>) {
    let mut files = Files::default().with("main.t", text);
    let mut pp = Preprocessor::new(&mut files);
    pp.set_test_report_mode(true);
    pp.set_compile_mode();
    assert!(pp.set_source("main.t"));
    let mut out = Vec::new();
    while pp.next_token() != Tok::Eof {
        let token = pp.cur_token();
        out.push((token.typ, String::from_utf8_lossy(&token.text).into_owned()));
    }
    let messages = pp
        .take_diagnostics()
        .into_iter()
        .map(|problem| format!("{}: {}", problem.line, problem.message))
        .collect();
    (out, messages)
}

fn token_list(text: &str) -> Vec<(Tok, String)> {
    let (tokens, messages) = tokens(text);
    assert_eq!(messages, Vec::<String>::new());
    tokens
}

fn tok(typ: Tok, text: &str) -> (Tok, String) {
    (typ, text.to_owned())
}

#[test]
fn keywords_numbers_and_operators() {
    assert_eq!(
        token_list("local x = nil;\nif (x >>>= 0x1F) return 1.5e3;\n"),
        [
            tok(Tok::Local, "local"),
            tok(Tok::Sym, "x"),
            tok(Tok::Eq, "="),
            tok(Tok::Nil, "nil"),
            tok(Tok::Sem, ";"),
            tok(Tok::If, "if"),
            tok(Tok::LPar, "("),
            tok(Tok::Sym, "x"),
            tok(Tok::LShrEq, ">>>="),
            tok(Tok::Int, "0x1F"),
            tok(Tok::RPar, ")"),
            tok(Tok::Return, "return"),
            tok(Tok::Float, "1.5e3"),
            tok(Tok::Sem, ";"),
        ]
    );
}

#[test]
fn macros_are_expanded_before_tokens_are_read() {
    assert_eq!(
        token_list("#define TWICE(x) x x\nTWICE(if)\n"),
        [tok(Tok::If, "if"), tok(Tok::If, "if")]
    );
}

#[test]
fn a_string_token_has_the_translated_text() {
    assert_eq!(
        token_list("'it\\'s\\n\\u00e9' \"\"\"a \"quoted\" word\"\"\" R'a+'"),
        [
            tok(Tok::SStr, "it's\n\u{e9}"),
            tok(Tok::DStr, "a \"quoted\" word"),
            tok(Tok::ReStr, "a+"),
        ]
    );
}

#[test]
fn a_string_with_embedded_expressions_is_several_tokens() {
    assert_eq!(
        token_list("\"a<<b + 1>>c<<%2d n>>d\""),
        [
            tok(Tok::DStrStart, "a"),
            tok(Tok::Sym, "b"),
            tok(Tok::Plus, "+"),
            tok(Tok::Int, "1"),
            tok(Tok::DStrMid, "c"),
            tok(Tok::FmtSpec, "%2d"),
            tok(Tok::Sym, "n"),
            tok(Tok::DStrEnd, "d"),
        ]
    );
    // A shift inside brackets is a shift, not the end of the expression.
    assert_eq!(
        token_list("'<<(a >> 1)>>'"),
        [
            tok(Tok::SStrStart, ""),
            tok(Tok::LPar, "("),
            tok(Tok::Sym, "a"),
            tok(Tok::AShr, ">>"),
            tok(Tok::Int, "1"),
            tok(Tok::RPar, ")"),
            tok(Tok::SStrEnd, ""),
        ]
    );
}

#[test]
fn tokens_can_be_put_back() {
    let mut files = Files::default().with("main.t", "a b c");
    let mut pp = Preprocessor::new(&mut files);
    pp.set_compile_mode();
    assert!(pp.set_source("main.t"));
    pp.next_token();
    pp.next_token();
    assert_eq!(pp.cur_token().text, b"b");
    pp.unget_token();
    assert_eq!(pp.cur_token().text, b"a");
    assert_eq!(pp.next_token(), Tok::Sym);
    assert_eq!(pp.cur_token().text, b"b");
    pp.next_token();
    assert_eq!(pp.cur_token().text, b"c");
    assert_eq!(pp.next_token(), Tok::Eof);
    assert_eq!(pp.next_token(), Tok::Eof);
}

#[test]
fn tokenizer_problems() {
    let (list, messages) = tokens("a $ b\n#pragma message(\"hello \" 2)\n#error stop\n\"no end\n");
    assert_eq!(
        list,
        [
            tok(Tok::Sym, "a"),
            tok(Tok::Sym, "b"),
            tok(Tok::DStr, "no end ")
        ]
    );
    assert_eq!(
        messages,
        [
            "1: invalid character in input \"$\"",
            "2: hello 2",
            "3: #error :  stop",
            "4: unterminated string literal: string started with \"no end \"",
        ]
    );
}

// ---- compiling ----
//
// The compiler's real test is the comparison of whole images with the
// reference compiler's (`tools/compile-compare.sh`). These pin down
// what that comparison established.

use crate::{CompileOptions, Compiled, compile};

fn compile_text(text: &str) -> Compiled {
    let mut files = Files::default().with("main.t", text);
    compile(&mut files, "main.t", &CompileOptions::default())
}

fn compile_messages(text: &str) -> Vec<String> {
    compile_text(text)
        .diagnostics
        .into_iter()
        .map(|problem| {
            let kind = match problem.severity {
                Severity::Error => "error",
                Severity::Fatal => "fatal error",
                Severity::Internal => "internal error",
                Severity::Warning => "warning",
                Severity::Message => "message",
            };
            format!("{}: {kind}: {}", problem.line, problem.message)
        })
        .collect()
}

/// The data of the image's first block of the given kind.
fn block<'i>(image: &'i [u8], kind: &[u8; 4]) -> &'i [u8] {
    let mut pos = 69;
    loop {
        let size = u32::from_le_bytes(image[pos + 4..pos + 8].try_into().unwrap()) as usize;
        if &image[pos..pos + 4] == kind {
            return &image[pos + 10..pos + 10 + size];
        }
        pos += 10 + size;
    }
}

/// The first page of code: what follows the page's own header.
fn code(image: &[u8]) -> &[u8] {
    &block(image, b"CPPG")[7..]
}

#[test]
fn a_function_is_a_header_its_code_and_its_locals() {
    let compiled = compile_text(
        "_main(args) { local a = 1; local b = a + 2; return b * 3; }\n\
         foo(x, y) { if (x > y) return x; else return y; }\n",
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    let code = code(&image);
    // One parameter, two locals, four stack slots, and the table of
    // locals 23 bytes in.
    assert_eq!(code[..10], [1, 0, 2, 0, 4, 0, 0, 0, 0x17, 0]);
    // onelcl1 0; getlcln0; pushint8 2; add; setlcl1 1; getlcln1;
    // pushint8 3; mul; retval
    assert_eq!(
        code[10..0x17],
        [0xda, 0, 0xaa, 3, 2, 0x22, 0xe0, 1, 0xab, 3, 3, 0x24, 0x50]
    );
    // The table: no line records, one scope, and in it `a`, `b` and
    // `args`, in the order of the reference's hash table. A name longer
    // than two characters is in the constant pool.
    assert_eq!(
        code[0x17..0x47],
        [
            0, 0, 0x2a, 0, 1, 0, 2, 0, // counts and offsets
            0, 0, 3, 0, 0x0a, 0, 0x17, 0, // the scope: three names, bytes 10 to 23
            0, 0, 0, 0, 0, 0, 1, 0, b'a', // local 0
            1, 0, 0, 0, 0, 0, 1, 0, b'b', // local 1
            0, 0, 5, 0, 0, 0, 0, 0, 0, 0, // parameter 0, named in the pool
            0, 0, 0, 0,
        ]
    );
    // The second function: a comparison and a jump become one
    // instruction, and nothing follows a return.
    assert_eq!(
        code[0x47 + 10..0x47 + 0x13],
        [0x7c, 0x7d, 0x99, 4, 0, 0x7c, 0x50, 0x7d, 0x50]
    );
    // The program starts at the first function.
    assert_eq!(block(&image, b"ENTP")[..4], [0, 0, 0, 0]);
}

#[test]
fn the_parts_that_mean_nothing_are_listed() {
    let compiled = compile_text("_main(args) { local a = 1; return a; }\n");
    // The compiler's bytes and the time stamp in the header come first.
    assert_eq!(compiled.unspecified[0], (41, 28));
    // Then the padding after each exported property number, and one
    // field for each local in a function's table.
    assert_eq!(compiled.unspecified.len(), 1 + 21 + 2);
    let image = compiled.image.expect("an image");
    for (offset, len) in &compiled.unspecified[1..] {
        assert!(image[*offset..offset + len].iter().all(|&byte| byte == 0));
    }
}

#[test]
fn a_program_needs_a_main_function() {
    assert_eq!(
        compile_messages("f() { }\n"),
        ["0: error: function _main() is not defined"]
    );
}

#[test]
fn constants_are_worked_out_when_compiling() {
    // `2 + 3 * 4` is 14, and the string is put together.
    let compiled =
        compile_text("_main(args) { if (args) return 2 + 3 * 4; return 'a' + 1 + true; }\n");
    let image = compiled.image.expect("an image");
    let code = code(&image);
    // getargn0; jf; pushint8 14; retval; pushstr; retval
    assert_eq!(code[10..17], [0x7c, 0x93, 5, 0, 3, 14, 0x50]);
    assert_eq!(code[17], 0x05);
    let mut pages = Vec::new();
    let mut pos = 69;
    while pos < image.len() {
        let size = u32::from_le_bytes(image[pos + 4..pos + 8].try_into().unwrap()) as usize;
        if &image[pos..pos + 4] == b"CPPG" {
            pages.push(&image[pos + 10..pos + 10 + size]);
        }
        pos += 10 + size;
    }
    // The last page is the constants, stored under a mask.
    let constants: Vec<u8> = pages.last().unwrap()[7..]
        .iter()
        .map(|byte| byte ^ 0xdf)
        .collect();
    assert_eq!(constants[..8], *b"\x06\x00a1true");
}

#[test]
fn adding_nil_to_a_constant_string_adds_nothing() {
    // The reference reads past the end of an empty string here; what
    // it makes of this depends on what is in its memory.
    let compiled = compile_text("_main(args) { return 'ab' + nil; }\n");
    let image = compiled.image.expect("an image");
    assert!(image.windows(4).any(|window| {
        window.iter().map(|byte| byte ^ 0xdf).collect::<Vec<u8>>() == b"\x02\x00ab"
    }));
}

#[test]
fn problems_are_reported_as_the_reference_reports_them() {
    assert_eq!(
        compile_messages("_main(args) { return undefinedThing; }\n"),
        ["1: error: undefined symbol \"undefinedThing\""]
    );
    assert_eq!(
        compile_messages("f(a) { return a; }\n_main(args) { return f(); }\n"),
        ["2: error: wrong number of arguments to function \"f\": 1 required, 0 actual"]
    );
    assert_eq!(
        compile_messages("_main(args) { break; }\n"),
        ["1: error: invalid \"break\" - not in a for/while/do/switch"]
    );
    assert_eq!(
        compile_messages(
            "_main(args) { if (args
"
        ),
        [
            "2: error: expected ')' after the \"if\" condition, but found \"<End Of File>\"",
            "2: error: unexpected end of file in code block - '}' missing",
        ]
    );
}

#[test]
fn locals_that_are_not_used_get_warnings() {
    assert_eq!(
        compile_messages(
            "_main(args)\n{\n    local a;\n    local b = 1;\n    local c;\n    return c;\n}\n"
        ),
        [
            "3: warning: local variable \"a\" is never used",
            "4: warning: value is assigned to local variable \"b\" but the value is never used",
            "5: warning: no value is assigned to local variable \"c\"",
        ]
    );
}

#[test]
fn a_false_while_condition_is_only_noticed_on_the_first_reading() {
    // The reference warns about this while it reads for symbols, and
    // not when it reads again to generate code; an `if` is the other
    // way around.
    assert_eq!(
        compile_messages("_main(args)\n{\n    while (nil) args = 1;\n    if (nil) args = 2;\n}\n"),
        [
            "3: warning: \"while\" condition is always false - loop body is unreachable",
            "4: warning: \"if\" condition is always false",
        ]
    );
}

/// The image's pages of constants, with the mask taken off.
fn constants(image: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut pos = 69;
    while pos < image.len() {
        let size = u32::from_le_bytes(image[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let data = &image[pos + 10..pos + 10 + size];
        if &image[pos..pos + 4] == b"CPPG" && data[0] == 2 {
            out.extend(data[7..].iter().map(|byte| byte ^ data[6]));
        }
        pos += 10 + size;
    }
    out
}

#[test]
fn built_in_functions_are_called_by_set_and_number() {
    let compiled = compile_text(
        "intrinsic 'first/010000' { a(); b(x, y?); }\n\
         intrinsic 'second/020000' { c(...); }\n\
         _main(args) { b(1); c(1, 2); return a; }\n",
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    // The sets, in the order they were declared.
    assert_eq!(
        block(&image, b"FNSD"),
        b"\x02\x00\x0cfirst/010000\x0dsecond/020000"
    );
    // push1; builtin_a 1, 1; push 2; push1; builtin_b 2, 0;
    // builtin_a 0, 0 (a name alone is a call); the result; retval
    assert_eq!(
        code(&image)[10..23],
        [0x02, 0xb1, 1, 1, 0x03, 2, 0x02, 0xb2, 2, 0, 0xb1, 0, 0]
    );
    assert_eq!(
        compile_messages("intrinsic 'x/1' { b(x, y?); }\n_main(args) { b(); b(1, 2, 3); }\n"),
        [
            "2: error: too few arguments to function \"b\"",
            "2: error: too many arguments to function \"b\"",
        ]
    );
}

#[test]
fn a_constant_list_goes_in_the_constants() {
    let compiled = compile_text("_main(args) { return [1, 'two', [3]]; }\n");
    let image = compiled.image.expect("an image");
    // The list: three elements, each a kind and four bytes. The string
    // and the inner list follow it, and the elements hold their
    // addresses.
    assert_eq!(
        constants(&image)[..31],
        [
            3, 0, // three elements
            7, 1, 0, 0, 0, // the integer 1
            8, 17, 0, 0, 0, // a string, at 17
            10, 22, 0, 0, 0, // a list, at 22
            3, 0, b't', b'w', b'o', // the string
            1, 0, 7, 3, 0, 0, 0, // the inner list
            4, 0, // the next constant: the name `args`
        ]
    );
}

#[test]
fn a_list_with_a_variable_is_made_when_the_program_runs() {
    let compiled = compile_text("_main(args) { return [args, 2]; }\n");
    let image = compiled.image.expect("an image");
    // The elements, last first; new1 2, list; and the result returned
    assert_eq!(code(&image)[10..17], [0x03, 2, 0x7c, 0xc0, 2, 1, 0x54]);
}

#[test]
fn a_folded_comparison_of_lists_is_still_the_list() {
    // The reference overwrites the left operand's value in place, and a
    // list that has had that done to it is still generated as a list.
    let compiled = compile_text("_main(args) { local a = [5] == [5]; return a; }\n");
    let image = compiled.image.expect("an image");
    // pushlst; setlcl1 0
    assert_eq!(code(&image)[10], 0x06);
    assert_eq!(constants(&image)[..7], [1, 0, 7, 5, 0, 0, 0]);
    // But where only the value is asked for, it is true.
    let compiled = compile_text("_main(args) { return [5] == [5]; }\n");
    let image = compiled.image.expect("an image");
    assert_eq!(code(&image)[10], 0x52);
}

#[test]
fn a_string_in_double_quotes_is_displayed_not_used() {
    let compiled = compile_text("_main(args) { \"hello\"; }\n");
    let image = compiled.image.expect("an image");
    // say; retnil
    assert_eq!(code(&image)[10], 0xb0);
    assert_eq!(code(&image)[15], 0x51);
    assert_eq!(
        compile_messages("_main(args) { local a = \"hello\"; return a; }\n"),
        ["1: error: a double-quoted string (\"hello\") is not valid within an expression"]
    );
}

#[test]
fn source_files_are_utf8_unless_they_say_otherwise() {
    let compiled = compile_text("_main(args) { return 'caf\u{e9}'; }\n");
    let image = compiled.image.expect("an image");
    assert_eq!(constants(&image)[..7], *"\u{5}\0caf\u{e9}".as_bytes());
}

#[test]
fn a_switch_is_a_table_of_values_and_jumps() {
    let compiled = compile_text(
        "_main(args) { switch (args) { case 1: return 1; case 'a': return 2; } return 3; }\n",
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    assert_eq!(
        code(&image)[10..38],
        [
            0x7c, // the argument
            0x90, 2, 0, // switch, two cases
            7, 1, 0, 0, 0, 11, 0, // the integer 1: eleven bytes on
            8, 0, 0, 0, 0, 6, 0, // a string: six bytes on
            7, 0, // no match: past the end
            0x02, 0x50, // return 1
            0x03, 2, 0x50, // return 2
            0x03, 3, 0x50, // return 3
        ]
    );
}

#[test]
fn a_case_has_to_be_a_constant_and_nothing_is_generated_if_one_is_not() {
    // The local that is never used would get a warning if the compiler
    // went on to generate code.
    assert_eq!(
        compile_messages("_main(args) { local q; switch (args) { case args: ; } }\n"),
        ["1: error: \"case\" expression has a non-constant value"]
    );
    assert_eq!(
        compile_messages("_main(args) { case 1: ; default: ; }\n"),
        [
            "1: error: misplaced \"case\" keyword - not in a \"switch\" statement",
            "1: error: misplaced \"default\" keyword - not in a \"switch\" statement",
        ]
    );
}

#[test]
fn a_finally_block_is_called_from_every_way_out() {
    let compiled = compile_text(
        "_main(args) { top: while (args) { try { continue top; } finally { --args; } } }\n",
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    let code = code(&image);
    // Two locals, though the source declares none: where the block goes
    // back to, and the exception it holds while it runs.
    assert_eq!(code[2..4], [2, 0]);
    assert_eq!(
        code[10..46],
        [
            0x7c, 0x93, 0x21, 0, // the loop's test
            0x9c, 0x12, 0, // continue: call the block,
            0x91, 0xf8, 0xff, // and go to the top
            0x9c, 0x0c, 0, // the end of the protected code: call the block,
            0x91, 0xf2, 0xff, // and go round again
            0xe0, 1, // an exception arrives: keep it,
            0x9c, 0x04, 0, // call the block,
            0xab, 0xb8, // and throw it again
            0xe0, 0, // the block: keep where to go back to
            0x7c, 0x2f, 0xe2, 0, // --args
            0x9d, 0, 0, // go back
            0x91, 0xdf, 0xff, // the end of the loop's body
            0x51,
        ]
    );
    // The table of handlers: one, for everything in the protected code,
    // whatever is thrown.
    let table = usize::from(u16::from_le_bytes([code[6], code[7]]));
    assert_eq!(
        code[table..table + 12],
        [1, 0, 14, 0, 25, 0, 0, 0, 0, 0, 26, 0]
    );
}

#[test]
fn labels_and_the_jumps_that_name_them() {
    assert_eq!(
        compile_messages(
            "_main(args)\n{\na: args = 1;\na: args = 2;\nb: ;\n\
             goto c;\nbreak d;\nwhile (args) continue e;\n}\n"
        ),
        ["4: error: code label \"a\" already defined in this function or method"]
    );
    assert_eq!(
        compile_messages(
            "_main(args)\n{\nused: args = 1;\nunused: args = 2;\n\
             if (args) goto nowhere;\nif (args) break used;\n\
             while (args) continue used;\ngoto used;\n}\n"
        ),
        [
            "5: error: invalid \"goto\" - label \"nowhere\" is not defined in this function or method",
            "6: error: invalid \"break\" - no label \"used\" on any enclosing statement",
            "7: error: invalid \"continue\" - no label \"used\" on any enclosing loop",
            "4: warning: label \"unused\" is not referenced",
        ]
    );
    assert_eq!(
        compile_messages("_main(args) { try { args = 1; } finally { in: args = 2; } goto in; }\n"),
        ["1: error: 'goto' cannot transfer control into a 'finally' block"]
    );
}

#[test]
fn an_expression_embedded_in_a_displayed_string_is_displayed() {
    let compiled = compile_text("_main(args) { \"a<<args>>b<<>>\"; }\n");
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    let code = code(&image);
    // say "a"; the argument; sayval; say "b"
    assert_eq!(code[10], 0xb0);
    assert_eq!(code[15..17], [0x7c, 0xb9]);
    assert_eq!(code[17], 0xb0);
    assert_eq!(code[22], 0x51);
}

#[test]
fn an_expression_embedded_in_a_string_value_is_added_to_it() {
    let compiled = compile_text("_main(args) { return 'a<<args>>b'; }\n");
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    let code = code(&image);
    // pushstr "a"; the argument; add; pushstr "b"; add; retval
    assert_eq!(code[10], 0x05);
    assert_eq!(code[15..17], [0x7c, 0x22]);
    assert_eq!(code[17], 0x05);
    assert_eq!(code[22..24], [0x22, 0x50]);

    // With nothing but constants embedded, the string is a constant,
    // which a `case` has to be.
    assert_eq!(
        compile_messages(
            "_main(args) { switch (args) { case 'a<<1>>b<<if nil>>c<<else>>d<<end>>': ; } }\n"
        ),
        [] as [&str; 0]
    );
}

#[test]
fn what_is_embedded_can_depend_on_a_condition() {
    let compiled = compile_text("_main(args) { \"<<if args>>yes<<else>>no<<end>>\"; }\n");
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    let code = code(&image);
    // the argument; jf; say "yes"; return, where a jump to the end
    // was; say "no"; return
    assert_eq!(code[10..12], [0x7c, 0x93]);
    assert_eq!(code[14], 0xb0);
    assert_eq!(code[19], 0x51);
    assert_eq!(code[22], 0xb0);
    assert_eq!(code[27], 0x51);

    assert_eq!(
        compile_messages("_main(args) { \"a<<else>>b\"; }\n"),
        ["1: error: <<else>> found in string without a matching <<if>> or <<unless>>"]
    );
    assert_eq!(
        compile_messages("_main(args) { \"a<<args args>>b\"; }\n"),
        ["1: error: expected '>>' and the string continuation, but found \"args\""]
    );
    // Choosing among alternatives needs a class, and so objects.
    assert_eq!(
        compile_messages("_main(args) { \"<<one of>>a<<or>>b<<stopping>>\"; }\n"),
        ["1: error: <<one of>> requires class OneOfIndexGen to be defined in the program"]
    );
}

#[test]
fn a_string_template_makes_an_embedded_phrase_a_call() {
    let compiled = compile_text(
        "string template <<* twice>> twice;\n\
         twice(x) { return x + x; }\n\
         _main(args) { \"<<args twice>>\"; }\n",
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    // In _main: the argument; call twice with one argument; the result;
    // sayval
    let code = code(&image);
    let call = code
        .windows(3)
        .position(|bytes| bytes == [0x7c, 0x58, 1])
        .expect("a call with the argument");
    assert_eq!(code[call + 7..call + 9], [0x8b, 0xb9]);

    // The function has to be defined somewhere if it is used,
    assert_eq!(
        compile_messages("string template <<* twice>> twice;\n_main(args) { \"<<1 twice>>\"; }\n"),
        ["0: error: unresolved external reference \"twice\""]
    );
    // but not if it is not.
    assert_eq!(
        compile_messages("string template <<* twice>> twice;\n_main(args) { }\n"),
        [] as [&str; 0]
    );
    assert_eq!(
        compile_messages("f() { }\nstring template <<a * b * c>> f;\n_main(args) { }\n"),
        [
            "2: error: only one '*' is allowed in a string template",
            "2: error: string template processor function f() does not match template - \
             must take 1 parameter(s) and return a value",
        ]
    );
}

#[test]
fn the_compiler_gives_up_after_a_hundred_errors() {
    let mut source = String::from("_main(args) {\n");
    for _ in 0..150 {
        source.push_str("1 2;\n");
    }
    source.push_str("}\n");
    let messages = compile_messages(&source);
    assert_eq!(messages.len(), 102);
    assert_eq!(messages[100], "102: error: expected ';', but found \"2\"");
    assert_eq!(
        messages[101],
        "102: fatal error: too many errors (try fixing the first couple of errors and recompiling)"
    );

    // The same when the errors are found generating the code.
    let mut source = String::from("_main(args) {\n");
    for index in 0..150 {
        source.push_str(&format!("u{index}();\n"));
    }
    source.push_str("}\n");
    let messages = compile_messages(&source);
    assert_eq!(messages.len(), 102);
    assert_eq!(messages[100], "102: error: undefined symbol \"u100\"");
    assert!(messages[101].starts_with("102: fatal error: too many errors"));
}

#[test]
fn an_undefined_name_is_reported_once() {
    assert_eq!(
        compile_messages("f() { nosuch(); }\n_main(args) { nosuch; other = nosuch; }\n"),
        [
            "1: error: undefined symbol \"nosuch\"",
            "2: error: undefined symbol \"other\"",
        ]
    );
}

#[test]
fn a_long_string_in_a_message_is_cut_short() {
    assert_eq!(
        compile_messages("_main(args) { 1 'a string that goes on\\nand on'; }\n"),
        ["1: error: expected ';', but found \"'a string that goes o...'\""]
    );
    assert_eq!(
        compile_messages("_main(args) { 1 'two\\nlines'; }\n"),
        ["1: error: expected ';', but found \"'two\\nlines'\""]
    );
}

#[test]
fn an_object_is_its_superclasses_and_its_properties_in_order_of_number() {
    let compiled = compile_text(
        "thing: object\n    name = 'thing'\n    weight = 3\n;\n\
         _main(args) { return thing.name; }\n",
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    // One object, of the first metaclass: number 1, 27 bytes, no
    // superclasses, three properties, not a class. The properties are
    // its place in the source (property 7), and then the two the
    // source gives it, numbered in the order of the table of names:
    // weight before name.
    let objs = block(&image, b"OBJS");
    assert_eq!(objs[..6], [1, 0, 0, 0, 0, 0]);
    assert_eq!(objs[6..12], [1, 0, 0, 0, 27, 0]);
    assert_eq!(objs[12..18], [0, 0, 3, 0, 0, 0]);
    assert_eq!(objs[18..25], [7, 0, 7, 1, 0, 0, 0]);
    assert_eq!(objs[25..32], [26, 0, 7, 3, 0, 0, 0]);
    assert_eq!(objs[32..35], [27, 0, 8]);
    // objgetprop object 1, property 27; return the result
    assert_eq!(code(&image)[10..18], [0x66, 1, 0, 0, 0, 27, 0, 0x54]);
}

#[test]
fn a_method_has_a_self_and_a_function_does_not() {
    let compiled = compile_text(
        "a: object\n    size = 1\n    grow(by) { size += by; return self.size; }\n;\n\
         _main(args) { return a.grow(2); }\n",
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    let code = code(&image);
    // getpropself; the result; the argument; add; setpropself;
    // getpropself; return the result
    assert_eq!(code[10], 0x63);
    assert_eq!(code[13..17], [0x8b, 0x7c, 0x22, 0xe7]);
    assert_eq!(code[19], 0x63);
    assert_eq!(code[22], 0x54);

    assert_eq!(
        compile_messages(
            "a: object size = 1;\n_main(args) { size = 2; return self.size + size; }\n"
        ),
        [
            "2: error: cannot assign to property \"size\" without \"object.\" prefix - no \"self\"",
            "2: error: 'self' is not valid in this context",
            "2: error: cannot call property \"size\" without \"object.\" prefix - no \"self\"",
        ]
    );
}

#[test]
fn a_name_nothing_defines_is_taken_for_a_property_where_one_makes_sense() {
    assert_eq!(
        compile_messages(
            "a: object m() { return size; }\n;\n_main(args) { return a.weight + other; }\n"
        ),
        [
            "1: warning: undefined symbol \"size\" - assuming this is a property name",
            "3: warning: undefined symbol \"weight\" - assuming this is a property name",
            "3: error: undefined symbol \"other\"",
        ]
    );
}

#[test]
fn object_definitions_that_are_wrong() {
    assert_eq!(
        compile_messages(
            "a: object x = 1 x = 2;\na: object;\nf() { }\nf: object;\nb: b;\nc object;\n\
             d: object x 1;\n_main(args) { }\n"
        ),
        [
            "1: error: property \"x\" already defined in object",
            "2: error: object \"a\" is already defined - can't redefine as object",
            "4: error: symbol \"f\" is already defined - can't redefine as object",
            "5: error: circular class definition: b is a subclass of b",
            // `c` is taken for the class of an object with no name.
            "6: error: missing semicolon at end of object definition - found \"object\"",
            "7: error: '=' required between property name and value - found \"1\"",
        ]
    );
    // A definition whose end was left out, and the next one starting.
    assert_eq!(
        compile_messages("a: object;\nb: object x = 1\na: object y = 2;\n_main(args) { }\n"),
        [
            "3: error: unterminated object definition",
            "3: error: object \"a\" is already defined - can't redefine as object",
        ]
    );
    assert_eq!(
        compile_messages("a: Nosuch;\nf() { }\nb: f;\n_main(args) { }\n"),
        [
            "1: error: undefined symbol \"Nosuch\" (used as superclass of \"a\")",
            "3: error: invalid superclass \"f\" - not an object",
        ]
    );
}

#[test]
fn a_constructor_returns_the_object_and_several_superclasses_get_one() {
    let compiled = compile_text("class A: object;\nclass B: object;\nc: A, B;\n_main(args) { }\n");
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    let code = code(&image);
    // It takes any number of arguments, and has one local, for them
    // as a list.
    assert_eq!(code[..4], [0x80, 0, 1, 0]);
    assert_eq!(
        code[10..36],
        [
            0x0d, 0, 0xe0, 0, // the arguments, into the local
            0x01, 0xaa, 0x0e, 0x76, // none more, and those: as arguments
            0x74, 0, 1, 0, 1, 0, 0, 0, // A's constructor
            0x01, 0xaa, 0x0e, 0x76, //
            0x74, 0, 1, 0, 2, 0, // B's
        ]
    );
    assert_eq!(code[38..40], [0x84, 0x50]);

    assert_eq!(
        compile_messages("a: object construct() { return 1; }\n;\n_main(args) { }\n"),
        ["1: error: \"construct\" cannot return a value"]
    );
}

#[test]
fn a_catch_names_a_class_of_exception() {
    let compiled = compile_text(
        "class Oops: object;\n_main(args) { try { throw args; } catch (Oops e) { return e; } }\n",
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    let code = code(&image);
    let table = usize::from(u16::from_le_bytes([code[6], code[7]]));
    // One handler, for the class, which is object 1.
    assert_eq!(code[table..table + 2], [1, 0]);
    assert_eq!(code[table + 6..table + 10], [1, 0, 0, 0]);
    assert_eq!(
        compile_messages("f() { }\n_main(args) { try { } catch (f e) { } catch (nosuch e) { } }\n"),
        [
            "2: error: exception class symbol \"f\" in \"catch\" clause is not an object",
            "2: error: undefined symbol \"nosuch\"",
        ]
    );
}

#[test]
fn a_jump_to_a_label_in_code_that_was_left_out_cannot_be_compiled() {
    assert_eq!(
        compile_messages("_main(args) { if (true) args = 1; else { a: args = 2; } goto a; }\n"),
        [
            "1: warning: \"if\" condition is always true - \"else\" part is unreachable",
            "0: internal error: 1 unresolved branches",
        ]
    );
}

#[test]
fn new_makes_an_object_of_a_class() {
    let compiled = compile_text("class A: object;\n_main(args) { return new A(args, 2); }\n");
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    // 2; the argument; the class; new, with three things on the stack,
    // of the first metaclass; return the result
    assert_eq!(
        code(&image)[10..22],
        [0x03, 2, 0x7c, 0x07, 1, 0, 0, 0, 0xc0, 3, 0, 0x54]
    );
    assert_eq!(
        compile_messages("f() { }\n_main(args) { new f(); new args; new nosuch; }\n"),
        [
            "2: error: cannot apply operator 'new' to this expression",
            "2: error: cannot apply operator 'new' to this expression",
            "2: error: undefined symbol \"nosuch\"",
        ]
    );
}

#[test]
fn inherited_calls_what_a_method_overrides() {
    let compiled = compile_text(
        "class A: object m(x) { return x; };\n\
         class B: A m(x) { return inherited(x) + inherited A.m(x); };\n\
         _main(args) { }\n",
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    let code = code(&image);
    // In B.m: the argument; which property this is; ptrinherit with
    // one argument; the result
    let start = code
        .windows(4)
        .position(|bytes| bytes == [0x7c, 0x8e, 1, 0x73])
        .expect("inherited through the property being run");
    assert_eq!(code[start + 4..start + 6], [1, 0x8b]);
    // and then the argument; expinherit, one argument, of class A
    assert_eq!(code[start + 6..start + 9], [0x7c, 0x74, 1]);
    assert_eq!(code[start + 11..start + 15], [1, 0, 0, 0]);

    assert_eq!(
        compile_messages("a: object;\n_main(args) { inherited.x; inherited a.x; targetprop; }\n"),
        [
            "2: error: 'self' is not valid in this context",
            "2: warning: undefined symbol \"x\" - assuming this is a property name",
            "2: error: 'self' is not valid in this context",
            "2: error: 'targetprop' is not available in this context",
        ]
    );
}

#[test]
fn the_address_of_a_property_is_a_constant() {
    let compiled = compile_text(
        "a: object x = 1 y = &x;\n_main(args) { switch (args) { case &x: return a.(&y); } return &x; }\n",
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    let code = code(&image);
    // The switch has one case, a property.
    assert_eq!(code[10..15], [0x7c, 0x90, 1, 0, 6]);
    // A name whose address is taken becomes a property unless a
    // function claims it afterward.
    assert_eq!(
        compile_messages("_main(args) { return [&later, &never]; }\nlater() { }\n"),
        [] as [&str; 0]
    );
    assert_eq!(
        compile_messages("_main(args) { local q; return [&q, &1]; }\n"),
        ["1: error: invalid address expression - can't apply '&' operator",]
    );
}

#[test]
fn a_built_in_class_is_declared_with_its_properties() {
    let compiled = compile_text(
        "intrinsic class Object 'root-object/030004' { ofKind(cls); }\n\
         intrinsic class List 'list/030008': Object { length(); }\n\
         _main(args) { return [args.length(), new List()]; }\n",
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    // Ten classes: the eight every image has, with the list's version
    // now the one declared, then the one declared that is new, and
    // the class of the objects that stand for classes.
    let mcld = block(&image, b"MCLD");
    assert_eq!(mcld[..2], [10, 0]);
    let text = String::from_utf8_lossy(mcld);
    let list = text.find("list/030008").expect("the list class");
    let root = text.find("root-object/030004").expect("the root class");
    let last = text
        .find("intrinsic-class/000000")
        .expect("the class of classes");
    assert!(list < root && root < last);
    // The list class has one property.
    assert_eq!(mcld[list + 11..list + 15], [1, 0, 2, 0]);
    // new, with no arguments, of the second class in the table
    let code = code(&image);
    assert!(code.windows(3).any(|bytes| bytes == [0xc0, 0, 1]));

    assert_eq!(
        compile_messages(
            "f() { }\nintrinsic class f 'f/030000' { }\n\
             intrinsic class A 'a/030000': Nosuch { 1; }\n_main(args) { return A(); }\n"
        ),
        [
            "2: error: symbol \"f\" is already defined - can't redefine as intrinsic class",
            "3: error: intrinsic class superclass \"Nosuch\" is not defined",
            "3: error: expected property name in intrinsic class list, but found \"1\"",
        ]
    );
}

#[test]
fn exports_come_first_among_the_names_the_image_gives() {
    let compiled = compile_text(
        "property alpha;\nexport alpha 'Alpha.name';\nexport f;\nf() { }\n_main(args) { }\n",
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    let symd = block(&image, b"SYMD");
    // Two more than the 21 every image has: a property, and a
    // function.
    assert_eq!(symd[..2], [23, 0]);
    assert_eq!(symd[2..5], [6, 26, 0]);
    assert_eq!(&symd[7..18], b"\x0aAlpha.name");
    assert_eq!(symd[18], 12);
    assert_eq!(&symd[23..25], b"\x01f");

    assert_eq!(
        compile_messages(
            "export a 'LastProp';\nexport b 'same';\nexport c 'same';\n_main(args) { }\n"
        ),
        [
            "0: error: external name \"LastProp\" illegal in 'export' - reserved for compiler use",
            "0: warning: undefined symbol \"a\" - assuming this is a property name",
            "0: warning: undefined symbol \"b\" - assuming this is a property name",
            "0: error: duplicate export for external name \"same\": \"b\" and \"c\"",
        ]
    );
}

const COLLECTION: &str = "intrinsic class Object 'root-object/030004' { ofKind(cls); }\n\
     intrinsic class Collection 'collection/030000': Object { createIterator(); }\n";

#[test]
fn a_loop_over_a_collection_asks_it_for_an_iterator() {
    let source = format!("{COLLECTION}_main(args) {{ foreach (local x in args) args = x; }}\n");
    let compiled = compile_text(&source);
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    let code = code(&image);
    // Two locals: the variable, and one for the iterator.
    assert_eq!(code[2..4], [2, 0]);
    // The argument; its createIterator; the result, into local 1;
    // iternext from local 1, or leave; into local 0
    assert_eq!(code[10..12], [0x7c, 0x60]);
    assert_eq!(code[14..16], [0xee, 1]);
    assert_eq!(code[16..19], [0xa2, 1, 0]);
    assert_eq!(code[21..23], [0xe0, 0]);

    // Without the class that says what makes an iterator, there is no
    // such loop.
    assert_eq!(
        compile_messages("_main(args) { foreach (local x in args) ; for (local y in args) ; }\n"),
        [
            "1: error: Container.createIterator is not defined - foreach..in is not valid",
            "1: error: Container.createIterator is not defined - for..in is not valid",
            "1: warning: value is assigned to local variable \"x\" but the value is never used",
            "1: warning: value is assigned to local variable \"y\" but the value is never used",
        ]
    );
}

#[test]
fn a_loop_over_a_range_counts() {
    let compiled = compile_text("_main(args) { for (local i in 1..args step -2) args = i; }\n");
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    let code = code(&image);
    // Two locals: the variable, and one for the end of the range,
    // which is not a constant.
    assert_eq!(code[2..4], [2, 0]);
    // i = 1; the end, into local 1; and at the top the two are
    // compared, to leave if i is the less, since the step is negative.
    assert_eq!(code[10..12], [0xda, 0]);
    assert!(code.windows(3).any(|bytes| bytes == [0x7c, 0xe0, 1]));
    assert!(code[12..40].contains(&0x98));
}

#[test]
fn a_test_for_being_among_some_values_compares_with_each() {
    let compiled = compile_text("_main(args) { return args is in (1, 2); }\n");
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    // the argument; dup; 1; je; dup; 2; je; discard; return nil,
    // where the jump past the rest was; discard; return true
    assert_eq!(
        code(&image)[10..30],
        [
            0x7c, 0x88, 0x02, 0x94, 0x0d, 0, 0x88, 0x03, 2, 0x94, 0x07, 0, 0x89, 0x08, 0x50, 0xf2,
            0xf2, 0x89, 0x52, 0x50,
        ]
    );
    assert_eq!(
        compile_messages("_main(args) { args is in 1; args not in (1 2); }\n"),
        [
            "1: error: missing '(' after 'in' operator - found \"1\"",
            "1: error: expected ')' at end of 'in' list, but found \";\"",
            "1: error: expected ',' or ')' in 'in' list, but found \"2\"",
        ]
    );
}

#[test]
fn a_source_that_ends_in_a_for_header_stops_at_the_limit_on_errors() {
    let messages = compile_messages("_main(a) { for (a x\n");
    assert_eq!(messages.len(), 102);
    assert!(messages[101].contains("fatal error: too many errors"));
}

/// Whether some bytes come one after another somewhere in the code.
fn has_bytes(code: &[u8], bytes: &[u8]) -> bool {
    code.windows(bytes.len()).any(|window| window == bytes)
}

#[test]
fn an_anonymous_function_that_shares_nothing_is_just_its_code() {
    let compiled = compile_text("_main(args) { local f = {x: x + 1}; return f(2); }\n");
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    let code = code(&image);
    // One local, and nothing made when the function is: the address of
    // its code goes straight into the local.
    assert_eq!(code[2..4], [1, 0]);
    assert_eq!(code[10], 0x0b);
    assert_eq!(code[15..17], [0xe0, 0]);
    assert!(!has_bytes(code, &[0xc0, 2, 4]));
    // The function's own code comes after: one parameter, and it
    // returns the parameter and one.
    assert!(has_bytes(code, &[0x7c, 0x2e, 0x50]));
}

#[test]
fn a_local_an_anonymous_function_uses_lives_in_a_vector() {
    let compiled =
        compile_text("_main(args) { local n = 5; local f = {x: x + n}; n = 6; return f(1); }\n");
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    let code = code(&image);
    // Three locals: n's old place, f, and one for the vector, which is
    // made first, with room for the method's `self` and for n.
    assert_eq!(code[2..4], [3, 0]);
    assert_eq!(code[10..18], [0x03, 2, 0x88, 0xc0, 2, 4, 0xee, 2]);
    // n = 5 stores into the vector's second place.
    assert_eq!(code[18..23], [0x03, 5, 0xef, 2, 2]);
    // The function is made of the vector and its code.
    assert!(has_bytes(code, &[0xc0, 2, 5, 0xee, 1]));
    // And inside it the vector is fetched from the function object,
    // and n from the vector.
    assert!(has_bytes(code, &[0x8e, 4, 0xbc, 2, 0xe0, 0]));
    assert!(has_bytes(code, &[0xbb, 0, 2]));
}

#[test]
fn an_anonymous_function_in_a_method_takes_the_methods_self() {
    let compiled = compile_text(
        "a: object v = 1 m() { local f = {: v}; return f(); } ;\n_main(args) { return a.m(); }\n",
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    let code = code(&image);
    // The method puts `self` in the first place of its vector; the
    // function takes it from there and makes it its own.
    assert!(has_bytes(code, &[0x84, 0xef, 1, 1]));
    assert!(has_bytes(code, &[0x88, 0xbc, 1, 0xeb]));
}

#[test]
fn a_short_anonymous_function_has_its_own_mistakes() {
    assert_eq!(
        compile_messages(
            "_main(args)\n{\n    local a = {x: x; };\n    local b = {x x };\n    \
             local c = function(x) multimethod { return x; };\n    return {: 1 };\n}\n"
        ),
        [
            "3: error: semicolon is not allowed in a short anonymous function",
            "4: error: expected a comma in formal parameter list, but found \"x\"",
            "4: error: formal parameter \"x\" defined more than once",
            "4: error: missing colon at end of anonymous function formal list - found \"}\"",
            "4: error: expected an operand, but found \"}\"",
            "5: error: 'multimethod' is not allowed here",
            "6: error: expected ';' or expression after \"return\", but found \"{\"",
            "6: error: expected integer, string, symbol, '[', or '(', but found \":\"",
            "6: error: expected ';', but found \"}\"",
        ]
    );
    // The source ending inside one: the reference's message is not the
    // one it means.
    assert_eq!(
        compile_messages("_main(args)\n{\n    local f = {x: x\n"),
        [
            "4: error: semicolon is not allowed in a short-form anonymous function",
            "4: error: expected ',' or ';' after local variable, but found \"<End Of File>\"",
            "4: error: unexpected end of file in code block - '}' missing",
        ]
    );
}

/// The declarations of the built-in functions that optional and named
/// parameters are filled in with.
const PARAM_SUPPORT: &str = "intrinsic 't3vm/010006' { t3RunGC(); t3SetSay(val); t3GetVMVsn(); \
    t3GetVMID(); t3GetVMBanner(); t3GetVMPreinitMode(); t3DebugTrace(mode, ...); \
    t3GetGlobalSymbols(which?); t3AllocProp(); t3GetStackTrace(level?, flags?); \
    t3GetNamedArg(name, defval?); t3GetNamedArgList(); }\n\
    intrinsic 'tads-gen/030008' { dataType(val); getArg(idx); }\n";

#[test]
fn an_optional_parameter_is_a_local_filled_in_at_the_start() {
    let source =
        format!("{PARAM_SUPPORT}f(a, b = a + 2) {{ return b; }}\n_main(args) {{ return f(1); }}\n");
    let compiled = compile_text(&source);
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    let code = code(&image);
    // One parameter that has to be there, one more that can be, and a
    // local for it.
    assert_eq!(code[0..4], [1, 1, 1, 0]);
    // If there is a second argument, it is fetched with getArg(2);
    // otherwise the default value is worked out. Either is stored.
    assert_eq!(
        code[10..31],
        [
            0x03, 2, 0x87, 0x96, 11, 0, // if 2 > the argument count, go on
            0x03, 2, 0xb2, 1, 1, 0x8b, // getArg(2)
            0x91, 6, 0, // and skip
            0x7c, 0x03, 2, 0x22, // a + 2
            0xe0, 0, // b = whichever
        ]
    );
}

#[test]
fn a_named_parameter_is_asked_for_by_name() {
    let source = format!(
        "{PARAM_SUPPORT}f(n:, m: = 3) {{ return n + m; }}\n\
         _main(args) {{ return f(m: 1, n: 2); }}\n"
    );
    let compiled = compile_text(&source);
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    let code = code(&image);
    // No parameters as the machine counts them, and two locals.
    assert_eq!(code[0..4], [0, 0, 2, 0]);
    // t3GetNamedArg('n'), and t3GetNamedArg('m', 3).
    assert_eq!(code[10], 0x05);
    assert_eq!(code[15..20], [0xb1, 1, 10, 0xee, 0]);
    assert_eq!(code[20..23], [0x03, 3, 0x05]);
    assert_eq!(code[27..32], [0xb1, 2, 10, 0xee, 1]);
    // The call passes no arguments by position. After it comes where
    // the names are, which are after the function's code: how many
    // bytes, how many names, where each starts and the last ends, and
    // the names, last argument first.
    assert!(has_bytes(code, &[0x03, 2, 0x02, 0x58, 0]));
    assert!(has_bytes(code, &[0x56, 2, 3, 0]));
    assert!(has_bytes(
        code,
        &[0x57, 10, 0, 2, 0, 6, 0, 7, 0, 8, 0, b'n', b'm']
    ));
}

#[test]
fn optional_and_named_parameters_have_their_own_mistakes() {
    let source = format!(
        "{PARAM_SUPPORT}f(a?, b) {{ }}\ng(a?) {{ }}\n\
         _main(args)\n{{\n    g(1, 2);\n    g(a: [1]...);\n}}\n"
    );
    assert_eq!(
        compile_messages(&source),
        [
            "3: error: parameter 'b' must be optional because it follows an optional parameter",
            "8: error: a pass-by-name argument value cannot use an ellipsis '...'",
        ]
    );
    // Without the functions that fill them in, they cannot be used. No
    // statement has been come to, so the message has no place.
    assert_eq!(
        compile_messages("f(a?) { }\n_main(args) { }\n"),
        ["0: error: optional parameters require support function \"getArg\", which is undefined"]
    );
}

#[test]
fn enum_values_are_numbered_in_the_order_of_the_table_of_names() {
    let compiled = compile_text(
        "enum red, green, blue;\n\
         _main(args) { local a = red == red; return [blue, green, red, a]; }\n",
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    let code = code(&image);
    // Compared while compiling, a value of an enumeration is not equal
    // even to itself, as the reference has it: the local is set to nil.
    assert_eq!(code[10..12], [0xd8, 0]);
    // The list's elements are pushed last first. The numbers go by the
    // sums of the names' bytes: green, red, blue.
    assert_eq!(
        code[13..28],
        [0x0f, 2, 0, 0, 0, 0x0f, 1, 0, 0, 0, 0x0f, 3, 0, 0, 0]
    );
    assert_eq!(
        compile_messages("f() { }\nenum f, ;\n_main(args) { }\n"),
        [
            "2: error: symbol \"f\" is already defined - can't be used as an enum name",
            "2: error: symbol expected in 'enum' list - found \";\"",
        ]
    );
}

#[test]
fn a_static_property_is_code_that_runs_before_the_program() {
    let compiled = compile_text(
        "f() { return 7; }\na: object p = static f() q = static 5;\n\
         _main(args) { return a.p; }\n",
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    // One property to work out, of object 1: the constant is a constant
    // like any other. The code for it starts where the code that is
    // only needed before the program starts is kept, on a page of its
    // own.
    let sini = block(&image, b"SINI");
    assert_eq!(sini[4..12], [0, 8, 0, 0, 1, 0, 0, 0]);
    assert_eq!(sini[12..16], [1, 0, 0, 0]);
    let prop = &sini[16..18];
    // It calls f, sets the property to what comes back, and returns
    // that too.
    assert!(has_bytes(
        &image,
        &[0x58, 0, 0, 0, 0, 0, 0xa5, 0xe7, prop[0], prop[1], 0x50]
    ));
}

/// The data of each of the image's blocks of the given kind.
fn blocks<'i>(image: &'i [u8], kind: &[u8; 4]) -> Vec<&'i [u8]> {
    let mut found = Vec::new();
    let mut pos = 69;
    while pos + 10 <= image.len() {
        let size = u32::from_le_bytes(image[pos + 4..pos + 8].try_into().unwrap()) as usize;
        if &image[pos..pos + 4] == kind {
            found.push(&image[pos + 10..pos + 10 + size]);
        }
        pos += 10 + size;
    }
    found
}

#[test]
fn modify_puts_a_function_in_front_of_the_one_there_was() {
    let compiled = compile_text(
        "f(x) { return x + 1; }\nmodify f(x) { return replaced(x) * 2; }\n\
         g() { return 77; }\nreplace g() { return 88; }\n\
         _main(args) { return f(1) + g(); }\n",
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    let code = code(&image);
    // The first f is still at the start of the code, and the second
    // calls it there.
    assert_eq!(code[10..13], [0x7c, 0x2e, 0x50]);
    assert!(has_bytes(code, &[0x7c, 0x58, 1, 0, 0, 0, 0]));
    // Nothing is generated for the g that was replaced: no function
    // returns 77.
    assert!(has_bytes(code, &[0x03, 88, 0x50]));
    assert!(!has_bytes(code, &[0x03, 77, 0x50]));
}

#[test]
fn modify_leaves_the_object_there_was_as_a_class_behind_the_new_one() {
    let compiled = compile_text("a: object v = 1;\nmodify a v = 2;\n_main(args) { return a.v; }\n");
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    let objs = block(&image, b"OBJS");
    // Two objects. The first definition has a number of its own, just
    // before the number the name has, and is a class with two
    // properties: v, and where it came in the source.
    assert_eq!(objs[0..2], [2, 0]);
    assert_eq!(objs[6..10], [1, 0, 0, 0]);
    assert_eq!(objs[12..18], [0, 0, 2, 0, 1, 0]);
    // The second has the first for its superclass, and one property.
    let second = &objs[6 + 12 + 14..];
    assert_eq!(second[0..4], [2, 0, 0, 0]);
    assert_eq!(second[6..16], [1, 0, 1, 0, 0, 0, 1, 0, 0, 0]);
}

#[test]
fn modify_of_a_built_in_class_makes_objects_for_what_is_added() {
    let compiled = compile_text(
        "intrinsic class Object 'root-object/030004' { ofKind(cls); }\n\
         modify Object hello = 7;\n_main(args) { }\n",
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    let all = blocks(&image, b"OBJS");
    // A block of two objects of the kind that adds to a built-in
    // class: the one with the property, whose superclass is an empty
    // one that comes after it.
    let added = all
        .iter()
        .find(|objs| objs[2..4] == [6, 0])
        .expect("a block of additions");
    assert_eq!(added[0..2], [2, 0]);
    assert_eq!(added[6..10], [3, 0, 0, 0]);
    assert_eq!(added[12..16], [1, 0, 1, 0]);
    assert_eq!(added[18..22], [2, 0, 0, 0]);
    // And the object that stands for the class names the first of
    // them.
    let class = all.last().expect("the classes");
    assert_eq!(class[6..10], [1, 0, 0, 0]);
    assert_eq!(class[16..20], [3, 0, 0, 0]);
}

#[test]
fn replace_and_modify_have_their_own_mistakes() {
    assert_eq!(
        compile_messages("replace a: object;\n_main(args) { }\n"),
        ["1: error: replace/modify cannot be used with an object not previously defined"]
    );
    assert_eq!(
        compile_messages("f() { return replaced(); }\n_main(args) { }\n"),
        ["1: error: 'replaced' is not available in this context"]
    );
    // This one is found on the first reading, which ends the matter.
    assert_eq!(
        compile_messages("b: object replace x = 1;\n_main(args) { }\n"),
        ["1: error: \"replace\" can be used with a property only in a \"modify\" object"]
    );
    // Used on a function before its definition, they are found out
    // when the program is put together.
    assert_eq!(
        compile_messages("modify f() { }\n_main(args) { }\n"),
        [
            "0: error: modified or replaced function \"f\" found in object file \"main.t3o\" \
             before original function definition was loaded"
        ]
    );
}

#[test]
fn a_function_can_be_modified_more_than_once() {
    // The reference cannot put such a program together; here each
    // definition reaches the one before.
    let compiled = compile_text(
        "g(x) { return x * 2; }\nmodify g(x) { return replaced(x) + 1; }\n\
         modify g(x) { return replaced(x) * 10; }\n_main(args) { return g(3); }\n",
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    let code = code(&image);
    // Two calls with one argument to code with no name: the first
    // definition, at the start of the code, and the second after it.
    assert!(has_bytes(code, &[0x58, 1, 0, 0, 0, 0]));
    let calls = code.windows(2).filter(|pair| *pair == [0x58, 1]).count();
    assert_eq!(calls, 3);
}

#[test]
fn a_template_gives_properties_to_the_values_an_object_starts_with() {
    let compiled = compile_text(
        "object template 'name' [items]? @where;\n\
         a: object 'first' @nil;\nb: object 'second' [1, 2] @a;\n\
         _main(args) { return a.name; }\n",
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    let objs = block(&image, b"OBJS");
    // The first object has two properties from the template, and
    // where it comes in the source; the second has the list as well.
    assert_eq!(objs[0..2], [2, 0]);
    assert_eq!(objs[14..16], [3, 0]);
    let second = &objs[6 + 12 + 21..];
    assert_eq!(second[8..10], [4, 0]);

    // Values that fit no template are found out when the object is
    // generated; a class that is only declared might have had one.
    assert_eq!(
        compile_messages(
            "object template 'name';\nclass C: object;\nC template [items];\n\
             a: object [1];\nb: C 'x' 'y';\nextern class E;\nc: E 'x';\n_main(args) { }\n"
        ),
        [
            "4: error: object definition does not match any template",
            "5: error: object definition does not match any template",
            "7: error: cannot use template with an unimported extern class as superclass",
        ]
    );
    let messages = compile_messages(
        "object template 'a b' @5;
_main(args) { }
",
    );
    assert_eq!(
        messages[..2],
        [
            "1: error: property name required in object template string - found 'a b'",
            "1: error: property name required after \"@\" in object template - found \"5\"",
        ]
    );
    assert!(messages[2].ends_with("error: empty template definition"));
}

#[test]
fn extern_declares_and_transient_objects_go_in_a_block_of_their_own() {
    let compiled = compile_text(
        "extern function f(x);\nextern object later;\nextern object never;\n\
         _main(args) { return f(later); }\nf(x) { return x; }\n\
         later: object;\ntransient t: object;\n",
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    let all = blocks(&image, b"OBJS");
    // Two blocks of one object each; the second says its objects are
    // transient.
    assert_eq!(all.len(), 2);
    assert_eq!(all[0][0..6], [1, 0, 0, 0, 0, 0]);
    assert_eq!(all[1][0..6], [1, 0, 0, 0, 2, 0]);

    // What is declared and used has to be defined somewhere.
    assert_eq!(
        compile_messages(
            "extern function f(x);\nextern object o;\nextern object unused;\n\
             _main(args) { return f(o); }\n"
        ),
        [
            "0: error: unresolved external reference \"f\"",
            "0: error: unresolved external reference \"o\"",
        ]
    );
}

#[test]
fn plus_signs_put_an_object_in_the_one_before_with_one_fewer() {
    let compiled = compile_text(
        "+ property loc;\na: object;\n+ b: object;\n++ c: object;\n+ d: object loc = c;\n\
         _main(args) { return c.loc; }\n",
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    let objs = block(&image, b"OBJS");
    // Four objects; the second has two properties: where it comes in
    // the source, and the first object.
    assert_eq!(objs[0..2], [4, 0]);
    let second = &objs[6 + 6 + 13..];
    assert_eq!(second[0..4], [2, 0, 0, 0]);
    assert_eq!(second[8..10], [2, 0]);
    assert_eq!(second[21..26], [5, 1, 0, 0, 0]);
    // The last says where it is itself: in the third.
    let last = &objs[6 + 6 + 13 + 2 * 26..];
    assert_eq!(last[0..4], [4, 0, 0, 0]);
    assert_eq!(last[21..26], [5, 3, 0, 0, 0]);

    // There has to be a property for it, and an object to be in.
    assert_eq!(
        compile_messages("a: object;\n+ b: object;\n_main(args) { }\n"),
        ["2: error: too many '+' signs in object definition - location not defined"]
    );
    assert_eq!(
        compile_messages("+ property loc;\na: object;\n+++ b: object;\n+ property 5;\n"),
        [
            "3: error: too many '+' signs in object definition - location not defined",
            "4: error: symbol expected after '+ property', but found \"5\" instead",
        ]
    );
}

#[test]
fn an_object_can_be_the_value_of_a_property_of_another() {
    let compiled = compile_text(
        "a: object\n    inner: object { v = 1 }\n;\n\
         _main(args) { return a.inner.lexicalParent; }\n",
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    let objs = block(&image, b"OBJS");
    // Two objects. The outer one has the inner one for a property;
    // the inner one has three properties, the first of them the outer
    // one.
    assert_eq!(objs[0..2], [2, 0]);
    assert_eq!(objs[6..10], [1, 0, 0, 0]);
    assert_eq!(objs[14..16], [2, 0]);
    assert_eq!(objs[27..32], [5, 2, 0, 0, 0]);
    let inner = &objs[6 + 6 + 20..];
    assert_eq!(inner[0..4], [2, 0, 0, 0]);
    assert_eq!(inner[8..10], [3, 0]);
    assert_eq!(inner[12..19], [6, 0, 5, 1, 0, 0, 0]);

    // It has to be in braces, and cannot say what it is written in.
    assert_eq!(
        compile_messages(
            "a: object\n    lexicalParent = 1\n    n: object { lexicalParent = 2 }\n;\n\
             b: object\n    n: object v = 1;\n_main(args) { }\n"
        ),
        [
            "3: error: property \"lexicalParent\" already defined in object",
            "6: error: unterminated object definition",
        ]
    );
}

#[test]
fn a_propertyset_gives_its_properties_a_pattern_for_their_names_and_parameters() {
    // The same object written out in full makes the same image.
    let grouped = compile_text(
        "a: object\n    propertyset 'ver*Take' (actor, *, extra) {\n\
         Dobj(x) { return actor + x + extra; }\n        Iobj { return actor; }\n\
         val = 7\n        propertyset 'x*' (*, last) { A(q) { return q + last; } }\n    }\n;\n\
         _main(args) { return a.verDobjTake(1, 2, 3) + a.vervalTake; }\n",
    );
    let written_out = compile_text(
        "a: object\n    verDobjTake(actor, x, extra) { return actor + x + extra; }\n\
         verIobjTake(actor, extra) { return actor; }\n    vervalTake = 7\n\
         verxATake(actor, q, last, extra) { return q + last; }\n;\n\
         _main(args) { return a.verDobjTake(1, 2, 3) + a.vervalTake; }\n",
    );
    assert!(grouped.diagnostics.is_empty(), "{:?}", grouped.diagnostics);
    assert!(grouped.image.is_some());
    assert_eq!(grouped.image, written_out.image);

    assert_eq!(
        compile_messages(
            "a: object\n    propertyset 'no star' { x = 1 }\n    propertyset 5 { }\n\
             propertyset 'a*' (x, y) { }\n    propertyset 'b*' w = 1 }\n;\n_main(args) { }\n"
        ),
        [
            "2: error: propertyset pattern string \"'no star'\" is invalid",
            "3: error: expected property name pattern string after 'propertyset' - found \"5\"",
            "4: error: invalid formal parameters in propertyset definition",
            "5: error: missing open brace '{' in propertyset definition - found \"w\"",
        ]
    );
}

#[test]
fn a_method_can_stand_for_an_operator() {
    // The operator's property is one every program has, so naming it
    // the long way makes the same image.
    let compiled = compile_text(
        "a: object\n    operator +(x) { return x; }\n    operator []=(i, v) { return self; }\n\
         operator negate() { return 1; }\n;\n\
         _main(args) { return [a + 1, &operator +, &operator []=]; }\n",
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    let objs = block(&image, b"OBJS");
    // One object with four properties: where it comes in the source,
    // and the three operators, whose properties are numbers 11, 23
    // and 25.
    assert_eq!(objs[0..2], [1, 0]);
    assert_eq!(objs[14..16], [4, 0]);
    assert_eq!(objs[25..27], [11, 0]);
    assert_eq!(objs[32..34], [23, 0]);
    assert_eq!(objs[39..41], [25, 0]);

    // The number of parameters is checked when the method has been
    // read, which is at the start of what comes after it.
    assert_eq!(
        compile_messages(
            "a: object\n    operator +() { }\n    operator ~(x) { }\n    operator []=(i) { }\n\
             operator ==(x) { }\n    propertyset 'x*' { operator -(x) { } }\n;\n_main(args) { }\n"
        ),
        [
            "3: error: wrong number of parameters defined for overloaded operator (1 required)",
            "4: error: wrong number of parameters defined for overloaded operator (0 required)",
            "5: error: wrong number of parameters defined for overloaded operator (2 required)",
            "5: error: invalid overloaded operator '=='",
            "6: error: an 'operator' property cannot be defined within a 'propertyset'",
        ]
    );
}

#[test]
fn delegated_runs_another_objects_method_as_this_ones() {
    let compiled = compile_text(
        "a: object m(x) { return x; };\n\
         b: object\n    m(x) { return delegated a.m(x) + delegated a(x) + delegated a.(&m)(x); }\n;\n\
         f(...) { return argcount; }\n_main(args) { return b.m(1) + f(); }\n",
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    let code = code(&image);
    // The object to take the method from, and then the call: by the
    // property's number, or with the property on the stack.
    assert!(has_bytes(code, &[0x77, 1]));
    assert!(has_bytes(code, &[0x78, 1]));
    // The count of arguments is an instruction of its own.
    assert!(has_bytes(code, &[0x87, 0x50]));

    // There has to be an object for the method to run as.
    assert_eq!(
        compile_messages("a: object m() { return 1; };\n_main(args) { return delegated a.m(); }\n"),
        ["2: error: 'self' is not valid in this context"]
    );
}

#[test]
fn a_number_that_is_not_an_integer_is_an_object_in_the_image() {
    let compiled = compile_text("_main(args) { return [1.5, 2.5 + 1, 3.0 * 2, 1.5 < 2]; }\n");
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    // Three objects in a block of their own, each its number, the
    // length of its data, and the data: how many digits, the exponent,
    // the sign and kind, and the digits two to a byte.
    let objs = block(&image, b"OBJS");
    assert_eq!(objs[0..2], [3, 0]);
    assert_eq!(objs[6..18], [1, 0, 0, 0, 6, 0, 2, 0, 1, 0, 0, 0x15]);
    assert_eq!(objs[18..30], [2, 0, 0, 0, 6, 0, 2, 0, 1, 0, 0, 0x35]);
    assert_eq!(objs[30..42], [3, 0, 0, 0, 6, 0, 1, 0, 1, 0, 0, 0x60]);
    // Their class is one the image has to say it depends on.
    let text = String::from_utf8_lossy(&image);
    assert!(text.contains("bignumber/000000"));
}

#[test]
fn an_integer_too_large_to_be_one_is_a_number_that_is_not() {
    // It is reported when code is generated for it, once for each
    // place it is used; arithmetic that brings it back in range makes
    // it an integer again.
    assert_eq!(
        compile_messages(
            "_main(args)\n{\n    local a = 2147483647 + 1;\n    local b = 4294967296 - 4294967295;\n\
             local c = -2147483648;\n    local d = 0x100000000;\n    return [a, b, c, d];\n}\n"
        ),
        [
            "3: warning: constant value exceeds integer range; promoted to BigNumber",
            "6: warning: constant value exceeds integer range; promoted to BigNumber",
        ]
    );
    // Only integers have bits.
    assert_eq!(
        compile_messages("_main(args) { return [1.5 | 2, 1.5 / 0, ~1.5]; }\n"),
        [
            "1: error: floating point values can't be used with this operator",
            "1: error: divide by zero in constant expression",
            "1: error: unary '~' operator requires numeric value in constant expression",
        ]
    );
}

#[test]
fn a_regular_expression_is_an_object_that_points_at_its_text() {
    let compiled = compile_text("_main(args) { return R'a+b'; }\n");
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    let objs = block(&image, b"OBJS");
    // One object, whose data is a string in the constants.
    assert_eq!(objs[0..2], [1, 0]);
    assert_eq!(objs[6..13], [1, 0, 0, 0, 5, 0, 8]);
    assert!(String::from_utf8_lossy(&image).contains("regex-pattern/000000"));
}

#[test]
fn a_list_of_keys_and_values_is_a_lookup_table() {
    let compiled = compile_text("_main(args) { return [1 -> 2, 3 -> 4, * -> 5]; }\n");
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    let code = code(&image);
    // A list of the five values is made when the program runs, though
    // all are constants, and then a table is made of the list.
    assert!(has_bytes(code, &[0xC0, 5, 1, 0x8B, 0xC0, 1, 7]));

    // Every pair has to have its arrow, and the default comes last.
    assert_eq!(
        compile_messages(
            "_main(args)\n{\n    local a = [1, 2 -> 3];\n    local b = [1 -> 2, 3];\n\
             local c = [1 -> 2 -> 3];\n    local d = [* -> 1, 2 -> 3];\n    return [a, b, c, d];\n}\n"
        ),
        [
            "3: error: unexpected -> found in list (after 2nd element)",
            "4: error: missing -> in 2nd element in LookupTable list",
            "5: error: misplaced -> in LookupTable list after 1st element - expected comma",
            "6: error: LookupTable list must end after default *-> value, but found \",\"",
        ]
    );
}

#[test]
fn a_choice_in_a_string_gets_an_object_to_do_the_choosing() {
    let compiled = compile_text(
        "class OneOfIndexGen: object getNextIndex() { return 1; };\n\
         _main(args) { return 'a <<one of>>b<<or>>c<<or>>d<<cycling>> e'; }\n",
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    let objs = block(&image, b"OBJS");
    // The class, and an object of it with two properties: how many
    // alternatives there are, and how to choose.
    assert_eq!(objs[0..2], [2, 0]);
    let chooser = &objs[6 + 6 + 13..];
    assert_eq!(chooser[0..4], [2, 0, 0, 0]);
    assert_eq!(chooser[6..12], [1, 0, 2, 0, 0, 0]);
    assert_eq!(chooser[12..16], [1, 0, 0, 0]);
    assert_eq!(chooser[18..23], [7, 3, 0, 0, 0]);

    // The object has to be asked with a property.
    assert_eq!(
        compile_messages(
            "class OneOfIndexGen: object;\ngetNextIndex() { }\n\
             _main(args) { return 'a <<one of>>b<<or>>c<<stopping>>'; }\n"
        ),
        ["3: error: <<one of>> requires getNextIndex to be defined as a property"]
    );
}

#[test]
fn a_constant_division_gives_the_quotient_the_program_would_get() {
    // The reference stops at the point when both operands are
    // constants: there, `1 / 3.0` is 0 and `10.0 / 4` is 2. Here the
    // compiler's answer is the one the program gets when it divides.
    let compiled = compile_text(
        "_main(args) { return [10.0 / 4, 1 / 4.00, 4294967296 / 8]; }
",
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    let objs = block(&image, b"OBJS");
    // 2.5 and 0.25, each with the digits its text has when written
    // out; the third is the integer 536870912, and no object.
    assert_eq!(objs[0..2], [2, 0]);
    assert_eq!(objs[6..18], [1, 0, 0, 0, 6, 0, 2, 0, 1, 0, 0, 0x25]);
    assert_eq!(objs[18..30], [2, 0, 0, 0, 6, 0, 2, 0, 0, 0, 0, 0x25]);
}

/// The dictionaries in an image: each one's number and its words, with
/// how many uses each word has.
fn dictionaries(image: &[u8]) -> Vec<(u32, Vec<(String, usize)>)> {
    let mut found = Vec::new();
    for objs in blocks(image, b"OBJS") {
        // The dictionaries are of the third class every image has, in
        // a block whose objects give their sizes in four bytes.
        if objs[2..6] != [2, 0, 1, 0] {
            continue;
        }
        let count = usize::from(u16::from_le_bytes([objs[0], objs[1]]));
        let mut at = 6;
        for _ in 0..count {
            let id = u32::from_le_bytes(objs[at..at + 4].try_into().expect("four bytes"));
            let size = u32::from_le_bytes(objs[at + 4..at + 8].try_into().expect("four bytes"));
            let data = &objs[at + 8..at + 8 + size as usize];
            at += 8 + size as usize;
            let entries = usize::from(u16::from_le_bytes([data[4], data[5]]));
            let mut words = Vec::new();
            let mut pos = 6;
            for _ in 0..entries {
                let len = usize::from(data[pos]);
                let text: Vec<u8> = data[pos + 1..pos + 1 + len]
                    .iter()
                    .map(|byte| byte ^ 0xBD)
                    .collect();
                pos += 1 + len;
                let uses = usize::from(u16::from_le_bytes([data[pos], data[pos + 1]]));
                pos += 2 + 6 * uses;
                words.push((String::from_utf8_lossy(&text).into_owned(), uses));
            }
            found.push((id, words));
        }
    }
    found
}

#[test]
fn a_dictionary_holds_the_words_objects_have_for_its_properties() {
    let compiled = compile_text(
        "dictionary gDict;\ndictionary property noun, adjective;\n\
         class Thing: object noun = 'thing';\n\
         lamp: Thing noun = 'lamp' 'lantern' adjective = 'brass';\n\
         key: Thing adjective = 'brass';\n_main(args) { return gDict; }\n",
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    let found = dictionaries(&image);
    assert_eq!(found.len(), 1);
    // The dictionary is numbered just before the first object whose
    // words go in it. A class's words are its objects', not its own:
    // `thing` is a word for the lamp and the key.
    let (id, mut words) = found[0].clone();
    assert_eq!(id, 1);
    words.sort();
    assert_eq!(
        words,
        [
            ("brass".to_owned(), 2),
            ("lamp".to_owned(), 1),
            ("lantern".to_owned(), 1),
            ("thing".to_owned(), 2),
        ]
    );
    // The class and the two objects.
    let objs = block(&image, b"OBJS");
    assert_eq!(objs[0..2], [3, 0]);
}

#[test]
fn words_go_in_the_dictionary_that_was_current() {
    // The reference loses track when there are several dictionaries,
    // and puts one's words in another; here each has its own.
    let compiled = compile_text(
        "dictionary property noun;\ndictionary first;\na: object noun = 'a' 'shared';\n\
         dictionary second;\nb: object noun = 'b' 'shared';\nclass C: object noun = 'c';\n\
         dictionary first;\nd: C noun = 'd';\n_main(args) { return [first, second]; }\n",
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    let mut found = dictionaries(&image);
    assert_eq!(found.len(), 2);
    for (_, words) in &mut found {
        words.sort();
    }
    let names = |words: &[(String, usize)]| -> Vec<String> {
        words.iter().map(|(word, _)| word.clone()).collect()
    };
    // `first` is numbered before `second`, whose first object is the
    // class C, later in the table of names than `first` itself.
    assert_eq!(names(&found[0].1), ["a", "c", "d", "shared"]);
    assert_eq!(names(&found[1].1), ["b", "shared"]);
}

#[test]
fn dictionary_words_have_their_own_mistakes() {
    assert_eq!(
        compile_messages(
            "dictionary property noun;\na: object noun = 'x';\ndictionary gDict;\n\
             b: object\n    noun = 5\n;\nobject template 'noun';\nmodify gDict x = 1;\n\
             _main(args) { }\n"
        ),
        [
            "2: error: vocabulary property cannot be defined - no dictionary is active",
            "5: error: vocabulary property requires string value, but found \"5\"",
            "7: error: property \"noun\" is a dictionary property - not valid in templates",
            "8: error: empty template definition",
            "8: error: objects of this type cannot be modified or replaced",
        ]
    );
    // A name that is something else cannot be a dictionary's. The
    // reference says so and then stops with a memory fault.
    assert_eq!(
        compile_messages("f() { }\na: object;\ndictionary f;\ndictionary a;\n_main(args) { }\n"),
        [
            "3: error: symbol \"f\" is already defined - can't redefine as object",
            "4: error: redefining symbol \"a\" as dictionary object",
        ]
    );
}

/// The grammar productions in an image: each one's number and, for
/// each way it can be made up, how bad it is and how many things there
/// are to match.
fn productions(image: &[u8]) -> Vec<(u32, Vec<(u16, u16)>)> {
    let mut found = Vec::new();
    for objs in blocks(image, b"OBJS") {
        // The productions are of the fourth class every image has, in
        // a block whose objects give their sizes in four bytes.
        if objs[2..6] != [3, 0, 1, 0] {
            continue;
        }
        let count = usize::from(u16::from_le_bytes([objs[0], objs[1]]));
        let mut at = 6;
        for _ in 0..count {
            let id = u32::from_le_bytes(objs[at..at + 4].try_into().expect("four bytes"));
            let size = u32::from_le_bytes(objs[at + 4..at + 8].try_into().expect("four bytes"));
            let data = &objs[at + 8..at + 8 + size as usize];
            at += 8 + size as usize;
            let alts = usize::from(u16::from_le_bytes([data[0], data[1]]));
            let mut ways = Vec::new();
            let mut pos = 2;
            for _ in 0..alts {
                let badness = u16::from_le_bytes([data[pos + 2], data[pos + 3]]);
                let toks = u16::from_le_bytes([data[pos + 8], data[pos + 9]]);
                pos += 10;
                for _ in 0..toks {
                    pos += 3 + match data[pos + 2] {
                        1 | 4 => 4,
                        2 => 2,
                        3 => 2 + usize::from(u16::from_le_bytes([data[pos + 3], data[pos + 4]])),
                        6 => {
                            2 + 2 * usize::from(u16::from_le_bytes([data[pos + 3], data[pos + 4]]))
                        }
                        _ => 0,
                    };
                }
                ways.push((badness, toks));
            }
            found.push((id, ways));
        }
    }
    found
}

#[test]
fn a_grammar_statement_says_how_a_phrase_is_made_up() {
    let compiled = compile_text(
        "dictionary gDict;\ndictionary property noun;\nclass Match: object;\n\
         grammar np(plain): noun->n_ : Match;\n\
         grammar np: 'the' np->np_ | [badness 50] 'a' np->np_ : Match;\n\
         grammar cmd(put): 'put' np->d_ ('in' | 'into' | ) np->i_ ('now' | ) : Match;\n\
         grammar later;\n_main(args) { return [np, cmd, later]; }\n",
    );
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );
    let image = compiled.image.expect("an image");
    let found = productions(&image);
    assert_eq!(found.len(), 3);
    // The ways with no tag come first, in the order written, and then
    // those of the classes with tags.
    assert_eq!(found[0].1, [(0, 2), (50, 2), (0, 1)]);
    // Groups in parentheses are written out, a row for every choice,
    // the first group's choices changing fastest.
    assert_eq!(found[1].1, [(0, 5), (0, 5), (0, 4), (0, 4), (0, 4), (0, 3)]);
    // A production that is only declared has none.
    assert_eq!(found[2].1, []);
    // The words in the rows are in the dictionary, as words for their
    // productions.
    let mut words: Vec<String> = dictionaries(&image)[0]
        .1
        .iter()
        .map(|(word, _)| word.clone())
        .collect();
    words.sort();
    assert_eq!(words, ["a", "in", "into", "now", "put", "the"]);
}

#[test]
fn grammar_statements_have_their_own_mistakes() {
    assert_eq!(
        compile_messages(
            "class Match: object;\nf() { }\ngrammar np(a): f : Match;\n\
             grammar np(b): 'x' -> 5 : Match;\ngrammar np(c): * 'x' : Match;\n\
             grammar np(d): ('x' | 'y')->p_ : Match;\ngrammar np(a): 'dup' : Match;\n\
             modify grammar np: 'x' : ;\n_main(args) { }\n"
        ),
        [
            "3: error: object or property symbol required in 'grammar' token list (found \"f\")",
            "4: error: '->' in 'grammar' token list must be followed by a property name (found \"5\")",
            "4: error: invalid token \"5\" in 'grammar' token list",
            "5: error: '*' must be the last token in a 'grammar' alternative list (found \"'x'\")",
            "6: error: '->' is not allowed after parenthesized group in 'grammar' statement",
            "7: error: object \"np(a)\" is already defined - can't redefine as object",
            "8: error: 'modify/replace grammar' requires name-tag",
        ]
    );
    // A production that nothing says how to make up, and no statement
    // declares, is taken for a name spelled wrong; that is found out
    // last of all.
    assert_eq!(
        compile_messages(
            "class Match: object;\ngrammar np(a): nuon->n_ : Match;\n_main(args) { }\n"
        ),
        ["0: error: grammar production \"nuon\" has no alternatives defined"]
    );
}
