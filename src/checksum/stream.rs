use std::io::{Read, Write};

use sha2::{Digest, Sha256};

use crate::core::error::{AegisError, AegisResult};
use crate::core::id::HashValue;

pub struct HashStream<R: Read> {
    inner: R,
    hasher: Sha256,
}

impl<R: Read> HashStream<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
        }
    }

    pub fn finalize(self) -> HashValue {
        let result = self.hasher.finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&result);
        HashValue::from_bytes(bytes)
    }

    pub fn into_inner(self) -> R {
        self.inner
    }
}

impl<R: Read> Read for HashStream<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n > 0 {
            self.hasher.update(&buf[..n]);
        }
        Ok(n)
    }
}

pub struct ChecksummedReader<R: Read> {
    inner: R,
    hasher: Sha256,
    expected: HashValue,
    verified: bool,
}

impl<R: Read> ChecksummedReader<R> {
    pub fn new(inner: R, expected: HashValue) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
            expected,
            verified: false,
        }
    }

    pub fn verify(&mut self) -> AegisResult<()> {
        let result = self.hasher.clone().finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&result);
        let computed = HashValue::from_bytes(bytes);

        if computed != self.expected {
            return Err(AegisError::ChecksumMismatch {
                expected: self.expected.to_hex(),
                actual: computed.to_hex(),
            });
        }
        self.verified = true;
        Ok(())
    }

    pub fn is_verified(&self) -> bool {
        self.verified
    }

    pub fn into_inner(self) -> R {
        self.inner
    }
}

impl<R: Read> Read for ChecksummedReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n > 0 {
            self.hasher.update(&buf[..n]);
        }
        Ok(n)
    }
}

pub struct ChecksummedWriter<W: Write> {
    inner: W,
    hasher: Sha256,
    computed: Option<HashValue>,
}

impl<W: Write> ChecksummedWriter<W> {
    pub fn new(inner: W) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
            computed: None,
        }
    }

    pub fn finalize(&mut self) -> AegisResult<HashValue> {
        self.inner.flush()?;
        let result = self.hasher.clone().finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&result);
        let hash = HashValue::from_bytes(bytes);
        self.computed = Some(hash);
        Ok(hash)
    }

    pub fn computed_hash(&self) -> Option<HashValue> {
        self.computed
    }

    pub fn into_inner(self) -> W {
        self.inner
    }
}

impl<W: Write> Write for ChecksummedWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        if n > 0 {
            self.hasher.update(&buf[..n]);
        }
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}
