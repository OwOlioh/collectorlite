//! 当前播放曲目 + 速记浮窗。
//!
//! ## 曲目从哪来：读网易云主窗口标题
//!
//! 背景（DEVELOPMENT.md 9.2.5）：网易云 PC 端**不注册 SMTC**，
//! `GlobalSystemMediaTransportControlsSessionManager.GetSessions()` 播放中仍是 0 个会话。
//! 剩下的路只有两条：注入进程（违反 ToS 且客户端一更新就废），或者读窗口标题。这里走后者。
//!
//! ## 窗口形态：按需创建 / 用完销毁
//!
//! 旧版（提交 `0a2e84c`，备份在 `fwbackup` 分支）是**常驻窄条 + hide/show 切显隐**，
//! 做的时候在「页面看着活着但点不动」上卡了很久。2026-09-12 重做时改成本形态，理由见
//! DEVELOPMENT.md 9.14，这里只记结论：
//!
//! - **不复用窗口**：每次都是新建，关闭即销毁。不存在 hide/show 状态漂移。
//! - **不常驻**：不看了就彻底没有这个窗口，也不占内存。
//! - **不轮询**：面板打开那一瞬间读一次标题就够，不需要 1.5 s 的轮询 + 事件广播。
//! - **不拖动**：贴右边固定，因此不需要 `start-dragging`，也不会撞上
//!   `data-tauri-drag-region` 吃掉 click 那个坑（同一元素上 drag 与 click 互斥）。

use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// 浮窗的 window label。前端与 Rust 两侧都靠它找到这个窗口。
pub const WINDOW_LABEL: &str = "nowplaying";

/// 面板尺寸（逻辑像素）。窄一点像侧边栏，但要放得下批注框和标签。
pub const PANEL_WIDTH: f64 = 360.0;
pub const PANEL_HEIGHT: f64 = 560.0;

/// 浮窗开关的配置文件名。放在 data_dir 而不是 localStorage ——
/// 全局快捷键注册发生在 Rust 启动阶段，那时候前端还没起来。
const PREFS_FILE: &str = "nowplaying_prefs.json";

/// 默认快捷键（用户 2026-09-12 改定 Ctrl+Alt+S；偏好文件从未存过旧值，直接换默认即可）。
pub const DEFAULT_HOTKEY: &str = "Ctrl+Alt+S";

// ── 曲目 ────────────────────────────────────────────────────────────────────

/// 从窗口标题解析出的曲目。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackInfo {
    pub title: String,
    /// 歌手，多个用 `/` 分隔（与网易云标题保持一致）
    pub artist: String,
    /// 原始窗口标题，排查解析问题时要用
    pub raw_title: String,
}

/// 客户端没在播歌时的窗口标题。命中这些就不算曲目。
///
/// ⚠️ 这份清单是**实测 + 保守兜底**：拿不准的一律不放进来，
/// 因为误判成 idle 会让面板静默失去曲目，而误判成曲目最多是多显示一条。
const IDLE_TITLES: &[&str] = &[
    "网易云音乐",
    "NeteaseMusicDesktop",
    "Netease Cloud Music",
];

/// 客户端**在跑但标题里没有曲目**的情况（实测：迷你模式下主窗口标题就是「迷你播放器」）。
///
/// 这些不算 idle（客户端明明在跑），而是「读不到」。UI 必须给出**准确**提示 ——
/// 若笼统显示「未在播放」，用户会因为歌在响而以为面板坏了。
const NO_TRACK_TITLES: &[&str] = &["迷你播放器", "迷你模式", "MiniPlayer", "Mini Player"];

