// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2018-2019, tevador <tevador@gmail.com>
// Copyright (c) 2026 RxMiner contributors
// Port of RandomX's superscalar generator; see LICENSE here.

//! Deterministic Ivy Bridge scheduling model used to construct dataset programs.
//! The modeled CPU is part of the algorithm, independent of the host CPU.

use super::{generator::Blake2Generator, superscalar::Program};

const LATENCY: usize = 170;
const MAX_SIZE: usize = 3 * LATENCY + 2;
const LOOK_FORWARD: usize = 4;
const MAX_THROWAWAY: usize = 256;
const BUFFERS: [&[usize]; 6] = [
    &[4, 8, 4],
    &[7, 3, 3, 3],
    &[3, 7, 3, 3],
    &[4, 9, 3],
    &[4, 4, 4, 4],
    &[3, 3, 10],
];

/// Scheduling diagnostics, also compared with the native generator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Schedule {
    pub code_size: usize,
    pub macro_ops: usize,
    pub decode_cycles: usize,
    pub cpu_latency: usize,
    pub asic_latency: usize,
    pub mul_count: usize,
    pub cpu_latencies: [usize; 8],
    pub asic_latencies: [usize; 8],
}

#[derive(Clone, Debug)]
pub struct GeneratedProgram {
    instructions: Vec<[u8; 8]>,
    address_register: u8,
    schedule: Schedule,
}

impl GeneratedProgram {
    /// Canonical RandomX instructions, with little-endian immediates.
    pub fn encoded(&self) -> &[u8] {
        self.instructions.as_flattened()
    }

    pub fn address_register(&self) -> u8 {
        self.address_register
    }

    pub fn schedule(&self) -> &Schedule {
        &self.schedule
    }

    /// Prepare constant operands and reciprocal multipliers for execution.
    pub fn prepare(&self) -> Program {
        Program::decode(self.encoded(), self.address_register, None)
            .expect("the scheduler emits valid superscalar instructions")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum Kind {
    Sub,
    Xor,
    AddShift,
    Mul,
    Rotate,
    Add7,
    Xor7,
    Add8,
    Xor8,
    Add9,
    Xor9,
    MulHigh,
    SignedMulHigh,
    Reciprocal,
}

#[derive(Clone, Copy)]
struct MacroOp {
    size: usize,
    latency: usize,
    ports: [u8; 2],
    dependent: bool,
}

const fn op(size: usize, latency: usize, ports: [u8; 2], dependent: bool) -> MacroOp {
    MacroOp {
        size,
        latency,
        ports,
        dependent,
    }
}

impl Kind {
    fn ops(self) -> &'static [MacroOp] {
        match self {
            Self::Sub | Self::Xor => const { &[op(3, 1, [7, 0], false)] },
            Self::AddShift => const { &[op(4, 1, [3, 0], false)] },
            Self::Mul => const { &[op(4, 3, [2, 0], false)] },
            Self::Rotate => const { &[op(4, 1, [5, 0], false)] },
            Self::Add7 | Self::Xor7 | Self::Add8 | Self::Xor8 | Self::Add9 | Self::Xor9 => {
                const { &[op(7, 1, [7, 0], false)] }
            }
            Self::MulHigh | Self::SignedMulHigh => {
                const {
                    &[
                        op(3, 0, [0, 0], false),
                        op(3, 4, [2, 4], false),
                        op(3, 0, [0, 0], false),
                    ]
                }
            }
            Self::Reciprocal => const { &[op(10, 1, [7, 0], false), op(4, 3, [2, 0], true)] },
        }
    }

    fn source_op(self) -> Option<usize> {
        match self {
            Self::Sub | Self::Xor | Self::AddShift | Self::Mul => Some(0),
            Self::MulHigh | Self::SignedMulHigh => Some(1),
            _ => None,
        }
    }

    fn destination_op(self) -> usize {
        usize::from(self == Self::Reciprocal)
    }

    fn result_op(self) -> usize {
        usize::from(matches!(
            self,
            Self::MulHigh | Self::SignedMulHigh | Self::Reciprocal
        ))
    }

    fn multiplication(self) -> bool {
        matches!(
            self,
            Self::Mul | Self::MulHigh | Self::SignedMulHigh | Self::Reciprocal
        )
    }
}

#[derive(Clone, Copy)]
struct Register {
    latency: usize,
    group: Option<Kind>,
    parameter: i32,
}

struct Pending {
    kind: Kind,
    src: Option<usize>,
    dst: Option<usize>,
    modifier: u8,
    immediate: u32,
    group: Kind,
    parameter: i32,
    reuse: bool,
    parameter_is_source: bool,
}

impl Pending {
    fn create(gen: &mut Blake2Generator, slot: usize, buffer: usize, last: bool) -> Self {
        use Kind::*;
        let kind = match slot {
            3 if last => [Sub, Xor, MulHigh, SignedMulHigh][usize::from(gen.next_byte() & 3)],
            3 => [Sub, Xor][usize::from(gen.next_byte() & 1)],
            4 if buffer == 4 && !last => Mul,
            4 => [Rotate, AddShift][usize::from(gen.next_byte() & 1)],
            7 => [Xor7, Add7][usize::from(gen.next_byte() & 1)],
            8 => [Xor8, Add8][usize::from(gen.next_byte() & 1)],
            9 => [Xor9, Add9][usize::from(gen.next_byte() & 1)],
            10 => Reciprocal,
            _ => unreachable!("decoder slot size"),
        };
        let mut result = Self {
            kind,
            src: None,
            dst: None,
            modifier: 0,
            immediate: 0,
            group: kind,
            parameter: -1,
            reuse: false,
            parameter_is_source: false,
        };
        match kind {
            Sub | Xor | AddShift | Mul => {
                result.parameter_is_source = true;
                if kind == Sub {
                    result.group = AddShift;
                }
                if kind == AddShift {
                    result.modifier = gen.next_byte();
                }
            }
            Rotate => {
                while result.immediate == 0 {
                    result.immediate = u32::from(gen.next_byte() & 63);
                }
            }
            Add7 | Add8 | Add9 => {
                result.immediate = gen.next_u32();
                result.group = Add7;
            }
            Xor7 | Xor8 | Xor9 => {
                result.immediate = gen.next_u32();
                result.group = Xor7;
            }
            MulHigh | SignedMulHigh => {
                result.reuse = true;
                result.parameter = gen.next_u32() as i32;
            }
            Reciprocal => {
                while result.immediate == 0 || result.immediate.is_power_of_two() {
                    result.immediate = gen.next_u32();
                }
            }
        }
        result
    }

