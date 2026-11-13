// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 RxMiner contributors
//! Integer implementation of the binary64 operations used by RandomX.
//! Rounding is explicit; hashing neither reads nor changes the host FP state.

const SIGN: u64 = 1 << 63;
const FRAC: u64 = (1 << 52) - 1;
const INF: u64 = 0x7ff0000000000000;
const NAN: u64 = 0xfff8000000000000;

/// RandomX encoding: nearest-even, toward -infinity, toward +infinity, toward 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Rounding {
    #[default]
    Nearest,
    Down,
    Up,
    Zero,
}
impl Rounding {
    pub fn from_bits(value: u64) -> Self {
        match value & 3 {
            0 => Self::Nearest,
            1 => Self::Down,
            2 => Self::Up,
            _ => Self::Zero,
        }
    }
    fn away(self, negative: bool) -> bool {
        matches!((self, negative), (Self::Down, true) | (Self::Up, false))
    }
}

fn decode(bits: u64) -> (bool, u128, i32) {
    let mut mantissa = u128::from(bits & FRAC);
    let mut exponent = ((bits >> 52) & 2047) as i32 - 1023;
    if exponent == -1023 {
        exponent = -1022;
        if mantissa != 0 {
            let shift = mantissa.leading_zeros() - 75;
            mantissa <<= shift;
            exponent -= shift as i32;
        }
    } else {
        mantissa |= 1 << 52;
    }
    (bits & SIGN != 0, mantissa, exponent)
}

fn jam(value: u128, shift: u32) -> u128 {
    if shift == 0 {
        value
    } else if shift >= 128 {
        u128::from(value != 0)
    } else {
        (value >> shift) | u128::from(value << (128 - shift) != 0)
    }
}

// Round an unsigned significand times 2^exponent; operations retain enough low
// bits to distinguish halfway cases before calling this common packing path.
fn pack(negative: bool, value: u128, exponent: i32, mode: Rounding) -> u64 {
    let sign = if negative { SIGN } else { 0 };
    if value == 0 {
        return sign;
    }
    let top = 127 - value.leading_zeros() as i32;
    let shift = (top - 52).max(-1074 - exponent);
    let mut unit_exp = exponent + shift;
    let mut rounded;
    if shift <= 0 {
        rounded = value << (-shift);
    } else {
        let (integer, remainder, half) = if shift > 128 {
            (0, value, u128::MAX)
        } else if shift == 128 {
            (0, value, 1 << 127)
        } else {
            (
                value >> shift,
                value & ((1u128 << shift) - 1),
                1u128 << (shift - 1),
            )
        };
        let increment = if mode == Rounding::Nearest {
            shift <= 128 && (remainder > half || (remainder == half && integer & 1 != 0))
        } else {
            remainder != 0 && mode.away(negative)
        };
        rounded = integer + u128::from(increment);
    }
    if rounded >= 1 << 53 {
        rounded >>= 1;
        unit_exp += 1;
    }
    let biased = unit_exp + 52 + 1023;
    if biased >= 2047 {
        return sign
            | if mode == Rounding::Nearest || mode.away(negative) {
                INF
            } else {
                INF - 1
            };
    }
    if rounded < 1 << 52 {
        return sign | rounded as u64;
    }
    sign | ((biased as u64) << 52) | (rounded as u64 & FRAC)
}

fn nan(a: u64, b: u64) -> Option<u64> {
    if a & !SIGN > INF {
        Some(a | (1 << 51))
    } else if b & !SIGN > INF {
        Some(b | (1 << 51))
    } else {
        None
    }
}

pub fn from_i32(value: i32) -> u64 {
    pack(
        value < 0,
        u128::from(value.unsigned_abs()),
        0,
        Rounding::Nearest,
    )
}

