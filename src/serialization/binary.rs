use crate::core::error::{AegisError, AegisResult};
use crate::core::traits::Serializer;

#[derive(Debug, Clone, Copy)]
pub struct BinSerializer;

impl BinSerializer {
    pub fn new() -> Self {
        Self
    }
}

impl Default for BinSerializer {
    fn default() -> Self {
        Self::new()
    }
}

impl Serializer for BinSerializer {
    fn serialize<T: serde::Serialize + ?Sized>(&self, value: &T) -> AegisResult<Vec<u8>> {
        bincode::serialize(value)
            .map_err(|e| AegisError::SerializationError(format!("bincode: {}", e)))
    }

    fn deserialize<T: serde::de::DeserializeOwned>(&self, data: &[u8]) -> AegisResult<T> {
        bincode::deserialize(data)
            .map_err(|e| AegisError::DeserializationError(format!("bincode: {}", e)))
    }
}
