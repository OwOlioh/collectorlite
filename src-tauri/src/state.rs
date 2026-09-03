use keyring::Entry;
use sqlx::SqlitePool;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU16;
use std::sync::Mutex;
use tauri::Manager;

use crate::db;
use crate::error::AppError;
use crate::models::ExternalItem;
use crate::source::bilibili::BilibiliClient;
use crate::source::csdn::CsdnClient;
use crate::source::github::GithubClient;
use crate::source::zhihu::ZhihuClient;

/// 预览阶段算好的 enriched items 缓存，供执行阶段复用，避免重复 fetch + enrich。
/// key 为 `{source}:{collection.id}`，命中即跳过二次抓取与逐条补全。
pub struct PreviewCache {
    pub key: String,
    pub items: Vec<ExternalItem>,
}

const KEYRING_SERVICE: &str = "bili-collector";
const KEYRING_USER: &str = "bilibili-cookie";
const KEYRING_ZHIHU_USER: &str = "zhihu-cookie";

pub struct AppState {
    pub pool: SqlitePool,
    pub bili: BilibiliClient,
    pub csdn: CsdnClient,
    pub github: GithubClient,
    pub zhihu: ZhihuClient,
    pub data_dir: PathBuf,
    pub import_cache: Mutex<Option<PreviewCache>>,
    /// 浏览器扩展本地桥实际占用的端口（0 = 未启动）。由 `capture::serve` 在绑定成功后写入。
    pub bridge_port: AtomicU16,
}

impl AppState {
    pub async fn new(app: &tauri::AppHandle) -> Result<Self, AppError> {
        let data_dir = app
            .path()
            .app_data_dir()
            .map_err(|error| AppError::Other(error.to_string()))?;
        let db_path = data_dir.join("bili_collector_v2.sqlite3");
        let pool = db::connect(&db_path).await?;
        let bili = BilibiliClient::new()?;
        let csdn = CsdnClient::new()?;
        let github = GithubClient::new()?;
        let zhihu = ZhihuClient::new()?;
        let cookie_file = data_dir.join("bilibili_cookie.txt");
        let zhihu_cookie_file = data_dir.join("zhihu_cookie.txt");
        let persisted = load_cookie_file(&cookie_file)
            .ok()
            .flatten()
            .or_else(|| load_cookie().ok().flatten());
        if let Some(cookie) = persisted {
            bili.set_cookie(Some(cookie));
        }
        let zhihu_persisted = load_cookie_file(&zhihu_cookie_file)
            .ok()
            .flatten()
            .or_else(|| load_zhihu_cookie().ok().flatten());
        eprintln!("[state] zhihu cookie loaded: {}", zhihu_persisted.is_some());
        if let Some(cookie) = zhihu_persisted {
            zhihu.set_cookie(Some(cookie));
        }
        Ok(Self {
            pool,
            bili,
            csdn,
            github,
            zhihu,
            data_dir,
            import_cache: Mutex::new(None),
            bridge_port: AtomicU16::new(0),
        })
    }

    pub fn save_bili_cookie(&self, cookie: Option<String>) -> Result<(), AppError> {
        let file = self.data_dir.join("bilibili_cookie.txt");
        save_cookie_file(&file, cookie.as_deref())?;
        let _ = save_cookie(cookie.as_deref());
        Ok(())
    }

    pub fn save_zhihu_cookie(&self, cookie: Option<String>) -> Result<(), AppError> {
        let file = self.data_dir.join("zhihu_cookie.txt");
        save_cookie_file(&file, cookie.as_deref())?;
        let _ = save_zhihu_cookie(cookie.as_deref());
        Ok(())
    }
}

fn load_cookie_file(path: &Path) -> Result<Option<String>, AppError> {
    match std::fs::read_to_string(path) {
        Ok(value) => {
            let value = value.trim();
            if value.is_empty() {
                Ok(None)
            } else {
                Ok(Some(value.to_string()))
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(AppError::Io(error)),
    }
}

fn save_cookie_file(path: &Path, cookie: Option<&str>) -> Result<(), AppError> {
    match cookie {
        Some(value) => {
            std::fs::write(path, value)?;
            Ok(())
        }
        None => {
            if path.exists() {
                std::fs::remove_file(path)?;
            }
            Ok(())
        }
    }
}

pub fn save_cookie(cookie: Option<&str>) -> Result<(), AppError> {
    let entry = Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .map_err(|error| AppError::Credential(error.to_string()))?;
    match cookie {
        Some(value) => entry
            .set_password(value)
            .map_err(|error| AppError::Credential(error.to_string())),
        None => {
            let _ = entry.delete_credential();
            Ok(())
        }
    }
}

pub fn load_cookie() -> Result<Option<String>, AppError> {
    let entry = Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .map_err(|error| AppError::Credential(error.to_string()))?;
    match entry.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(AppError::Credential(error.to_string())),
    }
}

pub fn save_zhihu_cookie(cookie: Option<&str>) -> Result<(), AppError> {
    let entry = Entry::new(KEYRING_SERVICE, KEYRING_ZHIHU_USER)
        .map_err(|error| AppError::Credential(error.to_string()))?;
    match cookie {
        Some(value) => entry
            .set_password(value)
            .map_err(|error| AppError::Credential(error.to_string())),
        None => {
            let _ = entry.delete_credential();
            Ok(())
        }
    }
}

pub fn load_zhihu_cookie() -> Result<Option<String>, AppError> {
    let entry = Entry::new(KEYRING_SERVICE, KEYRING_ZHIHU_USER)
        .map_err(|error| AppError::Credential(error.to_string()))?;
    match entry.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(AppError::Credential(error.to_string())),
    }
}
