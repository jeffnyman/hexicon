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
