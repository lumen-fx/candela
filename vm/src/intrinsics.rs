//! The standard library's native functions, built into the runtime for targets
//! that cannot load a dynamic library.
//!
//! `std/math`, `std/time`, `std/random` and `std/hash` reach their work
//! through a `dylib` block naming a small C library that ships beside the
//! toolchain. A browser has no such library and no loader, so on wasm32 the
//! same bindings resolve to the Rust functions here instead: the std modules
//! and every program that imports them read the same on every target. Each
//! binding is found by the library the std module names and the symbol it
//! declares, so a `.cdlb` built on a desktop that records those bindings runs
//! here unchanged.

use std::cell::Cell;

/// One argument, as the binding's declared type reads it.
#[derive(Clone, Copy, Debug)]
pub enum Arg<'a> {
    Int(i64),
    Float(f64),
    Str(&'a str),
}

impl Arg<'_> {
    fn float(self) -> f64 {
        match self {
            Self::Float(f) => f,
            Self::Int(i) => i as f64,
            Self::Str(_) => f64::NAN,
        }
    }

    fn int(self) -> i64 {
        match self {
            Self::Int(i) => i,
            Self::Float(f) => f as i64,
            Self::Str(_) => 0,
        }
    }

    fn bytes(&self) -> &[u8] {
        match self {
            Self::Str(s) => s.as_bytes(),
            Self::Int(_) | Self::Float(_) => &[],
        }
    }
}

/// What an intrinsic hands back to the program.
#[derive(Clone, Debug, PartialEq)]
pub enum Ret {
    Int(i64),
    Float(f64),
    Str(String),
    Null,
}

