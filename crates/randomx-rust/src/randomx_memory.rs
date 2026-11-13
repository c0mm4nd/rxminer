// SPDX-License-Identifier: BSD-3-Clause
//! Platform-owned, zero-initialized RandomX memory and NUMA-aware scratchpad pool.
use crate::randomx_core::memory::Memory;
use std::{
    cell::Cell,
    io,
    marker::PhantomData,
    ptr::NonNull,
    sync::{
        atomic::{AtomicBool, AtomicI8, AtomicU8, Ordering},
        Mutex,
    },
};

static THP: AtomicI8 = AtomicI8::new(-1);
pub(crate) static JIT_HUGE: AtomicBool = AtomicBool::new(false);
pub(crate) static PREFETCH: AtomicU8 = AtomicU8::new(1);
const HUGE: usize = 2 << 20;
const GIGA: usize = 1 << 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageKind {
    Ordinary,
    Huge2M,
    Giga1G,
}
impl PageKind {
    pub fn size(self) -> usize {
        match self {
            Self::Ordinary => 4096,
            Self::Huge2M => HUGE,
            Self::Giga1G => GIGA,
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Policy {
    pub huge: bool,
    pub giga: bool,
}

pub(crate) struct Mapping {
    pub pointer: NonNull<u8>,
    pub len: usize,
    pub kind: PageKind,
}
// SAFETY: owns its mapping; all mutable access requires exclusive borrowing.
unsafe impl Send for Mapping {}
// SAFETY: shared access is read-only; the mapping outlives every borrow.
unsafe impl Sync for Mapping {}
impl Mapping {
    pub fn new(len: usize, policy: Policy) -> io::Result<Self> {
        if policy.giga {
            if let Ok(v) = Self::exact(len, PageKind::Giga1G) {
                return Ok(v);
            }
        }
        if policy.huge || policy.giga {
            if let Ok(v) = Self::exact(len, PageKind::Huge2M) {
                return Ok(v);
            }
        }
        Self::exact(len, PageKind::Ordinary)
    }
    pub fn exact(bytes: usize, kind: PageKind) -> io::Result<Self> {
        if bytes == 0 {
            return Err(io::Error::other("zero-sized mapping"));
        }
        if bytes > isize::MAX as usize {
            return Err(io::Error::other("mapping exceeds addressable slice size"));
        }
        let page = kind.size();
        #[cfg(unix)]
        let page = if kind == PageKind::Ordinary {
            // SAFETY: read-only OS page-size query, no pointers or state changes.
            let native = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
            if native <= 0 || !(native as usize).is_power_of_two() {
                return Err(io::Error::other("invalid OS page size"));
            }
            native as usize
        } else {
            page
        };
        let len = bytes
            .checked_add(page - 1)
            .map(|n| n & !(page - 1))
            .ok_or_else(|| io::Error::other("mapping size overflow"))?;
        #[cfg(unix)]
        {
            #[allow(unused_mut)]
            let mut flags = libc::MAP_PRIVATE | libc::MAP_ANONYMOUS;
            #[allow(unused_mut)]
            let mut fd = -1;
            #[cfg(target_os = "linux")]
            if kind != PageKind::Ordinary {
                flags |= libc::MAP_HUGETLB | (if kind == PageKind::Giga1G { 30 } else { 21 } << 26);
            }
            #[cfg(target_os = "macos")]
            if kind != PageKind::Ordinary {
                if kind == PageKind::Giga1G {
                    return Err(io::Error::other("1 GiB pages unavailable"));
                }
                #[allow(deprecated)]
                // Retain libc-only bindings, no additional platform dependency.
                {
                    fd = libc::VM_FLAGS_SUPERPAGE_SIZE_2MB;
                }
            }
            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            if kind != PageKind::Ordinary {
                return Err(io::Error::other("explicit pages unavailable"));
            }
            let align = cfg!(target_os = "linux")
                && kind == PageKind::Ordinary
                && page <= HUGE
                && len >= HUGE;
            let reserve = len
                .checked_add(if align { HUGE } else { 0 })
                .ok_or_else(|| io::Error::other("mapping size overflow"))?;
            // SAFETY: anonymous writable mapping; successful pages are initialized to zero by the OS.
            let raw = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    reserve,
                    libc::PROT_READ | libc::PROT_WRITE,
                    flags,
                    fd,
                    0,
                )
            };
            if raw == libc::MAP_FAILED {
                return Err(io::Error::last_os_error());
            }
            let pointer = if align {
                let start = (raw as usize + HUGE - 1) & !(HUGE - 1);
                let prefix = start - raw as usize;
                let suffix = reserve - prefix - len;
                // SAFETY: trim only page-aligned edges of our fresh mapping; retain the entire requested region.
                unsafe {
                    if prefix > 0 {
                        libc::munmap(raw, prefix);
                    }
                    if suffix > 0 {
                        libc::munmap((start + len) as *mut _, suffix);
                    }
                }
                start as *mut u8
            } else {
                raw.cast()
            };
            let out = Self {
                pointer: NonNull::new(pointer).ok_or_else(|| io::Error::other("null mapping"))?,
                len,
                kind,
            };
            #[cfg(target_os = "linux")]
            {
                if kind != PageKind::Ordinary {
                    // SAFETY: fault in our complete writable reservation now; reject remote-node/unsupported backing instead of risking SIGBUS later.
                    if unsafe { libc::madvise(pointer.cast(), len, libc::MADV_POPULATE_WRITE) } != 0
                    {
                        return Err(io::Error::last_os_error());
                    }
                } else {
                    let advice = if thp_enabled() {
                        libc::MADV_HUGEPAGE
                    } else {
                        libc::MADV_NOHUGEPAGE
                    };
                    // SAFETY: advisory policy on an owned mapping; no lifetime or permission changes.
                    unsafe {
                        libc::madvise(pointer.cast(), len, advice);
                    }
                    if len >= HUGE && std::env::var_os("RXM_COLLAPSE").is_some() {
                        // SAFETY: both calls advise our complete writable ordinary mapping.
                        // Linux UAPI MADV_COLLAPSE=25; older kernels may reject it.
                        let (populate, collapse) = unsafe {
                            (
                                libc::madvise(pointer.cast(), len, libc::MADV_POPULATE_WRITE),
                                libc::madvise(pointer.cast(), len, 25),
                            )
                        };
                        let error = (collapse != 0).then(io::Error::last_os_error);
                        if std::env::var_os("RXM_DEBUG").is_some() {
                            eprintln!("rxm: mem={pointer:p} populate={populate} collapse={collapse} error={error:?} bytes={len}");
                        }
                    }
                }
            }
            Ok(out)
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Memory::*;
            if kind == PageKind::Giga1G {
                return Err(io::Error::other("1 GiB pages unavailable"));
            }
            let mut flags = MEM_COMMIT | MEM_RESERVE;
            if kind == PageKind::Huge2M {
                enable_large_pages()?;
                flags |= MEM_LARGE_PAGES;
            }
            // SAFETY: allocate new zero-filled owned pages, no borrowed input pointer.
            let raw = unsafe { VirtualAlloc(std::ptr::null(), len, flags, PAGE_READWRITE) };
            Ok(Self {
                pointer: NonNull::new(raw.cast()).ok_or_else(io::Error::last_os_error)?,
                len,
                kind,
            })
        }
    }
}
impl Drop for Mapping {
    fn drop(&mut self) {
        #[cfg(unix)]
        // SAFETY: mapping is exclusively owned and freed exactly once with its original rounded size.
        unsafe {
            libc::munmap(self.pointer.as_ptr().cast(), self.len);
        }
        #[cfg(windows)]
        // SAFETY: release the entire owned VirtualAlloc reservation exactly once.
        unsafe {
            windows_sys::Win32::System::Memory::VirtualFree(
                self.pointer.as_ptr().cast(),
                0,
                windows_sys::Win32::System::Memory::MEM_RELEASE,
            );
        }
    }
}
#[cfg(windows)]
fn enable_large_pages() -> io::Result<()> {
    use windows_sys::Win32::{Foundation::*, Security::*, System::Threading::*};
    // SAFETY: all Win32 output buffers are complete stack objects; close the owned token on every path.
    unsafe {
        let mut token = std::ptr::null_mut();
        if OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let result = (|| {
            let mut luid = std::mem::zeroed();
            if LookupPrivilegeValueW(std::ptr::null(), SE_LOCK_MEMORY_NAME, &mut luid) == 0 {
                return Err(io::Error::last_os_error());
            }
            let privileges = TOKEN_PRIVILEGES {
                PrivilegeCount: 1,
                Privileges: [LUID_AND_ATTRIBUTES {
                    Luid: luid,
                    Attributes: SE_PRIVILEGE_ENABLED,
                }],
            };
            SetLastError(0);
            let ok = AdjustTokenPrivileges(
                token,
                0,
                &privileges,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
            if ok == 0 || GetLastError() != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        })();
        CloseHandle(token);
        result
    }
}

// Closed set of element types for which zero-filled bytes are valid, with no drop glue.
pub(crate) trait Element: Copy + Send + Sync + 'static {}
impl Element for u8 {}
impl<const N: usize> Element for [u64; N] {}
pub(crate) struct Mapped<T: Element> {
    mapping: Mapping,
    count: usize,
    _type: PhantomData<T>,
}
impl<T: Element> Mapped<T> {
    pub fn new(count: usize, policy: Policy) -> io::Result<Self> {
        let len = count
            .checked_mul(std::mem::size_of::<T>())
            .ok_or_else(|| io::Error::other("buffer size overflow"))?;
        Ok(Self {
            mapping: Mapping::new(len, policy)?,
            count,
            _type: PhantomData,
        })
    }
    pub fn kind(&self) -> PageKind {
        self.mapping.kind
    }
    pub fn into_memory(self) -> Memory<T> {
        Memory::Platform(Box::new(self))
    }
}
impl<T: Element> AsRef<[T]> for Mapped<T> {
    fn as_ref(&self) -> &[T] {
        // SAFETY: initialized zero-valid elements in a sufficiently aligned OS mapping; count fits the allocation.
        unsafe { std::slice::from_raw_parts(self.mapping.pointer.as_ptr().cast(), self.count) }
    }
}
impl<T: Element> AsMut<[T]> for Mapped<T> {
    fn as_mut(&mut self) -> &mut [T] {
        // SAFETY: exclusive borrow of exactly count initialized elements in the owned mapping.
        unsafe { std::slice::from_raw_parts_mut(self.mapping.pointer.as_ptr().cast(), self.count) }
    }
}

thread_local! { static NODE: Cell<Option<usize>> = const { Cell::new(None) }; }
struct Pool {
    capacity: usize,
    epoch: u64,
    pages: Vec<(Option<usize>, Mapping)>,
}
static POOL: Mutex<Pool> = Mutex::new(Pool {
    capacity: 0,
    epoch: 0,
    pages: Vec::new(),
});
pub fn set_node(node: Option<usize>) {
    NODE.set(node);
}
pub fn set_pool(capacity: usize) {
    let mut pool = POOL.lock().unwrap_or_else(|p| p.into_inner());
    pool.pages.clear();
    pool.capacity = capacity;
    pool.epoch = pool.epoch.wrapping_add(1);
}
#[cfg(target_os = "linux")]
fn thp_enabled() -> bool {
    match THP.load(Ordering::Relaxed) {
        -1 => std::env::var_os("RXM_THP")
            .is_none_or(|value| !value.as_encoded_bytes().starts_with(b"0")),
        value => value != 0,
    }
}
pub fn set_thp(enabled: bool) {
    THP.store(i8::from(enabled), Ordering::Relaxed);
    let mut pool = POOL.lock().unwrap_or_else(|p| p.into_inner());
    pool.pages.clear();
    pool.epoch = pool.epoch.wrapping_add(1);
}
pub(crate) struct Scratchpad {
    mapping: Option<Mapping>,
    node: Option<usize>,
    epoch: u64,
}
impl Scratchpad {
    pub fn new(huge: bool) -> io::Result<Self> {
        let node = NODE.get();
        let mut pool = POOL.lock().unwrap_or_else(|p| p.into_inner());
        let epoch = pool.epoch;
        let wanted = if huge {
            PageKind::Huge2M
        } else {
            PageKind::Ordinary
        };
        let index = pool
            .pages
            .iter()
            .position(|(n, m)| *n == node && m.kind == wanted);
        let cached = index.map(|i| pool.pages.swap_remove(i).1);
        drop(pool);
        let mapping = if let Some(v) = cached {
            v
        } else if let Ok(v) = Mapping::exact(HUGE, wanted) {
            v
        } else if huge {
            let mut pool = POOL.lock().unwrap_or_else(|p| p.into_inner());
            let cached = pool
                .pages
                .iter()
                .position(|(n, m)| *n == node && m.kind == PageKind::Ordinary)
                .map(|i| pool.pages.swap_remove(i).1);
            drop(pool);
            match cached {
                Some(v) => v,
                None => Mapping::exact(HUGE, PageKind::Ordinary)?,
            }
        } else {
            return Err(io::Error::other("scratchpad allocation failed"));
        };
        Ok(Self {
            mapping: Some(mapping),
            node,
            epoch,
        })
    }
    pub fn kind(&self) -> PageKind {
        self.mapping.as_ref().unwrap().kind
    }
    pub fn bytes(&self) -> &[u8] {
        // SAFETY: the complete 2 MiB mapping is initialized and retained for this borrow.
        unsafe { std::slice::from_raw_parts(self.mapping.as_ref().unwrap().pointer.as_ptr(), HUGE) }
    }
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        // SAFETY: exclusive access to the complete initialized scratchpad.
        unsafe {
            std::slice::from_raw_parts_mut(self.mapping.as_ref().unwrap().pointer.as_ptr(), HUGE)
        }
    }
}
impl AsRef<[u8]> for Scratchpad {
    fn as_ref(&self) -> &[u8] {
        self.bytes()
    }
}
impl AsMut<[u8]> for Scratchpad {
    fn as_mut(&mut self) -> &mut [u8] {
        self.bytes_mut()
    }
}
impl Drop for Scratchpad {
    fn drop(&mut self) {
        let mut pool = POOL.lock().unwrap_or_else(|p| p.into_inner());
        if pool.epoch == self.epoch
            && pool.pages.len() < pool.capacity
            && pool.pages.try_reserve(1).is_ok()
        {
            pool.pages.push((self.node, self.mapping.take().unwrap()));
        }
    }
}

impl<T: Element> crate::randomx_core::memory::Buffer<T> for Mapped<T> {
    fn page_size(&self) -> usize {
        if self.kind() == PageKind::Ordinary {
            0
        } else {
            self.kind().size()
        }
    }
}
impl crate::randomx_core::memory::Buffer<u8> for Scratchpad {
    fn page_size(&self) -> usize {
        if self.kind() == PageKind::Ordinary {
            0
        } else {
            self.kind().size()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mapped_memory_is_zeroed_aligned_and_reports_fallback() {
        assert!(Mapping::new(0, Policy::default()).is_err());
        assert!(Mapped::<[u64; 128]>::new(usize::MAX, Policy::default()).is_err());
        let mut data = Mapped::<[u64; 8]>::new(
            HUGE / 64,
            Policy {
                huge: true,
                giga: false,
            },
        )
        .unwrap();
        #[cfg(target_os = "linux")]
        assert_eq!(data.as_ref().as_ptr().addr() % HUGE, 0);
        assert_eq!(data.as_ref().as_ptr().addr() % 64, 0);
        assert!(data.as_ref().iter().all(|v| *v == [0; 8]));
        data.as_mut()[HUGE / 64 - 1] = [u64::MAX; 8];
        assert_eq!(data.as_ref()[HUGE / 64 - 1], [u64::MAX; 8]);
        let kind = data.kind();
        let data = data.into_memory();
        assert_eq!(
            data.page_size(),
            if kind == PageKind::Ordinary { 0 } else { HUGE }
        );
    }
    #[test]
    #[ignore = "mutates global allocator policy; run with --test-threads=1"]
    fn pool_nodes_generations_fallback_and_thp_policy() {
        set_pool(4);
        set_node(Some(123));
        set_thp(false);
        let a = Scratchpad::new(true).unwrap();
        let first = a.bytes().as_ptr();
        let kind = a.kind();
        #[cfg(target_os = "linux")]
        if kind == PageKind::Ordinary {
            let smaps = std::fs::read_to_string("/proc/self/smaps").unwrap();
            let header = format!("{:x}-", first.addr());
            let block = smaps.split(&header).nth(1).expect("mapping in smaps");
            let flags = block
                .lines()
                .find(|line| line.starts_with("VmFlags:"))
                .unwrap();
            assert!(flags.split_whitespace().any(|v| v == "nh"));
        }
        drop(a);
        let b = Scratchpad::new(true).unwrap();
        assert_eq!(b.bytes().as_ptr(), first);
        assert_eq!(b.kind(), kind);
        drop(b);
        set_node(Some(124));
        let c = Scratchpad::new(false).unwrap();
        assert_ne!(c.bytes().as_ptr(), first);
        set_pool(0);
        drop(c);
        assert!(POOL.lock().unwrap().pages.is_empty());
        set_node(None);
        set_thp(true);
    }
}