/// 解析窗口标题 → 曲目。
///
/// 标题形如 `曲名 - 歌手A/歌手B`。用 `rsplit(" - ", 1)` 反向切是因为
/// **曲名本身可能含有 " - "**（电子乐尤其常见），歌手一定在最后一段。
///
/// 解析不出来一律返回 `None`：**宁可认不出，也不要猜**（DEVELOPMENT.md 9.8）。
pub fn parse_track_title(raw: &str) -> Option<TrackInfo> {
    let raw_title = raw.trim();
    if raw_title.is_empty() {
        return None;
    }
    if IDLE_TITLES
        .iter()
        .any(|idle| raw_title.eq_ignore_ascii_case(idle))
    {
        return None;
    }

    let (title, artist) = raw_title.rsplit_once(" - ")?;
    let title = title.trim();
    let artist = artist.trim();
    if title.is_empty() || artist.is_empty() {
        return None;
    }
    Some(TrackInfo {
        title: title.to_string(),
        artist: artist.to_string(),
        raw_title: raw_title.to_string(),
    })
}

/// 给「读得到曲目标题」以外的情况一句准确提示。
fn hint_for_title(raw: &str) -> Option<String> {
    if NO_TRACK_TITLES.iter().any(|t| raw.eq_ignore_ascii_case(t)) {
        return Some(format!(
            "网易云处于「{raw}」，标题里没有曲目 —— 切回普通窗口就能读到"
        ));
    }
    None
}

/// 当前曲目的完整状态。刻意**不**返回 `Option<TrackInfo>`：
/// `None` 会把「没启动」和「读不到」压成同一种情况，前端没法给出有用的提示。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NowPlayingState {
    pub track: Option<TrackInfo>,
    /// 有 track 时为 null；否则是针对当前情况的具体提示（迷你模式等）
    pub hint: Option<String>,
}

/// 读当前状态：优先给曲目，读不到就带上原因。
pub fn current_state() -> NowPlayingState {
    match read_netease_window_title() {
        Some(raw) => match parse_track_title(&raw) {
            Some(track) => NowPlayingState {
                track: Some(track),
                hint: None,
            },
            None => NowPlayingState {
                track: None,
                hint: hint_for_title(&raw).or(Some("网易云未在播放".to_string())),
            },
        },
        None => NowPlayingState {
            track: None,
            hint: None,
        },
    }
}

/// 读网易云主窗口标题（Windows）。其它平台不支持，返回 `None`。
pub fn read_netease_window_title() -> Option<String> {
    #[cfg(windows)]
    {
        read_netease_window_title_windows()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

#[cfg(windows)]
fn read_netease_window_title_windows() -> Option<String> {
    let hwnd = find_netease_main_hwnds().into_iter().next()?;
    get_window_text(hwnd)
}

/// 取一个窗口的标题文本（Windows）。
#[cfg(windows)]
fn get_window_text(hwnd: isize) -> Option<String> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowTextLengthW, GetWindowTextW};

    let hwnd = hwnd as _;
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len == 0 {
            return None;
        }
        let mut buf = vec![0u16; (len + 1) as usize];
        let written = GetWindowTextW(hwnd, buf.as_mut_ptr(), len + 1);
        if written == 0 {
            return None;
        }
        let title = OsString::from_wide(&buf[..written as usize])
            .to_string_lossy()
            .trim()
            .to_string();
        if title.is_empty() {
            None
        } else {
            Some(title)
        }
    }
}

