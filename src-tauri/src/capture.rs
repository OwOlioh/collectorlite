//! 浏览器扩展「快速入库」的本地桥服务。
//!
//! 只监听 127.0.0.1，所有请求必须带 token，防止本地任意进程或任意网页往库里塞数据
//! （本地 CSRF 与 DNS rebinding）。请求**直接写库**，不经过前端，因此 app 窗口是否
//! 存活都不影响入库；写完之后 emit 事件通知前端刷新列表。

use std::io::{Cursor, Read};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, Manager};
use tiny_http::{Header, Method, Request, Response, Server};

use crate::db;
use crate::error::AppError;
use crate::models::{ExternalItem, Tag, TagInput};
use crate::source::browser::BrowserBookmarkClient;
use crate::source::SourceAdapter;
use crate::state::AppState;

/// 桥监听的基端口，被占用时依次顺延到 `MAX_PORT`。
pub const BASE_PORT: u16 = 17820;
pub const MAX_PORT: u16 = 17829;

/// 入库成功后推给前端的事件名。
pub const CAPTURE_EVENT: &str = "capture://saved";

const TOKEN_HEADER: &str = "x-bridge-token";
const TOKEN_FILE: &str = "bridge_token.txt";
/// 请求体上界，超过直接截断（JSON 解析自然失败），防止恶意大 body 占内存。
const MAX_BODY_BYTES: usize = 64 * 1024;

