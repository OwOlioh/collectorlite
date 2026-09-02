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

/// enrich_items 并发度：用有界并发替代原来的「串行 + 每条 180ms 固定等待」，
/// 既提速又把对 B站接口的瞬时压力限制在一个保守值，降低触发风控的概率。
const ENRICH_CONCURRENCY: usize = 6;

/// 图文收藏（opus）在 `CollectionInfo.id` 中使用的哨兵值。
///
/// 图文收藏**不是收藏夹**：它没有 `media_id`，也永远不会出现在
/// `x/v3/fav/folder/created/list-all` 里（该接口会忽略 `type` 参数，
/// 恒返回全部视频收藏夹——已用 type=0..25 全量穷举验证）。
/// 它走的是独立的动态流接口，因此用一个不可能与数字 media_id 冲突的哨兵 id 承载。
pub const OPUS_FAV_COLLECTION_ID: &str = "bili_opus_fav";

/// 当前登录用户的图文收藏列表（**逆向自 B站空间页前端 bundle**，未见于公开文档）：
/// `GET /x/polymer/web-dynamic/v1/opus/feed/fav?page=&page_size=&timezone_offset=`
///
/// 注意**没有 `mid` 参数**——只返回当前登录用户自己的图文收藏，必须带 cookie。
const OPUS_FAV_URL: &str = "https://api.bilibili.com/x/polymer/web-dynamic/v1/opus/feed/fav";

/// 收藏夹导航栏，`data.opus` 为图文收藏条数（该接口免登录可读）。
const SPACE_FAV_NAV_URL: &str = "https://api.bilibili.com/x/space/fav/nav";

pub struct BilibiliClient {
    http: reqwest::Client,
    cookie: RwLock<Option<String>>,
    wbi_keys: RwLock<Option<WbiKeys>>,
}