/// A native function the runtime provides in place of a library symbol.
pub type Intrinsic = fn(&[Arg<'_>]) -> Ret;

/// The library path, relative to the toolchain's `libs` directory, that each
/// std module's `dylib` block names.
const MATH: &str = "std_src/math/math";
const TIME: &str = "std_src/time/time";
const RANDOM: &str = "std_src/random/random";
const HASH: &str = "std_src/hash/hash";

macro_rules! unary {
    ($f:expr) => {
        |a: &[Arg<'_>]| Ret::Float($f(a[0].float()))
    };
}

macro_rules! binary {
    ($f:expr) => {
        |a: &[Arg<'_>]| Ret::Float($f(a[0].float(), a[1].float()))
    };
}

/// Every binding the std modules declare, by library and symbol.
const TABLE: &[(&str, &str, Intrinsic)] = &[
    (MATH, "candela_acos", unary!(f64::acos)),
    (MATH, "candela_asin", unary!(f64::asin)),
    (MATH, "candela_atan", unary!(f64::atan)),
    (MATH, "candela_atan2", binary!(f64::atan2)),
    (MATH, "candela_cos", unary!(f64::cos)),
    (MATH, "candela_sin", unary!(f64::sin)),
    (MATH, "candela_tan", unary!(f64::tan)),
    (MATH, "candela_acosh", unary!(f64::acosh)),
    (MATH, "candela_asinh", unary!(f64::asinh)),
    (MATH, "candela_atanh", unary!(f64::atanh)),
    (MATH, "candela_cosh", unary!(f64::cosh)),
    (MATH, "candela_sinh", unary!(f64::sinh)),
    (MATH, "candela_tanh", unary!(f64::tanh)),
    (MATH, "candela_exp", unary!(f64::exp)),
    (MATH, "candela_expm1", unary!(f64::exp_m1)),
    (MATH, "candela_log", unary!(f64::ln)),
    (MATH, "candela_log10", unary!(f64::log10)),
    (MATH, "candela_log2", unary!(f64::log2)),
    (MATH, "candela_log1p", unary!(f64::ln_1p)),
    (MATH, "candela_logb", unary!(logb)),
    (MATH, "candela_ldexp", |a| {
        Ret::Float(libm::ldexp(a[0].float(), clamp_exponent(a[1].int())))
    }),
    (MATH, "candela_ilogb", |a| {
        Ret::Int(i64::from(libm::ilogb(a[0].float())))
    }),
    (MATH, "candela_scalbn", |a| {
        Ret::Float(libm::scalbn(a[0].float(), clamp_exponent(a[1].int())))
    }),
    (MATH, "candela_cbrt", unary!(f64::cbrt)),
    (MATH, "candela_hypot", binary!(f64::hypot)),
    (MATH, "candela_erf", unary!(libm::erf)),
    (MATH, "candela_erfc", unary!(libm::erfc)),
    (MATH, "candela_sqrt", unary!(f64::sqrt)),
    (MATH, "candela_pow", binary!(f64::powf)),
    (MATH, "candela_floor", unary!(f64::floor)),
    (MATH, "candela_ceil", unary!(f64::ceil)),
    (MATH, "candela_round", unary!(f64::round)),
    (MATH, "candela_trunc", unary!(f64::trunc)),
    (MATH, "candela_fmod", binary!(|x: f64, y: f64| x % y)),
    (MATH, "candela_copysign", binary!(f64::copysign)),
    (TIME, "now", |_| Ret::Int(clock::now())),
    (TIME, "format", |a| Ret::Str(time_format(a[0].int(), a[1]))),
    (RANDOM, "candela_seed", |a| {
        RNG.set(Some(Pcg32::new(a[0].int() as u64, 54)));
        Ret::Null
    }),
    (RANDOM, "candela_random_int", |_| Ret::Int(draw64() as i64)),
    (RANDOM, "candela_random_int_range", |a| {
        Ret::Int(random_int_range(a[0].int(), a[1].int()))
    }),
    (RANDOM, "candela_random", |_| Ret::Float(random_unit())),
    (RANDOM, "candela_random_float_range", |a| {
        let (min, max) = (a[0].float(), a[1].float());
        Ret::Float(min + random_unit() * (max - min))
    }),
    (HASH, "candela_md5", |a| {
        Ret::Str(hex(&digest::md5(a[0].bytes())))
    }),
    (HASH, "candela_sha1", |a| {
        Ret::Str(hex(&digest::sha1(a[0].bytes())))
    }),
    (HASH, "candela_sha256", |a| {
        Ret::Str(hex(&digest::sha256(a[0].bytes())))
    }),
];

/// The intrinsic that stands for `symbol` in `library`, where `library` is the
/// path relative to the `libs` directory a std module's binding records.
#[must_use]
pub fn lookup(library: &str, symbol: &str) -> Option<Intrinsic> {
    TABLE
        .iter()
        .find(|(lib, sym, _)| *lib == library && *sym == symbol)
        .map(|(_, _, f)| *f)
}

// ---------------------------------------------------------------------------
// math
// ---------------------------------------------------------------------------

/// An exponent wider than an `i32` says the same thing as the largest one that
/// fits, as the C library's wrapper has it.
fn clamp_exponent(y: i64) -> i32 {
    y.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// The unbiased exponent of `x` as a float, as C's `logb` gives it.
fn logb(x: f64) -> f64 {
    if x == 0.0 {
        return f64::NEG_INFINITY;
    }
    if !x.is_finite() {
        return x * x;
    }
    f64::from(libm::ilogb(x))
}

// ---------------------------------------------------------------------------
// time
// ---------------------------------------------------------------------------

/// The wall clock and the local calendar, read from the browser.
#[cfg(target_arch = "wasm32")]
mod clock {
    use super::Tm;

    /// Seconds since the epoch.
    pub fn now() -> i64 {
        (js_sys::Date::now() / 1000.0).floor() as i64
    }

    /// `timestamp` broken down in the browser's local time zone.
    pub fn local(timestamp: i64) -> Tm {
        let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(timestamp as f64 * 1000.0));
        Tm {
            year: i64::from(date.get_full_year()),
            month: date.get_month(),
            day: date.get_date(),
            hour: date.get_hours(),
            minute: date.get_minutes(),
            second: date.get_seconds(),
            weekday: date.get_day(),
            // JavaScript counts minutes west of UTC; `%z` wants east.
            offset_minutes: -(date.get_timezone_offset() as i32),
            timestamp,
        }
    }
}

/// The clock off the browser, where only the unit tests reach this module.
/// It reads UTC.
#[cfg(not(target_arch = "wasm32"))]
mod clock {
    use super::Tm;

    pub fn now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64)
    }

    pub fn local(timestamp: i64) -> Tm {
        Tm::utc(timestamp)
    }
}

/// A moment broken down into calendar fields, the way C's `struct tm` holds
/// it: `month` from 0, `weekday` from 0 for Sunday.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tm {
    year: i64,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
    weekday: u32,
    offset_minutes: i32,
    timestamp: i64,
}

impl Tm {
    /// `timestamp` in UTC.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn utc(timestamp: i64) -> Self {
        let days = timestamp.div_euclid(86_400);
        let secs = timestamp.rem_euclid(86_400) as u32;
        // Howard Hinnant's days-to-civil.
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        let year = yoe + era * 400 + i64::from(month <= 2);
        Self {
            year,
            month: month - 1,
            day,
            hour: secs / 3600,
            minute: secs / 60 % 60,
            second: secs % 60,
            weekday: (days + 4).rem_euclid(7) as u32,
            offset_minutes: 0,
            timestamp,
        }
    }

    const fn is_leap(&self) -> bool {
        (self.year % 4 == 0 && self.year % 100 != 0) || self.year % 400 == 0
    }

    /// Day of the year, from 0.
    fn yday(&self) -> u32 {
        const BEFORE: [u32; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
        BEFORE[self.month as usize] + self.day - 1 + u32::from(self.month > 1 && self.is_leap())
    }
}

const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// The longest result the C library's buffer holds; past it, nothing renders.
const FORMAT_LIMIT: usize = 127;

fn time_format(timestamp: i64, pattern: Arg<'_>) -> String {
    let Arg::Str(pattern) = pattern else {
        return String::new();
    };
    let out = strftime(&clock::local(timestamp), pattern);
    if out.len() > FORMAT_LIMIT {
        return String::new();
    }
    out
}

/// Renders `tm` through a C `strftime` pattern in the "C" locale.
///
/// `%Z` has no zone name to give in a browser, so it renders the offset the
/// way `%z` does.
#[must_use]
pub fn strftime(tm: &Tm, pattern: &str) -> String {
    use std::fmt::Write;

    let mut out = String::with_capacity(pattern.len() + 16);
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let Some(directive) = chars.next() else {
            out.push('%');
            break;
        };
        let hour12 = match tm.hour % 12 {
            0 => 12,
            h => h,
        };
        let _ = match directive {
            'a' => write!(out, "{}", &WEEKDAYS[tm.weekday as usize][..3]),
            'A' => write!(out, "{}", WEEKDAYS[tm.weekday as usize]),
            'b' | 'h' => write!(out, "{}", &MONTHS[tm.month as usize][..3]),
            'B' => write!(out, "{}", MONTHS[tm.month as usize]),
            'c' => write!(out, "{}", strftime(tm, "%a %b %e %H:%M:%S %Y")),
            'C' => write!(out, "{:02}", tm.year.div_euclid(100)),
            'd' => write!(out, "{:02}", tm.day),
            'D' | 'x' => write!(out, "{}", strftime(tm, "%m/%d/%y")),
            'e' => write!(out, "{:2}", tm.day),
            'F' => write!(out, "{}", strftime(tm, "%Y-%m-%d")),
            'H' => write!(out, "{:02}", tm.hour),
            'I' => write!(out, "{hour12:02}"),
            'j' => write!(out, "{:03}", tm.yday() + 1),
            'm' => write!(out, "{:02}", tm.month + 1),
            'M' => write!(out, "{:02}", tm.minute),
            'n' => out.write_char('\n'),
            'p' => write!(out, "{}", if tm.hour < 12 { "AM" } else { "PM" }),
            'r' => write!(out, "{}", strftime(tm, "%I:%M:%S %p")),
            'R' => write!(out, "{}", strftime(tm, "%H:%M")),
            's' => write!(out, "{}", tm.timestamp),
            'S' => write!(out, "{:02}", tm.second),
            't' => out.write_char('\t'),
            'T' | 'X' => write!(out, "{}", strftime(tm, "%H:%M:%S")),
            'u' => write!(out, "{}", if tm.weekday == 0 { 7 } else { tm.weekday }),
            'U' => write!(out, "{:02}", (tm.yday() + 7 - tm.weekday) / 7),
            'w' => write!(out, "{}", tm.weekday),
            'W' => write!(out, "{:02}", (tm.yday() + 7 - (tm.weekday + 6) % 7) / 7),
            'y' => write!(out, "{:02}", tm.year.rem_euclid(100)),
            'Y' => write!(out, "{}", tm.year),
            'z' | 'Z' => {
                let sign = if tm.offset_minutes < 0 { '-' } else { '+' };
                let offset = tm.offset_minutes.unsigned_abs();
                write!(out, "{sign}{:02}{:02}", offset / 60, offset % 60)
            }
            '%' => out.write_char('%'),
            other => write!(out, "%{other}"),
        };
    }
    out
}

