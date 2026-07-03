use std::collections::HashMap;
use std::sync::Arc;

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, error, info};

use crate::core::error::{AegisError, AegisResult};
use crate::core::traits::{Connection, Listener, NetworkTransport, RpcService};
use crate::network::ConnectionPool;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RpcRequest {
    pub method: String,
    pub request_id: u64,
    pub payload: Vec<u8>,
}

impl RpcRequest {
    pub fn new(method: impl Into<String>, request_id: u64, payload: Vec<u8>) -> Self {
        Self {
            method: method.into(),
            request_id,
            payload,
        }
    }

    pub fn serialize(&self) -> AegisResult<Vec<u8>> {
        bincode::serialize(self)
            .map_err(|e| AegisError::SerializationError(format!("rpc request serialize: {}", e)))
    }

    pub fn deserialize(data: &[u8]) -> AegisResult<Self> {
        bincode::deserialize(data).map_err(|e| {
            AegisError::DeserializationError(format!("rpc request deserialize: {}", e))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RpcResponse {
    pub request_id: u64,
    pub payload: Vec<u8>,
    pub error: Option<String>,
}

impl RpcResponse {
    pub fn new(request_id: u64, payload: Vec<u8>) -> Self {
        Self {
            request_id,
            payload,
            error: None,
        }
    }

    pub fn error(request_id: u64, message: impl Into<String>) -> Self {
        Self {
            request_id,
            payload: Vec::new(),
            error: Some(message.into()),
        }
    }

    pub fn serialize(&self) -> AegisResult<Vec<u8>> {
        bincode::serialize(self)
            .map_err(|e| AegisError::SerializationError(format!("rpc response serialize: {}", e)))
    }

    pub fn deserialize(data: &[u8]) -> AegisResult<Self> {
        bincode::deserialize(data).map_err(|e| {
            AegisError::DeserializationError(format!("rpc response deserialize: {}", e))
        })
    }
}

pub struct RpcClient {
    transport: Arc<dyn NetworkTransport>,
    pool: Arc<ConnectionPool>,
    next_request_id: Mutex<u64>,
}

impl RpcClient {
    pub fn new(transport: Arc<dyn NetworkTransport>) -> Self {
        Self {
            transport,
            pool: Arc::new(ConnectionPool::new()),
            next_request_id: Mutex::new(1),
        }
    }

    pub fn with_pool(transport: Arc<dyn NetworkTransport>, pool: Arc<ConnectionPool>) -> Self {
        Self {
            transport,
            pool,
            next_request_id: Mutex::new(1),
        }
    }

    pub async fn call(
        &self,
        endpoint: &str,
        method: &str,
        request: Vec<u8>,
    ) -> AegisResult<Vec<u8>> {
        let mut id_lock = self.next_request_id.lock().await;
        let request_id = *id_lock;
        *id_lock += 1;
        drop(id_lock);

        let rpc_req = RpcRequest::new(method, request_id, request);
        let req_bytes = rpc_req.serialize()?;

        let mut conn = self
            .pool
            .get_or_create(endpoint, self.transport.as_ref())
            .await?;
        conn.send(Bytes::from(req_bytes))
            .await
            .map_err(|e| AegisError::RpcError {
                code: -1,
                message: format!("send failed: {}", e),
            })?;

        let resp_bytes = conn.receive().await.map_err(|e| AegisError::RpcError {
            code: -1,
            message: format!("receive failed: {}", e),
        })?;

        let rpc_resp = RpcResponse::deserialize(&resp_bytes)?;

        if let Some(err) = rpc_resp.error {
            return Err(AegisError::RpcError {
                code: -2,
                message: err,
            });
        }

        if rpc_resp.request_id != request_id {
            return Err(AegisError::RpcError {
                code: -3,
                message: format!(
                    "request id mismatch: sent {} got {}",
                    request_id, rpc_resp.request_id
                ),
            });
        }

        Ok(rpc_resp.payload)
    }

    pub async fn close_all(&self) -> AegisResult<()> {
        self.pool.close_all().await
    }
}

type ServiceHandler = Arc<dyn RpcService>;

pub struct RpcServiceRegistry {
    services: RwLock<HashMap<String, ServiceHandler>>,
}

impl RpcServiceRegistry {
    pub fn new() -> Self {
        Self {
            services: RwLock::new(HashMap::new()),
        }
    }
}

impl Default for RpcServiceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl RpcServiceRegistry {
    pub async fn register(
        &self,
        name: impl Into<String>,
        handler: ServiceHandler,
    ) -> AegisResult<()> {
        let name = name.into();
        let mut services = self.services.write().await;
        if services.contains_key(&name) {
            return Err(AegisError::AlreadyExists(format!(
                "service already registered: {}",
                name
            )));
        }
        services.insert(name.clone(), handler);
        info!("registered RPC service: {}", name);
        Ok(())
    }

    pub async fn unregister(&self, name: &str) -> AegisResult<()> {
        let mut services = self.services.write().await;
        services
            .remove(name)
            .ok_or_else(|| AegisError::Internal(format!("service not found: {}", name)))?;
        info!("unregistered RPC service: {}", name);
        Ok(())
    }

    pub async fn get_handler(&self, name: &str) -> AegisResult<ServiceHandler> {
        let services = self.services.read().await;
        services
            .get(name)
            .cloned()
            .ok_or_else(|| AegisError::Internal(format!("service not found: {}", name)))
    }

    pub async fn list_services(&self) -> Vec<String> {
        let services = self.services.read().await;
        services.keys().cloned().collect()
    }

    pub async fn has_service(&self, name: &str) -> bool {
        let services = self.services.read().await;
        services.contains_key(name)
    }
}

pub struct RpcServer {
    registry: Arc<RpcServiceRegistry>,
    running: Arc<std::sync::atomic::AtomicBool>,
    shutdown_tx: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

impl RpcServer {
    pub fn new() -> Self {
        Self {
            registry: Arc::new(RpcServiceRegistry::new()),
            running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            shutdown_tx: Mutex::new(None),
        }
    }
}

impl Default for RpcServer {
    fn default() -> Self {
        Self::new()
    }
}

impl RpcServer {
    pub fn registry(&self) -> &Arc<RpcServiceRegistry> {
        &self.registry
    }

    pub async fn register_service(
        &self,
        name: impl Into<String>,
        handler: Arc<dyn RpcService>,
    ) -> AegisResult<()> {
        self.registry.register(name, handler).await
    }

    pub async fn start(&self, listener: Box<dyn Listener>) -> AegisResult<()> {
        self.running
            .store(true, std::sync::atomic::Ordering::Release);

        let (tx, mut rx) = tokio::sync::oneshot::channel::<()>();
        *self.shutdown_tx.lock().await = Some(tx);

        let registry = self.registry.clone();
        let running = self.running.clone();

        tokio::spawn(async move {
            let mut listener = listener;
            info!(
                "RPC server started on {}",
                listener.local_addr().unwrap_or_else(|_| "unknown".into())
            );

            loop {
                tokio::select! {
                    accept_result = listener.accept() => {
                        match accept_result {
                            Ok(conn) => {
                                let registry = registry.clone();
                                tokio::spawn(async move {
                                    Self::handle_connection(conn, registry).await;
                                });
                            }
                            Err(e) => {
                                error!("accept failed: {}", e);
                                if !running.load(std::sync::atomic::Ordering::Acquire) {
                                    break;
                                }
                            }
                        }
                    }
                    _ = &mut rx => {
                        info!("RPC server shutting down");
                        running.store(false, std::sync::atomic::Ordering::Release);
                        break;
                    }
                }
            }
        });

        Ok(())
    }

    async fn handle_connection(mut conn: Box<dyn Connection>, registry: Arc<RpcServiceRegistry>) {
        loop {
            let data = match conn.receive().await {
                Ok(d) => d,
                Err(e) => {
                    debug!("connection read error (client disconnect?): {}", e);
                    return;
                }
            };

            let request = match RpcRequest::deserialize(&data) {
                Ok(req) => req,
                Err(e) => {
                    error!("failed to deserialize request: {}", e);
                    let resp = RpcResponse::error(0, format!("deserialize error: {}", e));
                    if let Ok(bytes) = resp.serialize() {
                        let _ = conn.send(Bytes::from(bytes)).await;
                    }
                    return;
                }
            };

            debug!(
                "handling RPC request: {} id={}",
                request.method, request.request_id
            );

            let response = match registry.get_handler(&request.method).await {
                Ok(handler) => match handler.call(&request.method, request.payload).await {
                    Ok(payload) => RpcResponse::new(request.request_id, payload),
                    Err(e) => RpcResponse::error(request.request_id, e.to_string()),
                },
                Err(e) => RpcResponse::error(request.request_id, format!("service error: {}", e)),
            };

            match response.serialize() {
                Ok(bytes) => {
                    if let Err(e) = conn.send(Bytes::from(bytes)).await {
                        error!("failed to send response: {}", e);
                        return;
                    }
                }
                Err(e) => {
                    error!("failed to serialize response: {}", e);
                    return;
                }
            }
        }
    }

    pub async fn stop(&self) -> AegisResult<()> {
        self.running
            .store(false, std::sync::atomic::Ordering::Release);
        let mut tx = self.shutdown_tx.lock().await;
        if let Some(tx) = tx.take() {
            let _ = tx.send(());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::traits::BoxFuture;

    #[test]
    fn test_rpc_request_serialization_roundtrip() {
        let req = RpcRequest::new("test.method", 42, vec![1, 2, 3, 4]);
        let bytes = req.serialize().unwrap();
        let deserialized = RpcRequest::deserialize(&bytes).unwrap();
        assert_eq!(req, deserialized);
    }

    #[test]
    fn test_rpc_response_serialization_roundtrip() {
        let resp = RpcResponse::new(42, vec![5, 6, 7, 8]);
        let bytes = resp.serialize().unwrap();
        let deserialized = RpcResponse::deserialize(&bytes).unwrap();
        assert_eq!(resp, deserialized);
    }

    #[test]
    fn test_rpc_response_error_serialization() {
        let resp = RpcResponse::error(42, "something went wrong");
        let bytes = resp.serialize().unwrap();
        let deserialized = RpcResponse::deserialize(&bytes).unwrap();
        assert_eq!(resp, deserialized);
        assert_eq!(deserialized.error.as_deref(), Some("something went wrong"));
    }

    #[test]
    fn test_rpc_request_deserialize_invalid_data() {
        let result = RpcRequest::deserialize(b"not valid bincode");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_service_registration() {
        struct EchoService;

        impl RpcService for EchoService {
            fn call(&self, _method: &str, request: Vec<u8>) -> BoxFuture<'_, AegisResult<Vec<u8>>> {
                Box::pin(async { Ok(request) })
            }
        }

        let registry = RpcServiceRegistry::new();
        assert!(!registry.has_service("echo").await);

        registry
            .register("echo", Arc::new(EchoService))
            .await
            .unwrap();
        assert!(registry.has_service("echo").await);

        let handler = registry.get_handler("echo").await.unwrap();
        let result = handler.call("echo", vec![1, 2, 3]).await.unwrap();
        assert_eq!(result, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn test_service_registration_duplicate() {
        struct DummyService;

        impl RpcService for DummyService {
            fn call(&self, _method: &str, request: Vec<u8>) -> BoxFuture<'_, AegisResult<Vec<u8>>> {
                Box::pin(async { Ok(request) })
            }
        }

        let registry = RpcServiceRegistry::new();
        registry
            .register("dup", Arc::new(DummyService))
            .await
            .unwrap();
        let result = registry.register("dup", Arc::new(DummyService)).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_service_unregister() {
        struct DummyService;

        impl RpcService for DummyService {
            fn call(&self, _method: &str, request: Vec<u8>) -> BoxFuture<'_, AegisResult<Vec<u8>>> {
                Box::pin(async { Ok(request) })
            }
        }

        let registry = RpcServiceRegistry::new();
        registry
            .register("temp", Arc::new(DummyService))
            .await
            .unwrap();
        assert!(registry.has_service("temp").await);

        registry.unregister("temp").await.unwrap();
        assert!(!registry.has_service("temp").await);
    }

    #[tokio::test]
    async fn test_list_services() {
        struct DummyService;

        impl RpcService for DummyService {
            fn call(&self, _method: &str, request: Vec<u8>) -> BoxFuture<'_, AegisResult<Vec<u8>>> {
                Box::pin(async { Ok(request) })
            }
        }

        let registry = RpcServiceRegistry::new();
        registry
            .register("svc1", Arc::new(DummyService))
            .await
            .unwrap();
        registry
            .register("svc2", Arc::new(DummyService))
            .await
            .unwrap();

        let services = registry.list_services().await;
        assert_eq!(services.len(), 2);
        assert!(services.contains(&"svc1".to_string()));
        assert!(services.contains(&"svc2".to_string()));
    }

    #[tokio::test]
    async fn test_mock_transport_roundtrip() {
        struct LoopbackConnection {
            buffer: Arc<Mutex<Vec<u8>>>,
        }

        impl Connection for LoopbackConnection {
            fn send(&mut self, data: Bytes) -> BoxFuture<'_, AegisResult<()>> {
                let buf = self.buffer.clone();
                Box::pin(async move {
                    let mut guard = buf.lock().await;
                    *guard = data.to_vec();
                    Ok(())
                })
            }

            fn receive(&mut self) -> BoxFuture<'_, AegisResult<Bytes>> {
                let buf = self.buffer.clone();
                Box::pin(async move {
                    let guard = buf.lock().await;
                    Ok(Bytes::copy_from_slice(&guard))
                })
            }

            fn close(&mut self) -> BoxFuture<'_, AegisResult<()>> {
                Box::pin(async { Ok(()) })
            }
        }

        struct LoopbackTransport {
            buffer: Arc<Mutex<Vec<u8>>>,
        }

        impl NetworkTransport for LoopbackTransport {
            fn connect(&self, _endpoint: &str) -> BoxFuture<'_, AegisResult<Box<dyn Connection>>> {
                let buf = self.buffer.clone();
                Box::pin(async move {
                    Ok(Box::new(LoopbackConnection { buffer: buf }) as Box<dyn Connection>)
                })
            }

            fn bind(&self, _address: &str) -> BoxFuture<'_, AegisResult<Box<dyn Listener>>> {
                unimplemented!()
            }
        }

        struct EchoHandler;

        impl RpcService for EchoHandler {
            fn call(&self, _method: &str, request: Vec<u8>) -> BoxFuture<'_, AegisResult<Vec<u8>>> {
                Box::pin(async { Ok(request) })
            }
        }

        let buffer = Arc::new(Mutex::new(Vec::new()));
        let transport = Arc::new(LoopbackTransport {
            buffer: buffer.clone(),
        });

        let _client = RpcClient::new(transport);

        let request_id = 1u64;
        let req = RpcRequest::new("echo", request_id, vec![0xde, 0xad, 0xbe, 0xef]);
        let req_bytes = req.serialize().unwrap();

        {
            let mut buf = buffer.lock().await;
            *buf = req_bytes.clone();
        }

        let mut conn = LoopbackConnection {
            buffer: buffer.clone(),
        };
        conn.send(Bytes::from(req_bytes)).await.unwrap();

        let handler = Arc::new(EchoHandler);
        let response_payload = handler
            .call("echo", vec![0xde, 0xad, 0xbe, 0xef])
            .await
            .unwrap();
        let resp = RpcResponse::new(request_id, response_payload);
        let resp_bytes = resp.serialize().unwrap();

        {
            let mut buf = buffer.lock().await;
            *buf = resp_bytes;
        }

        let recv_bytes = {
            let guard = buffer.lock().await;
            guard.clone()
        };
        let response = RpcResponse::deserialize(&recv_bytes).unwrap();
        assert_eq!(response.request_id, request_id);
        assert_eq!(response.payload, vec![0xde, 0xad, 0xbe, 0xef]);
        assert!(response.error.is_none());
    }
}
