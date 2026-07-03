use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use bytes::Bytes;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener as TokioListener, TcpStream};
use tokio::sync::Mutex;
use tracing::{debug, warn};

use crate::core::error::{AegisError, AegisResult};
use crate::core::traits::{BoxFuture, Connection, Listener, NetworkTransport};

pub struct TcpTransport;

impl TcpTransport {
    pub fn new() -> Self {
        Self
    }
}

impl Default for TcpTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkTransport for TcpTransport {
    fn connect(&self, endpoint: &str) -> BoxFuture<'_, AegisResult<Box<dyn Connection>>> {
        let endpoint = endpoint.to_string();
        Box::pin(async move {
            let stream = TcpStream::connect(&endpoint)
                .await
                .map_err(|e| AegisError::NetworkError(format!("connect failed to {}: {}", endpoint, e)))?;
            debug!("connected to {}", endpoint);
            Ok(Box::new(TcpConnection::new(stream)) as Box<dyn Connection>)
        })
    }

    fn bind(&self, address: &str) -> BoxFuture<'_, AegisResult<Box<dyn Listener>>> {
        let address = address.to_string();
        Box::pin(async move {
            let listener = TokioListener::bind(&address)
                .await
                .map_err(|e| AegisError::NetworkError(format!("bind failed on {}: {}", address, e)))?;
            let local = listener
                .local_addr()
                .map_err(|e| AegisError::NetworkError(format!("get local addr failed: {}", e)))?;
            debug!("listening on {}", local);
            Ok(Box::new(TcpListener::new(listener)) as Box<dyn Listener>)
        })
    }
}

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

impl Connection for TcpConnection {
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

pub struct TcpListener {
    listener: TokioListener,
}

impl TcpListener {
    pub fn new(listener: TokioListener) -> Self {
        Self { listener }
    }
}

impl Listener for TcpListener {
    fn accept(&mut self) -> BoxFuture<'_, AegisResult<Box<dyn Connection>>> {
        Box::pin(async move {
            let (stream, peer) = self
                .listener
                .accept()
                .await
                .map_err(|e| AegisError::NetworkError(format!("accept failed: {}", e)))?;
            debug!("accepted connection from {}", peer);
            Ok(Box::new(TcpConnection::new(stream)) as Box<dyn Connection>)
        })
    }

    fn local_addr(&self) -> AegisResult<String> {
        self.listener
            .local_addr()
            .map(|a| a.to_string())
            .map_err(|e| AegisError::NetworkError(format!("local addr failed: {}", e)))
    }
}

pub struct ConnectionPool {
    idle: Mutex<HashMap<String, Vec<Box<dyn Connection>>>>,
}