// ── 请求 / 响应结构 ──

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CaptureRequest {
    url: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    note: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    description: String,
    /// 扩展从页面读到的 og:image（干净封面），用于知乎等「app 侧 API 抓不到」的站点兜底。
    #[serde(default)]
    og_image: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CaptureResponse {
    ok: bool,
    item_id: i64,
    created: bool,
    tags: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PingResponse {
    ok: bool,
    app: String,
    port: u16,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TagsResponse {
    ok: bool,
    tags: Vec<Tag>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ItemLookupResponse {
    ok: bool,
    #[serde(rename = "exists")]
    exists: bool,
    item: Option<SavedItemSummary>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SavedItemSummary {
    title: String,
    notes: String,
    tags: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorResponse {
    ok: bool,
    error: String,
}

/// 推给前端的事件载荷。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSavedPayload {
    pub item_id: i64,
    pub title: String,
    pub created: bool,
}

// ── 对外接口 ──

/// 与浏览器书签导入共用同一套键：`bk_<sha256(url) 前 16 位>`。
/// 这样「快速入库过的页面，之后再导书签」不会变成两条。
pub fn external_id_for_url(url: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(url.as_bytes());
    let hash = format!("{:x}", hasher.finalize());
    format!("bk_{}", &hash[..16])
}

/// 读取已有 token；不存在时用 OS 随机源生成 32 字节（64 hex）并持久化到数据目录。
/// 扩展读不到本地文件，只能由用户从设置页复制、粘到扩展选项页一次。
pub fn load_or_create_token(data_dir: &Path) -> Result<String, AppError> {
    let path = data_dir.join(TOKEN_FILE);
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let token = existing.trim().to_string();
        if !token.is_empty() {
            return Ok(token);
        }
    }
    let mut bytes = [0u8; 32];
    rand::Rng::fill(&mut rand::thread_rng(), &mut bytes);
    let token: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    std::fs::create_dir_all(data_dir)?;
    std::fs::write(&path, &token)?;
    Ok(token)
}

/// 丢弃旧 token 并重新生成（设置页「重新生成」按钮）。
pub fn regenerate_token(data_dir: &Path) -> Result<String, AppError> {
    let _ = std::fs::remove_file(data_dir.join(TOKEN_FILE));
    load_or_create_token(data_dir)
}

/// 在独立线程里启动桥。启动失败只打印告警，不影响 app 主流程。
pub fn start(app: AppHandle) {
    std::thread::spawn(move || {
        if let Err(error) = serve(&app) {
            eprintln!("[capture] 桥启动失败：{error}");
        }
    });
}

// ── 服务主体 ──

fn serve(app: &AppHandle) -> Result<(), AppError> {
    let state = app.state::<AppState>();
    let token = load_or_create_token(&state.data_dir)?;
    let pool = state.pool.clone();
    let (server, port) = bind_server()?;
    state
        .bridge_port
        .store(port, std::sync::atomic::Ordering::Relaxed);
    eprintln!("[capture] 桥已启动：http://127.0.0.1:{port}");

    for mut request in server.incoming_requests() {
        let response = handle(&mut request, &pool, &token, port, app);
        let _ = request.respond(response);
    }
    Ok(())
}

/// 从 `BASE_PORT` 顺延到 `MAX_PORT` 找一个空闲端口。
/// 扩展按顺序探测这 10 个端口，因此不需要写端口文件（扩展也读不到本地文件）。
fn bind_server() -> Result<(Server, u16), AppError> {
    for port in BASE_PORT..=MAX_PORT {
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        if let Ok(server) = Server::http(addr) {
            return Ok((server, port));
        }
    }
    Err(AppError::Other(format!(
        "端口 {BASE_PORT}-{MAX_PORT} 均被占用"
    )))
}

fn handle(
    request: &mut Request,
    pool: &SqlitePool,
    token: &str,
    port: u16,
    app: &AppHandle,
) -> Response<Cursor<Vec<u8>>> {
    if matches!(request.method(), Method::Options) {
        return cors_preflight();
    }
    // 只接受指向本机回环的 Host，挡掉 DNS rebinding（域名解析到 127.0.0.1 的攻击）。
    if !is_localhost_request(request) {
        return json(403, &ErrorResponse::new("只允许本机回环访问"));
    }
    if !authorized(request, token) {
        return json(401, &ErrorResponse::new("token 无效"));
    }

    let path = request
        .url()
        .split('?')
        .next()
        .unwrap_or(request.url())
        .trim_end_matches('/')
        .to_string();

    match (request.method(), path.as_str()) {
        (Method::Get, "/ping") => json(200, &PingResponse { ok: true, app: "bili-collector".into(), port }),
        (Method::Get, "/tags") => handle_tags(pool),
        (Method::Get, "/item") => handle_lookup(pool, request.url()),
        (Method::Post, "/capture") => handle_capture(request, pool, app),
        _ => json(404, &ErrorResponse::new("未知接口")),
    }
}

/// tiny_http 的 header 名是 `AsciiStr`，先转成 `&str` 再做大小写无关比较。
fn header_name(header: &Header) -> String {
    let field: &str = header.field.as_str().as_ref();
    field.to_ascii_lowercase()
}

fn is_localhost_request(request: &Request) -> bool {
    let host = request
        .headers()
        .iter()
        .find(|header| header_name(header) == "host")
        .map(|header| header.value.as_str().to_ascii_lowercase())
        // HTTP/1.0 允许不带 Host，此时放行（本地扩展必然带）。
        .unwrap_or_else(|| "localhost".to_string());
    host.starts_with("127.0.0.1")
        || host.starts_with("localhost")
        || host.starts_with("[::1]")
        || host.starts_with("[::0001]")
}

/// token 可来自请求头 `X-Bridge-Token`，或查询参数 `?token=`（后者方便扩展探测端口）。
fn authorized(request: &Request, token: &str) -> bool {
    let from_header = request
        .headers()
        .iter()
        .find(|header| header_name(header) == TOKEN_HEADER)
        .map(|header| header.value.as_str().trim().to_string());
    let provided = from_header
        .or_else(|| query_param(request.url(), "token"))
        .unwrap_or_default();
    constant_time_eq(&provided, token)
}

fn handle_tags(pool: &SqlitePool) -> Response<Cursor<Vec<u8>>> {
    match tauri::async_runtime::block_on(db::list_tags(pool)) {
        Ok(tags) => json(200, &TagsResponse { ok: true, tags }),
        Err(error) => json(500, &ErrorResponse::new(&error.to_string())),
    }
}

fn handle_lookup(pool: &SqlitePool, url: &str) -> Response<Cursor<Vec<u8>>> {
    let Some(target) = query_param(url, "url") else {
        return json(400, &ErrorResponse::new("缺少 url 参数"));
    };
    let external_id = external_id_for_url(&target);
    let found =
        tauri::async_runtime::block_on(db::find_item_by_source_id(pool, "browser", &external_id));
    match found {
        Ok(Some(item)) => {
            let tags = tauri::async_runtime::block_on(db::item_tag_names(pool, item.id))
                .unwrap_or_default();
            json(
                200,
                &ItemLookupResponse {
                    ok: true,
                    exists: true,
                    item: Some(SavedItemSummary {
                        title: item.title,
                        notes: item.notes,
                        tags,
                    }),
                },
            )
        }
        Ok(None) => json(
            200,
            &ItemLookupResponse {
                ok: true,
                exists: false,
                item: None,
            },
        ),
        Err(error) => json(500, &ErrorResponse::new(&error.to_string())),
    }
}

fn handle_capture(
    request: &mut Request,
    pool: &SqlitePool,
    app: &AppHandle,
) -> Response<Cursor<Vec<u8>>> {
    let body = match read_body(request) {
        Ok(body) => body,
        Err(error) => return json(400, &ErrorResponse::new(&error)),
    };
    let payload: CaptureRequest = match serde_json::from_str(&body) {
        Ok(payload) => payload,
        Err(error) => return json(400, &ErrorResponse::new(&format!("请求体解析失败：{error}"))),
    };

    let url = payload.url.trim().to_string();
    if url.is_empty() {
        return json(400, &ErrorResponse::new("url 不能为空"));
    }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return json(400, &ErrorResponse::new("只支持 http/https 链接"));
    }
    // 扩展传标题，回退到 URL 本身，保证卡片永远有可点可读的东西。
    let title = if payload.title.trim().is_empty() {
        url.clone()
    } else {
        payload.title.trim().to_string()
    };
    let tag_specs: Vec<TagInput> = payload
        .tags
        .iter()
        .filter(|name| !name.trim().is_empty())
        .map(|name| TagInput {
            id: None,
            // 沿用库内「用户自建标签」的命名空间，避免混进一个 UI 不认识的值。
            namespace: "manual".into(),
            name: name.trim().to_string(),
            color: None,
            description: None,
            category_id: None,
        })
        .collect();

    // 先尝试按域名路由到各站适配器，拿到更丰富的元数据（标题/作者/封面，或合集内多条）。
    // 任意一步失败都安全回退到下面的通用浏览器存档，不会丢数据。
    if let Some(routed) = route_capture(app, &url, &title, &payload.og_image) {
        if !routed.is_empty() {
            return handle_routed_capture(pool, app, &routed, &tag_specs, &payload.note);
        }
    }

    let external_id = external_id_for_url(&url);
    // 浏览器快速入库之前漏了 favicon：这里按域名取 favicon.im 图标地址，
    // 后续落盘到本地 covers/，WebView 才能正常显示（远程 favicon 在 WebView 无代理时加载失败）。
    let cover_url = BrowserBookmarkClient::resolve_favicon_url(&url);
    let existing = tauri::async_runtime::block_on(db::find_item_by_source_id(
        pool,
        "browser",
        &external_id,
    ));

    let result: Result<(i64, bool), AppError> = match existing {
        Ok(Some(item)) => {
            // 已存在：只改标题 / 备注 / 标签，不走 upsert_item，
            // 否则会清空 cover_local_path 与 extra_json（书签导入写进去的 folder_tags 会丢）。
            tauri::async_runtime::block_on(async {
                db::update_captured_item(pool, item.id, &title, &payload.note).await?;
                db::replace_item_tags(pool, item.id, &tag_specs).await?;
                // 已存在但还没有本地图标时，补一次 favicon 落盘（不覆盖已有封面）。
                if let Some(ref u) = cover_url {
                    let has_cover =
                        db::item_has_local_cover(pool, "browser", &external_id).await.unwrap_or(false);
                    if !has_cover {
                        let _ = localize_cover(&*app.state::<AppState>(), pool, "browser", &external_id, u).await;
                    }
                }
                Ok::<_, AppError>((item.id, false))
            })
        }
        Ok(None) => tauri::async_runtime::block_on(async {
            let item = crate::models::ExternalItem {
                source: "browser".into(),
                external_id: external_id.clone(),
                source_url: url.clone(),
                title,
                description: payload.description,
                cover_url: cover_url.clone(),
                cover_local_path: None,
                author_name: None,
                author_id: None,
                partition_name: Some("浏览器收集".into()),
                published_at: None,
                duration: None,
                favorite_time: Some(db::now_seconds()),
                extra: serde_json::json!({ "captured_from": "extension" }),
            };
            let (item_id, _) = db::upsert_item(pool, &item).await?;
            db::replace_item_tags(pool, item_id, &tag_specs).await?;
            db::update_item_notes(pool, item_id, &payload.note).await?;
            // 把 favicon 下载到本地 covers/，WebView 才能显示。
            if let Some(ref u) = cover_url {
                let _ = localize_cover(&*app.state::<AppState>(), pool, "browser", &external_id, u).await;
            }
            Ok::<_, AppError>((item_id, true))
        }),
        Err(error) => return json(500, &ErrorResponse::new(&error.to_string())),
    };

    match result {
        Ok((item_id, created)) => {
            let tags: Vec<String> = tag_specs.iter().map(|spec| spec.name.clone()).collect();
            let _ = app.emit(
                CAPTURE_EVENT,
                CaptureSavedPayload {
                    item_id,
                    title: tags_title_hint(&tags),
                    created,
                },
            );
            json(200, &CaptureResponse { ok: true, item_id, created, tags })
        }
        Err(error) => json(500, &ErrorResponse::new(&error.to_string())),
    }
}

/// 路由命中后逐条入库（与通用存档共用 upsert + 标签写入逻辑）。
/// 已存在的条目只补充标签/批注，不整体覆盖（避免抹掉封面与 extra）。
fn handle_routed_capture(
    pool: &SqlitePool,
    app: &AppHandle,
    items: &[ExternalItem],
    tag_specs: &[TagInput],
    note: &str,
) -> Response<Cursor<Vec<u8>>> {
    let mut imported = 0i64;
    let mut updated = 0i64;
    for item in items {
        let outcome: Result<(i64, bool), AppError> = tauri::async_runtime::block_on(async {
            let existing =
                db::find_item_by_source_id(pool, &item.source, &item.external_id).await?;
            let (item_id, created) = match existing {
                Some(existing) => {
                    if !note.is_empty() {
                        db::update_item_notes(pool, existing.id, note).await?;
                    }
                    (existing.id, false)
                }
                None => {
                    let (item_id, _) = db::upsert_item(pool, item).await?;
                    if !note.is_empty() {
                        db::update_item_notes(pool, item_id, note).await?;
                    }
                    (item_id, true)
                }
            };
            db::replace_item_tags(pool, item_id, tag_specs).await?;
            // 把远程封面 / 图标落本地（与正式导入一致）：仅当该项有封面 URL 且本地还没缓存时。
            if item.cover_url.as_deref().filter(|u| !u.is_empty()).is_some() {
                let has_cover = db::item_has_local_cover(pool, &item.source, &item.external_id)
                    .await
                    .unwrap_or(false);
                if !has_cover {
                    let url = item.cover_url.clone().unwrap();
                    let _ = localize_cover(
                        &*app.state::<AppState>(),
                        pool,
                        &item.source,
                        &item.external_id,
                        &url,
                    )
                    .await;
                }
            }
            Ok::<_, AppError>((item_id, created))
        });
        match outcome {
            Ok((_, created)) => {
                if created {
                    imported += 1;
                } else {
                    updated += 1;
                }
            }
            Err(error) => {
                eprintln!("[capture] 路由入库单条失败 {}: {error}", item.external_id);
            }
        }
    }

    let title = if imported > 0 && updated > 0 {
        format!("新增 {imported} / 更新 {updated}")
    } else if imported > 0 {
        format!("已收藏 {imported} 条")
    } else {
        format!("已更新 {updated} 条")
    };
    let _ = app.emit(
        CAPTURE_EVENT,
        CaptureSavedPayload {
            item_id: 0,
            title,
            created: imported > 0,
        },
    );
    json(
        200,
        &CaptureResponse {
            ok: true,
            item_id: 0,
            created: imported > 0,
            tags: tag_specs.iter().map(|spec| spec.name.clone()).collect(),
        },
    )
}

/// 尝试把链接路由到对应站的适配器，拿到丰富元数据。返回 `None` 表示「无法识别或失败」，
/// 调用方应回退到通用浏览器存档。
fn route_capture(
    app: &AppHandle,
    url: &str,
    title: &str,
    og_image: &str,
) -> Option<Vec<ExternalItem>> {
    let host = url::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_lowercase))?;
    let state = app.state::<AppState>();
    if host.contains("bilibili.com") {
        // 单条视频：从 URL 抽 BV 号，交给 enrich_items 补全标题/作者/封面。
        if let Some(bvid) = capture_bvid(url) {
            let minimal = ExternalItem {
                source: "bilibili".into(),
                external_id: bvid,
                source_url: url.to_string(),
                favorite_time: Some(db::now_seconds()),
                ..Default::default()
            };
            if let Ok(enriched) =
                tauri::async_runtime::block_on(state.bili.enrich_items(&[minimal]))
            {
                if !enriched.is_empty() {
                    return Some(enriched);
                }
            }
        }
        return tauri::async_runtime::block_on(route_via(&state.bili, url));
    }
    if host.contains("zhihu.com") {
        // 单条回答 / 文章 / 想法：有 cookie 时拿丰富元数据；
        // 知乎 API v4 有 x-zse-96 反爬，未登录/无签名时必 403，此时用扩展读到的
        // og:title / og:image 直接构造 zhihu 条目（仍识别为 zhihu 源，标题干净）；
        // 都失败才回退「按收藏夹路由」，再不行回退通用存档。
        if let Some(item) = tauri::async_runtime::block_on(state.zhihu.fetch_single(url)) {
            return Some(vec![item]);
        }
        if let Some(item) = crate::source::zhihu::item_from_url_and_meta(url, title, og_image) {
            return Some(vec![item]);
        }
        return tauri::async_runtime::block_on(route_via(&state.zhihu, url));
    }
    if host.contains("csdn.net") {
        // 单篇文章：抓 og: meta 拿标题 / 封面 / 摘要；
        // 不是文章详情页或抓取失败时回退「按收藏夹路由」，再不行回退通用存档。
        if let Some(item) = tauri::async_runtime::block_on(state.csdn.fetch_article(url)) {
            return Some(vec![item]);
        }
        return tauri::async_runtime::block_on(route_via(&state.csdn, url));
    }
    if host.contains("github.com") {
        // 单仓库链接（github.com/{owner}/{repo}）优先走丰富元数据；
        // 否则尝试按收藏夹（用户名）路由，失败则回退通用存档。
        if let Some(repo_item) =
            tauri::async_runtime::block_on(state.github.fetch_repo(url))
        {
            return Some(vec![repo_item]);
        }
        return tauri::async_runtime::block_on(route_via(&state.github, url));
    }
    None
}

/// 通用路由：resolve → fetch → enrich。任一步失败都返回 `None`（由调用方回退）。
async fn route_via<A: SourceAdapter>(adapter: &A, url: &str) -> Option<Vec<ExternalItem>> {
    let collection = adapter.resolve_collection(url).await.ok()?;
    let items = adapter.fetch_collection(&collection).await.ok()?;
    let enriched = adapter.enrich_items(&items).await.ok()?;
    if enriched.is_empty() {
        None
    } else {
        Some(enriched)
    }
}

/// 把远程封面 / 图标下载到本地 `covers/`，并写回 `cover_local_path`。
///
/// 这样前端 WebView 永远只读本地文件，不受 WebView 不继承 app 代理、够不到外网 CDN 的影响
/// （正式导入管线也是这么做的，capture 路径之前漏了这一步，导致 B站封面 / 浏览器图标加载失败）。
async fn localize_cover(
    state: &AppState,
    pool: &SqlitePool,
    source: &str,
    external_id: &str,
    cover_url: &str,
) {
    eprintln!("[capture] localize_cover 开始 source={source} id={external_id} url={cover_url}");
    let Some((bytes, extension)) = download_cover_for(state, source, cover_url).await else {
        eprintln!("[capture] localize_cover 下载失败，跳过 source={source} id={external_id}");
        return;
    };
    eprintln!(
        "[capture] localize_cover 下载成功 {} 字节 ext={extension} source={source} id={external_id}",
        bytes.len()
    );
    let path = match crate::commands::save_cover_file(&state, source, external_id, &bytes, &extension) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[capture] localize_cover 落盘失败 {e}");
            return;
        }
    };
    match db::set_item_cover_local_path(pool, source, external_id, &path).await {
        Ok(()) => eprintln!("[capture] localize_cover 已写 cover_local_path={path}"),
        Err(e) => eprintln!("[capture] localize_cover 写库失败 {e}"),
    }
}

