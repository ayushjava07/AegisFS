use tokio::net::TcpListener as TokioListener;
use tracing::debug;

use crate::core::error::{AegisError, AegisResult};
use crate::core::traits::{BoxFuture, Connection, Listener, NetworkTransport};
use crate::network::connection::TcpConnection;
use crate::network::TcpListener;

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
            let stream = tokio::net::TcpStream::connect(&endpoint)
                .await
                .map_err(|e| {
                    AegisError::NetworkError(format!("connect failed to {}: {}", endpoint, e))
                })?;
            debug!("connected to {}", endpoint);
            Ok(Box::new(TcpConnection::new(stream)) as Box<dyn Connection>)
        })
    }

    fn bind(&self, address: &str) -> BoxFuture<'_, AegisResult<Box<dyn Listener>>> {
        let address = address.to_string();
        Box::pin(async move {
            let listener = TokioListener::bind(&address).await.map_err(|e| {
                AegisError::NetworkError(format!("bind failed on {}: {}", address, e))
            })?;
            let local = listener
                .local_addr()
                .map_err(|e| AegisError::NetworkError(format!("get local addr failed: {}", e)))?;
            debug!("listening on {}", local);
            Ok(Box::new(TcpListener::new(listener)) as Box<dyn Listener>)
        })
    }
}