impl ConnectionPool {
    pub fn new() -> Self {
        Self {
            idle: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for ConnectionPool {
    fn default() -> Self {
        Self::new()
    }
}

impl ConnectionPool {
    pub async fn get_or_create(
        &self,
        endpoint: &str,
        transport: &dyn NetworkTransport,
    ) -> AegisResult<Box<dyn Connection>> {
        let key = endpoint.to_string();
        let mut idle = self.idle.lock().await;

        if let Some(connections) = idle.get_mut(&key) {
            if let Some(conn) = connections.pop() {
                return Ok(conn);
            }
        }

        let conn = transport
            .connect(endpoint)
            .await
            .map_err(|e| AegisError::NetworkError(format!("pool connect failed: {}", e)))?;

        Ok(conn)
    }

    pub async fn recycle(&self, endpoint: &str, connection: Box<dyn Connection>) {
        let key = endpoint.to_string();
        let mut idle = self.idle.lock().await;
        idle.entry(key).or_default().push(connection);
    }

    pub async fn close_all(&self) -> AegisResult<()> {
        let mut idle = self.idle.lock().await;
        for (_key, connections) in idle.iter_mut() {
            for conn in connections.iter_mut() {
                if let Err(e) = conn.close().await {
                    warn!("error closing pooled connection: {}", e);
                }
            }
        }
        idle.clear();
        Ok(())
    }
}

pub fn encode_frame(data: &[u8]) -> Vec<u8> {
    let len = data.len() as u32;
    let mut frame = len.to_be_bytes().to_vec();
    frame.extend_from_slice(data);
    frame
}

pub fn decode_frame(data: &[u8]) -> AegisResult<(Bytes, &[u8])> {
    if data.len() < 4 {
        return Err(AegisError::NetworkError("frame too short".into()));
    }
    let (len_bytes, rest) = data.split_at(4);
    let len = u32::from_be_bytes(len_bytes.try_into().unwrap()) as usize;
    if rest.len() < len {
        return Err(AegisError::NetworkError("incomplete frame".into()));
    }
    let (payload, remaining) = rest.split_at(len);
    Ok((Bytes::copy_from_slice(payload), remaining))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frame_encode_decode() {
        let data = b"hello world";
        let frame = encode_frame(data);
        assert_eq!(frame.len(), 4 + data.len());

        let (decoded, remaining) = decode_frame(&frame).unwrap();
        assert_eq!(decoded.as_ref(), data);
        assert!(remaining.is_empty());
    }

    #[test]
    fn test_frame_decode_incomplete_header() {
        let result = decode_frame(&[0x00, 0x01]);
        assert!(result.is_err());
    }

    #[test]
    fn test_frame_decode_incomplete_payload() {
        let data = vec![0x00, 0x00, 0x00, 0x05, 0x01, 0x02];
        let result = decode_frame(&data);
        assert!(result.is_err());
    }

    #[test]
    fn test_frame_decode_multiple_frames() {
        let data1 = b"first";
        let data2 = b"second";
        let mut combined = encode_frame(data1);
        combined.extend_from_slice(&encode_frame(data2));

        let (decoded1, remaining) = decode_frame(&combined).unwrap();
        assert_eq!(decoded1.as_ref(), data1);
        let (decoded2, remaining) = decode_frame(remaining).unwrap();
        assert_eq!(decoded2.as_ref(), data2);
        assert!(remaining.is_empty());
    }

    #[tokio::test]
    async fn test_connection_pool_get_create_and_recycle() {
        struct MockConnection {
            id: u64,
        }

        impl Connection for MockConnection {
            fn send(&mut self, _data: Bytes) -> BoxFuture<'_, AegisResult<()>> {
                Box::pin(async { Ok(()) })
            }

            fn receive(&mut self) -> BoxFuture<'_, AegisResult<Bytes>> {
                Box::pin(async { Ok(Bytes::new()) })
            }

            fn close(&mut self) -> BoxFuture<'_, AegisResult<()>> {
                Box::pin(async { Ok(()) })
            }
        }

        struct MockTransport {
            next_id: std::sync::atomic::AtomicU64,
        }

        impl NetworkTransport for MockTransport {
            fn connect(&self, _endpoint: &str) -> BoxFuture<'_, AegisResult<Box<dyn Connection>>> {
                let id = self
                    .next_id
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Box::pin(async move {
                    Ok(Box::new(MockConnection { id }) as Box<dyn Connection>)
                })
            }

            fn bind(&self, _address: &str) -> BoxFuture<'_, AegisResult<Box<dyn Listener>>> {
                unimplemented!()
            }
        }

        let pool = ConnectionPool::new();
        let transport = MockTransport {
            next_id: std::sync::atomic::AtomicU64::new(0),
        };

        let conn1 = pool.get_or_create("ep1", &transport).await.unwrap();
        let conn2 = pool.get_or_create("ep1", &transport).await.unwrap();

        pool.recycle("ep1", conn1).await;
        pool.recycle("ep1", conn2).await;

        let result = pool.close_all().await;
        assert!(result.is_ok());
    }
}
