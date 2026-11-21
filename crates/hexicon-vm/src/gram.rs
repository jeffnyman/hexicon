//! The GrammarProduction intrinsic class: a rule of a game's grammar,
//! and the parser that matches a list of tokens against it.
//!
//! A production has alternatives, each a sequence of things to match: a
//! literal word, a word that is some part of speech in the dictionary, a
//! token of some type, another production, or `*` for whatever is left.
//! Parsing returns every way the whole token list can be matched, each
//! as a tree of objects the program's classes describe.
//!
//! The parser is the reference VM's, step for step: the order of the
//! results, and of the objects created for them, comes from the order in
//! which it explores the alternatives.

use std::rc::Rc;

use crate::error::{VmError, VmResult, code};
use crate::object::Object;
use crate::value::{INVALID_PROP, NativeDesc, ObjId, PropId, Value};
use crate::vm::Vm;

/// What one item of an alternative matches.
#[derive(Debug, Clone)]
pub(crate) enum TokenMatch {
    /// Nothing: an item the compiler could not resolve.
    Undefined,
    /// Another production.
    Production(ObjId),
    /// A word the dictionary lists under a vocabulary property.
    Speech(PropId),
    /// A word the dictionary lists under any of several.
    AnySpeech(Vec<PropId>),
    /// A particular word.
    Literal(Vec<u8>),
    /// A token of a type, given as an enumerator.
    TokenType(u32),
    /// All the tokens that are left.
    Star,
}

impl TokenMatch {
    /// The type code the image format and `getGrammarInfo` use.
    fn code(&self) -> u8 {
        match self {
            Self::Undefined => 0,
            Self::Production(_) => 1,
            Self::Speech(_) => 2,
            Self::Literal(_) => 3,
            Self::TokenType(_) => 4,
            Self::Star => 5,
            Self::AnySpeech(_) => 6,
        }
    }
}

/// One item of an alternative.
#[derive(Debug, Clone)]
pub(crate) struct AltToken {
    /// The property of the match object that gets what this item
    /// matched, or the invalid property.
    pub prop: PropId,
    pub kind: TokenMatch,
}

/// One alternative of a production.
#[derive(Debug, Clone)]
pub(crate) struct Alt {
    pub score: i16,
    /// Alternatives with badness are tried only when nothing better
    /// matches, the least bad first.
    pub badness: i16,
    /// The class of the object a match creates.
    pub match_class: ObjId,
    pub tokens: Vec<AltToken>,
}

/// The hash of each item of each alternative that is a literal.
type LiteralHashes = Rc<Vec<Vec<u32>>>;

/// A grammar production.
#[derive(Debug, Clone, Default)]
pub(crate) struct GramProd {
    pub alts: Rc<Vec<Alt>>,
    /// The production has changed since it was loaded.
    pub modified: bool,
    /// The hash of each literal in each alternative under one
    /// comparator, kept from parse to parse.
    hashes: Option<(ObjId, LiteralHashes)>,
}

impl GramProd {
    /// Read a production's alternatives from image or saved data.
    /// Returns the production and how many bytes it took.
    pub(crate) fn from_bytes(data: &[u8]) -> Option<(Self, usize)> {
        let mut pos = 0;
        let u16_next = |pos: &mut usize| {
            let b = data.get(*pos..*pos + 2)?;
            *pos += 2;
            Some(u16::from_le_bytes([b[0], b[1]]))
        };
        let u32_next = |pos: &mut usize| {
            let b = data.get(*pos..*pos + 4)?;
            *pos += 4;
            Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };
        let alt_count = u16_next(&mut pos)?;
        let mut alts = Vec::with_capacity(alt_count.into());
        for _ in 0..alt_count {
            let score = u16_next(&mut pos)? as i16;
            let badness = u16_next(&mut pos)? as i16;
            let match_class = ObjId(u32_next(&mut pos)?);
            let token_count = u16_next(&mut pos)?;
            let mut tokens = Vec::with_capacity(token_count.into());
            for _ in 0..token_count {
                let prop = u16_next(&mut pos)?;
                let code = *data.get(pos)?;
                pos += 1;
                let kind = match code {
                    1 => TokenMatch::Production(ObjId(u32_next(&mut pos)?)),
                    2 => TokenMatch::Speech(u16_next(&mut pos)?),
                    6 => {
                        let count = u16_next(&mut pos)?;
                        let mut props = Vec::with_capacity(count.into());
                        for _ in 0..count {
                            props.push(u16_next(&mut pos)?);
                        }
                        TokenMatch::AnySpeech(props)
                    }
                    3 => {
                        let len = usize::from(u16_next(&mut pos)?);
                        let text = data.get(pos..pos + len)?.to_vec();
                        pos += len;
                        TokenMatch::Literal(text)
                    }
                    4 => TokenMatch::TokenType(u32_next(&mut pos)?),
                    5 => TokenMatch::Star,
                    _ => TokenMatch::Undefined,
                };
                tokens.push(AltToken { prop, kind });
            }
            alts.push(Alt {
                score,
                badness,
                match_class,
                tokens,
            });
        }
        Some((
            Self {
                alts: Rc::new(alts),
                modified: false,
                hashes: None,
            },
            pos,
        ))
    }

