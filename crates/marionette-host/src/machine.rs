//! What hardware the engine runs on, for benchmark reports (decision 0100). Hardware model
//! names only: no host name, user name or serial number is read.

/// CPU model name as the OS reports it (`"unknown"` if it cannot be read).
pub fn cpu_model() -> String {
    imp::cpu_model()
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

/// Share of all logical cores busy over `sample` (0 to 100), from the OS's whole-machine CPU
/// counters, so it includes other processes. `None` where the OS has no such counter.
pub fn cpu_busy_pct(sample: std::time::Duration) -> Option<f64> {
    let (busy0, total0) = imp::cpu_times()?;
    std::thread::sleep(sample);
    let (busy1, total1) = imp::cpu_times()?;
    let total = total1.checked_sub(total0)?;
    (total > 0).then(|| (busy1.saturating_sub(busy0)) as f64 / total as f64 * 100.0)
}

/// Time since the person last used keyboard or mouse on this machine's desktop session.
/// Windows only (`GetLastInputInfo`); `None` elsewhere or when it cannot be read.
pub fn user_idle() -> Option<std::time::Duration> {
    imp::user_idle()
}

#[cfg(target_os = "linux")]
mod imp {
    pub fn user_idle() -> Option<std::time::Duration> {
        None
    }

    /// (busy, total) jiffies over all cores.
    pub fn cpu_times() -> Option<(u64, u64)> {
        let text = std::fs::read_to_string("/proc/stat").ok()?;
        let line = text.lines().find(|l| l.starts_with("cpu "))?;
        let v: Vec<u64> = line
            .split_whitespace()
            .skip(1)
            .filter_map(|x| x.parse().ok())
            .collect();
        // user nice system idle iowait irq softirq steal
        let total: u64 = v.iter().take(8).sum();
        let idle = v.get(3)? + v.get(4).copied().unwrap_or(0);
        Some((total - idle, total))
    }

    pub fn cpu_model() -> Option<String> {
        let text = std::fs::read_to_string("/proc/cpuinfo").ok()?;
        text.lines()
            .find(|l| l.starts_with("model name") || l.starts_with("Model") || l.starts_with("Hardware"))
            .and_then(|l| l.split_once(':'))
            .map(|(_, v)| v.trim().to_string())
    }
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
    use windows_sys::Win32::System::SystemInformation::GetTickCount;
    use windows_sys::Win32::System::Threading::GetSystemTimes;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

    pub fn user_idle() -> Option<std::time::Duration> {
        let mut info = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        // SAFETY: `info` is a valid LASTINPUTINFO with its size set.
        if unsafe { GetLastInputInfo(&mut info) } == 0 {
            return None;
        }
        // SAFETY: no arguments.
        let now = unsafe { GetTickCount() };
        Some(std::time::Duration::from_millis(u64::from(
            now.wrapping_sub(info.dwTime),
        )))
    }

    fn ticks(t: &FILETIME) -> u64 {
        (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime)
    }

    /// (busy, total) in 100 ns units over all cores. Kernel time includes idle time.
    pub fn cpu_times() -> Option<(u64, u64)> {
        let zero = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let (mut idle, mut kernel, mut user) = (zero, zero, zero);
        // SAFETY: three valid out pointers.
        if unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) } == 0 {
            return None;
        }
        let total = ticks(&kernel) + ticks(&user);
        Some((total - ticks(&idle), total))
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn cpu_model() -> Option<String> {
        let key = wide(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0");
        let value = wide("ProcessorNameString");
        let mut buf = [0u16; 256];
        let mut size = (buf.len() * 2) as u32;
        // SAFETY: all pointers are valid for the call; `size` holds the buffer size in bytes.
        let status = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buf.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if status != 0 {
            return None;
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..len]))
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod imp {
    pub fn cpu_times() -> Option<(u64, u64)> {
        None
    }

    pub fn user_idle() -> Option<std::time::Duration> {
        None
    }

    pub fn cpu_model() -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn cpu_model_is_one_line() {
        let m = super::cpu_model();
        assert!(!m.is_empty() && !m.contains('\n'), "{m:?}");
    }

    #[test]
    fn cpu_busy_is_a_percentage() {
        if let Some(p) = super::cpu_busy_pct(std::time::Duration::from_millis(50)) {
            assert!((0.0..=100.0).contains(&p), "{p}");
        }
    }
}
