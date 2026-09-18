use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_yaml;

use crate::error::AppError;
use crate::models::VideoItem;
// 自定义协议（obsidian://）统一走公共模块，避免与网易云 orpheus:// 各写一份
use crate::uri::open_uri_system;

/// 分区托管标记：Obsidian 阅读视图下 HTML 注释不可见，但能圈出 app 的责任边界。
/// 同步时只替换这两个标记之间的内容，标记之外的用户区永不触动。
const NOTES_START: &str = "<!-- collector:notes:start -->";
const NOTES_END: &str = "<!-- collector:notes:end -->";
const SETTINGS_FILE: &str = "obsidian_settings.json";

/// Obsidian 联动配置（存于 app data 目录下的 obsidian_settings.json）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ObsidianSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub vault_path: String,
    #[serde(default)]
    pub vault_name: String,
    #[serde(default = "default_subdir")]
    pub subdir: String,
}

fn default_subdir() -> String {
    "收藏".to_string()
}

pub fn load_settings(data_dir: &Path) -> ObsidianSettings {
    let path = data_dir.join(SETTINGS_FILE);
    match fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => ObsidianSettings::default(),
    }
}

pub fn save_settings(data_dir: &Path, settings: &ObsidianSettings) -> Result<(), AppError> {
    let path = data_dir.join(SETTINGS_FILE);
    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| AppError::Other(format!("设置序列化失败: {e}")))?;
    fs::write(&path, json).map_err(AppError::Io)?;
    Ok(())
}

/// Windows 文件名非法字符白名单清洗 + 长度截断（中文按字符计，留余量避免超 255 字节）。
fn sanitize_filename(name: &str) -> String {
    let illegal: &[char] = &['\\', '/', ':', '*', '?', '"', '<', '>', '|'];
    let mut out: String = name
        .chars()
        .filter(|c| !illegal.contains(c))
        .map(|c| match c {
            '\n' | '\r' | '\t' => ' ',
            other => other,
        })
        .collect();
    out = out.trim().to_string();
    let limited: String = out.chars().take(200).collect();
    if limited.is_empty() {
        "未命名收藏".to_string()
    } else {
        limited
    }
}

/// Obsidian tag 不允许 `#`、空格与 `[]|`，清洗之。
fn sanitize_tag(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '#' => '_',
            ' ' | '\t' | '\n' | '\r' => '-',
            other => other,
        })
        .filter(|c| !"[]|".contains(*c))
        .collect()
}

/// 极简 unix 秒 -> YYYY-MM-DD（UTC），避免引入额外时间库。
fn format_unix_date(ts: i64) -> String {
    const DAY: i64 = 86400;
    let days = ts.div_euclid(DAY);
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 }.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096).div_euclid(365);
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2).div_euclid(153);
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

#[derive(Serialize)]
struct NoteFrontmatter {
    collector_id: String,
    title: String,
    url: String,
    source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    author: Option<String>,
    tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    favorited_at: Option<String>,
}

/// 用 serde_yaml 生成 frontmatter —— 绝不手拼字符串，避免标题含 `:`/`#`/`[` 等导致 YAML 崩坏。
fn build_frontmatter(item: &VideoItem) -> Result<String, AppError> {
    let tags: Vec<String> = item.tags.iter().map(|t| sanitize_tag(&t.name)).collect();
    let fm = NoteFrontmatter {
        collector_id: format!("{}:{}", item.source, item.external_id),
        title: item.title.clone(),
        url: item.source_url.clone(),
        source: item.source.clone(),
        author: item.author_name.clone(),
        tags,
        favorited_at: item.favorite_time.map(format_unix_date),
    };
    let yaml = serde_yaml::to_string(&fm)
        .map_err(|e| AppError::Other(format!("frontmatter 生成失败: {e}")))?;
    Ok(format!("---\n{yaml}---\n"))
}

