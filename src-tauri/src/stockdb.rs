use crate::datasource::history::{self, LocalHistoryConfig};
use crate::db::Database;
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

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
        }
    }
}

struct Runtime {
    status: StockDbStatus,
    child: Option<Child>,
    job: Option<crate::agent::ProcessJob>,
    updater_child: Option<Child>,
    updater_job: Option<crate::agent::ProcessJob>,
    shutting_down: bool,
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
        if status.engine_path.is_none() {
            status.engine_path = self.configured_engine().as_deref().map(path_string);
        }
        if status.updater_path.is_none() {
            status.updater_path = self
                .configured_updater(self.configured_engine().as_deref())
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
                status.message = "检测到已运行的外部 stockdb；Bull Arrives 不会结束它".into();
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
        let _guard = self.operation.try_lock().map_err(|_| "stockdb 正在执行其他操作，请稍后再试")?;
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

    pub async fn update(&self) -> Result<String, String> {
        if !cfg!(target_os = "windows") {
            return Err("stockdb 数据更新目前仅支持 Windows".into());
        }
        let _guard = self.operation.try_lock().map_err(|_| "stockdb 正在执行其他操作，请稍后再试")?;
        if !self.enabled_in_db() {
            return Err("请先开启本地历史数据".into());
        }
        let engine = self.configured_engine().ok_or("请先选择 stockdb.exe")?;
        let updater = self
            .configured_updater(Some(&engine))
            .ok_or("未找到“数据更新.exe”，请在设置中浏览选择")?;
        let was_owned = self.runtime.lock().unwrap_or_else(|e| e.into_inner()).child.is_some();
        if !was_owned && history::probe(&LocalHistoryConfig::new(self.service_url())).await.is_ok() {
            return Err("当前 stockdb 由外部启动。请先手工关闭它，再执行数据更新".into());
        }

        self.set_status(|status| {
            status.state = "updating".into();
            status.phase = Some("stopping".into());
            status.busy = true;
            status.message = "正在停止 stockdb…".into();
            status.last_error = None;
        });
        if was_owned {
            self.stop_owned_locked();
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        self.set_status(|status| {
            status.phase = Some("running_updater".into());
            status.message = "数据更新程序正在运行，请在其窗口中查看进度…".into();
        });
        let update_result = self.run_updater(&updater).await;

        let should_restart = self.enabled_in_db()
            && !self.runtime.lock().unwrap_or_else(|e| e.into_inner()).shutting_down;
        let restart_result = if should_restart {
            self.set_status(|status| {
                status.state = "restarting".into();
                status.phase = Some("probing".into());
                status.message = "正在重新启动 stockdb…".into();
            });
            self.ensure_running_locked().await
        } else {
            Ok(())
        };

        match (update_result, restart_result) {
            (Ok(()), Ok(())) => Ok("本地数据更新完成，stockdb 已恢复运行".into()),
            (Err(update), Ok(())) => {
                self.set_status(|status| {
                    status.message = format!("{update}；stockdb 已恢复运行");
                    status.last_error = Some(update.clone());
                });
                Err(format!("{update}；stockdb 已恢复运行"))
            }
            (Ok(()), Err(restart)) => Err(format!("数据更新完成，但 stockdb 重启失败：{restart}")),
            (Err(update), Err(restart)) => Err(format!("{update}；stockdb 重启也失败：{restart}")),
        }
    }

    async fn run_updater(&self, updater: &Path) -> Result<(), String> {
        {
            let mut runtime = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
            if runtime.shutting_down {
                return Err("应用正在退出，未启动数据更新程序".into());
            }
            let mut child = Command::new(updater)
                .current_dir(updater.parent().ok_or("数据更新程序路径缺少父目录")?)
                .spawn()
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
                        Ok(None) => None,
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
                return result;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    pub fn is_updating(&self) -> bool {
        self.runtime
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .status
            .state
            == "updating"
    }

    pub fn notify_exit_blocked(&self) {
        self.set_status(|status| {
            status.message = "数据更新正在进行，请等待更新窗口关闭后再退出应用".into();
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

#[cfg(test)]
mod tests {
    use super::*;

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
