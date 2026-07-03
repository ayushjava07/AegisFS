use std::collections::HashMap;

use crate::core::error::{AegisError, AegisResult};

pub struct SystemDiagnostics;

impl SystemDiagnostics {
    pub fn new() -> Self {
        Self
    }

    pub fn cpu_info() -> String {
        #[cfg(target_os = "linux")]
        {
            std::fs::read_to_string("/proc/cpuinfo").unwrap_or_else(|_| "unavailable".into())
        }
        #[cfg(not(target_os = "linux"))]
        {
            "unsupported platform".into()
        }
    }

    pub fn memory_info() -> String {
        #[cfg(target_os = "linux")]
        {
            std::fs::read_to_string("/proc/meminfo").unwrap_or_else(|_| "unavailable".into())
        }
        #[cfg(not(target_os = "linux"))]
        {
            "unsupported platform".into()
        }
    }

    pub fn disk_info(path: &str) -> AegisResult<String> {
        #[cfg(target_os = "linux")]
        {
            let output = std::process::Command::new("df")
                .arg("-h")
                .arg(path)
                .output()
                .map_err(AegisError::Io)?;
            let info = String::from_utf8_lossy(&output.stdout).to_string();
            Ok(info)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = path;
            Ok("unsupported platform".into())
        }
    }

    pub fn os_info() -> String {
        std::process::Command::new("uname")
            .arg("-a")
            .output()
            .ok()
            .and_then(|o| {
                if o.status.success() {
                    String::from_utf8(o.stdout).ok()
                } else {
                    None
                }
            })
            .unwrap_or_else(|| std::env::consts::OS.to_string())
    }

    pub fn collect_all() -> HashMap<String, String> {
        let mut info = HashMap::new();
        info.insert("cpu".into(), Self::cpu_info());
        info.insert("memory".into(), Self::memory_info());
        if let Ok(disk) = Self::disk_info("/") {
            info.insert("disk".into(), disk);
        }
        info.insert("os".into(), Self::os_info());
        info
    }
}

impl Default for SystemDiagnostics {
    fn default() -> Self {
        Self::new()
    }
}
