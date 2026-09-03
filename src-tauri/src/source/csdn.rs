use async_trait::async_trait;
use regex::Regex;
use reqwest::header::{HeaderMap, HeaderValue, REFERER, USER_AGENT};
use serde_json::Value;
use tokio::time::sleep;

use crate::error::AppError;
use crate::models::{CollectionInfo, ExternalItem};
use crate::source::SourceAdapter;

const USER_AGENT_STR: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

pub struct CsdnClient {
    client: reqwest::Client,
}

impl CsdnClient {
    pub fn new() -> Result<Self, AppError> {
        let client = reqwest::Client::builder()
            .build()
            .map_err(|e| AppError::Http(e))?;
        Ok(Self { client })
    }

    fn build_headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_STR));
        headers.insert(REFERER, HeaderValue::from_static("https://blog.csdn.net/"));
        headers.insert(
            "accept",
            HeaderValue::from_static("application/json, text/plain, */*"),
        );
        headers.insert(
            "accept-language",
            HeaderValue::from_static("zh-CN,zh;q=0.9,en;q=0.8"),
        );
        headers
    }

    /// Download a cover image and return the bytes + file extension
    pub async fn download_cover(&self, url: &str) -> Result<(Vec<u8>, String), AppError> {
        let response = self
            .client
            .get(url)
            .header(USER_AGENT, HeaderValue::from_static(USER_AGENT_STR))
            .header(REFERER, HeaderValue::from_static("https://blog.csdn.net/"))
            .send()
            .await
            .map_err(|e| AppError::Http(e))?;
        let status = response.status();
        if !status.is_success() {
            return Err(AppError::Other(format!("下载封面失败（HTTP {status}）")));
        }
        let extension = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .map(|value| {
                if value.contains("png") {
                    "png"
                } else if value.contains("webp") {
                    "webp"
                } else if value.contains("gif") {
                    "gif"
                } else {
                    "jpg"
                }
            })
            .unwrap_or("jpg")
            .to_string();
        let bytes = response
            .bytes()
            .await
            .map_err(|e| AppError::Http(e))?
            .to_vec();
        Ok((bytes, extension))
    }

    async fn fetch_json(&self, url: &str) -> Result<Value, AppError> {
        let headers = Self::build_headers();
        let resp = self
            .client
            .get(url)
            .headers(headers)
            .send()
            .await
            .map_err(|e| AppError::Http(e))?;
        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            eprintln!("[csdn] HTTP {} for {}", status, url);
            return Err(AppError::Other(format!(
                "CSDN API 请求失败: HTTP {}",
                status
            )));
        }
        let text = resp.text().await.map_err(|e| AppError::Http(e))?;
        let json: Value = serde_json::from_str(&text).map_err(|e| AppError::Json(e))?;
        Ok(json)
    }

    /// Parse a CSDN collection URL like https://blog.csdn.net/{username}/favorites?folderId=123
    pub fn parse_collection_url(input: &str) -> Result<(String, String), AppError> {
        // Try to match: blog.csdn.net/{username}/favorites?...folderId=123
        let re = Regex::new(r"blog\.csdn\.net/([^/?]+)/favorites\?.*folderId=(\d+)")
            .map_err(|e| AppError::Other(e.to_string()))?;
        if let Some(caps) = re.captures(input) {
            return Ok((caps[1].to_string(), caps[2].to_string()));
        }

        Err(AppError::InvalidInput(
            "无法解析 CSDN 收藏夹链接，请提供 https://blog.csdn.net/用户名/favorites?folderId=数字ID 格式的链接".into()
        ))
    }

    /// Extract article ID from a CSDN article URL
    fn extract_article_id(url: &str) -> String {
        let url = url.trim();
        // Pattern: blog.csdn.net/{author}/article/details/{articleId}
        let re = Regex::new(r"blog\.csdn\.net/[^/]+/article/details/(\d+)").unwrap();
        if let Some(caps) = re.captures(url) {
            return caps[1].to_string();
        }
        // Fallback: use the full URL as ID (but this is not ideal)
        url.to_string()
    }

    /// Build ExternalItem from a CSDN favorite item JSON
    fn item_from_json(item: &Value, username: &str) -> Option<ExternalItem> {
        let title = item["title"].as_str().unwrap_or("").to_string();
        let url = item["url"].as_str().unwrap_or("").to_string();

        if title.is_empty() || url.is_empty() {
            return None;
        }

        let article_id = Self::extract_article_id(&url);
        let author_name = item["nickname"].as_str().map(|s| s.to_string());
        let author_id = item["author"].as_str().map(|s| s.to_string());
        let dateline = item["dateline"].as_i64();

        Some(ExternalItem {
            source: "csdn".into(),
            external_id: article_id,
            source_url: url,
            title,
            description: String::new(),
            cover_url: None,
            cover_local_path: None,
            author_name,
            author_id,
            partition_name: None,
            published_at: dateline,
            duration: None,
            favorite_time: dateline,
            extra: serde_json::json!({
                "username": username,
            }),
        })
    }

    /// 抓 CSDN 单篇文章的丰富元数据（标题 / 封面 / 摘要 / 作者）。
    /// 复用已有的页面抓取基建（抓 `og:` meta），不引入新依赖。
    /// 仅当链接确实是 `.../article/details/{id}` 时才生效；其余 CSDN 链接返回 `None`，
    /// 由调用方回退到「按收藏夹路由」或通用存档。
    pub async fn fetch_article(&self, url: &str) -> Option<ExternalItem> {
        let article_re = Regex::new(r"blog\.csdn\.net/[^/]+/article/details/(\d+)").ok()?;
        let caps = article_re.captures(url)?;
        let article_id = caps[1].to_string();
        let username = Regex::new(r"blog\.csdn\.net/([^/]+)/article/details/")
            .ok()
            .and_then(|re| re.captures(url))
            .map(|c| c[1].to_string())
            .unwrap_or_default();

        let headers = Self::build_headers();
        let resp = self
            .client
            .get(url)
            .headers(headers)
            .send()
            .await
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let text = resp.text().await.ok()?;
        let title = meta_property(&text, "og:title").unwrap_or_else(|| url.to_string());
        let cover_url = meta_property(&text, "og:image");
        let description = meta_property(&text, "og:description").unwrap_or_default();
        let author_name = meta_name(&text, "author")
            .or_else(|| meta_property(&text, "og:author-name"));

        Some(ExternalItem {
            source: "csdn".into(),
            external_id: article_id,
            source_url: url.to_string(),
            title,
            description,
            cover_url,
            cover_local_path: None,
            author_name,
            author_id: None,
            partition_name: Some("CSDN".into()),
            published_at: None,
            duration: None,
            favorite_time: Some(crate::db::now_seconds()),
            extra: serde_json::json!({ "username": username }),
        })
    }
}

