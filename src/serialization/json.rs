use crate::core::error::{AegisError, AegisResult};
use crate::core::traits::Serializer;

#[derive(Debug, Clone, Copy)]
pub struct JsonSerializer;

impl JsonSerializer {
    pub fn new() -> Self {
        Self
    }
}

impl Default for JsonSerializer {
    fn default() -> Self {
        Self::new()
    }
}

impl Serializer for JsonSerializer {
    fn serialize<T: serde::Serialize + ?Sized>(&self, value: &T) -> AegisResult<Vec<u8>> {
        serde_json::to_vec(value)
            .map_err(|e| AegisError::SerializationError(format!("json: {}", e)))
    }

    fn deserialize<T: serde::de::DeserializeOwned>(&self, data: &[u8]) -> AegisResult<T> {
        serde_json::from_slice(data)
            .map_err(|e| AegisError::DeserializationError(format!("json: {}", e)))
    }
}
