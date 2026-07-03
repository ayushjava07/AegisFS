use bytes::Bytes;
use futures::future::BoxFuture;

use crate::config::AegisConfig;
use crate::core::error::AegisResult;
use crate::core::id::{ArchiveId, SnapshotId};
use crate::core::types::SyncResult;

pub struct AegisFs {
    #[allow(dead_code)]
    config: AegisConfig,
}

impl AegisFs {
    pub fn new(config: AegisConfig) -> Self {
        Self { config }
    }

    pub fn create_archive<'a>(&'a self, _name: &'a str) -> BoxFuture<'a, AegisResult<ArchiveId>> {
        Box::pin(async move {
            todo!("create_archive implementation")
        })
    }

    pub fn open_archive<'a>(&'a self, _id: &'a ArchiveId) -> BoxFuture<'a, AegisResult<()>> {
        Box::pin(async move {
            todo!("open_archive implementation")
        })
    }

    pub fn store<'a>(&'a self, _path: &'a str, _data: Bytes) -> BoxFuture<'a, AegisResult<()>> {
        Box::pin(async move {
            todo!("store implementation")
        })
    }

    pub fn retrieve<'a>(&'a self, _path: &'a str) -> BoxFuture<'a, AegisResult<Option<Bytes>>> {
        Box::pin(async move {
            todo!("retrieve implementation")
        })
    }

    pub fn delete<'a>(&'a self, _path: &'a str) -> BoxFuture<'a, AegisResult<()>> {
        Box::pin(async move {
            todo!("delete implementation")
        })
    }

    pub fn create_snapshot<'a>(&'a self, _label: &'a str) -> BoxFuture<'a, AegisResult<SnapshotId>> {
        Box::pin(async move {
            todo!("create_snapshot implementation")
        })
    }

    pub fn sync_to_remote<'a>(&'a self, _target: &'a str) -> BoxFuture<'a, AegisResult<SyncResult>> {
        Box::pin(async move {
            todo!("sync_to_remote implementation")
        })
    }
}
