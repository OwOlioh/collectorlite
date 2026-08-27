use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::error::AppError;

const MIXIN_KEY_ENC_TAB: [u8; 64] = [
    46, 47, 18, 2, 53, 8, 23, 32, 15, 50, 10, 31, 58, 3, 45, 35, 27, 43, 5, 49, 33, 9, 42, 19, 29,
    28, 14, 39, 12, 38, 41, 13, 37, 48, 7, 16, 24, 55, 40, 61, 26, 17, 0, 1, 60, 51, 30, 4, 22, 25,
    54, 21, 56, 59, 6, 63, 57, 62, 11, 36, 20, 34, 44, 52,
];

#[derive(Debug, Clone)]
pub struct WbiKeys {
    pub img_key: String,
    pub sub_key: String,
}

impl WbiKeys {
    pub fn mixin_key(&self) -> String {
        let raw = format!("{}{}", self.img_key, self.sub_key);
        MIXIN_KEY_ENC_TAB
            .iter()
            .map(|&index| raw.as_bytes()[index as usize] as char)
            .take(32)
            .collect()
    }
}

pub fn extract_wbi_keys(nav: &Value) -> Result<WbiKeys, AppError> {
    let wbi = nav
        .pointer("/data/wbi_img")
        .and_then(Value::as_object)
        .ok_or_else(|| AppError::Other("B 站未返回 WBI 密钥".into()))?;
    let img_url = wbi
        .get("img_url")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::Other("B 站未返回 WBI img_url".into()))?;
    let sub_url = wbi
        .get("sub_url")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::Other("B 站未返回 WBI sub_url".into()))?;
    Ok(WbiKeys {
        img_key: key_from_url(img_url)?,
        sub_key: key_from_url(sub_url)?,
    })
}

fn key_from_url(value: &str) -> Result<String, AppError> {
    let stem = value
        .rsplit('/')
        .next()
        .ok_or_else(|| AppError::Other("无效的 WBI 密钥地址".into()))?;
    Ok(stem
        .split('.')
        .next()
        .ok_or_else(|| AppError::Other("无效的 WBI 密钥文件名".into()))?
        .to_string())
}

pub fn signed_query(keys: &WbiKeys, mut params: Vec<(String, String)>) -> Vec<(String, String)> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    params.push(("wts".into(), timestamp.to_string()));
    params.sort_by(|a, b| a.0.cmp(&b.0));

    let query = params
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                encode_query_component(key),
                encode_query_component(value)
            )
        })
        .collect::<Vec<_>>()
        .join("&");
    let mixin_key = keys.mixin_key();
    let hash = md5::compute(format!("{query}{mixin_key}").as_bytes());
    params.push(("w_rid".into(), format!("{hash:x}")));
    params
}

pub fn build_query_string(params: &[(String, String)]) -> String {
    params
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                encode_query_component(key),
                encode_query_component(value)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

pub fn encode_query_component(value: &str) -> String {
    let filtered: String = value
        .chars()
        .filter(|character| !matches!(character, '!' | '\'' | '(' | ')' | '*'))
        .collect();
    let mut output = String::with_capacity(filtered.len());
    for byte in filtered.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            output.push(byte as char);
        } else {
            output.push_str(&format!("%{byte:02X}"));
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixin_key_is_stable() {
        let keys = WbiKeys {
            img_key: "7cd084941338484aae1ad9425b84077c".into(),
            sub_key: "4932caff0ff746eab6f01bf08b70ac45".into(),
        };
        assert_eq!(keys.mixin_key(), "ea1db124af3c7062474693fa704f4ff8");
    }

    #[test]
    fn query_encoding_uses_uppercase_space_and_filters_special_chars() {
        assert_eq!(encode_query_component("a b!c"), "a%20bc");
    }
}