    /// The alternatives as [`GramProd::from_bytes`] reads them.
    pub(crate) fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let u16_put = |out: &mut Vec<u8>, n: u16| out.extend_from_slice(&n.to_le_bytes());
        u16_put(&mut out, self.alts.len() as u16);
        for alt in self.alts.iter() {
            u16_put(&mut out, alt.score as u16);
            u16_put(&mut out, alt.badness as u16);
            out.extend_from_slice(&alt.match_class.0.to_le_bytes());
            u16_put(&mut out, alt.tokens.len() as u16);
            for token in &alt.tokens {
                u16_put(&mut out, token.prop);
                out.push(token.kind.code());
                match &token.kind {
                    TokenMatch::Production(id) => out.extend_from_slice(&id.0.to_le_bytes()),
                    TokenMatch::Speech(prop) => u16_put(&mut out, *prop),
                    TokenMatch::AnySpeech(props) => {
                        u16_put(&mut out, props.len() as u16);
                        for &prop in props {
                            u16_put(&mut out, prop);
                        }
                    }
                    TokenMatch::Literal(text) => {
                        u16_put(&mut out, text.len() as u16);
                        out.extend_from_slice(text);
                    }
                    TokenMatch::TokenType(id) => out.extend_from_slice(&id.to_le_bytes()),
                    TokenMatch::Star | TokenMatch::Undefined => {}
                }
            }
        }
        out
    }

    /// The objects the alternatives refer to: match classes and other
    /// productions.
    pub(crate) fn references(&self, out: &mut Vec<Value>) {
        for alt in self.alts.iter() {
            out.push(Value::Obj(alt.match_class));
            for token in &alt.tokens {
                if let TokenMatch::Production(id) = token.kind {
                    out.push(Value::Obj(id));
                }
            }
        }
    }

    /// Replace the object IDs in the alternatives, after a restore.
    pub(crate) fn fix_ids(&mut self, fix: &dyn Fn(ObjId) -> ObjId) {
        for alt in Rc::make_mut(&mut self.alts) {
            alt.match_class = fix(alt.match_class);
            for token in &mut alt.tokens {
                if let TokenMatch::Production(id) = &mut token.kind {
                    *id = fix(*id);
                }
            }
        }
    }

    /// Forget hashes worked out for a comparator that no longer exists.
    pub(crate) fn forget_comparator(&mut self, alive: &dyn Fn(ObjId) -> bool) {
        if self
            .hashes
            .as_ref()
            .is_some_and(|(id, _)| id.is_valid() && !alive(*id))
        {
            self.hashes = None;
        }
    }
}

/// An input token, as the parser sees it.
struct Token {
    value: Value,
    /// The token type's enumerator, or zero.
    kind: u32,
    text: Option<Vec<u8>>,
    hash: u32,
    /// The vocabulary properties the dictionary lists the word under.
    props: Vec<PropId>,
}

/// Something matched: a token, or a whole alternative with what each of
/// its items matched.
#[derive(Clone)]
struct Match {
    target_prop: PropId,
    token_pos: usize,
    /// What the comparator said about a literal.
    result: Value,
    star: bool,
    /// The class to make the match object from, for an alternative.
    match_class: ObjId,
    subs: Vec<usize>,
}

/// A place in the grammar the parser has reached: partway through an
/// alternative, inside whatever led to it.
#[derive(Clone)]
struct State {
    token_pos: usize,
    matched_star: bool,
    /// How many items of the alternative are matched.
    alt_pos: usize,
    enclosing: Option<usize>,
    alts: Rc<Vec<Alt>>,
    alt: usize,
    production: ObjId,
    /// Where the enclosing state wants what its current item matches.
    sub_target_prop: PropId,
    /// The production has an alternative that begins with itself.
    circular: bool,
    matches: Vec<usize>,
}

