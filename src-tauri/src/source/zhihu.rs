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
        headers.insert(
            "accept",
            HeaderValue::from_static("application/json, text/plain, */*"),
        );
        headers.insert(
            "accept-language",
            HeaderValue::from_static("zh-CN,zh;q=0.9,en;q=0.8"),
        );
        headers.insert("x-requested-with", HeaderValue::from_static("fetch"));
        if let Some(cookie_str) = cookie {
            if let Ok(val) = HeaderValue::from_str(cookie_str) {
                headers.insert(COOKIE, val);
            }
        }
        headers
    }

    /// Parse a zhihu collection URL like https://www.zhihu.com/collection/123456
    pub fn parse_collection_id(input: &str) -> Result<String, AppError> {
        let re = Regex::new(r"zhihu\.com/collection/(\d+)")
            .map_err(|e| AppError::Other(e.to_string()))?;
        if let Some(caps) = re.captures(input) {
            return Ok(caps[1].to_string());
        }
        // Try as a raw numeric ID
        let trimmed = input.trim();
        if trimmed.chars().all(|c| c.is_ascii_digit()) {
            return Ok(trimmed.to_string());
        }
        Err(AppError::InvalidInput(
            "无法解析知乎收藏夹链接，请提供 https://www.zhihu.com/collection/数字ID 格式的链接"
                .into(),
        ))
    }

    async fn fetch_json(&self, url: &str) -> Result<Value, AppError> {
        let cookie = self.get_cookie();
        eprintln!(
            "[zhihu] fetch_json url={}, has_cookie={}",
            url,
            cookie.is_some()
        );
        let headers = Self::build_headers(cookie.as_deref());
        let resp = self
            .client
            .get(url)
            .headers(headers)
            .send()
            .await
            .map_err(|e| AppError::Http(e))?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            if status == 401 {
                return Err(AppError::AuthRequired);
            }
            eprintln!("[zhihu] HTTP {} for {}", status, url);
            return Err(AppError::Other(format!(
                "知乎 API 请求失败: HTTP {}",
                status
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
                let q_id = json_value_to_string(&question["id"]);
                let answer_id = json_value_to_string(&content["id"]);
                let u = format!(
                    "https://www.zhihu.com/question/{}/answer/{}",
                    q_id, answer_id
                );
                (q_title.to_string(), u)
            }
            "article" => {
                let a_title = content["title"].as_str().unwrap_or("");
                let a_id = json_value_to_string(&content["id"]);
                let u = format!("https://zhuanlan.zhihu.com/p/{}", a_id);
                (a_title.to_string(), u)
            }
            "pin" => {
                let pin_title = content["excerpt_title"].as_str().unwrap_or("想法");
                let pin_id = json_value_to_string(&content["id"]);
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
        let created_time = content["created_time"]
            .as_i64()
            .or_else(|| content["updated_time"].as_i64());

        let collected_time = item["collected_time"].as_i64().or(created_time);

        let item_id = json_value_to_string(&content["id"]);
        if item_id.is_empty() {
            eprintln!("[zhihu] WARNING: empty content id, using url as fallback");
        }
        let item_id = if item_id.is_empty() {
            url.clone()
        } else {
            item_id
        };

        let cover_url = content["image_url"]
            .as_str()
            .or_else(|| content["thumbnail"].as_str())
            .or_else(|| content["cover"].as_str())
            .or_else(|| content["title_image"].as_str())
            .map(|s| s.to_string());

        Some(ExternalItem {
            source: "zhihu".into(),
            external_id: item_id,
            source_url: url,
            title,
            description: String::new(),
            cover_url,
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

    /// 抓知乎单条内容（回答 / 文章 / 想法）的丰富元数据。
    /// 需要登录 cookie；未登录会 401 → 返回 `None`，由调用方回退到通用存档。
    pub async fn fetch_single(&self, url: &str) -> Option<ExternalItem> {
        let api = single_api_url(url)?;
        let json = self.fetch_json(&api).await.ok()?;
        // 单条内容的 JSON 结构与收藏夹 item 的 `content` 子对象一致，
        // 这里包一层 `{ "content": json }` 直接复用 `item_from_json`，避免重复造轮子。
        let wrapped = serde_json::json!({ "content": json });
        Self::item_from_json(&wrapped)
    }
}

/// Convert a JSON value to string, handling both string and number types
fn json_value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// 把知乎单条内容链接映射成对应的 v4 API 端点。识别不了则返回 `None`。
/// - 回答：`www.zhihu.com/question/{qid}/answer/{aid}` → `/api/v4/answers/{aid}`
/// - 文章：`zhuanlan.zhihu.com/p/{id}` 或 `www.zhihu.com/p/{id}` → `/api/v4/articles/{id}`
/// - 想法：`www.zhihu.com/pin/{id}` → `/api/v4/pins/{id}`
fn single_api_url(url: &str) -> Option<String> {
    let answer_re = Regex::new(r"zhihu\.com/question/\d+/answer/(\d+)").ok()?;
    if let Some(caps) = answer_re.captures(url) {
        return Some(format!("https://www.zhihu.com/api/v4/answers/{}", &caps[1]));
    }
    let article_re = Regex::new(r"(zhuanlan\.zhihu\.com|www\.zhihu\.com)/p/(\d+)").ok()?;
    if let Some(caps) = article_re.captures(url) {
        return Some(format!("https://www.zhihu.com/api/v4/articles/{}", &caps[2]));
    }
    let pin_re = Regex::new(r"zhihu\.com/pin/(\d+)").ok()?;
    if let Some(caps) = pin_re.captures(url) {
        return Some(format!("https://www.zhihu.com/api/v4/pins/{}", &caps[1]));
    }
    None
}

/// 从知乎单条链接 + 扩展读到的 og 元数据构造 zhihu item（API 抓取失败时的兜底）。
///
/// 知乎 API v4 需要 `x-zse-96` 反爬签名，app 侧即使有 cookie 也常常 403；
/// 但扩展运行在用户已登录的浏览器里，能稳定读到页面的 og:title / og:image。
/// 这里用「URL 解析出的 id + 扩展传来的干净标题 / 封面」构造标准 zhihu 条目，
/// external_id 与收藏夹导入（`item_from_json` 用 content.id）同键，去重一致。
pub fn item_from_url_and_meta(url: &str, title: &str, og_image: &str) -> Option<ExternalItem> {
    let (external_id, item_type) = if let Some(caps) =
        Regex::new(r"zhihu\.com/question/\d+/answer/(\d+)").ok()?.captures(url)
    {
        (caps[1].to_string(), "answer")
    } else if let Some(caps) = Regex::new(r"zhihu\.com/p/(\d+)").ok()?.captures(url) {
        (caps[1].to_string(), "article")
    } else if let Some(caps) = Regex::new(r"zhihu\.com/pin/(\d+)").ok()?.captures(url) {
        (caps[1].to_string(), "pin")
    } else {
        return None;
    };

    let clean_title = title.trim();
    let title = if clean_title.is_empty() {
        if item_type == "pin" {
            "想法".to_string()
        } else {
            url.to_string()
        }
    } else {
        clean_title.to_string()
    };
    let cover = if og_image.trim().is_empty() {
        None
    } else {
        Some(og_image.trim().to_string())
    };

    Some(ExternalItem {
        source: "zhihu".into(),
        external_id,
        source_url: url.to_string(),
        title,
        description: String::new(),
        cover_url: cover,
        cover_local_path: None,
        author_name: None,
        author_id: None,
        partition_name: Some("知乎".into()),
        published_at: None,
        duration: None,
        favorite_time: Some(crate::db::now_seconds()),
        extra: serde_json::json!({ "item_type": item_type }),
    })
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
                    let id = item["id"]
                        .as_i64()
                        .map(|id| id.to_string())
                        .unwrap_or_default();
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
        let title = collection["title"]
            .as_str()
            .unwrap_or("未命名收藏夹")
            .to_string();
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

    async fn fetch_collection(
        &self,
        collection: &CollectionInfo,
    ) -> Result<Vec<ExternalItem>, AppError> {
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
        let id =
            ZhihuClient::parse_collection_id("https://www.zhihu.com/collection/19677733").unwrap();
        assert_eq!(id, "19677733");
    }

    #[test]
    fn test_parse_collection_url_with_query() {
        let id =
            ZhihuClient::parse_collection_id("https://www.zhihu.com/collection/19677733?page=1")
                .unwrap();
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

    #[test]
    fn test_single_api_url_maps_content_links() {
        assert_eq!(
            single_api_url("https://www.zhihu.com/question/123/answer/456?utm=x"),
            Some("https://www.zhihu.com/api/v4/answers/456".to_string())
        );
        assert_eq!(
            single_api_url("https://zhuanlan.zhihu.com/p/789"),
            Some("https://www.zhihu.com/api/v4/articles/789".to_string())
        );
        assert_eq!(
            single_api_url("https://www.zhihu.com/p/101112"),
            Some("https://www.zhihu.com/api/v4/articles/101112".to_string())
        );
        assert_eq!(
            single_api_url("https://www.zhihu.com/pin/131415"),
            Some("https://www.zhihu.com/api/v4/pins/131415".to_string())
        );
        // 非单条内容链接（收藏夹 / 用户主页 / 问题页）识别不了，回退通用存档。
        assert_eq!(single_api_url("https://www.zhihu.com/collection/19677733"), None);
        assert_eq!(single_api_url("https://www.zhihu.com/question/123"), None);
    }

    #[test]
    fn test_item_from_url_and_meta_article() {
        let item = item_from_url_and_meta(
            "https://zhuanlan.zhihu.com/p/642170180",
            "一篇知乎文章",
            "https://pic.zhimg.com/v2-abc.jpg",
        )
        .unwrap();
        assert_eq!(item.source, "zhihu");
        assert_eq!(item.external_id, "642170180");
        assert_eq!(item.title, "一篇知乎文章");
        assert_eq!(item.cover_url.as_deref(), Some("https://pic.zhimg.com/v2-abc.jpg"));
        assert_eq!(item.extra["item_type"], "article");
    }

    #[test]
    fn test_item_from_url_and_meta_answer() {
        let item = item_from_url_and_meta("https://www.zhihu.com/question/123/answer/456", "问题标题", "")
            .unwrap();
        assert_eq!(item.external_id, "456");
        assert_eq!(item.extra["item_type"], "answer");
        assert_eq!(item.cover_url, None);
    }

    #[test]
    fn test_item_from_url_and_meta_non_content() {
        assert!(item_from_url_and_meta("https://www.zhihu.com/hot", "热榜", "").is_none());
        assert!(item_from_url_and_meta("https://www.zhihu.com/question/123", "问题页", "").is_none());
    }
}