// ---------------------------------------------------------------------------
// random
// ---------------------------------------------------------------------------

/// PCG32, the generator the C library uses, so a seeded run draws the same
/// sequence here as it does on a desktop.
#[derive(Clone, Copy, Debug)]
struct Pcg32 {
    state: u64,
    inc: u64,
}

impl Pcg32 {
    const MULTIPLIER: u64 = 6_364_136_223_846_793_005;

    /// `pcg32_srandom_r`.
    const fn new(init_state: u64, init_seq: u64) -> Self {
        let mut rng = Self {
            state: 0,
            inc: (init_seq << 1) | 1,
        };
        rng.next();
        rng.state = rng.state.wrapping_add(init_state);
        rng.next();
        rng
    }

    /// `pcg32_random_r`.
    const fn next(&mut self) -> u32 {
        let old = self.state;
        self.state = old.wrapping_mul(Self::MULTIPLIER).wrapping_add(self.inc);
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        let rot = (old >> 59) as u32;
        xorshifted.rotate_right(rot)
    }
}

thread_local! {
    /// The program's one generator; empty until the first draw or `seed`.
    static RNG: Cell<Option<Pcg32>> = const { Cell::new(None) };
}

/// A generator seeded from the platform's entropy, for a run that never calls
/// `seed`.
fn entropy_seeded() -> Pcg32 {
    let mut bytes = [0_u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        // No entropy source: fall back on the clock, as the C library does.
        bytes[..8].copy_from_slice(&clock::now().to_le_bytes());
    }
    let (state, seq) = bytes.split_at(8);
    Pcg32::new(
        u64::from_le_bytes(state.try_into().unwrap_or_default()),
        u64::from_le_bytes(seq.try_into().unwrap_or_default()),
    )
}