fn render_note(item: &VideoItem, notes: &str) -> Result<String, AppError> {
    // 新建笔记：纯笔记内容，不写 collector frontmatter、不写托管标记（零 collector 痕迹）。
    // 身份识别靠数据库里的 `items.obsidian_path`，不需要在用户文件里留任何 collector 专属标记。
    let _ = item;
    Ok(notes.to_string())
}

fn write_utf8_no_bom(path: &Path, content: &str) -> Result<(), AppError> {
    // Rust 字符串即 UTF-8，直接写字节即无 BOM。
    fs::write(path, content.as_bytes()).map_err(AppError::Io)?;
    Ok(())
}

fn normalize_rel(p: &Path) -> String {
    p.to_string_lossy()
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_string()
}

/// 纯词法规范化：去掉 `.` 组件、回退 `..`，不触碰文件系统（路径可以尚不存在）。
fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// 最后一道防线：确保目标绝对路径仍落在 vault 内（subdir 若填了绝对路径 / `..` 也拦得住）。
///
/// 注意：**不能**用 `canonicalize()` 后做前缀比较 —— Windows 上 canonicalize 会给 vault
/// 返回带 `\\?\` 前缀的路径，而目标文件首次写入前父目录可能不存在、canonicalize 失败
/// 退回无前缀的原始路径，两边形式不一致导致 `starts_with` 永远为 false，所有写入都会
/// 被误判为「超出仓库范围」而拒绝（曾导致「有批注却导出提示没有批注」）。
fn ensure_within_vault(vault: &Path, abs: &Path) -> Result<(), AppError> {
    let vault = lexical_normalize(vault);
    let abs = lexical_normalize(abs);
    if !abs.starts_with(&vault) {
        return Err(AppError::InvalidInput(
            "目标路径超出 Obsidian 仓库范围，已拒绝写入".into(),
        ));
    }
    Ok(())
}

/// 托管区正文的哈希，双向同步用它比对内容是否变化。
///
/// 只用于本地变更检测（非安全场景），md5 足够且短。**推送方与拉取方必须共用这一个实现**，
/// 否则两边算出来的 base 不一致，会永远判定「有变化」。
pub fn hash_notes(text: &str) -> String {
    format!("{:x}", md5::compute(text.as_bytes()))
}

/// 托管区在一篇笔记里的字节位置（下标均落在 UTF-8 字符边界上，切片不会 panic）。
struct ManagedZone {
    /// START 标记之前的一切：frontmatter + 用户原文
    head_end: usize,
    /// 托管区正文起点（START 标记之后）
    body_start: usize,
    /// 托管区正文终点（END 标记的起点）
    body_end: usize,
    /// 用户区起点（END 标记之后）
    user_start: usize,
}

/// 定位托管区。两个标记缺一、或顺序颠倒（用户手动挪过）都返回 `None`。
///
/// 返回 None 一律表示「这篇笔记没被托管 / 已被取消托管」，调用方应跳过而不是重建。
fn locate_managed_zone(content: &str) -> Option<ManagedZone> {
    let start_marker = content.find(NOTES_START)?;
    let end_marker = content.find(NOTES_END)?;
    if end_marker < start_marker {
        return None;
    }
    Some(ManagedZone {
        head_end: start_marker,
        body_start: start_marker + NOTES_START.len(),
        body_end: end_marker,
        user_start: end_marker + NOTES_END.len(),
    })
}

/// 这篇笔记里是否已有托管区（双向同步 / link 的分岔判据）。
pub fn has_managed_zone(content: &str) -> bool {
    locate_managed_zone(content).is_some()
}

/// 读出这篇笔记里 **app 侧负责的正文**。
///
/// - `Managed`：托管区正文（首尾空白已去掉）
/// - `Full`：整篇原文（含 frontmatter、用户写过的一切）
///
/// 这是「关联后侧边栏该显示什么」的唯一答案，也是双向同步拉取方向的取数口径；
/// 推送方向必须严格按 `apply_note_body` 写回，两者互逆才不会漂移。
pub fn read_note_body(content: &str) -> String {
    match read_managed_note(content) {
        Some(managed) => managed.notes,
        None => content.to_string(),
    }
}

