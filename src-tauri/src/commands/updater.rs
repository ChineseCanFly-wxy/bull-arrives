use crate::datasource::market_clock::MarketSession;
use crate::domain::UpdateInfo;
use crate::PortableMode;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_updater::UpdaterExt;

/// 常见本地代理端口的兜底表（顺序即优先级）。
///
/// **这只是兜底**，不是主要依据 —— 系统代理设置（`system_proxy_port`）才是用户真在用的值。
/// 历史上这张表漏掉 7897（Clash Verge Rev 的 mixed 默认端口），导致更新客户端
/// 一个都探不到、直连 github.com 超时，自动更新静默失效。
const AUTO_PROXY_PORTS: &[u16] = &[
    7897, 7890, 7891, // Clash Verge (Rev) / Clash for Windows 的 mixed 端口
    10809, 10808, // v2rayN：HTTP 10809 / SOCKS 10808
    1080, 1087, 8889, // 通用 SOCKS / HTTP
    8118, // Privoxy
    8080, 8888, 2080, 20171, 10807, 9910, // 其它常见本地端口
];

/// 探测决策：系统代理优先，失败才退回端口表。
fn detect_http_proxy_port() -> Option<u16> {
    if let Some(port) = system_proxy_port() {
        if probe_connect(port) {
            log::info!("[updater] 使用系统代理 127.0.0.1:{}", port);
            return Some(port);
        }
        log::warn!(
            "[updater] 系统代理配的是 127.0.0.1:{}，但 CONNECT 探测未通过，回退到端口表",
            port
        );
    }

    match AUTO_PROXY_PORTS
        .iter()
        .copied()
        .find(|port| probe_connect(*port))
    {
        Some(port) => {
            log::info!("[updater] 端口表命中代理 127.0.0.1:{}", port);
            Some(port)
        }
        None => {
            log::warn!(
                "[updater] 未探测到可用代理（已试系统代理与 {:?}）。若更新失败，请设置 HTTPS_PROXY",
                AUTO_PROXY_PORTS
            );
            None
        }
    }
}

/// A listening port is not necessarily an HTTP proxy (8080 is often nginx or
/// a development server). Probe the CONNECT handshake before using it.
fn probe_connect(port: u16) -> bool {
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream =
        match TcpStream::connect_timeout(&address, std::time::Duration::from_millis(75)) {
            Ok(stream) => stream,
            Err(_) => return false,
        };
    if stream
        .set_write_timeout(Some(std::time::Duration::from_millis(250)))
        .is_err()
        || stream
            .set_read_timeout(Some(std::time::Duration::from_millis(250)))
            .is_err()
    {
        return false;
    }
    if stream
        .write_all(b"CONNECT github.com:443 HTTP/1.1\r\nHost: github.com:443\r\n\r\n")
        .is_err()
    {
        return false;
    }

    let mut response = [0u8; 256];
    let bytes_read = match stream.read(&mut response) {
        Ok(bytes_read) => bytes_read,
        Err(_) => return false,
    };
    is_successful_proxy_connect(&response[..bytes_read])
}

/// 读 Windows「Internet 选项」里的系统代理端口。
///
/// 端口表只能靠猜：用户自定义端口、或客户端换了默认端口时就会全线落空。
/// 系统代理是用户真正在用的值，优先信它。
/// 非 Windows 平台没有统一的系统代理存储，返回 `None`，由端口表兜底。
#[cfg(target_os = "windows")]
fn system_proxy_port() -> Option<u16> {
    use windows::core::w;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, KEY_READ,
    };

    const INTERNET_SETTINGS: windows::core::PCWSTR =
        w!("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings");

    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, INTERNET_SETTINGS, 0, KEY_READ, &mut hkey)
            != ERROR_SUCCESS
        {
            return None;
        }

        let enabled = reg_read_dword(hkey, w!("ProxyEnable")).unwrap_or(0);
        let server = reg_read_string(hkey, w!("ProxyServer"));
        let _ = RegCloseKey(hkey);

        if enabled == 0 {
            return None;
        }
        server.as_deref().and_then(parse_proxy_port)
    }
}

#[cfg(not(target_os = "windows"))]
fn system_proxy_port() -> Option<u16> {
    None
}

