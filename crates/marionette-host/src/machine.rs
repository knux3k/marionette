//! What hardware the engine runs on, for benchmark reports (decision 0100). Hardware model
//! names only: no host name, user name or serial number is read.

/// CPU model name as the OS reports it (`"unknown"` if it cannot be read).
pub fn cpu_model() -> String {
    imp::cpu_model()
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

#[cfg(target_os = "linux")]
mod imp {
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
    use windows_sys::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};

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
}
