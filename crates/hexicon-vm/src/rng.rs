//! Random numbers.
//!
//! The generators are the reference VM's, so that a program run from the
//! same seed makes the same choices under both. ISAAC is the default;
//! a program can ask instead for a linear congruential generator or the
//! Mersenne Twister, and can save and restore a generator's state. Until
//! it is seeded, ISAAC starts from a fixed state.

use crate::error::{VmError, VmResult, code};
use crate::value::Value;
use crate::vm::Vm;

const SIZE_LOG2: u32 = 8;
const SIZE: usize = 1 << SIZE_LOG2;

/// The ISAAC generator's state.
#[derive(Debug, Clone)]
pub(crate) struct Isaac {
    /// Results not yet handed out, counted down from the end.
    count: u32,
    results: [u32; SIZE],
    mem: [u32; SIZE],
    a: u32,
    b: u32,
    c: u32,
}

fn mix(s: &mut [u32; 8]) {
    s[0] ^= s[1] << 11;
    s[3] = s[3].wrapping_add(s[0]);
    s[1] = s[1].wrapping_add(s[2]);
    s[1] ^= s[2] >> 2;
    s[4] = s[4].wrapping_add(s[1]);
    s[2] = s[2].wrapping_add(s[3]);
    s[2] ^= s[3] << 8;
    s[5] = s[5].wrapping_add(s[2]);
    s[3] = s[3].wrapping_add(s[4]);
    s[3] ^= s[4] >> 16;
    s[6] = s[6].wrapping_add(s[3]);
    s[4] = s[4].wrapping_add(s[5]);
    s[4] ^= s[5] << 10;
    s[7] = s[7].wrapping_add(s[4]);
    s[5] = s[5].wrapping_add(s[6]);
    s[5] ^= s[6] >> 4;
    s[0] = s[0].wrapping_add(s[5]);
    s[6] = s[6].wrapping_add(s[7]);
    s[6] ^= s[7] << 8;
    s[1] = s[1].wrapping_add(s[6]);
    s[7] = s[7].wrapping_add(s[0]);
    s[7] ^= s[0] >> 9;
    s[2] = s[2].wrapping_add(s[7]);
    s[0] = s[0].wrapping_add(s[1]);
}

impl Isaac {
    /// A generator in the fixed starting state.
    pub(crate) fn new() -> Self {
        let mut isaac = Self {
            count: 0,
            results: [0; SIZE],
            mem: [0; SIZE],
            a: 0,
            b: 0,
            c: 0,
        };
        isaac.init(false);
        isaac
    }

    /// A generator seeded from random bytes.
    pub(crate) fn seeded(seed: &[u8]) -> Self {
        let mut isaac = Self::new();
        for (index, chunk) in seed.chunks(4).take(SIZE).enumerate() {
            let mut word = [0u8; 4];
            word[..chunk.len()].copy_from_slice(chunk);
            isaac.results[index] = u32::from_le_bytes(word);
        }
        isaac.init(true);
        isaac
    }

    /// Set up the state, from the contents of `results` if `use_seed`.
    fn init(&mut self, use_seed: bool) {
        self.a = 0;
        self.b = 0;
        self.c = 0;
        let mut s = [0x9e37_79b9u32; 8];
        for _ in 0..4 {
            mix(&mut s);
        }
        let passes: &[bool] = if use_seed { &[true, false] } else { &[false] };
        for (pass, &from_results) in passes.iter().enumerate() {
            for i in (0..SIZE).step_by(8) {
                if use_seed {
                    for k in 0..8 {
                        let source = if from_results {
                            self.results[i + k]
                        } else {
                            self.mem[i + k]
                        };
                        s[k] = s[k].wrapping_add(source);
                    }
                }
                let _ = pass;
                mix(&mut s);
                self.mem[i..i + 8].copy_from_slice(&s);
            }
        }
        self.generate();
        self.count = SIZE as u32;
    }