#[cfg(target_os = "windows")]
unsafe fn reg_read_dword(
    hkey: windows::Win32::System::Registry::HKEY,
    name: windows::core::PCWSTR,
) -> Option<u32> {
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{RegQueryValueExW, REG_DWORD, REG_VALUE_TYPE};

    let mut value: u32 = 0;
    let mut kind: REG_VALUE_TYPE = REG_DWORD;
    let mut size = std::mem::size_of::<u32>() as u32;
    let status = RegQueryValueExW(
        hkey,
        name,
        None,
        Some(&mut kind),
        Some(&mut value as *mut u32 as *mut u8),
        Some(&mut size),
    );
    (status == ERROR_SUCCESS && kind == REG_DWORD).then_some(value)
}

#[cfg(target_os = "windows")]
unsafe fn reg_read_string(
    hkey: windows::Win32::System::Registry::HKEY,
    name: windows::core::PCWSTR,
) -> Option<String> {
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{RegQueryValueExW, REG_SZ, REG_VALUE_TYPE};

    let mut kind: REG_VALUE_TYPE = REG_SZ;
    let mut size: u32 = 0;
    if RegQueryValueExW(hkey, name, None, Some(&mut kind), None, Some(&mut size)) != ERROR_SUCCESS
        || size == 0
    {
        return None;
    }

    // REG_SZ 的 size 含结尾 NUL，按 UTF-16 码元计算
    let mut buffer = vec![0u16; size as usize / 2 + 1];
    if RegQueryValueExW(
        hkey,
        name,
        None,
        Some(&mut kind),
        Some(buffer.as_mut_ptr() as *mut u8),
        Some(&mut size),
    ) != ERROR_SUCCESS
    {
        return None;
    }

    let end = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..end]))
}

/// 解析 `ProxyServer` 的端口。
///
/// 取值可能是 `127.0.0.1:7897`，也可能是分协议写法
/// `http=127.0.0.1:7890;https=127.0.0.1:7897` —— 后者优先取 `https=` 那一项。
fn parse_proxy_port(spec: &str) -> Option<u16> {
    let spec = spec.trim();
    if spec.is_empty() {
        return None;
    }

    let candidate = spec
        .split(';')
        .find_map(|entry| {
            let entry = entry.trim();
            entry
                .strip_prefix("https=")
                .or_else(|| entry.strip_prefix("HTTPS="))
        })
        .or_else(|| spec.split(';').next())
        .unwrap_or(spec);

    // 容错 `http://127.0.0.1:7897` 与结尾斜杠
    let candidate = candidate.trim().trim_end_matches('/');
    let host_port = candidate.rsplit('/').next().unwrap_or(candidate);
    host_port
        .rsplit(':')
        .next()?
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|port| *port != 0)
}

fn is_successful_proxy_connect(response: &[u8]) -> bool {
    let Some(status_line) = std::str::from_utf8(response)
        .ok()
        .and_then(|text| text.lines().next())
    else {
        return false;
    };
    let mut fields = status_line.split_whitespace();
    matches!(fields.next(), Some("HTTP/1.0") | Some("HTTP/1.1"))
        && fields
            .next()
            .and_then(|status| status.parse::<u16>().ok())
            .is_some_and(|status| (200..300).contains(&status))
}

/// 自动代理仅用于更新客户端，不修改进程环境，也不阻塞行情窗口启动。
async fn configured_updater(app: &AppHandle) -> Result<tauri_plugin_updater::Updater, String> {
    let mut builder = app.updater_builder();

    // 环境变量优先级最高：reqwest 默认就会读 HTTP(S)_PROXY / ALL_PROXY，
    // 这里只要探测到有，就不要再覆盖用户的显式配置。
    let explicit_proxy = [
        "HTTPS_PROXY",
        "HTTP_PROXY",
        "https_proxy",
        "http_proxy",
        "ALL_PROXY",
        "all_proxy",
    ]
    .into_iter()
    .find(|key| std::env::var_os(key).is_some());

    match explicit_proxy {
        Some(key) => log::info!("[updater] 使用环境变量 {} 指定的代理", key),
        None => {
            let port = tokio::task::spawn_blocking(detect_http_proxy_port)
                .await
                .map_err(|e| format!("更新代理探测失败：{}", e))?;
            if let Some(port) = port {
                let proxy = format!("http://127.0.0.1:{}", port)
                    .parse()
                    .map_err(|e| format!("更新代理地址无效：{}", e))?;
                builder = builder.proxy(proxy);
            } else {
                log::warn!("[updater] 未配置代理，将尝试直连 —— 国内网络下大概率会超时");
            }
        }
    }

    // updater_builder 保留插件默认的 cleanup_before_exit 回调，不自行退出进程。
    builder
        .build()
        .map_err(|e| format!("Updater init failed: {}", e))
}

#[cfg(test)]
mod tests {
    use super::{is_successful_proxy_connect, parse_proxy_port};

