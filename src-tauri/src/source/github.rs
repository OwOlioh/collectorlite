use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use reqwest::Proxy;
use serde_json::Value;
use tokio::time::sleep;

use crate::error::AppError;
use crate::models::{CollectionInfo, ExternalItem};
use crate::source::SourceAdapter;

const USER_AGENT_STR: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

/// 读取 Windows 注册表系统代理地址（仅 Windows）。无代理或读取失败返回 None。
#[cfg(windows)]
fn windows_registry_proxy_url() -> Option<String> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
    let hkcu = winreg::RegKey::predef(HKEY_CURRENT_USER);
    let settings = hkcu
        .open_subkey_with_flags(
            "Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings",
            KEY_READ,
        )
        .ok()?;
    let enabled: u32 = settings.get_value("ProxyEnable").ok()?;
    if enabled == 0 {
        return None;
    }
    let server: String = settings.get_value("ProxyServer").ok()?;
    let mut http: Option<String> = None;
    let mut https: Option<String> = None;
    for part in server.split(';') {
        if let Some((scheme, addr)) = part.split_once('=') {
            match scheme.to_lowercase().as_str() {
                "http" => http = Some(addr.to_string()),
                "https" => https = Some(addr.to_string()),
                _ => {}
            }
        } else if !part.is_empty() {
            http = Some(part.to_string());
            https = Some(part.to_string());
        }
    }
    let addr = http.or(https)?;
    if addr.starts_with("http://") || addr.starts_with("https://") {
        Some(addr)
    } else {
        Some(format!("http://{addr}"))
    }
}

#[cfg(not(windows))]
fn windows_registry_proxy_url() -> Option<String> {
    None
}

pub struct GithubClient {
    client: reqwest::Client,
}

impl GithubClient {
    pub fn new() -> Result<Self, AppError> {
        let mut builder = reqwest::Client::builder();
        // 与 B站客户端一致：优先环境变量代理，否则回退到 Windows 注册表代理，
        // 避免中国大陆直连 GitHub 受限；无代理配置时为空操作。
        let env_proxy = std::env::var_os("HTTPS_PROXY")
            .or_else(|| std::env::var_os("HTTP_PROXY"))
            .or_else(|| std::env::var_os("https_proxy"))
            .or_else(|| std::env::var_os("http_proxy"));
        let proxy = if let Some(value) = env_proxy {
            value.into_string().ok().filter(|u| !u.is_empty())
        } else {
            windows_registry_proxy_url()
        };
        if let Some(url) = proxy {
            if let Ok(p) = Proxy::all(&url) {
                builder = builder.proxy(p);
            }
        }
        let client = builder.build().map_err(|e| AppError::Http(e))?;
        Ok(Self { client })
    }

    fn build_headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_STR));
        headers.insert(
            "accept",
            HeaderValue::from_static("application/vnd.github+json"),
        );
        headers.insert(
            "X-GitHub-Api-Version",
            HeaderValue::from_static("2022-11-28"),
        );
        headers
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
            eprintln!("[github] HTTP {} for {}", status, url);
            if status == 404 {
                return Err(AppError::NotFound(format!(
                    "未找到 GitHub 用户，请检查用户名是否正确"
                )));
            }
            if status == 403 {
                return Err(AppError::Other(
                    "GitHub API 频率限制，请稍后再试或使用 Token 认证".into(),
                ));
            }
            return Err(AppError::Other(format!(
                "GitHub API 请求失败: HTTP {}",
                status
            )));
        }
        let text = resp.text().await.map_err(|e| AppError::Http(e))?;
        let json: Value = serde_json::from_str(&text).map_err(|e| AppError::Json(e))?;
        Ok(json)
    }

    /// Build ExternalItem from a GitHub starred repo JSON
    fn item_from_json(repo: &Value) -> Option<ExternalItem> {
        let full_name = repo["full_name"].as_str().unwrap_or("");
        let html_url = repo["html_url"].as_str().unwrap_or("");
        if full_name.is_empty() || html_url.is_empty() {
            return None;
        }

        let repo_id = repo["id"]
            .as_i64()
            .map(|id| id.to_string())
            .unwrap_or_default();
        let description = repo["description"].as_str().unwrap_or("").to_string();
        let owner_name = repo["owner"]["login"].as_str().map(|s| s.to_string());
        let owner_avatar = repo["owner"]["avatar_url"].as_str().map(|s| s.to_string());
        let language = repo["language"].as_str().map(|s| s.to_string());
        let stargazers_count = repo["stargazers_count"].as_i64().unwrap_or(0);
        let pushed_at = repo["pushed_at"].as_str().and_then(|s| parse_iso8601(s));

        Some(ExternalItem {
            source: "github".into(),
            external_id: repo_id,
            source_url: html_url.to_string(),
            title: full_name.to_string(),
            description,
            cover_url: owner_avatar,
            cover_local_path: None,
            author_name: owner_name,
            author_id: None,
            partition_name: language,
            published_at: pushed_at,
            duration: None,
            favorite_time: pushed_at,
            extra: serde_json::json!({
                "stargazers_count": stargazers_count,
            }),
        })
    }
}