fn draw32() -> u32 {
    let mut rng = RNG.get().unwrap_or_else(entropy_seeded);
    let value = rng.next();
    RNG.set(Some(rng));
    value
}

/// The generator draws 32 bits at a time, so a full-width draw is two of them.
fn draw64() -> u64 {
    let high = u64::from(draw32());
    (high << 32) | u64::from(draw32())
}

fn random_int_range(min: i64, max: i64) -> i64 {
    let span = (max as u64).wrapping_sub(min as u64).wrapping_add(1);
    if span == 0 {
        return draw64() as i64;
    }
    // Rejection sampling: drop the draws in the short tail so every value in
    // the span comes up equally often.
    let threshold = 0_u64.wrapping_sub(span) % span;
    loop {
        let r = draw64();
        if r >= threshold {
            return min.wrapping_add((r % span) as i64);
        }
    }
}

fn random_unit() -> f64 {
    f64::from(draw32()) / 4_294_967_296.0
}

// ---------------------------------------------------------------------------
// hash
// ---------------------------------------------------------------------------

/// `bytes` as lowercase hex, two digits a byte.
fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from(DIGITS[usize::from(b >> 4)]));
        out.push(char::from(DIGITS[usize::from(b & 15)]));
    }
    out
}

/// MD5 (RFC 1321), SHA-1 and SHA-256 (FIPS 180-4), the same three the C
/// library behind `std/hash` computes.
mod digest {
    /// Feeds `msg` through `block` in 64-byte blocks with the padding all three
    /// share: a 0x80 byte, zeros, then the length in bits as eight bytes,
    /// little-endian for MD5 and big-endian for the SHA family.
    fn run(msg: &[u8], big_endian: bool, mut block: impl FnMut(&[u8; 64])) {
        let (blocks, rest) = msg.as_chunks::<64>();
        for chunk in blocks {
            block(chunk);
        }
        let mut tail = [0u8; 128];
        tail[..rest.len()].copy_from_slice(rest);
        tail[rest.len()] = 0x80;
        let tail_len = if rest.len() < 56 { 64 } else { 128 };
        let bits = (msg.len() as u64).wrapping_mul(8);
        let len_bytes = if big_endian {
            bits.to_be_bytes()
        } else {
            bits.to_le_bytes()
        };
        tail[tail_len - 8..tail_len].copy_from_slice(&len_bytes);
        for chunk in tail[..tail_len].as_chunks::<64>().0 {
            block(chunk);
        }
    }

