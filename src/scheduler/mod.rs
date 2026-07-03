use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::core::error::{AegisError, AegisResult};
use crate::core::traits::*;
use crate::core::types::*;

pub struct ThreadPoolScheduler {
    tasks: Arc<Mutex<HashMap<TaskId, JoinHandle<()>>>>,
    running: Arc<AtomicBool>,
}

impl ThreadPoolScheduler {
    pub fn new() -> Self {
        Self {
            tasks: Arc::new(Mutex::new(HashMap::new())),
            running: Arc::new(AtomicBool::new(true)),
        }
    }
}

impl Default for ThreadPoolScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl Scheduler for ThreadPoolScheduler {
    fn submit<T, F>(&self, task: F) -> BoxFuture<'_, AegisResult<TaskHandle<T>>>
    where
        T: Send + 'static,
        F: Future<Output = AegisResult<T>> + Send + 'static,
    {
        let tasks = self.tasks.clone();
        let running = self.running.clone();
        Box::pin(async move {
            if !running.load(Ordering::SeqCst) {
                return Err(AegisError::Internal("scheduler is shut down".into()));
            }

            let task_id = TaskId::new();
            let handle = TaskHandle::new(task_id);
            let completed = handle.completed.clone();
            let result = handle.result.clone();

            let jh: JoinHandle<()> = tokio::spawn(async move {
                let res = task.await;
                {
                    let mut r = result.lock().unwrap();
                    *r = Some(res);
                }
                completed.store(true, Ordering::SeqCst);
            });

            tasks.lock().await.insert(task_id, jh);
            Ok(handle)
        })
    }

    fn schedule(
        &self,
        task: BoxFuture<'static, AegisResult<()>>,
        _priority: TaskPriority,
    ) -> BoxFuture<'_, AegisResult<TaskId>> {
        let tasks = self.tasks.clone();
        let running = self.running.clone();
        Box::pin(async move {
            if !running.load(Ordering::SeqCst) {
                return Err(AegisError::Internal("scheduler is shut down".into()));
            }

            let handle: JoinHandle<()> = tokio::spawn(async move {
                let _ = task.await;
            });

            let task_id = TaskId::new();
            tasks.lock().await.insert(task_id, handle);
            Ok(task_id)
        })
    }

    fn cancel(&self, task_id: TaskId) -> BoxFuture<'_, AegisResult<()>> {
        let tasks = self.tasks.clone();
        Box::pin(async move {
            let mut map = tasks.lock().await;
            if let Some(handle) = map.remove(&task_id) {
                handle.abort();
                Ok(())
            } else {
                Err(AegisError::Internal(format!("task {} not found", task_id)))
            }
        })
    }

    fn shutdown(&self) -> BoxFuture<'_, AegisResult<()>> {
        let tasks = self.tasks.clone();
        let running = self.running.clone();
        Box::pin(async move {
            running.store(false, Ordering::SeqCst);
            let mut map = tasks.lock().await;
            for (_id, handle) in map.drain() {
                handle.abort();
            }
            Ok(())
        })
    }
}

pub struct TaskRegistry {
    tasks: Arc<dashmap::DashMap<String, Arc<tokio::sync::Mutex<BoxFuture<'static, AegisResult<()>>>>>>,
}

impl TaskRegistry {
    pub fn new() -> Self {
        Self {
            tasks: Arc::new(dashmap::DashMap::new()),
        }
    }
}

impl Default for TaskRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskRegistry {
    pub fn register<F, Fut>(&self, name: &str, task_fn: F)
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = AegisResult<()>> + Send + 'static,
    {
        let task = Arc::new(tokio::sync::Mutex::new(Box::pin(async move { (task_fn)().await }) as BoxFuture<'static, AegisResult<()>>));
        self.tasks.insert(name.to_string(), task);
    }

    pub fn unregister(&self, name: &str) -> AegisResult<()> {
        self.tasks
            .remove(name)
            .ok_or_else(|| AegisError::Internal(format!("task '{}' not registered", name)))?;
        Ok(())
    }

    pub async fn run(&self, name: &str) -> AegisResult<()> {
        let task = self
            .tasks
            .get(name)
            .map(|r| r.value().clone())
            .ok_or_else(|| AegisError::Internal(format!("task '{}' not found", name)))?;
        let mut guard = task.lock().await;
        (&mut *guard).await
    }
}

fn remove_orphaned_chunks(
    storage: Arc<dyn ChunkStorage>,
    referenced: Vec<ChunkId>,
) -> BoxFuture<'static, AegisResult<u64>> {
    Box::pin(async move {
        let all = storage.list_chunks().await?;
        let ref_set: std::collections::HashSet<ChunkId> =
            referenced.iter().cloned().collect();
        let mut removed = 0u64;
        for id in &all {
            if !ref_set.contains(id) {
                storage.delete_chunk(id).await?;
                removed += 1;
            }
        }
        Ok(removed)
    })
}

pub struct GcTask {
    storage: Arc<dyn ChunkStorage>,
}

impl GcTask {
    pub fn new(storage: Arc<dyn ChunkStorage>) -> Self {
        Self { storage }
    }