impl BilibiliClient {
    pub fn new() -> Result<Self, AppError> {
        let mut builder = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::limited(5));
        // 复用统一代理解析（环境变量 → 注册表 → 本机端口兜底），
        // 保证中国大陆用户无论以何种方式启动 app 都能经代理出网，避免直连被风控 -400。
        if let Some(proxy) = crate::source::proxy::resolve_system_proxy() {
            builder = builder.proxy(proxy);
        }
        let http = builder.build()?;
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
        // 本地代理（如 Clash）上游偶发抖动会导致 CONNECT 隧道失败（TunnelUnsuccessful），
        // 仅对传输层错误重试；B站业务错误（如 -400）立即返回、不重试。
        let mut last_err: Option<reqwest::Error> = None;
        for attempt in 0..3 {
            let mut request = self
                .http
                .get(&full_url)
                .header(USER_AGENT, USER_AGENT_STR)
                .header(REFERER, BILIBILI_REFERER);
            let cookie_headers = self.cookie_header()?;
            request = request.headers(cookie_headers);
            let response = match request.send().await {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("[bili] 网络层错误（第{}次，将重试）: {}", attempt + 1, e);
                    last_err = Some(e);
                    continue;
                }
            };
            let status = response.status();
            let text = response.text().await?;
            let value: Value = serde_json::from_str(&text).map_err(|_| {
                // 非 JSON（如代理拦截页）：把原始响应打到 stderr 便于排查网络/代理问题
                eprintln!(
                    "[bili] 非 JSON 响应 {full_url} -> HTTP {status} body={}",
                    &text[..text.len().min(300)]
                );
                AppError::Other(format!("B 站返回了无法解析的数据（HTTP {status}）"))
            })?;
            let code = value.get("code").and_then(Value::as_i64).unwrap_or(-1);
            if code == 0 {
                return Ok(value);
            }
            let message = value
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("未知错误")
                .to_string();
            // 出错时打印真实请求与 B 站原始响应，便于定位（空 season_id / 代理未生效 等）
            eprintln!("[bili] 接口错误 {full_url} -> HTTP {status} code={code} msg={message}");
            if code == -352 {
                return Err(AppError::RiskControl(
                    "B 站触发了风控验证，请稍后重试或重新登录。".into(),
                ));
            } else if code == -101 {
                return Err(AppError::AuthRequired);
            } else {
                return Err(AppError::Bili(code, message));
            }
        }
        Err(AppError::Other(format!(
            "B 站网络请求失败（重试 3 次仍失败）：{}",
            last_err.map(|e| e.to_string()).unwrap_or_default()
        )))
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

    /// 图文收藏的元信息（标题 + 条数）。
    ///
    /// 条数取自 `/x/space/fav/nav` 的 `data.opus`。为 0 表示当前账号没有图文收藏，
    /// 前端据此隐藏入口——避免给没有图文收藏的用户一个点了就空的按钮。
    pub async fn opus_favorite_info(&self) -> Result<CollectionInfo, AppError> {
        let profile = self.profile().await?;
        let mid = profile
            .mid
            .ok_or_else(|| AppError::Other("无法获取当前用户 MID".into()))?;
        let value = self
            .get_json(
                SPACE_FAV_NAV_URL,
                vec![("mid".into(), mid.to_string())],
                false,
            )
            .await?;
        let count = value
            .get("data")
            .and_then(|data| data.get("opus"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        Ok(CollectionInfo {
            source: "bilibili".into(),
            id: OPUS_FAV_COLLECTION_ID.into(),
            title: "图文收藏".into(),
            owner: profile.name.clone(),
            count,
            url: Some(format!(
                "https://space.bilibili.com/{mid}/favlist?fid=opus&ftype=opus"
            )),
        })
    }

    /// 拉取当前登录用户的全部图文收藏（自动翻页直到取完）。
    pub async fn fetch_opus_favorites(&self) -> Result<Vec<ExternalItem>, AppError> {
        let mut items = Vec::new();
        let mut page = 1;
        loop {
            let value = self
                .get_json(
                    OPUS_FAV_URL,
                    vec![
                        ("page".into(), page.to_string()),
                        ("page_size".into(), "20".into()),
                        ("timezone_offset".into(), "-480".into()),
                    ],
                    false,
                )
                .await?;
            let list = value
                .get("data")
                .and_then(|data| data.get("items"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if list.is_empty() {
                break;
            }
            let parsed = list.iter().filter_map(parse_opus_item).collect::<Vec<_>>();
            if parsed.is_empty() {
                // B站改版导致字段形状变化时，落盘原始响应便于定位，而不是静默返回空列表。
                let path = std::env::temp_dir().join("bilibili_opus_fav_raw.json");
                if let Ok(text) = serde_json::to_string_pretty(&value) {
                    let _ = std::fs::write(&path, text);
                }
                eprintln!(
                    "[bili] 图文收藏第 {page} 页解析为空，原始响应已写入 {}",
                    path.display()
                );
                break;
            }
            items.extend(parsed);
            page += 1;
            // 安全上限：图文收藏一般最多几百条，2000 条足够且能兜住接口异常翻页。
            if page > 100 {
                break;
            }
        }
        Ok(items)
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
    async fn resolve_fav(
        &self,
        parsed: &ParsedBiliUrl,
        url: &str,
    ) -> Result<CollectionInfo, AppError> {
        let candidates = vec![
            parsed.id.clone(),
            format!("{}{:02}", parsed.id, parsed.mid % 100),
        ];
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
    async fn resolve_heji(
        &self,
        parsed: &ParsedBiliUrl,
        url: &str,
    ) -> Result<CollectionInfo, AppError> {
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
        let fallback_title = if is_series {
            "公开系列"
        } else {
            "公开合集"
        };
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

/// 合集/系列 collection.id 的前缀约定：`bili_heji_{season_id}` / `bili_series_{season_id}`。
/// 须与 `resolve_heji` 生成的 id 格式保持一致，否则 `fetch_collection` 会把合集错路由成普通收藏夹。
fn is_heji_collection_id(id: &str) -> bool {
    id.starts_with("bili_heji_") || id.starts_with("bili_series_")
}

/// 从合集/系列 collection.id 中提取 season_id（去掉 `bili_heji_` / `bili_series_` 前缀）。
fn heji_season_id(id: &str) -> String {
    id.strip_prefix("bili_heji_")
        .or_else(|| id.strip_prefix("bili_series_"))
        .map(|rest| rest.to_string())
        .unwrap_or_else(|| id.to_string())
}

/// 把 `/opus/feed/fav` 返回的单条图文项转成统一 `ExternalItem`。
///
/// 字段形状（逆向自 B站空间页前端 bundle，非公开文档）：
/// - `jump_url`：图文详情页链接，形如 `https://www.bilibili.com/opus/<id>`
/// - `content`：正文文本（前端直接当标题渲染）
/// - `cover.url` / `cover.width` / `cover.height`：封面
/// - `author.name` / `author.mid`：作者
/// - `pub_time`：发布时间（时间戳数字或 `YYYY-MM-DD` 字符串）
/// - `stat.view` / `stat.like`：浏览、点赞
///
/// `external_id` 取不到时返回 `None`（该项被跳过），因为 `(source, external_id)`
/// 是去重主键，编造 id 会导致重复导入。
/// 把 opus 响应里的 `author.mid` 统一成字符串 id。
///
/// 实测 B站「图文收藏」动态流接口返回的作者 mid 是**字符串**（`"3824575"`），
/// 而 `Value::as_i64` 对字符串返回 `None`。为兼容以后可能改成数字的情形，这里两种形态都认。
fn opus_mid_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => n.as_i64().map(|mid| mid.to_string()),
        _ => None,
    }
}

fn parse_opus_item(value: &Value) -> Option<ExternalItem> {
    let jump_url = value
        .get("jump_url")
        .and_then(Value::as_str)
        .map(normalize_opus_url);
    let external_id = value
        .get("id_str")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(|text| format!("opus_{text}"))
        .or_else(|| {
            value
                .get("id")
                .and_then(Value::as_i64)
                .map(|id| format!("opus_{id}"))
        })
        .or_else(|| jump_url.as_deref().and_then(opus_id_from_url))?;
    let content = value
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let title = truncate_chars(&content, 80);
    let url = jump_url.unwrap_or_else(|| {
        format!(
            "https://www.bilibili.com/opus/{}",
            external_id.trim_start_matches("opus_")
        )
    });
    Some(ExternalItem {
        source: "bilibili".into(),
        external_id,
        source_url: url,
        title: if title.is_empty() {
            "未命名图文".into()
        } else {
            title
        },
        description: content,
        cover_url: value
            .pointer("/cover/url")
            .and_then(Value::as_str)
            .map(normalize_opus_url),
        cover_local_path: None,
        author_name: value
            .pointer("/author/name")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        // opus 响应里 `author.mid` 是**字符串**（实测为 `"3824575"` 而非数字），
        // 用 `as_i64` 会直接返回 None 导致作者 id 丢失、链接失效，所以这里兼容字符串/数字两种形态。
        author_id: value.pointer("/author/mid").and_then(opus_mid_to_string),
        // 图文没有 B站分区，统一归入「图文」，便于在标签编辑器里整体打标签。
        partition_name: Some("图文".into()),
        published_at: parse_opus_pub_time(value.get("pub_time")),
        duration: None,
        favorite_time: None,
        extra: value.clone(),
    })
}

/// 图文的封面/链接可能是 `//host/path` 形式，补齐 scheme 供前端直接加载。
fn normalize_opus_url(url: &str) -> String {
    if url.starts_with("//") {
        format!("https:{url}")
    } else {
        url.to_string()
    }
}

/// 从图文链接里抠出 opus id：`https://www.bilibili.com/opus/123` → `opus_123`。
/// 与 `parse_media_link` 的 `opus/<数字>` 分支保持同一命名，确保跨入口去重一致。
fn opus_id_from_url(url: &str) -> Option<String> {
    let path = url.split('?').next().unwrap_or(url);
    let last = path.trim_end_matches('/').rsplit('/').next()?;
    if last.is_empty() || !last.chars().all(|char| char.is_ascii_digit()) {
        return None;
    }
    Some(format!("opus_{last}"))
}

/// `pub_time` 可能是秒/毫秒时间戳，也可能是 `YYYY-MM-DD[ HH:MM:SS]` 字符串。
fn parse_opus_pub_time(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    if let Some(number) = value.as_i64() {
        return Some(if number > 1_000_000_000_000 {
            number / 1000
        } else {
            number
        });
    }
    let text = value.as_str()?.trim();
    let date_part = text.split(' ').next()?;
    let mut parts = date_part.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.parse().ok()?;
    if !(1970..=2200).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86_400)
}

/// 公历日期 → 距 Unix 纪元的天数（Howard Hinnant 算法，避免为此引入 chrono 依赖）。
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// 按**字符数**截断（按字节切会切坏中文）。超出部分以省略号结尾。
fn truncate_chars(text: &str, limit: usize) -> String {
    let mut result: String = text.chars().take(limit).collect();
    if text.chars().count() > limit {
        result.push('…');
    }
    result
}

/// 把 `web-interface/view` 返回的 `data` 合并进已存在的 `ExternalItem`（只覆盖有值字段）。
/// 抽成独立函数，便于 enrich_items 并发化后复用，保持与旧逻辑一致的字段映射。
fn apply_view_data(item: &mut ExternalItem, data: &serde_json::Map<String, Value>) {
    item.title = data
        .get("title")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| item.title.clone());
    item.description = data
        .get("desc")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| item.description.clone());
    if let Some(pic) = data.get("pic").and_then(Value::as_str) {
        item.cover_url = Some(pic.to_string());
    }
    if let Some(owner) = data.get("owner").and_then(Value::as_object) {
        if let Some(name) = owner.get("name").and_then(Value::as_str) {
            item.author_name = Some(name.to_string());
        }
        if let Some(mid) = owner.get("mid").and_then(Value::as_i64) {
            item.author_id = Some(mid.to_string());
        }
    }
    item.partition_name = data
        .get("tname")
        .and_then(Value::as_str)
        .map(str::to_string);
    if let Some(pubdate) = data.get("pubdate").and_then(Value::as_i64) {
        item.published_at = Some(pubdate);
    }
    if let Some(duration) = data.get("duration").and_then(Value::as_i64) {
        item.duration = Some(duration);
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
        // collection.id 前缀编码了类型：
        // - `bili_heji_` / `bili_series_` → 合集/系列（走 seasons_archives_list）
        // - `bili_opus_fav`             → 图文收藏（走 opus/feed/fav，无 media_id）
        // - 其余（裸数字 media_id）      → 普通收藏夹
        // 注意：此前曾误用 `heji_`/`series_` 前缀判断，而 resolve_heji 实际生成的是
        // `bili_heji_{season_id}` / `bili_series_{season_id}`，导致合集被错路由到
        // fetch_fav_collection，把整串当成 media_id 去调 fav/folder/info → B站返回 -400。
        let route = if collection.id == OPUS_FAV_COLLECTION_ID {
            "图文收藏"
        } else if is_heji_collection_id(&collection.id) {
            "合集/系列"
        } else {
            "普通收藏夹"
        };
        eprintln!(
            "[bili] fetch_collection 路由：collection.id={} -> {route}",
            collection.id
        );
        match route {
            "图文收藏" => self.fetch_opus_favorites().await,
            "合集/系列" => self.fetch_heji_collection(collection).await,
            _ => self.fetch_fav_collection(collection).await,
        }
    }

    async fn enrich_items(&self, items: &[ExternalItem]) -> Result<Vec<ExternalItem>, AppError> {
        // 有界并发：每批最多 ENRICH_CONCURRENCY 条同时请求 web-interface/view，
        // 替代原先「串行 + 每条 180ms 固定等待」的模式（1000 条原需 ~3 分钟纯等待）。
        // 非视频项（opus/audio/cv，无 bvid）直接跳过补全，保持原顺序。
        let mut enriched: Vec<ExternalItem> = items.to_vec();
        let mut risk_error: Option<AppError> = None;
        for chunk_start in (0..items.len()).step_by(ENRICH_CONCURRENCY) {
            let end = (chunk_start + ENRICH_CONCURRENCY).min(items.len());
            let futures: Vec<_> = (chunk_start..end)
                .map(|i| {
                    let item = &items[i];
                    async move {
                        if !item.external_id.starts_with("BV") {
                            return Ok((i, None));
                        }
                        match self
                            .get_json(
                                "https://api.bilibili.com/x/web-interface/view",
                                vec![("bvid".into(), item.external_id.clone())],
                                true,
                            )
                            .await
                        {
                            Ok(value) => Ok((i, Some(value))),
                            Err(AppError::RiskControl(message)) => {
                                Err(AppError::RiskControl(message))
                            }
                            Err(_) => Ok((i, None)),
                        }
                    }
                })
                .collect();
            let results = futures::future::join_all(futures).await;
            for result in results {
                match result {
                    Ok((i, Some(value))) => {
                        if let Some(data) = value.get("data").and_then(Value::as_object) {
                            apply_view_data(&mut enriched[i], data);
                        }
                    }
                    Ok(_) => {}
                    Err(AppError::RiskControl(message)) => {
                        risk_error = Some(AppError::RiskControl(message));
                    }
                    Err(_) => {}
                }
            }
            if risk_error.is_some() {
                break;
            }
        }
        if let Some(error) = risk_error {
            return Err(error);
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
        let mut fetched = 0usize;
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
                    let link = media
                        .get("link")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
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
            fetched += medias.len();
            // 终止条件：返回空页，或已拉取到接口声明的 media_count 上限。
            // 注意首屏可能少于 ps 条（实测某 946 项收藏夹第一页仅返回 18 条），
            // 不能用「< ps 即末页」判断，否则会静默漏掉后续所有页（大量项丢失）。
            let media_count = data
                .get("info")
                .and_then(Value::as_object)
                .and_then(|info| info.get("media_count"))
                .and_then(Value::as_i64)
                .unwrap_or(0);
            if media_count > 0 && (fetched as i64) >= media_count {
                break;
            }
            page += 1;
            if page > 1000 {
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
        // collection.id 形如 `bili_heji_587216` / `bili_series_587216`，去掉前缀得到 season_id
        let season_id = heji_season_id(&collection.id);
        eprintln!("[bili] fetch_heji_collection season_id={season_id}");
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
                    author_name: archive
                        .pointer("/owner/name")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                    author_id: archive
                        .pointer("/owner/mid")
                        .and_then(Value::as_i64)
                        .map(|mid| mid.to_string()),
                    partition_name: None,
                    published_at: archive.get("pubdate").and_then(Value::as_i64),
                    duration: archive.get("duration").and_then(Value::as_i64),
                    favorite_time: None,
                    extra: json!({ "avid": aid, "page": 1 }),
                });
            }
            // 不再用「< ps 即末页」判断（首屏可能少于 ps 条导致漏页）；
            // 改为遇到空页才终止，并在页码上限处兜底，避免合集/系列大列表静默截断。
            page += 1;
            if page > 200 {
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
        let id = get("sid").ok_or_else(|| AppError::InvalidInput("链接中缺少合集 sid".into()))?;
        return Ok(ParsedBiliUrl {
            mid,
            id,
            collection_type: "bili_heji".into(),
        });
    }
    if path.contains("seriesdetail") {
        let id = get("sid").ok_or_else(|| AppError::InvalidInput("链接中缺少系列 sid".into()))?;
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
        let id = get("fid").ok_or_else(|| AppError::InvalidInput("链接中缺少收藏夹 fid".into()))?;
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
                    ));
                }
                "read" => {
                    let id = raw.strip_prefix("cv").unwrap_or(raw);
                    return Some((
                        format!("cv_{id}"),
                        format!("https://www.bilibili.com/read/{raw}"),
                    ));
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
        let (au_id, au_url) = parse_media_link("//www.bilibili.com/audio/au12345", 0, 12).unwrap();
        assert_eq!(au_id, "au_12345");
        assert_eq!(au_url, "https://www.bilibili.com/audio/au12345");

        let (cv_id, cv_url) = parse_media_link("//www.bilibili.com/read/cv67890", 0, 99).unwrap();
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

    // ---- 回归测试：fetch_collection 的合集/系列路由（曾因前缀 `heji_` 误判导致 -400）----

    #[test]
    fn routing_recognizes_heji_and_series_prefix() {
        // resolve_heji 实际生成的是 `bili_heji_<season_id>` / `bili_series_<season_id>`，
        // 路由判断必须匹配这个前缀，否则会错进 fetch_fav_collection → -400。
        assert!(is_heji_collection_id("bili_heji_429082"));
        assert!(is_heji_collection_id("bili_series_587216"));
        // 普通收藏夹是裸 media_id（无前缀），不应被识别为合集
        assert!(!is_heji_collection_id("12345"));
        assert!(!is_heji_collection_id("bili_fav_12345"));
    }

    #[test]
    fn routing_extracts_season_id_from_prefixed_id() {
        assert_eq!(heji_season_id("bili_heji_429082"), "429082");
        assert_eq!(heji_season_id("bili_series_587216"), "587216");
        // 兜底：无前缀时原样返回，避免 panic
        assert_eq!(heji_season_id("12345"), "12345");
    }

    // ---- 回归测试：图文收藏（opus）项解析（字段形状逆向自空间页 bundle，无公开文档）----

    #[test]
    fn parses_opus_item_from_bundle_shape() {
        let value = json!({
            "id_str": "953619104940425225",
            "jump_url": "https://www.bilibili.com/opus/953619104940425225",
            "content": "一张练习用的图",
            "cover": { "url": "//i0.hdslb.com/bfs/opus/abc.jpg", "width": 1200, "height": 800 },
            "author": { "name": "UP主", "mid": 12345 },
            "pub_time": 1700000000,
            "stat": { "view": 1000, "like": 20 }
        });
        let item = parse_opus_item(&value).expect("应能解析出图文项");
        assert_eq!(item.external_id, "opus_953619104940425225");
        assert_eq!(
            item.source_url,
            "https://www.bilibili.com/opus/953619104940425225"
        );
        assert_eq!(item.title, "一张练习用的图");
        assert_eq!(
            item.cover_url.as_deref(),
            Some("https://i0.hdslb.com/bfs/opus/abc.jpg")
        );
        assert_eq!(item.author_name.as_deref(), Some("UP主"));
        assert_eq!(item.author_id.as_deref(), Some("12345"));
        assert_eq!(item.partition_name.as_deref(), Some("图文"));
        assert_eq!(item.published_at, Some(1700000000));
    }

    #[test]
    fn parses_opus_item_string_mid_like_real_api() {
        // 真实「图文收藏」动态流接口返回的 author.mid 是字符串（如 "3824575"），
        // 早期用 as_i64 解析会丢 id。此测试固化该修复。
        let value = json!({
            "id_str": "666479495887192082",
            "jump_url": "https://www.bilibili.com/opus/666479495887192082",
            "content": "批量自动采集b站收藏夹内容",
            "author": { "name": "綾濑千早", "mid": "3824575" },
            "pub_time": "2024-11-14"
        });
        let item = parse_opus_item(&value).expect("应能解析出图文项");
        assert_eq!(item.author_name.as_deref(), Some("綾濑千早"));
        assert_eq!(item.author_id.as_deref(), Some("3824575"));
    }

    #[test]
    fn parses_opus_item_numeric_id_fallback() {
        // 老版本/边界返回里可能只有数字 id，没有 id_str
        let value = json!({ "id": 947531371067211815_u64, "content": "无 id_str" });
        let item = parse_opus_item(&value).expect("应能回退到数字 id");
        assert_eq!(item.external_id, "opus_947531371067211815");
    }

    #[test]
    fn parses_opus_item_jump_url_fallback() {
        // id_str 与 id 都缺失时，从 jump_url 里抠 id
        let value = json!({ "jump_url": "https://www.bilibili.com/opus/953619104940425225?share_source=copy_web", "content": "仅链接" });
        let item = parse_opus_item(&value).expect("应能从 jump_url 抠出 id");
        assert_eq!(item.external_id, "opus_953619104940425225");
    }

    #[test]
    fn opus_item_without_any_id_is_skipped() {
        // 三处 id 都取不到时返回 None，避免编造 id 破坏 (source, external_id) 去重
        assert!(parse_opus_item(&json!({ "content": "没有 id" })).is_none());
    }

    #[test]
    fn parses_opus_pub_time_millis_and_string() {
        // 毫秒时间戳 → 转秒
        assert_eq!(
            parse_opus_pub_time(Some(&json!(1700000000000_i64))),
            Some(1700000000)
        );
        // 秒时间戳原样返回
        assert_eq!(
            parse_opus_pub_time(Some(&json!(1700000000_i64))),
            Some(1700000000)
        );
        // 字符串日期也能解析
        assert!(parse_opus_pub_time(Some(&json!("2024-11-14"))).is_some());
        // 空值 → None
        assert_eq!(parse_opus_pub_time(None), None);
    }

    /// 真实环境性能基准（默认忽略，需 `--ignored` 才运行）：
    /// 对当前登录用户**最大的视频收藏夹**实跑优化后的 fetch_collection + enrich_items + 封面下载，
    /// 分相计时，并打印「串行旧版」在同样数据上的实测对照，用于校准 perf 估算。
    #[tokio::test]
    #[ignore]
    async fn bench_real_import() {
        use std::time::Instant;
        use futures::future::join_all;
        use crate::source::SourceAdapter;

        // 1) 读取本机登录 cookie（与 app 共用同一文件）
        let app_data = std::env::var("APPDATA").expect("APPDATA 未设置");
        let cookie_path = std::path::Path::new(&app_data)
            .join("com.local.bili-collector")
            .join("bilibili_cookie.txt");
        let cookie = std::fs::read_to_string(&cookie_path)
            .expect("读取 bilibili_cookie.txt 失败（请先在 app 内登录）");
        let cookie = cookie.trim().to_string();

        let client = BilibiliClient::new().expect("BilibiliClient::new 失败");
        client.set_cookie(Some(cookie));

        // 2) 列出收藏夹，挑 count 最大的普通视频收藏夹
        let collections = client.list_own_favorites().await.expect("列出收藏夹失败");
        assert!(!collections.is_empty(), "当前账号没有收藏夹");
        eprintln!("[bench] 收藏夹列表（按条数降序）：");
        let mut sorted = collections.clone();
        sorted.sort_by(|a, b| b.count.cmp(&a.count));
        for c in &sorted {
            eprintln!("[bench]   - id={} count={} title={:?}", c.id, c.count, c.title);
        }
        let target = sorted
            .iter()
            .find(|c| c.count > 0)
            .expect("没有非空收藏夹")
            .clone();
        eprintln!(
            "[bench] 选中测试收藏夹：id={} count={} title={:?}",
            target.id, target.count, target.title
        );

        // 3) fetch_collection（分页拉取）
        let t0 = Instant::now();
        let items = client
            .fetch_collection(&target)
            .await
            .expect("fetch_collection 失败");
        let fetch_dur = t0.elapsed();
        eprintln!(
            "[bench] fetch_collection: {} 条, 用时 {:?}",
            items.len(),
            fetch_dur
        );

        // 4) enrich_items（优化后：有界并发 ENRICH_CONCURRENCY=6，无 180ms 硬等）
        let t1 = Instant::now();
        let _enriched = client
            .enrich_items(&items)
            .await
            .expect("enrich_items 失败");
        let enrich_dur = t1.elapsed();
        let bv_count = items
            .iter()
            .filter(|i| i.external_id.starts_with("BV"))
            .count();
        eprintln!(
            "[bench] enrich_items: 视频项 {}（总 {}）, 用时 {:?}",
            bv_count,
            items.len(),
            enrich_dur
        );

        // 5) 封面下载（并发 COVER_CONCURRENCY=8，保留离线 covers/ 行为）
        let cover_items: Vec<_> = _enriched
            .iter()
            .filter(|i| i.cover_url.as_deref().filter(|u| !u.is_empty()).is_some())
            .collect();
        let t2 = Instant::now();
        let mut downloaded = 0u32;
        for chunk in cover_items.chunks(8) {
            let futs: Vec<_> = chunk
                .iter()
                .map(|item| async {
                    client
                        .download_cover(item.cover_url.as_deref().unwrap())
                        .await
                        .is_ok()
                })
                .collect();
            for ok in join_all(futs).await {
                if ok {
                    downloaded += 1;
                }
            }
        }
        let cover_dur = t2.elapsed();
        eprintln!(
            "[bench] 封面下载：成功 {} / 需下载 {}, 用时 {:?}",
            downloaded,
            cover_items.len(),
            cover_dur
        );

        let total = fetch_dur + enrich_dur + cover_dur;
        eprintln!(
            "[bench] 优化后单趟（fetch+enrich+cover）合计 {:?}（{} 条）",
            total,
            items.len()
        );

        // 6) 串行旧版对照：取前 50 个视频项，串行 + 每条约 180ms 硬等，测真实耗时并外推
        let sample: Vec<_> = items
            .iter()
            .filter(|i| i.external_id.starts_with("BV"))
            .take(50)
            .collect();
        let t3 = Instant::now();
        for item in &sample {
            let _ = client
                .get_json(
                    "https://api.bilibili.com/x/web-interface/view",
                    vec![("bvid".into(), item.external_id.clone())],
                    true,
                )
                .await;
            tokio::time::sleep(std::time::Duration::from_millis(180)).await;
        }
        let serial_sample_dur = t3.elapsed();
        let per_item_serial = serial_sample_dur / sample.len() as u32;
        let old_enrich_est = per_item_serial * bv_count as u32;
        eprintln!(
            "[bench] 串行旧版样本：{} 条用时 {:?} → 单项 ~{:?}；外推全量 {} 视频项约 {:?}",
            sample.len(),
            serial_sample_dur,
            per_item_serial,
            bv_count,
            old_enrich_est
        );

        eprintln!(
            "[bench] 结论：优化后 enrich 单项约 {:?}（并发6，无硬等），旧版约 {:?}（含180ms硬等 + 串行）；\
             满收藏夹经缓存复用后 execute 阶段可省掉一整趟 fetch+enrich（约 {:?}）",
            enrich_dur / bv_count.max(1) as u32,
            per_item_serial,
            fetch_dur + enrich_dur
        );
    }
}
