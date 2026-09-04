use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_yaml;

use crate::error::AppError;
use crate::models::VideoItem;

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
    let fm = build_frontmatter(item)?;
    Ok(format!("{fm}\n{NOTES_START}\n{notes}\n{NOTES_END}\n"))
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

/// 重写已存在笔记：只替换托管区，保留 END 标记之后的用户区。
/// 返回 None 表示托管标记被用户手动移除，调用方应跳过同步。
fn update_existing_note(path: &Path, item: &VideoItem, notes: &str) -> Result<Option<()>, AppError> {
    let old = fs::read_to_string(path).map_err(AppError::Io)?;
    let (_, end_idx) = match (old.find(NOTES_START), old.find(NOTES_END)) {
        (Some(s), Some(e)) => (s, e),
        _ => return Ok(None),
    };
    let user_zone_start = end_idx + NOTES_END.len();
    let user_zone = if user_zone_start <= old.len() {
        old[user_zone_start..].to_string()
    } else {
        String::new()
    };
    let fm = build_frontmatter(item)?;
    let content = format!("{fm}\n{NOTES_START}\n{notes}\n{NOTES_END}{user_zone}");
    write_utf8_no_bom(path, &content)?;
    Ok(Some(()))
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

/// 把一条收藏同步成 vault 内的 md 笔记。
/// - `Ok(Some(rel))`：已写入，rel 为 vault 内相对路径
/// - `Ok(None)`：跳过（托管标记被用户移除）
/// - `Err`：写入失败
pub fn write_or_update_note(
    settings: &ObsidianSettings,
    item: &VideoItem,
) -> Result<Option<String>, AppError> {
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
            return match update_existing_note(&target, item, &item.notes)? {
                Some(()) => Ok(Some(rel.clone())),
                None => Ok(None),
            };
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
    Ok(Some(rel))
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

/// 用系统默认方式打开 URI。
///
/// Windows 下**不能**用 `webbrowser`：它在 Windows 只认「默认浏览器」—— 实现里硬编码
/// 去查 `http` 协议的关联程序，然后把**任何 scheme**（包括 `obsidian://`）都丢给浏览器，
/// 表现为「点打开却跳到浏览器」。Obsidian 链接必须走 `ShellExecuteW`，由系统按注册表里
/// `obsidian://` 协议关联唤起 Obsidian.exe。
#[cfg(windows)]
fn open_uri_system(uri: &str) -> Result<(), AppError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let wide: Vec<u16> = std::ffi::OsStr::new(uri).encode_wide().chain(Some(0)).collect();
    // hwnd / lpOperation / lpParameters / lpDirectory 传空：用系统为该协议注册的默认动作打开
    let code = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            std::ptr::null(),
            wide.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecute 返回值 <= 32 表示失败
    if (code as isize) <= 32 {
        Err(AppError::Other(format!(
            "系统未能打开链接（错误码 {}）：{}",
            code as isize,
            std::io::Error::last_os_error()
        )))
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn open_uri_system(uri: &str) -> Result<(), AppError> {
    webbrowser::open(uri).map_err(|e| AppError::Other(format!("无法打开链接: {e}")))
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
