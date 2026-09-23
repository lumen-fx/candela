//! The standard library's native functions, built into the runtime for targets
//! that cannot load a dynamic library.
//!
//! `std/math`, `std/time` and `std/random` reach their work through a `dylib`
//! block naming a small C library that ships beside the toolchain. A browser
//! has no such library and no loader, so on wasm32 the same bindings resolve
//! to the Rust functions here instead: the std modules and every program that
//! imports them read the same on every target. Each binding is found by the
//! library the std module names and the symbol it declares, so a `.cdlb` built
//! on a desktop that records those bindings runs here unchanged.

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

#[cfg(test)]
mod tests {
    use super::*;

    fn call(library: &str, symbol: &str, args: &[Arg<'_>]) -> Ret {
        lookup(library, symbol).expect("bound")(args)
    }

    #[test]
    fn every_std_binding_has_an_intrinsic() {
        let std_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../libs/std");
        for (module, library) in [("math", MATH), ("time", TIME), ("random", RANDOM)] {
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
