pub struct BuildInfo;

impl BuildInfo {
    pub fn version() -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    pub fn name() -> &'static str {
        env!("CARGO_PKG_NAME")
    }

    pub fn description() -> &'static str {
        env!("CARGO_PKG_DESCRIPTION")
    }

    pub fn commit() -> &'static str {
        option_env!("VERGEN_GIT_SHA")
            .or(option_env!("GIT_COMMIT"))
            .unwrap_or("unknown")
    }

    pub fn build_info() -> String {
        let commit = Self::commit();
        if commit == "unknown" {
            format!("{} v{}", Self::name(), Self::version())
        } else {
            format!("{} v{} (commit {})", Self::name(), Self::version(), commit)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version() {
        assert!(!BuildInfo::version().is_empty());
    }

    #[test]
    fn test_name() {
        assert_eq!(BuildInfo::name(), "aegisfs");
    }

    #[test]
    fn test_build_info() {
        let info = BuildInfo::build_info();
        assert!(info.contains("aegisfs"));
        assert!(info.contains(BuildInfo::version()));
    }
}
