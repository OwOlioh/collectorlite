use std::sync::RwLock;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE, COOKIE, REFERER, USER_AGENT};
use serde_json::{json, Value};
use url::Url;

use crate::error::AppError;
use crate::models::{BilibiliProfile, CollectionInfo, ExternalItem, QrSession, QrStatus};
use crate::source::SourceAdapter;
use crate::wbi::{
    build_query_string, encode_query_component, extract_wbi_keys, signed_query, WbiKeys,
};

const BILIBILI_REFERER: &str = "https://www.bilibili.com/";
const USER_AGENT_STR: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/126.0 Safari/537.36";

pub struct BilibiliClient {
    http: reqwest::Client,
    cookie: RwLock<Option<String>>,
    wbi_keys: RwLock<Option<WbiKeys>>,
}

impl BilibiliClient {
    pub fn new() -> Result<Self, AppError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()?;
        Ok(Self {
            http,
            cookie: RwLock::new(None),
            wbi_keys: RwLock::new(None),
        })
    }

    pub fn set_cookie(&self, cookie: Option<String>) {
        if let Ok(mut guard) = self.cookie.write() {
            *guard = cookie;
        }
    }

    pub fn cookie_value(&self) -> Option<String> {
        self.cookie.read().ok().and_then(|guard| guard.clone())
    }

    fn cookie_header(&self) -> Result<HeaderMap, AppError> {
        let mut headers = HeaderMap::new();
        if let Some(cookie) = self.cookie_value() {
            headers.insert(
                COOKIE,
                HeaderValue::from_str(&cookie)
                    .map_err(|error| AppError::Other(error.to_string()))?,
            );
        }
        Ok(headers)
    }

    async fn get_json(
        &self,
        url: &str,
        params: Vec<(String, String)>,
        signed: bool,
    ) -> Result<Value, AppError> {
        let params = if signed {
            let keys = self.ensure_wbi_keys().await?;
            signed_query(&keys, params)
        } else {
            params
        };
        self.request_json_plain(url, params).await
    }

    async fn request_json_plain(
        &self,
        url: &str,
        params: Vec<(String, String)>,
    ) -> Result<Value, AppError> {
        let query = build_query_string(&params);
        let full_url = if query.is_empty() {
            url.to_string()
        } else {
            format!("{url}?{query}")
        };
        let mut request = self
            .http
            .get(&full_url)
            .header(USER_AGENT, USER_AGENT_STR)
            .header(REFERER, BILIBILI_REFERER);
        let cookie_headers = self.cookie_header()?;
        request = request.headers(cookie_headers);
        self.parse_response(request.send().await?).await
    }

    async fn parse_response(&self, response: reqwest::Response) -> Result<Value, AppError> {
        let status = response.status();
        let text = response.text().await?;
        let value: Value = serde_json::from_str(&text)
            .map_err(|_| AppError::Other(format!("B 站返回了无法解析的数据（HTTP {status}）")))?;
        let code = value.get("code").and_then(Value::as_i64).unwrap_or(-1);
        if code == 0 {
            return Ok(value);
        }
        let message = value
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("未知错误")
            .to_string();
        if code == -352 {
            Err(AppError::RiskControl(
                "B 站触发了风控验证，请稍后重试或重新登录。".into(),
            ))
        } else if code == -101 {
            Err(AppError::AuthRequired)
        } else {
            Err(AppError::Bili(code, message))
        }
    }

    async fn ensure_wbi_keys(&self) -> Result<WbiKeys, AppError> {
        if let Some(keys) = self.wbi_keys.read().ok().and_then(|guard| guard.clone()) {
            return Ok(keys);
        }
        let nav = self
            .request_json_plain("https://api.bilibili.com/x/web-interface/nav", vec![])
            .await?;
        let keys = extract_wbi_keys(&nav)?;
        if let Ok(mut guard) = self.wbi_keys.write() {
            *guard = Some(keys.clone());
        }
        Ok(keys)
    }

    pub async fn start_qr_login(&self) -> Result<QrSession, AppError> {
        let value = self
            .get_json(
                "https://passport.bilibili.com/x/passport-login/web/qrcode/generate",
                vec![],
                false,
            )
            .await?;
        let data = value
            .get("data")
            .and_then(Value::as_object)
            .ok_or_else(|| AppError::Other("二维码接口未返回数据".into()))?;
        Ok(QrSession {
            qrcode_key: data
                .get("qrcode_key")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            qrcode_url: data
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        })
    }

    pub async fn poll_qr_login(&self, qrcode_key: &str) -> Result<QrStatus, AppError> {
        let url = format!(
            "https://passport.bilibili.com/x/passport-login/web/qrcode/poll?qrcode_key={}",
            encode_query_component(qrcode_key)
        );
        let response = self
            .http
            .get(&url)
            .header(USER_AGENT, USER_AGENT_STR)
            .header(REFERER, BILIBILI_REFERER)
            .send()
            .await?;
        let set_cookie = response.headers().get_all(reqwest::header::SET_COOKIE);
        if !set_cookie.iter().collect::<Vec<_>>().is_empty() {
            let cookie = set_cookie
                .iter()
                .filter_map(|value| value.to_str().ok())
                .map(|value| value.split(';').next().unwrap_or_default())
                .collect::<Vec<_>>()
                .join("; ");
            if !cookie.is_empty() {
                self.set_cookie(Some(cookie));
            }
        }
        let status = response.status();
        let text = response.text().await?;
        let value: Value = serde_json::from_str(&text)
            .map_err(|_| AppError::Other(format!("B 站返回了无法解析的数据（HTTP {status}）")))?;
        let code = value
            .pointer("/data/code")
            .and_then(Value::as_i64)
            .or_else(|| value.get("code").and_then(Value::as_i64))
            .unwrap_or(-1);
        let message = value
            .pointer("/data/message")
            .and_then(Value::as_str)
            .or_else(|| value.get("message").and_then(Value::as_str))
            .unwrap_or("等待扫码")
            .to_string();
        let profile = if code == 0 {
            Some(self.profile().await?)
        } else {
            None
        };
        Ok(QrStatus {
            code,
            message,
            profile,
        })
    }

    pub async fn profile(&self) -> Result<BilibiliProfile, AppError> {
        let value = self
            .get_json(
                "https://api.bilibili.com/x/web-interface/nav",
                vec![],
                false,
            )
            .await?;
        let data = value.get("data").and_then(Value::as_object);
        let is_login = data
            .and_then(|item| item.get("isLogin"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !is_login {
            return Ok(BilibiliProfile {
                is_login: false,
                mid: None,
                name: None,
                face: None,
            });
        }
        Ok(BilibiliProfile {
            is_login: true,
            mid: data
                .and_then(|item| item.get("mid"))
                .and_then(Value::as_i64),
            name: data
                .and_then(|item| item.get("uname"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            face: data
                .and_then(|item| item.get("face"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
        })
    }

    pub async fn list_own_favorites(&self) -> Result<Vec<CollectionInfo>, AppError> {
        let profile = self.profile().await?;
        let mid = profile
            .mid
            .ok_or_else(|| AppError::Other("无法获取当前用户 MID".into()))?;
        let value = self
            .get_json(
                "https://api.bilibili.com/x/v3/fav/folder/created/list-all",
                vec![("up_mid".into(), mid.to_string())],
                false,
            )
            .await?;
        parse_favorite_folder_list(&value)
    }

    pub async fn download_cover(&self, url: &str) -> Result<(Vec<u8>, String), AppError> {
        let response = self
            .http
            .get(url)
            .header(USER_AGENT, USER_AGENT_STR)
            .header(REFERER, BILIBILI_REFERER)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            return Err(AppError::Other(format!("下载封面失败（HTTP {status}）")));
        }
        let extension = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| {
                if value.contains("png") {
                    "png"
                } else if value.contains("webp") {
                    "webp"
                } else {
                    "jpg"
                }
            })
            .unwrap_or("jpg")
            .to_string();
        let bytes = response.bytes().await?.to_vec();
        Ok((bytes, extension))
    }

    pub async fn resolve_public_favorite(&self, url: &str) -> Result<CollectionInfo, AppError> {
        let (mid, fid) = parse_public_favorite_url(url)?;
        let candidate = format!("{}{:02}", fid, mid % 100);
        let info = self
            .get_json(
                "https://api.bilibili.com/x/v3/fav/folder/info",
                vec![("media_id".into(), candidate.clone())],
                false,
            )
            .await?;
        let data = info
            .get("data")
            .and_then(Value::as_object)
            .ok_or_else(|| AppError::NotFound("没有找到这个公开收藏夹".into()))?;
        Ok(CollectionInfo {
            source: "bilibili".into(),
            id: data
                .get("id")
                .and_then(Value::as_i64)
                .map(|value| value.to_string())
                .unwrap_or(candidate),
            title: data
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("公开收藏夹")
                .to_string(),
            owner: data
                .get("upper")
                .and_then(Value::as_object)
                .and_then(|upper| upper.get("name"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            count: data.get("media_count").and_then(Value::as_i64).unwrap_or(0),
            url: Some(url.to_string()),
        })
    }
}

#[async_trait]
impl SourceAdapter for BilibiliClient {
    async fn list_collections(&self) -> Result<Vec<CollectionInfo>, AppError> {
        self.list_own_favorites().await
    }

    async fn resolve_collection(&self, input: &str) -> Result<CollectionInfo, AppError> {
        self.resolve_public_favorite(input).await
    }

    async fn fetch_collection(
        &self,
        collection: &CollectionInfo,
    ) -> Result<Vec<ExternalItem>, AppError> {
        let mut items = Vec::new();
        let mut page = 1;
        loop {
            let value = self
                .get_json(
                    "https://api.bilibili.com/x/v3/fav/resource/list",
                    vec![
                        ("media_id".into(), collection.id.clone()),
                        ("platform".into(), "web".into()),
                        ("pn".into(), page.to_string()),
                        ("ps".into(), "20".into()),
                    ],
                    false,
                )
                .await?;
            let data = value
                .get("data")
                .and_then(Value::as_object)
                .ok_or_else(|| AppError::Other("收藏夹接口未返回数据".into()))?;
            let medias = data
                .get("medias")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for media in medias {
                if media.get("type").and_then(Value::as_i64) != Some(2) {
                    continue;
                }
                let bvid = media
                    .get("bvid")
                    .and_then(Value::as_str)
                    .or_else(|| media.get("bv_id").and_then(Value::as_str))
                    .unwrap_or_default()
                    .to_string();
                if bvid.is_empty() {
                    continue;
                }
                let aid = media.get("id").and_then(Value::as_i64).unwrap_or(0);
                items.push(ExternalItem {
                    source: "bilibili".into(),
                    external_id: bvid.clone(),
                    source_url: format!("https://www.bilibili.com/video/{bvid}"),
                    title: media
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or("未命名视频")
                        .to_string(),
                    description: media
                        .get("intro")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    cover_url: media
                        .get("cover")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                    cover_local_path: None,
                    author_name: media
                        .pointer("/upper/name")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                    author_id: media
                        .pointer("/upper/mid")
                        .and_then(Value::as_i64)
                        .map(|value| value.to_string()),
                    partition_name: None,
                    published_at: media
                        .get("pubtime")
                        .and_then(Value::as_i64)
                        .or_else(|| media.get("ctime").and_then(Value::as_i64)),
                    duration: media.get("duration").and_then(Value::as_i64),
                    favorite_time: media.get("fav_time").and_then(Value::as_i64),
                    extra: json!({ "avid": aid, "page": media.get("page").and_then(Value::as_i64).unwrap_or(1) }),
                });
            }
            if !data
                .get("has_more")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                break;
            }
            page += 1;
            if page > 100 {
                break;
            }
        }
        Ok(items)
    }

    async fn enrich_items(&self, items: &[ExternalItem]) -> Result<Vec<ExternalItem>, AppError> {
        let mut enriched = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            if index > 0 {
                tokio::time::sleep(Duration::from_millis(180)).await;
            }
            let mut next = item.clone();
            match self
                .get_json(
                    "https://api.bilibili.com/x/web-interface/view",
                    vec![("bvid".into(), item.external_id.clone())],
                    true,
                )
                .await
            {
                Ok(value) => {
                    if let Some(data) = value.get("data").and_then(Value::as_object) {
                        next.title = data
                            .get("title")
                            .and_then(Value::as_str)
                            .unwrap_or(&next.title)
                            .to_string();
                        next.description = data
                            .get("desc")
                            .and_then(Value::as_str)
                            .unwrap_or(&next.description)
                            .to_string();
                        next.cover_url = data
                            .get("pic")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned)
                            .or_else(|| next.cover_url.clone());
                        next.author_name = data
                            .get("owner")
                            .and_then(Value::as_object)
                            .and_then(|owner| owner.get("name"))
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned)
                            .or_else(|| next.author_name.clone());
                        next.author_id = data
                            .get("owner")
                            .and_then(Value::as_object)
                            .and_then(|owner| owner.get("mid"))
                            .and_then(Value::as_i64)
                            .map(|value| value.to_string())
                            .or_else(|| next.author_id.clone());
                        next.partition_name = data
                            .get("tname")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned);
                        next.published_at = data
                            .get("pubdate")
                            .and_then(Value::as_i64)
                            .or(next.published_at);
                        next.duration = data
                            .get("duration")
                            .and_then(Value::as_i64)
                            .or(next.duration);
                    }
                }
                Err(AppError::RiskControl(message)) => return Err(AppError::RiskControl(message)),
                Err(_) => {}
            }
            enriched.push(next);
        }
        Ok(enriched)
    }
}