/// Parse ISO 8601 date string to Unix timestamp
fn parse_iso8601(s: &str) -> Option<i64> {
    // GitHub returns dates like "2025-10-11T12:36:10Z"
    // Simple parsing: YYYY-MM-DDTHH:MM:SSZ
    let s = s.trim();
    if s.len() < 19 {
        return None;
    }
    let parts: Vec<&str> = s[..19].split(&['-', 'T', ':']).collect();
    if parts.len() != 6 {
        return None;
    }
    let year: i32 = parts[0].parse().ok()?;
    let month: u32 = parts[1].parse().ok()?;
    let day: u32 = parts[2].parse().ok()?;
    let hour: u32 = parts[3].parse().ok()?;
    let min: u32 = parts[4].parse().ok()?;
    let sec: u32 = parts[5].parse().ok()?;

    // Use a simple formula: days since Unix epoch + time
    let days = days_since_epoch(year as i32, month, day)?;
    let total_seconds = days as i64 * 86400 + hour as i64 * 3600 + min as i64 * 60 + sec as i64;
    Some(total_seconds)
}

fn days_since_epoch(year: i32, month: u32, day: u32) -> Option<i32> {
    if month < 1 || month > 12 || day < 1 || day > 31 {
        return None;
    }
    let mut days = (year - 1970) * 365;
    // Add leap days
    for y in 1970..year {
        if is_leap(y) {
            days += 1;
        }
    }
    let month_days = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut mdays = 0;
    for i in 0..(month - 1) as usize {
        mdays += month_days[i];
    }
    if month > 2 && is_leap(year) {
        mdays += 1;
    }
    days += mdays + day as i32 - 1;
    Some(days)
}

fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

#[async_trait]
impl SourceAdapter for GithubClient {
    async fn list_collections(&self) -> Result<Vec<CollectionInfo>, AppError> {
        // GitHub stars are a single flat list, not organized into collections.
        // Return a single "Starred Repos" collection.
        // This is called by resolve_collection path.
        Ok(vec![CollectionInfo {
            source: "github".into(),
            id: "starred".into(),
            title: "Starred Repos".into(),
            owner: None,
            count: 0,
            url: None,
        }])
    }

    async fn resolve_collection(&self, input: &str) -> Result<CollectionInfo, AppError> {
        // input is a GitHub username
        let username = input.trim();
        if username.is_empty() {
            return Err(AppError::InvalidInput("GitHub 用户名不能为空".into()));
        }

        // Fetch first page to get the total count (via Link header or estimate)
        let url = format!(
            "https://api.github.com/users/{}/starred?per_page=1&page=1",
            username
        );
        let _resp = self.fetch_json(&url).await?;

        Ok(CollectionInfo {
            source: "github".into(),
            id: "starred".into(),
            title: format!("{}'s Stars", username),
            owner: Some(username.to_string()),
            count: 0, // Will be updated when fetching
            url: None,
        })
    }

    async fn fetch_collection(
        &self,
        collection: &CollectionInfo,
    ) -> Result<Vec<ExternalItem>, AppError> {
        let username = collection.owner.as_deref().unwrap_or("");
        if username.is_empty() {
            return Err(AppError::InvalidInput("GitHub 用户名不能为空".into()));
        }

        let mut items = Vec::new();
        let mut page = 1i64;

        loop {
            let url = format!(
                "https://api.github.com/users/{}/starred?per_page=100&page={}",
                username, page
            );
            let json = self.fetch_json(&url).await?;

            let arr = json.as_array();
            if let Some(repos) = arr {
                if repos.is_empty() {
                    break;
                }
                for repo in repos {
                    if let Some(external_item) = Self::item_from_json(repo) {
                        items.push(external_item);
                    }
                }
                if repos.len() < 100 {
                    break;
                }
            } else {
                break;
            }
            page += 1;

            // Rate limit: 200ms between pages
            sleep(std::time::Duration::from_millis(200)).await;
        }

        Ok(items)
    }

    async fn enrich_items(&self, items: &[ExternalItem]) -> Result<Vec<ExternalItem>, AppError> {
        // GitHub items already have full info from the API
        Ok(items.to_vec())
    }
}

// ── Dedicated methods for GitHub ──

impl GithubClient {
    /// List starred repos for a given GitHub username (alias for resolve + fetch)
    pub async fn list_stars_for_user(
        &self,
        username: &str,
    ) -> Result<Vec<CollectionInfo>, AppError> {
        let url = format!(
            "https://api.github.com/users/{}/starred?per_page=1&page=1",
            username
        );
        let _resp = self.fetch_json(&url).await?;

        Ok(vec![CollectionInfo {
            source: "github".into(),
            id: "starred".into(),
            title: format!("{}'s Stars", username),
            owner: Some(username.to_string()),
            count: 0,
            url: None,
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_iso8601() {
        let ts = parse_iso8601("2025-10-11T12:36:10Z").unwrap();
        assert!(ts > 0);
    }

    #[test]
    fn test_parse_iso8601_with_tz() {
        let ts = parse_iso8601("2025-10-11T12:36:10+00:00").unwrap();
        assert!(ts > 0);
    }
}
