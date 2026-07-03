use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::Utc;
use parking_lot::RwLock;
use sha2::{Digest, Sha256};

use crate::core::error::{AegisError, AegisResult};
use crate::core::traits::{AuthProvider, BoxFuture};
use crate::core::types::*;

#[derive(Debug, Clone)]
pub struct Permission {
    pub action: String,
    pub resource: String,
}

impl Permission {
    pub fn new(action: &str, resource: &str) -> Self {
        Self {
            action: action.to_string(),
            resource: resource.to_string(),
        }
    }

    pub fn matches(&self, action: &str, resource: &str) -> bool {
        if self.action != action && self.action != "*" {
            return false;
        }
        if self.resource == "*" {
            return true;
        }
        let res_parts: Vec<&str> = resource.split(':').collect();
        let perm_parts: Vec<&str> = self.resource.split(':').collect();
        if perm_parts.len() != res_parts.len() {
            return false;
        }
        for (p, r) in perm_parts.iter().zip(res_parts.iter()) {
            if *p != "*" && *p != *r {
                return false;
            }
        }
        true
    }
}

#[derive(Debug, Clone)]
pub struct AclEntry {
    pub principal: String,
    pub permission: Permission,
    pub grant: bool,
}

#[derive(Debug, Clone)]
pub struct AccessControlList {
    pub entries: Vec<AclEntry>,
}

impl AccessControlList {
    pub fn new(entries: Vec<AclEntry>) -> Self {
        Self { entries }
    }

    pub fn empty() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn check(&self, principal: &str, action: &str, resource: &str) -> bool {
        let mut granted = false;
        for entry in &self.entries {
            if (entry.principal == principal || entry.principal == "*")
                && entry.permission.matches(action, resource)
            {
                granted = entry.grant;
            }
        }
        granted
    }
}

pub struct SimpleAuthProvider {
    password_hashes: HashMap<String, String>,
    user_permissions: HashMap<String, Vec<String>>,
    active_tokens: Arc<RwLock<HashMap<SessionId, AuthToken>>>,
    revoked_tokens: Arc<RwLock<HashSet<SessionId>>>,
}

impl SimpleAuthProvider {
    pub fn new(
        password_hashes: HashMap<String, String>,
        user_permissions: HashMap<String, Vec<String>>,
    ) -> Self {
        Self {
            password_hashes,
            user_permissions,
            active_tokens: Arc::new(RwLock::new(HashMap::new())),
            revoked_tokens: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    fn hash_password(password: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(password.as_bytes());
        hex::encode(hasher.finalize())
    }

    pub fn with_default_users() -> Self {
        let mut pw = HashMap::new();
        pw.insert("admin".to_string(), Self::hash_password("admin123"));
        pw.insert("user".to_string(), Self::hash_password("user123"));

        let mut perms = HashMap::new();
        perms.insert(
            "admin".to_string(),
            vec![
                "read:*".to_string(),
                "write:*".to_string(),
                "admin:*".to_string(),
            ],
        );
        perms.insert(
            "user".to_string(),
            vec!["read:*".to_string(), "write:own:*".to_string()],
        );

        Self::new(pw, perms)
    }

    pub fn add_user(&mut self, username: &str, password: &str, permissions: Vec<String>) {
        self.password_hashes
            .insert(username.to_string(), Self::hash_password(password));
        self.user_permissions
            .insert(username.to_string(), permissions);
    }
}

impl AuthProvider for SimpleAuthProvider {
    fn authenticate<'a>(
        &'a self,
        credentials: &'a Credentials,
    ) -> BoxFuture<'a, AegisResult<AuthToken>> {
        let pw_hashes = self.password_hashes.clone();
        let user_perms = self.user_permissions.clone();
        let active_tokens = Arc::clone(&self.active_tokens);
        let creds_clone = match credentials {
            Credentials::Password { username, password } => {
                Credentials::Password {
                    username: username.clone(),
                    password: password.clone(),
                }
            }
            Credentials::Token { token } => {
                Credentials::Token { token: token.clone() }
            }
            Credentials::KeyPair { public_key, private_key } => {
                Credentials::KeyPair {
                    public_key: public_key.clone(),
                    private_key: private_key.clone(),
                }
            }
        };
        Box::pin(async move {
            match creds_clone {
                Credentials::Password { username, password } => {
                    let expected_hash =
                        pw_hashes
                            .get(&username)
                            .ok_or_else(|| {
                                AegisError::AuthenticationError("invalid credentials".into())
                            })?;
                    let actual_hash = {
                        let mut hasher = Sha256::new();
                        hasher.update(password.as_bytes());
                        hex::encode(hasher.finalize())
                    };
                    if *expected_hash != actual_hash {
                        return Err(AegisError::AuthenticationError("invalid credentials".into()));
                    }

                    let now = Utc::now();
                    let token = AuthToken {
                        session_id: SessionId::new(),
                        identity: username.clone(),
                        issued_at: now,
                        expires_at: now + chrono::Duration::hours(1),
                        permissions: user_perms.get(&username).cloned().unwrap_or_default(),
                    };

                    active_tokens
                        .write()
                        .insert(token.session_id, token.clone());
                    Ok(token)
                }
                Credentials::Token { token: _ } => {
                    Err(AegisError::AuthenticationError("token auth not implemented".into()))
                }
                Credentials::KeyPair { public_key: _, private_key: _ } => {
                    Err(AegisError::AuthenticationError("keypair auth not implemented".into()))
                }
            }
        })
    }