    const MD5_K: [u32; 64] = [
        0xd76a_a478,
        0xe8c7_b756,
        0x2420_70db,
        0xc1bd_ceee,
        0xf57c_0faf,
        0x4787_c62a,
        0xa830_4613,
        0xfd46_9501,
        0x6980_98d8,
        0x8b44_f7af,
        0xffff_5bb1,
        0x895c_d7be,
        0x6b90_1122,
        0xfd98_7193,
        0xa679_438e,
        0x49b4_0821,
        0xf61e_2562,
        0xc040_b340,
        0x265e_5a51,
        0xe9b6_c7aa,
        0xd62f_105d,
        0x0244_1453,
        0xd8a1_e681,
        0xe7d3_fbc8,
        0x21e1_cde6,
        0xc337_07d6,
        0xf4d5_0d87,
        0x455a_14ed,
        0xa9e3_e905,
        0xfcef_a3f8,
        0x676f_02d9,
        0x8d2a_4c8a,
        0xfffa_3942,
        0x8771_f681,
        0x6d9d_6122,
        0xfde5_380c,
        0xa4be_ea44,
        0x4bde_cfa9,
        0xf6bb_4b60,
        0xbebf_bc70,
        0x289b_7ec6,
        0xeaa1_27fa,
        0xd4ef_3085,
        0x0488_1d05,
        0xd9d4_d039,
        0xe6db_99e5,
        0x1fa2_7cf8,
        0xc4ac_5665,
        0xf429_2244,
        0x432a_ff97,
        0xab94_23a7,
        0xfc93_a039,
        0x655b_59c3,
        0x8f0c_cc92,
        0xffef_f47d,
        0x8584_5dd1,
        0x6fa8_7e4f,
        0xfe2c_e6e0,
        0xa301_4314,
        0x4e08_11a1,
        0xf753_7e82,
        0xbd3a_f235,
        0x2ad7_d2bb,
        0xeb86_d391,
    ];

    const MD5_S: [u32; 16] = [7, 12, 17, 22, 5, 9, 14, 20, 4, 11, 16, 23, 6, 10, 15, 21];