/// 一篇已存在笔记拆出来的三段。head 与 user_zone 都**原样**返回，写入时不得改写。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedNote {
    /// START 之前的一切（frontmatter、用户原文）
    pub head: String,
    /// 托管区正文，已去掉首尾空白
    pub notes: String,
    /// END 之后的用户区，app 永不写入这里
    pub user_zone: String,
}

/// 读出一篇笔记的托管区正文；没有托管标记时返回 `None`。
pub fn read_managed_note(content: &str) -> Option<ManagedNote> {
    let zone = locate_managed_zone(content)?;
    let head = content.get(..zone.head_end).unwrap_or("").to_string();
    let body = content.get(zone.body_start..zone.body_end).unwrap_or("");
    let user_zone = content.get(zone.user_start..).unwrap_or("").to_string();
    Some(ManagedNote {
        head,
        notes: body.trim().to_string(),
        user_zone,
    })
}

/// 判断 head 里的 frontmatter 是不是 collector 自己写的。
///
/// 只有自家文件才刷新元数据（标题/标签会变）。用户自选关联的笔记往往带着自己的
/// frontmatter（dataview、templater 之类），app 一旦重写就可能把用户的东西毁掉，
/// 所以遇到非自家文件一律原样保留 head。
fn owns_frontmatter(head: &str, collector_id: &str) -> bool {
    head.starts_with("---") && head.contains(&format!("collector_id: {collector_id}"))
}

/// 拼装一篇托管笔记的完整内容（纯函数，便于单测钉死它与 `read_managed_note` 的互逆性）。
///
/// ⚠️ 格式铁律：`head` **自带尾换行**，后面直接接 START。绝不能写成 `{head}\n{START}`
/// ——那样读出来的 head 会包含刚写的那个 `\n`，写回去时又补一个，每轮都多出一个空行，
/// 文件永远在漂移，轮询也就永远判定「有变化」，进入无限同步。
fn render_managed_content(head: &str, notes: &str, user_zone: &str) -> String {
    format!("{head}{NOTES_START}\n{notes}\n{NOTES_END}{user_zone}")
}

fn write_managed_note_file(
    path: &Path,
    head: &str,
    notes: &str,
    user_zone: &str,
) -> Result<(), AppError> {
    write_utf8_no_bom(path, &render_managed_content(head, notes, user_zone))
}

/// 把 app 侧的笔记正文写回一篇已存在的笔记文件。
///
/// 按文件的接管模式分派，是 `read_note_body` 的严格逆操作：
/// - `Managed`：只替换托管区，head 与 END 之后的用户区原样保留
/// - `Full`：整篇覆盖 —— 用户关联的就是整篇，他编辑的也是整篇
///
/// 两者**都不再返回「跳过」**。以前「缺标记 → 跳过同步」曾是「用户取消托管」的语义，
/// 但整篇接管后缺标记恰恰是最常见的合法状态（用户关联的笔记本来就没标记），
/// 再跳过就等于永远不同步。取消同步请走 `/obsidian/unlink`。
pub fn apply_note_body(path: &Path, item: &VideoItem, notes: &str) -> Result<(), AppError> {
    let old = fs::read_to_string(path).map_err(AppError::Io)?;
    let Some(managed) = read_managed_note(&old) else {
        // 整篇接管：正文即全文，不注入任何标记（注入了下一轮就会被判成 Managed 模式，行为跳变）。
        return write_utf8_no_bom(path, notes);
    };
    let collector_id = format!("{}:{}", item.source, item.external_id);
    let new_head = if owns_frontmatter(&managed.head, &collector_id) {
        build_frontmatter(item)?
    } else {
        managed.head
    };
    write_managed_note_file(path, &new_head, notes, &managed.user_zone)
}