    fn authorize(&self, token: &AuthToken, action: &str, resource: &str) -> BoxFuture<'_, AegisResult<bool>> {
        let revoked = Arc::clone(&self.revoked_tokens);
        let active = Arc::clone(&self.active_tokens);

        let t = token.clone();
        let action = action.to_string();
        let resource = resource.to_string();

        Box::pin(async move {
            if revoked.read().contains(&t.session_id) {
                return Ok(false);
            }

            if active.read().get(&t.session_id).is_none() {
                return Ok(false);
            }

            if t.is_expired() {
                return Ok(false);
            }

            let perm_str = format!("{}:{}", action, resource);
            let wildcard = format!("{}:*", action);
            let admin = "admin:*".to_string();

            Ok(t.permissions.iter().any(|p| *p == perm_str || *p == wildcard || *p == admin))
        })
    }

    fn revoke(&self, token: &AuthToken) -> BoxFuture<'_, AegisResult<()>> {
        let revoked = Arc::clone(&self.revoked_tokens);
        let active = Arc::clone(&self.active_tokens);
        let sid = token.session_id;

        Box::pin(async move {
            revoked.write().insert(sid);
            active.write().remove(&sid);
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_provider() -> SimpleAuthProvider {
        SimpleAuthProvider::with_default_users()
    }

    fn admin_creds() -> Credentials {
        Credentials::Password {
            username: "admin".into(),
            password: "admin123".into(),
        }
    }

    fn user_creds() -> Credentials {
        Credentials::Password {
            username: "user".into(),
            password: "user123".into(),
        }
    }

    #[tokio::test]
    async fn authenticate_valid_credentials() {
        let provider = test_provider();
        let token = provider.authenticate(&admin_creds()).await.unwrap();
        assert_eq!(token.identity, "admin");
        assert!(!token.is_expired());
        assert!(token
            .permissions
            .contains(&"read:*".to_string()));
    }

    #[tokio::test]
    async fn authenticate_invalid_credentials() {
        let provider = test_provider();
        let result = provider
            .authenticate(&Credentials::Password {
                username: "admin".into(),
                password: "wrong".into(),
            })
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn authenticate_unknown_user() {
        let provider = test_provider();
        let result = provider
            .authenticate(&Credentials::Password {
                username: "unknown".into(),
                password: "pwd".into(),
            })
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn authorize_sufficient_permissions() {
        let provider = test_provider();
        let token = provider.authenticate(&admin_creds()).await.unwrap();
        let result = provider
            .authorize(&token, "read", "snapshot:123")
            .await
            .unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn authorize_insufficient_permissions() {
        let provider = test_provider();
        let token = provider.authenticate(&user_creds()).await.unwrap();
        let result = provider
            .authorize(&token, "admin", "archive:foo")
            .await
            .unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn authorize_wildcard_read() {
        let provider = test_provider();
        let token = provider.authenticate(&user_creds()).await.unwrap();
        let result = provider
            .authorize(&token, "read", "archive:foo")
            .await
            .unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn authorize_revoked_token() {
        let provider = test_provider();
        let token = provider.authenticate(&admin_creds()).await.unwrap();
        provider.revoke(&token).await.unwrap();
        let result = provider
            .authorize(&token, "read", "archive:foo")
            .await
            .unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn revoke_token() {
        let provider = test_provider();
        let token = provider.authenticate(&admin_creds()).await.unwrap();
        provider.revoke(&token).await.unwrap();
        let result = provider.authenticate(&admin_creds()).await.unwrap();
        assert_ne!(result.session_id, token.session_id);
    }

    #[tokio::test]
    async fn token_expiry() {
        use std::sync::Arc;
        let mut provider = test_provider();
        provider.add_user(
            "expiry_test",
            "test",
            vec!["read:*".to_string()],
        );
        let provider = provider;
        let mut token = provider
            .authenticate(&Credentials::Password {
                username: "expiry_test".into(),
                password: "test".into(),
            })
            .await
            .unwrap();
        token.expires_at = chrono::DateTime::from_timestamp_millis(0).unwrap();
        let result = provider
            .authorize(&token, "read", "archive:foo")
            .await
            .unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn revoke_twice() {
        let provider = test_provider();
        let token = provider.authenticate(&admin_creds()).await.unwrap();
        provider.revoke(&token).await.unwrap();
        let result = provider.revoke(&token).await;
        assert!(result.is_ok());
    }

    #[derive(Debug, Clone)]
    struct TestAcl {
        entries: Vec<AclEntry>,
    }

    impl TestAcl {
        fn check(&self, principal: &str, action: &str, resource: &str) -> bool {
            let mut granted = false;
            for entry in &self.entries {
                if (entry.principal == principal || entry.principal == "*")
                    && entry.permission.matches(action, resource)
                {
                    granted = entry.grant;
                }
            }
            granted
        }
    }

    #[test]
    fn acl_permission_granted() {
        let acl = AccessControlList::new(vec![AclEntry {
            principal: "alice".into(),
            permission: Permission::new("read", "archive:*"),
            grant: true,
        }]);
        assert!(acl.check("alice", "read", "archive:123"));
    }

    #[test]
    fn acl_permission_denied() {
        let acl = AccessControlList::new(vec![AclEntry {
            principal: "alice".into(),
            permission: Permission::new("read", "archive:*"),
            grant: false,
        }]);
        assert!(!acl.check("alice", "read", "archive:123"));
    }

    #[test]
    fn acl_wildcard_principal() {
        let acl = AccessControlList::new(vec![AclEntry {
            principal: "*".into(),
            permission: Permission::new("read", "snapshot:*"),
            grant: true,
        }]);
        assert!(acl.check("anyone", "read", "snapshot:456"));
    }

    #[test]
    fn acl_no_match() {
        let acl = AccessControlList::new(vec![AclEntry {
            principal: "bob".into(),
            permission: Permission::new("write", "archive:foo"),
            grant: true,
        }]);
        assert!(!acl.check("alice", "read", "archive:foo"));
    }

    #[test]
    fn acl_wildcard_action() {
        let acl = AccessControlList::new(vec![AclEntry {
            principal: "admin".into(),
            permission: Permission::new("*", "*"),
            grant: true,
        }]);
        assert!(acl.check("admin", "delete", "archive:anything"));
    }

    #[test]
    fn acl_last_entry_wins() {
        let acl = AccessControlList::new(vec![
            AclEntry {
                principal: "alice".into(),
                permission: Permission::new("read", "archive:*"),
                grant: true,
            },
            AclEntry {
                principal: "alice".into(),
                permission: Permission::new("read", "archive:*"),
                grant: false,
            },
        ]);
        assert!(!acl.check("alice", "read", "archive:123"));
    }

    #[test]
    fn permission_exact_match() {
        let p = Permission::new("read", "archive:123");
        assert!(p.matches("read", "archive:123"));
    }

    #[test]
    fn permission_wildcard_resource() {
        let p = Permission::new("read", "archive:*");
        assert!(p.matches("read", "archive:123"));
        assert!(!p.matches("write", "archive:123"));
    }

    #[test]
    fn permission_wildcard_action() {
        let p = Permission::new("*", "snapshot:456");
        assert!(p.matches("read", "snapshot:456"));
        assert!(p.matches("delete", "snapshot:456"));
    }

    #[test]
    fn permission_mismatch() {
        let p = Permission::new("read", "archive:123");
        assert!(!p.matches("write", "archive:123"));
        assert!(!p.matches("read", "archive:999"));
    }
}