/// 从页面 HTML 里抽 `<meta property="X" content="Y">` 的值。
fn meta_property(html: &str, key: &str) -> Option<String> {
    let pattern = format!(
        r#"<meta[^>]*property="{}"[^>]*content="([^"]*)""#,
        regex::escape(key)
    );
    Regex::new(&pattern)
        .ok()
        .and_then(|re| re.captures(html))
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
}

/// 从页面 HTML 里抽 `<meta name="X" content="Y">` 的值。
fn meta_name(html: &str, key: &str) -> Option<String> {
    let pattern = format!(r#"<meta[^>]*name="{}"[^>]*content="([^"]*)""#, regex::escape(key));
    Regex::new(&pattern)
        .ok()
        .and_then(|re| re.captures(html))
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
}

#[async_trait]
impl SourceAdapter for CsdnClient {
    async fn list_collections(&self) -> Result<Vec<CollectionInfo>, AppError> {
        // CSDN is public, but list_collections requires a username.
        // This is called with a username from the command layer.
        // Since the trait doesn't take a username parameter, we return an error
        // and use the dedicated list_csdn_collections method instead.
        Err(AppError::InvalidInput(
            "CSDN 需要提供用户名，请使用 list_csdn_collections 命令".into(),
        ))
    }

    async fn resolve_collection(&self, input: &str) -> Result<CollectionInfo, AppError> {
        // input is a URL like https://blog.csdn.net/{username}/favorites?folderId=123
        let (username, folder_id) = Self::parse_collection_url(input)?;

        // Fetch the collection list to get the title and count
        let url = format!(
            "https://blog.csdn.net/community/home-api/v1/get-favorites-created-list?page=1&size=20&noMore=false&blogUsername={}",
            username
        );
        let json = self.fetch_json(&url).await?;
        let code = json["code"].as_i64().unwrap_or(-1);
        if code != 200 {
            return Err(AppError::Other("CSDN 收藏夹列表获取失败".into()));
        }

        let data = &json["data"];
        if data.is_null() {
            return Err(AppError::NotFound(format!(
                "未找到 CSDN 用户 '{}'，请检查用户名是否正确",
                username
            )));
        }

        let list = data["list"].as_array();
        let fid = folder_id.parse::<i64>().unwrap_or(0);

        if let Some(items) = list {
            for item in items {
                let id = item["id"].as_i64().unwrap_or(0);
                if id == fid {
                    let title = item["name"].as_str().unwrap_or("未命名收藏夹").to_string();
                    let count = item["favoriteNum"].as_i64().unwrap_or(0);
                    return Ok(CollectionInfo {
                        source: "csdn".into(),
                        id: folder_id.clone(),
                        title,
                        owner: Some(username.clone()),
                        count,
                        url: Some(input.to_string()),
                    });
                }
            }
        }

        Err(AppError::NotFound(
            "未找到该收藏夹，请检查链接是否正确".into(),
        ))
    }

    async fn fetch_collection(
        &self,
        collection: &CollectionInfo,
    ) -> Result<Vec<ExternalItem>, AppError> {
        let username = collection.owner.as_deref().unwrap_or("");
        if username.is_empty() {
            return Err(AppError::InvalidInput("CSDN 用户名不能为空".into()));
        }

        let mut items = Vec::new();
        let mut page = 1i64;

        loop {
            let url = format!(
                "https://blog.csdn.net/community/home-api/v1/get-favorites-item-list?blogUsername={}&folderId={}&page={}&pageSize=200",
                username, collection.id, page
            );
            let json = self.fetch_json(&url).await?;
            let code = json["code"].as_i64().unwrap_or(-1);
            if code != 200 {
                break;
            }

            let data = &json["data"];
            // data is null when the collection is empty or doesn't exist
            if data.is_null() {
                break;
            }
            let total = data["total"].as_i64().unwrap_or(0);
            let list = data["list"].as_array();

            if let Some(arr) = list {
                for item in arr {
                    if let Some(external_item) = Self::item_from_json(item, username) {
                        items.push(external_item);
                    }
                }
            }

            // Check if we've fetched all items
            if items.len() as i64 >= total || list.map(|l| l.len()).unwrap_or(0) == 0 {
                break;
            }
            page += 1;

            // Rate limit: 200ms between pages
            sleep(std::time::Duration::from_millis(200)).await;
        }

        Ok(items)
    }

    async fn enrich_items(&self, items: &[ExternalItem]) -> Result<Vec<ExternalItem>, AppError> {
        let mut enriched = Vec::with_capacity(items.len());
        for item in items {
            let mut next = item.clone();
            if let Some(cover_url) = self.fetch_article_cover(&item.source_url).await {
                next.cover_url = Some(cover_url);
            }
            enriched.push(next);
            sleep(std::time::Duration::from_millis(100)).await;
        }
        Ok(enriched)
    }
}

// ── Dedicated methods for CSDN (since the trait doesn't take username) ──

impl CsdnClient {
    /// Fetch the cover image URL from a CSDN article page via og:image meta tag
    async fn fetch_article_cover(&self, article_url: &str) -> Option<String> {
        if article_url.is_empty() {
            return None;
        }
        let headers = Self::build_headers();
        let resp = self
            .client
            .get(article_url)
            .headers(headers)
            .send()
            .await
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let text = resp.text().await.ok()?;
        let re =
            regex::Regex::new(r#"<meta[^>]*property="og:image"[^>]*content="([^"]+)""#).ok()?;
        re.captures(&text)
            .and_then(|caps| caps.get(1))
            .map(|m| m.as_str().to_string())
    }

    /// List collections for a given CSDN username
    pub async fn list_collections_for_user(
        &self,
        username: &str,
    ) -> Result<Vec<CollectionInfo>, AppError> {
        let mut collections = Vec::new();
        let mut page = 1i64;

        loop {
            let url = format!(
                "https://blog.csdn.net/community/home-api/v1/get-favorites-created-list?page={}&size=20&noMore=false&blogUsername={}",
                page, username
            );
            let json = self.fetch_json(&url).await?;
            let code = json["code"].as_i64().unwrap_or(-1);
            if code != 200 {
                break;
            }

            let data = &json["data"];
            // data is null when the user doesn't exist
            if data.is_null() {
                return Err(AppError::NotFound(format!(
                    "未找到 CSDN 用户 '{}'，请检查用户名是否正确",
                    username
                )));
            }
            let total = data["total"].as_i64().unwrap_or(0);
            let list = data["list"].as_array();

            if let Some(items) = list {
                for item in items {
                    let id = item["id"]
                        .as_i64()
                        .map(|id| id.to_string())
                        .unwrap_or_default();
                    let title = item["name"].as_str().unwrap_or("未命名收藏夹").to_string();
                    let count = item["favoriteNum"].as_i64().unwrap_or(0);
                    collections.push(CollectionInfo {
                        source: "csdn".into(),
                        id,
                        title,
                        owner: Some(username.to_string()),
                        count,
                        url: None,
                    });
                }
            }

            if collections.len() as i64 >= total || list.map(|l| l.len()).unwrap_or(0) == 0 {
                break;
            }
            page += 1;
        }

        Ok(collections)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_collection_url() {
        let (username, id) = CsdnClient::parse_collection_url(
            "https://blog.csdn.net/testuser/favorites?folderId=12345",
        )
        .unwrap();
        assert_eq!(username, "testuser");
        assert_eq!(id, "12345");
    }

    #[test]
    fn test_parse_collection_url_with_query() {
        let (username, id) = CsdnClient::parse_collection_url(
            "https://blog.csdn.net/testuser/favorites?spm=1001&folderId=67890",
        )
        .unwrap();
        assert_eq!(username, "testuser");
        assert_eq!(id, "67890");
    }

    #[test]
    fn test_extract_article_id() {
        let id = CsdnClient::extract_article_id(
            "https://blog.csdn.net/2401_83830408/article/details/164078212",
        );
        assert_eq!(id, "164078212");
    }

    #[test]
    fn test_invalid_url() {
        let result = CsdnClient::parse_collection_url("https://www.csdn.net/");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_fetch_article_cover() {
        let client = CsdnClient::new().unwrap();
        let cover = client
            .fetch_article_cover("https://blog.csdn.net/2401_83830408/article/details/164078212")
            .await;
        eprintln!("cover result: {:?}", cover);
        assert!(cover.is_some(), "Cover should be found for this article");
        assert!(
            cover.unwrap().contains("i-blog.csdnimg.cn"),
            "Cover URL should be from CSDN image CDN"
        );
    }
}
