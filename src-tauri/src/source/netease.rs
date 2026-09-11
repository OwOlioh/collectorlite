//! 网易云音乐来源。
//!
//! - **P0 深链打开**（已实施 2026-09-11）：`song_play_uri()` / `open_song_in_client()`。
//! - **P2 歌单导入**（已实施）：weapi 加密拉取歌单与曲目，见下方 `SourceAdapter` 实现。
//! - **P2 增量同步**：水位取 `trackIds[].at`（加入歌单时间，整表严格倒序），见 `DEVELOPMENT.md` 9.7。
//!
//! 所有接口行为都是 2026-09-11 实测出来的，坑与理由见 `DEVELOPMENT.md` 第九章。

use async_trait::async_trait;
use base64::Engine;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE, COOKIE, REFERER, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::RwLock;
use std::time::Duration;

use crate::error::AppError;
use crate::models::{CollectionInfo, ExternalItem};
use crate::source::SourceAdapter;
use crate::uri::open_uri_system;
use crate::weapi;

const BASE_URL: &str = "https://music.163.com";
const USER_AGENT_STR: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

/// `song/detail` 一次传超过 201 个 id 会**静默截断成 201**（不报错、不提示），
/// 所以分块大小固定 200。这个要是改大，导入会静默丢数据。
const SONG_DETAIL_CHUNK: usize = 200;

pub struct NeteaseClient {
    http: reqwest::Client,
    cookie: RwLock<Option<String>>,
    /// 用户 uid。`user/playlist` 必须传 uid，传空串会 400；查到后缓存，cookie 变更时清空。
    uid: RwLock<Option<String>>,
    /// 昵称，仅用于设置页展示。
    nickname: RwLock<Option<String>>,
}

