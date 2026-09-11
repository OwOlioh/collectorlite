//! 用系统默认方式打开 URI —— 自定义协议（`obsidian://` / `orpheus://` 等）的唯一入口。
//!
//! ⚠️ 凡是「不是 http(s) 的链接」都必须走这里，**不要各写一份**（3.16 纪律：重复实现迟早 diverge，
//! 而这类平台相关代码 diverge 后极难发现）。
//!
//! Windows 下**不能**用 `webbrowser`：它在 Windows 只认「默认浏览器」—— 实现里硬编码去查 `http`
//! 协议的关联程序，然后把**任何 scheme** 都丢给浏览器，表现为「点打开却跳到浏览器」。
//! 自定义协议必须由系统按注册表里的协议关联唤起，走 ShellExecute 系列 API。

use crate::error::AppError;

/// 从 URI 里取出 scheme（`orpheus://xxx` -> `orpheus`）。取不出来时返回整个字符串，
/// 调用方拿去做注册表查询会失败 —— 那正是我们想要的结论（"这个东西打不开"）。
fn scheme_of(uri: &str) -> &str {
    match uri.split_once("://") {
        Some((scheme, _)) if !scheme.is_empty() => scheme,
        _ => uri,
    }
}

#[cfg(windows)]
pub fn open_uri_system(uri: &str) -> Result<(), AppError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SHELLEXECUTEINFOW};
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    // ── 第一道：先看协议到底注册了没有 ────────────────────────────────────
    //
    // 这一步是被实测逼出来的（2026-09-11）：**`ShellExecuteW` 的返回值完全不可信**。
    // 探针给它一个根本不存在的协议 `orpheus-nonexistent-scheme-test://abc123`，
    // 它照样返回 42（成功码），还顺手阻塞了 499 ms。
    //
    // 后果比慢严重得多：调用方是靠返回码决定要不要 fallback 浏览器的。返回码是噪声，
    // 就意味着 fallback 永远不触发 —— 协议真出问题时，用户点下去既不开客户端也不开浏览器，
    // 界面毫无反应。所以「能不能打开」必须查注册表，Shell 说了不算。
    if !is_scheme_registered(scheme_of(uri)) {
        return Err(AppError::Other(format!(
            "系统未注册 {} 协议（客户端可能未安装）",
            scheme_of(uri)
        )));
    }

    // ── 第二道：用 ShellExecuteExW + SEE_MASK_NOASYNC 唤起 ────────────────
    //
    // 换掉 `ShellExecuteW` 是纯性能考虑：同一个 `orpheus://` URI，实测
    //     首次调用  ShellExecuteW 310 ms  /  ShellExecuteExW+NOASYNC 11 ms
    // 相差近 30 倍。`ShellExecuteW` 内部会等 Shell 自己的异步收尾（协议解析 / DDE
    // 握手之类），而调用方根本不需要等它 —— `SEE_MASK_NOASYNC` 就是在说「别等我」。
    //
    // 注意别加 `SEE_MASK_NOCLOSEPROCESS`：那样会回传一个进程句柄，得由我们手动 CloseHandle，
    // 漏了就是句柄泄漏。这里不需要句柄，不传它更安全。
    const SEE_MASK_NOASYNC: u32 = 0x0000_0100;

    let wide: Vec<u16> = std::ffi::OsStr::new(uri)
        .encode_wide()
        .chain(Some(0))
        .collect();
    let wide_verb: Vec<u16> = std::ffi::OsStr::new("open")
        .encode_wide()
        .chain(Some(0))
        .collect();

    // SHELLEXECUTEINFOW 字段很多，用零初始化再逐个赋值，避免漏字段导致未定义行为。
    // `cbSize` 必须填对，否则 ShellExecuteExW 直接失败。
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOASYNC;
    info.lpVerb = wide_verb.as_ptr();
    info.lpFile = wide.as_ptr();
    info.nShow = SW_SHOWNORMAL;

    let ok = unsafe { ShellExecuteExW(&mut info) };
    if ok == 0 {
        return Err(AppError::Other(format!(
            "系统未能打开链接：{}",
            std::io::Error::last_os_error()
        )));
    }

    // 即便走到这里，`>32` 也只代表"已递交"。有了上面的注册表预检，
    // 至少能保证「被调用的那个人确实存在」，不会再出现点了完全没反应的情况。
    Ok(())
}

