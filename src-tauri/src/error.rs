use std::fmt;

#[derive(Debug)]
pub enum AppError {
    Bili(i64, String),
    Db(sqlx::Error),
    Http(reqwest::Error),
    Json(serde_json::Error),
    InvalidInput(String),
    NotFound(String),
    AuthRequired,
    RiskControl(String),
    Credential(String),
    Io(std::io::Error),
    Other(String),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::Bili(code, message) => write!(f, "B 站接口错误 {code}: {message}"),
            AppError::Db(error) => write!(f, "数据库错误: {error}"),
            AppError::Http(error) => write!(f, "网络错误: {error}"),
            AppError::Json(error) => write!(f, "数据解析错误: {error}"),
            AppError::InvalidInput(message) => write!(f, "{message}"),
            AppError::NotFound(message) => write!(f, "{message}"),
            AppError::AuthRequired => write!(f, "需要登录后才能获取内容，请先登录"),
            AppError::RiskControl(message) => write!(f, "{message}"),
            AppError::Credential(message) => write!(f, "{message}"),
            AppError::Io(error) => write!(f, "文件错误: {error}"),
            AppError::Other(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for AppError {}

impl From<sqlx::Error> for AppError {
    fn from(value: sqlx::Error) -> Self {
        AppError::Db(value)
    }
}

impl From<reqwest::Error> for AppError {
    fn from(value: reqwest::Error) -> Self {
        AppError::Http(value)
    }
}

impl From<serde_json::Error> for AppError {
    fn from(value: serde_json::Error) -> Self {
        AppError::Json(value)
    }
}

impl From<std::io::Error> for AppError {
    fn from(value: std::io::Error) -> Self {
        AppError::Io(value)
    }
}