/// The state of one parse.
struct Parser {
    tokens: Vec<Token>,
    dict: Option<ObjId>,
    states: Vec<State>,
    matches: Vec<Match>,
    /// States to work on, the last first.
    work: Vec<usize>,
    /// States for alternatives with badness, the last first.
    bad: Vec<usize>,
    /// Complete matches, with how many tokens each used.
    successes: Vec<(usize, usize)>,
}

impl Parser {
    fn new_match(&mut self, m: Match) -> usize {
        self.matches.push(m);
        self.matches.len() - 1
    }

    /// Copy a state and everything it is inside of, so that two ways of
    /// going on from here do not share them.
    fn clone_state(&mut self, index: usize) -> usize {
        let mut state = self.states[index].clone();
        state.enclosing = state.enclosing.map(|outer| self.clone_state(outer));
        state.matches.truncate(state.alt_pos);
        self.states.push(state);
        self.states.len() - 1
    }
}

/// The name and calling interface of each method, by method number.
const METHODS: &[(&str, NativeDesc)] = &[
    ("", NativeDesc::fixed(0)),
    ("parseTokens", NativeDesc::fixed(2)),
    ("getGrammarInfo", NativeDesc::fixed(0)),
    ("addAlt", NativeDesc::optional(2, 2)),
    ("deleteAlt", NativeDesc::optional(1, 1)),
    ("clearAlts", NativeDesc::optional(0, 1)),
];

impl Vm {
    fn gram(&self, id: ObjId) -> VmResult<&GramProd> {
        match self.objects.get(id) {
            Some(Object::GramProd(production)) => Ok(production),
            _ => Err(VmError::new(code::INVAL_OBJ_TYPE)),
        }
    }

    fn gram_mut(&mut self, id: ObjId) -> VmResult<&mut GramProd> {
        match self.objects.get_mut(id) {
            Some(Object::GramProd(production)) => Ok(production),
            _ => Err(VmError::new(code::INVAL_OBJ_TYPE)),
        }
    }

    /// `new GrammarProd()`.
    pub(crate) fn create_gram(&mut self, argc: u32) -> VmResult<ObjId> {
        if argc != 0 {
            return Err(VmError::new(code::WRONG_NUM_OF_ARGS));
        }
        let production = GramProd {
            modified: true,
            ..GramProd::default()
        };
        Ok(self.alloc_obj(Object::GramProd(Box::new(production))))
    }

    pub(crate) fn gram_method(
        &mut self,
        id: ObjId,
        index: u16,
        argc: Option<&mut u32>,
    ) -> VmResult<Option<Value>> {
        let Some(&(_, desc)) = METHODS.get(index as usize).filter(|_| index != 0) else {
            return Ok(None);
        };
        self.native_method(argc, desc, |vm, argc| match index {
            1 => vm.gram_parse(id, argc),
            2 => vm.gram_info(id),
            3 => Err(VmError::unimplemented(
                "GrammarProd.addAlt, which compiles a grammar rule",
            )),
            _ => vm.gram_delete(id, index, argc),
        })
        .map(Some)
    }

    /// The alternatives of a production and the hashes of their
    /// literals under the dictionary's comparator.
    fn gram_alts(
        &mut self,
        id: ObjId,
        dict: Option<ObjId>,
    ) -> VmResult<(Rc<Vec<Alt>>, LiteralHashes)> {
        let comparator = match dict {
            Some(dict) => self.dict_comparator(dict)?,
            None => ObjId::INVALID,
        };
        let production = self.gram(id)?;
        let alts = production.alts.clone();
        if let Some((cached_for, hashes)) = &production.hashes {
            if *cached_for == comparator {
                return Ok((alts, hashes.clone()));
            }
        }
        let mut hashes = Vec::with_capacity(alts.len());
        for alt in alts.iter() {
            let mut row = Vec::with_capacity(alt.tokens.len());
            for token in &alt.tokens {
                row.push(match &token.kind {
                    TokenMatch::Literal(text) => self.dict_word_hash(dict, None, text)?,
                    _ => 0,
                });
            }
            hashes.push(row);
        }
        let hashes = Rc::new(hashes);
        self.gram_mut(id)?.hashes = Some((comparator, hashes.clone()));
        Ok((alts, hashes))
    }

    /// Whether an input token matches a literal.
    fn token_is_literal(
        &mut self,
        parser: &Parser,
        token: usize,
        literal: &[u8],
    ) -> VmResult<Option<Value>> {
        let token = &parser.tokens[token];
        let Some(text) = &token.text else {
            return Ok(None);
        };
        match parser.dict {
            Some(dict) => self.dict_match(dict, token.value, text, literal),
            None => Ok((text.as_slice() == literal).then_some(Value::Int(1))),
        }
    }