/// 枚举顶层窗口，找出网易云**主窗口**的句柄（可见 + 类名 `OrpheusBrowserHost` + 属于 cloudmusic.exe）。
///
/// ⚠️ 必须按类名筛：`MiniPlayer` / `MiniVolume` / `DesktopLyrics` / `SystemHintWindow`
/// 同样属于 cloudmusic.exe 且都带标题，只有 `OrpheusBrowserHost` 才是曲目/主窗口来源。
#[cfg(windows)]
fn find_netease_main_hwnds() -> Vec<isize> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    use windows_sys::Win32::Foundation::{BOOL, CloseHandle, HWND, LPARAM, MAX_PATH};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindowThreadProcessId, IsWindowVisible,
    };

    /// 只有这个窗口类是主窗口（曲目来源）。
    const TRACK_WINDOW_CLASS: &str = "OrpheusBrowserHost";

    struct Collector {
        hwnds: Vec<isize>,
    }

    unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let collector = &mut *(lparam as *mut Collector);

        if IsWindowVisible(hwnd) == 0 {
            return 1;
        }

        // 先按窗口类过滤：只有主窗口是曲目来源
        let mut class_buf = vec![0u16; 256];
        let written = GetClassNameW(hwnd, class_buf.as_mut_ptr(), class_buf.len() as i32);
        if written == 0 {
            return 1;
        }
        let class = OsString::from_wide(&class_buf[..written as usize])
            .to_string_lossy()
            .to_string();
        if !class.eq_ignore_ascii_case(TRACK_WINDOW_CLASS) {
            return 1;
        }

        // 再按进程镜像名确认是网易云自己
        let mut pid = 0u32;
        if GetWindowThreadProcessId(hwnd, &mut pid) == 0 {
            return 1;
        }
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return 1;
        }
        let mut size = MAX_PATH as u32;
        let mut buf = vec![0u16; size as usize];
        let ok = QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut size);
        let _ = CloseHandle(handle);
        if ok == 0 {
            return 1;
        }
        let exe = OsString::from_wide(&buf[..size as usize])
            .to_string_lossy()
            .to_ascii_lowercase();
        if !exe.ends_with("cloudmusic.exe") {
            return 1;
        }

        collector.hwnds.push(hwnd as isize);
        1
    }

    let mut collector = Collector { hwnds: Vec::new() };
    unsafe {
        EnumWindows(
            Some(enum_cb),
            &mut collector as *mut Collector as LPARAM,
        );
    }
    collector.hwnds
}

/// 某个 pid 的进程镜像名是否以指定后缀结尾（不区分大小写）。
#[cfg(windows)]
#[allow(dead_code)]
fn process_image_ends_with(pid: u32, suffix: &str) -> bool {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    use windows_sys::Win32::Foundation::{CloseHandle, MAX_PATH};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    if pid == 0 {
        return false;
    }
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return false;
    }
    let mut size = MAX_PATH as u32;
    let mut buf = vec![0u16; size as usize];
    let ok = unsafe { QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut size) };
    unsafe { let _ = CloseHandle(handle); };
    if ok == 0 {
        return false;
    }
    let exe = OsString::from_wide(&buf[..size as usize])
        .to_string_lossy()
        .to_ascii_lowercase();
    exe.ends_with(suffix)
}

// ── 播放状态检测 ─────────────────────────────────────────────────────────────

/// 网易云是否**正在出声**（暂停 / 停止 / 没开 = false）。
///
/// 窗口标题里没有播放进度，暂停与否标题也不变；唯一可靠的非注入信号是 WASAPI
/// 音频会话状态：会话 Active = 该进程正在渲染音频，一暂停就变 Inactive。
/// 按进程**镜像名**匹配会话（网易云有多个 cloudmusic.exe 子进程，
/// 音频会话挂在哪个上面不去赌）。找不到任何匹配一律 false —— 宁可让计时器停走。
#[allow(dead_code)]
pub fn netease_is_playing() -> bool {
    #[cfg(windows)]
    {
        netease_is_playing_windows().unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(windows)]
fn netease_is_playing_windows() -> Option<bool> {
    use windows::core::Interface;
    use windows::Win32::Media::Audio::{
        AudioSessionStateActive, DEVICE_STATE_ACTIVE, IAudioSessionControl2,
        IMMDeviceEnumerator, IAudioSessionManager2, MMDeviceEnumerator, eRender,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
    };

    unsafe {
        let init_ok = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
        // 闭包内统一走一遍枚举，结尾按 init 成败配平 CoUninitialize
        let result = (|| -> windows::core::Result<bool> {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;

            // ⚠️ 必须扫**所有**活动渲染设备：网易云的音频会话挂在哪个设备上是不确定的
            //（实测它挂在某个非默认设备上，只扫默认端点必然漏检）。
            let devices = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
            let device_count = devices.GetCount()?;
            for d in 0..device_count {
                let Ok(device) = devices.Item(d) else {
                    continue;
                };
                let Ok(manager) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None)
                else {
                    continue;
                };
                let Ok(sessions) = manager.GetSessionEnumerator() else {
                    continue;
                };
                let Ok(session_count) = sessions.GetCount() else {
                    continue;
                };
                for i in 0..session_count {
                    let Ok(control) = sessions.GetSession(i) else {
                        continue;
                    };
                    let Ok(control2) = control.cast::<IAudioSessionControl2>() else {
                        continue;
                    };
                    let Ok(pid) = control2.GetProcessId() else {
                        continue;
                    };
                    if !process_image_ends_with(pid, "cloudmusic.exe") {
                        continue;
                    }
                    if control.GetState()? == AudioSessionStateActive {
                        return Ok(true);
                    }
                }
            }
            Ok(false)
        })();
        if init_ok {
            CoUninitialize();
        }
        Some(result.unwrap_or(false))
    }
}


