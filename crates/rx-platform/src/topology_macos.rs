use std::{ffi::CStr, mem, ptr};

pub(super) fn physical_cpus() -> Option<usize> {
    let mut value: libc::c_int = 0;
    let mut len = mem::size_of_val(&value);
    let name: &CStr = c"hw.physicalcpu";
    // SAFETY: NUL-terminated name and correctly sized writable integer; no new value.
    let result = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            (&mut value as *mut libc::c_int).cast(),
            &mut len,
            ptr::null_mut(),
            0,
        )
    };
    (result == 0 && len == mem::size_of_val(&value) && value > 0).then_some(value as usize)
}

unsafe extern "C" {
    // Stable libSystem Mach interfaces. libc is removing its convenience
    // bindings; keep the small set of port-lifetime APIs declared together.
    fn mach_host_self() -> libc::mach_port_t;
    static mach_task_self_: libc::mach_port_t;
    fn mach_port_deallocate(
        task: libc::mach_port_t,
        name: libc::mach_port_t,
    ) -> libc::kern_return_t;
}

pub(super) fn available_memory() -> Option<u64> {
    // SAFETY: sysconf has no pointer arguments; a nonpositive result is rejected.
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if page_size <= 0 {
        return None;
    }
    // SAFETY: all fields are integers, for which zero is valid. Older kernels
    // may fill only a prefix of this SDK structure; the rest stays initialized.
    let mut stats: libc::vm_statistics64 = unsafe { mem::zeroed() };
    let mut count = libc::HOST_VM_INFO64_COUNT;
    // SAFETY: valid output and integer-count capacity. The host send right is
    // released on both success and failure, so periodic health checks do not leak it.
    let result = unsafe {
        let host = mach_host_self();
        let result = libc::host_statistics64(
            host,
            libc::HOST_VM_INFO64,
            (&mut stats as *mut libc::vm_statistics64).cast(),
            &mut count,
        );
        mach_port_deallocate(mach_task_self_, host);
        result
    };
    if result != libc::KERN_SUCCESS || count < 4 {
        return None;
    }
    // Approximate reclaimable headroom. Speculative pages are already included
    // in free_count; purgeable pages overlap other categories. Do not add twice
    // or count compressed/swapped memory as available physical RAM.
    (u64::from(stats.free_count) + u64::from(stats.inactive_count)).checked_mul(page_size as u64)
}

#[cfg(test)]
mod tests {
    #[test]
    fn native_cpu_and_memory_queries() {
        let physical = super::physical_cpus().expect("physical CPU count");
        assert!(physical > 0);
        // Zero is a valid answer under memory pressure; absence is not.
        assert!(super::available_memory().is_some());
        let all = crate::topology::discover().unwrap();
        assert!(all.iter().all(|core| !core.pinned));
        assert_eq!(
            all.iter().filter(|core| !core.smt).count(),
            physical.min(all.len())
        );
    }
}