impl NeteaseClient {
    pub fn new() -> Result<Self, AppError> {
        let mut builder = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::limited(5));
        // 与 B站同款：复用统一代理解析，保证不同启动方式都能出网（直连易被风控）。
        if let Some(proxy) = crate::source::proxy::resolve_system_proxy() {
            builder = builder.proxy(proxy);
        }
        let http = builder.build()?;
        Ok(Self {
            http,
            cookie: RwLock::new(None),
            uid: RwLock::new(None),
            nickname: RwLock::new(None),
        })
    }

    pub fn set_cookie(&self, cookie: Option<String>) {
        if let Ok(mut guard) = self.cookie.write() {
            *guard = cookie;
        }
        // 换账号后 uid / 昵称必须重查，否则会一直拿上一个账号的歌单
        if let Ok(mut guard) = self.uid.write() {
            *guard = None;
        }
        if let Ok(mut guard) = self.nickname.write() {
            *guard = None;
        }
    }

    pub fn cookie_value(&self) -> Option<String> {
        self.cookie.read().ok().and_then(|guard| guard.clone())
    }

    /// 登录态校验 + 账号信息（uid、昵称），结果缓存。
    /// cookie 无效 / 过期时返回明确错误，供前端提示重新粘贴。
    pub async fn account_info(&self) -> Result<(String, Option<String>), AppError> {
        if let Some(uid) = self.uid.read().ok().and_then(|g| g.clone()) {
            let nickname = self.nickname.read().ok().and_then(|g| g.clone());
            return Ok((uid, nickname));
        }
        let res = self
            .weapi_post("/weapi/w/nuser/account/get", json!({ "csrf_token": "" }))
            .await?;
        let uid = res["profile"]["userId"]
            .as_i64()
            .or_else(|| res["account"]["id"].as_i64())
            .map(|v| v.to_string())
            .ok_or_else(|| {
                AppError::Other("网易云登录态无效（拿不到 uid），请重新粘贴 cookie".into())
            })?;
        let nickname = res["profile"]["nickname"]
            .as_str()
            .map(|s| s.to_string());
        if let Ok(mut guard) = self.uid.write() {
            *guard = Some(uid.clone());
        }
        if let Ok(mut guard) = self.nickname.write() {
            *guard = nickname.clone();
        }
        Ok((uid, nickname))
    }

    /// 只取 uid（拉歌单必须用它）。
    pub async fn ensure_uid(&self) -> Result<String, AppError> {
        Ok(self.account_info().await?.0)
    }

    async fn weapi_post(&self, path: &str, payload: Value) -> Result<Value, AppError> {
        let body = weapi::weapi_form(&payload);
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_STR));
        headers.insert(REFERER, HeaderValue::from_static("https://music.163.com/"));
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        );
        if let Some(cookie) = self.cookie_value() {
            if let Ok(value) = HeaderValue::from_str(&cookie) {
                headers.insert(COOKIE, value);
            }
        }

        let url = format!("{BASE_URL}{path}");
        let response = self
            .http
            .post(&url)
            .headers(headers)
            .body(body)
            .send()
            .await?;
        let text = response.text().await?;
        let json: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        let code = json["code"].as_i64().unwrap_or(-1);
        if code != 200 {
            // 能解出结构化 JSON 说明加密是对的，非 200 是业务规则（登录态 / 风控 / 参数）
            return Err(AppError::Other(format!(
                "网易云接口 {path} 返回 code={code}{}",
                json["message"]
                    .as_str()
                    .map(|m| format!("（{m}）"))
                    .unwrap_or_default()
            )));
        }
        Ok(json)
    }

    /// 拉歌单的全部 `trackIds`（含 `at` 加入时间）。实测一次可拿全（2934 首约 1.7 s），
    /// 但仍按分页写，避免大歌单被截断时静默丢数据。
    ///
    /// 返回的 `at` 是**毫秒**。
    pub async fn fetch_track_ids(
        &self,
        playlist_id: &str,
    ) -> Result<Vec<(i64, Option<i64>)>, AppError> {
        let mut all: Vec<(i64, Option<i64>)> = Vec::new();
        let mut offset = 0i64;

        loop {
            let res = self
                .weapi_post(
                    "/weapi/v6/playlist/detail",
                    json!({
                        "id": playlist_id,
                        "n": 1000,
                        "offset": offset,
                        "total": true,
                        "csrf_token": ""
                    }),
                )
                .await?;
            let playlist = &res["playlist"];
            let tracks = match playlist["trackIds"].as_array() {
                Some(arr) if !arr.is_empty() => arr,
                _ => break,
            };
            for track in tracks {
                if let Some(id) = track["id"].as_i64() {
                    all.push((id, track["at"].as_i64()));
                }
            }
            let total = playlist["trackCount"].as_i64().unwrap_or(0);
            offset += tracks.len() as i64;
            // 三个退出条件：拿够了 / 这一页不满（说明到底了）/ 接口不再返回
            if total > 0 && all.len() as i64 >= total {
                break;
            }
            if (tracks.len() as i64) < 1000 {
                break;
            }
        }

        Ok(all)
    }

    /// 按给定 id 列表抓曲目详情。内部按 `SONG_DETAIL_CHUNK`(200) 分块——
    /// 一次传超过 201 个 id 接口会**静默截断**，不报错。
    ///
    /// - `added_at`：id → `at`（毫秒，来自 `trackIds`），用来回填收藏时间；
    ///   `song/detail` 本身不返回收藏时间。查不到就是 None。
    /// - `playlist_id`：写进每条的 `extra.playlistId`，增量同步取消收藏时要用。
    pub async fn fetch_songs(
        &self,
        ids: &[i64],
        added_at: &HashMap<i64, Option<i64>>,
        playlist_id: &str,
    ) -> Result<Vec<ExternalItem>, AppError> {
        let mut items: Vec<ExternalItem> = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(SONG_DETAIL_CHUNK) {
            let ids_json = serde_json::to_string(
                &chunk.iter().map(|id| json!(id)).collect::<Vec<Value>>(),
            )
            .map_err(|e| AppError::Other(format!("序列化歌曲 id 失败：{e}")))?;
            let res = self
                .weapi_post(
                    "/weapi/song/detail",
                    json!({ "ids": ids_json, "csrf_token": "" }),
                )
                .await?;
            let songs = res["songs"].as_array();
            for song in songs.into_iter().flatten() {
                if let Some(id) = song["id"].as_i64() {
                    // at 是毫秒，库里存秒
                    let favorite_time = added_at
                        .get(&id)
                        .copied()
                        .flatten()
                        .map(|ms| ms / 1000);
                    if let Some(mut item) = song_to_item(song, favorite_time) {
                        item.extra =
                            json!({ "kind": "song", "playlistId": playlist_id });
                        items.push(item);
                    }
                }
            }
        }
        Ok(items)
    }
}

