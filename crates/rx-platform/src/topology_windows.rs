use super::Core;
use anyhow::{bail, ensure, Context, Result};
use std::{collections::BTreeMap, io, mem, ptr};
use windows_sys::Win32::{
    Foundation::ERROR_INSUFFICIENT_BUFFER,
    System::SystemInformation::{
        GetLogicalProcessorInformationEx, RelationProcessorCore, GROUP_AFFINITY,
        PROCESSOR_RELATIONSHIP, SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX,
    },
};

const PAYLOAD: usize = mem::offset_of!(SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX, Anonymous);
const GROUP_COUNT: usize = PAYLOAD + mem::offset_of!(PROCESSOR_RELATIONSHIP, GroupCount);
const MASKS: usize = PAYLOAD + mem::offset_of!(PROCESSOR_RELATIONSHIP, GroupMask);

fn physical_map(data: &[u8]) -> Result<BTreeMap<usize, usize>> {
    let mut map = BTreeMap::new();
    let mut offset = 0;
    while offset < data.len() {
        let header = data
            .get(offset..offset + 8)
            .context("truncated CPU record")?;
        let relation = i32::from_ne_bytes(header[..4].try_into().unwrap());
        let size = u32::from_ne_bytes(header[4..].try_into().unwrap()) as usize;
        ensure!(
            size >= 8 && size <= data.len() - offset,
            "invalid CPU record length"
        );
        let record = &data[offset..offset + size];
        if relation == RelationProcessorCore {
            ensure!(size >= MASKS, "truncated processor relationship");
            let count = u16::from_ne_bytes(record[GROUP_COUNT..GROUP_COUNT + 2].try_into().unwrap())
                as usize;
            ensure!(
                count > 0 && count <= (size - MASKS) / mem::size_of::<GROUP_AFFINITY>(),
                "invalid CPU group count"
            );
            for index in 0..count {
                let start = MASKS + index * mem::size_of::<GROUP_AFFINITY>();
                let mask = usize::from_ne_bytes(
                    record[start..start + mem::size_of::<usize>()]
                        .try_into()
                        .unwrap(),
                );
                let group_offset = start + mem::offset_of!(GROUP_AFFINITY, Group);
                let group = usize::from(u16::from_ne_bytes(
                    record[group_offset..group_offset + 2].try_into().unwrap(),
                ));
                for bit in 0..usize::BITS as usize {
                    if mask & (1usize << bit) != 0 {
                        ensure!(
                            map.insert(group * 64 + bit, offset).is_none(),
                            "duplicate logical CPU"
                        );
                    }
                }
            }
        }
        offset += size;
    }
    Ok(map)
}

fn query() -> Result<BTreeMap<usize, usize>> {
    let mut bytes = 0;
    // SAFETY: null buffer requests the required size; bytes is a valid output.
    unsafe { GetLogicalProcessorInformationEx(RelationProcessorCore, ptr::null_mut(), &mut bytes) };
    for _ in 0..3 {
        ensure!(
            bytes > 0 && bytes <= 16 * 1024 * 1024,
            "invalid topology buffer size"
        );
        // usize storage provides sufficient alignment for every Windows record.
        let mut buffer = vec![0usize; (bytes as usize).div_ceil(mem::size_of::<usize>())];
        let capacity = mem::size_of_val(buffer.as_slice());
        // SAFETY: aligned writable buffer covers bytes; the API returns actual length.
        if unsafe {
            GetLogicalProcessorInformationEx(
                RelationProcessorCore,
                buffer.as_mut_ptr().cast(),
                &mut bytes,
            )
        } != 0
        {
            ensure!(
                bytes as usize <= capacity,
                "topology output exceeded buffer"
            );
            // SAFETY: API initialized this byte prefix and it fits in the allocation.
            return physical_map(unsafe {
                std::slice::from_raw_parts(buffer.as_ptr().cast(), bytes as usize)
            });
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER as i32) {
            return Err(error.into());
        }
    }
    bail!("CPU topology changed repeatedly during discovery")
}

pub(super) fn discover(allowed: &[usize]) -> Result<Vec<Core>> {
    let physical = query()?;
    select_physical(&physical, allowed)
}

fn select_physical(physical: &BTreeMap<usize, usize>, allowed: &[usize]) -> Result<Vec<Core>> {
    let mut seen = std::collections::BTreeSet::new();
    allowed
        .iter()
        .map(|&cpu| {
            let id = physical
                .get(&cpu)
                .context("allowed CPU missing from topology")?;
            Ok(Core {
                cpu,
                pinned: true,
                node: 0,
                smt: !seen.insert(*id),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_topology_respects_process_affinity() {
        let allowed = crate::topology::affinity().unwrap();
        let cores = discover(&allowed).unwrap();
        assert_eq!(
            cores.iter().map(|core| core.cpu).collect::<Vec<_>>(),
            allowed
        );
        assert!(cores.iter().all(|core| core.pinned));
        assert!(cores.iter().any(|core| !core.smt));
        // Do not change the test harness's affinity.
        std::thread::spawn(move || crate::topology::pin(cores[0].cpu).unwrap())
            .join()
            .unwrap();
    }

    #[test]
    fn malformed_topology_is_rejected() {
        assert!(physical_map(&[0; 7]).is_err());
        assert!(physical_map(&[0; 8]).is_err());
        let mut record = vec![0; MASKS];
        let size = record.len() as u32;
        record[4..8].copy_from_slice(&size.to_ne_bytes());
        record[GROUP_COUNT..GROUP_COUNT + 2].copy_from_slice(&1u16.to_ne_bytes());
        assert!(physical_map(&record).is_err());
    }

    #[test]
    fn sparse_masks_and_restricted_siblings() {
        let record = |group: u16, mask: usize| {
            let mut bytes = vec![0u8; MASKS + mem::size_of::<GROUP_AFFINITY>()];
            let size = bytes.len() as u32;
            bytes[4..8].copy_from_slice(&size.to_ne_bytes());
            bytes[GROUP_COUNT..GROUP_COUNT + 2].copy_from_slice(&1u16.to_ne_bytes());
            bytes[MASKS..MASKS + mem::size_of::<usize>()].copy_from_slice(&mask.to_ne_bytes());
            let group_offset = MASKS + mem::offset_of!(GROUP_AFFINITY, Group);
            bytes[group_offset..group_offset + 2].copy_from_slice(&group.to_ne_bytes());
            bytes
        };
        let mut data = record(0, 0b1010);
        data.extend(record(2, 0b1001));
        let map = physical_map(&data).unwrap();
        let cores = select_physical(&map, &[1, 3, 128, 131]).unwrap();
        assert_eq!(
            cores.iter().map(|c| c.smt).collect::<Vec<_>>(),
            [false, true, false, true]
        );
        // The first allowed sibling is a physical worker even if the lower CPU
        // ID is excluded by process affinity. Group-local indices never alias.
        let restricted = select_physical(&map, &[3, 131]).unwrap();
        assert!(restricted.iter().all(|core| !core.smt));
        assert!(select_physical(&map, &[99]).is_err());
        data.extend(record(0, 0b0010));
        assert!(physical_map(&data).is_err());
    }
}
