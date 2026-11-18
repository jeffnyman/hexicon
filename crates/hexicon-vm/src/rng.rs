//! Random numbers.
//!
//! The generator is ISAAC, as in the reference VM, so that a program run
//! from the same seed makes the same choices under both. Until it is
//! seeded, the generator starts from a fixed state.

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

/// Scale 32 random bits to the range `0..range`.
fn rand_range(value: u32, range: u32) -> u32 {
    ((u64::from(value) * u64::from(range)) >> 32) as u32
}

impl Vm {
    /// Seed the random number generator from the host's entropy. Without
    /// this, every run of a program makes the same random choices.
    pub fn seed_random(&mut self) {
        let mut seed = [0u8; SIZE * 4];
        self.host.random_bytes(&mut seed);
        self.rng = Isaac::seeded(&seed);
    }

    /// `randomize(...)`.
    pub(crate) fn bif_randomize(&mut self, argc: u32) -> VmResult<()> {
        match argc {
            0 => {
                self.seed_random();
                self.r0 = Value::Nil;
                Ok(())
            }
            1 | 2 => Err(VmError::unimplemented(
                "randomize with a generator, seed, or saved state",
            )),
            _ => Err(VmError::new(code::WRONG_NUM_OF_ARGS)),
        }
    }

    /// `rand(...)`: a random integer below a limit, a random element of a
    /// list, or one of the arguments at random.
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
            if self.as_str(arg).is_some() {
                return Err(VmError::unimplemented("rand with a string template"));
            }
        }
        // Choose one of the arguments.
        let choice = rand_range(self.rng.next(), argc);
        self.r0 = self.arg(choice as usize);
        Ok(())
    }
}
