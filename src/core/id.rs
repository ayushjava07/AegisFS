use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::error::{AegisError, AegisResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub struct ChunkId([u8; 32]);

impl ChunkId {
    pub fn from_data(data: &[u8]) -> Self {
        Self(Sha256::digest(data).into())
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    pub fn nil() -> Self {
        Self([0u8; 32])
    }

    pub fn is_nil(&self) -> bool {
        self.0.iter().all(|&b| b == 0)
    }
}

impl fmt::Display for ChunkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

impl FromStr for ChunkId {
    type Err = AegisError;
    fn from_str(s: &str) -> AegisResult<Self> {
        let bytes = hex::decode(s)
            .map_err(|e| AegisError::InvalidArgument(format!("invalid hex: {}", e)))?;
        if bytes.len() != 32 {
            return Err(AegisError::InvalidArgument(
                "chunk id must be 32 bytes".into(),
            ));
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(Self(arr))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HashValue([u8; 32]);

impl HashValue {
    pub fn sha256(data: &[u8]) -> Self {
        Self(Sha256::digest(data).into())
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    pub fn verify(&self, data: &[u8]) -> bool {
        let computed = Sha256::digest(data);
        computed.as_slice() == self.0
    }

    pub fn nil() -> Self {
        Self([0u8; 32])
    }
}

impl Default for HashValue {
    fn default() -> Self {
        Self::nil()
    }
}

impl fmt::Display for HashValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

impl FromStr for HashValue {
    type Err = AegisError;
    fn from_str(s: &str) -> AegisResult<Self> {
        let bytes = hex::decode(s)
            .map_err(|e| AegisError::InvalidArgument(format!("invalid hex: {}", e)))?;
        if bytes.len() != 32 {
            return Err(AegisError::InvalidArgument("hash must be 32 bytes".into()));
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(Self(arr))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(Uuid);

impl NodeId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }

    pub fn nil() -> Self {
        Self(Uuid::nil())
    }

    pub fn root() -> Self {
        Self(Uuid::from_bytes([
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
        ]))
    }
}

impl Default for NodeId {
    fn default() -> Self {
        Self::nil()
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for NodeId {
    type Err = AegisError;
    fn from_str(s: &str) -> AegisResult<Self> {
        let uuid = Uuid::from_str(s)
            .map_err(|e| AegisError::InvalidArgument(format!("invalid node id: {}", e)))?;
        Ok(Self(uuid))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SnapshotId(Uuid);

impl SnapshotId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }

    pub fn nil() -> Self {
        Self(Uuid::nil())
    }
}

impl Default for SnapshotId {
    fn default() -> Self {
        Self::nil()
    }
}

impl fmt::Display for SnapshotId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for SnapshotId {
    type Err = AegisError;
    fn from_str(s: &str) -> AegisResult<Self> {
        let uuid = Uuid::from_str(s)
            .map_err(|e| AegisError::InvalidArgument(format!("invalid snapshot id: {}", e)))?;
        Ok(Self(uuid))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ArchiveId(Uuid);

impl ArchiveId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }

    pub fn nil() -> Self {
        Self(Uuid::nil())
    }
}

impl Default for ArchiveId {
    fn default() -> Self {
        Self::nil()
    }
}

impl fmt::Display for ArchiveId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for ArchiveId {
    type Err = AegisError;
    fn from_str(s: &str) -> AegisResult<Self> {
        let uuid = Uuid::from_str(s)
            .map_err(|e| AegisError::InvalidArgument(format!("invalid archive id: {}", e)))?;
        Ok(Self(uuid))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ManifestId(Uuid);

impl ManifestId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }

    pub fn nil() -> Self {
        Self(Uuid::nil())
    }
}

impl Default for ManifestId {
    fn default() -> Self {
        Self::nil()
    }
}

impl fmt::Display for ManifestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for ManifestId {
    type Err = AegisError;
    fn from_str(s: &str) -> AegisResult<Self> {
        let uuid = Uuid::from_str(s)
            .map_err(|e| AegisError::InvalidArgument(format!("invalid manifest id: {}", e)))?;
        Ok(Self(uuid))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskId(Uuid);

impl TaskId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl Default for TaskId {
    fn default() -> Self {
        Self(Uuid::nil())
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(Uuid);

impl SessionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self(Uuid::nil())
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