    /// Queue the alternatives of a production that could match from a
    /// token position. Each is first checked against the tokens ahead,
    /// as far as that can be done without parsing, to leave out the ones
    /// that cannot match.
    ///
    /// With `circular`, only the alternatives that begin with the
    /// production itself are queued, with that first item already
    /// matched by `circular`: this is how a left-recursive rule, such as
    /// a list of things joined by "and", grows from what it has matched
    /// so far.
    fn enqueue_alts(
        &mut self,
        parser: &mut Parser,
        production: ObjId,
        start: usize,
        enclosing: Option<usize>,
        circular: Option<usize>,
    ) -> VmResult<()> {
        let (alts, hashes) = self.gram_alts(production, parser.dict)?;
        let has_circular = alts.iter().any(|alt| {
            matches!(alt.tokens.first(), Some(AltToken { kind: TokenMatch::Production(id), .. }) if *id == production)
        });
        let circular_only = circular.is_some();
        let mut need_to_clone = circular_only;
        let token_count = parser.tokens.len();
        for (alt_index, alt) in alts.iter().enumerate() {
            let is_circular = matches!(
                alt.tokens.first(),
                Some(AltToken { kind: TokenMatch::Production(id), .. }) if *id == production
            );
            let mut token_index = start;
            let mut mismatch = false;
            // Once a production comes before an item, where the item
            // starts is unknown, so it is looked for anywhere ahead.
            let mut production_before = false;
            for (item_index, item) in alt.tokens.iter().enumerate() {
                let mut matched = false;
                let mut matched_star = false;
                let mut ignore = false;
                loop {
                    match &item.kind {
                        TokenMatch::Production(_) => {
                            if !(is_circular && circular_only && item_index == 0) {
                                production_before = true;
                            }
                            ignore = true;
                        }
                        TokenMatch::Speech(prop) => {
                            matched = token_index < token_count
                                && parser.tokens[token_index].props.contains(prop);
                        }
                        TokenMatch::AnySpeech(props) => {
                            matched = token_index < token_count
                                && props
                                    .iter()
                                    .any(|prop| parser.tokens[token_index].props.contains(prop));
                        }
                        TokenMatch::Literal(text) => {
                            matched = token_index < token_count
                                && parser.tokens[token_index].text.is_some()
                                && hashes[alt_index][item_index] == parser.tokens[token_index].hash
                                && self.token_is_literal(parser, token_index, text)?.is_some();
                        }
                        TokenMatch::TokenType(kind) => {
                            matched = token_index < token_count
                                && parser.tokens[token_index].kind == *kind;
                        }
                        TokenMatch::Star => {
                            token_index = token_count;
                            matched = true;
                            matched_star = true;
                        }
                        TokenMatch::Undefined => {}
                    }
                    if matched || !production_before || ignore || token_index >= token_count {
                        break;
                    }
                    token_index += 1;
                }
                if matched {
                    if !matched_star && token_index < token_count {
                        token_index += 1;
                    }
                } else if !ignore {
                    mismatch = true;
                    break;
                }
            }
            if mismatch || circular_only != is_circular {
                continue;
            }

            // Every alternative after the first needs its own copy of
            // what it is inside of.
            let outer = match enclosing {
                Some(outer) if need_to_clone => Some(parser.clone_state(outer)),
                other => other,
            };
            need_to_clone = true;
            let mut state = State {
                token_pos: start,
                matched_star: false,
                alt_pos: 0,
                enclosing: outer,
                alts: alts.clone(),
                alt: alt_index,
                production,
                sub_target_prop: INVALID_PROP,
                circular: has_circular,
                matches: Vec::new(),
            };
            if let Some(circular) = circular {
                let first = parser.matches[circular].clone();
                let matched = parser.new_match(Match {
                    target_prop: alt.tokens[0].prop,
                    token_pos: 0,
                    result: Value::Nil,
                    star: false,
                    match_class: first.match_class,
                    subs: first.subs,
                });
                state.matches.push(matched);
                state.alt_pos = 1;
            }
            parser.states.push(state);
            let index = parser.states.len() - 1;
            if alt.badness != 0 {
                parser.bad.push(index);
            } else {
                parser.work.push(index);
            }
        }
        Ok(())
    }