// ── 浮窗（按需创建 / 用完销毁） ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FloatPrefs {
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    hotkey: Option<String>,
}

fn default_true() -> bool {
    true
}

impl Default for FloatPrefs {
    fn default() -> Self {
        Self {
            enabled: true,
            hotkey: None,
        }
    }
}

fn load_prefs(data_dir: &std::path::Path) -> FloatPrefs {
    std::fs::read_to_string(data_dir.join(PREFS_FILE))
        .ok()
        .and_then(|text| serde_json::from_str::<FloatPrefs>(&text).ok())
        .unwrap_or_default()
}

/// 功能开关是否打开。**默认开**。文件缺失 / JSON 损坏一律按「开」处理：
/// 这只是 UI 偏好，不值得弹错误打断用户。
pub fn load_enabled(data_dir: &std::path::Path) -> bool {
    load_prefs(data_dir).enabled
}

pub fn set_enabled(data_dir: &std::path::Path, enabled: bool) -> Result<(), AppError> {
    let mut prefs = load_prefs(data_dir);
    prefs.enabled = enabled;
    save_prefs(data_dir, &prefs)
}

/// 用户自定义的快捷键；没配过就用默认值。
pub fn load_hotkey(data_dir: &std::path::Path) -> String {
    load_prefs(data_dir)
        .hotkey
        .filter(|h| !h.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_HOTKEY.to_string())
}

fn save_prefs(data_dir: &std::path::Path, prefs: &FloatPrefs) -> Result<(), AppError> {
    let text = serde_json::to_string_pretty(prefs)
        .map_err(|e| AppError::Other(format!("序列化浮窗开关失败：{e}")))?;
    std::fs::write(data_dir.join(PREFS_FILE), text).map_err(AppError::Io)
}

/// 打开速记面板。已存在就直接置前（正常流程下不该发生，属于兜底）。
pub fn open_window(app: &tauri::AppHandle) -> Result<(), AppError> {
    use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

    if let Some(win) = app.get_webview_window(WINDOW_LABEL) {
        win.show().ok();
        let _ = win.set_focus();
        return Ok(());
    }

    // debug 下可用 NP_PANEL_URL 覆盖面板地址，用来区分「第二个 webview 坏了」还是「这个页面坏了」：
    // 设成 `/` 会把主窗口的页面塞进面板 —— 若仍空白，就是 webview 的问题，与页面无关。
    #[cfg(debug_assertions)]
    let panel_url = std::env::var("NP_PANEL_URL").unwrap_or_else(|_| "/nowplaying.html".into());
    #[cfg(not(debug_assertions))]
    let panel_url = "/nowplaying.html".to_string();

    let win = WebviewWindowBuilder::new(app, WINDOW_LABEL, WebviewUrl::App(panel_url.into()))
    .title("速记")
    .inner_size(PANEL_WIDTH, PANEL_HEIGHT)
    .resizable(false)
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .build()
    .map_err(|e| AppError::Other(format!("创建速记面板失败：{e}")))?;

    // 优先吸附网易云主窗口（并启动实时跟随）；网易云没在跑就回落屏幕右缘
    snap_or_edge(&win, app);
    let _ = win.set_focus();
    Ok(())
}

