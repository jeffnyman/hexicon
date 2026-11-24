//! The Dictionary built-in class: the words a game's parser knows, each
//! with the objects it can refer to and the part of speech it is for
//! each.
//!
//! A dictionary finds words through a comparator, which decides what
//! matches what: usually a StringComparator, but it can be any object
//! with `calcHash` and `matchValues` methods, or nothing, for exact
//! matching.
//!
//! The order in which a dictionary returns matches and goes through its
//! words comes from how the reference VM stores them: 256 hash buckets,
//! each a chain with the newest word first. A program can see that
//! order, so the storage here is the same.

use std::rc::Rc;

use crate::error::{VmError, VmResult, code};
use crate::object::Object;
use crate::strcomp::StrComp;
use crate::value::{INVALID_PROP, NativeDesc, ObjId, PropId, Value};
use crate::vm::{NativeCall, Vm};

const BUCKETS: usize = 256;

/// A word and what it refers to.
#[derive(Debug, Clone)]
pub(crate) struct Word {
    pub text: Vec<u8>,
    /// Each object the word refers to, with the vocabulary property
    /// that says what part of speech the word is for it.
    pub entries: Vec<(ObjId, PropId)>,
}

/// A node of the tree of words used for spelling correction: each path
/// from the root spells the start of a word.
#[derive(Debug, Clone)]
struct TrieNode {
    ch: char,
    /// The first node one character further on, and the next node with
    /// the same parent.
    child: Option<usize>,
    next: Option<usize>,
    /// How many words end here.
    words: u32,
}

/// A dictionary.
#[derive(Debug, Clone)]
pub(crate) struct Dictionary {
    /// The comparator, or the invalid ID for exact matching.
    pub comparator: ObjId,
    /// The words, by hash. Within a bucket the newest word is first.
    buckets: Vec<Vec<Word>>,
    /// The tree for spelling correction, once it has been asked for.
    trie: Option<Vec<TrieNode>>,
    /// The dictionary has changed since it was loaded.
    pub modified: bool,
}

/// The hash with no comparator: the sum of the bytes.
fn plain_hash(text: &[u8]) -> u32 {
    text.iter()
        .fold(0u32, |hash, &b| hash.wrapping_add(b.into()))
}

fn trie_add(trie: &mut Vec<TrieNode>, text: &[u8]) {
    let mut node = 0;
    for c in String::from_utf8_lossy(text).chars() {
        let mut child = trie[node].child;
        while let Some(index) = child {
            if trie[index].ch == c {
                break;
            }
            child = trie[index].next;
        }
        node = match child {
            Some(index) => index,
            None => {
                // A new character goes first among its siblings.
                trie.push(TrieNode {
                    ch: c,
                    child: None,
                    next: trie[node].child,
                    words: 0,
                });
                let index = trie.len() - 1;
                trie[node].child = Some(index);
                index
            }
        };
    }
    trie[node].words += 1;
}

fn trie_remove(trie: &mut [TrieNode], text: &[u8]) {
    let mut node = 0;
    for c in String::from_utf8_lossy(text).chars() {
        let mut child = trie[node].child;
        while let Some(index) = child {
            if trie[index].ch == c {
                break;
            }
            child = trie[index].next;
        }
        match child {
            Some(index) => node = index,
            None => return,
        }
    }
    trie[node].words = trie[node].words.saturating_sub(1);
}

impl Dictionary {
    pub(crate) fn new(comparator: ObjId) -> Self {
        Self {
            comparator,
            buckets: vec![Vec::new(); BUCKETS],
            trie: None,
            modified: false,
        }
    }