    /// Take the next state and match as much of its alternative as
    /// possible.
    fn process_state(&mut self, parser: &mut Parser, index: usize) -> VmResult<()> {
        let token_count = parser.tokens.len();
        // Once `*` has matched, the reference marks every later item
        // matched in the same pass as part of it.
        let mut star = false;
        loop {
            let state = &parser.states[index];
            let alt = &state.alts[state.alt];
            let Some(item) = alt.tokens.get(state.alt_pos) else {
                break;
            };
            let position = state.token_pos;
            let mut result = Value::Nil;
            let matched = match &item.kind {
                TokenMatch::Production(sub) => {
                    // Go into the other production. This state waits
                    // until an alternative of it finishes.
                    let (sub, prop) = (*sub, item.prop);
                    if !matches!(self.objects.get(sub), Some(Object::GramProd(_))) {
                        return Err(VmError::new(code::INVAL_OBJ_TYPE));
                    }
                    parser.states[index].sub_target_prop = prop;
                    return self.enqueue_alts(parser, sub, position, Some(index), None);
                }
                TokenMatch::Speech(prop) => {
                    position < token_count && parser.tokens[position].props.contains(prop)
                }
                TokenMatch::AnySpeech(props) => {
                    position < token_count
                        && props
                            .iter()
                            .any(|prop| parser.tokens[position].props.contains(prop))
                }
                TokenMatch::Literal(text) => {
                    let text = text.clone();
                    let (alt_index, production) = (state.alt, state.production);
                    let alt_pos = state.alt_pos;
                    let (_, hashes) = self.gram_alts(production, parser.dict)?;
                    let same_hash = position < token_count
                        && hashes
                            .get(alt_index)
                            .and_then(|row| row.get(alt_pos))
                            .is_some_and(|&hash| hash == parser.tokens[position].hash);
                    match same_hash {
                        true => match self.token_is_literal(parser, position, &text)? {
                            Some(value) => {
                                result = value;
                                true
                            }
                            None => false,
                        },
                        false => false,
                    }
                }
                TokenMatch::TokenType(kind) => {
                    position < token_count && parser.tokens[position].kind == *kind
                }
                TokenMatch::Star => {
                    star = true;
                    true
                }
                TokenMatch::Undefined => false,
            };
            if !matched {
                return Ok(());
            }
            let prop = parser.states[index].alts[parser.states[index].alt].tokens
                [parser.states[index].alt_pos]
                .prop;
            let matched = parser.new_match(Match {
                target_prop: prop,
                token_pos: position,
                result,
                star,
                match_class: ObjId::INVALID,
                subs: Vec::new(),
            });
            let state = &mut parser.states[index];
            if star {
                state.matched_star = true;
            }
            state.matches.push(matched);
            state.alt_pos += 1;
            if !state.matched_star {
                state.token_pos += 1;
            }
        }

        // The whole alternative matched.
        let state = parser.states[index].clone();
        let alt = &state.alts[state.alt];
        let target_prop = state
            .enclosing
            .map_or(INVALID_PROP, |outer| parser.states[outer].sub_target_prop);
        let matched = parser.new_match(Match {
            target_prop,
            token_pos: state.token_pos,
            result: Value::Nil,
            star: false,
            match_class: alt.match_class,
            subs: state.matches.clone(),
        });
        // A left-recursive alternative of the same production can take
        // this match as its first item and go on.
        if state.circular {
            if !matches!(
                self.objects.get(state.production),
                Some(Object::GramProd(_))
            ) {
                return Err(VmError::new(code::INVAL_OBJ_TYPE));
            }
            self.enqueue_alts(
                parser,
                state.production,
                state.token_pos,
                state.enclosing,
                Some(matched),
            )?;
        }
        match state.enclosing {
            Some(outer) => {
                let outer_state = &mut parser.states[outer];
                outer_state.matches.truncate(outer_state.alt_pos);
                outer_state.matches.push(matched);
                outer_state.alt_pos += 1;
                outer_state.token_pos = state.token_pos;
                outer_state.matched_star = state.matched_star;
                parser.work.push(outer);
            }
            None => {
                // At the top, a match has to use every token.
                if state.token_pos >= token_count || state.matched_star {
                    parser.successes.push((matched, state.token_pos));
                }
            }
        }
        Ok(())
    }