    #[test]
    fn proxy_probe_requires_successful_connect_response() {
        assert!(is_successful_proxy_connect(
            b"HTTP/1.1 200 Connection Established\r\n\r\n"
        ));
        assert!(!is_successful_proxy_connect(
            b"HTTP/1.1 400 Bad Request\r\n\r\n"
        ));
        assert!(!is_successful_proxy_connect(
            b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n"
        ));
        assert!(!is_successful_proxy_connect(b"not an HTTP response"));
    }

    #[test]
    fn parses_plain_host_port() {
        assert_eq!(parse_proxy_port("127.0.0.1:7897"), Some(7897));
        assert_eq!(parse_proxy_port(" 127.0.0.1:7890 "), Some(7890));
    }

    #[test]
    fn prefers_https_entry_in_per_protocol_spec() {
        assert_eq!(
            parse_proxy_port("http=127.0.0.1:7890;https=127.0.0.1:7897"),
            Some(7897)
        );
        // 只有 http 时退回第一项
        assert_eq!(parse_proxy_port("http=127.0.0.1:7890"), Some(7890));
    }

    #[test]
    fn tolerates_scheme_prefix_and_rejects_junk() {
        assert_eq!(parse_proxy_port("http://127.0.0.1:7897/"), Some(7897));
        assert_eq!(parse_proxy_port(""), None);
        assert_eq!(parse_proxy_port("   "), None);
        assert_eq!(parse_proxy_port("127.0.0.1"), None);
        assert_eq!(parse_proxy_port("127.0.0.1:0"), None);
        assert_eq!(parse_proxy_port("127.0.0.1:99999"), None);
    }
}

/// Core update-check logic (no State dependency). Callable from the tray menu
/// handler where Tauri's automatic State injection is not available.
pub async fn do_check_update(app: &AppHandle) -> Result<Option<UpdateInfo>, String> {
    let current_version = app
        .config()
        .version
        .clone()
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());

    log::info!(
        "[updater] Checking for updates (current: {})...",
        current_version
    );

    let updater = configured_updater(app).await?;

    let Some(update) = updater.check().await.map_err(|e| {
        log::error!("[updater] Check failed: {}", e);
        format!("Update check failed: {}", e)
    })?
    else {
        log::info!(
            "[updater] No update available (current: {})",
            current_version
        );
        return Ok(None);
    };

    let latest_version = update.version.clone();
    let body = update.body.clone().unwrap_or_default();
    let date = update.date.map(|d| d.to_string()).unwrap_or_default();

    log::info!(
        "[updater] Update found: {} -> {} (date: {}, notes length: {})",
        current_version,
        latest_version,
        date,
        body.len()
    );

    let info = UpdateInfo {
        current_version,
        latest_version: latest_version.clone(),
        release_date: date,
        notes: body,
        release_url: format!(
            "https://github.com/ChineseCanFly-wxy/bull-arrives/releases/tag/v{}",
            latest_version.strip_prefix('v').unwrap_or(&latest_version)
        ),
        download_size: None,
    };

    Ok(Some(info))
}

/// Check for update. Returns UpdateInfo if a newer version is available,
/// or null if the current version is already the latest.
/// In portable mode, always returns None (updates are managed by the user).
#[tauri::command]
pub async fn check_update(
    app: AppHandle,
    portable: State<'_, PortableMode>,
) -> Result<Option<UpdateInfo>, String> {
    if portable.0 {
        log::info!("[updater] Skipping update check — running in portable mode");
        return Ok(None);
    }
    do_check_update(&app).await
}

