//! TADS regular expressions.
//!
//! The dialect is TADS's own: `%` is the escape character, character
//! classes are written `<alpha|digit>`, and flags such as `<nocase>` and
//! `<min>` appear in the pattern text. Which of several possible matches
//! wins is defined by how the matcher explores the pattern, so this is a
//! port of the reference VM's engine (`vmregex.cpp`): the same state
//! machine, built the same way, and searched the same way. Hexicon does
//! not use a general-purpose regex library for that reason.
//!
//! Positions are byte offsets into UTF-8 text.

/// Number of groups whose matches are recorded.
pub(crate) const GROUP_COUNT: usize = 10;

/// A state number, or `NONE`.
type State = i32;
const NONE: State = -1;

/// A named class of characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Alpha,
    Digit,
    Upper,
    Lower,
    AlphaNum,
    Space,
    VSpace,
    Punct,
    Newline,
    Null,
}

/// One member of a character range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RangeItem {
    Chars(char, char),
    Class(Class),
}

/// What a state matches.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    Invalid,
    /// Matches nothing; just moves on. With two exits, a choice.
    Epsilon,
    Literal(char),
    TextBegin,
    TextEnd,
    /// The position a look-back assertion started from.
    LookbackPos,
    WordBegin,
    WordEnd,
    WordChar,
    NonWordChar,
    WordBoundary,
    NonWordBoundary,
    Wildcard,
    Range(Vec<RangeItem>, bool),
    Class(Class),
    NonDigit,
    NonSpace,
    NonVSpace,
    /// Matches the text an earlier group matched.
    GroupMatch(usize),
    GroupEnter(usize),
    GroupExit(usize),
    /// A look-ahead or look-back assertion over a sub-machine.
    Assert {
        negative: bool,
        back: bool,
        init: State,
        fin: State,
        min_len: i32,
        max_len: i32,
    },
    /// Reset a loop counter.
    ZeroVar(usize),
    /// The top of a counted loop.
    LoopBranch {
        var: usize,
        min: i32,
        max: i32,
    },
}

#[derive(Debug, Clone)]
struct Tuple {
    kind: Kind,
    next1: State,
    next2: State,
    /// At a choice, prefer the shorter match.
    shortest: bool,
}

/// A sub-machine under construction: its entry and exit states.
#[derive(Debug, Clone, Copy)]
struct Machine {
    init: State,
    fin: State,
}

const NULL_MACHINE: Machine = Machine {
    init: NONE,
    fin: NONE,
};

/// A compiled pattern.
#[derive(Debug, Clone)]
pub(crate) struct Pattern {
    tuples: Vec<Tuple>,
    machine: Machine,
    /// Number of capturing groups, up to [`GROUP_COUNT`].
    pub group_count: usize,
    loop_vars: usize,
    /// Prefer the longest match (`<max>`, the default) or the shortest.
    longest_match: bool,
    /// Prefer the match that begins first (the default) or ends first.
    first_begin: bool,
    case_sensitive: bool,
    case_specified: bool,
}

/// Where each group matched: start and end offsets, or `None`.
pub(crate) type Groups = [Option<(usize, usize)>; GROUP_COUNT];

// ---- character classes ----