/// 从「歌单链接」或「纯数字 id」里取出歌单 id。
/// 支持 `https://music.163.com/#/playlist?id=123&userid=456` 与 `123` 两种写法。
fn extract_playlist_id(input: &str) -> Option<String> {
    let text = input.trim();
    if !text.is_empty() && text.chars().all(|c| c.is_ascii_digit()) {
        return Some(text.to_string());
    }
    for part in text.split(['?', '&', '#']) {
        if let Some(rest) = part.strip_prefix("id=") {
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if !digits.is_empty() {
                return Some(digits);
            }
        }
    }
    None
}

/// 把 `song/detail` 返回的一首歌转成统一中间结构。
///
/// ⚠️ `duration` 是毫秒，这里除 1000 转成库里统一的秒。
/// ⚠️ `favorite_time` **入参已经是秒** —— 从 `trackIds[].at`（毫秒）到秒的转换在
/// 调用方 `fetch_collection` 里完成，这里不要二次转换。
fn song_to_item(song: &Value, favorite_time: Option<i64>) -> Option<ExternalItem> {
    let id = song["id"].as_i64()?;
    let title = song["name"].as_str().unwrap_or("未知曲目").to_string();
    let artists: Vec<&str> = song["artists"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|a| a["name"].as_str()).collect())
        .unwrap_or_default();
    let album = &song["album"];
    let album_name = album["name"].as_str().unwrap_or("");

    Some(ExternalItem {
        source: "netease".into(),
        external_id: id.to_string(),
        source_url: format!("{BASE_URL}/#/song?id={id}"),
        title,
        description: album_name.to_string(),
        cover_url: album["picUrl"].as_str().map(|s| s.to_string()),
        cover_local_path: None,
        author_name: if artists.is_empty() {
            None
        } else {
            Some(artists.join("/"))
        },
        author_id: song["artists"]
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|a| a["id"].as_i64())
            .map(|v| v.to_string()),
        partition_name: None,
        published_at: album["publishTime"].as_i64().map(|ms| ms / 1000),
        duration: song["duration"].as_i64().map(|ms| ms / 1000),
        favorite_time,
        extra: json!({ "kind": "song" }),
    })
}

#[async_trait]
impl SourceAdapter for NeteaseClient {
    async fn list_collections(&self) -> Result<Vec<CollectionInfo>, AppError> {
        let uid = self.ensure_uid().await?;
        let res = self
            .weapi_post(
                "/weapi/user/playlist",
                json!({ "uid": uid, "limit": 1000, "offset": 0, "csrf_token": "" }),
            )
            .await?;
        let playlists = res["playlist"]
            .as_array()
            .ok_or_else(|| AppError::Other("网易云未返回歌单列表".into()))?;

        Ok(playlists
            .iter()
            .filter_map(|p| {
                let id = p["id"].as_i64()?.to_string();
                Some(CollectionInfo {
                    source: "netease".into(),
                    id: id.clone(),
                    title: p["name"].as_str().unwrap_or("未命名歌单").to_string(),
                    owner: p["creator"]["nickname"].as_str().map(|s| s.to_string()),
                    count: p["trackCount"].as_i64().unwrap_or(0),
                    url: Some(format!("{BASE_URL}/#/playlist?id={id}")),
                })
            })
            .collect())
    }

    async fn resolve_collection(&self, input: &str) -> Result<CollectionInfo, AppError> {
        let id = extract_playlist_id(input)
            .ok_or_else(|| AppError::InvalidInput("无法从输入中解析出歌单 id".into()))?;
        let res = self
            .weapi_post(
                "/weapi/v6/playlist/detail",
                json!({ "id": id, "n": 1, "offset": 0, "total": true, "csrf_token": "" }),
            )
            .await?;
        let playlist = &res["playlist"];
        if playlist.is_null() {
            return Err(AppError::Other(
                "歌单不存在，或该歌单需要登录才能访问".into(),
            ));
        }
        Ok(CollectionInfo {
            source: "netease".into(),
            id: id.clone(),
            title: playlist["name"].as_str().unwrap_or("未命名歌单").to_string(),
            owner: playlist["creator"]["nickname"]
                .as_str()
                .map(|s| s.to_string()),
            count: playlist["trackCount"].as_i64().unwrap_or(0),
            url: Some(format!("{BASE_URL}/#/playlist?id={id}")),
        })
    }