fn parse_favorite_folder_list(value: &Value) -> Result<Vec<CollectionInfo>, AppError> {
    let list = value
        .pointer("/data/list")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut collections = Vec::new();
    for item in list {
        let id = item
            .get("id")
            .and_then(Value::as_i64)
            .map(|value| value.to_string())
            .unwrap_or_default();
        if id.is_empty() {
            continue;
        }
        let fid = item.get("fid").and_then(Value::as_i64).unwrap_or(0);
        let mid = item.get("mid").and_then(Value::as_i64).unwrap_or(0);
        collections.push(CollectionInfo {
            source: "bilibili".into(),
            id,
            title: item
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("未命名收藏夹")
                .to_string(),
            owner: item
                .pointer("/upper/name")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            count: item.get("media_count").and_then(Value::as_i64).unwrap_or(0),
            url: Some(format!(
                "https://space.bilibili.com/{mid}/favlist?fid={fid}"
            )),
        });
    }
    Ok(collections)
}

fn parse_public_favorite_url(value: &str) -> Result<(i64, String), AppError> {
    let url = Url::parse(value)
        .map_err(|_| AppError::InvalidInput("请输入有效的 B 站收藏夹链接".into()))?;
    if !url
        .host_str()
        .is_some_and(|host| host == "space.bilibili.com" || host.ends_with(".bilibili.com"))
    {
        return Err(AppError::InvalidInput("仅支持 bilibili 收藏夹链接".into()));
    }
    let mid = url
        .path_segments()
        .and_then(|mut segments| segments.next())
        .and_then(|segment| segment.parse::<i64>().ok())
        .ok_or_else(|| AppError::InvalidInput("链接中缺少用户 MID".into()))?;
    let fid = url
        .query_pairs()
        .find(|(key, _)| key == "fid" || key == "sid")
        .map(|(_, value)| value.into_owned())
        .ok_or_else(|| AppError::InvalidInput("链接中缺少收藏夹 fid".into()))?;
    Ok((mid, fid))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_public_favorite_url() {
        let (mid, fid) =
            parse_public_favorite_url("https://space.bilibili.com/7792521/favlist?fid=442339")
                .unwrap();
        assert_eq!(mid, 7792521);
        assert_eq!(fid, "442339");
    }
}