    /// `parseTokens(tokens, dict)`: every way the tokens match the
    /// production, each as a tree of match objects.
    fn gram_parse(&mut self, id: ObjId, argc: u32) -> VmResult<Value> {
        let token_list = self.arg(0);
        let Some(token_count) = self.list_like_len(token_list)? else {
            return Err(VmError::new(code::LIST_VAL_REQD));
        };
        let dict = match if argc >= 2 { self.arg(1) } else { Value::Nil } {
            Value::Nil => None,
            Value::Obj(dict) => {
                if !matches!(self.objects.get(dict), Some(Object::Dict(_))) {
                    return Err(VmError::new(code::INVAL_OBJ_TYPE));
                }
                Some(dict)
            }
            _ => return Err(VmError::new(code::OBJ_VAL_REQD)),
        };

        // Each token is a list: its text, its type, and anything else
        // the tokenizer added.
        let mut tokens = Vec::with_capacity(token_count);
        for index in 1..=token_count {
            let element = self.list_like_get(token_list, index)?;
            if self.list_like_len(element)?.is_none() {
                return Err(VmError::new(code::BAD_TYPE_BIF));
            }
            let value = self.list_like_get(element, 1)?;
            let kind = match self.list_like_get(element, 2)? {
                Value::Enum(kind) => kind,
                _ => 0,
            };
            let text = self.as_str(value).map(|text| text.to_vec());
            let (hash, props) = match &text {
                Some(text) => {
                    let hash = self.dict_word_hash(dict, Some(value), text)?;
                    let props = match dict {
                        Some(dict) => self.dict_word_props(dict, value, text)?,
                        None => Vec::new(),
                    };
                    (hash, props)
                }
                None => (0, Vec::new()),
            };
            tokens.push(Token {
                value,
                kind,
                text,
                hash,
                props,
            });
        }

        let mut parser = Parser {
            tokens,
            dict,
            states: Vec::new(),
            matches: Vec::new(),
            work: Vec::new(),
            bad: Vec::new(),
            successes: Vec::new(),
        };
        self.enqueue_alts(&mut parser, id, 0, None, None)?;
        loop {
            // With nothing left to try and nothing found, fall back on
            // the least bad of the alternatives set aside.
            if parser.work.is_empty() && parser.successes.is_empty() {
                let least = parser
                    .bad
                    .iter()
                    .map(|&state| {
                        let state = &parser.states[state];
                        state.alts[state.alt].badness
                    })
                    .min();
                if let Some(least) = least {
                    // They are taken newest first, and each goes to the
                    // front of the work, so the oldest is worked first.
                    let mut kept = Vec::new();
                    for &state in parser.bad.clone().iter().rev() {
                        let s = &parser.states[state];
                        if s.alts[s.alt].badness == least {
                            parser.work.push(state);
                        } else {
                            kept.push(state);
                        }
                    }
                    kept.reverse();
                    parser.bad = kept;
                }
            }
            let Some(state) = parser.work.pop() else {
                break;
            };
            self.process_state(&mut parser, state)?;
        }

        // Build the trees, the last match found first.
        let result = self.new_list(Vec::new());
        self.push(result);
        let mut trees = Vec::with_capacity(parser.successes.len());
        let mut failed = None;
        for &(matched, used) in parser.successes.iter().rev() {
            let token_matches = self.new_list(vec![Value::Nil; used]);
            self.push(token_matches);
            let tree = self.build_match_tree(&parser, matched, token_list, token_matches);
            self.discard(1);
            match tree {
                Ok((tree, _, _)) => trees.push(tree),
                Err(error) => {
                    failed = Some(error);
                    break;
                }
            }
            if let Some(Object::List(stored)) = self.objects.get_mut(result.obj_id()) {
                *stored = trees.clone().into();
            }
        }
        self.discard(1);
        match failed {
            Some(error) => Err(error),
            None => Ok(result),
        }
    }

