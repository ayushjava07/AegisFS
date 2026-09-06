//! Typed identifiers for the Runvane domain.
//!
//! Every entity that crosses a persistence, API, or RPC boundary is addressed
//! by a strongly-typed identifier rather than a raw string. Each id type:
//!
//! * has a fixed, checked prefix (`wf_`, `rn_`, `tr_`, `hn_`, `tn_`, `cs_`);
//! * can be constructed from an untyped string and rejects malformed values;
//! * serializes to/from JSON as its plain string form;
//! * implements `Display`/`FromStr` so `format!` and parsing stay trivial.
//!
//! The Go-originated plan required "typed ids"; in Rust the newtype pattern
//! gives us the same guarantee at zero runtime cost while keeping the ability
//! to store ids as `TEXT` in SQL and as `bytes` in gRPC messages.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Maximum supported length for any entity id, enforced by the parsers below.
pub const MAX_ID_LEN: usize = 96;

/// Base32-hex alphabet used for compact, case-insensitive ids.
///
/// Durations and uuids are encoded with this alphabet instead of hex to keep
/// ids short and URL-safe without any special-casing of the alphabet.
const B32HEX: &[u8; 32] = b"0123456789abcdefghijklmnopqrstuv";

/// Encodes `src` as lowercase base32hex without padding.
pub fn encode_base32hex(src: &[u8]) -> String {
    let mut out = String::with_capacity((src.len() * 8).div_ceil(5));
    let mut buffer: u64 = 0;
    let mut bits: u32 = 0;
    for &byte in src {
        buffer = (buffer << 8) | u64::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(char::from(B32HEX[((buffer >> bits) & 0x1f) as usize]));
        }
    }
    if bits > 0 {
        out.push(char::from(B32HEX[((buffer << (5 - bits)) & 0x1f) as usize]));
    }
    out
}

/// Decodes a base32hex string into bytes, rejecting invalid characters,
/// length overruns, and encodings that would exceed the destination size.
pub fn decode_base32hex(input: &str, max_bytes: usize) -> Result<Vec<u8>, IdError> {
    if input.is_empty() {
        return Err(IdError::Malformed {
            kind: "entity",
            id: input.to_owned(),
            reason: "empty id body".to_owned(),
        });
    }
    let mut out = Vec::with_capacity((input.len() * 5) / 8 + 1);
    let mut buffer: u64 = 0;
    let mut bits: u32 = 0;
    for ch in input.chars() {
        let value = match ch {
            '0'..='9' => ch as u32 - '0' as u32,
            'a'..='v' => ch as u32 - 'a' as u32 + 10,
            _ => {
                return Err(IdError::Malformed {
                    kind: "entity",
                    id: input.to_owned(),
                    reason: format!("invalid base32hex character {ch:?}"),
                })
            }
        };
        buffer = (buffer << 5) | u64::from(value);
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    if out.len() > max_bytes {
        return Err(IdError::TooLong {
            id: input.to_owned(),
            limit: max_bytes,
        });
    }
    Ok(out)
}

/// Errors produced while parsing or constructing an identifier.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdError {
    /// The value does not carry the required prefix.
    #[error("id {id:?} must start with prefix {prefix:?}")]
    WrongPrefix {
        /// The offending value.
        id: String,
        /// The prefix the type requires.
        prefix: &'static str,
    },
    /// The id is structurally malformed for its type.
    #[error("malformed {kind} id {id:?}: {reason}")]
    Malformed {
        /// Human-readable entity kind ("run", "task", ...).
        kind: &'static str,
        /// The offending value.
        id: String,
        /// Why the value is malformed.
        reason: String,
    },
    /// The id is too long for its typed body.
    #[error("id {id:?} exceeds {limit} bytes of payload")]
    TooLong {
        /// The offending value.
        id: String,
        /// Maximum acceptable payload length.
        limit: usize,
    },
}