    async fn fetch_collection(
        &self,
        collection: &CollectionInfo,
    ) -> Result<Vec<ExternalItem>, AppError> {
        let tracks = self.fetch_track_ids(&collection.id).await?;
        // id → 加入时间（毫秒）。song/detail 不返回收藏时间，必须靠这张表回填。
        let added_at: HashMap<i64, Option<i64>> = tracks.iter().copied().collect();
        let ids: Vec<i64> = tracks.into_iter().map(|(id, _)| id).collect();
        self.fetch_songs(&ids, &added_at, &collection.id).await
    }

    async fn enrich_items(&self, items: &[ExternalItem]) -> Result<Vec<ExternalItem>, AppError> {
        // song/detail 已经给了曲名 / 歌手 / 专辑 / 封面 / 时长，无需二次补全
        Ok(items.to_vec())
    }
}

/// 构造唤起桌面客户端**并播放**指定歌曲的深链。
///
/// ⚠️ 实测（2026-09-11）三种写法里**只有这一种确认可用**：
/// - `orpheus://<base64 json>` + `cmd:"play"` ✅ 标题确实切到目标曲目并开始播放
/// - `orpheus://song/{id}` ⚠️ 未观察到跳转（早期误判为可用，实际目标恰好是当时在播的歌）
/// - `orpheus://openurl?url=...` ❌ 编码/不编码都无任何反应
///
/// 另：带 `cmd:"play"` 意味着**会替换掉用户当前正在播放的歌**——这是用户拍板接受的
/// 行为（DEVELOPMENT.md 9.11-2），不要擅自去掉。
pub fn song_play_uri(song_id: &str) -> String {
    let payload = format!(
        r#"{{"type":"song","id":"{}","cmd":"play"}}"#,
        song_id.replace('"', "")
    );
    let encoded = base64::engine::general_purpose::STANDARD.encode(payload.as_bytes());
    format!("orpheus://{encoded}")
}

/// 在网易云桌面客户端打开并播放该歌曲。
///
/// 返回 `Ok` **只代表请求已递交给系统**，不代表客户端真的处理了（见 `uri::open_uri_system` 注释）。
pub fn open_song_in_client(song_id: &str) -> Result<(), AppError> {
    open_uri_system(&song_play_uri(song_id))
}

const SYNC_SETTINGS_FILE: &str = "netease_sync.json";
/// 同步最小间隔（分钟）。网易云对频繁请求敏感，低于这个值容易触发风控（8821 / -462）。
pub const MIN_SYNC_INTERVAL_MINUTES: u32 = 5;

/// 网易云增量同步配置（存 app data 目录下的 `netease_sync.json`）。
///
/// 水位（`waterMarks`）= 上次同步时该歌单最新一条 `at`（加入歌单时间）的秒值。
/// 因为 `trackIds` **严格倒序**（实测 2934 条零违例），下次同步从头部扫、遇到 `at`
/// 不比水位新就能停 —— 日常一次同步往往只要 1~2 个请求。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_interval")]
    pub interval_minutes: u32,
    /// 取消收藏的歌是否自动软删除进回收站（用户 2026-09-11 拍板：默认开，可关）
    #[serde(default = "default_true")]
    pub auto_remove_unfavorited: bool,
    /// 参与自动同步的歌单 id（导入过的歌单会自动登记）
    #[serde(default)]
    pub playlist_ids: Vec<String>,
    /// 歌单 id → 水位（秒）
    #[serde(default)]
    pub water_marks: HashMap<String, i64>,
    #[serde(default)]
    pub last_sync_at: Option<i64>,
}

fn default_true() -> bool {
    true
}

fn default_interval() -> u32 {
    15
}

impl Default for SyncSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_minutes: 15,
            auto_remove_unfavorited: true,
            playlist_ids: Vec::new(),
            water_marks: HashMap::new(),
            last_sync_at: None,
        }
    }
}