    /// Produce the next batch of results.
    fn generate(&mut self) {
        self.c = self.c.wrapping_add(1);
        let mut a = self.a;
        let mut b = self.b.wrapping_add(self.c);
        for i in 0..SIZE {
            let x = self.mem[i];
            let shifted = match i % 4 {
                0 => a << 13,
                1 => a >> 6,
                2 => a << 2,
                _ => a >> 16,
            };
            a = (a ^ shifted).wrapping_add(self.mem[(i + SIZE / 2) % SIZE]);
            let y = self.mem[(x >> 2) as usize & (SIZE - 1)]
                .wrapping_add(a)
                .wrapping_add(b);
            self.mem[i] = y;
            b = self.mem[(y >> SIZE_LOG2 >> 2) as usize & (SIZE - 1)].wrapping_add(x);
            self.results[i] = b;
        }
        self.a = a;
        self.b = b;
    }

    /// Seed from up to 1024 bytes, such as the text of a string.
    fn seed_bytes(&mut self, seed: &[u8]) {
        self.results = [0; SIZE];
        for (index, chunk) in seed.chunks(4).take(SIZE).enumerate() {
            let mut word = [0u8; 4];
            word[..chunk.len()].copy_from_slice(chunk);
            self.results[index] = u32::from_le_bytes(word);
        }
        self.init(true);
    }

    /// The whole state, as a program saves it.
    fn state(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 * (4 + 2 * SIZE));
        for word in [self.count, self.a, self.b, self.c] {
            out.extend_from_slice(&word.to_le_bytes());
        }
        for (result, mem) in self.results.iter().zip(&self.mem) {
            out.extend_from_slice(&result.to_le_bytes());
            out.extend_from_slice(&mem.to_le_bytes());
        }
        out
    }

    fn set_state(&mut self, state: &[u8]) {
        let mut words = state
            .chunks_exact(4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        let mut next = || words.next().unwrap_or(0);
        self.count = next();
        self.a = next();
        self.b = next();
        self.c = next();
        for i in 0..SIZE {
            self.results[i] = next();
            self.mem[i] = next();
        }
        if self.count as usize > SIZE {
            self.count = 0;
        }
    }

    /// The next 32 random bits.
    pub(crate) fn next(&mut self) -> u32 {
        if self.count == 0 {
            self.generate();
            self.count = SIZE as u32;
        }
        self.count -= 1;
        self.results[self.count as usize]
    }
}

/// The Mersenne Twister, MT19937.
#[derive(Debug, Clone)]
pub(crate) struct Mt {
    mt: [u32; Mt::N],
    idx: usize,
}

impl Mt {
    const N: usize = 624;

    pub(crate) fn new() -> Self {
        Self {
            mt: [0; Self::N],
            idx: 0,
        }
    }

    fn base_seed(&mut self, seed: u32) {
        self.mt[0] = seed;
        for i in 1..Self::N {
            let previous = self.mt[i - 1];
            self.mt[i] = 0x6c07_8965u32
                .wrapping_mul(previous ^ (previous >> 30))
                .wrapping_add(i as u32);
        }
        self.idx = Self::N;
    }

    fn seed_int(&mut self, seed: u32) {
        self.base_seed(seed);
        // The first few numbers after a simple seed are poorly mixed.
        for _ in 0..37 {
            self.next();
        }
    }