    fn select_source(
        &mut self,
        cycle: usize,
        registers: &[Register; 8],
        gen: &mut Blake2Generator,
    ) -> bool {
        let (available, count) = candidates(|i| registers[i].latency <= cycle);
        // r5 cannot be the destination of LEA, so reserve the other register.
        let selected =
            if count == 2 && self.kind == Kind::AddShift && available[..count].contains(&5) {
                Some(5)
            } else {
                select(&available[..count], gen)
            };
        if let Some(src) = selected {
            self.src = Some(src);
            if self.parameter_is_source {
                self.parameter = src as i32;
            }
            true
        } else {
            false
        }
    }

    fn select_destination(
        &mut self,
        cycle: usize,
        chained_mul: bool,
        registers: &[Register; 8],
        gen: &mut Blake2Generator,
    ) -> bool {
        let (available, count) = candidates(|i| {
            let r = registers[i];
            r.latency <= cycle
                && (self.reuse || Some(i) != self.src)
                && (chained_mul || self.group != Kind::Mul || r.group != Some(Kind::Mul))
                && (r.group != Some(self.group) || r.parameter != self.parameter)
                && (self.kind != Kind::AddShift || i != 5)
        });
        if let Some(dst) = select(&available[..count], gen) {
            self.dst = Some(dst);
            true
        } else {
            false
        }
    }