pub fn load_sync_settings(data_dir: &Path) -> SyncSettings {
    let path = data_dir.join(SYNC_SETTINGS_FILE);
    match fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => SyncSettings::default(),
    }
}

pub fn save_sync_settings(data_dir: &Path, settings: &SyncSettings) -> Result<(), AppError> {
    let path = data_dir.join(SYNC_SETTINGS_FILE);
    let text = serde_json::to_string_pretty(settings)
        .map_err(|e| AppError::Other(format!("序列化网易云同步配置失败：{e}")))?;
    fs::write(path, text).map_err(AppError::Io)
}

/// 把歌单登记进自动同步范围（导入成功后调用）。
///
/// `water_mark` 传本次导入里最大的 `favorite_time`（即 `trackIds[].at`）：
/// 不传的话，首次同步会因为水位为空而只立水位、不抓歌，
/// 「导入之后、首次同步之前」新收藏的歌就漏掉了。
/// 已登记过则只在水位更大时前推，**不会**把水位往回拉。
pub fn register_playlist_for_sync(
    data_dir: &Path,
    playlist_id: &str,
    water_mark: Option<i64>,
) -> Result<SyncSettings, AppError> {
    let mut settings = load_sync_settings(data_dir);
    if !settings.playlist_ids.iter().any(|p| p == playlist_id) {
        settings.playlist_ids.push(playlist_id.to_string());
    }
    if let Some(mark) = water_mark {
        let entry = settings
            .water_marks
            .entry(playlist_id.to_string())
            .or_insert(mark);
        *entry = (*entry).max(mark);
    }
    save_sync_settings(data_dir, &settings)?;
    Ok(settings)
}

/// 清掉已不在同步范围内的歌单水位，避免无限堆积成脏数据。
/// 前端改完 `playlistIds` 保存时会调用。
pub fn prune_stale_water_marks(settings: &mut SyncSettings) {
    let active: HashSet<String> = settings.playlist_ids.iter().cloned().collect();
    settings
        .water_marks
        .retain(|playlist_id, _| active.contains(playlist_id));
}

// ── 增量同步的纯函数（不碰网络、不碰数据库，方便单测钉死） ──────────────────

/// 一次歌单同步的计划：要抓哪些新歌、同步完水位该是多少。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncPlan {
    /// 需要抓详情的曲目 id（按加入时间从新到旧）
    pub new_ids: Vec<i64>,
    /// 同步完成后应写入的水位（秒）
    pub new_water_mark: Option<i64>,
}

/// 依据水位挑出新增曲目。
///
/// ⚠️ 依赖 `trackIds` **严格按 `at` 倒序**这个前提（实测 2934 条零违例）：
/// 从头部扫，遇到第一条不比水位新的就能停 —— 日常一次同步常常只要
/// 1 个请求就能确认「没有新歌」。哪天网易云改了排序，这里会漏歌，
/// 所以 `new_water_mark` 用全表最大值而不是「扫到的最后一个」。
///
/// 水位为 `None`（歌单刚登记、还没同步过）时**不回填历史**：只立水位，
/// 本次 `new_ids` 为空。历史曲目在导入那一步已经进过库了。
pub fn plan_incremental(
    tracks: &[(i64, Option<i64>)],
    water_mark: Option<i64>,
) -> SyncPlan {
    // 全表最大值，空歌单或全部缺 at 时沿用旧水位，避免水位倒退
    let new_water_mark = tracks
        .iter()
        .filter_map(|(_, at)| at.map(|ms| ms / 1000))
        .max()
        .or(water_mark);

    let mut new_ids = Vec::new();
    if let Some(mark) = water_mark {
        for (id, at) in tracks {
            let at_seconds = match at {
                Some(ms) => ms / 1000,
                // 缺 at 无法比较，保守跳过（不重复入库，代价是这首歌暂时同步不到）
                None => continue,
            };
            if at_seconds > mark {
                new_ids.push(*id);
            } else {
                break;
            }
        }
    }

    SyncPlan {
        new_ids,
        new_water_mark,
    }
}