    fn seed_bytes(&mut self, seed: &[u8]) {
        self.base_seed(19_650_218);
        // The seed as 32-bit words, the last made of what bytes are left.
        // The reference reads those through a signed `char`.
        let whole = seed.len() / 4;
        let mut words: Vec<u32> = seed
            .chunks_exact(4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        if seed.len() > whole * 4 {
            let mut partial: u32 = 0;
            let mut scale: u32 = 1;
            for &byte in &seed[whole * 4..] {
                partial = partial.wrapping_add((i32::from(byte as i8) as u32).wrapping_mul(scale));
                scale = scale.wrapping_mul(256);
            }
            words.push(partial);
        }
        let n = Self::N;
        let (mut i, mut j) = (1, 0);
        for _ in 0..n.max(words.len()) {
            let word = words.get(j).copied().unwrap_or(0);
            let previous = self.mt[i - 1];
            self.mt[i] = (self.mt[i] ^ (previous ^ (previous >> 30)).wrapping_mul(0x0019_660d))
                .wrapping_add(word)
                .wrapping_add(j as u32);
            i += 1;
            if i >= n {
                self.mt[0] = self.mt[n - 1];
                i = 1;
            }
            j += 1;
            if j >= words.len() {
                j = 0;
            }
        }
        for _ in 0..n - 1 {
            let previous = self.mt[i - 1];
            self.mt[i] = (self.mt[i] ^ (previous ^ (previous >> 30)).wrapping_mul(0x5d58_8b65))
                .wrapping_sub(i as u32);
            i += 1;
            if i >= n {
                self.mt[0] = self.mt[n - 1];
                i = 1;
            }
        }
        self.mt[0] = 0x8000_0000;
        self.idx = 0;
        for _ in 0..=n {
            self.next();
        }
    }

    fn next(&mut self) -> u32 {
        let n = Self::N;
        if self.idx >= n {
            for i in 0..n {
                let y = (self.mt[i] & 0x8000_0000) + (self.mt[(i + 1) % n] & 0x7fff_ffff);
                self.mt[i] = self.mt[(i + 397) % n] ^ (y >> 1);
                if y & 1 != 0 {
                    self.mt[i] ^= 0x9908_b0df;
                }
            }
            self.idx = 0;
        }
        let mut y = self.mt[self.idx];
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c_5680;
        y ^= (y << 15) & 0xefc6_0000;
        y ^= y >> 18;
        self.idx += 1;
        y
    }

    fn state(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(2 + Self::N * 4);
        out.extend_from_slice(&(self.idx as u16).to_le_bytes());
        for word in self.mt {
            out.extend_from_slice(&word.to_le_bytes());
        }
        out
    }

    fn set_state(&mut self, state: &[u8]) {
        self.idx = usize::from(u16::from_le_bytes([state[0], state[1]])) % Self::N;
        for (word, b) in self.mt.iter_mut().zip(state[2..].chunks_exact(4)) {
            *word = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        }
    }
}

/// The CRC-32 the reference uses to turn a text seed into a number: the
/// usual polynomial, but starting from zero and not inverted at the end.
fn crc32(bytes: &[u8]) -> u32 {
    let mut acc: u32 = 0;
    for &byte in bytes {
        let mut entry = (acc ^ u32::from(byte)) & 0xff;
        for _ in 0..8 {
            entry = if entry & 1 != 0 {
                (entry >> 1) ^ 0xedb8_8320
            } else {
                entry >> 1
            };
        }
        acc = entry ^ (acc >> 8);
    }
    acc
}

/// The generators a program can choose among.
#[derive(Debug, Clone)]
pub(crate) struct Rng {
    /// Which generator `rand` uses.
    id: i32,
    pub(crate) isaac: Isaac,
    lcg: u32,
    mt: Box<Mt>,
}

const ISAAC: i32 = 1;
const LCG: i32 = 2;
const MT19937: i32 = 3;

impl Rng {
    pub(crate) fn new() -> Self {
        Self {
            id: ISAAC,
            isaac: Isaac::new(),
            lcg: 0o24136543305,
            mt: Box::new(Mt::new()),
        }
    }

    /// The next 32 random bits from the generator in use.
    pub(crate) fn next(&mut self) -> u32 {
        match self.id {
            LCG => {
                self.lcg = self.lcg.wrapping_mul(1_664_525).wrapping_add(1);
                self.lcg
            }
            MT19937 => self.mt.next(),
            _ => self.isaac.next(),
        }
    }

