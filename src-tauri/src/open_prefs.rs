//! 收藏的「打开方式偏好」——客户端优先还是浏览器优先。
//!
//! 存在的理由：网易云的 `orpheus://` 深链能唤起桌面端并直接播放（P0），但用户有时
//! 就是想看网页版（比如要顺手评论、看评论区、分享链接）。与其把两种入口硬塞进一张卡片
//! 靠 hover 区分，不如给一个显式开关。
//!
//! 设计为**按 source 存档**，而不是给网易云单开一个文件：现在只有 netease 有客户端
//! 深链，但 B站、Spotify 一类迟早会有，到时候只需要在 `VideoCard` 里多认一个 source，
//! 存档这一层完全不用改。
//!
//! 缺省值刻意设为**客户端优先** —— 这是用户 2026-09-11 拍板的默认值（DEVELOPMENT.md 9.11-1）；
//! 而且这样「老数据文件里没有这一项」也能自然落到最想要的那个分支上。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::error::AppError;

const PREF_FILE: &str = "open_prefs.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpenTarget {
    /// 唤起桌面客户端
    Client,
    /// 打开网页版
    Browser,
}

impl Default for OpenTarget {
    fn default() -> Self {
        Self::Client
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenPrefs {
    /// source -> 打开方式。**未列出的 source 走 `OpenTarget::default()`**（客户端优先），
    /// 所以老存档（没有这个字段或没这个 key）也能正常读出默认行为。
    #[serde(default)]
    pub targets: HashMap<String, OpenTarget>,
}

pub fn load_open_prefs(data_dir: &Path) -> OpenPrefs {
    let path = data_dir.join(PREF_FILE);
    match fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        // 文件不存在、或内容损坏（手写坏了、写一半断电）—— 一律回退默认，
        // 这只是一个 UI 偏好，不值得弹错误打断用户。
        Err(_) => OpenPrefs::default(),
    }
}

pub fn save_open_prefs(data_dir: &Path, prefs: &OpenPrefs) -> Result<(), AppError> {
    let path = data_dir.join(PREF_FILE);
    let json = serde_json::to_string_pretty(prefs)
        .map_err(|e| AppError::Other(format!("打开方式偏好序列化失败：{e}")))?;
    fs::write(&path, json).map_err(AppError::Io)?;
    Ok(())
}

pub fn target_for(prefs: &OpenPrefs, source: &str) -> OpenTarget {
    prefs
        .targets
        .get(source)
        .copied()
        .unwrap_or_default()
}

pub fn set_target(
    data_dir: &Path,
    source: &str,
    target: OpenTarget,
) -> Result<OpenPrefs, AppError> {
    let mut prefs = load_open_prefs(data_dir);
    prefs.targets.insert(source.to_string(), target);
    save_open_prefs(data_dir, &prefs)?;
    Ok(prefs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!("collectorlite_openprefs_{tag}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("建临时目录失败");
        dir
    }

    #[test]
    fn missing_file_falls_back_to_client_first() {
        let dir = temp_dir("missing");
        let prefs = load_open_prefs(&dir);
        // 默认值必须仍是「客户端优先」——这是用户拍板的默认行为，
        // 不能因为一次重构悄悄改成浏览器优先。
        assert_eq!(target_for(&prefs, "netease"), OpenTarget::Client);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn broken_json_falls_back_to_default() {
        let dir = temp_dir("broken");
        fs::write(dir.join(PREF_FILE), "{ 这不是 json ").expect("写坏文件失败");
        let prefs = load_open_prefs(&dir);
        assert_eq!(prefs, OpenPrefs::default());
        assert_eq!(target_for(&prefs, "netease"), OpenTarget::Client);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_target_persists_and_reloads() {
        let dir = temp_dir("roundtrip");
        let saved = set_target(&dir, "netease", OpenTarget::Browser).expect("保存失败");
        assert_eq!(target_for(&saved, "netease"), OpenTarget::Browser);

        let reloaded = load_open_prefs(&dir);
        assert_eq!(reloaded, saved, "存盘再读回来必须一致");
        assert_eq!(target_for(&reloaded, "netease"), OpenTarget::Browser);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_source_uses_default_without_polluting_file() {
        let dir = temp_dir("unknown");
        set_target(&dir, "netease", OpenTarget::Browser).expect("保存失败");
        let prefs = load_open_prefs(&dir);
        // bilibili 没配过 → 走默认；而且配置文件里不该多出一个 key
        assert_eq!(target_for(&prefs, "bilibili"), OpenTarget::Client);
        assert!(!prefs.targets.contains_key("bilibili"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn target_roundtrips_through_serde_string() {
        // 前端传字符串 "client" / "browser"，serde 必须认
        for (raw, expected) in [
            (r#""client""#, OpenTarget::Client),
            (r#""browser""#, OpenTarget::Browser),
        ] {
            let parsed: OpenTarget = serde_json::from_str(raw).expect("反序列化失败");
            assert_eq!(parsed, expected);
            assert_eq!(serde_json::to_string(&parsed).unwrap(), raw);
        }
    }
}