/// Download and install the update.
///
/// **Important**: We use `app.updater()` (NOT `updater_builder()` with a custom
/// `on_before_exit`). The plugin's default `on_before_exit` only runs
/// `cleanup_before_exit()`. The installer is launched via `ShellExecuteW` AFTER
/// `on_before_exit`, and the plugin calls `std::process::exit(0)` AFTER that.
/// If we set a custom `on_before_exit` that calls `std::process::exit(0)`, we
/// kill the process BEFORE the installer is launched — the update silently fails.
///
/// **Note on progress**: The plugin's `on_chunk` callback receives the individual
/// chunk size (not cumulative bytes). We accumulate them ourselves to compute
/// actual download progress.
#[tauri::command]
pub async fn install_update(
    app: AppHandle,
    portable: State<'_, PortableMode>,
) -> Result<(), String> {
    if portable.0 {
        log::info!("[updater] Skipping update install — running in portable mode");
        return Err("更新功能在绿色版中不可用，请手动下载新版本".to_string());
    }

    let handle = app.clone();
    let updater = configured_updater(&app).await?;

    log::info!("[updater] Checking for update before download...");
    let Some(update) = updater.check().await.map_err(|e| {
        log::error!("[updater] Pre-download check failed: {}", e);
        format!("Update check failed: {}", e)
    })?
    else {
        log::warn!("[updater] No update available for download");
        return Err("No update available".into());
    };

    let target_version = update.version.clone();
    let current_version = app.config().version.clone().unwrap_or_default();
    log::info!(
        "[updater] Starting download for v{} (current: {})",
        target_version,
        current_version
    );

    // The plugin's progress callback receives chunk sizes (not cumulative).
    // We accumulate them to track real progress.
    let cumulative_bytes = std::sync::Arc::new(AtomicU64::new(0));
    let cum_bytes = cumulative_bytes.clone();
    let total_size = std::sync::Arc::new(AtomicU64::new(0));
    let total_sz = total_size.clone();
    let call_count = std::sync::Arc::new(AtomicU64::new(0));
    let cc = call_count.clone();
    // Track last logged percentage milestone
    let last_logged_pct = std::sync::Arc::new(AtomicU64::new(0));
    let llp = last_logged_pct.clone();

    let result = update
        .download_and_install(
            move |chunk_size, total| {
                // chunk_size is the size of this individual chunk, NOT cumulative.
                // Accumulate to get real downloaded bytes.
                let cumulative =
                    cum_bytes.fetch_add(chunk_size as u64, Ordering::Relaxed) + chunk_size as u64;

                let count = cc.fetch_add(1, Ordering::Relaxed);

                if let Some(t) = total {
                    total_sz.store(t as u64, Ordering::Relaxed);
                }

                let total_for_pct = total.unwrap_or(1);
                let pct = if total_for_pct > 0 {
                    ((cumulative as f64 / total_for_pct as f64) * 100.0) as u64
                } else {
                    0
                };

                // Log at milestones: first call, every 10% increment, every 200th call
                let last = llp.load(Ordering::Relaxed);
                let milestone = pct / 10;
                let last_milestone = last / 10;
                let should_log =
                    count == 0 || milestone > last_milestone || (count > 0 && count % 200 == 0);

                if should_log {
                    llp.store(pct, Ordering::Relaxed);
                    let total_str = total
                        .map(|t| format!("{:.1} MB", t as f64 / 1_048_576.0))
                        .unwrap_or_else(|| "unknown".to_string());
                    log::info!(
                        "[updater] Download progress: {:.1} KB / {} ({}%, {} chunks)",
                        cumulative as f64 / 1024.0,
                        total_str,
                        pct,
                        count + 1
                    );
                }

                // Emit to frontend using cumulative bytes
                let percent = match total {
                    Some(t) if t > 0 => (cumulative as f64 / t as f64) * 100.0,
                    _ => 0.0,
                };
                let _ = handle.emit(
                    "update-download-progress",
                    serde_json::json!({
                        "downloaded": cumulative,
                        "total": total,
                        "percent": (percent * 10.0).round() / 10.0,
                    }),
                );
            },
            || {
                let final_bytes = cumulative_bytes.load(Ordering::Relaxed);
                let total = total_size.load(Ordering::Relaxed);
                let chunks = call_count.load(Ordering::Relaxed);
                log::info!(
                    "[updater] Download complete. Downloaded: {:.1} KB / {:.1} MB ({} chunks)",
                    final_bytes as f64 / 1024.0,
                    total as f64 / 1_048_576.0,
                    chunks
                );
                if final_bytes > 0 && total > 0 && final_bytes < total * 9 / 10 {
                    log::warn!(
                        "[updater] WARNING: only {:.1}% of the file was downloaded!",
                        (final_bytes as f64 / total as f64) * 100.0
                    );
                }
            },
        )
        .await
        .map_err(|e| {
            log::error!("[updater] Download/install failed: {}", e);
            format!("Download/install failed: {}", e)
        });

    match &result {
        Ok(()) => {
            // Note: we won't actually reach here for a successful install because
            // the plugin calls std::process::exit(0) after launching the installer.
            log::info!("[updater] download_and_install returned Ok (installer launched)");
        }
        Err(e) => {
            log::error!("[updater] install_update error: {}", e);
        }
    }

    result
}

/// Check if currently in an active A-share trading session (9:30-11:30 or 13:00-15:00)
#[tauri::command]
pub fn is_trading_session() -> bool {
    let session = MarketSession::current();
    matches!(
        session,
        MarketSession::MorningTrade | MarketSession::AfternoonTrade
    )
}