    fn seed_int(&mut self, id: i32, seed: i32) {
        match id {
            LCG => self.lcg = seed as u32,
            MT19937 => self.mt.seed_int(seed as u32),
            _ => {
                // ISAAC takes bytes: the number in binary and in text.
                let mut bytes = seed.to_le_bytes().to_vec();
                bytes.extend_from_slice(seed.to_string().as_bytes());
                self.isaac.seed_bytes(&bytes);
            }
        }
    }

    fn seed_text(&mut self, id: i32, seed: &[u8]) {
        match id {
            LCG => self.lcg = crc32(seed),
            MT19937 => self.mt.seed_bytes(seed),
            _ => self.isaac.seed_bytes(seed),
        }
    }
}

/// Scale 32 random bits to the range `0..range`.
fn rand_range(value: u32, range: u32) -> u32 {
    ((u64::from(value) * u64::from(range)) >> 32) as u32
}

/// Scale 32 random bits to a choice among inclusive ranges, each as
/// likely as it is long.
fn rand_ranges(value: u32, ranges: &[(u32, u32)]) -> u32 {
    let total = ranges.iter().map(|&(lo, hi)| hi - lo + 1).sum();
    let mut n = rand_range(value, total);
    for &(lo, hi) in ranges {
        if n <= hi - lo {
            return lo + n;
        }
        n -= hi - lo + 1;
    }
    0
}

// ---- string templates ----

/// A piece of a template for random text. `rand('[a-z]{3,5}%-d+')` makes
/// three to five letters, a hyphen, and some digits.
enum Node {
    /// One of several alternatives, separated by `|`.
    Alt(Vec<Node>),
    /// Pieces one after another.
    Cat(Vec<Node>),
    /// A piece repeated `lo` to `hi` times, or `lo` or more if there is
    /// no `hi`.
    Repeat(Box<Node>, u32, Option<u32>),
    /// Text in quotes, to copy.
    Lit(Vec<char>),
    /// A character to copy.
    Char(char),
    /// A character from a class named by a letter, such as `d` for a
    /// digit.
    Class(char),
    /// A character from a list of ranges in brackets, with how many
    /// characters the list covers.
    List(Vec<(u32, u32)>, u32),
}

/// A repeat with no upper limit stops after this many.
const MAX_UNBOUNDED: u32 = 50;

struct Template {
    chars: Vec<char>,
    at: usize,
}

impl Template {
    fn more(&self) -> bool {
        self.at < self.chars.len()
    }

    fn getch(&self) -> char {
        self.chars.get(self.at).copied().unwrap_or('\0')
    }

    fn skip(&mut self) {
        if self.more() {
            self.at += 1;
        }
    }

    fn parse_int(&mut self) -> u32 {
        let mut acc: u32 = 0;
        while let Some(digit) = self.getch().to_digit(10).filter(|_| self.more()) {
            acc = acc.wrapping_mul(10).wrapping_add(digit);
            self.skip();
        }
        acc
    }

    fn alt(&mut self, outer: bool) -> Node {
        let mut alternatives = Vec::new();
        while self.more() && (outer || self.getch() != ')') {
            let start = self.at;
            alternatives.push(self.cat());
            if self.getch() == '|' {
                self.skip();
            }
            // A `)` with no `(` would otherwise stop everything here.
            if self.at == start {
                self.skip();
            }
        }
        Node::Alt(alternatives)
    }

    fn cat(&mut self) -> Node {
        let mut pieces = Vec::new();
        while self.more() && self.getch() != ')' && self.getch() != '|' {
            pieces.push(self.repeat());
        }
        Node::Cat(pieces)
    }

