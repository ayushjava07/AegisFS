use std::sync::atomic::{AtomicBool, Ordering};

use bytes::Bytes;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;

use crate::core::error::{AegisError, AegisResult};
use crate::core::traits::BoxFuture;

pub struct TcpConnection {
    stream: Mutex<TcpStream>,
    closed: AtomicBool,
}

impl TcpConnection {
    pub fn new(stream: TcpStream) -> Self {
        Self {
            stream: Mutex::new(stream),
            closed: AtomicBool::new(false),
        }
    }
}

impl super::Connection for TcpConnection {
    fn send(&mut self, data: Bytes) -> BoxFuture<'_, AegisResult<()>> {
        Box::pin(async move {
            if self.closed.load(Ordering::Acquire) {
                return Err(AegisError::NetworkError("connection closed".into()));
            }
            let len = data.len() as u32;
            let len_bytes = len.to_be_bytes();
            let mut stream = self.stream.lock().await;
            stream
                .write_all(&len_bytes)
                .await
                .map_err(|e| AegisError::NetworkError(format!("send length failed: {}", e)))?;
            stream
                .write_all(&data)
                .await
                .map_err(|e| AegisError::NetworkError(format!("send data failed: {}", e)))?;
            stream
                .flush()
                .await
                .map_err(|e| AegisError::NetworkError(format!("flush failed: {}", e)))?;
            Ok(())
        })
    }

    fn receive(&mut self) -> BoxFuture<'_, AegisResult<Bytes>> {
        Box::pin(async move {
            if self.closed.load(Ordering::Acquire) {
                return Err(AegisError::NetworkError("connection closed".into()));
            }
            let mut stream = self.stream.lock().await;
            let mut len_buf = [0u8; 4];
            stream
                .read_exact(&mut len_buf)
                .await
                .map_err(|e| AegisError::NetworkError(format!("receive length failed: {}", e)))?;
            let len = u32::from_be_bytes(len_buf) as usize;
            let mut buf = vec![0u8; len];
            stream
                .read_exact(&mut buf)
                .await
                .map_err(|e| AegisError::NetworkError(format!("receive data failed: {}", e)))?;
            Ok(Bytes::from(buf))
        })
    }

    fn close(&mut self) -> BoxFuture<'_, AegisResult<()>> {
        Box::pin(async move {
            self.closed.store(true, Ordering::Release);
            let mut stream = self.stream.lock().await;
            stream
                .shutdown()
                .await
                .map_err(|e| AegisError::NetworkError(format!("shutdown failed: {}", e)))?;
            Ok(())
        })
    }
}