    /// Read a dictionary from its image data: the comparator, the number
    /// of words, and each word, lightly disguised, with its entries. The
    /// words are stored by their plain hash; the caller rehashes them
    /// for the comparator.
    pub(crate) fn from_image(data: &[u8]) -> Option<Self> {
        let u16_at = |at: usize| Some(u16::from_le_bytes([*data.get(at)?, *data.get(at + 1)?]));
        let u32_at = |at: usize| {
            let b = data.get(at..at + 4)?;
            Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };
        let mut dictionary = Self::new(ObjId(u32_at(0)?));
        let count = u16_at(4)?;
        let mut pos = 6;
        for _ in 0..count {
            if pos >= data.len() {
                break;
            }
            let len = usize::from(*data.get(pos)?);
            let text: Vec<u8> = data
                .get(pos + 1..pos + 1 + len)?
                .iter()
                .map(|b| b ^ 0xbd)
                .collect();
            pos += 1 + len;
            let items = u16_at(pos)?;
            pos += 2;
            let hash = plain_hash(&text);
            for _ in 0..items {
                let object = ObjId(u32_at(pos)?);
                let prop = u16_at(pos + 4)?;
                dictionary.add(hash, &text, object, prop);
                pos += 6;
            }
        }
        Some(dictionary)
    }

    /// Every word, in the order the dictionary goes through them.
    pub(crate) fn words(&self) -> impl Iterator<Item = &Word> {
        self.buckets.iter().flatten()
    }

    fn bucket(hash: u32) -> usize {
        hash as usize & (BUCKETS - 1)
    }

    /// The words in the bucket a hash selects.
    pub(crate) fn candidates(&self, hash: u32) -> &[Word] {
        &self.buckets[Self::bucket(hash)]
    }

    /// Add an entry for a word. Returns false if it was there already.
    pub(crate) fn add(&mut self, hash: u32, text: &[u8], object: ObjId, prop: PropId) -> bool {
        let bucket = &mut self.buckets[Self::bucket(hash)];
        let index = match bucket.iter().position(|word| word.text == text) {
            Some(index) => index,
            None => {
                bucket.insert(
                    0,
                    Word {
                        text: text.to_vec(),
                        entries: Vec::new(),
                    },
                );
                if let Some(trie) = &mut self.trie {
                    trie_add(trie, text);
                }
                0
            }
        };
        let entries = &mut bucket[index].entries;
        if entries.contains(&(object, prop)) {
            return false;
        }
        // An entry goes after the last one for the same part of speech,
        // or first if there is none.
        let at = entries
            .iter()
            .rposition(|&(_, p)| p == prop)
            .map_or(0, |last| last + 1);
        entries.insert(at, (object, prop));
        true
    }

    /// Remove an entry for a word, and the word if that was its last.
    /// Returns whether the entry was there.
    pub(crate) fn remove(&mut self, hash: u32, text: &[u8], object: ObjId, prop: PropId) -> bool {
        let bucket = &mut self.buckets[Self::bucket(hash)];
        let Some(index) = bucket.iter().position(|word| word.text == text) else {
            return false;
        };
        if let Some(trie) = &mut self.trie {
            trie_remove(trie, text);
        }
        let entries = &mut bucket[index].entries;
        let before = entries.len();
        entries.retain(|&entry| entry != (object, prop));
        let found = entries.len() != before;
        if entries.is_empty() {
            bucket.remove(index);
        }
        found
    }

    /// Move every word to the bucket a new hash function gives it. The
    /// words are taken in order and each goes first in its new bucket.
    pub(crate) fn rehash(
        &mut self,
        hashes: &mut dyn FnMut(&[u8]) -> VmResult<u32>,
    ) -> VmResult<()> {
        let old = std::mem::replace(&mut self.buckets, vec![Vec::new(); BUCKETS]);
        self.trie = None;
        for word in old.into_iter().flatten() {
            let hash = hashes(&word.text)?;
            self.buckets[Self::bucket(hash)].insert(0, word);
        }
        Ok(())
    }

    /// Drop the entries for objects that no longer exist, as the
    /// collector asks: a dictionary does not keep its objects alive.
    pub(crate) fn remove_dead(&mut self, alive: &dyn Fn(ObjId) -> bool) {
        for bucket in &mut self.buckets {
            for word in bucket.iter_mut() {
                let before = word.entries.len();
                word.entries.retain(|&(object, _)| alive(object));
                if let Some(trie) = &mut self.trie {
                    for _ in word.entries.len()..before {
                        trie_remove(trie, &word.text);
                    }
                }
            }
            bucket.retain(|word| !word.entries.is_empty());
        }
    }