    fn repeat(&mut self) -> Node {
        let piece = match self.getch() {
            '(' => {
                self.skip();
                let group = self.alt(false);
                if self.getch() == ')' {
                    self.skip();
                }
                group
            }
            '"' => self.literal(),
            _ => self.atom(),
        };
        let (lo, hi) = match self.getch() {
            '{' => {
                self.skip();
                let mut lo = self.parse_int();
                let hi = if self.getch() == ',' {
                    self.skip();
                    while self.getch().is_ascii_whitespace() {
                        self.skip();
                    }
                    if self.getch() == '}' || self.getch() == '\0' {
                        None
                    } else {
                        let hi = self.parse_int();
                        Some(if hi < lo {
                            std::mem::replace(&mut lo, hi)
                        } else {
                            hi
                        })
                    }
                } else {
                    Some(lo)
                };
                if self.getch() == '}' {
                    self.skip();
                }
                (lo, hi)
            }
            '+' => {
                self.skip();
                (1, None)
            }
            '*' => {
                self.skip();
                (0, None)
            }
            '?' => {
                self.skip();
                (0, Some(1))
            }
            _ => (1, Some(1)),
        };
        Node::Repeat(Box::new(piece), lo, hi)
    }

    /// Text in quotes. A doubled quote is a quote in the text.
    fn literal(&mut self) -> Node {
        let mut text = Vec::new();
        self.skip();
        while self.more() {
            let mut c = self.getch();
            if c == '"' {
                self.skip();
                c = self.getch();
                if c != '"' {
                    break;
                }
            }
            text.push(c);
            self.skip();
        }
        Node::Lit(text)
    }

    fn atom(&mut self) -> Node {
        match self.getch() {
            '[' => {
                // A list of characters and ranges. `%` takes the next
                // character as itself.
                let mut ranges = Vec::new();
                let mut count = 0;
                self.skip();
                while self.more() && self.getch() != ']' {
                    let mut first = self.getch();
                    if first == '%' {
                        self.skip();
                        first = if self.more() { self.getch() } else { '%' };
                    }
                    self.skip();
                    let mut last = first;
                    if self.getch() == '-' {
                        self.skip();
                        if self.getch() == '%' {
                            self.skip();
                        }
                        if self.more() {
                            last = self.getch();
                        }
                        self.skip();
                    }
                    let (lo, hi) = (u32::from(first.min(last)), u32::from(first.max(last)));
                    ranges.push((lo, hi));
                    count += hi - lo + 1;
                }
                self.skip();
                Node::List(ranges, count)
            }
            '%' => {
                self.skip();
                let c = if self.more() { self.getch() } else { '%' };
                self.skip();
                Node::Char(c)
            }
            c => {
                self.skip();
                Node::Class(c)
            }
        }
    }
}

impl Node {
    /// The most characters the piece can make. A template that can make
    /// none uses no random numbers at all.
    fn max_len(&self) -> u32 {
        match self {
            Self::Alt(nodes) => nodes.iter().map(Self::max_len).max().unwrap_or(0),
            Self::Cat(nodes) => nodes.iter().map(Self::max_len).sum(),
            Self::Repeat(node, _, hi) => node.max_len().saturating_mul(hi.unwrap_or(MAX_UNBOUNDED)),
            Self::Lit(text) => text.len() as u32,
            Self::Char(_) | Self::Class(_) | Self::List(..) => 1,
        }
    }

