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
        let parsed = parse_public_favorite_url(url)?;
        match parsed.collection_type.as_str() {
            "bili_heji" | "bili_series" => self.resolve_heji(&parsed, url).await,
            _ => self.resolve_fav(&parsed, url).await,
        }
    }

    /// 解析普通收藏夹（/favlist?fid=）：`fid` 即 media_id，个别旧链接需拼接 `mid % 100` 再混淆。
    async fn resolve_fav(&self, parsed: &ParsedBiliUrl, url: &str) -> Result<CollectionInfo, AppError> {
        let candidates = vec![parsed.id.clone(), format!("{}{:02}", parsed.id, parsed.mid % 100)];
        let mut last_error = AppError::NotFound("没有找到这个公开收藏夹".into());
        for candidate in &candidates {
            let info = self
                .get_json(
                    "https://api.bilibili.com/x/v3/fav/folder/info",
                    vec![("media_id".into(), candidate.clone())],
                    false,
                )
                .await;
            match info {
                Ok(value) => {
                    if let Some(data) = value.get("data").and_then(Value::as_object) {
                        return Ok(CollectionInfo {
                            source: "bilibili".into(),
                            id: data
                                .get("id")
                                .and_then(Value::as_i64)
                                .map(|value| value.to_string())
                                .unwrap_or(candidate.clone()),
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
                        });
                    }
                    // 接口成功但 data 为 null：说明这个 media_id 不对，尝试下一个候选
                    last_error = AppError::NotFound("没有找到这个公开收藏夹".into());
                }
                Err(error) => return Err(error),
            }
        }
        Err(last_error)
    }

    /// 解析合集/系列（/channel/collectiondetail 或 /channel/seriesdetail）：用 seasons_archives_list 取元信息。
    async fn resolve_heji(&self, parsed: &ParsedBiliUrl, url: &str) -> Result<CollectionInfo, AppError> {
        let value = self
            .get_json(
                "https://api.bilibili.com/x/polymer/web-space/seasons_archives_list",
                vec![
                    ("season_id".into(), parsed.id.clone()),
                    ("page_num".into(), "1".into()),
                    ("page_size".into(), "1".into()),
                ],
                false,
            )
            .await?;
        let meta = value
            .pointer("/data/meta")
            .and_then(Value::as_object)
            .ok_or_else(|| AppError::NotFound("没有找到这个合集/系列".into()))?;
        let is_series = parsed.collection_type == "bili_series";
        let fallback_title = if is_series { "公开系列" } else { "公开合集" };
        Ok(CollectionInfo {
            source: "bilibili".into(),
            // 用前缀编码类型，fetch_collection 据此分支（避免给共享结构体加字段而改动其它来源）
            id: format!("{}_{}", parsed.collection_type, parsed.id),
            title: meta
                .get("name")
                .and_then(Value::as_str)
                .or_else(|| meta.get("title").and_then(Value::as_str))
                .unwrap_or(fallback_title)
                .to_string(),
            owner: None,
            count: meta.get("total").and_then(Value::as_i64).unwrap_or(0),
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
        // collection.id 前缀编码了类型（fav 无前缀；合集 `heji_`、系列 `series_`）
        if collection.id.starts_with("heji_") || collection.id.starts_with("series_") {
            self.fetch_heji_collection(collection).await
        } else {
            self.fetch_fav_collection(collection).await
        }
    }

    async fn enrich_items(&self, items: &[ExternalItem]) -> Result<Vec<ExternalItem>, AppError> {
        let mut enriched = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            // 图文 / 音频 / 专栏等非视频项没有 bvid，跳过 web-interface/view 详情补全
            if !item.external_id.starts_with("BV") {
                enriched.push(item.clone());
                continue;
            }
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

impl BilibiliClient {
    /// 拉取普通收藏夹：同时支持视频（type=2）与图文/音频/专栏等非视频项（用 link 解析）。
    async fn fetch_fav_collection(
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
            if medias.is_empty() {
                break;
            }
            for media in &medias {
                let mtype = media.get("type").and_then(Value::as_i64).unwrap_or(2);
                if mtype == 2 {
                    // 视频：用 bvid
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
                } else {
                    // 图文 / 音频 / 专栏等非视频项：用 link 跳转 uri 解析出 id 与地址
                    let id = media.get("id").and_then(Value::as_i64).unwrap_or(0);
                    let link = media.get("link").and_then(Value::as_str).unwrap_or_default();
                    let Some((external_id, source_url)) = parse_media_link(link, id, mtype) else {
                        continue;
                    };
                    items.push(ExternalItem {
                        source: "bilibili".into(),
                        external_id,
                        source_url,
                        title: media
                            .get("title")
                            .and_then(Value::as_str)
                            .unwrap_or("未命名图文")
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
                        extra: json!({ "bili_type": mtype, "id": id }),
                    });
                }
            }
            if medias.len() < 20 {
                break;
            }
            page += 1;
            if page > 100 {
                break;
            }
        }
        Ok(items)
    }

    /// 拉取合集/系列：seasons_archives_list 返回标准视频（bvid），与收藏夹视频同构。
    async fn fetch_heji_collection(
        &self,
        collection: &CollectionInfo,
    ) -> Result<Vec<ExternalItem>, AppError> {
        // collection.id 形如 `heji_587216` / `series_587216`，去掉前缀得到 season_id
        let season_id = collection
            .id
            .split_once('_')
            .map(|(_, rest)| rest.to_string())
            .unwrap_or_else(|| collection.id.clone());
        let mut items = Vec::new();
        let mut page = 1;
        loop {
            let value = self
                .get_json(
                    "https://api.bilibili.com/x/polymer/web-space/seasons_archives_list",
                    vec![
                        ("season_id".into(), season_id.clone()),
                        ("page_num".into(), page.to_string()),
                        ("page_size".into(), "20".into()),
                    ],
                    false,
                )
                .await?;
            let data = value
                .get("data")
                .and_then(Value::as_object)
                .ok_or_else(|| AppError::Other("合集/系列接口未返回数据".into()))?;
            let archives = data
                .get("archives")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if archives.is_empty() {
                break;
            }
            for archive in &archives {
                let bvid = archive
                    .get("bvid")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                if bvid.is_empty() {
                    continue;
                }
                let aid = archive.get("aid").and_then(Value::as_i64).unwrap_or(0);
                items.push(ExternalItem {
                    source: "bilibili".into(),
                    external_id: bvid.clone(),
                    source_url: format!("https://www.bilibili.com/video/{bvid}"),
                    title: archive
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or("未命名视频")
                        .to_string(),
                    description: String::new(),
                    cover_url: archive
                        .get("pic")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                    cover_local_path: None,
                    author_name: None,
                    author_id: None,
                    partition_name: None,
                    published_at: archive.get("pubdate").and_then(Value::as_i64),
                    duration: archive.get("duration").and_then(Value::as_i64),
                    favorite_time: None,
                    extra: json!({ "avid": aid, "page": 1 }),
                });
            }
            if archives.len() < 20 {
                break;
            }
            page += 1;
            if page > 100 {
                break;
            }
        }
        Ok(items)
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

/// 解析出的 B 站公开链接信息：用户 MID、内容 ID（收藏夹 fid / 合集·系列 sid）、以及类型。
struct ParsedBiliUrl {
    mid: i64,
    id: String,
    collection_type: String,
}

fn parse_public_favorite_url(value: &str) -> Result<ParsedBiliUrl, AppError> {
    let url = Url::parse(value)
        .map_err(|_| AppError::InvalidInput("请输入有效的 B 站收藏夹链接".into()))?;
    if !url
        .host_str()
        .is_some_and(|host| host == "space.bilibili.com" || host.ends_with(".bilibili.com"))
    {
        return Err(AppError::InvalidInput(
            "仅支持 bilibili 收藏夹/合集/系列链接".into(),
        ));
    }
    let mid = url
        .path_segments()
        .and_then(|mut segments| segments.next())
        .and_then(|segment| segment.parse::<i64>().ok())
        .ok_or_else(|| AppError::InvalidInput("链接中缺少用户 MID".into()))?;

    let path = url.path().to_string();
    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let get = |key: &str| pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());

    // 1) 新版合集/系列：/lists/{id}?type=season|series （V6.39+ 空间页默认形态）
    if path.contains("/lists/") {
        // 取 /lists/ 后的第一段作为 id，截掉可能的 ?query 或 / 后缀
        let after = path.split("/lists/").nth(1).unwrap_or("");
        let id = after.split(['?', '/']).next().unwrap_or(after).to_string();
        if id.is_empty() {
            return Err(AppError::InvalidInput("链接中缺少合集/系列 id".into()));
        }
        let collection_type = match get("type").as_deref() {
            Some("series") => "bili_series",
            _ => "bili_heji", // type=season 或省略均视为合集
        };
        return Ok(ParsedBiliUrl {
            mid,
            id,
            collection_type: collection_type.to_string(),
        });
    }

    // 2) 旧版合集/系列：channel/collectiondetail?sid= | channel/seriesdetail?sid=
    if path.contains("collectiondetail") {
        let id = get("sid")
            .ok_or_else(|| AppError::InvalidInput("链接中缺少合集 sid".into()))?;
        return Ok(ParsedBiliUrl {
            mid,
            id,
            collection_type: "bili_heji".into(),
        });
    }
    if path.contains("seriesdetail") {
        let id = get("sid")
            .ok_or_else(|| AppError::InvalidInput("链接中缺少系列 sid".into()))?;
        return Ok(ParsedBiliUrl {
            mid,
            id,
            collection_type: "bili_series".into(),
        });
    }

    // 3) 收藏夹（含「被收藏的合集」ftype=collect / ctype=21）
    if path.contains("favlist") {
        let is_collected_heji =
            get("ftype").as_deref() == Some("collect") || get("ctype").as_deref() == Some("21");
        let id = get("fid")
            .ok_or_else(|| AppError::InvalidInput("链接中缺少收藏夹 fid".into()))?;
        // 「被收藏的合集」其 fid 实为底层合集的 season_id，须走 heji 路径而非普通收藏夹
        // （B站 season_id 与 media_id 是两套独立命名空间，同名数字指向不同内容）
        let collection_type = if is_collected_heji {
            "bili_heji"
        } else {
            "bili_fav"
        };
        return Ok(ParsedBiliUrl {
            mid,
            id,
            collection_type: collection_type.to_string(),
        });
    }

    // 4) 兜底：尝试从 fid/sid 解析为普通收藏夹
    if let Some(id) = get("fid").or_else(|| get("sid")) {
        return Ok(ParsedBiliUrl {
            mid,
            id,
            collection_type: "bili_fav".into(),
        });
    }

    Err(AppError::InvalidInput(
        "无法识别的 B 站链接类型（需为收藏夹/合集/系列）".into(),
    ))
}

/// 从收藏夹 media 的 `link` 跳转 uri 解析出 (external_id, source_url)。
/// 例如 `//www.bilibili.com/opus/12345` -> ("opus_12345", "https://www.bilibili.com/opus/12345")。
/// 无法解析时回退到 `id` 字段构造通用标识。
fn parse_media_link(link: &str, fallback_id: i64, item_type: i64) -> Option<(String, String)> {
    let trimmed = link
        .trim()
        .trim_start_matches("bilibili://")
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("//");
    if let Some(idx) = trimmed.find("bilibili.com/") {
        let path = &trimmed[idx + "bilibili.com/".len()..];
        let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if segs.len() >= 2 {
            let kind = segs[0];
            // 去掉分享链接可能携带的 ?query / #fragment，保证 id 干净
            let raw = segs[1].split(['?', '#']).next().unwrap_or(segs[1]);
            match kind {
                // opus 路径段即纯数字 id（如 /opus/947531371067211815）
                "opus" => {
                    return Some((
                        format!("opus_{raw}"),
                        format!("https://www.bilibili.com/opus/{raw}"),
                    ))
                }
                // audio / read 的 URL 路径段里已自带 au / cv 前缀（如 /audio/au3688627），
                // external_id 再拼前缀时要剥掉，避免 au_au3688627 这种重复前缀
                "audio" => {
                    let id = raw.strip_prefix("au").unwrap_or(raw);
                    return Some((
                        format!("au_{id}"),
                        format!("https://www.bilibili.com/audio/{raw}"),
                    ))
                }
                "read" => {
                    let id = raw.strip_prefix("cv").unwrap_or(raw);
                    return Some((
                        format!("cv_{id}"),
                        format!("https://www.bilibili.com/read/{raw}"),
                    ))
                }
                _ => {}
            }
        }
    }
    if fallback_id != 0 {
        return Some((
            format!("bili_{item_type}_{fallback_id}"),
            "https://www.bilibili.com/".to_string(),
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_public_favorite_url() {
        let parsed =
            parse_public_favorite_url("https://space.bilibili.com/7792521/favlist?fid=442339")
                .unwrap();
        assert_eq!(parsed.mid, 7792521);
        assert_eq!(parsed.id, "442339");
        assert_eq!(parsed.collection_type, "bili_fav");
    }

    #[test]
    fn parses_public_heji_url() {
        let parsed = parse_public_favorite_url(
            "https://space.bilibili.com/397274588/channel/collectiondetail?sid=12345",
        )
        .unwrap();
        assert_eq!(parsed.mid, 397274588);
        assert_eq!(parsed.id, "12345");
        assert_eq!(parsed.collection_type, "bili_heji");
    }

    #[test]
    fn parses_public_series_url() {
        let parsed = parse_public_favorite_url(
            "https://space.bilibili.com/397274588/channel/seriesdetail?sid=67890",
        )
        .unwrap();
        assert_eq!(parsed.id, "67890");
        assert_eq!(parsed.collection_type, "bili_series");
    }

    // 新版空间页合集：/lists/{id}?type=season（id 在路径段，不在 ?sid=）
    #[test]
    fn parses_public_lists_heji_url() {
        let parsed = parse_public_favorite_url(
            "https://space.bilibili.com/170122257/lists/429082?type=season",
        )
        .unwrap();
        assert_eq!(parsed.mid, 170122257);
        assert_eq!(parsed.id, "429082");
        assert_eq!(parsed.collection_type, "bili_heji");
    }

    // 新版空间页系列：/lists/{id}?type=series
    #[test]
    fn parses_public_lists_series_url() {
        let parsed = parse_public_favorite_url(
            "https://space.bilibili.com/170122257/lists/123456?type=series",
        )
        .unwrap();
        assert_eq!(parsed.id, "123456");
        assert_eq!(parsed.collection_type, "bili_series");
    }

    // 收藏页复制的「被收藏的合集」：fid 实为底层合集的 season_id，须走 heji 路径
    #[test]
    fn parses_public_collected_heji_url() {
        let parsed = parse_public_favorite_url(
            "https://space.bilibili.com/3546659524970809/favlist?fid=429082&ftype=collect&ctype=21",
        )
        .unwrap();
        assert_eq!(parsed.mid, 3546659524970809);
        assert_eq!(parsed.id, "429082");
        assert_eq!(parsed.collection_type, "bili_heji");
    }

    // 图文/音频/专栏等非视频项：从收藏夹 media 的 `link` 字段解析出 (external_id, source_url)
    #[test]
    fn parses_media_link_opus() {
        let (external_id, url) =
            parse_media_link("//www.bilibili.com/opus/953619104940425225", 0, 11).unwrap();
        assert_eq!(external_id, "opus_953619104940425225");
        assert_eq!(url, "https://www.bilibili.com/opus/953619104940425225");
    }

    #[test]
    fn parses_media_link_opus_with_query() {
        // 分享链接常带 ?share_source=... 等后缀，id 必须被截断干净
        let (external_id, _) = parse_media_link(
            "https://www.bilibili.com/opus/953619104940425225?share_source=copy_web",
            0,
            11,
        )
        .unwrap();
        assert_eq!(external_id, "opus_953619104940425225");
    }

    #[test]
    fn parses_media_link_audio_and_read() {
        let (au_id, au_url) =
            parse_media_link("//www.bilibili.com/audio/au12345", 0, 12).unwrap();
        assert_eq!(au_id, "au_12345");
        assert_eq!(au_url, "https://www.bilibili.com/audio/au12345");

        let (cv_id, cv_url) =
            parse_media_link("//www.bilibili.com/read/cv67890", 0, 99).unwrap();
        assert_eq!(cv_id, "cv_67890");
        assert_eq!(cv_url, "https://www.bilibili.com/read/cv67890");
    }

    #[test]
    fn parses_media_link_fallback_without_recognized_path() {
        // 无法从 link 识别类型时，回退用 id 字段构造通用标识
        let (external_id, url) =
            parse_media_link("bilibili://some/unknown/format", 7788, 11).unwrap();
        assert_eq!(external_id, "bili_11_7788");
        assert_eq!(url, "https://www.bilibili.com/");
    }
}