    /// The tree of words, built on first use.
    fn trie(&mut self) -> &[TrieNode] {
        if self.trie.is_none() {
            let mut trie = vec![TrieNode {
                ch: '\0',
                child: None,
                next: None,
                words: 0,
            }];
            for word in self.buckets.iter().flatten() {
                trie_add(&mut trie, &word.text);
            }
            self.trie = Some(trie);
        }
        self.trie.as_deref().unwrap_or(&[])
    }

    /// Find the words within a number of single-character edits of some
    /// input: insertions, deletions, replacements, and swaps of adjacent
    /// characters. Returns each word with its distance and how many of
    /// the edits were replacements, in the reference VM's order.
    pub(crate) fn correct(
        &mut self,
        input: &[char],
        max_distance: i32,
        comparator: Option<&StrComp>,
    ) -> Vec<(String, i32, i32)> {
        #[derive(Clone, Copy, PartialEq)]
        enum Edit {
            None,
            Insertion,
            Deletion,
            Replacement,
            Transposition,
        }
        struct State {
            word: Vec<char>,
            distance: i32,
            replacements: i32,
            /// How much of the input is used.
            used: usize,
            node: usize,
            edit: Edit,
        }
        let truncation = comparator.map_or(0, |c| c.truncation);
        let trie = self.trie().to_vec();
        let mut stack = vec![State {
            word: Vec::new(),
            distance: 0,
            replacements: 0,
            used: 0,
            node: 0,
            edit: Edit::None,
        }];
        // Found words, newest last.
        let mut results: Vec<(Vec<char>, i32, i32)> = Vec::new();
        while let Some(s) = stack.pop() {
            if trie[s.node].words != 0 && s.used == input.len() {
                let index = match results.iter().position(|(word, _, _)| *word == s.word) {
                    Some(index) => index,
                    None => {
                        results.push((s.word.clone(), s.distance, s.replacements));
                        results.len() - 1
                    }
                };
                let found = &mut results[index];
                if s.distance < found.1 || (s.distance == found.1 && s.replacements < found.2) {
                    found.1 = s.distance;
                    found.2 = s.replacements;
                }
            }
            // The input has a character the word lacks.
            if s.distance < max_distance && s.used < input.len() && s.edit != Edit::Deletion {
                stack.push(State {
                    word: s.word.clone(),
                    distance: s.distance + 1,
                    replacements: s.replacements,
                    used: s.used + 1,
                    node: s.node,
                    edit: Edit::Insertion,
                });
            }
            let mut child = trie[s.node].child;
            while let Some(node) = child {
                let ch = trie[node].ch;
                child = trie[node].next;
                let mut longer = s.word.clone();
                longer.push(ch);
                let mut push = |distance, replacements, used, edit| {
                    stack.push(State {
                        word: longer.clone(),
                        distance,
                        replacements,
                        used,
                        node,
                        edit,
                    });
                };
                let mut matched = 0;
                if s.used < input.len() {
                    matched = match comparator {
                        Some(comparator) => comparator.match_chars(&input[s.used..], ch),
                        None => usize::from(input[s.used] == ch),
                    };
                    if matched != 0 {
                        push(s.distance, s.replacements, s.used + matched, Edit::None);
                    }
                } else if truncation != 0 && s.word.len() >= truncation && s.used >= truncation {
                    // Past the truncation length, the rest of a word is
                    // free.
                    push(s.distance, s.replacements, s.used, Edit::None);
                }
                if s.distance < max_distance {
                    if s.used < input.len() && matched == 0 {
                        push(
                            s.distance + 1,
                            s.replacements + 1,
                            s.used + 1,
                            Edit::Replacement,
                        );
                    }
                    if s.edit != Edit::Insertion {
                        push(s.distance + 1, s.replacements, s.used, Edit::Deletion);
                    }
                }
                // Two replacements in a row that swap two characters
                // count as one edit.
                if s.edit == Edit::Replacement
                    && s.used > 0
                    && s.used < input.len()
                    && input[s.used - 1] == ch
                    && s.word.last() == Some(&input[s.used])
                {
                    push(
                        s.distance,
                        s.replacements - 1,
                        s.used + 1,
                        Edit::Transposition,
                    );
                }
            }
        }
        // The reference lists the newest first, and leaves out the input
        // itself.
        results
            .into_iter()
            .rev()
            .filter(|&(_, distance, _)| distance != 0)
            .map(|(word, distance, replacements)| {
                (word.into_iter().collect(), distance, replacements)
            })
            .collect()
    }
}

