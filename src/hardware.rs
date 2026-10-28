//! What this machine can run: its processor, memory and a rough class of power, which set how many threads the
//! models take and which voices and recognizers the settings suggest.

use serde::Serialize;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Weak,
    Medium,
    Strong,
}

#[derive(Debug, Serialize)]
pub struct Machine {
    /// The processor's name, such as "Apple M4" or "AMD Ryzen 5 5500U".
    pub cpu: String,
    /// Logical processors.
    pub threads: usize,
    pub ram_gb: f64,
    pub tier: Tier,
}

pub fn machine() -> &'static Machine {
    static MACHINE: OnceLock<Machine> = OnceLock::new();
    MACHINE.get_or_init(|| {
        let threads = std::thread::available_parallelism().map_or(2, |n| n.get());
        let ram_gb = ram_bytes() as f64 / f64::from(1u32 << 30);
        let apple = cfg!(all(target_os = "macos", target_arch = "aarch64"));
        Machine { cpu: cpu_name(), threads, ram_gb, tier: tier(threads, ram_gb, apple) }
    })
}

/// Two cores or 8 GB is weak; six cores with hyper-threading and 16 GB is strong. Apple's chips are fast for their
/// core count, so there memory alone decides.
fn tier(threads: usize, ram_gb: f64, apple: bool) -> Tier {
    // A little under the round numbers: the system keeps some memory for itself and reports the rest.
    let (eight, sixteen) = (7.5, 15.0);
    if apple {
        return if ram_gb >= sixteen { Tier::Strong } else { Tier::Medium };
    }
    if threads <= 4 || ram_gb < eight {
        Tier::Weak
    } else if threads >= 12 && ram_gb >= sixteen {
        Tier::Strong
    } else {
        Tier::Medium
    }
}

/// The threads a model runs on: about one per core, at most four, or six on a strong machine, which is asked to
/// work harder for its heavier models. On Apple's chips only the performance cores count, since a model waits for
/// its slowest thread.
pub fn model_threads() -> i32 {
    let m = machine();
    let most = if m.tier == Tier::Strong { 6 } else { 4 };
    let mut n = (m.threads / 2).clamp(1, most);
    if let Some(fast) = performance_cores() {
        n = n.min(fast.max(1));
    }
    n as i32
}

#[cfg(target_os = "macos")]
fn sysctl_text(name: &str) -> Option<String> {
    let name = std::ffi::CString::new(name).ok()?;
    let mut len = 0usize;
    // SAFETY: the first call only asks for the length, the second writes at most that many bytes into `buf`.
    unsafe {
        if libc::sysctlbyname(name.as_ptr(), std::ptr::null_mut(), &mut len, std::ptr::null_mut(), 0) != 0 || len == 0 {
            return None;
        }
        let mut buf = vec![0u8; len];
        if libc::sysctlbyname(name.as_ptr(), buf.as_mut_ptr().cast(), &mut len, std::ptr::null_mut(), 0) != 0 {
            return None;
        }
        buf.truncate(len);
        Some(String::from_utf8_lossy(&buf).trim_end_matches('\0').trim().to_owned())
    }
}

#[cfg(target_os = "macos")]
fn sysctl_number(name: &str) -> Option<u64> {
    let name = std::ffi::CString::new(name).ok()?;
    let mut value = 0u64;
    let mut len = std::mem::size_of::<u64>();
    // SAFETY: `value` holds the eight bytes the call is told it may write.
    let ok = unsafe { libc::sysctlbyname(name.as_ptr(), (&mut value as *mut u64).cast(), &mut len, std::ptr::null_mut(), 0) } == 0;
    // Some values are four bytes wide.
    ok.then(|| if len == 4 { value & 0xffff_ffff } else { value })
}

#[cfg(target_os = "macos")]
fn performance_cores() -> Option<usize> {
    sysctl_number("hw.perflevel0.physicalcpu").map(|n| n as usize).filter(|&n| n > 0)
}

#[cfg(not(target_os = "macos"))]
fn performance_cores() -> Option<usize> {
    None
}

#[cfg(target_os = "macos")]
fn cpu_name() -> String {
    sysctl_text("machdep.cpu.brand_string").unwrap_or_default()
}

#[cfg(target_os = "macos")]
fn ram_bytes() -> u64 {
    sysctl_number("hw.memsize").unwrap_or(0)
}

#[cfg(windows)]
fn cpu_name() -> String {
    use windows_sys::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (key, value) = (wide(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0"), wide("ProcessorNameString"));
    let mut buf = [0u16; 256];
    let mut size = std::mem::size_of_val(&buf) as u32;
    // SAFETY: the strings end in a zero and `buf` holds the `size` bytes the call may write.
    let status = unsafe {
        RegGetValueW(HKEY_LOCAL_MACHINE, key.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut size)
    };
    if status != 0 {
        return std::env::var("PROCESSOR_IDENTIFIER").unwrap_or_default();
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end]).trim().to_owned()
}

#[cfg(windows)]
fn ram_bytes() -> u64 {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    // SAFETY: the struct is plain data that the call fills, once its length is set.
    unsafe {
        let mut status: MEMORYSTATUSEX = std::mem::zeroed();
        status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        if GlobalMemoryStatusEx(&mut status) != 0 { status.ullTotalPhys } else { 0 }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn cpu_name() -> String {
    std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|info| info.lines().find(|l| l.starts_with("model name")).and_then(|l| l.split(':').nth(1)).map(|s| s.trim().to_owned()))
        .unwrap_or_default()
}

#[cfg(not(any(windows, target_os = "macos")))]
fn ram_bytes() -> u64 {
    // SAFETY: sysconf only reads system values.
    let (pages, size) = unsafe { (libc::sysconf(libc::_SC_PHYS_PAGES), libc::sysconf(libc::_SC_PAGESIZE)) };
    (pages.max(0) as u64) * (size.max(0) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machines_fall_into_tiers() {
        // An Intel N100 with 8 GB, a Core i5-8250U with 8 GB, a Ryzen 5 5500U with 16 GB, a Mac with 8 and 16 GB.
        assert_eq!(tier(4, 7.7, false), Tier::Weak);
        assert_eq!(tier(8, 3.8, false), Tier::Weak);
        assert_eq!(tier(8, 7.8, false), Tier::Medium);
        assert_eq!(tier(12, 15.4, false), Tier::Strong);
        assert_eq!(tier(10, 8.0, true), Tier::Medium);
        assert_eq!(tier(10, 16.0, true), Tier::Strong);
        let m = machine();
        assert!(m.threads >= 1 && m.ram_gb > 0.5 && !m.cpu.is_empty(), "{m:?}");
        assert!((1..=6).contains(&model_threads()));
    }
}
