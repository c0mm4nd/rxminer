// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 RxMiner contributors
//! Owned aligned data and separate RW/RX code mappings, with a W^X fallback.
//! Linux aliases and Windows mappings never combine write/execute permissions.
//! Apple Silicon uses MAP_JIT with per-thread hardware write protection.
//! Exclusive Rust borrows serialize writes/execution.
use crate::randomx_memory::{Mapping, PageKind};
use std::{io, ptr::NonNull};

pub(super) struct Executable {
    pointer: NonNull<u8>,
    writable: Option<NonNull<u8>>,
    len: usize,
    executable: bool,
    // Retains ownership even on platforms where Drop has no alias cleanup.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    single: Option<Mapping>,
    huge: bool,
}
impl Executable {
    #[cfg(all(test, target_arch = "x86_64"))]
    pub(super) fn mapped_len(&self) -> usize {
        self.len
    }
    pub fn new(len: usize) -> io::Result<Self> {
        Self::with_options(
            len,
            crate::randomx_memory::JIT_HUGE.load(std::sync::atomic::Ordering::Relaxed),
            false,
        )
    }
    pub fn huge(&self) -> bool {
        self.huge
    }
    pub fn with_options(len: usize, huge: bool, secure: bool) -> io::Result<Self> {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            let _ = (huge, secure);
            Self::apple_mapping(len)
        }
        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        Self::ordinary_options(len, huge, secure)
    }
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    fn ordinary_options(len: usize, huge: bool, secure: bool) -> io::Result<Self> {
        // RV64 uses a single mapping: code replacement must invalidate translated
        // instructions as well as hardware caches, including QEMU user mode.
        let secure = secure || cfg!(target_arch = "riscv64");
        #[cfg(not(target_os = "linux"))]
        let _ = secure;
        assert!(len > 0 && len.is_multiple_of(4096));
        if huge {
            #[cfg(target_os = "linux")]
            if !secure {
                if let Ok(v) = Self::dual_mapping(2 << 20, true) {
                    return Ok(v);
                }
            }
            if let Ok(v) = Self::single_mapping_kind(len, PageKind::Huge2M) {
                return Ok(v);
            }
        }
        #[cfg(target_os = "linux")]
        if !secure {
            if let Ok(v) = Self::dual_mapping(len, false) {
                return Ok(v);
            }
        }
        Self::single_mapping(len)
    }
    #[cfg(target_os = "linux")]
    fn dual_mapping(len: usize, huge: bool) -> io::Result<Self> {
        use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
        // SAFETY: fixed nul-terminated name and supported Linux memfd flags.
        let fd = unsafe {
            libc::memfd_create(
                c"rxminer-jit".as_ptr(),
                libc::MFD_CLOEXEC
                    | libc::MFD_ALLOW_SEALING
                    | if huge {
                        libc::MFD_HUGETLB | (21 << 26)
                    } else {
                        0
                    },
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: successful memfd_create returns a new exclusively owned fd.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        // SAFETY: fd owns a new anonymous file; len is a bounded JIT mapping size.
        if unsafe { libc::ftruncate(fd.as_raw_fd(), len as libc::off_t) } != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: prevent resizing the backing file while either mapping lives.
        if unsafe {
            libc::fcntl(
                fd.as_raw_fd(),
                libc::F_ADD_SEALS,
                libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: shared RW mapping of the newly sized anonymous file.
        let write = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd.as_raw_fd(),
                0,
            )
        };
        if write == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        if huge {
            // SAFETY: validate writable backing before publishing either alias; otherwise a reserved huge page can SIGBUS on first use.
            if unsafe { libc::madvise(write, len, libc::MADV_POPULATE_WRITE) } != 0 {
                let error = io::Error::last_os_error();
                // SAFETY: release our only successful mapping on failure.
                unsafe {
                    libc::munmap(write, len);
                }
                return Err(error);
            }
        }
        // SAFETY: a distinct RX alias of the same live backing file, never RWX.
        let execute = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_EXEC,
                libc::MAP_SHARED,
                fd.as_raw_fd(),
                0,
            )
        };
        if execute == libc::MAP_FAILED {
            let error = io::Error::last_os_error();
            // SAFETY: release the successful first mapping on partial failure.
            unsafe {
                libc::munmap(write, len);
            }
            return Err(error);
        }
        Ok(Self {
            pointer: NonNull::new(execute.cast()).unwrap(),
            writable: Some(NonNull::new(write.cast()).unwrap()),
            len,
            executable: false,
            single: None,
            huge,
        })
    }
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    fn single_mapping(len: usize) -> io::Result<Self> {
        Self::single_mapping_kind(len, PageKind::Ordinary)
    }
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    fn single_mapping_kind(len: usize, kind: PageKind) -> io::Result<Self> {
        let mapping = Mapping::exact(len, kind)?;
        Ok(Self {
            pointer: mapping.pointer,
            writable: None,
            len: mapping.len,
            executable: false,
            huge: kind == PageKind::Huge2M,
            single: Some(mapping),
        })
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn apple_mapping(len: usize) -> io::Result<Self> {
        // SAFETY: MAP_JIT is protected per-thread by Apple's JIT write-protect API.
        let raw = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_JIT,
                -1,
                0,
            )
        };
        if raw == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        let mapping = Mapping {
            pointer: NonNull::new(raw.cast()).unwrap(),
            len,
            kind: PageKind::Ordinary,
        };
        Ok(Self {
            pointer: mapping.pointer,
            writable: None,
            len,
            executable: false,
            single: Some(mapping),
            huge: false,
        })
    }
    pub fn publish(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.publish_ranges(bytes, std::slice::from_ref(&(0..bytes.len())))
    }
    /// Publish only dirty ranges, with a single W^X transition and cache flush.
    pub fn publish_ranges(
        &mut self,
        bytes: &[u8],
        ranges: &[std::ops::Range<usize>],
    ) -> io::Result<()> {
        assert!(bytes.len() <= self.len);
        for range in ranges {
            assert!(range.start <= range.end && range.end <= bytes.len());
        }
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        self.protect(false)?;
        if self.executable && self.writable.is_none() {
            self.protect(false)?;
            self.executable = false;
        }
        for range in ranges {
            // SAFETY: checked source/destination bounds; the writable mapping
            // cannot alias the emitter's independently owned input buffer.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    bytes.as_ptr().add(range.start),
                    self.writable
                        .unwrap_or(self.pointer)
                        .as_ptr()
                        .add(range.start),
                    range.len(),
                );
            }
        }
        if self.writable.is_none() {
            self.protect(true)?;
        }
        #[cfg(target_arch = "aarch64")]
        self.flush_instruction_cache(bytes.len());
        #[cfg(all(target_os = "linux", target_arch = "riscv64"))]
        {
            // SAFETY: flush the complete live executable range process-wide,
            // including future execution after migration to another CPU.
            if unsafe {
                libc::syscall(
                    259 as libc::c_long, // Linux RISC-V UAPI: __NR_arch_specific_syscall (244) + 15.
                    self.pointer.as_ptr(),
                    self.pointer.as_ptr().add(bytes.len()),
                    0,
                )
            } != 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        #[cfg(target_arch = "riscv64")]
        // SAFETY: complete local instruction synchronization after publishing code,
        // in addition to Linux's process-wide flush for CPU migration.
        unsafe {
            core::arch::asm!("fence.i", options(nostack, preserves_flags));
        }
        // x86-64 has coherent instruction/data caches, including virtual aliases.
        // Publish bytes before handing the executable entry point to the caller.
        std::sync::atomic::fence(std::sync::atomic::Ordering::SeqCst);
        self.executable = true;
        Ok(())
    }
    fn protect(&self, execute: bool) -> io::Result<()> {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            unsafe extern "C" {
                fn pthread_jit_write_protect_np(enabled: i32);
            }
            // SAFETY: switches this thread between writing and executing MAP_JIT pages.
            unsafe {
                pthread_jit_write_protect_np(i32::from(execute));
            }
        }
        #[cfg(all(unix, not(all(target_os = "macos", target_arch = "aarch64"))))]
        {
            let flags = libc::PROT_READ
                | if execute {
                    libc::PROT_EXEC
                } else {
                    libc::PROT_WRITE
                };
            // SAFETY: the mapping is live, page aligned, and owns exactly len bytes.
            if unsafe { libc::mprotect(self.pointer.as_ptr().cast(), self.len, flags) } != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::{
                Diagnostics::Debug::FlushInstructionCache, Memory::*, Threading::GetCurrentProcess,
            };
            let flags = if execute {
                PAGE_EXECUTE_READ
            } else {
                PAGE_READWRITE
            };
            let mut old = 0;
            // SAFETY: live owned VirtualAlloc region, complete output field; flush before exposing executable bytes.
            unsafe {
                if VirtualProtect(self.pointer.as_ptr().cast(), self.len, flags, &mut old) == 0 {
                    return Err(io::Error::last_os_error());
                }
                if execute
                    && FlushInstructionCache(
                        GetCurrentProcess(),
                        self.pointer.as_ptr().cast(),
                        self.len,
                    ) == 0
                {
                    return Err(io::Error::last_os_error());
                }
            }
        }
        Ok(())
    }
    #[cfg(target_arch = "aarch64")]
    fn flush_instruction_cache(&self, len: usize) {
        #[cfg(target_os = "macos")]
        {
            unsafe extern "C" {
                fn sys_icache_invalidate(start: *const u8, len: usize);
            }
            // SAFETY: entire range belongs to the live code mapping just written.
            unsafe {
                sys_icache_invalidate(self.pointer.as_ptr(), len);
            }
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        // SAFETY: cache maintenance affects only our live mappings. CTR_EL0
        // supplies line sizes; clean writable aliases before invalidating RX.
        unsafe {
            let ctr: u64;
            core::arch::asm!("mrs {}, ctr_el0",out(reg) ctr,options(nostack,preserves_flags));
            let data_line = 4usize << ((ctr >> 16) & 15);
            let instruction_line = 4usize << (ctr & 15);
            let write = self.writable.unwrap_or(self.pointer).as_ptr().addr();
            for p in ((write & !(data_line - 1))..write + len).step_by(data_line) {
                core::arch::asm!("dc cvau, {}",in(reg) p,options(nostack,preserves_flags));
            }
            core::arch::asm!("dsb ish", options(nostack, preserves_flags));
            let execute = self.pointer.as_ptr().addr();
            for p in ((execute & !(instruction_line - 1))..execute + len).step_by(instruction_line)
            {
                core::arch::asm!("ic ivau, {}",in(reg) p,options(nostack,preserves_flags));
            }
            core::arch::asm!("dsb ish", "isb", options(nostack, preserves_flags));
        }
    }
    pub fn entry(&self) -> *const u8 {
        assert!(self.executable);
        self.pointer.as_ptr()
    }
}
impl Drop for Executable {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        // SAFETY: aliases are owned by this object and no borrowed call can outlive them.
        unsafe {
            if self.single.is_none() {
                libc::munmap(self.pointer.as_ptr().cast(), self.len);
            }
            if let Some(write) = self.writable {
                libc::munmap(write.as_ptr().cast(), self.len);
            }
        }
    }
}
// SAFETY: mutation requires exclusive access; immutable mappings can execute on
// any thread. Generated code uses only call-local registers, stack and arguments.
unsafe impl Send for Executable {}
// SAFETY: shared references cannot publish code or unmap it; read/execute accesses
// to an immutable mapping are safe across threads.
unsafe impl Sync for Executable {}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
mod tests {
    use super::*;
    fn permissions(pointer: *const u8) -> String {
        let address = pointer.addr();
        std::fs::read_to_string("/proc/self/maps")
            .unwrap()
            .lines()
            .find_map(|line| {
                let mut fields = line.split_whitespace();
                let (start, end) = fields.next()?.split_once('-')?;
                let start = usize::from_str_radix(start, 16).ok()?;
                let end = usize::from_str_radix(end, 16).ok()?;
                (start <= address && address < end).then(|| fields.next().unwrap().to_owned())
            })
            .unwrap()
    }
    #[test]
    fn mappings_never_rwx_and_recompilation_is_visible() {
        for mut code in [
            Executable::new(4096).unwrap(),
            Executable::single_mapping(4096).unwrap(),
        ] {
            for value in [7u32, 42, u32::MAX, 0] {
                let mut bytes = vec![0xb8];
                bytes.extend_from_slice(&value.to_le_bytes());
                bytes.push(0xc3);
                code.publish(&bytes).unwrap();
                assert!(permissions(code.entry()).starts_with("r-x"));
                if let Some(write) = code.writable {
                    assert!(permissions(write.as_ptr()).starts_with("rw-"));
                }
                // SAFETY: the emitted function is mov eax, imm32; ret, obeying
                // the System V ABI with no stack or memory access.
                unsafe {
                    let function: unsafe extern "C" fn() -> u32 = std::mem::transmute(code.entry());
                    assert_eq!(function(), value);
                }
            }
        }
    }
}