/// 把一个可能带 collector 托管标记 / frontmatter 的笔记，整理成「整篇纯 Markdown」。
///
/// - 有托管标记：去掉 `<!-- collector:notes:start/end -->` 那对注释，保留 head + 托管区正文 +
///   用户区，三者拼接回原样内容（只删标记，不丢任何文字）。
/// - head 若是 collector 自己的 frontmatter（开头 `---` 且含 `collector_id:`）：一并去掉，
///   做到「零 collector 痕迹」。用户自己的 frontmatter 原样保留。
/// - 本来就干净的文件：原样返回。
fn strip_collector_traces(content: &str) -> String {
    let body = match read_managed_note(content) {
        Some(m) => format!("{}{}{}", m.head, m.notes, m.user_zone),
        None => content.to_string(),
    };
    // 去掉 collector 专属 frontmatter（只认自己写的那种：开头 `---` 且含 `collector_id:`）
    if body.starts_with("---\n") && body.contains("collector_id:") {
        if let Some(pos) = body.find("\n---\n") {
            return body[pos + 5..].to_string();
        }
    }
    body
}

/// 把一条收藏关联到用户自选的笔记文件，返回**应写入 `items.notes` 的正文**。
///
/// ## 整篇接管 + 零 collector 痕迹
///
/// 直接读取文件原文当正文，不去动用户的文件；但若文件里带有 collector 的托管标记
/// （`<!-- collector:notes:start/end -->`）或 collector 专属 frontmatter（`collector_id:`），
/// 关联即**就地剥离**，使文件变成纯 Markdown、正文即整篇原文。这样「关联已有」与「新建笔记」
/// 产出的文件完全一样：侧边栏看到并编辑的是整个文件，app 不在用户文件里留任何专属标记。
///
/// ⚠️ 已关联的旧版带标记笔记（vault 里现存的那些）**不**走这里 —— 它们仍走同步的托管区逻辑
/// （`read_note_body` / `apply_note_body` 的 Managed 分支），行为保持不变，只对新增关联生效。
///
/// 关联本身**不写** collector 的 frontmatter：识别关系靠数据库里的 `items.obsidian_path`。
pub fn link_item_to_note_file(vault: &Path, rel: &str) -> Result<String, AppError> {
    let abs = vault.join(rel);
    ensure_within_vault(vault, &abs)?;
    let content = if abs.exists() {
        fs::read_to_string(&abs).map_err(AppError::Io)?
    } else {
        String::new()
    };
    // 整篇接管 + 零 collector 痕迹：若文件带标记 / collector frontmatter，关联即接管为纯笔记，
    // 就地改写成干净版本（`body != content` 时才动盘，干净文件一个字节都不改）。
    let body = strip_collector_traces(&content);
    if body != content {
        write_utf8_no_bom(&abs, &body)?;
    }
    Ok(body)
}

/// 计算不冲突的相对路径；若已存在文件且 `collector_id` 是自己的就复用，否则追加 `[source-id前8]` 消歧。
fn resolve_unique_rel(
    vault: &Path,
    sub: &Path,
    file_name: &str,
    collector_id: &str,
) -> Result<String, AppError> {
    let candidate = sub.join(file_name);
    let abs = vault.join(&candidate);
    if !abs.exists() {
        return Ok(normalize_rel(&candidate));
    }
    if let Ok(existing) = fs::read_to_string(&abs) {
        if existing.contains(&format!("collector_id: {collector_id}")) {
            return Ok(normalize_rel(&candidate));
        }
    }
    let stem = file_name.trim_end_matches(".md");
    let id_suffix: String = collector_id.replace(':', "-").chars().take(8).collect();
    let mut disambig = format!("{stem} [{id_suffix}].md");
    let mut rel = sub.join(&disambig);
    let mut i = 1;
    while vault.join(&rel).exists() {
        i += 1;
        disambig = format!("{stem} [{id_suffix}]-{i}.md");
        rel = sub.join(&disambig);
    }
    Ok(normalize_rel(&rel))
}