    pub async fn collect_garbage(&self, referenced: &[ChunkId]) -> AegisResult<u64> {
        remove_orphaned_chunks(self.storage.clone(), referenced.to_vec()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test(flavor = "multi_thread")]
    async fn test_submit_and_complete_task() {
        let scheduler = Arc::new(ThreadPoolScheduler::new());
        let mut handle = scheduler
            .submit(async { Ok::<i32, AegisError>(42) })
            .await
            .unwrap();

        let result = handle.await_completion().unwrap();
        assert_eq!(result, 42);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_task_is_completed() {
        let scheduler = Arc::new(ThreadPoolScheduler::new());
        let mut handle = scheduler
            .submit(async {
                tokio::time::sleep(Duration::from_millis(10)).await;
                Ok::<(), AegisError>(())
            })
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(handle.is_completed());
        handle.await_completion().unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_cancel_task() {
        let scheduler = Arc::new(ThreadPoolScheduler::new());
        let mut handle = scheduler
            .submit(async {
                tokio::time::sleep(Duration::from_millis(100)).await;
                Ok::<(), AegisError>(())
            })
            .await
            .unwrap();

        handle.cancel().unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        let _ = handle.await_completion();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_schedule_fire_and_forget() {
        let scheduler = Arc::new(ThreadPoolScheduler::new());
        let flag = Arc::new(AtomicBool::new(false));
        let flag_clone = flag.clone();

        let task_id = scheduler
            .schedule(
                Box::pin(async move {
                    flag_clone.store(true, Ordering::SeqCst);
                    Ok(())
                }),
                TaskPriority::Normal,
            )
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(flag.load(Ordering::SeqCst));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_shutdown() {
        let scheduler = Arc::new(ThreadPoolScheduler::new());
        let _handle = scheduler
            .submit(async {
                tokio::time::sleep(Duration::from_millis(100)).await;
                Ok::<(), AegisError>(())
            })
            .await
            .unwrap();

        scheduler.shutdown().await.unwrap();
        let result = scheduler
            .submit(async { Ok::<(), AegisError>(()) })
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_task_registry() {
        let registry = TaskRegistry::new();
        let flag = Arc::new(AtomicBool::new(false));
        let f = flag.clone();

        registry.register("test", move || {
            let inner = f.clone();
            async move {
                inner.store(true, Ordering::SeqCst);
                Ok(())
            }
        });

        registry.run("test").await.unwrap();
        assert!(flag.load(Ordering::SeqCst));

        registry.unregister("test").unwrap();
        let result = registry.run("test").await;
        assert!(result.is_err());
    }

    struct SimpleChunkStorage {
        chunks: std::sync::Mutex<std::collections::HashMap<ChunkId, Chunk>>,
    }

    impl SimpleChunkStorage {
        fn new() -> Self {
            Self {
                chunks: std::sync::Mutex::new(std::collections::HashMap::new()),
            }
        }
    }

    impl ChunkStorage for SimpleChunkStorage {
        fn store_chunk(&self, chunk: Chunk) -> BoxFuture<'_, AegisResult<ChunkId>> {
            let id = chunk.id;
            self.chunks.lock().unwrap().insert(id, chunk);
            Box::pin(async move { Ok(id) })
        }
        fn read_chunk(&self, id: &ChunkId) -> BoxFuture<'_, AegisResult<Chunk>> {
            let id = *id;
            let chunks = self.chunks.lock().unwrap().get(&id).cloned();
            Box::pin(async move {
                chunks.ok_or_else(|| AegisError::ChunkNotFound(id.to_string()))
            })
        }
        fn delete_chunk(&self, id: &ChunkId) -> BoxFuture<'_, AegisResult<()>> {
            let id = *id;
            self.chunks.lock().unwrap().remove(&id);
            Box::pin(async move { Ok(()) })
        }
        fn has_chunk(&self, id: &ChunkId) -> BoxFuture<'_, AegisResult<bool>> {
            let id = *id;
            let chunks = self.chunks.lock().unwrap();
            let exists = chunks.contains_key(&id);
            Box::pin(async move { Ok(exists) })
        }
        fn list_chunks(&self) -> BoxFuture<'_, AegisResult<Vec<ChunkId>>> {
            let ids: Vec<ChunkId> = self.chunks.lock().unwrap().keys().copied().collect();
            Box::pin(async move { Ok(ids) })
        }
        fn total_size(&self) -> BoxFuture<'_, AegisResult<u64>> {
            let total: u64 = self.chunks.lock().unwrap().values().map(|c| c.size).sum();
            Box::pin(async move { Ok(total) })
        }
        fn chunk_count(&self) -> BoxFuture<'_, AegisResult<u64>> {
            let count = self.chunks.lock().unwrap().len() as u64;
            Box::pin(async move { Ok(count) })
        }
    }

    #[tokio::test]
    async fn test_gc_task() {
        let storage = Arc::new(SimpleChunkStorage::new());
        let data1 = bytes::Bytes::from("orphaned");
        let data2 = bytes::Bytes::from("referenced");
        let id1 = storage
            .store_chunk(crate::core::types::Chunk::new(
                crate::core::id::ChunkId::from_data(&data1),
                data1,
            ))
            .await
            .unwrap();
        let id2 = storage
            .store_chunk(crate::core::types::Chunk::new(
                crate::core::id::ChunkId::from_data(&data2),
                data2,
            ))
            .await
            .unwrap();

        let gc = GcTask::new(storage.clone());
        let removed = gc.collect_garbage(&[id2]).await.unwrap();
        assert_eq!(removed, 1);
        assert!(!storage.has_chunk(&id1).await.unwrap());
        assert!(storage.has_chunk(&id2).await.unwrap());
    }
}