/// The name and calling interface of each method, by method number.
const METHODS: &[(&str, NativeDesc)] = &[
    ("", NativeDesc::fixed(0)),
    ("setComparator", NativeDesc::fixed(1)),
    ("findWord", NativeDesc::optional(1, 1)),
    ("addWord", NativeDesc::fixed(3)),
    ("removeWord", NativeDesc::fixed(3)),
    ("isWordDefined", NativeDesc::optional(1, 1)),
    ("forEachWord", NativeDesc::fixed(1)),
    ("correctSpelling", NativeDesc::fixed(2)),
];

/// How a dictionary's comparator works.
enum Comparator {
    None,
    Builtin(Rc<StrComp>),
    /// An object of the program's, with methods to call.
    Program(ObjId),
}

impl Vm {
    fn dict(&self, id: ObjId) -> VmResult<&Dictionary> {
        match self.objects.get(id) {
            Some(Object::Dict(dictionary)) => Ok(dictionary),
            _ => Err(VmError::new(code::BAD_TYPE_BIF)),
        }
    }

    fn dict_mut(&mut self, id: ObjId) -> VmResult<&mut Dictionary> {
        match self.objects.get_mut(id) {
            Some(Object::Dict(dictionary)) => Ok(dictionary),
            _ => Err(VmError::new(code::BAD_TYPE_BIF)),
        }
    }

    fn comparator_of(&self, comparator: ObjId) -> Comparator {
        if !comparator.is_valid() {
            Comparator::None
        } else if let Some(builtin) = self.as_strcomp(comparator) {
            Comparator::Builtin(builtin)
        } else {
            Comparator::Program(comparator)
        }
    }

    /// The hash of a word under a comparator. `value` is the word as a
    /// string value, if there is one already.
    fn word_hash(
        &mut self,
        comparator: &Comparator,
        value: Option<Value>,
        text: &[u8],
    ) -> VmResult<u32> {
        match comparator {
            Comparator::None => Ok(plain_hash(text)),
            Comparator::Builtin(builtin) => Ok(builtin.hash(text)),
            Comparator::Program(object) => {
                let value = match value {
                    Some(value) => value,
                    None => self.try_new_string(text)?,
                };
                self.push(value);
                let native = NativeCall::named("HashComparator.calcHash");
                let prop = self.predef.calc_hash;
                match self.call_prop(Value::Obj(*object), prop, 1, Some(native))? {
                    Value::Int(hash) => Ok(hash as u32),
                    _ => Err(VmError::new(code::BAD_TYPE_BIF)),
                }
            }
        }
    }

    /// Whether input matches a dictionary word under a comparator, as
    /// the value the comparator gives, or `None` for no match.
    fn word_match(
        &mut self,
        comparator: &Comparator,
        value: Value,
        text: &[u8],
        reference: &[u8],
    ) -> VmResult<Option<Value>> {
        match comparator {
            Comparator::None => Ok((text == reference).then_some(Value::Int(1))),
            Comparator::Builtin(builtin) => match builtin.match_strings(text, reference) {
                0 => Ok(None),
                flags => Ok(Some(Value::Int(flags as i32))),
            },
            Comparator::Program(object) => {
                let reference = self.try_new_string(reference)?;
                self.push(reference);
                self.push(value);
                let native = NativeCall::named("HashComparator.matchValues");
                let prop = self.predef.match_values;
                let result = self.call_prop(Value::Obj(*object), prop, 2, Some(native))?;
                Ok(match result {
                    Value::Nil | Value::Int(0) => None,
                    other => Some(other),
                })
            }
        }
    }