/// 从 `extra_json` 里取出 `playlistId`（导入时写进去的归属歌单）。
/// 老数据 / 手工导入的条目没有这个字段 → 返回 None，同步不会碰它们。
pub fn extra_playlist_id(extra_json: &str) -> Option<String> {
    let value: Value = serde_json::from_str(extra_json).ok()?;
    match value["playlistId"].clone() {
        Value::String(s) => Some(s),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// 判断库里哪些条目已经被用户取消收藏、该移进回收站。
///
/// - `rows`：库内未删除的 netease 条目 `(item_id, external_id, 归属歌单)`
/// - `synced`：当前参与同步的歌单 id
/// - `remote_ids`：这些歌单**此刻**云端的曲目 id 并集
///
/// 两道闸门，少一道就会误删：
/// 1. 只考虑归属歌单在同步范围内的条目（用户手工导入、没开同步的歌单一概不碰）；
/// 2. 只要这首歌还在**任意一个**同步歌单里就不删 —— 同一首歌可以同时属于
///    A、B 两个歌单，而库里只有一行（`(source, external_id)` 复合键去重），
///    从 B 里移除不代表它该消失。
pub fn pick_unfavorited(
    rows: &[(i64, String, Option<String>)],
    synced: &HashSet<String>,
    remote_ids: &HashSet<String>,
) -> Vec<i64> {
    rows.iter()
        .filter(|(_, _, playlist)| {
            playlist
                .as_ref()
                .map(|p| synced.contains(p))
                .unwrap_or(false)
        })
        .filter(|(_, external_id, _)| !remote_ids.contains(external_id))
        .map(|(item_id, _, _)| *item_id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── 深链 ────────────────────────────────────────────────────────────

    // 钉死 payload 格式：这是唯一被实测确认可用的深链写法，改动等于破坏功能
    #[test]
    fn song_play_uri_matches_verified_vector() {
        assert_eq!(
            song_play_uri("2709782550"),
            "orpheus://eyJ0eXBlIjoic29uZyIsImlkIjoiMjcwOTc4MjU1MCIsImNtZCI6InBsYXkifQ=="
        );
        assert_eq!(
            song_play_uri("447926067"),
            "orpheus://eyJ0eXBlIjoic29uZyIsImlkIjoiNDQ3OTI2MDY3IiwiY21kIjoicGxheSJ9"
        );
    }

    #[test]
    fn song_play_uri_decodes_to_expected_json() {
        let uri = song_play_uri("123");
        let b64 = uri.strip_prefix("orpheus://").expect("应有 orpheus:// 前缀");
        let raw = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .expect("应为合法 base64");
        let json = String::from_utf8(raw).expect("应为 UTF-8");
        assert_eq!(json, r#"{"type":"song","id":"123","cmd":"play"}"#);
    }

    #[test]
    fn song_play_uri_strips_quotes() {
        assert_eq!(song_play_uri("123"), song_play_uri(r#"12"3"#));
    }

    // ── 歌单 id 解析 ────────────────────────────────────────────────────

    #[test]
    fn extract_playlist_id_accepts_url_and_digits() {
        assert_eq!(
            extract_playlist_id("https://music.163.com/#/playlist?id=5124170445&userid=3418238560"),
            Some("5124170445".to_string())
        );
        assert_eq!(
            extract_playlist_id("https://music.163.com/#/playlist?id=123"),
            Some("123".to_string())
        );
        assert_eq!(extract_playlist_id("  5124170445  "), Some("5124170445".to_string()));
        assert_eq!(extract_playlist_id("不是链接也不是数字"), None);
    }

    // ── 曲目字段映射 ────────────────────────────────────────────────────

    // 真实的 song/detail 片段（字段做了精简，保留我们用到的全部）
    fn sample_song() -> Value {
        json!({
            "id": 2709782550i64,
            "name": "下等马",
            "duration": 186944i64,
            "artists": [
                { "id": 906118i64, "name": "洛天依Official" },
                { "id": 32101385i64, "name": "ChiliChill乐团" }
            ],
            "album": {
                "name": "闪耀",
                "picUrl": "https://p2.music.126.net/cVnYnnHwjXj9xN5ymz6dsw==/109951172051500248.jpg",
                "publishTime": 1751328000000i64
            }
        })
    }

    #[test]
    fn song_to_item_maps_fields_and_converts_ms_to_seconds() {
        // favorite_time 入参已经是秒（毫秒→秒的转换在 fetch_collection 里做）
        let item = song_to_item(&sample_song(), Some(1757570000i64)).expect("应解析成功");
        assert_eq!(item.source, "netease");
        assert_eq!(item.external_id, "2709782550");
        assert_eq!(item.title, "下等马");
        // 毫秒 → 秒（整数截断，非四舍五入）：186944ms = 186s。duration 在库里统一存秒
        assert_eq!(item.duration, Some(186));
        assert_eq!(item.favorite_time, Some(1757570000));
        assert_eq!(item.published_at, Some(1751328000));
        assert_eq!(item.author_name.as_deref(), Some("洛天依Official/ChiliChill乐团"));
        assert_eq!(item.author_id.as_deref(), Some("906118"));
        assert_eq!(item.description, "闪耀");
        assert!(item.cover_url.as_deref().unwrap().contains("109951172051500248"));
        assert!(item.source_url.ends_with("#/song?id=2709782550"));
    }

    #[test]
    fn song_to_item_without_id_returns_none() {
        assert!(song_to_item(&json!({ "name": "缺 id" }), None).is_none());
    }

    #[test]
    fn chunk_size_stays_below_silent_truncation_limit() {
        // 接口超过 201 个 id 会静默截断，200 是实测安全值
        assert!(SONG_DETAIL_CHUNK <= 200, "分块不能大于 200，否则静默丢数据");
    }

    // ── 增量水位 ────────────────────────────────────────────────────────
    //
    // 时间戳用「相对量」写，避免写死具体日期后看起来像魔法数字。
    // 单位：入参毫秒（接口原样），水位 / 输出秒（库里统一）。

    const HOUR_MS: i64 = 3_600_000;

    // 倒序的 trackIds：最新在前
    fn descending_tracks() -> Vec<(i64, Option<i64>)> {
        vec![
            (300, Some(300 * HOUR_MS)),
            (200, Some(200 * HOUR_MS)),
            (100, Some(100 * HOUR_MS)),
        ]
    }

    #[test]
    fn plan_incremental_picks_only_newer_than_water_mark() {
        // 水位 150h：300h 和 200h 都比它新，100h 是旧的 → 只收前两条
        let plan = plan_incremental(&descending_tracks(), Some(150 * HOUR_MS / 1000));
        assert_eq!(plan.new_ids, vec![300, 200]);
        // 水位取全表最大值（300h），不是「扫到的最后一个」
        assert_eq!(plan.new_water_mark, Some(300 * HOUR_MS / 1000));
    }

    #[test]
    fn plan_incremental_nothing_new_keeps_water_mark() {
        let plan = plan_incremental(&descending_tracks(), Some(999 * HOUR_MS / 1000));
        assert!(plan.new_ids.is_empty());
        assert_eq!(plan.new_water_mark, Some(300 * HOUR_MS / 1000));
    }

    #[test]
    fn plan_incremental_without_water_mark_only_sets_it() {
        // 歌单刚登记：只立水位、不回填历史（历史在导入时已进库）
        let plan = plan_incremental(&descending_tracks(), None);
        assert!(plan.new_ids.is_empty());
        assert_eq!(plan.new_water_mark, Some(300 * HOUR_MS / 1000));
    }

    #[test]
    fn plan_incremental_empty_playlist_never_rewinds_water_mark() {
        // 空歌单（或全缺 at）时沿用旧水位：否则下次同步会把整张歌单当新歌重抓
        let plan = plan_incremental(&[], Some(42));
        assert!(plan.new_ids.is_empty());
        assert_eq!(plan.new_water_mark, Some(42));
    }

    #[test]
    fn plan_incremental_skips_tracks_without_at() {
        let tracks = vec![(300, None), (200, Some(200 * HOUR_MS))];
        let plan = plan_incremental(&tracks, Some(100 * HOUR_MS / 1000));
        // 缺 at 的保守跳过（不重复入库）
        assert_eq!(plan.new_ids, vec![200]);
        assert_eq!(plan.new_water_mark, Some(200 * HOUR_MS / 1000));
    }

    // ── 同步配置持久化 ──────────────────────────────────────────────────

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("netease_sync_test_{}_{}", std::process::id(), tag));
        path
    }

    #[test]
    fn register_playlist_for_sync_never_rewinds_water_mark() {
        let dir = temp_dir("rewind");
        let _ = fs::create_dir_all(&dir);
        let settings = register_playlist_for_sync(&dir, "A", Some(1000)).expect("应写入成功");
        assert_eq!(settings.playlist_ids, vec!["A".to_string()]);
        assert_eq!(settings.water_marks.get("A"), Some(&1000));

        // 重复导入同一个歌单：水位只会前推，不会倒退
        let settings = register_playlist_for_sync(&dir, "A", Some(500)).expect("应写入成功");
        assert_eq!(settings.water_marks.get("A"), Some(&1000));
        // 也不会重复登记
        assert_eq!(settings.playlist_ids, vec!["A".to_string()]);

        let settings = register_playlist_for_sync(&dir, "A", Some(2000)).expect("应写入成功");
        assert_eq!(settings.water_marks.get("A"), Some(&2000));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn register_playlist_for_sync_without_water_mark_still_registers() {
        let dir = temp_dir("nomark");
        let _ = fs::create_dir_all(&dir);
        let settings = register_playlist_for_sync(&dir, "B", None).expect("应写入成功");
        assert_eq!(settings.playlist_ids, vec!["B".to_string()]);
        assert!(!settings.water_marks.contains_key("B"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_stale_water_marks_drops_removed_playlists() {
        let mut settings = SyncSettings::default();
        settings.playlist_ids = vec!["A".to_string()];
        settings.water_marks = [
            ("A".to_string(), 100i64),
            ("B".to_string(), 200i64),
        ]
        .into_iter()
        .collect();
        prune_stale_water_marks(&mut settings);
        assert_eq!(settings.water_marks.len(), 1);
        assert_eq!(settings.water_marks.get("A"), Some(&100));
    }

    // ── 取消收藏判定 ────────────────────────────────────────────────────

    #[test]
    fn extra_playlist_id_reads_string_and_number() {
        assert_eq!(
            extra_playlist_id(r#"{"kind":"song","playlistId":"5124170445"}"#),
            Some("5124170445".to_string())
        );
        // 导入时若以数字写入，也要能读出来
        assert_eq!(
            extra_playlist_id(r#"{"playlistId":5124170445}"#),
            Some("5124170445".to_string())
        );
        assert_eq!(extra_playlist_id(r#"{"kind":"song"}"#), None);
        assert_eq!(extra_playlist_id("不是 json"), None);
    }

    #[test]
    fn pick_unfavorited_only_touches_synced_playlists() {
        let rows = vec![
            (1, "100".to_string(), Some("A".to_string())),
            (2, "200".to_string(), Some("B".to_string())), // B 不在同步范围
            (3, "300".to_string(), None),                  // 手工导入，无归属
        ];
        let synced: HashSet<String> = ["A".to_string()].into_iter().collect();
        // 云端 A 里只剩 100；200 / 300 虽然在库里，但不归我们管
        let remote: HashSet<String> = ["100".to_string()].into_iter().collect();
        assert!(pick_unfavorited(&rows, &synced, &remote).is_empty());
    }

    #[test]
    fn pick_unfavorited_deletes_when_gone_from_all_synced_playlists() {
        let rows = vec![
            (1, "100".to_string(), Some("A".to_string())),
            (2, "200".to_string(), Some("A".to_string())),
        ];
        let synced: HashSet<String> = ["A".to_string()].into_iter().collect();
        let remote: HashSet<String> = ["100".to_string()].into_iter().collect();
        assert_eq!(pick_unfavorited(&rows, &synced, &remote), vec![2]);
    }

    #[test]
    fn pick_unfavorited_keeps_song_still_in_another_synced_playlist() {
        // 同一首歌在 A、B 两个歌单里，库里只有一行（归属记成 B）。
        // 用户把它从 B 移除 → 不能删，因为 A 里还有。
        let rows = vec![(7, "555".to_string(), Some("B".to_string()))];
        let synced: HashSet<String> =
            ["A".to_string(), "B".to_string()].into_iter().collect();
        // 并集里还有它（来自 A）
        let remote: HashSet<String> = ["555".to_string()].into_iter().collect();
        assert!(pick_unfavorited(&rows, &synced, &remote).is_empty());
    }
}
