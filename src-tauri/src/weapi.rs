//! 网易云 weapi 请求加密。
//!
//! 算法是公开的，本模块是独立实现（不依赖任何第三方网易云 SDK）：
//!
//! ```text
//! text   = JSON 请求体（无空格）
//! first  = AES-128-CBC(text,   key=NONCE,  iv=IV) → base64
//! params = AES-128-CBC(first,  key=seckey, iv=IV) → base64
//! encSecKey = RSA_no_padding(seckey) → 256 位 hex
//! ```
//!
//! ⚠️ 不实测根本发现不了的细节（每一条都写进单测钉死了）：
//! 1. JSON 必须**无空格**（`serde_json::to_string` 默认即紧凑格式，别用 `to_string_pretty`）。
//! 2. PKCS7 填充按**字节**算 —— 中文是 3 字节，按字符算会错。
//! 3. 第二层的输入是**第一层 base64 字符串本身**，不是它的原始字节。
//! 4. RSA 明文是密钥**反转**后的字节，且**无填充**。
//! 5. `encSecKey` 必须左填充到 **256 个 hex 字符**，短了服务端解不开。
//! 6. `csrf_token` 通常传空串即可，但字段不能少。
//!
//! 参考实现在 `tools/netease_probe/probe_api.py`（Python 探针，已跑通真实接口）。

use aes::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
use base64::Engine;
use num_bigint::BigUint;
use num_traits::Num;
use rand::Rng;

type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;

/// 第一层固定密钥（公开常量）。**必须是 16 字节**，见 `tests::constants_have_expected_lengths`。
const NONCE: &[u8] = b"0CoJUm6Qyw8W8jud";
/// 固定 IV（公开常量）
const IV: &[u8] = b"0102030405060708";
/// RSA 公钥指数（65537）
const PUBKEY_HEX: &str = "010001";
/// RSA 模数（公开常量）。**必须是 258 个 hex 字符**——曾因手工断行漏字符导致全部加密失败，
/// 所以长度由单测钉死。
const MODULUS_HEX: &str = "00e0b509f6259df8642dbc35662901477df22677ec152b5ff68ace615bb7b7\
    25152b3ab17a876aea8a5aa76d2e417629ec4ee341f56135fccf695280104e\
    0312ecbda92557c93870114af6c9d05c4f7f0c3685b7a46bee255932575cce\
    10b424d813cfe4875d3e82047b97ddef52741d546b8e289dc6935b3ece0462\
    db0a22b8e7";

fn aes_cbc_b64(plain: &[u8], key: &[u8]) -> String {
    let encryptor =
        Aes128CbcEnc::new_from_slices(key, IV).expect("key 与 iv 长度固定为 16 字节");
    let ciphertext = encryptor.encrypt_padded_vec_mut::<Pkcs7>(plain);
    base64::engine::general_purpose::STANDARD.encode(ciphertext)
}

/// RSA 无填充加密：明文反转 → 模幂 → 左填充到 256 位 hex。
pub fn rsa_no_padding(seckey: &str) -> String {
    let n = BigUint::from_str_radix(MODULUS_HEX, 16).expect("模数是编译期常量");
    let e = BigUint::from_str_radix(PUBKEY_HEX, 16).expect("指数是编译期常量");
    let reversed: Vec<u8> = seckey.as_bytes().iter().rev().copied().collect();
    let m = BigUint::from_bytes_be(&reversed);
    let c = m.modpow(&e, &n);
    format!("{:0>256}", c.to_str_radix(16))
}

/// 16 位字母数字随机密钥（与官方客户端同字符集）。
fn random_secret() -> String {
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut rng = rand::thread_rng();
    (0..16)
        .map(|_| {
            let idx = rng.gen_range(0..CHARS.len());
            CHARS[idx] as char
        })
        .collect()
}

/// 用给定密钥加密请求体，返回 `(params, encSecKey)`。固定密钥版本主要用于测试。
pub fn encrypt_with_secret(text: &str, seckey: &str) -> (String, String) {
    let first = aes_cbc_b64(text.as_bytes(), NONCE);
    let params = aes_cbc_b64(first.as_bytes(), seckey.as_bytes());
    (params, rsa_no_padding(seckey))
}

/// 用随机密钥加密请求体，返回 `(params, encSecKey)`。
pub fn encrypt_request(text: &str) -> (String, String) {
    encrypt_with_secret(text, &random_secret())
}

