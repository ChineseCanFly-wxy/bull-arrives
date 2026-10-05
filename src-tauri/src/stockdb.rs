use crate::datasource::history::{self, LocalHistoryConfig};
use crate::db::Database;
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter,Manager};

fn updater_command(updater:&Path,verify:bool,cli:bool)->Result<Command,String>{
    let mut command=Command::new(updater);
    if cli{command.arg(if verify{"--verify"}else{"--sync"});}
    command.current_dir(updater.parent().ok_or("更新程序缺少父目录")?);
    #[cfg(windows)]{use std::os::windows::process::CommandExt;command.creation_flags(0x08000000);}
    Ok(command)
}

fn updater_supports_cli(updater:&Path)->Result<bool,String>{
    // Older distributed updaters start on a no-argument launch. Inspect the program,
    // never invoke --help to detect capabilities (old tools may start a real update).
    let bytes=std::fs::read(updater).map_err(|e|format!("读取更新程序失败：{e}"))?;
    Ok(bytes.windows(6).any(|v|v==b"--sync")&&bytes.windows(8).any(|v|v==b"--verify"))
}

pub const STATUS_EVENT: &str = "stockdb-status-changed";
const ENGINE_FILE: &str = "stockdb.exe";
const UPDATER_FILE: &str = "数据更新.exe";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StockDbCandidate {
    pub engine_path: String,
    pub updater_path: Option<String>,
    pub source: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StockDbStatus {
    pub enabled: bool,
    pub platform_supported: bool,
    pub state: String,
    pub phase: Option<String>,
    pub engine_path: Option<String>,
    pub updater_path: Option<String>,
    pub updater_available: bool,
    pub owned: bool,
    pub busy: bool,
    pub message: String,
    pub last_error: Option<String>,
    pub candidates: Vec<StockDbCandidate>,
    pub auto_update:crate::stockdb_schedule::UpdateStatus,
}

impl StockDbStatus {
    fn disabled() -> Self {
        Self {
            enabled: false,
            platform_supported: cfg!(target_os = "windows"),
            state: if cfg!(target_os = "windows") { "disabled" } else { "unsupported" }.into(),
            phase: None,
            engine_path: None,
            updater_path: None,
            updater_available: false,
            owned: false,
            busy: false,
            message: if cfg!(target_os = "windows") {
                "本地历史数据已关闭".into()
            } else {
                "stockdb 自动管理目前仅支持 Windows".into()
            },
            last_error: None,
            candidates: Vec::new(),
            auto_update:crate::stockdb_schedule::UpdateStatus::default(),
        }
    }
}

// Stop and restore form one transaction for both manual and scheduled updates.
// The update future is not polled until stop succeeds; restoration also runs
// after a partial stop or any updater failure.
trait UpdateService: Send {
    fn stop(&mut self) -> impl std::future::Future<Output = Result<(), String>> + Send;
    fn restore(&mut self) -> impl std::future::Future<Output = Result<bool, String>> + Send;
}

async fn update_with_service_restore(
    service: &mut impl UpdateService,
    update: impl std::future::Future<Output = Result<(), String>> + Send,
) -> Result<(), String> {
    let update_result = match service.stop().await {
        Ok(()) => update.await,
        Err(error) => Err(error),
    };
    let restore_result = service.restore().await;
    match (update_result, restore_result) {
        (Ok(()), Ok(true)) => Ok(()),
        (Ok(()), Ok(false)) => Err("更新程序已结束，但本地服务已关闭或应用正在退出，尚未完成日期核验".into()),
        (Err(error), Ok(true)) => Err(format!("{error}；stockdb 已恢复运行")),
        (Err(error), Ok(false)) => Err(format!("{error}；本地服务保持关闭")),
        (Ok(()), Err(error)) => Err(format!("数据更新完成，但 stockdb 恢复失败：{error}")),
        (Err(update), Err(restore)) => Err(format!("{update}；stockdb 恢复也失败：{restore}")),
    }
}

#[derive(Clone, Debug)]
struct ServiceEndpoint {
    addresses: Vec<std::net::IpAddr>,
    port: u16,
}

impl ServiceEndpoint {
    fn parse(raw: &str) -> Result<Self, String> {
        use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
        let url = reqwest::Url::parse(raw).map_err(|e| format!("StockDB 地址无效：{e}"))?;
        if url.scheme() != "http" || !url.username().is_empty() || url.password().is_some() {
            return Err("StockDB 自动更新仅允许无身份信息的本地 HTTP 地址".into());
        }
        let addresses = match url.host_str() {
            Some("localhost") => vec![IpAddr::V4(Ipv4Addr::LOCALHOST), IpAddr::V6(Ipv6Addr::LOCALHOST)],
            Some(host) => vec![host.trim_matches(['[', ']']).parse::<IpAddr>()
                .ok().filter(IpAddr::is_loopback).ok_or("StockDB 自动更新仅允许 loopback 地址，不会停止远程服务")?],
            None => return Err("StockDB 地址缺少主机".into()),
        };
        Ok(Self { addresses, port: url.port_or_known_default().ok_or("StockDB 地址缺少端口")? })
    }

    #[cfg(windows)]
    fn matches_listener(&self, address: std::net::IpAddr, port: u16) -> bool {
        port == self.port && (self.addresses.contains(&address) || address.is_unspecified())
    }
}

struct UpdateActivity<'a>(&'a StockDbManager);

impl Drop for UpdateActivity<'_> {
    fn drop(&mut self) {
        self.0.runtime.lock().unwrap_or_else(|e| e.into_inner()).update_active = false;
    }
}

#[cfg(windows)]
struct LocalUpdateService<'a> {
    manager: &'a StockDbManager,
    engine: PathBuf,
    endpoint: ServiceEndpoint,
    previous: Option<local_service::CapturedService>,
    was_owned: bool,
    stopped_owned: bool,
}