    /// Make the object for a match and everything under it. Returns the
    /// value for the match and the first and last token it covers; a
    /// match of no tokens has a first past its last.
    fn build_match_tree(
        &mut self,
        parser: &Parser,
        index: usize,
        token_list: Value,
        token_matches: Value,
    ) -> VmResult<(Value, usize, usize)> {
        let matched = &parser.matches[index];
        if !matched.match_class.is_valid() {
            // A single token: its text.
            let token_count = self.list_like_len(token_list)?.unwrap_or(0);
            let value = if matched.token_pos < token_count {
                let element = self.list_like_get(token_list, matched.token_pos + 1)?;
                let value = self.list_like_get(element, 1)?;
                if !matched.star {
                    if let Some(Object::List(items)) = self.objects.get_mut(token_matches.obj_id())
                    {
                        let mut updated = items.to_vec();
                        if let Some(slot) = updated.get_mut(matched.token_pos) {
                            *slot = matched.result;
                        }
                        *items = updated.into();
                    }
                }
                value
            } else {
                Value::Nil
            };
            // `*` covers no tokens of its own.
            return Ok(if matched.star {
                (value, matched.token_pos + 1, matched.token_pos)
            } else {
                (value, matched.token_pos, matched.token_pos)
            });
        }

        // An alternative: an instance of its match class, with a
        // property for each item that names one.
        self.create_instance(matched.match_class, 0)?;
        let object = self.r0;
        let Value::Obj(id) = object else {
            return Err(VmError::new(code::OBJ_VAL_REQD));
        };
        self.push(object);
        let mut first = matched.token_pos + 1;
        let mut last = matched.token_pos;
        let mut failed = None;
        for (position, &sub) in matched.subs.iter().enumerate() {
            let (value, sub_first, sub_last) =
                match self.build_match_tree(parser, sub, token_list, token_matches) {
                    Ok(built) => built,
                    Err(error) => {
                        failed = Some(error);
                        break;
                    }
                };
            if position == 0 {
                first = sub_first;
                last = sub_last;
            } else if sub_first <= sub_last {
                first = first.min(sub_first);
                last = last.max(sub_last);
            }
            let prop = parser.matches[sub].target_prop;
            if prop != INVALID_PROP {
                self.set_prop(id, prop, value)?;
            }
        }
        self.discard(1);
        if let Some(error) = failed {
            return Err(error);
        }
        let predef = &self.predef;
        let (first_prop, last_prop) = (predef.gram_first_token, predef.gram_last_token);
        let (list_prop, match_list_prop) = (predef.gram_token_list, predef.gram_token_match_list);
        if first_prop != INVALID_PROP && last_prop != INVALID_PROP {
            self.set_prop(id, first_prop, Value::Int(first as i32 + 1))?;
            self.set_prop(id, last_prop, Value::Int(last as i32 + 1))?;
        }
        if list_prop != INVALID_PROP {
            self.set_prop(id, list_prop, token_list)?;
        }
        if match_list_prop != INVALID_PROP {
            self.set_prop(id, match_list_prop, token_matches)?;
        }
        Ok((object, first, last))
    }

    /// `getGrammarInfo()`: the alternatives, as objects of the classes
    /// the program names for the purpose.
    fn gram_info(&mut self, id: ObjId) -> VmResult<Value> {
        let alts = self.gram(id)?.alts.clone();
        let alt_class = self.predef.gram_alt_info;
        let token_class = self.predef.gram_alt_token_info;
        let result = self.new_list(Vec::new());
        self.push(result);
        let outcome = (|| -> VmResult<()> {
            let mut alt_objects = Vec::with_capacity(alts.len());
            for alt in alts.iter() {
                let alt_object = if !alt_class.is_valid() {
                    Value::Nil
                } else {
                    let token_list = self.new_list(Vec::new());
                    self.push(token_list);
                    let mut token_objects = Vec::with_capacity(alt.tokens.len());
                    for token in &alt.tokens {
                        let token_object = if !token_class.is_valid() {
                            Value::Nil
                        } else {
                            let detail = match &token.kind {
                                TokenMatch::Production(id) => Value::Obj(*id),
                                TokenMatch::Speech(prop) => Value::Prop(*prop),
                                TokenMatch::AnySpeech(props) => self.new_list(
                                    props.iter().map(|&p| Value::Prop(p)).collect::<Vec<_>>(),
                                ),
                                TokenMatch::Literal(text) => self.new_string(text.clone()),
                                TokenMatch::TokenType(kind) => Value::Enum(*kind),
                                TokenMatch::Star | TokenMatch::Undefined => Value::Nil,
                            };
                            self.push(detail);
                            self.push(Value::Int(token.kind.code().into()));
                            self.push(if token.prop != INVALID_PROP {
                                Value::Prop(token.prop)
                            } else {
                                Value::Nil
                            });
                            self.create_instance(token_class, 3)?;
                            self.r0
                        };
                        token_objects.push(token_object);
                        if let Some(Object::List(stored)) =
                            self.objects.get_mut(token_list.obj_id())
                        {
                            *stored = token_objects.clone().into();
                        }
                    }
                    self.push(token_list);
                    self.push(Value::obj_or_nil(alt.match_class));
                    self.push(Value::Int(alt.badness.into()));
                    self.push(Value::Int(alt.score.into()));
                    self.create_instance(alt_class, 4)?;
                    let alt_object = self.r0;
                    self.discard(1);
                    alt_object
                };
                alt_objects.push(alt_object);
                if let Some(Object::List(stored)) = self.objects.get_mut(result.obj_id()) {
                    *stored = alt_objects.clone().into();
                }
            }
            Ok(())
        })();
        self.discard(1);
        outcome.map(|()| result)
    }

