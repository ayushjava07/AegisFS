//! API-token authentication and read/admin authorization.
//!
//! Two levels are defined:
//! - `Operator` for every `/v1` route except health and the dashboard;
//! - `Admin` for run cancellation (the destructive control-plane action).
//!
//! Tokens are configured in `Config` (`auth_token`, `admin_token`). When no
//! token is configured the surfaces are open (local-first default); once an
//! operator token exists, all guarded routes require it. An absent admin
//! token means the operator token also authorizes admin actions, so a
//! single-secret deployment stays usable.
//!
//! Comparison is constant-time so a token hunt cannot leak a prefix match.

use zeroize::Zeroizing;

/// The authorization level an endpoint demands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Any authenticated caller (the default for `/v1` reads and creates).
    Operator,
    /// The most sensitive control-plane action (`CancelRun`): needs the
    /// `admin_token` when configured, else falls back to the operator token.
    Admin,
}

/// The static token configuration carried by the control plane.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AuthConfig {
    /// Token required for operator-level routes; `None` opens the surface.
    pub operator_token: Option<Zeroizing<String>>,
    /// Optional distinct token for admin routes; falls back to the operator
    /// token when `None`.
    pub admin_token: Option<Zeroizing<String>>,
}

impl AuthConfig {
    /// Builds the config from the resolved `Config` values.
    pub fn from_config(operator_token: Option<String>, admin_token: Option<String>) -> Self {
        Self {
            operator_token: operator_token.map(Zeroizing::new),
            admin_token: admin_token.map(Zeroizing::new),
        }
    }

    /// Whether the surface is guarded at all.
    pub fn enabled(&self) -> bool {
        self.operator_token.is_some() || self.admin_token.is_some()
    }

    /// Whether `presented` satisfies `level`. Unauthenticated callers pass a
    /// `None` token, which only succeeds on an open surface.
    pub fn authorized(&self, presented: Option<&str>, level: Level) -> bool {
        // Without an operator secret the surface is either fully open or —
        // when only an admin secret exists — intentionally locked for the
        // operator tier (misconfiguration, surface stays closed).
        let operator = match self.operator_token.as_deref() {
            None => return !self.enabled(),
            Some(operator) => operator,
        };
        let presented = presented.unwrap_or("");
        match level {
            Level::Operator => ct_eq(presented, operator),
            // A distinct admin secret demands itself; without one the
            // operator secret covers admin so a single-secret deployment
            // stays usable. The operator secret never clears admin when a
            // separate admin secret exists.
            Level::Admin => match self.admin_token.as_deref() {
                Some(admin) => ct_eq(presented, admin),
                None => ct_eq(presented, operator),
            },
        }
    }
}

/// Timing-safe equality over two byte strings.
fn ct_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_surface_accepts_everyone() {
        let cfg = AuthConfig::default();
        assert!(!cfg.enabled());
        assert!(cfg.authorized(None, Level::Operator));
        assert!(cfg.authorized(Some("anything"), Level::Admin));
    }

    #[test]
    fn operator_token_guards_operator_routes() {
        let cfg = AuthConfig::from_config(Some("s3cret".to_owned()), None);
        assert!(cfg.enabled());
        assert!(cfg.authorized(Some("s3cret"), Level::Operator));
        assert!(!cfg.authorized(None, Level::Operator));
        assert!(!cfg.authorized(Some("wrong"), Level::Operator));
        // No admin token configured: operator secret may act as admin.
        assert!(cfg.authorized(Some("s3cret"), Level::Admin));
    }

    #[test]
    fn distinct_admin_token_gates_admin_routes() {
        let cfg = AuthConfig::from_config(Some("op".to_owned()), Some("adm".to_owned()));
        assert!(cfg.authorized(Some("op"), Level::Operator));
        assert!(
            !cfg.authorized(Some("op"), Level::Admin),
            "admin demands its own secret"
        );
        assert!(cfg.authorized(Some("adm"), Level::Admin));
        assert!(
            !cfg.authorized(Some("adm"), Level::Operator),
            "admin secret does not unlock operator routes"
        );
        assert!(!cfg.authorized(None, Level::Admin));
    }

    #[test]
    fn comparison_is_prefix_safe() {
        let cfg = AuthConfig::from_config(Some("secret-value".to_owned()), None);
        // A partial prefix must not authenticate.
        assert!(!cfg.authorized(Some("secret"), Level::Operator));
        assert!(!cfg.authorized(Some("secret-valuX"), Level::Operator));
        assert!(cfg.authorized(Some("secret-value"), Level::Operator));
        // Different lengths short-circuit (public length, harmless here).
        assert!(!cfg.authorized(Some("secret-value!".to_owned().as_str()), Level::Operator));
    }

    #[test]
    fn ct_eq_matches_only_exact_bytes() {
        assert!(ct_eq("abc", "abc"));
        assert!(!ct_eq("abc", "abd"));
        assert!(!ct_eq("abc", "abcd"));
        assert!(!ct_eq("", "a"));
        assert!(ct_eq("", ""));
    }
}
