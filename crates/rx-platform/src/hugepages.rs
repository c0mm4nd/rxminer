//! Best-effort promotion of long-lived, writable buffers after NUMA placement.
use std::sync::atomic::AtomicBool;

/// Try to promote complete PMD-sized regions inside an initialized buffer on Linux.
/// Call on the intended NUMA node after preparing higher-priority working buffers.
/// Other platforms, disabled THP, and `RXM_COLLAPSE=0` are no-ops. Failure leaves
/// the buffer usable; older kernels need no special handling by the caller.
///
/// Work uses 32 MiB requests (or one native huge page if larger), checking
/// cancellation between requests.
/// Two failed requests end the attempt, avoiding repeated reclaim under pressure.
/// A single kernel request can still block during memory compaction.
pub fn promote_huge_pages<T>(memory: &[T], enabled: bool, stop: Option<&AtomicBool>) {
    #[cfg(target_os = "linux")]
    {
        use std::sync::atomic::Ordering;
        if !enabled
            || std::env::var_os("RXM_COLLAPSE")
                .is_some_and(|v| v.as_encoded_bytes().starts_with(b"0"))
        {
            return;
        }
        let huge = huge_page_size();
        let chunk = (32 << 20).max(huge);
        let start = memory.as_ptr() as usize;
        let Some(end) = start.checked_add(std::mem::size_of_val(memory)) else {
            return;
        };
        let end = end & !(huge - 1);
        let Some(aligned) = start.checked_add(huge - 1) else {
            return;
        };
        let mut address = aligned & !(huge - 1);
        let mut failures = 0;
        while address < end && !stop.is_some_and(|s| s.load(Ordering::Relaxed)) {
            let len = (end - address).min(chunk);
            // SAFETY: only complete huge pages inside the borrowed, initialized
            // buffer; both operations preserve its contents. Populate
            // also makes fresh, zero-backed scratchpads eligible for collapse.
            let result = unsafe {
                let p = address as *mut libc::c_void;
                if libc::madvise(p, len, libc::MADV_POPULATE_WRITE) != 0 {
                    -1
                } else {
                    libc::madvise(p, len, 25) // Linux UAPI MADV_COLLAPSE (6.1+).
                }
            };
            if result != 0 {
                let error = std::io::Error::last_os_error();
                failures += 1;
                if std::env::var_os("RXM_DEBUG").is_some() {
                    eprintln!("THP promotion: {len} bytes: {error}");
                }
                // EINVAL includes old kernels and process/VMA THP restrictions.
                if failures >= 2
                    || matches!(error.raw_os_error(), Some(libc::EINVAL | libc::ENOSYS))
                {
                    break;
                }
            }
            address += len;
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (memory, enabled, stop);
}

#[cfg(target_os = "linux")]
fn huge_page_size() -> usize {
    static SIZE: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *SIZE.get_or_init(|| {
        std::fs::read_to_string("/sys/kernel/mm/transparent_hugepage/hpage_pmd_size")
            .ok()
            .and_then(|s| s.trim().parse::<usize>().ok())
            .filter(|n| n.is_power_of_two() && (4096..=1 << 30).contains(n))
            .unwrap_or(2 << 20)
    })
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn promotion_preserves_contents() {
        const HUGE: usize = 2 << 20;
        // Extra space accommodates alignment and two untouched boundary pages.
        let mut bytes = vec![0xa5u8; 4 * HUGE];
        let offset = (HUGE - bytes.as_ptr() as usize % HUGE) % HUGE;
        let slice = &mut bytes[offset..offset + 3 * HUGE];
        // A subrange must not promote its partially covered first/last pages.
        // SAFETY: the exclusively owned aligned range spans initialized bytes.
        let policy = unsafe {
            libc::madvise(
                slice.as_mut_ptr().cast(),
                slice.len(),
                libc::MADV_NOHUGEPAGE,
            )
        };
        if policy != 0 {
            // THP may be absent from the kernel; promotion remains optional.
            promote_huge_pages(slice, true, None);
            assert!(bytes.iter().all(|v| *v == 0xa5));
            return;
        }
        let stop = AtomicBool::new(true);
        promote_huge_pages(&slice[1..3 * HUGE - 1], true, Some(&stop));
        stop.store(false, Ordering::Relaxed);
        promote_huge_pages(slice, false, Some(&stop));
        // Only the inner huge page is eligible. A denied collapse is valid too:
        // test data integrity on restricted kernels as well as successful ones.
        // SAFETY: the inner page is inside the same writable aligned allocation.
        assert_eq!(
            // SAFETY: range is inside the exclusively owned writable allocation.
            unsafe {
                libc::madvise(
                    slice.as_mut_ptr().add(HUGE).cast(),
                    HUGE,
                    libc::MADV_HUGEPAGE,
                )
            },
            0
        );
        promote_huge_pages(&slice[1..3 * HUGE - 1], true, Some(&stop));
        assert!(bytes.iter().all(|v| *v == 0xa5));
    }
    #[test]
    fn disabled_cancelled_and_partial_pages_are_not_populated() {
        const HUGE: usize = 2 << 20;
        const LEN: usize = 5 * HUGE;
        if huge_page_size() != HUGE {
            return;
        }
        // SAFETY: read-only system page-size query.
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as usize;
        // SAFETY: fresh anonymous writable mapping, released below.
        let raw = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                LEN,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        assert_ne!(raw, libc::MAP_FAILED);
        struct Region(*mut libc::c_void);
        impl Drop for Region {
            fn drop(&mut self) {
                // SAFETY: unique owner of the original mapping.
                unsafe {
                    libc::munmap(self.0, LEN);
                }
            }
        }
        let _region = Region(raw);
        // Probe populate support outside the tested range; old/restricted kernels
        // may reject the syscall that promotion intentionally treats as optional.
        // SAFETY: first page of the owned writable allocation.
        if unsafe { libc::madvise(raw, 4096, libc::MADV_POPULATE_WRITE) } != 0 {
            return;
        }
        let address = (raw as usize + 2 * HUGE - 1) & !(HUGE - 1);
        // SAFETY: aligned range remains within the allocation and is zero-filled.
        let bytes = unsafe { std::slice::from_raw_parts_mut(address as *mut u8, 3 * HUGE) };
        // Prevent natural THP faults from populating adjacent boundary pages.
        // SAFETY: complete writable range inside the owned mapping.
        if unsafe {
            libc::madvise(
                bytes.as_mut_ptr().cast(),
                bytes.len(),
                libc::MADV_NOHUGEPAGE,
            )
        } != 0
        {
            return;
        }
        let resident = || {
            let mut state = vec![0u8; 3 * HUGE / page_size];
            // SAFETY: mapped range and an output byte for every base page.
            assert_eq!(
                // SAFETY: output has one byte per page of the live mapping.
                unsafe { libc::mincore(address as *mut _, 3 * HUGE, state.as_mut_ptr()) },
                0
            );
            state
        };
        let stop = AtomicBool::new(true);
        promote_huge_pages(&bytes[1..3 * HUGE - 1], true, Some(&stop));
        stop.store(false, Ordering::Relaxed);
        promote_huge_pages(&bytes[1..3 * HUGE - 1], false, Some(&stop));
        assert!(resident().iter().all(|v| v & 1 == 0));
        if std::env::var_os("RXM_COLLAPSE").is_some_and(|v| v.as_encoded_bytes().starts_with(b"0"))
        {
            return;
        }
        promote_huge_pages(&bytes[1..3 * HUGE - 1], true, Some(&stop));
        for (i, page) in resident().into_iter().enumerate() {
            assert_eq!(
                page & 1 != 0,
                (HUGE / page_size..2 * HUGE / page_size).contains(&i),
                "page {i}"
            );
        }
    }
}
