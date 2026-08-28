use async_trait::async_trait;
use regex::Regex;
use reqwest::header::{HeaderMap, HeaderValue, COOKIE, REFERER, USER_AGENT};
use serde_json::Value;
use std::sync::RwLock;
use tokio::time::sleep;

use crate::error::AppError;
use crate::models::{CollectionInfo, ExternalItem};
use crate::source::SourceAdapter;

const USER_AGENT_STR: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

pub struct ZhihuClient {
    client: reqwest::Client,
    cookie: RwLock<Option<String>>,
}

impl ZhihuClient {
    pub fn new() -> Result<Self, AppError> {
        let client = reqwest::Client::builder()
            .cookie_store(true)
            .build()
            .map_err(|e| AppError::Http(e))?;
        Ok(Self {
            client,
            cookie: RwLock::new(None),
        })
    }

    pub fn set_cookie(&self, cookie: Option<String>) {
        if let Ok(mut guard) = self.cookie.write() {
            *guard = cookie;
        }
    }

    pub fn get_cookie(&self) -> Option<String> {
        self.cookie.read().ok().and_then(|g| g.clone())
    }

    fn build_headers(cookie: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_STR));
        headers.insert(REFERER, HeaderValue::from_static("https://www.zhihu.com/"));
        if let Some(cookie_str) = cookie {
            if let Ok(val) = HeaderValue::from_str(cookie_str) {
                headers.insert(COOKIE, val);
            }
        }
        headers
    }

    /// Parse a zhihu collection URL like https://www.zhihu.com/collection/123456
    pub fn parse_collection_id(input: &str) -> Result<String, AppError> {
        let re = Regex::new(r"zhihu\.com/collection/(\d+)").map_err(|e| AppError::Other(e.to_string()))?;
        if let Some(caps) = re.captures(input) {
            return Ok(caps[1].to_string());
        }
        // Try as a raw numeric ID
        let trimmed = input.trim();
        if trimmed.chars().all(|c| c.is_ascii_digit()) {
            return Ok(trimmed.to_string());
        }
        Err(AppError::InvalidInput("无法解析知乎收藏夹链接，请提供 https://www.zhihu.com/collection/数字ID 格式的链接".into()))
    }

    async fn fetch_json(&self, url: &str) -> Result<Value, AppError> {
        let cookie = self.get_cookie();
        let headers = Self::build_headers(cookie.as_deref());
        let resp = self
            .client
            .get(url)
            .headers(headers)
            .send()
            .await
            .map_err(|e| AppError::Http(e))?;
        if !resp.status().is_success() {
            return Err(AppError::Other(format!(
                "知乎 API 请求失败: HTTP {}",
                resp.status()
            )));
        }
        let text = resp.text().await.map_err(|e| AppError::Http(e))?;
        let json: Value = serde_json::from_str(&text).map_err(|e| AppError::Json(e))?;
        Ok(json)
    }

    /// Fetch user profile to get url_token
    pub async fn get_url_token(&self) -> Result<String, AppError> {
        let json = self.fetch_json("https://www.zhihu.com/api/v4/me").await?;
        let token = json["url_token"]
            .as_str()
            .ok_or_else(|| AppError::AuthRequired)?;
        Ok(token.to_string())
    }

    /// Build ExternalItem from a collection item JSON
    fn item_from_json(item: &Value) -> Option<ExternalItem> {
        let content = &item["content"];
        let item_type = content["type"].as_str().unwrap_or("");

        let (title, url) = match item_type {
            "answer" => {
                let question = &content["question"];
                let q_title = question["title"].as_str().unwrap_or("");
                let q_id = question["id"].as_i64().map(|id| id.to_string()).unwrap_or_default();
                let answer_id = content["id"].as_i64().map(|id| id.to_string()).unwrap_or_default();
                let u = format!("https://www.zhihu.com/question/{}/answer/{}", q_id, answer_id);
                (q_title.to_string(), u)
            }
            "article" => {
                let a_title = content["title"].as_str().unwrap_or("");
                let a_id = content["id"].as_i64().map(|id| id.to_string()).unwrap_or_default();
                let u = format!("https://zhuanlan.zhihu.com/p/{}", a_id);
                (a_title.to_string(), u)
            }
            "pin" => {
                let pin_title = content["excerpt_title"].as_str().unwrap_or("想法");
                let pin_id = content["id"].as_i64().map(|id| id.to_string()).unwrap_or_default();
                let u = format!("https://www.zhihu.com/pin/{}", pin_id);
                (pin_title.to_string(), u)
            }
            _ => {
                let t = content["title"].as_str().unwrap_or("");
                if t.is_empty() {
                    return None;
                }
                let u = content["url"].as_str().unwrap_or("").to_string();
                (t.to_string(), u)
            }
        };

        if title.is_empty() || url.is_empty() {
            return None;
        }

        let author_name = content["author"]["name"].as_str().map(|s| s.to_string());
        let author_id = content["author"]["id"].as_str().map(|s| s.to_string());
        let created_time = content["created_time"].as_i64().or_else(|| {
            content["updated_time"].as_i64()
        });

        let collected_time = item["collected_time"].as_i64().or(created_time);

        let external_id = url.clone();

        Some(ExternalItem {
            source: "zhihu".into(),
            external_id,
            source_url: url,
            title,
            description: String::new(),
            cover_url: None,
            cover_local_path: None,
            author_name,
            author_id,
            partition_name: None,
            published_at: created_time,
            duration: None,
            favorite_time: collected_time,
            extra: serde_json::json!({
                "item_type": item_type,
            }),
        })
    }
}