/// 把 JSON 请求体直接包成 `application/x-www-form-urlencoded` 表单体。
pub fn weapi_form(payload: &serde_json::Value) -> String {
    let text = serde_json::to_string(payload).unwrap_or_default();
    let (params, enc_sec_key) = encrypt_request(&text);
    url::form_urlencoded::Serializer::new(String::new())
        .append_pair("params", &params)
        .append_pair("encSecKey", &enc_sec_key)
        .finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    // 以下期望值全部由 tools/netease_probe/probe_api.py（已跑通真实接口）生成，
    // 逐字节钉死，防止"看起来对、实际差一个填充/反转"这类静默错误。
    const TEXT: &str = r#"{"id":"5124170445","n":1000,"offset":0,"total":true,"csrf_token":""}"#;
    const TEXT_CN: &str = r#"{"s":"鼓楼 赵雷","type":1,"limit":1,"offset":0,"csrf_token":""}"#;
    const SECKEY: &str = "abcdefghijklmnop";

    // 曾因手工给常量断行时漏字符，导致 AES 全部 InvalidLength —— 用长度断言钉死。
    // 这类错误不会编译报错，只会让接口返回"解密失败"，极难定位。
    #[test]
    fn constants_have_expected_lengths() {
        assert_eq!(NONCE.len(), 16, "NONCE 必须是 16 字节（AES-128 密钥）");
        assert_eq!(IV.len(), 16, "IV 必须是 16 字节");
        assert_eq!(MODULUS_HEX.len(), 258, "RSA 模数必须是 258 个 hex 字符");
        assert!(
            BigUint::from_str_radix(MODULUS_HEX, 16).is_ok(),
            "模数必须是合法 hex"
        );
    }

    #[test]
    fn first_layer_matches_python() {
        assert_eq!(
            aes_cbc_b64(TEXT.as_bytes(), NONCE),
            "b3hpx+dAw5cfsHs6b+Iz/6tsgFVVypvkx8HjhBPxwRN7nVUHxd7uay+GyzaP2sn5l//SJXnop7gH0Ks6WiIN9FzDF09M/jDJxjb9QmFrQOc="
        );
    }

    // 中文负载：钉死"PKCS7 按字节填充"这一点（按字符算会得到完全不同的结果）
    #[test]
    fn first_layer_handles_multibyte_utf8() {
        assert_eq!(
            aes_cbc_b64(TEXT_CN.as_bytes(), NONCE),
            "8FRKDPt2YnqM9ZAkRsk/RPogKLjTdEdSIVwdmGl12yrmZozJUWasr5Z77q23ziSKZ03snxJnLBNm9pNpXcGQjJDtwhTP/7JP6is9vPDPtsQ="
        );
    }

    #[test]
    fn params_matches_python() {
        let (params, _) = encrypt_with_secret(TEXT, SECKEY);
        assert_eq!(
            params,
            "CAF7uyLyyE6qgMptV8dtXvkiurBueOdSu0CHlI3KUjzVjL7b7TwRTNTp8qQmAC04fN4nD4jLcDWoyUEXVdrbQnLSOndPjhYSK3WiCU1q0piCZG9zObbtRhlHA2wkhBroaQtgDQmlQuNdYvjPwjtH8A=="
        );
    }

    #[test]
    fn enc_sec_key_matches_python() {
        let (_, enc) = encrypt_with_secret(TEXT, SECKEY);
        assert_eq!(
            enc,
            "d15a1683c992095d0c234c19966605c5c5964911268bbeda8cb8d08d834913e59d53b32358903a121b5fca784c1f5ae44951fd02524df58ecc98e52cc7cf8689b42c2e93ddf05b0592512d87f5960467e2f086c018849d76014d323500e30f13ef4cafbb0cf5a66731a3f1776c75ca35d0062dac70a3e33245afabcf47938487"
        );
    }

    #[test]
    fn enc_sec_key_is_padded_to_256_hex_chars() {
        let (_, enc) = encrypt_request("{}");
        assert_eq!(enc.len(), 256, "encSecKey 必须是 256 个 hex 字符");
        assert!(
            enc.chars().all(|c| c.is_ascii_hexdigit()),
            "encSecKey 只应包含 hex 字符"
        );
    }

    #[test]
    fn random_secret_differs_each_call() {
        let (a, _) = encrypt_request(TEXT);
        let (b, _) = encrypt_request(TEXT);
        assert_ne!(a, b, "每次请求应使用新的随机密钥");
    }

    #[test]
    fn form_body_is_urlencoded() {
        let body = weapi_form(&serde_json::json!({"id": "1", "csrf_token": ""}));
        assert!(body.starts_with("params="), "字段名必须是 params");
        assert!(body.contains("&encSecKey="), "必须带 encSecKey");
        assert!(
            !body.contains('+') && !body.contains(' '),
            "base64 里的 + 必须被转义：{body}"
        );
    }
}