/// 该 scheme 在系统里有没有注册过处理器。
///
/// 只查 `HKEY_CLASSES_ROOT` 下的 `<scheme>\shell\open\command`，与 Shell 自己的查找
/// 顺序一致（HKCU\Software\Classes 会被合并进 HKCR）。查不到就当没注册。
#[cfg(windows)]
fn is_scheme_registered(scheme: &str) -> bool {
    use winreg::RegKey;
    use winreg::enums::HKEY_CLASSES_ROOT;

    if scheme.is_empty() {
        return false;
    }
    let path = format!(r"{scheme}\shell\open\command");
    RegKey::predef(HKEY_CLASSES_ROOT)
        .open_subkey(path)
        .map(|key| key.get_value::<String, _>("").is_ok())
        .unwrap_or(false)
}

#[cfg(not(windows))]
pub fn open_uri_system(uri: &str) -> Result<(), AppError> {
    webbrowser::open(uri).map_err(|e| AppError::Other(format!("无法打开链接: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheme_of_extracts_scheme() {
        assert_eq!(scheme_of("orpheus://abc"), "orpheus");
        assert_eq!(scheme_of("obsidian://open?x=1"), "obsidian");
        // http(s) 也照常解析，不做特殊化
        assert_eq!(scheme_of("https://example.com/a?b=1#c"), "https");
    }

    #[test]
    fn scheme_of_handles_garbage_without_panicking() {
        // 没有 "://"、空串、以 :// 开头 —— 都不能 panic，
        // 返回的东西拿去查注册表会失败，那正是期望结论。
        assert_eq!(scheme_of("nonsense"), "nonsense");
        assert_eq!(scheme_of(""), "");
        // 注意这里是回退到整个串（而不是空串）：没有有效 scheme 时原样返回，
        // 拿去查注册表同样查不到 —— 结论依然是"打不开"，只是错误信息里会带上原文。
        assert_eq!(scheme_of("://x"), "://x");
    }

    #[cfg(windows)]
    #[test]
    fn scheme_registration_check_agrees_with_reality() {
        // 这两个是与 Rust 侧的 deeplink 直接绑定的，探针实测过的结论：
        // orpheus 注册了，而随手编的协议一定没注册。
        assert!(is_scheme_registered("orpheus"), "本机应已注册 orpheus 协议");
        assert!(
            !is_scheme_registered("thisshouldneverexist12345"),
            "编造的协议不该被认为已注册 —— 否则 fallback 浏览器这条兜底就废了"
        );
    }

    #[cfg(windows)]
    #[test]
    fn opening_unregistered_scheme_reports_error() {
        // 这是本次优化要保住的核心契约：给不了就明说失败，让调用方能 fallback。
        // 旧实现用 ShellExecuteW 时，不存在的协议会返回 42（成功），导致静默失败。
        let err = open_uri_system("thisshouldneverexist12345://abc")
            .expect_err("未注册协议必须返回 Err");
        assert!(
            err.to_string().contains("未注册"),
            "错误信息里应说明是协议未注册，实际：{err}"
        );
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "会真的唤起网易云客户端并打断正在听的歌；想复核提速效果时手动跑：cargo test -- --ignored bench_first_open"]
    fn bench_first_open_is_fast() {
        // 为什么留 bench：`ShellExecuteW` → `ShellExecuteExW+NOASYNC` 的提速**没法用断言锁住**
        // （它不影响正确性，只影响耗时，且要真唤起客户端才测得出来）。留一个手动 bench，
        // 让以后改动这段时有办法复核，别悄悄退化回慢路径。
        //
        // 2026-09-11 基准（Python 探针调同一套 Win32 API，客户端已运行）：
        //     ShellExecuteW             首次 310 ms / 后续 9 ms
        //     ShellExecuteExW+NOASYNC   首次  11 ms / 后续 10 ms
        let uri =
            "orpheus://eyJ0eXBlIjoic29uZyIsImlkIjoiMjcwOTc4MjU1MCIsImNtZCI6InBsYXkifQ==";
        for round in 1..=3 {
            let t0 = std::time::Instant::now();
            let result = open_uri_system(uri);
            let ms = t0.elapsed().as_secs_f64() * 1000.0;
            println!(
                "第 {round} 轮: {ms:7.1} ms  result={:?}",
                result.as_ref().map(|_| "ok").map_err(|e| e.to_string())
            );
            std::thread::sleep(std::time::Duration::from_millis(800));
        }
        println!("判据：<50 ms 基本无感；~300 ms 就是用户抱怨的那种顿感。");
    }
}