/// 关闭面板 —— **销毁，不是隐藏**。
///
/// 这是本形态的关键：窗口不留到下次，也就不存在「隐藏后再显示」那一类状态问题。
pub fn close_window(app: &tauri::AppHandle) {
    use tauri::Manager;
    stop_snap_watch();
    if let Some(win) = app.get_webview_window(WINDOW_LABEL) {
        let _ = win.destroy();
    }
}

// ---------------------------------------------------------------------------
// 全局快捷键用的入口。
//
// ⚠️ IPC 命令侧**不要**用这里的函数 —— 那两个命令必须是 `async`（见 commands.rs 的注释）：
//    同步命令跑在主线程，而 `WebviewWindowBuilder::build()` 要等 WebView 初始化，依赖主线程继续泵
//    消息 → 自锁，新窗口白屏且无响应（tauri-apps/tauri#13963）。`async` 命令跑在 tokio 线程池上，没事。
//
// 快捷键回调不走 IPC，但它所在的线程不确定，统一丢到后台线程最稳。
// ---------------------------------------------------------------------------

fn spawn_window_op<F>(op: F)
where
    F: FnOnce() + Send + 'static,
{
    // 连点 / 连按快捷键时可能并发进来，而「边建边删」没有意义 —— 直接丢弃后到的那个。
    // 窗口操作是毫秒级，丢弃的那一下用户感知不到。
    static BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    use std::sync::atomic::Ordering;
    if BUSY.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        op();
        BUSY.store(false, Ordering::SeqCst);
    });
}

/// 快捷键用：有就销毁、没有就新建。判断和执行都放在后台线程，避免和建窗口打架。
pub fn toggle_window_async(app: &tauri::AppHandle) {
    use tauri::Manager;
    let app = app.clone();
    spawn_window_op(move || {
        if app.get_webview_window(WINDOW_LABEL).is_some() {
            close_window(&app);
        } else if let Err(e) = open_window(&app) {
            eprintln!("[nowplaying] 打开面板失败：{e}");
        }
    });
}

/// 把面板摆到主显示器右侧、垂直居中；拿不到显示器信息就保持默认位置，不算错误。
///
/// 注意换算：`monitor.size()` 给的是**物理**像素，而 `set_position` 传逻辑像素，
/// 高 DPI 屏上不除 scale factor 会飞到屏幕外。
fn place_at_edge(win: &tauri::WebviewWindow) {
    use tauri::LogicalPosition;

    let Ok(Some(monitor)) = win.current_monitor() else {
        return;
    };
    let scale = monitor.scale_factor();
    if scale <= 0.0 {
        return;
    }
    let size = monitor.size();
    let screen_w = size.width as f64 / scale;
    let screen_h = size.height as f64 / scale;
    let x = (screen_w - PANEL_WIDTH - 16.0).max(0.0);
    let y = ((screen_h - PANEL_HEIGHT) / 2.0).max(0.0);
    let _ = win.set_position(LogicalPosition::new(x, y));
}

// ── 吸附网易云主窗口（实时跟随） ─────────────────────────────────────────────
//
// 形态：SetWinEventHook **事件驱动**，不轮询。监听三件事：
//   · 网易云窗口移动/缩放（LOCATIONCHANGE）→ 面板立刻跟着挪
//   · 网易云最小化 / 隐藏 / 销毁 → 面板跟着销毁（用户拍板：失主即关）
// 面板销毁时停掉 hook，不破坏「按需创建 / 用完销毁」的形态。