    /// `deleteAlt(id, dict?)` and `clearAlts(dict?)`: remove
    /// alternatives, chosen by position, by the tag of their match
    /// class, or by match class.
    fn gram_delete(&mut self, id: ObjId, index: u16, argc: u32) -> VmResult<Value> {
        let alts = self.gram(id)?.alts.clone();
        let (selector, dict_arg) = if index == 4 {
            (
                Some(self.arg(0)),
                if argc >= 2 { self.arg(1) } else { Value::Nil },
            )
        } else {
            (None, if argc >= 1 { self.arg(0) } else { Value::Nil })
        };
        let dict = match dict_arg {
            Value::Nil => None,
            Value::Obj(dict) if matches!(self.objects.get(dict), Some(Object::Dict(_))) => {
                Some(dict)
            }
            _ => return Err(VmError::new(code::BAD_TYPE_BIF)),
        };

        let mut marked = vec![false; alts.len()];
        match selector {
            None => marked.fill(true),
            Some(Value::Int(n)) => {
                if n < 1 || n as usize > alts.len() {
                    return Err(VmError::new(code::INDEX_OUT_OF_RANGE));
                }
                marked[n as usize - 1] = true;
            }
            Some(selector) => {
                if let Some(tag) = self.as_str(selector) {
                    let tag = tag.to_vec();
                    let tag_prop = self.predef.gram_tag;
                    for position in (0..alts.len()).rev() {
                        let class = alts[position].match_class;
                        if !class.is_valid() || tag_prop == INVALID_PROP {
                            continue;
                        }
                        let native =
                            self.native_call("GrammarProd.deleteAlt", Value::Obj(id), 4, argc);
                        let value = self.call_prop(Value::Obj(class), tag_prop, 0, Some(native))?;
                        if self.as_str(value).is_some_and(|found| *found == *tag) {
                            marked[position] = true;
                        }
                    }
                } else if let Value::Obj(wanted) = selector {
                    for (position, alt) in alts.iter().enumerate() {
                        let class = alt.match_class;
                        if class.is_valid()
                            && (class == wanted || self.is_instance_of(class, wanted))
                        {
                            marked[position] = true;
                        }
                    }
                } else {
                    return Err(VmError::new(code::BAD_VAL_BIF));
                }
            }
        }

        // Take the words of the deleted alternatives out of the
        // dictionary, where no remaining alternative uses them.
        let misc_vocab = self.predef.gram_misc_vocab;
        if let (Some(dict), true) = (dict, misc_vocab != INVALID_PROP) {
            for (position, alt) in alts.iter().enumerate() {
                if !marked[position] {
                    continue;
                }
                for token in &alt.tokens {
                    let TokenMatch::Literal(text) = &token.kind else {
                        continue;
                    };
                    let still_used = alts.iter().enumerate().any(|(other, alt)| {
                        !marked[other]
                            && alt.tokens.iter().any(
                                |t| matches!(&t.kind, TokenMatch::Literal(used) if used == text),
                            )
                    });
                    if !still_used {
                        self.dict_remove_word(dict, text, id, misc_vocab)?;
                    }
                }
            }
        }

        // Delete from the end, bringing each match class's list of the
        // properties its alternatives fill up to date as its last
        // marked alternative goes.
        let mut remaining: Vec<Alt> = alts.to_vec();
        for position in (0..remaining.len()).rev() {
            if !marked[position] {
                continue;
            }
            marked[position] = false;
            let class = remaining[position].match_class;
            let more_to_come = (0..position)
                .any(|earlier| marked[earlier] && remaining[earlier].match_class == class);
            if !more_to_come && class.is_valid() {
                self.build_alt_props(&remaining, &marked, class)?;
            }
            remaining.remove(position);
            marked.remove(position);
        }
        let production = self.gram_mut(id)?;
        production.alts = Rc::new(remaining);
        production.modified = true;
        production.hashes = None;
        Ok(Value::Obj(id))
    }

    /// Set a match class's list of the properties its alternatives
    /// fill.
    fn build_alt_props(&mut self, alts: &[Alt], marked: &[bool], class: ObjId) -> VmResult<()> {
        let prop = self.predef.gram_alt_props;
        if prop == INVALID_PROP {
            return Ok(());
        }
        let mut props: Vec<Value> = Vec::new();
        for (position, alt) in alts.iter().enumerate() {
            if marked[position] || alt.match_class != class {
                continue;
            }
            for token in &alt.tokens {
                let value = Value::Prop(token.prop);
                if token.prop != INVALID_PROP && !props.contains(&value) {
                    props.push(value);
                }
            }
        }
        let list = self.new_list(props);
        self.set_prop(class, prop, list)
    }
}