/// 把一条收藏同步成 vault 内的 md 笔记，返回 vault 内相对路径。
///
/// - 已有 `obsidian_path` 且文件还在 → 更新原文件（按模式：托管区 / 整篇）
/// - 否则 → 在收藏子目录下新建一篇带托管区的笔记
pub fn write_or_update_note(
    settings: &ObsidianSettings,
    item: &VideoItem,
) -> Result<String, AppError> {
    let vault = Path::new(&settings.vault_path);
    if !vault.is_dir() {
        return Err(AppError::InvalidInput(
            "Obsidian 仓库目录不存在或不是文件夹，请在设置中重新选择".into(),
        ));
    }
    let sub: PathBuf = if settings.subdir.trim().is_empty() {
        PathBuf::new()
    } else {
        PathBuf::from(settings.subdir.trim())
    };
    let file_name = format!("{}.md", sanitize_filename(&item.title));
    let collector_id = format!("{}:{}", item.source, item.external_id);

    // 已有映射：优先更新原文件，避免标题变更产生孤儿文件
    if let Some(rel) = &item.obsidian_path {
        let target = vault.join(rel);
        if target.exists() {
            apply_note_body(&target, item, &item.notes)?;
            return Ok(rel.clone());
        }
    }

    // 首次 / 回退：按标题生成文件名
    let rel = resolve_unique_rel(vault, &sub, &file_name, &collector_id)?;
    let abs = vault.join(&rel);
    ensure_within_vault(vault, &abs)?;
    if let Some(parent) = abs.parent() {
        fs::create_dir_all(parent).map_err(AppError::Io)?;
    }
    let content = render_note(item, &item.notes)?;
    write_utf8_no_bom(&abs, &content)?;
    Ok(rel)
}

fn rel_for_new(settings: &ObsidianSettings, item: &VideoItem) -> String {
    let sub = settings.subdir.trim().trim_end_matches('/');
    let file_name = sanitize_filename(&item.title);
    if sub.is_empty() {
        format!("{file_name}.md")
    } else {
        format!("{sub}/{file_name}.md")
    }
}

fn urlencode(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 3);
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => {
                out.push('%');
                out.push_str(&format!("{byte:02X}"));
            }
        }
    }
    out
}

fn build_open_uri(vault: &str, rel_path: &str) -> String {
    let mut uri = String::from("obsidian://open?");
    if !vault.is_empty() {
        uri.push_str(&format!("vault={}&", urlencode(vault)));
    }
    uri.push_str(&format!("file={}", urlencode(rel_path)));
    uri
}

fn build_new_uri(vault: &str, rel_path: &str, content: &str) -> String {
    let mut uri = String::from("obsidian://new?");
    if !vault.is_empty() {
        uri.push_str(&format!("vault={}&", urlencode(vault)));
    }
    uri.push_str(&format!(
        "file={}&content={}",
        urlencode(rel_path),
        urlencode(content)
    ));
    uri
}