/// Shared definition of an id newtype.
///
/// The macro centralizes the boilerplate (newtype, prefix constant, from/parse,
/// serde, display, equality, hash) while keeping the per-type behaviour —
/// payload length and validation — in one place per type.
macro_rules! define_id {
    (
        $(#[$meta:meta])*
        $name:ident,
        $prefix:expr,
        $max_payload:expr,
        $kind:expr,
        $doc:expr
    ) => {
        #[doc = $doc]
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        $(#[$meta])*
        pub struct $name(String);

        impl $name {
            /// The required prefix for ids of this type.
            pub const PREFIX: &'static str = $prefix;

            /// Maximum payload length preserved by `decode_base32hex`.
            pub const MAX_PAYLOAD: usize = $max_payload;

            /// Builds an id from a prefix-validated string.
            ///
            /// This is the public constructor for values that were already
            /// validated (e.g. re-hydrated from storage or a request body
            /// passed through [`Self::parse`]).
            pub fn from_validated(inner: String) -> Self {
                debug_assert!(
                    inner.starts_with(Self::PREFIX),
                    "id must start with {}",
                    Self::PREFIX
                );
                Self(inner)
            }

            /// Returns the raw string form of the id.
            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }

            /// Consumes the id, returning its raw string form.
            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.0.as_str())
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                // Debug shows the raw id, not the enum variant name, so logs
                // read naturally while remaining type-safe.
                fmt::Display::fmt(self, f)
            }
        }

        impl FromStr for $name {
            type Err = IdError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::parse(s)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let s = String::deserialize(deserializer)?;
                Self::parse(&s).map_err(serde::de::Error::custom)
            }
        }

        impl $name {
            /// Parses and validates a raw id string.
            pub fn parse(value: &str) -> Result<Self, IdError> {
                if !value.starts_with(Self::PREFIX) {
                    return Err(IdError::WrongPrefix {
                        id: value.to_owned(),
                        prefix: Self::PREFIX,
                    });
                }
                let body = &value[Self::PREFIX.len()..];
                if value.len() > MAX_ID_LEN {
                    return Err(IdError::TooLong {
                        id: value.to_owned(),
                        limit: MAX_ID_LEN,
                    });
                }
                if body.is_empty() {
                    return Err(IdError::Malformed {
                        kind: $kind,
                        id: value.to_owned(),
                        reason: "missing id body".to_owned(),
                    });
                }
                // The body must round-trip through the base32hex decoder so a
                // duplicate/aliased encoding can never slip through.
                decode_base32hex(body, Self::MAX_PAYLOAD)?;
                Ok(Self(value.to_owned()))
            }
        }
    };
}

// The concrete typename receives the payload inside the body; we keep the
// string value (not a decoded blob) so round-tripping preserves formatting.

define_id!(
    /// Identifier for a workflow definition record.
    WorkflowId,
    "wf_",
    24,
    "wf",
    "Typed identifier for a workflow definition record."
);

define_id!(
    /// Identifier for one submitted run of a workflow.
    RunId,
    "rn_",
    26,
    "run",
    "Typed identifier for a single submitted run."
);

define_id!(
    /// Identifier for a single task execution inside a run.
    TaskRunId,
    "tr_",
    26,
    "task",
    "Typed identifier for one task-run record."
);

define_id!(
    /// Identifier for a webhook endpoint registered for event delivery.
    HookId,
    "hx_",
    24,
    "hook",
    "Typed identifier for a registered webhook endpoint."
);

define_id!(
    /// Identifier for an API token or credential row.
    TokenId,
    "tk_",
    24,
    "token",
    "Typed identifier for an API token row."
);

define_id!(
    /// Identifier for a compacted history summary row.
    CompactId,
    "cs_",
    24,
    "compact",
    "Typed identifier for a history-compaction summary row."
);