    /// Give a dictionary a comparator, and move its words to where the
    /// comparator's hash puts them.
    pub(crate) fn dict_set_comparator(&mut self, id: ObjId, comparator: ObjId) -> VmResult<()> {
        if self.dict(id)?.comparator == comparator {
            return Ok(());
        }
        self.dict_mut(id)?.comparator = comparator;
        self.dict_rehash(id)
    }

    /// Put a dictionary's words where its comparator's hash says.
    pub(crate) fn dict_rehash(&mut self, id: ObjId) -> VmResult<()> {
        let comparator = self.comparator_of(self.dict(id)?.comparator);
        // The words are moved in a copy, since hashing can run code.
        let mut dictionary = self.dict(id)?.clone();
        dictionary.rehash(&mut |text| self.word_hash(&comparator, None, text))?;
        *self.dict_mut(id)? = dictionary;
        Ok(())
    }

    /// `new Dictionary(comparator?)`.
    pub(crate) fn create_dict(&mut self, argc: u32) -> VmResult<ObjId> {
        let comparator = match (argc, if argc == 1 { self.arg(0) } else { Value::Nil }) {
            (0 | 1, Value::Nil) => ObjId::INVALID,
            (1, Value::Obj(id)) => id,
            (0 | 1, _) => return Err(VmError::new(code::BAD_TYPE_BIF)),
            _ => return Err(VmError::new(code::WRONG_NUM_OF_ARGS)),
        };
        let mut dictionary = Dictionary::new(comparator);
        dictionary.modified = true;
        Ok(self.alloc_obj(Object::Dict(Box::new(dictionary))))
    }

    pub(crate) fn dict_method(
        &mut self,
        id: ObjId,
        index: u16,
        argc: Option<&mut u32>,
    ) -> VmResult<Option<Value>> {
        let Some(&(_, desc)) = METHODS.get(index as usize).filter(|_| index != 0) else {
            return Ok(None);
        };
        self.native_method(argc, desc, |vm, argc| vm.dict_call(id, index, argc))
            .map(Some)
    }

    /// The strings an `addWord` or `removeWord` argument names: one, or
    /// a list of them.
    fn word_args(&mut self, value: Value) -> VmResult<Vec<Vec<u8>>> {
        let required = || VmError::new(code::STRING_VAL_REQD);
        match self.list_like_len(value)? {
            Some(count) => {
                let mut words = Vec::with_capacity(count);
                for index in 1..=count {
                    let item = self.list_like_get(value, index)?;
                    words.push(self.as_str(item).ok_or_else(required)?.to_vec());
                }
                Ok(words)
            }
            None => Ok(vec![self.as_str(value).ok_or_else(required)?.to_vec()]),
        }
    }