#[async_trait]
impl SourceAdapter for ZhihuClient {
    async fn list_collections(&self) -> Result<Vec<CollectionInfo>, AppError> {
        let url_token = self.get_url_token().await?;
        let mut collections = Vec::new();
        let mut offset = 0i64;

        loop {
            let url = format!(
                "https://www.zhihu.com/api/v4/people/{}/collections?limit=20&offset={}",
                url_token, offset
            );
            let json = self.fetch_json(&url).await?;
            let data = json["data"].as_array();

            let paging = &json["paging"];
            let is_end = paging["is_end"].as_bool().unwrap_or(true);

            if let Some(items) = data {
                for item in items {
                    let id = item["id"].as_i64().map(|id| id.to_string()).unwrap_or_default();
                    let title = item["title"].as_str().unwrap_or("未命名收藏夹").to_string();
                    let count = item["item_count"].as_i64().unwrap_or(0);
                    collections.push(CollectionInfo {
                        source: "zhihu".into(),
                        id,
                        title,
                        owner: None,
                        count,
                        url: None,
                    });
                }
            }

            if is_end {
                break;
            }
            offset += 20;
        }

        Ok(collections)
    }

    async fn resolve_collection(&self, input: &str) -> Result<CollectionInfo, AppError> {
        let collection_id = Self::parse_collection_id(input)?;
        let url = format!("https://www.zhihu.com/api/v4/collections/{}", collection_id);
        let json = self.fetch_json(&url).await?;

        let collection = &json["collection"];
        let title = collection["title"].as_str().unwrap_or("未命名收藏夹").to_string();
        let count = collection["item_count"].as_i64().unwrap_or(0);

        Ok(CollectionInfo {
            source: "zhihu".into(),
            id: collection_id,
            title,
            owner: None,
            count,
            url: Some(input.to_string()),
        })
    }

    async fn fetch_collection(&self, collection: &CollectionInfo) -> Result<Vec<ExternalItem>, AppError> {
        let mut items = Vec::new();
        let mut offset = 0i64;

        loop {
            let url = format!(
                "https://www.zhihu.com/api/v4/collections/{}/items?limit=20&offset={}",
                collection.id, offset
            );
            let json = self.fetch_json(&url).await?;
            let data = json["data"].as_array();

            let paging = &json["paging"];
            let is_end = paging["is_end"].as_bool().unwrap_or(true);

            if let Some(arr) = data {
                for item in arr {
                    if let Some(external_item) = Self::item_from_json(item) {
                        items.push(external_item);
                    }
                }
            }

            if is_end {
                break;
            }
            offset += 20;

            // Rate limit: 200ms between pages
            sleep(std::time::Duration::from_millis(200)).await;
        }

        Ok(items)
    }

    async fn enrich_items(&self, items: &[ExternalItem]) -> Result<Vec<ExternalItem>, AppError> {
        // Zhihu items already have full info from the collection API
        Ok(items.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_collection_url() {
        let id = ZhihuClient::parse_collection_id("https://www.zhihu.com/collection/19677733").unwrap();
        assert_eq!(id, "19677733");
    }

    #[test]
    fn test_parse_collection_url_with_query() {
        let id = ZhihuClient::parse_collection_id("https://www.zhihu.com/collection/19677733?page=1").unwrap();
        assert_eq!(id, "19677733");
    }

    #[test]
    fn test_parse_raw_id() {
        let id = ZhihuClient::parse_collection_id("19677733").unwrap();
        assert_eq!(id, "19677733");
    }

    #[test]
    fn test_invalid_url() {
        let result = ZhihuClient::parse_collection_id("https://www.zhihu.com/question/123");
        assert!(result.is_err());
    }
}