/// 按来源选择下载器：bilibili / csdn 用各自客户端（带 Referer 等），
/// 其余（浏览器 favicon、知乎、GitHub 头像）走通用带代理的 GET。
async fn download_cover_for(
    state: &AppState,
    source: &str,
    url: &str,
) -> Option<(Vec<u8>, String)> {
    let outcome: Result<(Vec<u8>, String), AppError> = match source {
        "bilibili" => state.bili.download_cover(url).await,
        "csdn" => state.csdn.download_cover(url).await,
        _ => match download_cover_generic(url).await {
            Some(v) => Ok(v),
            None => Err(AppError::Other("通用封面下载返回 None".into())),
        },
    };
    match outcome {
        Ok(v) => Some(v),
        Err(e) => {
            eprintln!("[capture] download_cover_for 失败 source={source} url={url} err={e}");
            None
        }
    }
}

/// 通用封面下载：带系统代理（与 B站客户端一致），用于浏览器 favicon / 知乎 / GitHub 等
/// 远程 https 封面。WebView 默认不继承 app 代理，所以必须落本地。
async fn download_cover_generic(url: &str) -> Option<(Vec<u8>, String)> {
    let mut builder = reqwest::Client::builder();
    if let Some(proxy) = crate::source::proxy::resolve_system_proxy() {
        builder = builder.proxy(proxy);
    }
    let client = builder.build().ok()?;
    let response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, "Mozilla/5.0")
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    // 先取出 content-type，再消费 response 拿 bytes（bytes() 会转移所有权）。
    let extension = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|ct| {
            if ct.contains("png") {
                "png"
            } else if ct.contains("webp") {
                "webp"
            } else if ct.contains("gif") {
                "gif"
            } else {
                "jpg"
            }
        })
        .unwrap_or("jpg")
        .to_string();
    let bytes = response.bytes().await.ok()?;
    Some((bytes.to_vec(), extension))
}