/// 面板与网易云窗口的间隙（物理像素，0 = 紧贴）。
#[cfg(windows)]
const SNAP_GAP: f64 = 0.0;

#[cfg(windows)]
static SNAP_APP: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();

#[cfg(windows)]
struct SnapWatch {
    thread_id: u32,
    /// 被跟随的网易云主窗口句柄（isize 形式）。
    hwnd: isize,
}
// hook 句柄不存进结构体：Unhook 只能由 watch 线程自己做（在消息循环退出后），
// 存下来反而诱导别处去 Unhook —— 在 hook 回调里反注册自身是不安全的。

#[cfg(windows)]
static SNAP_WATCH: std::sync::Mutex<Option<SnapWatch>> = std::sync::Mutex::new(None);

// 事件常量：不引 windows-sys 的路径（各版本常量归属模块有差异），直接按 MSDN 值定义。
#[cfg(windows)]
const EVENT_SYSTEM_MINIMIZESTART: u32 = 0x0016;
#[cfg(windows)]
const EVENT_OBJECT_DESTROY: u32 = 0x8001;
#[cfg(windows)]
const EVENT_OBJECT_HIDE: u32 = 0x8002;
#[cfg(windows)]
const EVENT_OBJECT_LOCATIONCHANGE: u32 = 0x800B;
#[cfg(windows)]
const OBJID_WINDOW: i32 = 0;
#[cfg(windows)]
const WINEVENT_OUTOFCONTEXT: u32 = 0x0000;

/// 某个 hwnd 的可见矩形（物理像素）。优先 DWM 扩展边界 —— `GetWindowRect`
/// 对 DWM 窗口会把不可见的阴影边距也算进去，贴边会贴出一道缝。
#[cfg(windows)]
fn window_frame_rect_windows(hwnd: isize) -> Option<(i32, i32, i32, i32)> {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect;

    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    let hwnd = hwnd as _;
    let dwm_ok = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS as u32,
            &mut rect as *mut RECT as *mut core::ffi::c_void,
            std::mem::size_of::<RECT>() as u32,
        ) == 0
    };
    if !dwm_ok {
        // DWM 拿不到就退回 GetWindowRect（可能含阴影，但总比没有强）
        if unsafe { GetWindowRect(hwnd, &mut rect) } == 0 {
            return None;
        }
    }
    if rect.right <= rect.left || rect.bottom <= rect.top {
        return None;
    }
    Some((rect.left, rect.top, rect.right, rect.bottom))
}

/// 把面板贴到网易云窗口左缘（左侧放不下就贴右缘），垂直居中于网易云。
/// 返回 false 表示网易云不可用（没找到主窗口 / 矩形非法），调用方应回落 `place_at_edge`。
#[cfg(windows)]
fn position_near(win: &tauri::WebviewWindow, hwnd: isize) -> bool {
    use tauri::PhysicalPosition;

    let Some((l, t, r, b)) = window_frame_rect_windows(hwnd) else {
        return false;
    };
    let (pw, ph) = match win.outer_size() {
        Ok(s) => (s.width as f64, s.height as f64),
        Err(_) => return false,
    };
    let ne_left = l as f64;
    let ne_top = t as f64;
    let ne_w = (r - l) as f64;
    let ne_h = (b - t) as f64;

    // 网易云所在显示器（按其中心点找），用来夹取坐标，防止面板落到可视区外
    let cx = (ne_left + ne_w / 2.0) as i32;
    let cy = (ne_top + ne_h / 2.0) as i32;
    let monitor = win.available_monitors().ok().and_then(|ms| {
        ms.into_iter().find(|m| {
            let pos = m.position();
            let sz = m.size();
            cx >= pos.x
                && cx < pos.x + sz.width as i32
                && cy >= pos.y
                && cy < pos.y + sz.height as i32
        })
    });

    // 默认贴左缘；会伸到显示器外就换到右缘
    let mut x = ne_left - pw - SNAP_GAP;
    if let Some(m) = &monitor {
        if x < m.position().x as f64 {
            x = ne_left + ne_w + SNAP_GAP;
        }
    }
    let mut y = ne_top + (ne_h - ph) / 2.0;
    if let Some(m) = &monitor {
        let mon_top = m.position().y as f64;
        let mon_bottom = mon_top + m.size().height as f64;
        y = y.clamp(mon_top, (mon_bottom - ph).max(mon_top));
    }

    let _ = win.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
    true
}