    fn generate(&self, rng: &mut Rng, out: &mut String) {
        let mut push = |code: u32| out.push(char::from_u32(code & 0xffff).unwrap_or('\u{fffd}'));
        match self {
            Self::Alt(nodes) => {
                let choice = rand_range(rng.next(), nodes.len() as u32);
                if let Some(node) = nodes.get(choice as usize) {
                    node.generate(rng, out);
                }
            }
            Self::Cat(nodes) => {
                for node in nodes {
                    node.generate(rng, out);
                }
            }
            Self::Repeat(node, lo, Some(hi)) => {
                let count = lo + rand_range(rng.next(), hi - lo + 1);
                for _ in 0..count {
                    node.generate(rng, out);
                }
            }
            Self::Repeat(node, lo, None) => {
                // The minimum, then two chances in three of another,
                // each time.
                for _ in 0..*lo {
                    node.generate(rng, out);
                }
                let mut extra = 0;
                while rand_range(rng.next(), 100) < 67 && extra < MAX_UNBOUNDED {
                    node.generate(rng, out);
                    extra += 1;
                }
            }
            Self::Lit(text) => out.extend(text),
            Self::Char(c) => out.push(*c),
            Self::List(ranges, count) => {
                // An empty list makes nothing, but like a letter that
                // names no class, it still uses a random number.
                if *count == 0 {
                    rng.next();
                    return;
                }
                let mut n = rand_range(rng.next(), *count) & 0xffff;
                for &(lo, hi) in ranges {
                    if n <= hi - lo {
                        push(lo + n);
                        break;
                    }
                    n -= hi - lo + 1;
                }
            }
            Self::Class(class) => {
                // A letter that names no class makes nothing, but still
                // uses a random number.
                let value = rng.next();
                let d = |c: char| u32::from(c);
                let ranges: &[(u32, u32)] = match class {
                    'i' => &[(32, 126)],
                    'l' => &[(32, 126), (160, 255)],
                    'u' => &[(32, 127), (160, 0xdfff), (0xf900, 0xfffe)],
                    'd' => &[(d('0'), d('9'))],
                    'X' => &[(d('0'), d('9')), (d('A'), d('F'))],
                    'x' => &[(d('0'), d('9')), (d('a'), d('f'))],
                    'A' => &[(d('A'), d('Z'))],
                    'a' => &[(d('a'), d('z'))],
                    'b' => &[(0, 255)],
                    'c' => &[(d('a'), d('z')), (d('A'), d('Z'))],
                    'z' => &[(d('0'), d('9')), (d('a'), d('z')), (d('A'), d('Z'))],
                    _ => return,
                };
                push(rand_ranges(value, ranges));
            }
        }
    }
}

/// Random text from a template.
fn rand_text(template: &str, rng: &mut Rng) -> String {
    let mut parser = Template {
        chars: template.chars().collect(),
        at: 0,
    };
    let tree = parser.alt(true);
    let mut out = String::new();
    if tree.max_len() != 0 {
        tree.generate(rng, &mut out);
    }
    out
}

impl Vm {
    /// Seed the random number generator from the host's entropy. Without
    /// this, every run of a program makes the same random choices.
    pub fn seed_random(&mut self) {
        let mut seed = [0u8; SIZE * 4];
        self.host.random_bytes(&mut seed);
        self.rng.id = ISAAC;
        self.rng.isaac = Isaac::seeded(&seed);
    }

    /// Seed one of the generators from the host's entropy.
    fn seed_random_generator(&mut self, id: i32) {
        match id {
            LCG => {
                let mut seed = [0u8; 4];
                self.host.random_bytes(&mut seed);
                self.rng.lcg = u32::from_le_bytes(seed);
            }
            MT19937 => {
                let mut seed = [0u8; Mt::N * 4];
                self.host.random_bytes(&mut seed);
                for (word, b) in self.rng.mt.mt.iter_mut().zip(seed.chunks_exact(4)) {
                    *word = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                }
            }
            _ => {
                let mut seed = [0u8; SIZE * 4];
                self.host.random_bytes(&mut seed);
                self.rng.isaac = Isaac::seeded(&seed);
            }
        }
    }