/// 从 bilibili 链接里抽 BV 号（BV 后接字母数字）。抽不到返回 `None`。
fn capture_bvid(url: &str) -> Option<String> {
    let bytes = url.as_bytes();
    let mut index = 0;
    while index + 2 < bytes.len() {
        if bytes[index] == b'B' && bytes[index + 1] == b'V' {
            let mut end = index + 2;
            while end < bytes.len() && bytes[end].is_ascii_alphanumeric() {
                end += 1;
            }
            if end > index + 2 {
                return Some(url[index..end].to_string());
            }
        }
        index += 1;
    }
    None
}

/// 事件载荷里的标题只是给 toast 用；真正的标题已入库，这里回传标签串避免再查一次库。
fn tags_title_hint(tags: &[String]) -> String {
    if tags.is_empty() {
        "已收藏".into()
    } else {
        tags.join("、")
    }
}

fn read_body(request: &mut Request) -> Result<String, String> {
    let mut body = Vec::new();
    request
        .as_reader()
        .take(MAX_BODY_BYTES as u64)
        .read_to_end(&mut body)
        .map_err(|error| error.to_string())?;
    String::from_utf8(body).map_err(|error| error.to_string())
}

// ── 响应构造 ──

fn json(status: u16, body: &impl Serialize) -> Response<Cursor<Vec<u8>>> {
    // 兜底必须是 ASCII：byte string 里不能出现中文。
    let payload = serde_json::to_vec(body)
        .unwrap_or_else(|_| br#"{"ok":false,"error":"serialize failed"}"#.to_vec());
    let mut response = Response::from_data(payload).with_status_code(status);
    add_cors_headers(&mut response);
    response
}

/// 扩展源是 `chrome-extension://<id>`，跨源且带自定义头，必然触发预检。
fn cors_preflight() -> Response<Cursor<Vec<u8>>> {
    let mut response = Response::from_data(Vec::new()).with_status_code(204);
    add_cors_headers(&mut response);
    response
}

fn add_cors_headers(response: &mut Response<Cursor<Vec<u8>>>) {
    // token 是真正的防线，所以 Origin 可以放开；这样扩展 id 变化时不用改代码。
    for (key, value) in [
        ("Access-Control-Allow-Origin", "*"),
        ("Access-Control-Allow-Headers", "content-type, x-bridge-token"),
        ("Access-Control-Allow-Methods", "GET, POST, OPTIONS"),
        ("Access-Control-Max-Age", "86400"),
    ] {
        if let Ok(header) = Header::from_bytes(key.as_bytes(), value.as_bytes()) {
            response.add_header(header);
        }
    }
}

// ── 小工具 ──

fn query_param(url: &str, key: &str) -> Option<String> {
    let query = url.split_once('?')?.1;
    // 去掉可能存在的 fragment
    let query = query.split('#').next().unwrap_or(query);
    for pair in query.split('&') {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        if name == key {
            return Some(percent_decode(value));
        }
    }
    None
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
            {
                out.push(hi * 16 + lo);
                index += 3;
                continue;
            }
        }
        out.push(if bytes[index] == b'+' { b' ' } else { bytes[index] });
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// 定长比较，避免通过响应时间逐字节猜 token。
fn constant_time_eq(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in left.iter().zip(right.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

impl ErrorResponse {
    fn new(message: &str) -> Self {
        Self {
            ok: false,
            error: message.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_id_is_stable_and_short() {
        let first = external_id_for_url("https://example.com/a");
        let second = external_id_for_url("https://example.com/a");
        assert_eq!(first, second);
        assert!(first.starts_with("bk_"));
        assert_eq!(first.len(), 3 + 16);
        assert_ne!(first, external_id_for_url("https://example.com/b"));
    }

    #[test]
    fn query_param_reads_decoded_value() {
        assert_eq!(
            query_param("/item?url=https%3A%2F%2Fa.test%2Fb%3Fx%3D1", "url").as_deref(),
            Some("https://a.test/b?x=1")
        );
        assert_eq!(query_param("/item?token=abc", "token").as_deref(), Some("abc"));
        assert_eq!(query_param("/item", "token"), None);
        assert_eq!(query_param("/item?a=1&b=2", "c"), None);
    }

    #[test]
    fn percent_decode_handles_plus_and_invalid_sequences() {
        assert_eq!(percent_decode("a+b"), "a b");
        assert_eq!(percent_decode("a%2Fb"), "a/b");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
    }

    #[test]
    fn token_compares_in_constant_time() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "abcd"));
        assert!(constant_time_eq("", ""));
    }

    #[test]
    fn token_persists_and_regenerates() {
        let dir = std::env::temp_dir().join(format!("capture-token-{}", db::now_seconds()));
        let _ = std::fs::create_dir_all(&dir);
        let first = load_or_create_token(&dir).expect("生成 token");
        assert_eq!(first.len(), 64);
        assert_eq!(load_or_create_token(&dir).expect("读取 token"), first);
        let second = regenerate_token(&dir).expect("重新生成 token");
        assert_ne!(first, second);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn capture_bvid_extracts_from_video_url() {
        assert_eq!(
            capture_bvid("https://www.bilibili.com/video/BV1xx411c7mD?p=1"),
            Some("BV1xx411c7mD".to_string())
        );
        assert_eq!(
            capture_bvid("https://b23.tv/BVabcdef123"),
            Some("BVabcdef123".to_string())
        );
        assert_eq!(capture_bvid("https://www.bilibili.com/favlist?fid=123"), None);
        assert_eq!(capture_bvid("https://example.com/never"), None);
    }
}