fn is_vspace(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

pub(crate) fn is_space(c: char) -> bool {
    c.is_whitespace() && !is_vspace(c)
}

fn is_digit(c: char) -> bool {
    c.is_ascii_digit() || (!c.is_ascii() && c.is_numeric() && !c.is_alphabetic())
}

fn is_punct(c: char) -> bool {
    if c.is_ascii() {
        c.is_ascii_punctuation()
    } else {
        !c.is_alphanumeric() && !c.is_whitespace() && !c.is_control()
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphabetic() || is_digit(c)
}

fn is_newline(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{b}' | '\u{2028}' | '\u{2029}')
}

fn in_class(class: Class, c: char) -> bool {
    match class {
        Class::Alpha => c.is_alphabetic(),
        Class::Digit => is_digit(c),
        Class::Upper => c.is_uppercase(),
        Class::Lower => c.is_lowercase(),
        Class::AlphaNum => c.is_alphabetic() || is_digit(c),
        Class::Space => is_space(c),
        Class::VSpace => is_vspace(c),
        Class::Punct => is_punct(c),
        Class::Newline => is_newline(c),
        Class::Null => c == '\0',
    }
}

/// Fold a character for a comparison that ignores case.
pub(crate) fn fold(c: char) -> char {
    let mut lower = c.to_lowercase();
    match (lower.next(), lower.next()) {
        (Some(folded), None) => folded,
        _ => c,
    }
}

// ---- compiling ----

struct Compiler {
    tuples: Vec<Tuple>,
    range: Vec<RangeItem>,
}

struct GroupFrame {
    old_cur: Machine,
    old_alter: Machine,
    group_id: usize,
    capturing: bool,
    negative: bool,
    positive: bool,
    back: bool,
}

impl Compiler {
    fn alloc_state(&mut self) -> State {
        self.tuples.push(Tuple {
            kind: Kind::Invalid,
            next1: NONE,
            next2: NONE,
            shortest: false,
        });
        self.tuples.len() as State - 1
    }

    /// Add a transition out of a state. The first one sets what the
    /// state matches; a second only adds an exit.
    fn set_trans(&mut self, id: State, dest: State, kind: Kind) {
        if id == NONE {
            return;
        }
        let tuple = &mut self.tuples[id as usize];
        if tuple.next1 == NONE {
            tuple.kind = kind;
            tuple.next1 = dest;
        } else {
            tuple.next2 = dest;
        }
    }

    fn init_machine(&mut self) -> Machine {
        let init = self.alloc_state();
        let fin = self.alloc_state();
        Machine { init, fin }
    }

    fn build(&mut self, kind: Kind) -> Machine {
        let machine = self.init_machine();
        self.set_trans(machine.init, machine.fin, kind);
        machine
    }

    fn build_range(&mut self, exclude: bool) -> Machine {
        let items = self.range.clone();
        self.build(Kind::Range(items, exclude))
    }

    fn build_concat(&mut self, lhs: Machine, rhs: Machine) -> Machine {
        let machine = self.init_machine();
        self.set_trans(machine.init, lhs.init, Kind::Epsilon);
        self.set_trans(lhs.fin, rhs.init, Kind::Epsilon);
        self.set_trans(rhs.fin, machine.fin, Kind::Epsilon);
        machine
    }

    fn build_group(&mut self, sub: Machine, id: usize) -> Machine {
        let machine = self.init_machine();
        self.set_trans(machine.init, sub.init, Kind::GroupEnter(id));
        self.set_trans(sub.fin, machine.fin, Kind::GroupExit(id));
        machine
    }

    fn build_assert(&mut self, sub: Machine, negative: bool, back: bool) -> Machine {
        let machine = self.init_machine();
        let (mut min_len, mut max_len) = (0, 0);
        if back {
            self.match_length(
                sub.init,
                sub.fin,
                &mut min_len,
                &mut max_len,
                &mut Vec::new(),
            );
        }
        self.set_trans(
            machine.init,
            machine.fin,
            Kind::Assert {
                negative,
                back,
                init: sub.init,
                fin: sub.fin,
                min_len,
                max_len,
            },
        );
        machine
    }

    fn build_alter(&mut self, lhs: Machine, rhs: Machine) -> Machine {
        let machine = self.init_machine();
        self.set_trans(machine.init, lhs.init, Kind::Epsilon);
        self.set_trans(machine.init, rhs.init, Kind::Epsilon);
        self.set_trans(lhs.fin, machine.fin, Kind::Epsilon);
        self.set_trans(rhs.fin, machine.fin, Kind::Epsilon);
        machine
    }

    /// `*`, `+`, or `?`. The order the two exits are added in decides
    /// which the matcher tries first.
    fn build_closure(&mut self, sub: Machine, specifier: char, shortest: bool) -> Machine {
        let machine = self.init_machine();
        if !shortest {
            self.set_trans(machine.init, sub.init, Kind::Epsilon);
        } else {
            self.set_trans(sub.fin, machine.fin, Kind::Epsilon);
        }
        if specifier != '?' {
            self.set_trans(sub.fin, sub.init, Kind::Epsilon);
            if shortest {
                self.tuples[sub.fin as usize].shortest = true;
            }
        }
        if specifier != '+' {
            self.set_trans(machine.init, machine.fin, Kind::Epsilon);
            if shortest {
                self.tuples[machine.init as usize].shortest = true;
            }
        }
        if !shortest {
            self.set_trans(sub.fin, machine.fin, Kind::Epsilon);
        } else {
            self.set_trans(machine.init, sub.init, Kind::Epsilon);
        }
        machine
    }

    /// `{min,max}`.
    fn build_interval(
        &mut self,
        sub: Machine,
        min: i32,
        max: i32,
        var: usize,
        shortest: bool,
    ) -> Machine {
        let machine = self.init_machine();
        let inner = self.init_machine();
        let branch = Kind::LoopBranch { var, min, max };
        if shortest {
            self.set_trans(inner.init, machine.fin, branch.clone());
            self.set_trans(inner.init, sub.init, branch);
        } else {
            self.set_trans(inner.init, sub.init, branch.clone());
            self.set_trans(inner.init, machine.fin, branch);
        }
        self.set_trans(sub.fin, inner.init, Kind::Epsilon);
        self.set_trans(machine.init, inner.init, Kind::ZeroVar(var));
        if shortest {
            self.tuples[inner.init as usize].shortest = true;
        }
        machine
    }

    fn concat_onto(&mut self, dest: &mut Machine, rhs: Machine) {
        *dest = if dest.init == NONE {
            rhs
        } else {
            self.build_concat(*dest, rhs)
        };
    }

    fn alternate_onto(&mut self, dest: &mut Machine, rhs: Machine) {
        if dest.init == NONE {
            *dest = rhs;
        } else if rhs.init != NONE {
            *dest = self.build_alter(*dest, rhs);
        }
    }

    fn add_range_chars(&mut self, lo: char, hi: char) {
        let mut lo = lo;
        if lo == '\0' {
            self.range.push(RangeItem::Class(Class::Null));
            if hi == '\0' {
                return;
            }
            lo = '\u{1}';
        }
        self.range.push(RangeItem::Chars(lo, hi));
    }

    /// Parse an angle-bracket class expression such as `<alpha|digit>`,
    /// starting at the `<`. On success, returns the machine and the index
    /// of the closing `>`.
    fn class_expr(&mut self, expr: &[char], at: usize) -> Option<(Machine, usize)> {
        const LITERALS: &[(&str, char)] = &[
            ("langle", '<'),
            ("rangle", '>'),
            ("vbar", '|'),
            ("caret", '^'),
            ("squote", '\''),
            ("dquote", '"'),
            ("star", '*'),
            ("question", '?'),
            ("percent", '%'),
            ("dot", '.'),
            ("period", '.'),
            ("plus", '+'),
            ("lsquare", '['),
            ("rsquare", ']'),
            ("lparen", '('),
            ("rparen", ')'),
            ("lbrace", '{'),
            ("rbrace", '}'),
            ("dollar", '$'),
            ("backslash", '\\'),
            ("return", '\r'),
            ("linefeed", '\n'),
            ("tab", '\t'),
            ("nul", '\0'),
            ("null", '\0'),
        ];
        const CLASSES: &[(&str, Class)] = &[
            ("alpha", Class::Alpha),
            ("digit", Class::Digit),
            ("upper", Class::Upper),
            ("lower", Class::Lower),
            ("alphanum", Class::AlphaNum),
            ("space", Class::Space),
            ("punct", Class::Punct),
            ("newline", Class::Newline),
            ("vspace", Class::VSpace),
        ];
        let mut p = at + 1;
        let mut exclude = false;
        if expr.get(p) == Some(&'^') {
            exclude = true;
            p += 1;
        }
        self.range.clear();
        loop {
            let start = p;
            while p < expr.len() && expr[p] != '>' && expr[p] != '|' {
                p += 1;
            }
            if p >= expr.len() {
                return None;
            }
            let delim = expr[p];
            let part = &expr[start..p];
            if part.len() == 1 {
                self.add_range_chars(part[0], part[0]);
            } else if part.len() == 3 && part[1] == '-' {
                self.add_range_chars(part[0], part[2]);
            } else {
                let name: String = part.iter().collect::<String>().to_ascii_lowercase();
                if let Some(&(_, c)) = LITERALS.iter().find(|(n, _)| *n == name) {
                    self.add_range_chars(c, c);
                } else if let Some(&(_, class)) = CLASSES.iter().find(|(n, _)| *n == name) {
                    // A class on its own needs no range.
                    if self.range.is_empty() && delim == '>' && !exclude {
                        return Some((self.build(Kind::Class(class)), p));
                    }
                    self.range.push(RangeItem::Class(class));
                } else {
                    return None;
                }
            }
            if delim == '|' {
                p += 1;
            } else {
                return Some((self.build_range(exclude), p));
            }
        }
    }

    /// Cut the empty loops out of a machine, such as the one `(a*)*`
    /// makes, which would otherwise never terminate.
    fn break_loops(&mut self, init: State, fin: State, stack: &mut Vec<State>) -> bool {
        let depth = stack.len();
        stack.push(NONE);
        let result = self.break_loops_from(init, fin, stack, depth);
        stack.truncate(depth);
        result
    }

    fn break_loops_from(
        &mut self,
        init: State,
        fin: State,
        stack: &mut Vec<State>,
        depth: usize,
    ) -> bool {
        let mut cur = init;
        loop {
            if cur == fin || cur == NONE {
                return false;
            }
            stack[depth] = cur;
            let tuple = self.tuples[cur as usize].clone();
            if tuple.kind == Kind::Epsilon && tuple.next2 != NONE {
                if stack[..depth].contains(&cur) {
                    return true;
                }
                if self.break_loops(tuple.next2, fin, stack) {
                    self.tuples[cur as usize].next2 = NONE;
                }
                if self.break_loops(tuple.next1, fin, stack) {
                    let t = &mut self.tuples[cur as usize];
                    t.next1 = t.next2;
                    t.next2 = NONE;
                }
                return self.tuples[cur as usize].next1 == NONE;
            }
            match tuple.kind {
                Kind::Epsilon
                | Kind::GroupEnter(_)
                | Kind::GroupExit(_)
                | Kind::TextBegin
                | Kind::TextEnd
                | Kind::LookbackPos
                | Kind::WordBegin
                | Kind::WordEnd
                | Kind::WordBoundary
                | Kind::NonWordBoundary
                | Kind::ZeroVar(_) => cur = tuple.next1,
                Kind::Assert {
                    init, fin: sub_fin, ..
                } => {
                    if self.break_loops(init, sub_fin, &mut Vec::new())
                        && let Kind::Assert { init, fin, .. } = &mut self.tuples[cur as usize].kind
                    {
                        *init = *fin;
                    }
                    cur = tuple.next1;
                }
                _ => return false,
            }
        }
    }

    /// Skip over states that do nothing but lead to another state.
    fn optimize_transition(&self, fin: State, mut trans: State) -> State {
        loop {
            if trans == NONE || trans == fin {
                return trans;
            }
            let next = &self.tuples[trans as usize];
            if next.kind != Kind::Epsilon || next.next2 != NONE {
                return trans;
            }
            trans = next.next1;
        }
    }

    /// The shortest and longest text a sub-machine can match, in
    /// characters; -1 for no limit.
    fn match_length(
        &self,
        init: State,
        fin: State,
        min_len: &mut i32,
        max_len: &mut i32,
        stack: &mut Vec<State>,
    ) {
        fn add(a: i32, b: i32) -> i32 {
            if a < 0 || b < 0 { -1 } else { a + b }
        }
        fn mul(a: i32, b: i32) -> i32 {
            if a < 0 || b < 0 { -1 } else { a * b }
        }
        fn min(a: i32, b: i32) -> i32 {
            if a < 0 {
                b
            } else if b < 0 || a < b {
                a
            } else {
                b
            }
        }
        fn max(a: i32, b: i32) -> i32 {
            if a < 0 || b < 0 { -1 } else { a.max(b) }
        }
        *min_len = 0;
        *max_len = 0;
        let depth = stack.len();
        stack.push(NONE);
        let mut i = init;
        while i != fin && i != NONE {
            stack[depth] = i;
            let recursing = stack[..depth].contains(&i);
            let t = &self.tuples[i as usize];
            match &t.kind {
                Kind::Epsilon if t.next2 != NONE => {
                    if recursing {
                        *max_len = -1;
                    } else {
                        let (mut min1, mut max1, mut min2, mut max2) = (0, 0, 0, 0);
                        self.match_length(t.next1, fin, &mut min1, &mut max1, stack);
                        self.match_length(t.next2, fin, &mut min2, &mut max2, stack);
                        *min_len = add(*min_len, min(min1, min2));
                        *max_len = add(*max_len, max(max1, max2));
                    }
                    break;
                }
                Kind::LoopBranch {
                    min: lo, max: hi, ..
                } => {
                    if recursing {
                        *max_len = -1;
                    } else {
                        let (mut loop_min, mut loop_max) = (0, 0);
                        self.match_length(t.next1, i, &mut loop_min, &mut loop_max, stack);
                        *min_len = add(*min_len, mul(loop_min, *lo));
                        *max_len = add(*max_len, mul(loop_max, *hi));
                    }
                    i = t.next2;
                }
                Kind::GroupMatch(_) => {
                    // The length of a back-reference is not known here.
                    *max_len = -1;
                    i = t.next1;
                }
                Kind::Epsilon
                | Kind::TextBegin
                | Kind::TextEnd
                | Kind::LookbackPos
                | Kind::WordBegin
                | Kind::WordEnd
                | Kind::WordBoundary
                | Kind::NonWordBoundary
                | Kind::GroupEnter(_)
                | Kind::GroupExit(_)
                | Kind::Assert { .. }
                | Kind::ZeroVar(_) => i = t.next1,
                _ => {
                    *min_len = add(*min_len, 1);
                    *max_len = add(*max_len, 1);
                    i = t.next1;
                }
            }
        }
        stack.truncate(depth);
    }
}

fn parse_int(expr: &[char], at: &mut usize) -> i32 {
    if !expr.get(*at).is_some_and(char::is_ascii_digit) {
        return -1;
    }
    let mut acc: i32 = 0;
    while let Some(digit) = expr.get(*at).and_then(|c| c.to_digit(10)) {
        acc = acc.saturating_mul(10).saturating_add(digit as i32);
        *at += 1;
    }
    acc
}

impl Pattern {
    /// Compile a pattern. Every string compiles to something; text that
    /// is not valid syntax is taken literally.
    pub(crate) fn compile(text: &[u8]) -> Self {
        let expr: Vec<char> = String::from_utf8_lossy(text).chars().collect();
        let mut c = Compiler {
            tuples: Vec::new(),
            range: Vec::new(),
        };
        let mut pattern = Pattern {
            tuples: Vec::new(),
            machine: NULL_MACHINE,
            group_count: 0,
            loop_vars: 0,
            longest_match: true,
            first_begin: true,
            case_sensitive: true,
            case_specified: false,
        };
        let mut cur = NULL_MACHINE;
        let mut alter = NULL_MACHINE;
        let mut groups: Vec<GroupFrame> = Vec::new();
        let starts_with = |at: usize, word: &str| {
            let end = at + word.len();
            end <= expr.len()
                && expr[at..end]
                    .iter()
                    .zip(word.chars())
                    .all(|(a, b)| a.eq_ignore_ascii_case(&b))
        };

        let peek = |i: usize, k: usize| expr.get(i + k).copied();
        let mut i = 0;
        // After the text, any groups left open are closed.
        while i < expr.len() || !groups.is_empty() {
            // What to do with the machine built for this element.
            enum Then {
                /// Nothing further.
                Done,
                /// Look for a closure suffix, then add it to the sequence.
                Postfix(Machine),
            }
            let ch = expr.get(i).copied();
            let remaining = expr.len().saturating_sub(i);
            let then = match ch {
                None | Some(')') => {
                    match groups.pop() {
                        // A close with nothing open is ignored.
                        None => Then::Done,
                        Some(frame) => {
                            if frame.back {
                                let m = c.build(Kind::LookbackPos);
                                c.concat_onto(&mut cur, m);
                            }
                            c.alternate_onto(&mut alter, cur);
                            let new = if frame.capturing {
                                c.build_group(alter, frame.group_id)
                            } else if frame.positive || frame.negative {
                                c.build_assert(alter, frame.negative, frame.back)
                            } else {
                                alter
                            };
                            cur = frame.old_cur;
                            alter = frame.old_alter;
                            Then::Postfix(new)
                        }
                    }
                }
                Some('^') if cur.init == NONE => {
                    let m = c.build(Kind::TextBegin);
                    c.concat_onto(&mut cur, m);
                    Then::Done
                }
                Some('$')
                    if !(remaining > 1 && peek(i, 1) != Some(')') && peek(i, 1) != Some('|')) =>
                {
                    let m = c.build(Kind::TextEnd);
                    c.concat_onto(&mut cur, m);
                    Then::Done
                }
                Some('(') => {
                    let mut frame = GroupFrame {
                        old_cur: cur,
                        old_alter: alter,
                        group_id: pattern.group_count,
                        capturing: true,
                        negative: false,
                        positive: false,
                        back: false,
                    };
                    if remaining > 2 && peek(i, 1) == Some('?') {
                        match peek(i, 2) {
                            Some('<') if remaining > 3 && matches!(peek(i, 3), Some('=' | '!')) => {
                                frame.back = true;
                                frame.capturing = false;
                                frame.positive = peek(i, 3) == Some('=');
                                frame.negative = !frame.positive;
                                i += 3;
                            }
                            Some(':') => {
                                frame.capturing = false;
                                i += 2;
                            }
                            Some('=') => {
                                frame.positive = true;
                                frame.capturing = false;
                                i += 2;
                            }
                            Some('!') => {
                                frame.negative = true;
                                frame.capturing = false;
                                i += 2;
                            }
                            _ => {}
                        }
                    }
                    if frame.capturing {
                        pattern.group_count += 1;
                    }
                    groups.push(frame);
                    cur = NULL_MACHINE;
                    alter = NULL_MACHINE;
                    Then::Done
                }
                Some('|') => {
                    c.alternate_onto(&mut alter, cur);
                    cur = NULL_MACHINE;
                    Then::Done
                }
                Some('<') => {
                    let mut flag = true;
                    if starts_with(i, "<fe>") || starts_with(i, "<firstend>") {
                        pattern.first_begin = false;
                    } else if starts_with(i, "<fb>") || starts_with(i, "<firstbegin>") {
                        pattern.first_begin = true;
                    } else if starts_with(i, "<max>") {
                        pattern.longest_match = true;
                    } else if starts_with(i, "<min>") {
                        pattern.longest_match = false;
                    } else if starts_with(i, "<case>") {
                        pattern.case_sensitive = true;
                        pattern.case_specified = true;
                    } else if starts_with(i, "<nocase>") {
                        pattern.case_sensitive = false;
                        pattern.case_specified = true;
                    } else {
                        flag = false;
                    }
                    if flag {
                        while i < expr.len() && expr[i] != '>' {
                            i += 1;
                        }
                        Then::Done
                    } else {
                        match c.class_expr(&expr, i) {
                            Some((machine, end)) => {
                                i = end;
                                Then::Postfix(machine)
                            }
                            None => Then::Postfix(c.build(Kind::Literal('<'))),
                        }
                    }
                }
                Some('%') => {
                    if remaining < 2 {
                        // A lone % at the end is ignored.
                        Then::Done
                    } else {
                        i += 1;
                        let simple = |c: &mut Compiler, cur: &mut Machine, kind: Kind| {
                            let m = c.build(kind);
                            c.concat_onto(cur, m);
                            Then::Done
                        };
                        match expr[i] {
                            d @ '1'..='9' => {
                                Then::Postfix(c.build(Kind::GroupMatch(d as usize - '1' as usize)))
                            }
                            '<' => simple(&mut c, &mut cur, Kind::WordBegin),
                            '>' => simple(&mut c, &mut cur, Kind::WordEnd),
                            'b' => simple(&mut c, &mut cur, Kind::WordBoundary),
                            'B' => simple(&mut c, &mut cur, Kind::NonWordBoundary),
                            'w' => Then::Postfix(c.build(Kind::WordChar)),
                            'W' => Then::Postfix(c.build(Kind::NonWordChar)),
                            's' => Then::Postfix(c.build(Kind::Class(Class::Space))),
                            'S' => Then::Postfix(c.build(Kind::NonSpace)),
                            'd' => Then::Postfix(c.build(Kind::Class(Class::Digit))),
                            'D' => Then::Postfix(c.build(Kind::NonDigit)),
                            'v' => Then::Postfix(c.build(Kind::Class(Class::VSpace))),
                            'V' => Then::Postfix(c.build(Kind::NonVSpace)),
                            other => Then::Postfix(c.build(Kind::Literal(other))),
                        }
                    }
                }
                Some('.') => Then::Postfix(c.build(Kind::Wildcard)),
                Some('[') => {
                    c.range.clear();
                    i += 1;
                    let mut exclude = false;
                    if expr.get(i) == Some(&'^') {
                        i += 1;
                        exclude = true;
                    }
                    if expr.get(i) == Some(&']') {
                        c.add_range_chars(']', ']');
                        i += 1;
                    }
                    if expr.get(i) == Some(&'-') {
                        c.add_range_chars('-', '-');
                        i += 1;
                    }
                    while i < expr.len() && expr[i] != ']' {
                        let lo = expr[i];
                        i += 1;
                        if expr.get(i) == Some(&'-') {
                            i += 1;
                            if let Some(&hi) = expr.get(i) {
                                i += 1;
                                c.add_range_chars(lo.min(hi), lo.max(hi));
                            }
                        } else {
                            c.add_range_chars(lo, lo);
                        }
                    }
                    Then::Postfix(c.build_range(exclude))
                }
                Some(other) => Then::Postfix(c.build(Kind::Literal(other))),
            };

            if let Then::Postfix(mut new) = then {
                let at = peek;
                let mut skip_closures = false;
                match at(i, 1) {
                    Some(spec @ ('*' | '+' | '?')) => {
                        i += 1;
                        let shortest = at(i, 1) == Some('?');
                        new = c.build_closure(new, spec, shortest);
                        skip_closures = true;
                    }
                    Some('{') => {
                        let var = groups.len();
                        pattern.loop_vars = pattern.loop_vars.max(var + 1);
                        i += 2;
                        let min = parse_int(&expr, &mut i);
                        let max = if expr.get(i) == Some(&',') {
                            i += 1;
                            parse_int(&expr, &mut i)
                        } else {
                            min
                        };
                        while i < expr.len() && expr[i] != '}' {
                            i += 1;
                        }
                        let mut shortest = false;
                        if at(i, 1) == Some('?') {
                            shortest = true;
                            i += 1;
                        }
                        new = c.build_interval(new, min, max, var, shortest);
                        skip_closures = true;
                    }
                    _ => {}
                }
                // Only one closure applies; any more are skipped.
                #[allow(clippy::while_immutable_condition)]
                while skip_closures {
                    // Runs until there are no more suffixes to skip.
                    match at(i, 1) {
                        Some('?' | '+' | '*') => i += 1,
                        Some('{') => {
                            while i + 1 < expr.len() && expr[i + 1] != '}' {
                                i += 1;
                            }
                            if at(i, 1) != Some('}') {
                                break;
                            }
                            i += 1;
                        }
                        _ => break,
                    }
                }
                c.concat_onto(&mut cur, new);
            }
            i += 1;
        }

        c.alternate_onto(&mut alter, cur);
        if alter.init != NONE {
            if c.break_loops(alter.init, alter.fin, &mut Vec::new()) {
                alter.fin = alter.init;
            }
            alter.init = c.optimize_transition(alter.fin, alter.init);
            for index in 0..c.tuples.len() {
                if index as State != alter.fin {
                    let next1 = c.optimize_transition(alter.fin, c.tuples[index].next1);
                    let next2 = c.optimize_transition(alter.fin, c.tuples[index].next2);
                    c.tuples[index].next1 = next1;
                    c.tuples[index].next2 = next2;
                }
            }
        }
        pattern.machine = alter;
        pattern.tuples = c.tuples;
        pattern.group_count = pattern.group_count.min(GROUP_COUNT);
        pattern
    }
}

// ---- matching ----

/// The state the matcher saves at a choice and restores to try the other
/// way: the group registers and loop counters.
#[derive(Debug, Clone)]
struct Saved {
    groups: [(i64, i64); GROUP_COUNT],
    loops: Vec<i32>,
}

enum Frame {
    /// The first branch of a choice is being tried.
    First {
        state: State,
        fin: State,
        pos: usize,
        saved: Box<Saved>,
    },
    /// The second branch is being tried; the first ended with `first`.
    Second {
        state: State,
        fin: State,
        saved: Box<Saved>,
        first: Option<usize>,
        first_result: Box<Saved>,
    },
    /// An assertion's sub-pattern is being tried.
    Assert {
        state: State,
        fin: State,
        pos: usize,
        /// The limit in force outside the assertion.
        limit: usize,
        saved: Box<Saved>,
        /// Extra characters of look-back tried so far.
        extra: i32,
    },
}

fn char_at(text: &[u8], pos: usize) -> Option<(char, usize)> {
    let first = *text.get(pos)?;
    let len = match first {
        0x00..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf7 => 4,
        _ => 1,
    };
    let end = (pos + len).min(text.len());
    let c = std::str::from_utf8(&text[pos..end])
        .ok()
        .and_then(|s| s.chars().next())
        .unwrap_or('\u{fffd}');
    Some((c, end - pos))
}

/// The start of the character before a position.
fn prev_pos(text: &[u8], pos: usize) -> usize {
    let mut at = pos.saturating_sub(1);
    while at > 0 && text[at] & 0xc0 == 0x80 {
        at -= 1;
    }
    at
}

fn char_before(text: &[u8], pos: usize) -> Option<char> {
    if pos == 0 {
        return None;
    }
    char_at(text, prev_pos(text, pos)).map(|(c, _)| c)
}

impl Pattern {
    /// Match the pattern at a position. Returns the length of the match
    /// in bytes and fills in the groups.
    ///
    /// At each choice in the pattern, both ways are tried and the better
    /// one kept: the longer match, unless the pattern says otherwise.
    pub(crate) fn match_at(
        &self,
        text: &[u8],
        start: usize,
        limit: usize,
        default_case_sensitive: bool,
        groups: &mut Groups,
    ) -> Option<usize> {
        let case_sensitive = if self.case_specified {
            self.case_sensitive
        } else {
            default_case_sensitive
        };
        let mut cur = self.machine.init;
        let mut fin = self.machine.fin;
        if cur == fin {
            return Some(0);
        }
        let mut regs = Saved {
            groups: [(-1, -1); GROUP_COUNT],
            loops: vec![0; self.loop_vars.max(1)],
        };
        for (reg, group) in regs.groups.iter_mut().zip(groups.iter()) {
            if let Some((a, b)) = group {
                *reg = (*a as i64, *b as i64);
            }
        }
        let mut pos = start;
        // Characters are consumed only up to the limit, though tests of
        // the surrounding text see all of it.
        let mut limit = limit.min(text.len());
        let mut stack: Vec<Frame> = Vec::new();

        let end = 'run: loop {
            // Step through states until this path ends: `Some(end)` when
            // it reaches the final state, `None` when it fails.
            let mut result: Option<usize> = 'path: loop {
                if cur == fin {
                    break 'path Some(pos);
                }
                if cur == NONE {
                    break 'path None;
                }
                let tuple = &self.tuples[cur as usize];
                let next = char_at(text, pos);
                let here = if pos < limit { next } else { None };
                // Match one character with a test.
                macro_rules! one {
                    ($test:expr) => {{
                        match here {
                            Some((c, len)) if ($test)(c) => pos += len,
                            _ => break 'path None,
                        }
                    }};
                }
                match &tuple.kind {
                    Kind::Invalid => break 'path None,
                    Kind::ZeroVar(var) => regs.loops[*var] = 0,
                    Kind::LoopBranch { var, min, max } => {
                        let count = regs.loops[*var];
                        if count < *min {
                            regs.loops[*var] += 1;
                            cur = if tuple.shortest {
                                tuple.next2
                            } else {
                                tuple.next1
                            };
                            continue 'path;
                        }
                        if *max >= 0 && count >= *max {
                            cur = if tuple.shortest {
                                tuple.next1
                            } else {
                                tuple.next2
                            };
                            continue 'path;
                        }
                        regs.loops[*var] += 1;
                        stack.push(Frame::First {
                            state: cur,
                            fin,
                            pos,
                            saved: Box::new(regs.clone()),
                        });
                        cur = tuple.next1;
                        continue 'path;
                    }
                    Kind::GroupMatch(group) => {
                        let (a, b) = regs.groups.get(*group).copied().unwrap_or((-1, -1));
                        if a < 0 || b < 0 || b < a {
                            break 'path None;
                        }
                        let wanted = &text[a as usize..b as usize];
                        if limit.saturating_sub(pos) < wanted.len()
                            || &text[pos..pos + wanted.len()] != wanted
                        {
                            break 'path None;
                        }
                        pos += wanted.len();
                    }
                    Kind::TextBegin => {
                        if pos != 0 {
                            break 'path None;
                        }
                    }
                    Kind::TextEnd => {
                        if pos != text.len() {
                            break 'path None;
                        }
                    }
                    Kind::LookbackPos => {
                        let origin = stack.iter().rev().find_map(|frame| match frame {
                            Frame::Assert { pos, .. } => Some(*pos),
                            _ => None,
                        });
                        if origin != Some(pos) {
                            break 'path None;
                        }
                    }
                    Kind::WordBegin => {
                        if char_before(text, pos).is_some_and(is_word_char)
                            || !next.is_some_and(|(c, _)| is_word_char(c))
                        {
                            break 'path None;
                        }
                    }
                    Kind::WordEnd => {
                        if next.is_some_and(|(c, _)| is_word_char(c))
                            || !char_before(text, pos).is_some_and(is_word_char)
                        {
                            break 'path None;
                        }
                    }
                    Kind::WordBoundary | Kind::NonWordBoundary => {
                        let before = char_before(text, pos).is_some_and(is_word_char);
                        let after = next.is_some_and(|(c, _)| is_word_char(c));
                        let boundary = before != after;
                        if boundary != (tuple.kind == Kind::WordBoundary) {
                            break 'path None;
                        }
                    }
                    Kind::WordChar => one!(is_word_char),
                    Kind::NonWordChar => one!(|c| !is_word_char(c)),
                    Kind::Wildcard => one!(|_| true),
                    Kind::Class(class) => one!(|c| in_class(*class, c)),
                    Kind::NonDigit => one!(|c| !is_digit(c)),
                    Kind::NonSpace => one!(|c| !is_space(c)),
                    Kind::NonVSpace => one!(|c| !is_vspace(c)),
                    Kind::Range(items, exclude) => {
                        let Some((c, len)) = here else {
                            break 'path None;
                        };
                        let found = items.iter().any(|item| match *item {
                            RangeItem::Class(class) => in_class(class, c),
                            RangeItem::Chars(lo, hi) if case_sensitive => c >= lo && c <= hi,
                            RangeItem::Chars(lo, hi) if lo == hi => c == lo || fold(c) == fold(lo),
                            RangeItem::Chars(lo, hi) => {
                                let folded = fold(c);
                                folded >= fold(lo) && folded <= fold(hi)
                            }
                        });
                        if found == *exclude {
                            break 'path None;
                        }
                        pos += len;
                    }
                    Kind::Literal(wanted) => match here {
                        Some((c, len))
                            if c == *wanted || (!case_sensitive && fold(c) == fold(*wanted)) =>
                        {
                            pos += len;
                        }
                        _ => break 'path None,
                    },
                    Kind::GroupEnter(group) => {
                        if let Some(reg) = regs.groups.get_mut(*group) {
                            reg.0 = pos as i64;
                        }
                    }
                    Kind::GroupExit(group) => {
                        if let Some(reg) = regs.groups.get_mut(*group) {
                            reg.1 = pos as i64;
                        }
                    }
                    Kind::Assert {
                        back,
                        init,
                        fin: sub_fin,
                        min_len,
                        ..
                    } => {
                        stack.push(Frame::Assert {
                            state: cur,
                            fin,
                            pos,
                            limit,
                            saved: Box::new(regs.clone()),
                            extra: 0,
                        });
                        cur = *init;
                        fin = *sub_fin;
                        limit = text.len();
                        if *back {
                            // Start the sub-pattern as far back as its
                            // shortest possible match.
                            let mut left = *min_len;
                            while left > 0 && pos > 0 {
                                pos = prev_pos(text, pos);
                                left -= 1;
                            }
                            if left != 0 {
                                break 'path None;
                            }
                        }
                        continue 'path;
                    }
                    Kind::Epsilon => {
                        if tuple.next2 != NONE {
                            stack.push(Frame::First {
                                state: cur,
                                fin,
                                pos,
                                saved: Box::new(regs.clone()),
                            });
                        }
                    }
                }
                cur = tuple.next1;
            };

            // The path ended. Hand its result to whatever is waiting.
            loop {
                match stack.pop() {
                    None => break 'run result,
                    Some(Frame::First {
                        state,
                        fin: saved_fin,
                        pos: branch_pos,
                        saved,
                    }) => {
                        // Go back and try the second branch.
                        let first_result = Box::new(std::mem::replace(&mut regs, (*saved).clone()));
                        stack.push(Frame::Second {
                            state,
                            fin: saved_fin,
                            saved,
                            first: result,
                            first_result,
                        });
                        cur = self.tuples[state as usize].next2;
                        fin = saved_fin;
                        pos = branch_pos;
                        continue 'run;
                    }
                    Some(Frame::Second {
                        state,
                        fin: saved_fin,
                        saved,
                        first,
                        first_result,
                    }) => {
                        // The frame this result goes to restores the final state.
                        let _ = saved_fin;
                        let second = result;
                        let first_wins = match (first, second) {
                            (None, None) => {
                                regs = *saved;
                                result = None;
                                continue;
                            }
                            (Some(_), None) => true,
                            (None, Some(_)) => false,
                            (Some(a), Some(b)) => {
                                if self.longest_match && !self.tuples[state as usize].shortest {
                                    a >= b
                                } else {
                                    a <= b
                                }
                            }
                        };
                        if first_wins {
                            regs = *first_result;
                            result = first;
                        }
                    }
                    Some(Frame::Assert {
                        state,
                        fin: saved_fin,
                        pos: origin,
                        limit: saved_limit,
                        saved,
                        extra,
                    }) => {
                        let Kind::Assert {
                            negative,
                            back,
                            init,
                            fin: sub_fin,
                            min_len,
                            max_len,
                        } = &self.tuples[state as usize].kind
                        else {
                            break 'run None;
                        };
                        let (negative, back, init, sub_fin, min_len, max_len) =
                            (*negative, *back, *init, *sub_fin, *min_len, *max_len);
                        // Nothing an assertion matches is kept.
                        regs = (*saved).clone();
                        if result.is_none() && back {
                            // Try again one character further back.
                            let extra = extra + 1;
                            let look = extra + min_len;
                            let available = std::str::from_utf8(&text[..origin])
                                .map_or(origin, |s| s.chars().count());
                            if (max_len < 0 || look <= max_len) && available >= look as usize {
                                stack.push(Frame::Assert {
                                    state,
                                    fin: saved_fin,
                                    pos: origin,
                                    limit: saved_limit,
                                    saved,
                                    extra,
                                });
                                pos = origin;
                                for _ in 0..look {
                                    pos = prev_pos(text, pos);
                                }
                                cur = init;
                                fin = sub_fin;
                                continue 'run;
                            }
                        }
                        fin = saved_fin;
                        pos = origin;
                        limit = saved_limit;
                        if result.is_some() == negative {
                            result = None;
                            continue;
                        }
                        cur = self.tuples[state as usize].next1;
                        continue 'run;
                    }
                }
            }
        };

        let end = end?;
        for (group, reg) in groups.iter_mut().zip(regs.groups.iter()) {
            *group = (reg.0 >= 0 && reg.1 >= reg.0).then_some((reg.0 as usize, reg.1 as usize));
        }
        Some(end.saturating_sub(start))
    }

    /// Search forward for the pattern from a position. Returns the
    /// offset and length of the match.
    pub(crate) fn search(
        &self,
        text: &[u8],
        from: usize,
        default_case_sensitive: bool,
        groups: &mut Groups,
    ) -> Option<(usize, usize)> {
        let mut best: Option<(usize, usize, Groups)> = None;
        let mut limit = text.len();
        let mut pos = from;
        while pos <= limit {
            let mut found = [None; GROUP_COUNT];
            if let Some(len) =
                self.match_at(text, pos, text.len(), default_case_sensitive, &mut found)
            {
                if self.first_begin {
                    *groups = found;
                    return Some((pos, len));
                }
                // The match that ends first wins.
                let keep = match &best {
                    None => true,
                    Some((start, best_len, _)) => match (pos + len).cmp(&(start + best_len)) {
                        std::cmp::Ordering::Less => true,
                        std::cmp::Ordering::Equal => !self.longest_match,
                        std::cmp::Ordering::Greater => false,
                    },
                };
                if keep {
                    best = Some((pos, len, found));
                    limit = pos + len;
                }
            }
            match char_at(text, pos) {
                Some((_, len)) => pos += len,
                None => break,
            }
        }
        let (start, len, found) = best?;
        *groups = found;
        Some((start, len))
    }

    /// Search backward for the pattern: for a match that ends at or
    /// before a position. Returns the offset and length of the match.
    pub(crate) fn search_back(
        &self,
        text: &[u8],
        from: usize,
        default_case_sensitive: bool,
        groups: &mut Groups,
    ) -> Option<(usize, usize)> {
        // A match cannot run past the starting point.
        let from = from.min(text.len());
        let mut best: Option<(usize, usize, Groups)> = None;
        let mut pos = from;
        loop {
            let mut found = [None; GROUP_COUNT];
            if let Some(len) = self.match_at(text, pos, from, default_case_sensitive, &mut found) {
                if !self.first_begin {
                    *groups = found;
                    return Some((pos, len));
                }
                let keep = match &best {
                    None => true,
                    Some((start, best_len, _)) => match (pos + len).cmp(&(start + best_len)) {
                        std::cmp::Ordering::Greater => true,
                        std::cmp::Ordering::Equal => self.longest_match,
                        std::cmp::Ordering::Less => false,
                    },
                };
                if keep {
                    best = Some((pos, len, found));
                }
            }
            if pos == 0 {
                break;
            }
            pos = prev_pos(text, pos);
        }
        let (start, len, found) = best?;
        *groups = found;
        Some((start, len))
    }
}