pub fn add(a: u64, b: u64, mode: Rounding) -> u64 {
    if let Some(nan) = nan(a, b) {
        return nan;
    }
    if a & !SIGN == INF || b & !SIGN == INF {
        return if a & !SIGN == INF && b == (a ^ SIGN) {
            NAN
        } else if a & !SIGN == INF {
            a
        } else {
            b
        };
    }
    let (mut sa, mut ma, mut ea) = decode(a);
    let (mut sb, mut mb, mut eb) = decode(b);
    if ma == 0 && mb == 0 {
        return if (sa && sb) || (sa != sb && mode == Rounding::Down) {
            SIGN
        } else {
            0
        };
    }
    if ma == 0 {
        return b;
    }
    if mb == 0 {
        return a;
    }
    if (ea, ma) < (eb, mb) {
        std::mem::swap(&mut sa, &mut sb);
        std::mem::swap(&mut ma, &mut mb);
        std::mem::swap(&mut ea, &mut eb);
    }
    ma <<= 3;
    mb = jam(mb << 3, (ea - eb) as u32);
    let value = if sa == sb { ma + mb } else { ma - mb };
    if value == 0 {
        return if mode == Rounding::Down { SIGN } else { 0 };
    }
    pack(sa, value, ea - 55, mode)
}

pub fn sub(a: u64, b: u64, mode: Rounding) -> u64 {
    add(a, b ^ SIGN, mode)
}

pub fn mul(a: u64, b: u64, mode: Rounding) -> u64 {
    if let Some(nan) = nan(a, b) {
        return nan;
    }
    let sign = (a ^ b) & SIGN;
    if a & !SIGN == INF || b & !SIGN == INF {
        return if a & !SIGN == 0 || b & !SIGN == 0 {
            NAN
        } else {
            sign | INF
        };
    }
    let (_, ma, ea) = decode(a);
    let (_, mb, eb) = decode(b);
    pack(sign != 0, ma * mb, ea + eb - 104, mode)
}

pub fn div(a: u64, b: u64, mode: Rounding) -> u64 {
    if let Some(nan) = nan(a, b) {
        return nan;
    }
    let sign = (a ^ b) & SIGN;
    let aa = a & !SIGN;
    let bb = b & !SIGN;
    if (aa == 0 && bb == 0) || (aa == INF && bb == INF) {
        return NAN;
    }
    if aa == INF || bb == 0 {
        return sign | INF;
    }
    if aa == 0 || bb == INF {
        return sign;
    }
    let (_, ma, ea) = decode(a);
    let (_, mb, eb) = decode(b);
    let numerator = ma << 64;
    let value = (numerator / mb) | u128::from(numerator % mb != 0);
    pack(sign != 0, value, ea - eb - 64, mode)
}

pub fn sqrt(a: u64, mode: Rounding) -> u64 {
    if let Some(nan) = nan(a, 0) {
        return nan;
    }
    if a & !SIGN == 0 {
        return a;
    }
    if a & SIGN != 0 {
        return NAN;
    }
    if a == INF {
        return a;
    }
    let (_, mut mantissa, mut exponent) = decode(a);
    if exponent & 1 != 0 {
        mantissa <<= 1;
        exponent -= 1;
    }
    let radicand = mantissa << 74;
    let root = radicand.isqrt();
    let value = root | u128::from(root * root != radicand);
    pack(false, value, exponent / 2 - 63, mode)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nearest_matches_host_arithmetic() {
        let mut state = 0x3378273bc47f3a91u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..20000 {
            let a = next() & 0xffefffffffffffff;
            let b = next() & 0xffefffffffffffff;
            let x = f64::from_bits(a);
            let y = f64::from_bits(b);
            for (actual, expected) in [
                (add(a, b, Rounding::Nearest), (x + y).to_bits()),
                (sub(a, b, Rounding::Nearest), (x - y).to_bits()),
                (mul(a, b, Rounding::Nearest), (x * y).to_bits()),
                (div(a, b, Rounding::Nearest), (x / y).to_bits()),
                (sqrt(a & !SIGN, Rounding::Nearest), x.abs().sqrt().to_bits()),
            ] {
                assert_eq!(actual, expected, "a={a:x} b={b:x}");
            }
        }
    }
}
