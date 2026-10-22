//! MD5 (RFC 1321), SHA-1 and SHA-256 (FIPS 180-4), the same three the C
//! library behind `std/hash` computes.
//!
//! `fs::digest` streams a file through these, and a browser build answers
//! `std/hash` with them where it cannot load the C library.

use std::io::Read;

/// `bytes` as lowercase hex, two digits a byte.
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from(DIGITS[usize::from(b >> 4)]));
        out.push(char::from(DIGITS[usize::from(b & 15)]));
    }
    out
}

/// The MD5 digest of `msg`.
#[cfg(any(target_arch = "wasm32", test))]
#[must_use]
pub fn md5(msg: &[u8]) -> [u8; 16] {
    md5_of(msg).unwrap_or_default()
}

/// The SHA-1 digest of `msg`.
#[cfg(any(target_arch = "wasm32", test))]
#[must_use]
pub fn sha1(msg: &[u8]) -> [u8; 20] {
    sha1_of(msg).unwrap_or_default()
}

/// The SHA-256 digest of `msg`.
#[cfg(any(target_arch = "wasm32", test))]
#[must_use]
pub fn sha256(msg: &[u8]) -> [u8; 32] {
    sha256_of(msg).unwrap_or_default()
}

/// The digest of everything `input` reads under `algorithm`, `"md5"`,
/// `"sha1"` or `"sha256"`, as lowercase hex. `None` for another algorithm.
pub fn hex_digest(algorithm: &str, input: impl Read) -> Option<std::io::Result<String>> {
    Some(match algorithm {
        "md5" => md5_of(input).map(|d| hex(&d)),
        "sha1" => sha1_of(input).map(|d| hex(&d)),
        "sha256" => sha256_of(input).map(|d| hex(&d)),
        _ => return None,
    })
}

/// Feeds what `input` reads through `block` in 64-byte blocks, with the
/// padding all three share: a 0x80 byte, zeros, then the length in bits as
/// eight bytes, little-endian for MD5 and big-endian for the SHA family.
///
/// The input streams through a buffer, so a file never sits in memory whole.
fn run(
    mut input: impl Read,
    big_endian: bool,
    mut block: impl FnMut(&[u8; 64]),
) -> std::io::Result<()> {
    let mut buf = [0u8; 64 * 64];
    let mut pending = 0;
    let mut total: u64 = 0;
    loop {
        let n = match input.read(&mut buf[pending..]) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        total += n as u64;
        pending += n;
        let full = pending / 64 * 64;
        for chunk in buf[..full].as_chunks::<64>().0 {
            block(chunk);
        }
        buf.copy_within(full..pending, 0);
        pending -= full;
    }
    let mut tail = [0u8; 128];
    tail[..pending].copy_from_slice(&buf[..pending]);
    tail[pending] = 0x80;
    let tail_len = if pending < 56 { 64 } else { 128 };
    let bits = total.wrapping_mul(8);
    let len_bytes = if big_endian {
        bits.to_be_bytes()
    } else {
        bits.to_le_bytes()
    };
    tail[tail_len - 8..tail_len].copy_from_slice(&len_bytes);
    for chunk in tail[..tail_len].as_chunks::<64>().0 {
        block(chunk);
    }
    Ok(())
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

pub fn md5_of(input: impl Read) -> std::io::Result<[u8; 16]> {
    let mut h: [u32; 4] = [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
    run(input, false, |p| {
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
    })?;
    let mut out = [0u8; 16];
    for (chunk, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(h) {
        *chunk = word.to_le_bytes();
    }
    Ok(out)
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

pub fn sha1_of(input: impl Read) -> std::io::Result<[u8; 20]> {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xefcd_ab89,
        0x98ba_dcfe,
        0x1032_5476,
        0xc3d2_e1f0,
    ];
    run(input, true, |p| {
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
    })?;
    Ok(store_be(h))
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

pub fn sha256_of(input: impl Read) -> std::io::Result<[u8; 32]> {
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
    run(input, true, |p| {
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
    })?;
    Ok(store_be(h))
}