    fn encode(&self) -> [u8; 8] {
        let dst = self.dst.expect("destination selected before retirement");
        let mut bytes = [
            self.kind as u8,
            dst as u8,
            self.src.unwrap_or(dst) as u8,
            self.modifier,
            0,
            0,
            0,
            0,
        ];
        bytes[4..].copy_from_slice(&self.immediate.to_le_bytes());
        bytes
    }
}

// Register selection uses a fixed stack buffer, preserving the native order and
// consuming randomness only when there is more than one candidate.
fn candidates(mut accepts: impl FnMut(usize) -> bool) -> ([usize; 8], usize) {
    let mut available = [0; 8];
    let mut count = 0;
    for i in 0..8 {
        if accepts(i) {
            available[count] = i;
            count += 1;
        }
    }
    (available, count)
}

fn select(available: &[usize], gen: &mut Blake2Generator) -> Option<usize> {
    match available.len() {
        0 => None,
        1 => Some(available[0]),
        len => Some(available[gen.next_u32() as usize % len]),
    }
}

type Ports = [[bool; 3]; LATENCY + 4];

fn schedule_uop(mask: u8, busy: &mut Ports, start: usize, commit: bool) -> Option<usize> {
    for (cycle, ports) in busy.iter_mut().enumerate().skip(start) {
        // P5 -> P0 -> P1 keeps the multiplication port free where possible.
        for (port, bit) in [(2, 4), (0, 1), (1, 2)] {
            if mask & bit != 0 && !ports[port] {
                if commit {
                    ports[port] = true;
                }
                return Some(cycle);
            }
        }
    }
    None
}

fn schedule_op(
    op: MacroOp,
    busy: &mut Ports,
    mut cycle: usize,
    dependency: usize,
    commit: bool,
) -> Option<usize> {
    if op.dependent {
        cycle = cycle.max(dependency);
    }
    if op.ports[0] == 0 {
        return Some(cycle);
    }
    if op.ports[1] == 0 {
        return schedule_uop(op.ports[0], busy, cycle, commit);
    }
    // Both micro-ops of a high multiply must issue in the same cycle.
    for start in cycle..busy.len() {
        let first = schedule_uop(op.ports[0], busy, start, false);
        let second = schedule_uop(op.ports[1], busy, start, false);
        if let Some(found) = first.filter(|_| first == second) {
            if commit {
                schedule_uop(op.ports[0], busy, found, true);
                schedule_uop(op.ports[1], busy, found, true);
            }
            return Some(found);
        }
    }
    None
}

/// Generate one program, advancing the byte generator exactly as native RandomX
/// does. Reuse the generator for the eight consecutive programs in a cache.
pub fn generate(gen: &mut Blake2Generator) -> GeneratedProgram {
    let mut busy = [[false; 3]; LATENCY + 4];
    let mut registers = [Register {
        latency: 0,
        group: None,
        parameter: -1,
    }; 8];
    let mut current: Option<Pending> = None;
    let mut op_index = 0;
    let mut cycle = 0;
    let mut dependency = 0;
    let mut saturated = false;
    let mut throwaway = 0;
    let mut instructions = Vec::with_capacity(MAX_SIZE);
    let mut stats = Schedule {
        code_size: 0,
        macro_ops: 0,
        decode_cycles: 0,
        cpu_latency: 0,
        asic_latency: 0,
        mul_count: 0,
        cpu_latencies: [0; 8],
        asic_latencies: [0; 8],
    };
    while stats.decode_cycles < LATENCY && !saturated && instructions.len() < MAX_SIZE {
        let kind = current.as_ref().map(|instruction| instruction.kind);
        let buffer = if matches!(kind, Some(Kind::MulHigh | Kind::SignedMulHigh)) {
            5
        } else if stats.mul_count < stats.decode_cycles + 1 {
            4
        } else if kind == Some(Kind::Reciprocal) {
            if gen.next_byte() & 1 != 0 {
                0
            } else {
                3
            }
        } else {
            usize::from(gen.next_byte() & 3)
        };
        let slots = BUFFERS[buffer];
        let mut slot = 0;
        while slot < slots.len() {
            let top_cycle = cycle;
            if current
                .as_ref()
                .is_none_or(|i| op_index >= i.kind.ops().len())
            {
                if saturated || instructions.len() >= MAX_SIZE {
                    break;
                }
                current = Some(Pending::create(
                    gen,
                    slots[slot],
                    buffer,
                    slot + 1 == slots.len(),
                ));
                op_index = 0;
            }
            let instruction = current.as_mut().unwrap();
            let op = instruction.kind.ops()[op_index];
            let Some(mut scheduled) = schedule_op(op, &mut busy, cycle, dependency, false) else {
                saturated = true;
                break;
            };

            // Failed selection advances the cycle even if this instruction is
            // discarded. Restoring top_cycle here would change the byte stream.
            let mut failed = false;
            if Some(op_index) == instruction.kind.source_op() {
                let mut forward = 0;
                while forward < LOOK_FORWARD
                    && !instruction.select_source(scheduled, &registers, gen)
                {
                    scheduled += 1;
                    cycle += 1;
                    forward += 1;
                }
                failed = forward == LOOK_FORWARD;
            }
            if !failed && op_index == instruction.kind.destination_op() {
                let mut forward = 0;
                while forward < LOOK_FORWARD
                    && !instruction.select_destination(scheduled, throwaway > 0, &registers, gen)
                {
                    scheduled += 1;
                    cycle += 1;
                    forward += 1;
                }
                failed = forward == LOOK_FORWARD;
            }
            if failed {
                if throwaway < MAX_THROWAWAY {
                    throwaway += 1;
                    op_index = instruction.kind.ops().len();
                    continue;
                }
                current = None;
                break;
            }
            throwaway = 0;
            let Some(scheduled) = schedule_op(op, &mut busy, scheduled, scheduled, true) else {
                saturated = true;
                break;
            };
            dependency = scheduled + op.latency;
            if op_index == instruction.kind.result_op() {
                stats.cpu_latency = dependency;
                registers[instruction.dst.unwrap()] = Register {
                    latency: dependency,
                    group: Some(instruction.group),
                    parameter: instruction.parameter,
                };
            }
            stats.code_size += op.size;
            slot += 1;
            op_index += 1;
            stats.macro_ops += 1;
            saturated |= scheduled >= LATENCY;
            cycle = top_cycle;
            if op_index >= instruction.kind.ops().len() {
                instructions.push(instruction.encode());
                stats.mul_count += usize::from(instruction.kind.multiplication());
            }
        }
        cycle += 1;
        stats.decode_cycles += 1;
    }
    for bytes in &instructions {
        let dst = usize::from(bytes[1]);
        let src = usize::from(bytes[2]);
        let source = if src != dst {
            stats.asic_latencies[src] + 1
        } else {
            0
        };
        stats.asic_latencies[dst] = (stats.asic_latencies[dst] + 1).max(source);
    }
    let mut address_register = 0;
    for (i, register) in registers.iter().enumerate() {
        // Strict comparison preserves the first register when latencies tie.
        if stats.asic_latencies[i] > stats.asic_latency {
            stats.asic_latency = stats.asic_latencies[i];
            address_register = i as u8;
        }
        stats.cpu_latencies[i] = register.latency;
    }
    GeneratedProgram {
        instructions,
        address_register,
        schedule: stats,
    }
}

#[cfg(test)]
mod tests {
    use super::super::blake2b;
    use super::*;