#[cfg(windows)]
impl UpdateService for LocalUpdateService<'_> {
    async fn stop(&mut self) -> Result<(), String> {
        self.manager.set_status(|status| {
            status.state = "updating".into();
            status.phase = Some("stopping".into());
            status.busy = true;
            status.message = "正在停止已确认的本地 stockdb 服务…".into();
            status.last_error = None;
        });
        if let Some(previous) = self.previous.as_mut() {
            previous.revalidate(&self.endpoint)?;
            if self.was_owned {
                self.stopped_owned = self.manager.stop_owned_locked();
            } else {
                previous.stop()?;
            }
            previous.wait_stopped().await?;
        }
        // Check the actual listener, even when HTTP probing fails or a new
        // process claims the port while the old service is stopping.
        for _ in 0..50 {
            if local_service::listener_pids(&self.endpoint)?.is_empty() {
                self.manager.set_status(|status| status.owned = false);
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err("StockDB 端口尚未释放，未启动数据更新程序".into())
    }

    async fn restore(&mut self) -> Result<bool, String> {
        self.manager.set_status(|status| {
            status.state = "restarting".into();
            status.phase = Some("probing".into());
            status.busy = true;
            status.message = "正在恢复 stockdb 服务…".into();
        });
        if !self.was_owned {
            if let Some(previous) = self.previous.as_mut() {
                // An external service keeps its original launch settings and
                // remains external, including when the application is exiting.
                let pid = previous.restore(&self.endpoint).await?;
                let config = LocalHistoryConfig { base_url: self.manager.service_url(), timeout: Duration::from_secs(1) };
                for _ in 0..30 {
                    if history::probe(&config).await.is_ok() {
                        local_service::confirm_listener(&self.engine, &self.endpoint, pid)?;
                        self.manager.set_status(|status| {
                            status.state = "running_external".into();
                            status.phase = None;
                            status.owned = false;
                            status.busy = false;
                            status.engine_path = Some(path_string(&self.engine));
                            status.message = format!("外部 stockdb 已恢复运行（PID {pid}）");
                            status.last_error = None;
                        });
                        return Ok(true);
                    }
                    tokio::time::sleep(Duration::from_millis(300)).await;
                }
                return Err("已按原参数恢复外部 stockdb，但本地服务连接核验超时".into());
            }
        }
        if self.was_owned && !self.stopped_owned {
            self.manager.ensure_running_locked().await?;
            return Ok(true);
        }
        if !self.manager.enabled_in_db()
            || self.manager.runtime.lock().unwrap_or_else(|e| e.into_inner()).shutting_down {
            return Ok(false);
        }
        if !local_service::listener_pids(&self.endpoint)?.is_empty() {
            return Err("恢复时本地端口被其他进程占用，未启动重复服务".into());
        }
        self.manager.start_owned_locked(&self.engine).await?;
        let pid = self.manager.runtime.lock().unwrap_or_else(|e| e.into_inner())
            .child.as_ref().map(Child::id).ok_or("恢复后 stockdb 进程已退出")?;
        local_service::confirm_listener(&self.engine, &self.endpoint, pid)?;
        Ok(true)
    }
}

struct Runtime {
    status: StockDbStatus,
    child: Option<Child>,
    job: Option<crate::agent::ProcessJob>,
    updater_child: Option<Child>,
    updater_job: Option<crate::agent::ProcessJob>,
    shutting_down: bool,
    update_active: bool,
}

// Share the operation lock through the URL write, including updater recovery.
fn save_service_url(db: &Database, operation: &tokio::sync::Mutex<()>, url: &str) -> Result<(), String> {
    let _guard = operation.try_lock().map_err(|_| "stockdb 正在执行其他操作，服务地址暂不能修改，请稍后再试")?;
    db.set_setting("local_history_url", url).map_err(|e| e.to_string())
}

pub struct StockDbManager {
    db: Arc<Database>,
    app: AppHandle,
    runtime: Mutex<Runtime>,
    operation: tokio::sync::Mutex<()>,
}

impl StockDbManager {
    pub fn new(db: Arc<Database>, app: AppHandle) -> Self {
        Self {
            db,
            app,
            runtime: Mutex::new(Runtime {
                status: StockDbStatus::disabled(),
                child: None,
                job: None,
                updater_child: None,
                updater_job: None,
                shutting_down: false,
                update_active: false,
            }),
            operation: tokio::sync::Mutex::new(()),
        }
    }

    pub fn status(&self) -> StockDbStatus {
        let mut status = {
            let mut runtime = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(child) = runtime.child.as_mut() {
                if let Ok(Some(exit)) = child.try_wait() {
                    runtime.child = None;
                    runtime.job = None;
                    runtime.status.owned = false;
                    runtime.status.busy = false;
                    runtime.status.state = "error".into();
                    runtime.status.message = format!("stockdb 已意外退出（{exit}）");
                    runtime.status.last_error = Some(runtime.status.message.clone());
                }
            }
            runtime.status.clone()
        };
        status.enabled = self.enabled_in_db();
        status.auto_update=crate::stockdb_schedule::status(&self.db,chrono::Utc::now()).unwrap_or_default();
        let configured_engine = (status.engine_path.is_none() || status.updater_path.is_none())
            .then(|| self.configured_engine())
            .flatten();
        if status.engine_path.is_none() {
            status.engine_path = configured_engine.as_deref().map(path_string);
        }
        if status.updater_path.is_none() {
            status.updater_path = self
                .configured_updater(configured_engine.as_deref())
                .as_deref()
                .map(path_string);
        }
        status.updater_available = status.updater_path.is_some();
        status
    }

    fn emit_status(&self) {
        let _ = self.app.emit(STATUS_EVENT, self.status());
    }

    fn set_status(&self, update: impl FnOnce(&mut StockDbStatus)) {
        {
            let mut runtime = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
            update(&mut runtime.status);
        }
        self.emit_status();
    }

    fn enabled_in_db(&self) -> bool {
        self.db
            .get_setting("local_history_enabled")
            .ok()
            .flatten()
            .as_deref()
            == Some("1")
    }

    fn service_url(&self) -> String {
        self.db
            .get_setting("local_history_url")
            .ok()
            .flatten()
            .unwrap_or_else(|| "http://127.0.0.1:7899".into())
    }

    pub(crate) fn set_service_url(&self, url: &str) -> Result<(), String> {
        save_service_url(&self.db, &self.operation, url)
    }

    fn configured_engine(&self) -> Option<PathBuf> {
        if let Some(path) = self
            .db
            .get_setting("local_history_engine_path")
            .ok()
            .flatten()
            .filter(|v| !v.trim().is_empty())
        {
            let path = PathBuf::from(path);
            if validate_engine(&path).is_ok() {
                return Some(path);
            }
        }
        self.db
            .get_setting("local_history_engine_dir")
            .ok()
            .flatten()
            .filter(|v| !v.trim().is_empty())
            .map(PathBuf::from)
            .map(|dir| dir.join(ENGINE_FILE))
            .filter(|path| validate_engine(path).is_ok())
    }

    fn configured_updater(&self, engine: Option<&Path>) -> Option<PathBuf> {
        if let Some(path) = self
            .db
            .get_setting("local_history_updater_path")
            .ok()
            .flatten()
            .filter(|v| !v.trim().is_empty())
        {
            let path = PathBuf::from(path);
            if validate_updater(&path, engine).is_ok() {
                return Some(path);
            }
        }
        engine
            .and_then(Path::parent)
            .map(|dir| dir.join(UPDATER_FILE))
            .filter(|path| validate_updater(path, engine).is_ok())
    }

    pub async fn initialize(&self) {
        if !cfg!(target_os = "windows") {
            self.emit_status();
            return;
        }
        if !self.enabled_in_db() {
            self.emit_status();
            return;
        }
        if let Err(error) = self.ensure_running().await {
            log::warn!("[stockdb] 启动失败: {error}");
        }
    }

    pub async fn set_enabled(&self, enabled: bool) -> Result<StockDbStatus, String> {
        if !cfg!(target_os = "windows") {
            return Err("stockdb 自动管理目前仅支持 Windows".into());
        }
        let _guard = self.operation.try_lock().map_err(|_| "stockdb 正在执行其他操作，请稍后再试")?;
        self.db
            .set_setting("local_history_enabled", if enabled { "1" } else { "0" })
            .map_err(|e| e.to_string())?;
        if enabled {
            if let Err(error) = self.ensure_running_locked().await {
                let _ = self.db.set_setting("local_history_enabled", "0");
                self.stop_owned_locked();
                self.set_status(|status| {
                    let candidates = std::mem::take(&mut status.candidates);
                    *status = StockDbStatus::disabled();
                    status.candidates = candidates;
                    status.message = format!("开启失败：{error}");
                    status.last_error = Some(error.clone());
                });
                return Err(error);
            }
        } else {
            self.stop_owned_locked();
            self.set_status(|status| {
                *status = StockDbStatus::disabled();
            });
        }
        Ok(self.status())
    }

    pub async fn ensure_running(&self) -> Result<StockDbStatus, String> {
        let _guard = self.operation.try_lock().map_err(|_| "stockdb 正在执行其他操作，请稍后再试")?;
        self.ensure_running_locked().await?;
        Ok(self.status())
    }

    async fn ensure_running_locked(&self) -> Result<(), String> {
        if !self.enabled_in_db() {
            self.set_status(|status| *status = StockDbStatus::disabled());
            return Ok(());
        }
        if self
            .runtime
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .child
            .is_some()
        {
            self.set_status(|status| {
                status.enabled = true;
                status.state = "running_owned".into();
                status.phase = None;
                status.owned = true;
                status.busy = false;
                status.message = "stockdb 正在运行".into();
                status.last_error = None;
            });
            return Ok(());
        }
        if history::probe(&LocalHistoryConfig::new(self.service_url())).await.is_ok() {
            self.set_status(|status| {
                status.enabled = true;
                status.state = "running_external".into();
                status.phase = None;
                status.owned = false;
                status.busy = false;
                status.message = "检测到已运行的外部 stockdb；更新时会核对本地进程身份并自动停机、恢复".into();
                status.last_error = None;
            });
            return Ok(());
        }

        let engine = match self.configured_engine() {
            Some(path) => path,
            None => {
                let candidates = discover_stockdb();
                let message = if candidates.is_empty() {
                    "未找到 stockdb.exe，请浏览选择程序"
                } else {
                    "已找到 stockdb 候选，请在设置中选择确认后再启动"
                };
                self.set_status(|status| {
                    status.enabled = true;
                    status.state = "not_configured".into();
                    status.busy = false;
                    status.message = message.into();
                    status.candidates = candidates;
                });
                return Err(message.into());
            }
        };
        match self.start_owned_locked(&engine).await {
            Ok(()) => Ok(()),
            Err(error) => {
                if !self.runtime.lock().unwrap_or_else(|e| e.into_inner()).shutting_down {
                    self.fail(error.clone());
                }
                Err(error)
            }
        }
    }

    async fn start_owned_locked(&self, engine: &Path) -> Result<(), String> {
        if self.runtime.lock().unwrap_or_else(|e| e.into_inner()).shutting_down {
            return Err("应用正在退出，不再启动 stockdb".into());
        }
        let engine = validate_engine(engine)?;
        let updater = self.configured_updater(Some(&engine));
        self.set_status(|status| {
            status.enabled = true;
            status.state = "starting".into();
            status.phase = Some("probing".into());
            status.busy = true;
            status.engine_path = Some(path_string(&engine));
            status.updater_path = updater.as_deref().map(path_string);
            status.updater_available = updater.is_some();
            status.message = "正在启动 stockdb…".into();
            status.last_error = None;
        });

        let mut command = engine_command(&engine)?;
        let mut child = command.spawn().map_err(|e| format!("启动 stockdb 失败: {e}"))?;
        let pid = child.id();
        let job = match crate::agent::ProcessJob::assign(&child) {
            Ok(job) => job,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("无法接管 stockdb 进程树: {error}"));
            }
        };
        {
            let mut runtime = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
            runtime.child = Some(child);
            runtime.job = Some(job);
        }

        let config = LocalHistoryConfig {
            base_url: self.service_url(),
            timeout: Duration::from_secs(1),
        };
        for _ in 0..30 {
            {
                let mut runtime = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
                if runtime.shutting_down {
                    drop(runtime);
                    self.stop_owned_locked();
                    return Err("应用正在退出，已停止 stockdb 启动".into());
                }
                if let Some(child) = runtime.child.as_mut() {
                    if let Some(exit) = child.try_wait().map_err(|e| e.to_string())? {
                        runtime.child = None;
                        drop(runtime);
                        let error = format!("stockdb 启动后立即退出（{exit}）");
                        self.fail(error.clone());
                        return Err(error);
                    }
                }
            }
            if history::probe(&config).await.is_ok() {
                self.set_status(|status| {
                    status.state = "running_owned".into();
                    status.phase = None;
                    status.owned = true;
                    status.busy = false;
                    status.message = format!("stockdb 正在运行（PID {pid}）");
                    status.last_error = None;
                });
                log::info!("[stockdb] 已启动 {:?}，PID {}", engine, pid);
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        self.stop_owned_locked();
        let error = "stockdb 启动超时，未能连接本地服务".to_string();
        self.fail(error.clone());
        Err(error)
    }

    fn fail(&self, error: String) {
        self.set_status(|status| {
            status.state = "error".into();
            status.phase = None;
            status.owned = false;
            status.busy = false;
            status.message = error.clone();
            status.last_error = Some(error);
        });
    }

    fn stop_owned_locked(&self) -> bool {
        let (child, job) = {
            let mut runtime = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
            (runtime.child.take(), runtime.job.take())
        };
        if let Some(mut child) = child {
            let pid = child.id();
            if let Some(job) = job.as_ref() {
                job.terminate();
            }
            let _ = child.kill();
            let _ = child.wait();
            log::info!("[stockdb] 已停止 owned 进程 PID {pid}");
            true
        } else {
            false
        }
    }

    pub async fn scan(&self) -> Result<StockDbStatus, String> {
        if !cfg!(target_os = "windows") {
            return Err("stockdb 自动管理目前仅支持 Windows".into());
        }
        let _guard = self.operation.try_lock().map_err(|_| "stockdb 正在执行其他操作，请稍后再试")?;
        self.set_status(|status| {
            status.state = "locating".into();
            status.busy = true;
            status.message = "正在常见位置查找 stockdb…".into();
        });
        let candidates = discover_stockdb();
        // 扫描只返回候选；任何未由用户确认的可执行文件都不会被自动运行。
        let engine = self.configured_engine();
        let updater = self.configured_updater(engine.as_deref());
        self.set_status(|status| {
            status.enabled = self.enabled_in_db();
            status.state = if engine.is_some() { "configured" } else { "not_configured" }.into();
            status.busy = false;
            status.engine_path = engine.as_deref().map(path_string);
            status.updater_path = updater.as_deref().map(path_string);
            status.updater_available = updater.is_some();
            status.candidates = candidates;
            status.message = if engine.is_some() { "已找到 stockdb" } else { "常见位置未找到 stockdb.exe" }.into();
        });
        if self.enabled_in_db() && engine.is_some() {
            self.ensure_running_locked().await?;
        }
        Ok(self.status())
    }

    pub async fn select_engine(&self, path: PathBuf) -> Result<StockDbStatus, String> {
        let _guard = self.operation.try_lock().map_err(|_| "stockdb 正在执行其他操作，请稍后再试")?;
        let engine = validate_engine(&path)?;
        let old_configured_engine = self.configured_engine();
        let old_engine = self.db.get_setting("local_history_engine_path").map_err(|e| e.to_string())?.unwrap_or_default();
        let old_dir = self.db.get_setting("local_history_engine_dir").map_err(|e| e.to_string())?.unwrap_or_default();
        let old_updater = self.db.get_setting("local_history_updater_path").map_err(|e| e.to_string())?.unwrap_or_default();
        let was_owned = self.runtime.lock().unwrap_or_else(|e| e.into_inner()).child.is_some();
        self.save_engine(&engine)?;
        if was_owned {
            self.stop_owned_locked();
        }
        let updater = self.configured_updater(Some(&engine));
        self.set_status(|status| {
            status.engine_path = Some(path_string(&engine));
            status.updater_path = updater.as_deref().map(path_string);
            status.updater_available = updater.is_some();
            status.state = "configured".into();
            status.message = "stockdb 程序已保存".into();
            status.last_error = None;
        });
        if self.enabled_in_db() {
            if let Err(error) = self.ensure_running_locked().await {
                let _ = self.db.set_setting("local_history_engine_path", &old_engine);
                let _ = self.db.set_setting("local_history_engine_dir", &old_dir);
                let _ = self.db.set_setting("local_history_updater_path", &old_updater);
                if was_owned && old_configured_engine.is_some() {
                    let _ = self.ensure_running_locked().await;
                }
                return Err(format!("新 stockdb 启动失败，已恢复原配置：{error}"));
            }
        }
        Ok(self.status())
    }

    pub async fn select_updater(&self, path: PathBuf) -> Result<StockDbStatus, String> {
        let engine = self.configured_engine().ok_or("请先选择 stockdb.exe")?;
        let updater = validate_updater(&path, Some(&engine))?;
        self.db
            .set_setting("local_history_updater_path", &path_string(&updater))
            .map_err(|e| e.to_string())?;
        self.set_status(|status| {
            status.updater_path = Some(path_string(&updater));
            status.updater_available = true;
            status.message = "数据更新程序已保存".into();
            status.last_error = None;
        });
        Ok(self.status())
    }

    fn save_engine(&self, engine: &Path) -> Result<(), String> {
        let engine = validate_engine(engine)?;
        let dir = engine.parent().ok_or("stockdb 路径缺少父目录")?;
        self.db
            .set_setting("local_history_engine_path", &path_string(&engine))
            .map_err(|e| e.to_string())?;
        self.db
            .set_setting("local_history_engine_dir", &path_string(dir))
            .map_err(|e| e.to_string())?;
        if let Some(updater) = self.configured_updater(Some(&engine)) {
            self.db
                .set_setting("local_history_updater_path", &path_string(&updater))
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub async fn update(&self) -> Result<String,String> {
        let _guard=self.operation.try_lock().map_err(|_|"StockDB正在执行其他操作，请稍后再试")?;
        let now=chrono::Utc::now();let cfg=crate::stockdb_schedule::config(&self.db)?;
        crate::stockdb_schedule::claim(&self.db,&cfg,now,false,true)?;
        self.perform_update(now).await
    }

    pub async fn scheduled_update(&self,now:chrono::DateTime<chrono::Utc>,startup:bool)->Result<(),String>{
        if !cfg!(target_os="windows")||self.runtime.lock().unwrap_or_else(|e|e.into_inner()).shutting_down{return Ok(());}
        let cfg=crate::stockdb_schedule::config(&self.db)?;
        let record=self.db.stockdb_update_record()?;
        let state=crate::stockdb_schedule::due(&cfg,&record,now,startup,record.running_owner.is_some_and(crate::stockdb_schedule::owner_alive))?;
        if state!="due" {if state=="failed"{self.publish_update_failure(now)?;}return Ok(());}
        let Ok(_guard)=self.operation.try_lock() else{return Ok(());};
        if !crate::stockdb_schedule::claim(&self.db,&cfg,now,startup,false)?{self.publish_update_failure(now)?;return Ok(());}
        self.perform_update(now).await.map(|_|())
    }

    async fn perform_update(&self,now:chrono::DateTime<chrono::Utc>)->Result<String,String>{
        let result=self.update_locked(now).await;
        crate::stockdb_schedule::finish(&self.db,now,chrono::Utc::now(),match &result{Ok((_,as_of))=>Ok(as_of.clone()),Err(error)=>Err(error.clone())})?;
        self.set_status(|status|{status.busy=false;status.phase=None;if matches!(status.state.as_str(),"updating"|"restarting"){status.state="error".into();}if let Err(error)=&result{status.last_error=Some(error.clone());status.message=error.clone();}else if let Ok((message,_))=&result{status.message=message.clone();status.last_error=None;}});
        self.publish_update_failure(chrono::Utc::now())?;
        result.map(|(message,_)|message)
    }

    fn publish_update_failure(&self,now:chrono::DateTime<chrono::Utc>)->Result<(),String>{
        if let Some(error)=crate::stockdb_schedule::claim_failure_notice(&self.db,now)?{
            crate::notifications::publish(&self.app,serde_json::json!({"signal_kind":"system","signal_tag":"数据更新失败","title":"StockDB更新连续5次失败","body":format!("已每隔1分钟重试，今天的自动重试已停止。原因：{}。请检查网络和本地数据设置，修复后可点击更新数据重试。近期行情源兜底仍保留，模型会继续核对数据日期。",error.chars().take(600).collect::<String>()),"stockdb_update_alert":{"schema":"stockdb-update-failed-v1","day":crate::stockdb_schedule::day(now),"failures":5}}));
        }
        Ok(())
    }

    async fn verify_completed_data(&self,now:chrono::DateTime<chrono::Utc>)->Result<String,String>{
        let expected=crate::commands::research::model_completed_day(now)?;
        let mut previous=expected-chrono::Duration::days(1);
        let mut found=false;
        for _ in 0..35{if crate::datasource::trading_calendar::is_trading_day_at(now,previous)?{found=true;break;}previous-=chrono::Duration::days(1);}
        if !found{return Err("无法核对StockDB更新目标日期".into());}
        history::verify_completed_day(&LocalHistoryConfig::new(self.service_url()),&expected.to_string(),&previous.to_string()).await?;
        Ok(expected.to_string())
    }

    async fn update_locked(&self, now: chrono::DateTime<chrono::Utc>) -> Result<(String, Option<String>), String> {
        #[cfg(not(windows))]
        { let _ = now; Err("stockdb 数据更新目前仅支持 Windows".into()) }
        #[cfg(windows)]
        {
            if !self.enabled_in_db() {
                return Err("请先开启本地历史数据".into());
            }
            let engine = validate_engine(&self.configured_engine().ok_or("请先选择 stockdb.exe")?)?;
            let updater = self.configured_updater(Some(&engine))
                .ok_or("未找到“数据更新.exe”，请在设置中浏览选择")?;
            let cli = updater_supports_cli(&updater)?;
            let endpoint = ServiceEndpoint::parse(&self.service_url())?;
            let owned_pid = {
                let mut runtime = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(child) = runtime.child.as_mut() {
                    if child.try_wait().map_err(|e| format!("检查 stockdb 进程失败：{e}"))?.is_some() {
                        runtime.child = None;
                        runtime.job = None;
                        runtime.status.owned = false;
                    }
                }
                runtime.child.as_ref().map(Child::id)
            };
            // This preflight has no process side effects. A port occupant whose
            // executable/family/launch settings cannot be verified is untouched.
            let previous = local_service::capture(&engine, &endpoint, owned_pid)?;
            let mut service = LocalUpdateService {
                manager: self, engine, endpoint, previous,
                was_owned: owned_pid.is_some(), stopped_owned: false,
            };
            self.runtime.lock().unwrap_or_else(|e| e.into_inner()).update_active = true;
            let _activity = UpdateActivity(self);
            update_with_service_restore(&mut service, async {
                self.set_status(|status| {
                    status.phase = Some("running_updater".into());
                    status.message = "正在后台同步并核验StockDB数据…".into();
                });
                self.run_updater(&updater, false, cli).await?;
                if cli { self.run_updater(&updater, true, true).await?; }
                Ok(())
            }).await?;
            self.set_status(|status| {
                status.phase = Some("verifying_data".into());
                status.busy = true;
                status.message = "stockdb 已恢复，正在核对更新后的数据日期和完整性…".into();
            });
            let as_of = Some(self.verify_completed_data(now).await?);
            Ok(("本地数据更新完成，stockdb 已恢复运行".into(), as_of))
        }
    }

    async fn run_updater(&self, updater: &Path,verify:bool,cli:bool) -> Result<(), String> {
        let log_dir=self.app.path().app_log_dir().map_err(|e|e.to_string())?.join("stockdb-updates");
        std::fs::create_dir_all(&log_dir).map_err(|e|e.to_string())?;
        let log_path=log_dir.join(format!("{}.log",uuid::Uuid::new_v4()));
        let stdout=std::fs::File::create(&log_path).map_err(|e|e.to_string())?;
        let stderr=stdout.try_clone().map_err(|e|e.to_string())?;
        let started=std::time::Instant::now();
        {
            let mut runtime = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
            if runtime.shutting_down {
                return Err("应用正在退出，未启动数据更新程序".into());
            }
            let mut command=updater_command(updater,verify,cli)?;
            command.stdin(Stdio::null()).stdout(stdout).stderr(stderr);
            let mut child = command.spawn()
                .map_err(|e| format!("启动数据更新程序失败: {e}"))?;
            let job = match crate::agent::ProcessJob::assign(&child) {
                Ok(job) => job,
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("无法接管数据更新进程树: {error}"));
                }
            };
            log::info!("[stockdb] 数据更新程序已启动，PID {}", child.id());
            runtime.updater_child = Some(child);
            runtime.updater_job = Some(job);
        }

        loop {
            let result = {
                let mut runtime = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
                match runtime.updater_child.as_mut() {
                    Some(child) => match child.try_wait() {
                        Ok(Some(exit)) => {
                            runtime.updater_child = None;
                            runtime.updater_job = None;
                            Some(if exit.success() {
                                Ok(())
                            } else {
                                Err(format!("数据更新程序退出码异常：{exit}"))
                            })
                        }
                        Ok(None) => {
                            if started.elapsed()>Duration::from_secs(1800){
                                if let Some(job)=runtime.updater_job.take(){job.terminate();}
                                if let Some(mut child)=runtime.updater_child.take(){let _=child.kill();let _=child.wait();}
                                Some(Err("StockDB更新超过30分钟，本次停止；后续按重试规则处理".into()))
                            }else{None}
                        },
                        Err(error) => {
                            if let Some(job) = runtime.updater_job.take() {
                                job.terminate();
                            }
                            if let Some(mut child) = runtime.updater_child.take() {
                                let _ = child.kill();
                                let _ = child.wait();
                            }
                            Some(Err(format!("等待数据更新程序失败: {error}")))
                        }
                    },
                    None => Some(Err("数据更新程序已在应用退出时终止".into())),
                }
            };
            if let Some(result) = result {
                return result.map_err(|error|format!("{error}；更新日志：{}",log_path.display()));
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    pub fn is_updating(&self) -> bool {
        let runtime = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
        runtime.update_active || runtime.status.state == "updating"
    }

    pub fn notify_exit_blocked(&self) {
        self.set_status(|status| {
            status.message = "数据更新或服务恢复正在进行，请完成后再退出应用".into();
        });
    }

    pub fn shutdown(&self) {
        let (updater_child, updater_job) = {
            let mut runtime = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
            runtime.shutting_down = true;
            (runtime.updater_child.take(), runtime.updater_job.take())
        };
        if let Some(mut child) = updater_child {
            if let Some(job) = updater_job.as_ref() {
                job.terminate();
            }
            let _ = child.kill();
            let _ = child.wait();
            log::warn!("[stockdb] 应用退出，已终止数据更新程序");
        }
        self.stop_owned_locked();
    }
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn canonical_file(path: &Path, expected_name: &str) -> Result<PathBuf, String> {
    if !path.is_file() {
        return Err(format!("文件不存在：{}", path.display()));
    }
    let actual = path.file_name().and_then(|name| name.to_str()).unwrap_or_default();
    if !actual.eq_ignore_ascii_case(expected_name) {
        return Err(format!("请选择 {expected_name}"));
    }
    std::fs::canonicalize(path).map_err(|e| format!("无法读取文件路径：{e}"))
}

fn validate_engine(path: &Path) -> Result<PathBuf, String> {
    canonical_file(path, ENGINE_FILE)
}

fn validate_updater(path: &Path, engine: Option<&Path>) -> Result<PathBuf, String> {
    let updater = canonical_file(path, UPDATER_FILE)?;
    if let Some(engine) = engine {
        let engine = validate_engine(engine)?;
        if updater.parent() != engine.parent() {
            return Err("数据更新.exe 必须与 stockdb.exe 位于同一目录".into());
        }
    }
    Ok(updater)
}

fn discover_stockdb() -> Vec<StockDbCandidate> {
    if !cfg!(target_os = "windows") {
        return Vec::new();
    }
    let mut roots: Vec<(PathBuf, &str, usize)> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            roots.push((dir.to_path_buf(), "应用目录", 2));
            if let Some(parent) = dir.parent() {
                roots.push((parent.to_path_buf(), "应用附近", 2));
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        roots.push((cwd.clone(), "项目目录", 3));
        if let Some(parent) = cwd.parent() {
            roots.push((parent.to_path_buf(), "项目附近", 3));
        }
    }
    if let Some(dir) = dirs::desktop_dir() {
        roots.push((dir, "桌面", 2));
    }
    if let Some(dir) = dirs::download_dir() {
        roots.push((dir, "下载目录", 2));
    }
    discover_in_roots(&roots, 20)
}

fn discover_in_roots(roots: &[(PathBuf, &str, usize)], limit: usize) -> Vec<StockDbCandidate> {
    let mut found = Vec::new();
    let mut seen = HashSet::new();
    for (root, source, depth) in roots {
        scan_dir(root, *depth, source, limit, &mut seen, &mut found);
        if found.len() >= limit {
            break;
        }
    }
    found
}

fn scan_dir(
    dir: &Path,
    depth: usize,
    source: &str,
    limit: usize,
    seen: &mut HashSet<String>,
    found: &mut Vec<StockDbCandidate>,
) {
    if found.len() >= limit || !dir.is_dir() {
        return;
    }
    let engine = dir.join(ENGINE_FILE);
    if let Ok(engine) = validate_engine(&engine) {
        let key = path_string(&engine).to_lowercase();
        if seen.insert(key) {
            let updater = dir.join(UPDATER_FILE);
            found.push(StockDbCandidate {
                engine_path: path_string(&engine),
                updater_path: validate_updater(&updater, Some(&engine)).ok().as_deref().map(path_string),
                source: source.into(),
            });
        }
    }
    if depth == 0 || found.len() >= limit {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        if found.len() >= limit {
            break;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || matches!(name.as_str(), "node_modules" | "target" | "$RECYCLE.BIN" | "System Volume Information") {
            continue;
        }
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() && !kind.is_symlink() {
            scan_dir(&entry.path(), depth - 1, source, limit, seen, found);
        }
    }
}

/// 组装 stockdb 引擎的启动命令。
///
/// **必须带 `-d`**。free-stockdb 0.3.5 发行版 `stockdb.exe -h` 给出的官方选项是：
///
/// ```text
/// Usage:
/// Options:
/// -d    run as daemon
/// -s    option to start|stop|restart the server
/// -h    show this message
/// ```
///
/// 不带 `-d` 时它走桌面模式 —— 建托盘图标（`Shell_NotifyIconW`）并用
/// `ShellExecuteA` 打开它的网页，于是每次应用自动拉起 stockdb 都会弹浏览器。
/// Bull Arrives 只要后台 HTTP 服务，**任何路径都不许绕过 daemon 模式**。
///
/// 工作目录固定为引擎所在目录：它按相对路径读同目录下的 `stockdb.conf`。
///
/// 另注：仓库里还有一份「Fully Open-Source C++ Edition」，参数是
/// `--host/--port/--data/--help`，既不认 `-d`、也**完全不碰浏览器**；
/// 它的解析器会忽略未知参数，所以同一条命令行对两个版本都安全。
fn engine_command(engine: &Path) -> Result<Command, String> {
    let mut command = Command::new(engine);
    command.current_dir(engine.parent().ok_or("stockdb 路径缺少父目录")?);
    command.arg("-d");
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    hide_window(&mut command);
    Ok(command)
}

#[cfg(target_os = "windows")]
fn hide_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x08000000);
}

#[cfg(not(target_os = "windows"))]
fn hide_window(_command: &mut Command) {}

#[cfg(windows)]
mod local_service {
    use super::{ServiceEndpoint, Duration, Path, PathBuf};
    use std::collections::{HashMap, HashSet};
    use std::ffi::{c_void, OsString};
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::{BOOL, FILETIME, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows::Win32::Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION};
    use windows::Win32::Security::{EqualSid, GetTokenInformation, TokenElevation, TokenUser, TOKEN_QUERY, TOKEN_USER};
    use windows::Win32::System::Threading::{
        CreateProcessW, GetCurrentProcess, GetProcessTimes, IsWow64Process, OpenProcess, OpenProcessToken,
        QueryFullProcessImageNameW, TerminateProcess, WaitForSingleObject,
        CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION,
        PROCESS_NAME_FORMAT, PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION,
        PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, PROCESS_VM_READ, STARTUPINFOW,
    };

    // The existing windows dependency does not enable IP Helper or ToolHelp.
    // Keep these small ABI declarations local instead of changing dependencies.
    #[link(name = "iphlpapi")]
    extern "system" {
        fn GetExtendedTcpTable(table: *mut c_void, size: *mut u32, order: BOOL,
            family: u32, class: u32, reserved: u32) -> u32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> HANDLE;
        fn GetSystemDirectoryW(buffer: *mut u16, size: u32) -> u32;
        fn ProcessIdToSessionId(pid: u32, session: *mut u32) -> BOOL;
        fn Process32FirstW(snapshot: HANDLE, entry: *mut ProcessEntry) -> BOOL;
        fn Process32NextW(snapshot: HANDLE, entry: *mut ProcessEntry) -> BOOL;
        fn ReadProcessMemory(process: HANDLE, address: *const c_void, buffer: *mut c_void,
            size: usize, read: *mut usize) -> BOOL;
    }
    #[link(name = "ntdll")]
    extern "system" {
        fn NtQueryInformationProcess(process: HANDLE, class: u32, info: *mut c_void,
            size: u32, returned: *mut u32) -> i32;
    }
    #[repr(C)]
    struct ProcessEntry {
        size: u32, usage: u32, pid: u32, heap: usize, module: u32,
        threads: u32, parent: u32, priority: i32, flags: u32, name: [u16; 260],
    }

    fn error(context: &str) -> String { format!("{context}：{}", std::io::Error::last_os_error()) }
    fn handle(value: &OwnedHandle) -> HANDLE { HANDLE(value.as_raw_handle()) }
    unsafe fn own(value: HANDLE) -> OwnedHandle { OwnedHandle::from_raw_handle(value.0) }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct FileId(u32, u32, u32);

    fn file_id(path: &Path) -> Result<FileId, String> {
        let file = std::fs::File::open(path).map_err(|e| format!("无法核对引擎文件身份：{e}"))?;
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
            .map_err(|e| format!("无法核对引擎文件身份：{e}"))?;
        Ok(FileId(info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow))
    }

    fn system_console_host(path: &Path) -> Result<bool, String> {
        let mut directory = vec![0u16; 32768];
        let length = unsafe { GetSystemDirectoryW(directory.as_mut_ptr(), directory.len() as u32) } as usize;
        if length == 0 || length >= directory.len() { return Err(error("无法核对 Windows 控制台宿主身份")); }
        let console = PathBuf::from(OsString::from_wide(&directory[..length])).join("conhost.exe");
        Ok(file_id(path)? == file_id(&console)?)
    }

    struct VerifiedProcess {
        pid: u32,
        handle: OwnedHandle,
        created: u64,
        image: PathBuf,
    }

    impl VerifiedProcess {
        fn open(pid: u32, full: bool) -> Result<Self, String> {
            if pid == 0 || pid == std::process::id() { return Err("StockDB 监听进程身份不安全".into()); }
            let mut access = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE;
            if full { access |= PROCESS_QUERY_INFORMATION | PROCESS_VM_READ | PROCESS_TERMINATE; }
            let raw = unsafe { OpenProcess(access, false, pid) }
                .map_err(|e| format!("无法核对或管理本地监听进程 PID {pid}：{e}"))?;
            let owned = unsafe { own(raw) };
            let mut name = vec![0u16; 32768];
            let mut length = name.len() as u32;
            unsafe { QueryFullProcessImageNameW(raw, PROCESS_NAME_FORMAT(0), PWSTR(name.as_mut_ptr()), &mut length) }
                .map_err(|e| format!("无法核对监听程序路径 PID {pid}：{e}"))?;
            let mut times = [FILETIME::default(); 4];
            let [created, exit, kernel, user] = &mut times;
            unsafe { GetProcessTimes(raw, created, exit, kernel, user) }
                .map_err(|e| format!("无法核对监听进程启动时间 PID {pid}：{e}"))?;
            let created = (u64::from(times[0].dwHighDateTime) << 32) | u64::from(times[0].dwLowDateTime);
            Ok(Self { pid, handle: owned, created,
                image: PathBuf::from(OsString::from_wide(&name[..length as usize])) })
        }
        fn alive(&self) -> Result<bool, String> {
            match unsafe { WaitForSingleObject(handle(&self.handle), 0) } {
                WAIT_TIMEOUT => Ok(true), WAIT_OBJECT_0 => Ok(false),
                _ => Err(error("无法核对 StockDB 进程状态")),
            }
        }
        fn stop(&self) -> Result<(), String> {
            if self.alive()? {
                unsafe { TerminateProcess(handle(&self.handle), 1) }
                    .map_err(|e| format!("停止已确认的 StockDB 进程 PID {} 失败：{e}", self.pid))?;
            }
            Ok(())
        }
    }

    pub(super) fn listener_pids(endpoint: &ServiceEndpoint) -> Result<HashSet<u32>, String> {
        let mut pids = HashSet::new();
        for family in [2u32, 23] { // AF_INET, AF_INET6; OWNER_PID_LISTENER only.
            let mut size = 0u32;
            let first = unsafe { GetExtendedTcpTable(std::ptr::null_mut(), &mut size, BOOL(0), family, 3, 0) };
            if first != 0 && first != 122 { return Err(format!("读取本地监听端口失败（{first}）")); }
            let mut table = Vec::<u32>::new();
            let mut complete = false;
            for _ in 0..4 {
                if !(4..=16 * 1024 * 1024).contains(&size) { return Err("本地端口表大小无效".into()); }
                table.resize((size as usize + 3) / 4, 0);
                let result = unsafe { GetExtendedTcpTable(table.as_mut_ptr().cast(), &mut size, BOOL(0), family, 3, 0) };
                if result == 0 { complete = true; break; }
                if result != 122 { return Err(format!("读取本地监听端口失败（{result}）")); }
            }
            if !complete { return Err("本地监听端口变化过快，未能安全核对".into()); }
            let bytes = unsafe { std::slice::from_raw_parts(table.as_ptr().cast::<u8>(), table.len() * 4) };
            let count = table[0] as usize;
            let row_size = if family == 2 { 24 } else { 56 };
            if count > (bytes.len() - 4) / row_size { return Err("本地端口表格式无效".into()); }
            for row in bytes[4..4 + count * row_size].chunks_exact(row_size) {
                let word = |offset| u32::from_ne_bytes(row[offset..offset + 4].try_into().unwrap());
                let (address, port, pid) = if family == 2 {
                    (std::net::IpAddr::V4(std::net::Ipv4Addr::from(<[u8; 4]>::try_from(&row[4..8]).unwrap())),
                        (word(8) as u16).swap_bytes(), word(20))
                } else {
                    (std::net::IpAddr::V6(std::net::Ipv6Addr::from(<[u8; 16]>::try_from(&row[..16]).unwrap())),
                        (word(20) as u16).swap_bytes(), word(52))
                };
                if endpoint.matches_listener(address, port) { pids.insert(pid); }
            }
        }
        Ok(pids)
    }

    fn parents() -> Result<HashMap<u32, u32>, String> {
        let raw = unsafe { CreateToolhelp32Snapshot(2, 0) }; // TH32CS_SNAPPROCESS
        if raw.is_invalid() { return Err(error("读取本地进程关系失败")); }
        let snapshot = unsafe { own(raw) };
        let mut entry: ProcessEntry = unsafe { std::mem::zeroed() };
        entry.size = std::mem::size_of::<ProcessEntry>() as u32;
        let mut result = HashMap::new();
        if !unsafe { Process32FirstW(handle(&snapshot), &mut entry) }.as_bool() {
            return Err(error("读取本地进程关系失败"));
        }
        loop {
            result.insert(entry.pid, entry.parent);
            if !unsafe { Process32NextW(handle(&snapshot), &mut entry) }.as_bool() {
                if std::io::Error::last_os_error().raw_os_error() != Some(18) {
                    return Err(error("读取本地进程关系失败"));
                }
                break;
            }
        }
        Ok(result)
    }

    fn family(engine: &Path, endpoint: &ServiceEndpoint, expected_root: Option<u32>)
        -> Result<Vec<VerifiedProcess>, String> {
        let selected = file_id(engine)?;
        let owners = listener_pids(endpoint)?;
        if owners.is_empty() && expected_root.is_none() { return Ok(Vec::new()); }
        let parents = parents()?;
        let mut root = expected_root;
        for pid in owners {
            let mut process = VerifiedProcess::open(pid, false)?;
            if file_id(&process.image)? != selected {
                return Err(format!("本地端口 {} 的进程 PID {pid} 与所选 StockDB 引擎不匹配，未停止任何进程", endpoint.port));
            }
            let mut seen = HashSet::new();
            while Some(process.pid) != expected_root {
                if !seen.insert(process.pid) { return Err("本地进程关系存在循环".into()); }
                let Some(&parent) = parents.get(&process.pid) else { break; };
                let Ok(candidate) = VerifiedProcess::open(parent, false) else { break; };
                if candidate.created > process.created || file_id(&candidate.image)? != selected { break; }
                process = candidate;
            }
            if root.is_some_and(|value| value != process.pid) {
                return Err("本地端口不属于同一个已确认的 StockDB 进程树，未停止任何进程".into());
            }
            root = Some(process.pid);
        }
        let mut processes = vec![VerifiedProcess::open(root.ok_or("无法确定 StockDB 根进程")?, true)?];
        if file_id(&processes[0].image)? != selected { return Err("StockDB 根进程与所选引擎不匹配".into()); }
        let mut index = 0;
        while index < processes.len() {
            let parent_pid = processes[index].pid;
            let parent_created = processes[index].created;
            for (&pid, &parent) in &parents {
                if parent != parent_pid || processes.iter().any(|p| p.pid == pid) { continue; }
                let child = VerifiedProcess::open(pid, false)?;
                if child.created < parent_created { continue; } // Reused parent PID.
                if file_id(&child.image)? != selected {
                    // Windows attaches its console host even to hidden console
                    // processes. It does not hold StockDB data and is never a
                    // termination target. Require the actual system file ID.
                    if system_console_host(&child.image)? { continue; }
                    return Err(format!("StockDB 进程树的子进程 PID {pid}（{}）与所选引擎不匹配，未停止任何进程", child.image.display()));
                }
                let managed = VerifiedProcess::open(pid, true)?;
                if managed.created != child.created || file_id(&managed.image)? != selected {
                    return Err("StockDB 子进程身份在核对期间发生变化，未停止任何进程".into());
                }
                processes.push(managed);
                if processes.len() > 64 { return Err("StockDB 进程树超出安全管理范围".into()); }
            }
            index += 1;
        }
        Ok(processes)
    }

    fn token_profile(process: HANDLE) -> Result<(Vec<usize>, u32), String> {
        let mut raw = HANDLE::default();
        unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut raw) }
            .map_err(|e| format!("无法核对 StockDB 服务账户：{e}"))?;
        let token = unsafe { own(raw) };
        // TOKEN_USER plus the bounded Windows SID fits in this aligned buffer.
        let mut user = vec![0usize; 128];
        let mut returned = 0u32;
        unsafe { GetTokenInformation(handle(&token), TokenUser, Some(user.as_mut_ptr().cast()),
            (user.len() * std::mem::size_of::<usize>()) as u32, &mut returned) }
            .map_err(|e| format!("无法核对 StockDB 服务账户：{e}"))?;
        let mut elevated = 0u32;
        unsafe { GetTokenInformation(handle(&token), TokenElevation, Some((&mut elevated as *mut u32).cast()),
            4, &mut returned) }.map_err(|e| format!("无法核对 StockDB 服务权限：{e}"))?;
        Ok((user, elevated))
    }

    fn confirm_restore_context(process: &VerifiedProcess) -> Result<(), String> {
        let mut service_session = 0u32;
        let mut app_session = 0u32;
        if !unsafe { ProcessIdToSessionId(process.pid, &mut service_session) }.as_bool()
            || !unsafe { ProcessIdToSessionId(std::process::id(), &mut app_session) }.as_bool() {
            return Err(error("无法核对 StockDB 服务会话"));
        }
        if service_session != app_session { return Err("外部 StockDB 属于其他登录会话，无法安全按原身份恢复，未停止服务".into()); }
        let (service_user, service_elevated) = token_profile(handle(&process.handle))?;
        let (app_user, app_elevated) = token_profile(unsafe { GetCurrentProcess() })?;
        let same_user = unsafe { EqualSid((*(service_user.as_ptr().cast::<TOKEN_USER>())).User.Sid,
            (*(app_user.as_ptr().cast::<TOKEN_USER>())).User.Sid) }.is_ok();
        if !same_user || service_elevated != app_elevated {
            return Err("外部 StockDB 的账户或管理员权限与应用不同，无法安全按原身份恢复，未停止服务".into());
        }
        Ok(())
    }

    #[derive(PartialEq, Eq)]
    struct LaunchSettings {
        command_line: Vec<u16>,
        directory: Vec<u16>,
        environment: Vec<u16>,
    }

    fn read_memory(process: &VerifiedProcess, address: usize, size: usize) -> Result<Vec<u8>, String> {
        if address == 0 || size > 1024 * 1024 { return Err("StockDB 启动参数地址或长度无效".into()); }
        let mut bytes = vec![0u8; size];
        let mut read = 0usize;
        if !unsafe { ReadProcessMemory(handle(&process.handle), address as *const c_void,
            bytes.as_mut_ptr().cast(), size, &mut read) }.as_bool() || read != size {
            return Err(error("无法完整记录 StockDB 原始启动参数，未安全接管服务"));
        }
        Ok(bytes)
    }

    fn pointer(bytes: &[u8], offset: usize, width: usize) -> usize {
        if width == 4 { u32::from_ne_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize }
        else { u64::from_ne_bytes(bytes[offset..offset + 8].try_into().unwrap()) as usize }
    }

    #[derive(Clone, Copy)]
    struct ParameterLayout {
        peb_parameters: usize,
        directory: usize,
        command_line: usize,
        environment: usize,
        parameters_size: usize,
    }
    fn parameter_layout(width: usize) -> ParameterLayout {
        match width {
            4 => ParameterLayout { peb_parameters: 0x10, directory: 0x24, command_line: 0x40, environment: 0x48, parameters_size: 0x4c },
            8 => ParameterLayout { peb_parameters: 0x20, directory: 0x38, command_line: 0x70, environment: 0x80, parameters_size: 0x88 },
            _ => unreachable!("Windows pointer width must be 4 or 8"),
        }
    }
    fn unicode_descriptor(bytes: &[u8], offset: usize, width: usize) -> Result<(usize, usize), String> {
        let size = u16::from_ne_bytes(bytes[offset..offset + 2].try_into().unwrap()) as usize;
        let maximum = u16::from_ne_bytes(bytes[offset + 2..offset + 4].try_into().unwrap()) as usize;
        if size == 0 || size % 2 != 0 || size > maximum { return Err("StockDB 原始启动参数格式无效".into()); }
        let address = pointer(bytes, offset + width, width);
        if address == 0 || address % 2 != 0 { return Err("StockDB 原始 Unicode 参数地址无效".into()); }
        Ok((address, size))
    }
    fn unicode(process: &VerifiedProcess, bytes: &[u8], offset: usize, width: usize) -> Result<Vec<u16>, String> {
        let (address, size) = unicode_descriptor(bytes, offset, width)?;
        let data = read_memory(process, address, size)?;
        let result: Vec<u16> = data.chunks_exact(2).map(|v| u16::from_ne_bytes(v.try_into().unwrap())).collect();
        if result.contains(&0) { return Err("StockDB 原始启动参数含意外终止符".into()); }
        Ok(result)
    }

    fn launch_settings(process: &VerifiedProcess) -> Result<LaunchSettings, String> {
        let mut basic = [0usize; 6];
        let status = unsafe { NtQueryInformationProcess(handle(&process.handle), 0,
            basic.as_mut_ptr().cast(), std::mem::size_of_val(&basic) as u32, std::ptr::null_mut()) };
        if status < 0 { return Err(format!("无法记录 StockDB 进程参数（NTSTATUS {status:#x}）")); }
        let mut wow64 = BOOL(0);
        unsafe { IsWow64Process(handle(&process.handle), &mut wow64) }.map_err(|e| e.to_string())?;
        let mut width = std::mem::size_of::<usize>();
        let mut peb = basic[1];
        if width == 8 && wow64.as_bool() {
            let status = unsafe { NtQueryInformationProcess(handle(&process.handle), 26,
                (&mut peb as *mut usize).cast(), width as u32, std::ptr::null_mut()) };
            if status < 0 || peb == 0 { return Err("无法记录 32 位 StockDB 的启动参数".into()); }
            width = 4;
        }
        if width == 4 && !wow64.as_bool() {
            let mut current_wow64 = BOOL(0);
            unsafe { IsWow64Process(windows::Win32::System::Threading::GetCurrentProcess(), &mut current_wow64) }
                .map_err(|e| e.to_string())?;
            if current_wow64.as_bool() { return Err("32 位应用无法安全记录 64 位 StockDB 的启动参数".into()); }
        }
        // PEB.ProcessParameters and the stable initial RTL_USER_PROCESS_PARAMETERS
        // fields. Decode the target's pointer width; never dereference remote pointers.
        let layout = parameter_layout(width);
        let peb_bytes = read_memory(process, peb, layout.peb_parameters + width)?;
        let parameters = pointer(&peb_bytes, layout.peb_parameters, width);
        let bytes = read_memory(process, parameters, layout.parameters_size)?;
        let command_line = unicode(process, &bytes, layout.command_line, width)?;
        let directory = unicode(process, &bytes, layout.directory, width)?;
        let environment_pointer = pointer(&bytes, layout.environment, width);
        if environment_pointer == 0 || environment_pointer % 2 != 0 {
            return Err("StockDB 原始环境参数地址无效，未停止服务".into());
        }
        let mut environment = Vec::new();
        let mut address = environment_pointer;
        loop {
            // Read to the next page boundary, so an otherwise valid environment
            // at the end of a mapped region does not cross into unreadable memory.
            let size = 4096 - address % 4096;
            let data = read_memory(process, address, size)?;
            for pair in data.chunks_exact(2) {
                environment.push(u16::from_ne_bytes(pair.try_into().unwrap()));
                if environment.ends_with(&[0, 0]) {
                    return Ok(LaunchSettings { command_line, directory, environment });
                }
            }
            if environment.len() * 2 >= 1024 * 1024 { return Err("StockDB 环境参数超出安全记录范围".into()); }
            address = address.checked_add(size).ok_or("StockDB 环境参数地址溢出")?;
        }
    }

    pub(super) struct CapturedService {
        engine: PathBuf,
        identity: FileId,
        processes: Vec<VerifiedProcess>,
        launch: Option<LaunchSettings>,
        stopped_any: bool,
        restarted: Option<VerifiedProcess>,
    }

    pub(super) fn capture(engine: &Path, endpoint: &ServiceEndpoint, owned_pid: Option<u32>)
        -> Result<Option<CapturedService>, String> {
        let processes = family(engine, endpoint, owned_pid)?;
        if processes.is_empty() { return Ok(None); }
        let launch = if owned_pid.is_none() {
            for process in &processes { confirm_restore_context(process)?; }
            Some(launch_settings(&processes[0])?)
        } else { None };
        Ok(Some(CapturedService { engine: engine.to_owned(), identity: file_id(engine)?,
            processes, launch, stopped_any: false, restarted: None }))
    }

    pub(super) fn confirm_listener(engine: &Path, endpoint: &ServiceEndpoint, pid: u32) -> Result<(), String> {
        if listener_pids(endpoint)?.is_empty() { return Err("恢复后的 StockDB 未监听配置端口".into()); }
        family(engine, endpoint, Some(pid)).map(|_| ())
    }

    impl CapturedService {
        pub(super) fn revalidate(&self, endpoint: &ServiceEndpoint) -> Result<(), String> {
            if file_id(&self.engine)? != self.identity { return Err("所选 StockDB 文件身份已变化，未停止服务".into()); }
            let current = family(&self.engine, endpoint, Some(self.processes[0].pid))?;
            let keys = |processes: &[VerifiedProcess]| processes.iter().map(|p| (p.pid, p.created)).collect::<HashSet<_>>();
            if keys(&current) != keys(&self.processes) { return Err("StockDB 进程树已变化，未停止服务".into()); }
            if let Some(launch) = &self.launch {
                if launch_settings(&self.processes[0])? != *launch {
                    return Err("StockDB 启动参数已变化，未停止服务".into());
                }
            }
            Ok(())
        }
        pub(super) fn stop(&mut self) -> Result<(), String> {
            // Handles pin the verified process objects: no name-based taskkill,
            // PID reopening, wildcard python termination, or job-tree adoption.
            for process in &self.processes {
                if process.alive()? {
                    process.stop()?;
                    self.stopped_any = true;
                }
            }
            Ok(())
        }
        pub(super) async fn wait_stopped(&self) -> Result<(), String> {
            for _ in 0..50 {
                let alive = self.processes.iter().map(VerifiedProcess::alive).collect::<Result<Vec<_>, _>>()?;
                if alive.iter().all(|value| !value) { return Ok(()); }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err("已请求停止 StockDB，但原进程尚未退出，未运行更新".into())
        }
        pub(super) async fn restore(&mut self, endpoint: &ServiceEndpoint) -> Result<u32, String> {
            if !self.stopped_any && self.processes[0].alive()? { return Ok(self.processes[0].pid); }
            // Finish a partial stop using the already verified handles only.
            self.stop()?;
            self.wait_stopped().await?;
            if !listener_pids(endpoint)?.is_empty() { return Err("恢复时本地端口已被占用，未启动重复服务".into()); }
            if file_id(&self.engine)? != self.identity { return Err("所选 StockDB 引擎文件已变化，无法按原身份恢复".into()); }
            let launch = self.launch.as_ref().ok_or("缺少外部 StockDB 原始启动参数")?;
            let mut command_line = launch.command_line.clone(); command_line.push(0);
            let mut directory = launch.directory.clone(); directory.push(0);
            let original_image = &self.processes[0].image;
            if file_id(original_image)? != self.identity { return Err("外部 StockDB 原始程序路径已变化，无法按原参数恢复".into()); }
            let engine: Vec<u16> = original_image.as_os_str().encode_wide().chain(Some(0)).collect();
            let startup = STARTUPINFOW { cb: std::mem::size_of::<STARTUPINFOW>() as u32, ..Default::default() };
            let mut info = PROCESS_INFORMATION::default();
            unsafe { CreateProcessW(PCWSTR(engine.as_ptr()), PWSTR(command_line.as_mut_ptr()), None, None, false,
                CREATE_UNICODE_ENVIRONMENT | CREATE_NO_WINDOW, Some(launch.environment.as_ptr().cast()),
                PCWSTR(directory.as_ptr()), &startup, &mut info) }
                .map_err(|e| format!("按原参数恢复外部 StockDB 失败：{e}"))?;
            let process_handle = unsafe { own(info.hProcess) };
            let _thread_handle = unsafe { own(info.hThread) };
            // Restored external services deliberately remain outside the app's
            // kill-on-close ProcessJob. App-owned engines/updaters still use it.
            let verified = VerifiedProcess::open(info.dwProcessId, true)?;
            self.restarted = Some(verified);
            drop(process_handle);
            Ok(info.dwProcessId)
        }
        #[cfg(test)]
        pub(super) fn cleanup_fixture(&mut self, endpoint: &ServiceEndpoint) {
            // Test cleanup also covers a restored supervisor before its worker
            // starts listening; all targets retain the unique fixture file ID.
            if let Some(restarted) = self.restarted.as_ref() {
                if restarted.alive().unwrap_or(false) {
                    if let Ok(processes) = family(&self.engine, endpoint, Some(restarted.pid)) {
                        if processes.first().is_some_and(|p| p.created == restarted.created) {
                            for process in processes { let _ = process.stop(); }
                        }
                    }
                    let _ = restarted.stop();
                }
            }
            let _ = self.stop();
        }
    }
    #[cfg(test)]
    mod layout_tests {
        use super::*;
        #[test]
        fn process_parameters_decode_command_directory_and_environment_at_x64_and_x86_offsets() {
            // Independent byte fixtures use the actual ABI offsets, including
            // the x64 UNICODE_STRING padding before Buffer (8 versus 4 bytes).
            for (width, peb_offset, directory_offset, command_offset, environment_offset, size) in
                [(8, 0x20, 0x38, 0x70, 0x80, 0x88), (4, 0x10, 0x24, 0x40, 0x48, 0x4c)] {
                let mut peb = vec![0u8; peb_offset + width];
                let mut parameters = vec![0u8; size];
                let write_pointer = |bytes: &mut [u8], offset: usize, value: u64| {
                    bytes[offset..offset + width].copy_from_slice(&value.to_ne_bytes()[..width]);
                };
                write_pointer(&mut peb, peb_offset, 0x12340000);
                for (offset, address, length) in [(directory_offset, 0x45670000, 36u16), (command_offset, 0x56780000, 128u16)] {
                    parameters[offset..offset + 2].copy_from_slice(&length.to_ne_bytes());
                    parameters[offset + 2..offset + 4].copy_from_slice(&(length + 2).to_ne_bytes());
                    write_pointer(&mut parameters, offset + width, address);
                }
                write_pointer(&mut parameters, environment_offset, 0x67890000);
                let layout = parameter_layout(width);
                assert_eq!(layout.parameters_size, size);
                assert_eq!(pointer(&peb, layout.peb_parameters, width), 0x12340000);
                assert_eq!(unicode_descriptor(&parameters, layout.directory, width).unwrap(), (0x45670000, 36));
                assert_eq!(unicode_descriptor(&parameters, layout.command_line, width).unwrap(), (0x56780000, 128));
                assert_eq!(pointer(&parameters, layout.environment, width), 0x67890000);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockUpdateService {
        events: Arc<Mutex<Vec<&'static str>>>,
        stop_error: Option<String>,
        restore_error: Option<String>,
    }
    impl UpdateService for MockUpdateService {
        async fn stop(&mut self) -> Result<(), String> {
            self.events.lock().unwrap().push("stop");
            self.stop_error.take().map_or(Ok(()), Err)
        }
        async fn restore(&mut self) -> Result<bool, String> {
            self.events.lock().unwrap().push("restore");
            self.restore_error.take().map_or(Ok(true), Err)
        }
    }

    #[tokio::test]
    async fn lifecycle_stops_before_sync_verify_and_restores_on_success() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut service = MockUpdateService { events: events.clone(), stop_error: None, restore_error: None };
        update_with_service_restore(&mut service, async {
            assert_eq!(*events.lock().unwrap(), ["stop"]);
            events.lock().unwrap().extend(["sync", "verify"]);
            Ok(())
        }).await.unwrap();
        assert_eq!(*events.lock().unwrap(), ["stop", "sync", "verify", "restore"]);
    }

    #[tokio::test]
    async fn lifecycle_restores_after_sync_or_verify_failure() {
        for fail_verify in [false, true] {
            let events = Arc::new(Mutex::new(Vec::new()));
            let mut service = MockUpdateService { events: events.clone(), stop_error: None, restore_error: None };
            let result = update_with_service_restore(&mut service, async {
                events.lock().unwrap().push("sync");
                if fail_verify { events.lock().unwrap().push("verify"); }
                Err(if fail_verify { "verify failed" } else { "sync failed" }.into())
            }).await.unwrap_err();
            assert!(result.contains("已恢复运行"));
            assert_eq!(events.lock().unwrap().last(), Some(&"restore"));
        }
    }

    #[tokio::test]
    async fn lifecycle_partial_stop_failure_skips_update_and_attempts_restore() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut service = MockUpdateService { events: events.clone(), stop_error: Some("stop failed".into()), restore_error: None };
        let result = update_with_service_restore(&mut service, async {
            events.lock().unwrap().push("must not update");
            Ok(())
        }).await.unwrap_err();
        assert!(result.contains("stop failed"));
        assert_eq!(*events.lock().unwrap(), ["stop", "restore"]);
    }

    #[tokio::test]
    async fn lifecycle_preserves_update_and_restore_errors() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut service = MockUpdateService { events, stop_error: None, restore_error: Some("restore failed".into()) };
        let result = update_with_service_restore(&mut service, async { Err("update failed".into()) }).await.unwrap_err();
        assert!(result.contains("update failed"));
        assert!(result.contains("restore failed"));
    }

    #[tokio::test]
    async fn service_url_change_is_blocked_through_update_and_recovery() {
        fn assert_blocked(db: &Database, operation: &tokio::sync::Mutex<()>) {
            assert!(save_service_url(db, operation, "http://127.0.0.1:7900")
                .unwrap_err().contains("服务地址暂不能修改"));
            assert_eq!(db.get_setting("local_history_url").unwrap().as_deref(), Some("http://127.0.0.1:7899"));
        }
        struct Service<'a> {
            db: &'a Database,
            operation: &'a tokio::sync::Mutex<()>,
            fail_stop: bool,
            fail_restore: bool,
        }
        impl UpdateService for Service<'_> {
            async fn stop(&mut self) -> Result<(), String> {
                assert_blocked(self.db, self.operation);
                if self.fail_stop { Err("stop failed".into()) } else { Ok(()) }
            }
            async fn restore(&mut self) -> Result<bool, String> {
                assert_blocked(self.db, self.operation);
                if self.fail_restore { Err("restore failed".into()) } else { Ok(true) }
            }
        }

        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("tmp")
            .join(format!("stockdb-url-guard-{}", uuid::Uuid::new_v4()));
        let db = Database::open(root.clone()).unwrap();
        let operation = tokio::sync::Mutex::new(());
        for (fail_stop, fail_update, fail_restore) in [
            (false, false, false), (false, true, false), (false, false, true),
            (false, true, true), (true, false, false),
        ] {
            save_service_url(&db, &operation, "http://127.0.0.1:7899").unwrap();
            let guard = operation.try_lock().unwrap();
            let mut service = Service { db: &db, operation: &operation, fail_stop, fail_restore };
            let result = update_with_service_restore(&mut service, async {
                assert_blocked(&db, &operation);
                if fail_update { Err("update failed".into()) } else { Ok(()) }
            }).await;
            assert_eq!(result.is_err(), fail_stop || fail_update || fail_restore);
            // Final data verification still belongs to the same operation.
            assert_blocked(&db, &operation);
            drop(guard);
            save_service_url(&db, &operation, "http://127.0.0.1:7900").unwrap();
            assert_eq!(db.get_setting("local_history_url").unwrap().as_deref(), Some("http://127.0.0.1:7900"));
        }
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn update_endpoint_rejects_remote_and_authenticated_services() {
        for url in ["http://192.168.1.2:7899", "http://example.com:7899", "http://user@127.0.0.1:7899", "https://127.0.0.1:7899"] {
            assert!(ServiceEndpoint::parse(url).is_err(), "{url}");
        }
        assert_eq!(ServiceEndpoint::parse("http://localhost:7899").unwrap().addresses.len(), 2);
        assert_eq!(ServiceEndpoint::parse("http://[::1]:7899").unwrap().port, 7899);
        assert!(ServiceEndpoint::parse("http://127.2.3.4:7899").is_ok());
    }

    // This ignored test is only invoked in a copied test executable, in a unique
    // temporary directory, on an ephemeral port. It never launches StockDB.
    #[cfg(windows)]
    #[test]
    #[ignore = "isolated subprocess fixture; launched by the lifecycle tests"]
    fn isolated_service_process() {
        use std::io::Write;
        let Ok(address) = std::env::var("BULL_STOCKDB_TEST_ADDRESS") else { return; };
        let role = std::env::var("BULL_STOCKDB_TEST_ROLE").unwrap();
        if role == "supervisor" {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command.args(["stockdb::tests::isolated_service_process", "--exact", "--ignored", "--nocapture", "--test-threads=1"])
                .env("BULL_STOCKDB_TEST_ROLE", "listener").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
            hide_window(&mut command);
            let mut child = command.spawn().unwrap();
            let _ = child.wait();
            return;
        }
        let listener = std::net::TcpListener::bind(&address).unwrap();
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let report = format!("{}\n{}\n{}", std::env::current_dir().unwrap().display(),
                std::env::var("BULL_STOCKDB_TEST_VALUE").unwrap(), std::env::args().collect::<Vec<_>>().join("|"));
            let _ = stream.write_all(report.as_bytes());
        }
    }

    #[cfg(windows)]
    struct IsolatedService {
        root: PathBuf,
        engine: PathBuf,
        address: std::net::SocketAddr,
        endpoint: ServiceEndpoint,
        child: Child,
        job: crate::agent::ProcessJob,
        cleanup: Vec<local_service::CapturedService>,
    }

    #[cfg(windows)]
    impl IsolatedService {
        fn start(supervisor: bool) -> Self {
            let root = std::env::temp_dir().join(format!("bull-stockdb-isolated-{}", uuid::Uuid::new_v4())).join("服务 参数");
            std::fs::create_dir_all(&root).unwrap();
            let engine = root.join(ENGINE_FILE);
            std::fs::copy(std::env::current_exe().unwrap(), &engine).unwrap();
            let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = socket.local_addr().unwrap();
            drop(socket);
            let endpoint = ServiceEndpoint::parse(&format!("http://{address}")).unwrap();
            let mut command = Command::new(&engine);
            command.current_dir(&root)
                .args(["stockdb::tests::isolated_service_process", "--exact", "--ignored", "--nocapture", "--test-threads=1"])
                .env("BULL_STOCKDB_TEST_ADDRESS", address.to_string())
                .env("BULL_STOCKDB_TEST_ROLE", if supervisor { "supervisor" } else { "listener" })
                .env("BULL_STOCKDB_TEST_VALUE", "参数 空格 custom=keep")
                .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
            hide_window(&mut command);
            let mut child = command.spawn().unwrap();
            let job = crate::agent::ProcessJob::assign(&child).unwrap_or_else(|error| {
                let _ = child.kill(); let _ = child.wait();
                panic!("isolated fixture job assignment failed: {error}");
            });
            let fixture = Self { root, engine, address, endpoint, child, job, cleanup: Vec::new() };
            fixture.report();
            fixture
        }
        fn report(&self) -> String { isolated_report(&self.address) }
    }

    #[cfg(windows)]
fn isolated_report(address: &std::net::SocketAddr) -> String {
        use std::io::Read;
        for _ in 0..100 {
            if let Ok(mut stream) = std::net::TcpStream::connect_timeout(address, Duration::from_millis(100)) {
                stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let mut report = String::new();
                stream.read_to_string(&mut report).unwrap();
                return report;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("isolated fixture did not listen on {}", address);
    }

    #[cfg(windows)]
    impl Drop for IsolatedService {
        fn drop(&mut self) {
            // Only handles verified against this unique copied test executable
            // are eligible for cleanup; no name-based or user-engine operations.
            if let Ok(Some(mut service)) = local_service::capture(&self.engine, &self.endpoint, None) {
                let _ = service.stop();
                self.cleanup.push(service);
            }
            for service in &mut self.cleanup { service.cleanup_fixture(&self.endpoint); }
            self.job.terminate();
            let _ = self.child.kill();
            let _ = self.child.wait();
            self.cleanup.clear();
            for _ in 0..40 {
                if std::fs::remove_dir_all(&self.root).is_ok() { break; }
                std::thread::sleep(Duration::from_millis(25));
            }
            if let Some(parent) = self.root.parent() { let _ = std::fs::remove_dir(parent); }
        }
    }

    #[cfg(windows)]
    struct IsolatedUpdateService<'a> {
        engine: &'a Path,
        endpoint: &'a ServiceEndpoint,
        address: std::net::SocketAddr,
        captured: &'a mut local_service::CapturedService,
    }
    #[cfg(windows)]
    impl UpdateService for IsolatedUpdateService<'_> {
        async fn stop(&mut self) -> Result<(), String> {
            self.captured.revalidate(self.endpoint)?;
            self.captured.stop()?;
            self.captured.wait_stopped().await?;
            assert!(local_service::listener_pids(self.endpoint)?.is_empty());
            Ok(())
        }
        async fn restore(&mut self) -> Result<bool, String> {
            let pid = self.captured.restore(self.endpoint).await?;
            isolated_report(&self.address);
            local_service::confirm_listener(self.engine, self.endpoint, pid)?;
            Ok(true)
        }
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn external_service_stops_updates_and_restores_original_launch_on_success_and_failure() {
        for (supervisor, fail) in [(false, false), (false, true), (true, false), (true, true)] {
            let mut fixture = IsolatedService::start(supervisor);
            let before = fixture.report();
            assert!(before.contains("参数 空格 custom=keep"));
            let captured = local_service::capture(&fixture.engine, &fixture.endpoint, None).unwrap().unwrap();
            fixture.cleanup.push(captured);
            let mut captured = fixture.cleanup.pop().unwrap();
            let endpoint = fixture.endpoint.clone();
            let address = fixture.address;
            let result = update_with_service_restore(&mut IsolatedUpdateService { engine: &fixture.engine, endpoint: &fixture.endpoint, address, captured: &mut captured }, async move {
                assert!(local_service::listener_pids(&endpoint).unwrap().is_empty());
                assert!(std::net::TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_err());
                if fail { Err("isolated updater failed".into()) } else { Ok(()) }
            }).await;
            fixture.cleanup.push(captured);
            assert_eq!(fixture.report(), before, "working directory, environment and arguments must survive");
            assert_eq!(result.is_err(), fail, "supervisor={supervisor}, result={result:?}");
            if fail { assert!(result.unwrap_err().contains("已恢复运行")); }
        }
    }

    #[cfg(windows)]
    #[test]
    fn mismatched_program_even_with_the_same_name_is_never_stopped() {
        let fixture = IsolatedService::start(false);
        let other = fixture.root.join("other");
        std::fs::create_dir_all(&other).unwrap();
        let engine = other.join(ENGINE_FILE);
        // Identical bytes and basename still denote a different selected file.
        std::fs::copy(&fixture.engine, &engine).unwrap();
        let error = local_service::capture(&engine, &fixture.endpoint, None).err().unwrap();
        assert!(error.contains("不匹配"));
        assert!(fixture.report().contains("custom=keep"));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn changed_port_occupant_blocks_update_and_restore_without_killing_it() {
        let mut fixture = IsolatedService::start(false);
        let mut captured = local_service::capture(&fixture.engine, &fixture.endpoint, None).unwrap().unwrap();
        captured.revalidate(&fixture.endpoint).unwrap();
        captured.stop().unwrap();
        captured.wait_stopped().await.unwrap();
        let unrelated = std::net::TcpListener::bind(fixture.address).unwrap();
        assert!(captured.restore(&fixture.endpoint).await.unwrap_err().contains("端口"));
        assert_eq!(unrelated.local_addr().unwrap(), fixture.address);
        assert!(unrelated.try_clone().is_ok());
        drop(unrelated);
        captured.restore(&fixture.endpoint).await.unwrap();
        fixture.report();
        fixture.cleanup.push(captured);
    }

    #[cfg(windows)]
    #[test]
    fn ipv6_listener_pid_is_identified_without_http_probing() {
        let listener = std::net::TcpListener::bind("[::1]:0").unwrap();
        let endpoint = ServiceEndpoint::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let pids = local_service::listener_pids(&endpoint).unwrap();
        assert!(pids.contains(&std::process::id()));
    }


    #[test]
    fn bounded_discovery_finds_engine_and_updater() {
        let root = std::env::temp_dir().join(format!("bull-stockdb-{}", std::process::id()));
        let nested = root.join("downloaded").join("stockdb");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join(ENGINE_FILE), b"test").unwrap();
        std::fs::write(nested.join(UPDATER_FILE), b"test").unwrap();
        let found = discover_in_roots(&[(root.clone(), "test", 2)], 10);
        assert_eq!(found.len(), 1);
        assert!(found[0].updater_path.is_some());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn updater_uses_explicit_sync_then_independent_verify_and_its_own_directory(){
        let path=std::env::temp_dir().join("stockdb-参数 空格").join("数据更新.exe");
        let sync=updater_command(&path,false,true).unwrap();let verify=updater_command(&path,true,true).unwrap();
        assert_eq!(sync.get_program(),path.as_os_str());assert_eq!(sync.get_current_dir(),path.parent());
        assert_eq!(sync.get_args().collect::<Vec<_>>(),vec![std::ffi::OsStr::new("--sync")]);
        assert_eq!(verify.get_args().collect::<Vec<_>>(),vec![std::ffi::OsStr::new("--verify")]);
        assert_eq!(updater_command(&path,false,false).unwrap().get_args().count(),0);
    }
    #[test]
    fn updater_capability_detection_never_runs_the_program(){
        let path=std::env::temp_dir().join(format!("bull-updater-capabilities-{}.bin",uuid::Uuid::new_v4()));
        std::fs::write(&path,b"old updater payload").unwrap();assert!(!updater_supports_cli(&path).unwrap());
        std::fs::write(&path,b"usage --sync or --verify").unwrap();assert!(updater_supports_cli(&path).unwrap());
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn updater_must_share_engine_directory() {
        let root = std::env::temp_dir().join(format!("bull-stockdb-updater-{}", std::process::id()));
        let first = root.join("first");
        let second = root.join("second");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        let engine = first.join(ENGINE_FILE);
        let updater = second.join(UPDATER_FILE);
        std::fs::write(&engine, b"test").unwrap();
        std::fs::write(&updater, b"test").unwrap();
        assert!(validate_updater(&updater, Some(&engine)).is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn engine_is_always_started_as_daemon() {
        // 回归保护：v2.1.1 这里没有 `-d`，结果每次由应用拉起 stockdb 都会
        // 弹出它的托盘图标和浏览器页面。`-d`（run as daemon）是官方选项，
        // 一旦被“简化”掉，这个用例必须失败。
        let dir = std::env::temp_dir().join(format!("bull-stockdb-daemon-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let engine = dir.join(ENGINE_FILE);
        std::fs::write(&engine, b"test").unwrap();

        let command = engine_command(&engine).expect("应能组装启动命令");
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert!(
            args.iter().any(|arg| arg == "-d"),
            "必须以 daemon 模式启动，否则会弹浏览器；实际参数：{args:?}"
        );
        assert_eq!(
            command.get_current_dir(),
            Some(dir.as_path()),
            "工作目录必须是引擎所在目录 —— 它按相对路径读同目录的 stockdb.conf"
        );

        let _ = std::fs::remove_dir_all(dir);
    }
}
