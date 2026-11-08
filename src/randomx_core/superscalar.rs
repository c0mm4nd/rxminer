// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2018-2019, tevador <tevador@gmail.com>
// Copyright (c) 2026 RxMiner contributors
// Port of the RandomX superscalar executor and reciprocal; see LICENSE here.

//! Validated integer programs used during dataset construction.
//! Program generation/scheduling is migrated separately.
use core::fmt;

/// Return the multiplier for IMUL_RCP. Zero and powers of two are not divisors
/// emitted by the superscalar generator and are rejected at this boundary.
pub fn reciprocal(divisor: u32) -> Option<u64> {
    if divisor == 0 || divisor.is_power_of_two() {
        return None;
    }
    let exponent = 63 + (32 - divisor.leading_zeros());
    Some(((1u128 << exponent) / u128::from(divisor)) as u64)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidProgram;
impl fmt::Display for InvalidProgram {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid superscalar program")
    }
}
impl std::error::Error for InvalidProgram {}

#[derive(Clone, Copy, Debug)]
enum Operation {
    Sub,
    Xor,
    AddShift(u32),
    Mul,
    Rotate(u32),
    AddConstant(u64),
    XorConstant(u64),
    MulHigh,
    SignedMulHigh,
    MulReciprocal(u64),
}

#[derive(Clone, Copy, Debug)]
struct Instruction {
    dst: usize,
    src: usize,
    operation: Operation,
}

#[derive(Clone, Debug)]
pub struct Program {
    instructions: Box<[Instruction]>,
    address_register: usize,
}

impl Program {
    /// Decode the upstream 8-byte instruction encoding. If reciprocals is
    /// present, IMUL_RCP immediates index that table; otherwise they are divisors.
    /// Invalid input is rejected before any register state can be modified.
    pub fn decode(
        bytes: &[u8],
        address_register: u8,
        reciprocals: Option<&[u64]>,
    ) -> Result<Self, InvalidProgram> {
        if bytes.is_empty()
            || !bytes.len().is_multiple_of(8)
            || bytes.len() / 8 > 512
            || address_register >= 8
        {
            return Err(InvalidProgram);
        }
        let mut instructions = Vec::with_capacity(bytes.len() / 8);
        for bytes in bytes.as_chunks::<8>().0 {
            let dst = usize::from(bytes[1]);
            let src = usize::from(bytes[2]);
            if dst >= 8 || src >= 8 {
                return Err(InvalidProgram);
            }
            let immediate = u32::from_le_bytes(bytes[4..].try_into().unwrap());
            let signed = immediate as i32 as i64 as u64;
            let operation = match bytes[0] {
                0 => Operation::Sub,
                1 => Operation::Xor,
                2 => Operation::AddShift(u32::from((bytes[3] >> 2) & 3)),
                3 => Operation::Mul,
                4 if immediate < 64 => Operation::Rotate(immediate),
                5 | 7 | 9 => Operation::AddConstant(signed),
                6 | 8 | 10 => Operation::XorConstant(signed),
                11 => Operation::MulHigh,
                12 => Operation::SignedMulHigh,
                13 => Operation::MulReciprocal(match reciprocals {
                    Some(table) => *table.get(immediate as usize).ok_or(InvalidProgram)?,
                    None => reciprocal(immediate).ok_or(InvalidProgram)?,
                }),
                _ => return Err(InvalidProgram),
            };
            instructions.push(Instruction {
                dst,
                src,
                operation,
            });
        }
        Ok(Self {
            instructions: instructions.into_boxed_slice(),
            address_register: usize::from(address_register),
        })
    }

    pub fn address_register(&self) -> usize {
        self.address_register
    }

    pub fn execute(&self, registers: &mut [u64; 8]) {
        for instruction in &self.instructions {
            let Instruction {
                dst,
                src,
                operation,
            } = *instruction;
            let a = registers[dst];
            let b = registers[src];
            registers[dst] = match operation {
                Operation::Sub => a.wrapping_sub(b),
                Operation::Xor => a ^ b,
                Operation::AddShift(shift) => a.wrapping_add(b << shift),
                Operation::Mul => a.wrapping_mul(b),
                Operation::Rotate(shift) => a.rotate_right(shift),
                Operation::AddConstant(value) => a.wrapping_add(value),
                Operation::XorConstant(value) => a ^ value,
                Operation::MulHigh => ((u128::from(a) * u128::from(b)) >> 64) as u64,
                Operation::SignedMulHigh => {
                    ((i128::from(a as i64) * i128::from(b as i64)) >> 64) as u64
                }
                Operation::MulReciprocal(value) => a.wrapping_mul(value),
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reciprocal_vectors_and_invalid_divisors() {
        for (divisor, expected) in [
            (3, 12297829382473034410),
            (13, 11351842506898185609),
            (33, 17887751829051686415),
            (65537, 18446462603027742720),
            (15000001, 10316166306300415204),
            (3845182035, 10302264209224146340),
            (u32::MAX, 9223372039002259456),
        ] {
            assert_eq!(reciprocal(divisor), Some(expected));
        }
        assert_eq!(reciprocal(0), None);
        for bit in 0..32 {
            assert_eq!(reciprocal(1 << bit), None);
        }
    }

    #[test]
    fn malformed_programs_are_rejected() {
        for bytes in [
            &[][..],
            &[0; 7][..],
            &[0; 4097][..],
            &[14, 0, 0, 0, 0, 0, 0, 0][..],
            &[0, 8, 0, 0, 0, 0, 0, 0][..],
            &[0, 0, 8, 0, 0, 0, 0, 0][..],
            &[4, 0, 0, 0, 64, 0, 0, 0][..],
            &[13, 0, 0, 0, 0, 0, 0, 0][..],
            &[13, 0, 0, 0, 2, 0, 0, 0][..],
        ] {
            assert!(Program::decode(bytes, 0, None).is_err());
        }
        assert!(Program::decode(&[0; 8], 8, None).is_err());
        assert!(Program::decode(&[13, 0, 0, 0, 1, 0, 0, 0], 0, Some(&[7])).is_err());
    }
}
