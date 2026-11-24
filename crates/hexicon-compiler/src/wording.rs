//! Messages in Hexicon's own words.
//!
//! The compiler finds problems as the reference compiler did, and the
//! parts of it that find them say so in the reference's words. The
//! common ones are said again here, in words meant for the reader, often
//! with a note on what to look at. A message not in this table keeps the
//! reference's words, for now.

use crate::pp::Diagnostic;

/// The text between `before` and `after`, if `text` is that and nothing
/// else.
fn between<'t>(text: &'t str, before: &str, after: &str) -> Option<&'t str> {
    text.strip_prefix(before)?.strip_suffix(after)
}

/// A message and a note, in Hexicon's words.
fn hexicon_words(message: &str) -> Option<(String, Option<String>)> {
    if let Some(name) = between(
        message,
        "undefined symbol \"",
        "\" - assuming this is a property name",
    ) {
        return Some((
            format!("\"{name}\" is not defined anywhere"),
            Some(format!(
                "it is taken to be a property, which has no value until something gives it \
                 one; check how \"{name}\" is spelled"
            )),
        ));
    }
    if let Some((name, object)) = between(message, "undefined symbol \"", "\")")
        .and_then(|rest| rest.split_once("\" (used as superclass of \""))
    {
        return Some((
            format!("\"{name}\" is not defined anywhere"),
            Some(format!(
                "\"{object}\" is given it as a class; check how it is spelled, and that the \
                 file that defines it is among the sources"
            )),
        ));
    }
    if let Some(name) =
        between(message, "undefined symbol \"", "\"").filter(|name| !name.contains('"'))
    {
        return Some((
            format!("\"{name}\" is not defined anywhere"),
            Some(
                "check how it is spelled, and that the file that defines it is among the \
                 sources"
                    .to_owned(),
            ),
        ));
    }
    for (kind, a) in [
        ("function", "a"),
        ("object", "an"),
        ("property", "a"),
        ("intrinsic class", "an"),
    ] {
        if let Some(name) = between(
            message,
            "symbol \"",
            &format!("\" is already defined - can't redefine as {kind}"),
        ) {
            return Some((
                format!(
                    "\"{name}\" is already the name of something else, so it cannot also \
                     name {a} {kind}"
                ),
                None,
            ));
        }
    }
    if let Some(start) = message.strip_prefix("unterminated string literal: string started with ") {
        return Some((
            format!("this string has no end: it starts {start}"),
            Some(
                "a string ends with the quote it starts with; a quote inside it is written \
                 \\' or \\\""
                    .to_owned(),
            ),
        ));
    }
    if let Some(name) = between(message, "local variable \"", "\" is never used") {
        return Some((format!("the local variable \"{name}\" is never used"), None));
    }
    if let Some(name) = between(
        message,
        "value is assigned to local variable \"",
        "\" but the value is never used",
    ) {
        return Some((
            format!("\"{name}\" is given a value that is never used"),
            None,
        ));
    }
    if let Some(name) = between(message, "no value is assigned to local variable \"", "\"") {
        return Some((
            format!("\"{name}\" is used, but never given a value"),
            Some("its value is nil".to_owned()),
        ));
    }
    if message == "unterminated object definition" {
        return Some((
            "this object has no ';' at its end".to_owned(),
            Some("each object ends with ';'".to_owned()),
        ));
    }
    if let Some(found) = between(
        message,
        "missing right brace at end of object definition - found \"",
        "\"",
    ) {
        return Some((
            format!("this object, written in braces, has no '}}' at its end: found \"{found}\""),
            None,
        ));
    }
    if let Some(found) = between(
        message,
        "expected function or object definition, but found \"",
        "\"",
    ) {
        return Some((
            format!("\"{found}\" cannot start anything here"),
            Some("what is expected is a function, an object, or a class".to_owned()),
        ));
    }
    if let Some(name) = between(message, "cannot open include file \"", "\"") {
        return Some((
            format!("there is no file \"{name}\" to include"),
            Some("it is looked for beside this file, and then in the include folders".to_owned()),
        ));
    }
    if message == "object definition does not match any template" {
        return Some((
            "the values written here without names fit none of the class's templates".to_owned(),
            Some(
                "a template says what such values stand for: a Thing's, for instance, are its vocabulary \
                 in single quotes, then its description in double quotes"
                    .to_owned(),
            ),
        ));
    }
    None
}

/// Mark a problem found in a Hexicon source, and say it in Hexicon's
/// words.
pub(crate) fn for_hexicon(diagnostic: &mut Diagnostic) {
    diagnostic.hexicon = true;
    if diagnostic.note.is_none()
        && let Some((message, note)) = hexicon_words(&diagnostic.message)
    {
        diagnostic.message = message;
        diagnostic.note = note;
    }
}

/// A name the program does not define, said to be a slip for one it
/// does, if there is one: `"weigth" is not defined anywhere` gets
/// `did you mean "weight"?` for its note.
pub(crate) fn suggest_name(diagnostic: &mut Diagnostic, globals: &crate::parse::Globals) {
    let Some(name) = between(&diagnostic.message, "\"", "\" is not defined anywhere") else {
        return;
    };
    let mut names: Vec<&[u8]> = globals
        .names()
        .filter(|have| crate::names::is_slip(name.as_bytes(), have))
        .collect();
    names.sort_unstable();
    if let Some(meant) = names.first() {
        diagnostic.note = Some(format!(
            "did you mean \"{}\"?",
            String::from_utf8_lossy(meant)
        ));
    }
}