/// 在 Obsidian 中打开（或兜底新建）该收藏对应的笔记。
pub fn open_in_obsidian(settings: &ObsidianSettings, item: &VideoItem) -> Result<(), AppError> {
    let uri = match &item.obsidian_path {
        Some(rel) => build_open_uri(&settings.vault_name, rel),
        None => {
            let rel = rel_for_new(settings, item);
            let content = render_note(item, &item.notes)?;
            build_new_uri(&settings.vault_name, &rel, &content)
        }
    };
    open_uri_system(&uri)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- 托管区读写的互逆性 --------------------------------------------------
    // 这条是整个双向同步的地基：一旦「读→写」不能稳定到幂等，轮询每轮都认为文件变了，
    // 就会把同一条笔记反复来回同步（典型的同步死循环）。下面几组用例把它钉死。

    /// 拼一篇笔记。注意 `head` 自带尾换行 —— 这是写入格式的约定。
    fn sample(head: &str, notes: &str, user_zone: &str) -> String {
        render_managed_content(head, notes, user_zone)
    }

    #[test]
    fn read_managed_note_extracts_three_zones() {
        let content = sample("---\ncollector_id: bilibili:BV1\n---\n", "第一行\n第二行", "\n自己的话\n");
        let note = read_managed_note(&content).expect("应识别出托管区");
        assert_eq!(note.notes, "第一行\n第二行");
        assert_eq!(note.head, "---\ncollector_id: bilibili:BV1\n---\n");
        assert_eq!(note.user_zone, "\n自己的话\n");
    }

    #[test]
    fn read_managed_note_returns_none_without_markers() {
        assert!(read_managed_note("随便写点什么，没有标记").is_none());
        // 只有一半标记同样算「未托管」
        assert!(read_managed_note(&format!("{NOTES_START}\n孤儿")).is_none());
        assert!(read_managed_note(&format!("孤儿\n{NOTES_END}")).is_none());
    }

    #[test]
    fn read_then_write_is_idempotent() {
        // 最关键的一条：按同样格式读出来再写回去，必须立刻稳定下来，不能持续漂移。
        // 曾经这里有个 head 多补一个 \n 的 bug —— 每轮同步给文件加一个空行，
        // 轮询也就永远认为「文件变了」，直接无限同步。
        for notes in ["", "单行", "多行\n第二行\n", " 前后带空白 \n"] {
            let original = sample("HEAD\n", notes, "\nTAIL\n");
            let parsed = read_managed_note(&original).expect("应识别出托管区");
            let rebuilt = render_managed_content(&parsed.head, &parsed.notes, &parsed.user_zone);
            let twice = read_managed_note(&rebuilt).expect("应识别出托管区");
            assert_eq!(twice, parsed, "notes 为 {notes:?} 时读写不互逆");
        }
    }

    #[test]
    fn write_does_not_drift_on_repeat() {
        // 连续三轮「读→写」，除了首次会规范化笔记首尾空白外，之后必须字节级稳定。
        let mut content = sample("# 用户原有笔记\n\n自己的内容\n", "app 侧笔记", "\n");
        for round in 0..3 {
            let parsed = read_managed_note(&content).expect("应识别出托管区");
            let rebuilt = render_managed_content(&parsed.head, &parsed.notes, &parsed.user_zone);
            if round > 0 {
                assert_eq!(rebuilt, content, "第 {round} 轮发生了漂移");
            }
            // 用户的 head 与 user_zone 必须始终原样保留
            assert!(parsed.head.contains("# 用户原有笔记"));
            content = rebuilt;
        }
    }

    #[test]
    fn marks_are_part_of_render_output() {
        // head 为空（用户自选文件里压根没有 collector frontmatter）时，
        // 托管区应当被夹在文件中间/末尾，标记本身完整保留。
        let content = render_managed_content("# 用户笔记\n", "app 笔记", "\n");
        assert!(content.starts_with("# 用户笔记\n"));
        assert!(content.contains(NOTES_START));
        assert!(content.contains(NOTES_END));
        assert!(content.ends_with("\n"));
    }

    #[test]
    fn has_managed_zone_detects_presence() {
        assert!(has_managed_zone(&sample("", "x", "")));
        assert!(!has_managed_zone("没有任何标记"));
        // 标记顺序颠倒（用户手动挪过）→ 视为未托管
        assert!(!has_managed_zone(&format!("{NOTES_END}\n{NOTES_START}")));
    }

    // ---- 整篇接管（关联用户已有笔记）----------------------------------------
    // 用户明确要求「关联后看到并编辑整篇内容」，所以无标记的笔记一律整篇接管：
    // 读全文、写全文、且绝不往用户文件里注入托管标记（注入了下一轮就会切成托管模式，行为跳变）。

    fn tmp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "collector-obsidian-test-{}-{name}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("建临时目录失败");
        dir
    }

    fn test_item() -> VideoItem {
        VideoItem {
            id: 1,
            source: "bilibili".into(),
            external_id: "BV1".into(),
            source_url: "https://example.com/BV1".into(),
            title: "标题".into(),
            description: String::new(),
            notes: String::new(),
            cover_url: None,
            cover_local_path: None,
            author_name: None,
            author_id: None,
            partition_name: None,
            published_at: None,
            duration: None,
            favorite_time: None,
            deleted_at: None,
            starred: false,
            starred_at: None,
            obsidian_path: None,
            tags: vec![],
        }
    }

    #[test]
    fn read_note_body_returns_whole_file_when_unmanaged() {
        // 一篇普通的 Obsidian 笔记：有自己的 frontmatter、自己的正文
        let content = "---\ntags: [读书]\n---\n\n# 我的读书笔记\n\n正文第一段\n";
        assert_eq!(read_note_body(content), content, "整篇接管：正文就是整篇原文");
        // 首尾空白必须原样保留：在这里 trim 的话，关联一瞬间就会改掉用户的文件
        // （吃掉文末换行），而「关联不动用户文件」是硬要求。
        assert_eq!(
            read_note_body("\n\n带前后空行的笔记\n\n"),
            "\n\n带前后空行的笔记\n\n"
        );
    }

    #[test]
    fn read_note_body_returns_only_managed_zone_when_marked() {
        let content = sample("---\ncollector_id: bilibili:BV1\n---\n", "app 笔记", "\n用户区\n");
        assert_eq!(read_note_body(&content), "app 笔记");
        // 标记本身绝不能进入正文 —— 否则写回去就是标记套娃
        assert!(!read_note_body(&content).contains(NOTES_START));
        assert!(!read_note_body(&content).contains(NOTES_END));
    }

    #[test]
    fn link_reads_whole_file_and_leaves_it_untouched() {
        let vault = tmp_dir("link");
        let rel = "收藏/我的笔记.md";
        let abs = vault.join("收藏").join("我的笔记.md");
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        let original = "# 我写了很久的笔记\n\n正文\n";
        fs::write(&abs, original).unwrap();

        let body = link_item_to_note_file(&vault, rel).expect("关联应成功");
        assert_eq!(body, original, "关联后的正文应是整篇原文");
        // 用户的文件一个字节都不能被动过（早期版本会往末尾追加托管标记）
        assert_eq!(
            fs::read_to_string(&abs).unwrap(),
            original,
            "关联不得修改用户的文件"
        );
    }

    #[test]
    fn link_strips_markers_and_collector_frontmatter() {
        // 关联一篇 collector 自己建的（带标记 + collector_id frontmatter）旧笔记：
        // 应就地剥离成纯 Markdown，正文即整篇原文，文件被改写干净。
        let vault = tmp_dir("link-strip");
        let rel = "收藏/旧笔记.md";
        let abs = vault.join("收藏").join("旧笔记.md");
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        let original = "---\ncollector_id: bilibili:BV1\ntitle: x\n---\n\
            <!-- collector:notes:start -->\napp 笔记\n<!-- collector:notes:end -->\n用户区\n";
        fs::write(&abs, original).unwrap();

        let body = link_item_to_note_file(&vault, rel).expect("关联应成功");
        assert_eq!(body, "app 笔记\n用户区\n", "整篇接管：去掉标记与 collector frontmatter");

        let after = fs::read_to_string(&abs).unwrap();
        assert!(!after.contains(NOTES_START), "关联后应去掉托管标记");
        assert!(!after.contains(NOTES_END));
        assert!(!after.contains("collector_id:"), "关联后应去掉 collector frontmatter");
        assert_eq!(after, body, "文件应被改写为干净纯笔记");
    }

    #[test]
    fn link_leaves_plain_file_untouched() {
        // 本来就干净的笔记：关联不得改写用户文件，正文即整篇原文。
        let vault = tmp_dir("link-plain");
        let rel = "收藏/普通.md";
        let abs = vault.join("收藏").join("普通.md");
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        let original = "# 我自己的笔记\n\n正文\n";
        fs::write(&abs, original).unwrap();

        let body = link_item_to_note_file(&vault, rel).expect("关联应成功");
        assert_eq!(body, original, "关联后的正文应是整篇原文");
        assert_eq!(
            fs::read_to_string(&abs).unwrap(),
            original,
            "干净文件不应被改写"
        );
    }

    #[test]
    fn apply_note_body_overwrites_whole_file_when_unmanaged() {
        let vault = tmp_dir("apply-full");
        let abs = vault.join("note.md");
        fs::write(&abs, "旧内容\n").unwrap();
        let mut item = test_item();
        item.notes = "# 新整篇\n\n包含 frontmatter 在内的一切\n".to_string();

        apply_note_body(&abs, &item, &item.notes).unwrap();

        let after = fs::read_to_string(&abs).unwrap();
        assert_eq!(after, item.notes, "整篇接管应整篇覆盖");
        // 写回后读回来必须还是同一份 —— 否则双向同步会永远判定「有变化」而死循环
        assert_eq!(read_note_body(&after), item.notes, "整篇读写必须互逆");
        assert!(!after.contains(NOTES_START), "整篇模式不得注入托管标记");
    }

    #[test]
    fn apply_note_body_keeps_user_zones_when_managed() {
        let vault = tmp_dir("apply-managed");
        let abs = vault.join("note.md");
        let original = sample("# 用户原文\n\n", "app 旧笔记", "\n用户自己的话\n");
        fs::write(&abs, &original).unwrap();
        let mut item = test_item();
        item.notes = "app 新笔记".to_string();

        apply_note_body(&abs, &item, &item.notes).unwrap();

        let after = fs::read_to_string(&abs).unwrap();
        assert_eq!(read_note_body(&after), "app 新笔记");
        assert!(after.contains("# 用户原文"), "托管模式下用户原文必须保留");
        assert!(after.contains("用户自己的话"), "托管模式下用户区必须保留");
    }

    // ---- 用户自选笔记的前置内容必须被保留 ------------------------------------

    #[test]
    fn owns_frontmatter_only_for_collector_files() {
        let own = format!("---\ncollector_id: bilibili:BV1\ntitle: x\n---");
        assert!(owns_frontmatter(&own, "bilibili:BV1"));
        // 别人家的 collector_id → 不是自家文件
        assert!(!owns_frontmatter(&own, "bilibili:BV9"));
        // 用户自选的笔记：压根没有 collector frontmatter
        assert!(!owns_frontmatter("# 我的读书笔记\n\n随便写", "bilibili:BV1"));
    }
    // Windows 上 canonicalize 会给存在的路径加 \\?\ 前缀，而目标文件首次写入前
    // 父目录可能不存在、canonicalize 失败退回无前缀原始路径 —— 两侧形式不一致导致
    // 前缀比较永远 false（曾让所有导出被误拒）。这里用词法比较钉死行为，不访问磁盘。
    #[test]
    fn ensure_within_vault_accepts_child_with_uncreated_parent() {
        let vault = Path::new(r"C:\Users\lioh\Documents\lioh");
        let abs = Path::new(r"C:\Users\lioh\Documents\lioh\收藏\某标题.md");
        assert!(ensure_within_vault(vault, abs).is_ok(), "vault 内子路径应通过");
    }

    #[test]
    fn ensure_within_vault_rejects_outside_path() {
        let vault = Path::new(r"C:\Users\lioh\Documents\lioh");
        let abs = Path::new(r"C:\Users\lioh\Documents\Other\某标题.md");
        assert!(ensure_within_vault(vault, abs).is_err(), "vault 外路径应拒绝");
    }

    #[test]
    fn ensure_within_vault_rejects_parent_escape() {
        let vault = Path::new(r"C:\Users\lioh\Documents\lioh");
        let abs = Path::new(r"C:\Users\lioh\Documents\lioh\..\Other\某标题.md");
        assert!(ensure_within_vault(vault, abs).is_err(), "含 .. 逃逸应拒绝");
    }

    #[test]
    fn ensure_within_vault_rejects_absolute_subdir_escape() {
        // 模拟 subdir 填了绝对路径：vault.join(绝对路径) 会整体换成绝对路径 → 应被拒
        let vault = Path::new(r"C:\Users\lioh\Documents\lioh");
        let abs = Path::new(r"C:\Windows\System32\某标题.md");
        assert!(ensure_within_vault(vault, abs).is_err(), "绝对路径逃逸应拒绝");
    }
}