    #[test]
    fn upstream_program_hashes() {
        // The ten consecutive vectors shipped with RandomX, not generated by
        // this implementation. This test also runs without a C/C++ toolchain.
        let expected = [
            "d3a4a6623738756f77e6104469102f082eff2a3e60be7ad696285ef7dfc72a61",
            "f5e7e0bbc7e93c609003d6359208688070afb4a77165a552ff7be63b38dfbc86",
            "85ed8b11734de5b3e9836641413a8f36e99e89694f419c8cd25c3f3f16c40c5a",
            "5dd956292cf5d5704ad99e362d70098b2777b2a1730520be52f772ca48cd3bc0",
            "6f14018ca7d519e9b48d91af094c0f2d7e12e93af0228782671a8640092af9e5",
            "134be097c92e2c45a92f23208cacd89e4ce51f1009a0b900dbe83b38de11d791",
            "268f9392c20c6e31371a5131f82bd7713d3910075f2f0468baafaa1abd2f3187",
            "c668a05fd909714ed4a91e8d96d67b17e44329e88bc71e0672b529a3fc16be47",
            "99739351315840963011e4c5d8e90ad0bfed3facdcb713fe8f7138fbf01c4c94",
            "14ab53d61880471f66e80183968d97effd5492b406876060e595fcf9682f9295",
        ];
        let mut generator = Blake2Generator::new(b"test key 000", 0);
        for reference in expected {
            let program = generate(&mut generator);
            let mut digest = [0; 32];
            blake2b::hash(program.encoded(), &mut digest).unwrap();
            let actual: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
            assert_eq!(actual, reference);
        }
    }
}