/// 打开面板时吸附网易云；失败（网易云没在跑）回落屏幕右缘。成功则启动跟随。
#[cfg(windows)]
fn snap_or_edge(win: &tauri::WebviewWindow, app: &tauri::AppHandle) {
    if let Some(hwnd) = find_netease_main_hwnds().into_iter().next() {
        if position_near(win, hwnd) {
            stop_snap_watch();
            start_snap_watch(app, hwnd);
            return;
        }
    }
    place_at_edge(win);
}

#[cfg(not(windows))]
fn snap_or_edge(win: &tauri::WebviewWindow, _app: &tauri::AppHandle) {
    place_at_edge(win);
}

/// 启动跟随线程：out-of-context hook + 消息循环。
/// 只按**进程**过滤（idprocess=网易云 pid），回调里再按 hwnd 精确匹配，
/// 否则 hook 会收到全系统所有窗口的移动事件。
#[cfg(windows)]
fn start_snap_watch(app: &tauri::AppHandle, hwnd: isize) {
    use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent};
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

    let _ = SNAP_APP.set(app.clone());

    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd as _, &mut pid) };
    if pid == 0 {
        return;
    }

    std::thread::spawn(move || unsafe {
        use windows_sys::Win32::System::Threading::GetCurrentThreadId;
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG};

        let thread_id = GetCurrentThreadId();
        let hook = SetWinEventHook(
            EVENT_SYSTEM_MINIMIZESTART,
            EVENT_OBJECT_LOCATIONCHANGE,
            std::ptr::null_mut(),
            Some(snap_event_proc),
            pid,
            0, // 该进程的所有线程
            WINEVENT_OUTOFCONTEXT,
        );
        if hook.is_null() {
            eprintln!("[nowplaying] SetWinEventHook 失败，面板停在原位不跟随");
            return;
        }
        if let Ok(mut guard) = SNAP_WATCH.lock() {
            *guard = Some(SnapWatch { thread_id, hwnd });
        }

        // out-of-context hook 的事件靠本线程的消息循环派发；收到 WM_QUIT 才退
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {}
        let _ = UnhookWinEvent(hook);
        if let Ok(mut guard) = SNAP_WATCH.lock() {
            *guard = None;
        }
    });
}

#[cfg(not(windows))]
fn start_snap_watch(_app: &tauri::AppHandle, _hwnd: isize) {}

/// 停掉跟随（幂等）。向 watch 线程投递 WM_QUIT，由它自己 Unhook —— 不在 hook 回调里
/// 反注册自身，那是不安全的。
#[cfg(windows)]
fn stop_snap_watch() {
    use windows_sys::Win32::Foundation::{LPARAM, WPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};

    let state = SNAP_WATCH.lock().ok().and_then(|mut g| g.take());
    if let Some(s) = state {
        unsafe {
            PostThreadMessageW(s.thread_id, WM_QUIT, 0 as WPARAM, 0 as LPARAM);
        }
    }
}

#[cfg(not(windows))]
fn stop_snap_watch() {}