    pub fn md5(msg: &[u8]) -> [u8; 16] {
        let mut h: [u32; 4] = [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
        run(msg, false, |p| {
            let m: [u32; 16] = std::array::from_fn(|i| u32::from_le_bytes(p.as_chunks::<4>().0[i]));
            let [mut a, mut b, mut c, mut d] = h;
            for i in 0..64 {
                let (f, g) = match i / 16 {
                    0 => ((b & c) | (!b & d), i),
                    1 => ((d & b) | (!d & c), (5 * i + 1) & 15),
                    2 => (b ^ c ^ d, (3 * i + 5) & 15),
                    _ => (c ^ (b | !d), (7 * i) & 15),
                };
                let next = d;
                d = c;
                c = b;
                b = b.wrapping_add(
                    a.wrapping_add(f)
                        .wrapping_add(MD5_K[i])
                        .wrapping_add(m[g])
                        .rotate_left(MD5_S[(i / 16) * 4 + i % 4]),
                );
                a = next;
            }
            for (word, add) in h.iter_mut().zip([a, b, c, d]) {
                *word = word.wrapping_add(add);
            }
        });
        let mut out = [0u8; 16];
        for (chunk, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(h) {
            *chunk = word.to_le_bytes();
        }
        out
    }

    fn words_be(p: &[u8; 64]) -> [u32; 16] {
        std::array::from_fn(|i| u32::from_be_bytes(p.as_chunks::<4>().0[i]))
    }

    fn store_be<const N: usize, const B: usize>(h: [u32; N]) -> [u8; B] {
        let mut out = [0u8; B];
        for (chunk, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(h) {
            *chunk = word.to_be_bytes();
        }
        out
    }

    pub fn sha1(msg: &[u8]) -> [u8; 20] {
        let mut h: [u32; 5] = [
            0x6745_2301,
            0xefcd_ab89,
            0x98ba_dcfe,
            0x1032_5476,
            0xc3d2_e1f0,
        ];
        run(msg, true, |p| {
            let mut w = [0u32; 80];
            w[..16].copy_from_slice(&words_be(p));
            for i in 16..80 {
                w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
            }
            let [mut a, mut b, mut c, mut d, mut e] = h;
            for (i, wi) in w.iter().enumerate() {
                let (f, k) = match i / 20 {
                    0 => ((b & c) | (!b & d), 0x5a82_7999),
                    1 => (b ^ c ^ d, 0x6ed9_eba1),
                    2 => ((b & c) | (b & d) | (c & d), 0x8f1b_bcdc),
                    _ => (b ^ c ^ d, 0xca62_c1d6),
                };
                let t = a
                    .rotate_left(5)
                    .wrapping_add(f)
                    .wrapping_add(e)
                    .wrapping_add(k)
                    .wrapping_add(*wi);
                e = d;
                d = c;
                c = b.rotate_left(30);
                b = a;
                a = t;
            }
            for (word, add) in h.iter_mut().zip([a, b, c, d, e]) {
                *word = word.wrapping_add(add);
            }
        });
        store_be(h)
    }

    const SHA256_K: [u32; 64] = [
        0x428a_2f98,
        0x7137_4491,
        0xb5c0_fbcf,
        0xe9b5_dba5,
        0x3956_c25b,
        0x59f1_11f1,
        0x923f_82a4,
        0xab1c_5ed5,
        0xd807_aa98,
        0x1283_5b01,
        0x2431_85be,
        0x550c_7dc3,
        0x72be_5d74,
        0x80de_b1fe,
        0x9bdc_06a7,
        0xc19b_f174,
        0xe49b_69c1,
        0xefbe_4786,
        0x0fc1_9dc6,
        0x240c_a1cc,
        0x2de9_2c6f,
        0x4a74_84aa,
        0x5cb0_a9dc,
        0x76f9_88da,
        0x983e_5152,
        0xa831_c66d,
        0xb003_27c8,
        0xbf59_7fc7,
        0xc6e0_0bf3,
        0xd5a7_9147,
        0x06ca_6351,
        0x1429_2967,
        0x27b7_0a85,
        0x2e1b_2138,
        0x4d2c_6dfc,
        0x5338_0d13,
        0x650a_7354,
        0x766a_0abb,
        0x81c2_c92e,
        0x9272_2c85,
        0xa2bf_e8a1,
        0xa81a_664b,
        0xc24b_8b70,
        0xc76c_51a3,
        0xd192_e819,
        0xd699_0624,
        0xf40e_3585,
        0x106a_a070,
        0x19a4_c116,
        0x1e37_6c08,
        0x2748_774c,
        0x34b0_bcb5,
        0x391c_0cb3,
        0x4ed8_aa4a,
        0x5b9c_ca4f,
        0x682e_6ff3,
        0x748f_82ee,
        0x78a5_636f,
        0x84c8_7814,
        0x8cc7_0208,
        0x90be_fffa,
        0xa450_6ceb,
        0xbef9_a3f7,
        0xc671_78f2,
    ];

    pub fn sha256(msg: &[u8]) -> [u8; 32] {
        let mut h: [u32; 8] = [
            0x6a09_e667,
            0xbb67_ae85,
            0x3c6e_f372,
            0xa54f_f53a,
            0x510e_527f,
            0x9b05_688c,
            0x1f83_d9ab,
            0x5be0_cd19,
        ];
        run(msg, true, |p| {
            let mut w = [0u32; 64];
            w[..16].copy_from_slice(&words_be(p));
            for i in 16..64 {
                let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
                let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
                w[i] = w[i - 16]
                    .wrapping_add(s0)
                    .wrapping_add(w[i - 7])
                    .wrapping_add(s1);
            }
            let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
            for (k, wi) in SHA256_K.iter().zip(w) {
                let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
                let ch = (e & f) ^ (!e & g);
                let t1 = hh
                    .wrapping_add(s1)
                    .wrapping_add(ch)
                    .wrapping_add(*k)
                    .wrapping_add(wi);
                let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
                let maj = (a & b) ^ (a & c) ^ (b & c);
                let t2 = s0.wrapping_add(maj);
                hh = g;
                g = f;
                f = e;
                e = d.wrapping_add(t1);
                d = c;
                c = b;
                b = a;
                a = t1.wrapping_add(t2);
            }
            for (word, add) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
                *word = word.wrapping_add(add);
            }
        });
        store_be(h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(library: &str, symbol: &str, args: &[Arg<'_>]) -> Ret {
        lookup(library, symbol).expect("bound")(args)
    }

    #[test]
    fn every_std_binding_has_an_intrinsic() {
        let std_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../libs/std");
        for (module, library) in [
            ("math", MATH),
            ("time", TIME),
            ("random", RANDOM),
            ("hash", HASH),
        ] {
            let source = std::fs::read_to_string(std_dir.join(format!("{module}.cdl"))).unwrap();
            let block = source.split('{').nth(1).unwrap().split('}').next().unwrap();
            for line in block.lines().map(str::trim).filter(|l| !l.is_empty()) {
                let name = line.split('(').next().unwrap();
                let symbol = name.rsplit(' ').next().unwrap();
                assert!(
                    lookup(library, symbol).is_some(),
                    "{module}: {symbol} has no intrinsic"
                );
            }
        }
    }

    #[test]
    fn math_matches_the_c_library() {
        let f = |x| [Arg::Float(x)];
        assert_eq!(call(MATH, "candela_sqrt", &f(16.0)), Ret::Float(4.0));
        assert_eq!(call(MATH, "candela_sin", &f(0.0)), Ret::Float(0.0));
        assert_eq!(call(MATH, "candela_round", &f(-2.5)), Ret::Float(-3.0));
        assert_eq!(call(MATH, "candela_logb", &f(8.0)), Ret::Float(3.0));
        assert_eq!(
            call(MATH, "candela_logb", &f(0.0)),
            Ret::Float(f64::NEG_INFINITY)
        );
        assert_eq!(call(MATH, "candela_ilogb", &f(1024.0)), Ret::Int(10));
        assert_eq!(
            call(
                MATH,
                "candela_ldexp",
                &[Arg::Float(1.0), Arg::Int(i64::MAX)]
            ),
            Ret::Float(f64::INFINITY)
        );
        assert_eq!(
            call(MATH, "candela_fmod", &[Arg::Float(7.5), Arg::Float(2.0)]),
            Ret::Float(1.5)
        );
        assert_eq!(call(MATH, "candela_erf", &f(0.0)), Ret::Float(0.0));
        let Ret::Float(nan) = call(MATH, "candela_sqrt", &f(-1.0)) else {
            unreachable!()
        };
        assert!(nan.is_nan());
    }

    /// The published test vectors, plus lengths either side of the one-block
    /// and two-block padding boundaries.
    #[test]
    fn digests_match_the_reference_vectors() {
        let digest = |symbol, text: &str| match call(HASH, symbol, &[Arg::Str(text)]) {
            Ret::Str(s) => s,
            other => panic!("{symbol}: {other:?}"),
        };
        assert_eq!(
            digest("candela_md5", ""),
            "d41d8cd98f00b204e9800998ecf8427e"
        );
        assert_eq!(
            digest("candela_md5", "abc"),
            "900150983cd24fb0d6963f7d28e17f72"
        );
        assert_eq!(
            digest("candela_sha1", "abc"),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(
            digest("candela_sha256", "abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let two_blocks = "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
        assert_eq!(
            digest("candela_sha1", two_blocks),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        assert_eq!(
            digest("candela_sha256", two_blocks),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        assert_eq!(
            digest(
                "candela_md5",
                "12345678901234567890123456789012345678901234567890123456789012345678901234567890"
            ),
            "57edf4a22be3c955ac49da2e2107b67a"
        );
        // The text's UTF-8 bytes, two of them for this one character.
        assert_eq!(
            digest("candela_md5", "\u{e9}"),
            "66ddcd97cfdeabb2f6fb8a999b4bc76f"
        );
        let million_a = "a".repeat(1_000_000);
        assert_eq!(
            digest("candela_sha256", &million_a),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
        assert_eq!(
            digest("candela_sha1", &million_a),
            "34aa973cd4c4daa4f61eeb2bdbad27316534016f"
        );
        // A run of `a` either side of where the length field stops fitting
        // in the last block.
        let boundaries = [
            (
                55,
                "ef1772b6dff9a122358552954ad0df65",
                "c1c8bbdc22796e28c0e15163d20899b65621d65a",
                "9f4390f8d30c2dd92ec9f095b65e2b9ae9b0a925a5258e241c9f1e910f734318",
            ),
            (
                56,
                "3b0c8ac703f828b04c6c197006d17218",
                "c2db330f6083854c99d4b5bfb6e8f29f201be699",
                "b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a",
            ),
            (
                64,
                "014842d480b571495a4a0363793f7367",
                "0098ba824b5c16427bd7a1122a5a442a25ec644d",
                "ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb",
            ),
            (
                120,
                "5f61c0ccad4cac44c75ff505e1f1e537",
                "f34c1488385346a55709ba056ddd08280dd4c6d6",
                "2f3d335432c70b580af0e8e1b3674a7c020d683aa5f73aaaedfdc55af904c21c",
            ),
        ];
        for (n, md5, sha1, sha256) in boundaries {
            let text = "a".repeat(n);
            assert_eq!(digest("candela_md5", &text), md5, "md5 of {n}");
            assert_eq!(digest("candela_sha1", &text), sha1, "sha1 of {n}");
            assert_eq!(digest("candela_sha256", &text), sha256, "sha256 of {n}");
        }
    }

    /// The reference output of the PCG32 demo program for seed 42, sequence 54.
    #[test]
    fn a_seeded_generator_draws_the_reference_sequence() {
        let mut rng = Pcg32::new(42, 54);
        let drawn: Vec<u32> = (0..6).map(|_| rng.next()).collect();
        assert_eq!(
            drawn,
            [
                0xa15c_02b7,
                0x7b47_f409,
                0xba1d_3330,
                0x83d2_f293,
                0xbfa4_784b,
                0xcbed_606e
            ]
        );
    }

    #[test]
    fn a_range_holds_both_ends_and_nothing_past_them() {
        call(RANDOM, "candela_seed", &[Arg::Int(7)]);
        for _ in 0..1000 {
            let Ret::Int(n) = call(
                RANDOM,
                "candela_random_int_range",
                &[Arg::Int(1), Arg::Int(6)],
            ) else {
                unreachable!()
            };
            assert!((1..=6).contains(&n));
            let Ret::Float(x) = call(RANDOM, "candela_random", &[]) else {
                unreachable!()
            };
            assert!((0.0..1.0).contains(&x));
        }
    }

    #[test]
    fn strftime_renders_the_c_directives() {
        // 2001-09-09 01:46:40 UTC, a Sunday.
        let tm = Tm::utc(1_000_000_000);
        assert_eq!(strftime(&tm, "%Y-%m-%d %H:%M:%S"), "2001-09-09 01:46:40");
        assert_eq!(
            strftime(&tm, "%a %A %b %B %j %p %I %e"),
            "Sun Sunday Sep September 252 AM 01  9"
        );
        assert_eq!(
            strftime(&tm, "%F %T %D %u %w %y %C %%"),
            "2001-09-09 01:46:40 09/09/01 7 0 01 20 %"
        );
        assert_eq!(strftime(&tm, "%U %W %z %s"), "36 36 +0000 1000000000");
        assert_eq!(strftime(&Tm::utc(0), "%c"), "Thu Jan  1 00:00:00 1970");
        assert_eq!(strftime(&Tm::utc(951_782_400), "%F %j"), "2000-02-29 060");
    }
}
