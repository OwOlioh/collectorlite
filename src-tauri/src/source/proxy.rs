//! 系统代理解析：环境变量 → Windows 注册表 → 本机常见代理端口兜底扫描。
//! 任一命中即返回 `reqwest::Proxy`；都不命中返回 `None`（调用方直连）。
//! 兜底扫描仅在「没有任何显式代理配置」时启用，避免桌面端直连被风控返回 -400。

use reqwest::Proxy;

/// 解析系统代理。返回 `Some` 时调用方应将其应用到 `reqwest::Client` 上。
pub(crate) fn resolve_system_proxy() -> Option<Proxy> {
    // 1) 环境变量（HTTPS_PROXY / HTTP_PROXY 等，大小写兼容）
    if let Some(url) = env_proxy_url() {
        if let Ok(proxy) = Proxy::all(&url) {
            eprintln!("[net] 使用环境变量代理 {url}");
            return Some(proxy);
        }
    }
    // 2) Windows 注册表 Internet Settings 代理
    #[cfg(windows)]
    if let Some(url) = windows_registry_proxy_url() {
        if let Ok(proxy) = Proxy::all(&url) {
            eprintln!("[net] 使用注册表代理 {url}");
            return Some(proxy);
        }
    }
    // 3) 本机常见代理端口兜底：仅当上述都为空时扫描，命中即用
    if let Some(url) = detect_local_proxy() {
        if let Ok(proxy) = Proxy::all(&url) {
            eprintln!("[net] 未检测到显式代理，自动使用本机代理 {url}");
            return Some(proxy);
        }
    }
    None
}

fn env_proxy_url() -> Option<String> {
    for var in [
        "HTTPS_PROXY",
        "HTTP_PROXY",
        "https_proxy",
        "http_proxy",
        "ALL_PROXY",
        "all_proxy",
    ] {
        if let Ok(value) = std::env::var(var) {
            let v = value.trim();
            if !v.is_empty() {
                return Some(if v.starts_with("http://") || v.starts_with("https://") {
                    v.to_string()
                } else {
                    format!("http://{v}")
                });
            }
        }
    }
    None
}

#[cfg(windows)]
fn windows_registry_proxy_url() -> Option<String> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
    let hkcu = winreg::RegKey::predef(HKEY_CURRENT_USER);
    let settings = hkcu
        .open_subkey_with_flags(
            "Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings",
            KEY_READ,
        )
        .ok()?;
    let enabled: u32 = settings.get_value("ProxyEnable").ok()?;
    if enabled == 0 {
        return None;
    }
    let server: String = settings.get_value("ProxyServer").ok()?;
    let mut http: Option<String> = None;
    let mut https: Option<String> = None;
    for part in server.split(';') {
        if let Some((scheme, addr)) = part.split_once('=') {
            match scheme.to_lowercase().as_str() {
                "http" => http = Some(addr.to_string()),
                "https" => https = Some(addr.to_string()),
                _ => {}
            }
        } else if !part.is_empty() {
            http = Some(part.to_string());
            https = Some(part.to_string());
        }
    }
    let addr = http.or(https)?;
    Some(if addr.starts_with("http://") || addr.starts_with("https://") {
        addr
    } else {
        format!("http://{addr}")
    })
}

/// 本机常见代理端口兜底：当没有任何显式代理配置时扫描，避免直连被风控。
/// 仅对真正在监听的端口尝试；都未监听则返回 None（调用方直连）。
fn detect_local_proxy() -> Option<String> {
    // 覆盖常见本地代理默认端口（Clash / 系统代理工具等）
    const CANDIDATE_PORTS: &[u16] =
        &[7890, 7891, 7893, 33210, 3273, 8080, 8888, 8118, 3128, 1087];
    for &port in CANDIDATE_PORTS {
        if is_local_port_listening(port) {
            return Some(format!("http://127.0.0.1:{port}"));
        }
    }
    None
}

fn is_local_port_listening(port: u16) -> bool {
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;
    let addr: SocketAddr = match format!("127.0.0.1:{port}").parse() {
        Ok(a) => a,
        Err(_) => return false,
    };
    TcpStream::connect_timeout(&addr, Duration::from_millis(120)).is_ok()
}