define_id!(
    /// Identifier for an audit log record.
    AuditRecordId,
    "au_",
    26,
    "audit",
    "Typed identifier for an audit log record."
);

/// Identifiers that carry a small, human-assigned lowercase string (tenant,
/// definition name, plugin handler name) rather than a generated blob.
///
/// These are not prefixed with a base32hex body; they are validated against a
/// narrow character set and length range instead.
pub fn validate_tenant_id(value: &str) -> Result<(), IdError> {
    if value.is_empty() || value.len() > 48 {
        return Err(IdError::Malformed {
            kind: "tenant",
            id: value.to_owned(),
            reason: "tenant id must be 1..=48 characters".to_owned(),
        });
    }
    let bytes = value.as_bytes();
    if !bytes[0].is_ascii_lowercase() {
        return Err(IdError::Malformed {
            kind: "tenant",
            id: value.to_owned(),
            reason: "tenant id must start with a lowercase letter".to_owned(),
        });
    }
    if bytes
        .iter()
        .any(|b| !(b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-'))
    {
        return Err(IdError::Malformed {
            kind: "tenant",
            id: value.to_owned(),
            reason: "tenant id may only contain [a-z0-9-]".to_owned(),
        });
    }
    Ok(())
}

/// A tenant/workspace identifier.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TenantId(String);

impl TenantId {
    /// Parses and validates a tenant id.
    pub fn parse(value: &str) -> Result<Self, IdError> {
        validate_tenant_id(value)?;
        Ok(Self(value.to_owned()))
    }

    /// Constructs a tenant id from an already-validated string.
    pub fn from_validated(inner: String) -> Self {
        debug_assert!(validate_tenant_id(&inner).is_ok());
        Self(inner)
    }

    /// Returns the tenant id as a string slice.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for TenantId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for TenantId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl FromStr for TenantId {
    type Err = IdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

/// A task-handler name, e.g. `runvane.http.call` or `runvane.shell.stdout`.
///
/// Handler ids are registered in the handler registry and referenced by task
/// specs. They share the character-set rules of names but allow dots so plugin
/// families can namespace their handlers.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct HandlerId(String);

/// Maximum length of a handler id.
pub const MAX_HANDLER_LEN: usize = 80;

impl HandlerId {
    /// Parses and validates a handler id.
    ///
    /// Names must start with a lowercase letter and continue with lowercase
    /// letters, digits, `.` or `-`. Dot segments cannot be empty (`a..b` is
    /// rejected).
    pub fn parse(value: &str) -> Result<Self, IdError> {
        let bytes = value.as_bytes();
        if value.is_empty() || value.len() > MAX_HANDLER_LEN {
            return Err(IdError::Malformed {
                kind: "handler",
                id: value.to_owned(),
                reason: "handler id must be 1..=80 characters".to_owned(),
            });
        }
        if !bytes[0].is_ascii_lowercase() {
            return Err(IdError::Malformed {
                kind: "handler",
                id: value.to_owned(),
                reason: "handler id must start with a lowercase letter".to_owned(),
            });
        }
        let mut prev_dot = false;
        for &b in bytes {
            if b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' {
                prev_dot = false;
            } else if b == b'.' {
                if prev_dot {
                    return Err(IdError::Malformed {
                        kind: "handler",
                        id: value.to_owned(),
                        reason: "handler id may not contain empty dot segments".to_owned(),
                    });
                }
                prev_dot = true;
            } else {
                return Err(IdError::Malformed {
                    kind: "handler",
                    id: value.to_owned(),
                    reason: "handler id may only contain [a-z0-9.-]".to_owned(),
                });
            }
        }
        if prev_dot {
            return Err(IdError::Malformed {
                kind: "handler",
                id: value.to_owned(),
                reason: "handler id may not end with a dot".to_owned(),
            });
        }
        Ok(Self(value.to_owned()))
    }

    /// Constructs a handler id from an already-validated string.
    pub fn from_validated(inner: String) -> Self {
        debug_assert!(Self::parse(&inner).is_ok());
        Self(inner)
    }

    /// Returns the handler id as a string slice.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for HandlerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for HandlerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl FromStr for HandlerId {
    type Err = IdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

/// Generates a fresh random id body for the given prefix using
/// [`uuid::Uuid::new_v4`]. The host RNG is used; the scheduler layer is
/// responsible for seeding a deterministic RNG under test when stable ids are
/// required.
pub fn generate_id(prefix: &'static str) -> String {
    let uuid = uuid::Uuid::new_v4();
    format!("{prefix}{}", encode_base32hex(uuid.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn run_id_parses_and_round_trips() {
        let raw = generate_id("rn_");
        let parsed = RunId::parse(&raw).expect("generated id parses");
        assert_eq!(parsed.as_str(), raw);
        assert_eq!(parsed.to_string(), raw);
        assert_eq!(RunId::from_str(&raw).unwrap(), parsed);
    }

    #[test]
    fn wrong_prefix_is_rejected() {
        let err = RunId::parse("wf_aaaa").unwrap_err();
        assert!(matches!(err, IdError::WrongPrefix { .. }));
    }

    #[test]
    fn empty_body_is_rejected() {
        let err = WorkflowId::parse("wf_").unwrap_err();
        assert!(matches!(err, IdError::Malformed { .. }));
    }

    #[test]
    fn oversized_id_is_rejected() {
        let long = format!("rn_{}", "a".repeat(MAX_ID_LEN));
        let err = RunId::parse(&long).unwrap_err();
        assert!(matches!(err, IdError::TooLong { .. }));
    }

    #[test]
    fn invalid_base32_character_is_rejected() {
        let err = RunId::parse("rn_z!").unwrap_err();
        assert!(matches!(err, IdError::Malformed { .. }));
    }

    #[test]
    fn node_encode_decoder_round_trip() {
        let raw = [0u8, 1, 2, 3, 0xff, 0xaa, 0x55];
        let enc = encode_base32hex(&raw);
        let dec = decode_base32hex(&enc, 16).unwrap();
        assert_eq!(dec, raw);
        // Uppercase is not part of the alphabet.
        assert!(decode_base32hex(&enc.to_uppercase(), 16).is_err());
    }

    #[test]
    fn tenant_id_validation() {
        assert!(TenantId::parse("acme").is_ok());
        assert!(TenantId::parse("a-1-b").is_ok());
        assert!(TenantId::parse("Acme").is_err());
        assert!(TenantId::parse("1acme").is_err());
        assert!(TenantId::parse("acme!").is_err());
        assert!(TenantId::parse("").is_err());
        assert!(TenantId::parse("a".repeat(49).as_str()).is_err());
    }

    #[test]
    fn handler_id_validation() {
        assert!(HandlerId::parse("runvane.http.call").is_ok());
        assert!(HandlerId::parse("shell").is_ok());
        assert!(HandlerId::parse("runvane..dup").is_err());
        assert!(HandlerId::parse("runvane.").is_err());
        assert!(HandlerId::parse(".leading").is_err());
        assert!(HandlerId::parse("UPPER").is_err());
        assert!(HandlerId::parse("has space").is_err());
    }

    #[test]
    fn serde_round_trip_via_json() {
        let id = RunId::parse(&generate_id("rn_")).unwrap();
        let json = serde_json::to_string(&id).unwrap();
        let back: RunId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);

        // A malformed string fails to deserialize.
        let bad: Result<RunId, _> = serde_json::from_str("\"wf_bad\"");
        assert!(bad.is_err());
    }

    #[test]
    fn audit_record_id_round_trips() {
        let raw = generate_id("au_");
        let parsed = AuditRecordId::parse(&raw).expect("valid audit id");
        assert_eq!(parsed.as_str(), raw);
        assert_eq!(parsed.to_string(), raw);
    }
}