    fn dict_call(&mut self, id: ObjId, index: u16, argc: u32) -> VmResult<Value> {
        let comparator = self.comparator_of(self.dict(id)?.comparator);
        Ok(match index {
            // setComparator(comparator)
            1 => {
                let comparator = match self.arg(0) {
                    Value::Nil => ObjId::INVALID,
                    Value::Obj(id) => id,
                    _ => return Err(VmError::new(code::BAD_TYPE_BIF)),
                };
                self.dict_set_comparator(id, comparator)?;
                self.dict_mut(id)?.modified = true;
                Value::Nil
            }
            // findWord(str, vocabProp?): each object the word refers
            // to, followed by how well the word matched.
            2 => {
                let value = self.arg(0);
                let text = self.require_str(value)?.to_vec();
                let prop = match if argc >= 2 { self.arg(1) } else { Value::Nil } {
                    Value::Nil => INVALID_PROP,
                    Value::Prop(prop) => prop,
                    _ => return Err(VmError::new(code::PROPPTR_VAL_REQD)),
                };
                let hash = self.word_hash(&comparator, Some(value), &text)?;
                let candidates = self.dict(id)?.candidates(hash).to_vec();
                let mut found = Vec::new();
                for word in candidates {
                    if let Some(result) = self.word_match(&comparator, value, &text, &word.text)? {
                        for (object, word_prop) in word.entries {
                            if prop == INVALID_PROP || word_prop == prop {
                                found.push(Value::Obj(object));
                                found.push(result);
                            }
                        }
                    }
                }
                self.new_list(found)
            }
            // addWord(obj, str, vocabProp) and removeWord(...)
            3 | 4 => {
                let object = self.obj_arg(self.arg(0))?;
                let prop = self.prop_arg(self.arg(2))?;
                if index == 3
                    && (self.as_list(Value::Obj(object)).is_some()
                        || self.as_str(Value::Obj(object)).is_some())
                {
                    return Err(VmError::new(code::OBJ_VAL_REQD));
                }
                for text in self.word_args(self.arg(1))? {
                    let hash = self.word_hash(&comparator, None, &text)?;
                    // The reference hashes a word once to look for it,
                    // and again to put in a new word or take out an
                    // empty one. With a comparator the program wrote,
                    // each is a call the program can see.
                    if matches!(comparator, Comparator::Program(_)) {
                        let word = self
                            .dict(id)?
                            .candidates(hash)
                            .iter()
                            .find(|word| word.text == text);
                        let again = match (index, word) {
                            (3, None) => true,
                            (4, Some(word)) => word.entries == [(object, prop)],
                            _ => false,
                        };
                        if again {
                            self.word_hash(&comparator, None, &text)?;
                        }
                    }
                    let dictionary = self.dict_mut(id)?;
                    if index == 3 {
                        dictionary.add(hash, &text, object, prop);
                    } else {
                        dictionary.remove(hash, &text, object, prop);
                    }
                    dictionary.modified = true;
                }
                Value::Nil
            }
            // isWordDefined(str, filter?)
            5 => {
                let value = self.arg(0);
                let text = self.require_str(value)?.to_vec();
                let filter = if argc >= 2 { self.arg(1) } else { Value::Nil };
                let hash = self.word_hash(&comparator, Some(value), &text)?;
                let candidates = self.dict(id)?.candidates(hash).to_vec();
                let mut found = false;
                for word in candidates {
                    let Some(result) = self.word_match(&comparator, value, &text, &word.text)?
                    else {
                        continue;
                    };
                    // A filter gets to turn down a match by its result.
                    if filter != Value::Nil {
                        let native = self.native_call("Dict.isDefined", Value::Obj(id), 5, argc);
                        match self.call_value(filter, &[result], native)? {
                            Value::Nil | Value::Int(0) => continue,
                            _ => {}
                        }
                    }
                    found = true;
                    break;
                }
                Value::logical(found)
            }
            // forEachWord(func): func(obj, str, vocabProp) for each
            // entry. The function can change the dictionary, so each
            // step finds its place again by position.
            6 => {
                let func = self.arg(0);
                for bucket in 0..BUCKETS {
                    let mut word_index = 0;
                    while let Some(word) = self
                        .dict(id)?
                        .candidates(bucket as u32)
                        .get(word_index)
                        .cloned()
                    {
                        let text = self.try_new_string(word.text.clone())?;
                        let mut entry_index = 0;
                        loop {
                            // The word is followed through the calls by
                            // its text, as the reference follows it by
                            // its address.
                            let entry = self
                                .dict(id)?
                                .candidates(bucket as u32)
                                .iter()
                                .find(|w| w.text == word.text)
                                .and_then(|w| w.entries.get(entry_index).copied());
                            let Some((object, prop)) = entry else {
                                break;
                            };
                            let native =
                                self.native_call("Dict.forEachWord", Value::Obj(id), 6, argc);
                            let args = [Value::Obj(object), text, Value::Prop(prop)];
                            self.call_value(func, &args, native)?;
                            entry_index += 1;
                        }
                        word_index += 1;
                    }
                }
                Value::Nil
            }
            // correctSpelling(str, maxDistance): the words close to a
            // misspelled one, each as [word, distance, replacements].
            7 => {
                let text = self.require_str(self.arg(0))?.to_vec();
                let max_distance = self.int_arg(self.arg(1))?;
                let input: Vec<char> = String::from_utf8_lossy(&text).chars().collect();
                let builtin = match &comparator {
                    Comparator::Builtin(builtin) => Some(builtin.clone()),
                    _ => None,
                };
                let found = self
                    .dict_mut(id)?
                    .correct(&input, max_distance, builtin.as_deref());
                // Made at their full lengths, as the reference makes them.
                let result = self.new_list(vec![Value::Nil; found.len()]);
                self.push(result);
                let mut items = Vec::with_capacity(found.len());
                for (word, distance, replacements) in found {
                    let entry = self.new_list(vec![Value::Nil; 3]);
                    items.push(entry);
                    if let Some(Object::List(stored)) = self.objects.get_mut(result.obj_id()) {
                        *stored = items.clone().into();
                    }
                    let word = self.try_new_str(&word)?;
                    if let Some(Object::List(stored)) = self.objects.get_mut(entry.obj_id()) {
                        *stored = vec![word, Value::Int(distance), Value::Int(replacements)].into();
                    }
                }
                self.discard(1);
                result
            }
            _ => Value::Nil,
        })
    }
}