/// hook 回调：跑在 watch 线程（靠消息循环驱动）。
/// 网易云窗口动了 → 面板跟着挪；最小化/隐藏/销毁 → 面板跟着关。
#[cfg(windows)]
unsafe extern "system" fn snap_event_proc(
    _hook: windows_sys::Win32::UI::Accessibility::HWINEVENTHOOK,
    event: u32,
    hwnd: windows_sys::Win32::Foundation::HWND,
    idobject: i32,
    _idchild: i32,
    _thread: u32,
    _time: u32,
) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{IsIconic, IsWindowVisible};
    use tauri::Manager;

    // 锁内只做快照，不在锁里干活
    let (tracked_hwnd, app) = {
        let Ok(guard) = SNAP_WATCH.lock() else {
            return;
        };
        let Some(state) = guard.as_ref() else {
            return;
        };
        let Some(a) = SNAP_APP.get() else {
            return;
        };
        (state.hwnd, a.clone())
    };
    if hwnd as isize != tracked_hwnd || idobject != OBJID_WINDOW {
        return;
    }

    let gone = match event {
        EVENT_OBJECT_DESTROY | EVENT_OBJECT_HIDE | EVENT_SYSTEM_MINIMIZESTART => true,
        EVENT_OBJECT_LOCATIONCHANGE => {
            // 最小化也会触发 LOCATIONCHANGE，用 IsIconic 区分
            unsafe { IsIconic(hwnd) != 0 || IsWindowVisible(hwnd) == 0 }
        }
        _ => false,
    };

    if gone {
        // close_window 内部会停 watch：向本线程投递 WM_QUIT，回调返回后消息循环退出
        close_window(&app);
        return;
    }
    if let Some(win) = app.get_webview_window(WINDOW_LABEL) {
        position_near(&win, tracked_hwnd);
    }
}

#[cfg(test)]
mod tests {
    use super::{hint_for_title, parse_track_title, IDLE_TITLES};

    #[test]
    fn parses_standard_title() {
        let t = parse_track_title("下等马 - 洛天依Official/ChiliChill乐团").unwrap();
        assert_eq!(t.title, "下等马");
        assert_eq!(t.artist, "洛天依Official/ChiliChill乐团");
    }

    #[test]
    fn parses_title_containing_dash() {
        // 曲名自带 " - " 时必须靠 rsplit 反向切，否则会把曲名切碎
        let t = parse_track_title("My Song - Live - Some Artist").unwrap();
        assert_eq!(t.title, "My Song - Live");
        assert_eq!(t.artist, "Some Artist");
    }

    #[test]
    fn rejects_idle_titles() {
        for idle in IDLE_TITLES {
            assert!(
                parse_track_title(idle).is_none(),
                "idle 标题不该识别为曲目：{idle}"
            );
        }
    }

    #[test]
    fn rejects_empty_and_malformed() {
        assert!(parse_track_title("").is_none());
        assert!(parse_track_title("   ").is_none());
        assert!(parse_track_title("网易云音乐正在播放").is_none());
        assert!(parse_track_title(" - ").is_none());
        assert!(parse_track_title("曲名 - ").is_none());
        assert!(parse_track_title(" - 歌手").is_none());
    }

    #[test]
    fn trims_surrounding_whitespace() {
        let t = parse_track_title("  鼓楼 - 赵雷  ").unwrap();
        assert_eq!(t.title, "鼓楼");
        assert_eq!(t.artist, "赵雷");
        assert_eq!(t.raw_title, "鼓楼 - 赵雷");
    }

    #[test]
    fn mini_mode_gets_a_precise_hint_instead_of_generic_idle() {
        // 迷你模式下歌在播、标题却是「迷你播放器」。
        // 若笼统说「未在播放」，用户只会以为面板坏了 —— 这是实测撞出来的坑。
        let hint = hint_for_title("迷你播放器").expect("迷你模式必须给出提示");
        assert!(hint.contains("迷你"), "提示要说清是迷你模式：{hint}");
        assert!(hint.contains("播放器"), "提示要带上原始标题：{hint}");
        assert!(hint_for_title("鼓楼 - 赵雷").is_none());
    }
}