    /// `randomize()` seeds the default generator at random.
    /// `randomize(nil)` returns the generator in use and its state, and
    /// `randomize(list)` restores them. `randomize(id, seed?)` chooses a
    /// generator, seeding it from an integer or a string, or at random
    /// for nil.
    pub(crate) fn bif_randomize(&mut self, argc: u32) -> VmResult<()> {
        if argc > 2 {
            return Err(VmError::new(code::WRONG_NUM_OF_ARGS));
        }
        let check_id = |id: i32| {
            if (ISAAC..=MT19937).contains(&id) {
                Ok(id)
            } else {
                Err(VmError::new(code::BAD_VAL_BIF))
            }
        };
        let first = if argc >= 1 { self.arg(0) } else { Value::Nil };
        self.r0 = Value::Nil;
        if argc == 0 {
            self.seed_random();
        } else if argc == 1 && first == Value::Nil {
            let state = match self.rng.id {
                LCG => Value::Int(self.rng.lcg as i32),
                MT19937 => {
                    let state = self.rng.mt.state();
                    self.new_bytes(state)
                }
                _ => {
                    let state = self.rng.isaac.state();
                    self.new_bytes(state)
                }
            };
            self.r0 = self.new_list(vec![Value::Int(self.rng.id), state]);
        } else if argc == 1 && self.list_like_len(first)?.is_some() {
            let id = self.list_like_get(first, 1)?;
            let state = self.list_like_get(first, 2)?;
            let id = check_id(self.cast_num_to_int(id)?)?;
            if id == LCG {
                self.rng.lcg = self.cast_num_to_int(state)? as u32;
            } else {
                let Some(bytes) = self.as_bytes(state).cloned() else {
                    return Err(VmError::new(code::BAD_TYPE_BIF));
                };
                let expected = if id == MT19937 {
                    2 + Mt::N * 4
                } else {
                    4 * (4 + 2 * SIZE)
                };
                if bytes.len() != expected {
                    return Err(VmError::new(code::BAD_VAL_BIF));
                }
                if id == MT19937 {
                    self.rng.mt.set_state(&bytes);
                } else {
                    self.rng.isaac.set_state(&bytes);
                }
            }
            self.rng.id = id;
        } else if matches!(first, Value::Int(_)) || self.as_big(first).is_some() {
            let id = check_id(self.cast_num_to_int(first)?)?;
            if argc == 2 {
                let seed = self.arg(1);
                if seed == Value::Nil {
                    self.seed_random_generator(id);
                } else if matches!(seed, Value::Int(_)) || self.as_big(seed).is_some() {
                    let seed = self.cast_num_to_int(seed)?;
                    self.rng.seed_int(id, seed);
                } else if let Some(text) = self.as_str(seed) {
                    let text = text.to_vec();
                    self.rng.seed_text(id, &text);
                } else {
                    return Err(VmError::new(code::BAD_TYPE_BIF));
                }
            }
            self.rng.id = id;
        } else {
            return Err(VmError::new(code::WRONG_NUM_OF_ARGS));
        }
        Ok(())
    }

    /// An integer from an integer or a Decimal.
    fn cast_num_to_int(&self, value: Value) -> VmResult<i32> {
        match value {
            Value::Int(n) => Ok(n),
            other => match self.as_big(other) {
                Some(big) => Self::big_to_int(big),
                None => Err(VmError::new(code::NUM_VAL_REQD)),
            },
        }
    }

    /// `rand(...)`: a random integer below a limit, a random element of a
    /// list, random text from a template, or one of the arguments at
    /// random.
    pub(crate) fn bif_rand(&mut self, argc: u32) -> VmResult<()> {
        if argc == 0 {
            self.r0 = Value::Int(self.rng.next() as i32);
            return Ok(());
        }
        if argc == 1 {
            let arg = self.arg(0);
            if let Value::Int(range) = arg {
                self.r0 = Value::Int(rand_range(self.rng.next(), range as u32) as i32);
                return Ok(());
            }
            if let Some(len) = self.list_like_len(arg)? {
                let choice = rand_range(self.rng.next(), len as u32);
                self.r0 = if len == 0 {
                    Value::Nil
                } else {
                    self.list_like_get(arg, choice as usize + 1)?
                };
                return Ok(());
            }
            if let Some(template) = self.as_str(arg) {
                let template = String::from_utf8_lossy(&template).into_owned();
                let text = rand_text(&template, &mut self.rng);
                self.r0 = self.try_new_str(&text)?;
                return Ok(());
            }
        }
        // Choose one of the arguments.
        let choice = rand_range(self.rng.next(), argc);
        self.r0 = self.arg(choice as usize);
        Ok(())
    }
}
