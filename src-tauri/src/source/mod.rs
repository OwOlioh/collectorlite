use async_trait::async_trait;

use crate::error::AppError;
use crate::models::{CollectionInfo, ExternalItem};

#[async_trait]
pub trait SourceAdapter: Send + Sync {
    async fn list_collections(&self) -> Result<Vec<CollectionInfo>, AppError>;
    async fn resolve_collection(&self, input: &str) -> Result<CollectionInfo, AppError>;
    async fn fetch_collection(
        &self,
        collection: &CollectionInfo,
    ) -> Result<Vec<ExternalItem>, AppError>;
    async fn enrich_items(&self, items: &[ExternalItem]) -> Result<Vec<ExternalItem>, AppError>;
}

pub mod bilibili;
