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