// ---- what the grammar parser needs from a dictionary ----

impl Vm {
    /// A dictionary's comparator.
    pub(crate) fn dict_comparator(&self, dict: ObjId) -> VmResult<ObjId> {
        Ok(self.dict(dict)?.comparator)
    }

    /// The hash of a word under a dictionary's comparator, or the plain
    /// hash with no dictionary.
    pub(crate) fn dict_word_hash(
        &mut self,
        dict: Option<ObjId>,
        value: Option<Value>,
        text: &[u8],
    ) -> VmResult<u32> {
        match dict {
            Some(dict) => {
                let comparator = self.comparator_of(self.dict(dict)?.comparator);
                self.word_hash(&comparator, value, text)
            }
            None => Ok(plain_hash(text)),
        }
    }

    /// Whether input matches a word under a dictionary's comparator,
    /// and what the comparator says about the match.
    pub(crate) fn dict_match(
        &mut self,
        dict: ObjId,
        value: Value,
        text: &[u8],
        reference: &[u8],
    ) -> VmResult<Option<Value>> {
        let comparator = self.comparator_of(self.dict(dict)?.comparator);
        self.word_match(&comparator, value, text, reference)
    }

    /// The vocabulary properties a dictionary lists a word under, each
    /// once, in the order the dictionary comes to them.
    pub(crate) fn dict_word_props(
        &mut self,
        dict: ObjId,
        value: Value,
        text: &[u8],
    ) -> VmResult<Vec<PropId>> {
        let comparator = self.comparator_of(self.dict(dict)?.comparator);
        let hash = self.word_hash(&comparator, Some(value), text)?;
        let candidates = self.dict(dict)?.candidates(hash).to_vec();
        let mut props = Vec::new();
        for word in candidates {
            if self
                .word_match(&comparator, value, text, &word.text)?
                .is_some()
            {
                for (_, prop) in word.entries {
                    if !props.contains(&prop) {
                        props.push(prop);
                    }
                }
            }
        }
        Ok(props)
    }

    /// Put an entry in a dictionary, as `addWord` does.
    pub(crate) fn dict_add_word(
        &mut self,
        dict: ObjId,
        text: &[u8],
        object: ObjId,
        prop: PropId,
    ) -> VmResult<()> {
        let comparator = self.comparator_of(self.dict(dict)?.comparator);
        let hash = self.word_hash(&comparator, None, text)?;
        // A new word is hashed again to be put in, which a comparator
        // the program wrote sees.
        if matches!(comparator, Comparator::Program(_))
            && !self
                .dict(dict)?
                .candidates(hash)
                .iter()
                .any(|word| word.text == text)
        {
            self.word_hash(&comparator, None, text)?;
        }
        let dictionary = self.dict_mut(dict)?;
        dictionary.add(hash, text, object, prop);
        dictionary.modified = true;
        Ok(())
    }

    /// Take an entry out of a dictionary.
    pub(crate) fn dict_remove_word(
        &mut self,
        dict: ObjId,
        text: &[u8],
        object: ObjId,
        prop: PropId,
    ) -> VmResult<()> {
        let comparator = self.comparator_of(self.dict(dict)?.comparator);
        let hash = self.word_hash(&comparator, None, text)?;
        let dictionary = self.dict_mut(dict)?;
        dictionary.remove(hash, text, object, prop);
        dictionary.modified = true;
        Ok(())
    }
}
