use super::interactive::{self, InteractiveTask};
use super::{app_launcher_path, hide_window, minimal_environment, status, ProcessJob, START_FILE};
use crate::db::Database;
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};
use uuid::Uuid;

const MAX_LINE: usize = 256 * 1024;
const MAX_STREAM: usize = 2 * 1024 * 1024;
const MAX_EVENTS: usize = 240;
const MAX_TEXT: usize = 4000;
const TURN_TIMEOUT: Duration = Duration::from_secs(300);
const EVENT_NAME: &str = "agent-live-event";

#[derive(Clone, Serialize)]
pub struct LiveEvent {
    pub seq: u64,
    pub task_id: Uuid,
    pub fingerprint: String,
    pub kind: String,
    pub text: String,
}

#[derive(Clone, Serialize)]
pub struct LiveStatus {
    pub task_id: Uuid,
    pub fingerprint: String,
    pub running: bool,
    pub can_resume: bool,
    pub events: Vec<LiveEvent>,
}

struct Session {
    fingerprint: String,
    state: Mutex<SessionState>,
    cancel: AtomicBool,
}

struct SessionState {
    running: bool,
    can_resume: bool,
    seq: u64,
    events: VecDeque<LiveEvent>,
}

static SESSIONS: OnceLock<Mutex<HashMap<Uuid, Arc<Session>>>> = OnceLock::new();
static LIVE_BUSY: AtomicBool = AtomicBool::new(false);
struct LiveGate;
impl Drop for LiveGate {
    fn drop(&mut self) { LIVE_BUSY.store(false, Ordering::Release); }
}
fn sessions() -> &'static Mutex<HashMap<Uuid, Arc<Session>>> {
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn session(id: Uuid) -> Option<Arc<Session>> {
    sessions().lock().unwrap_or_else(|e| e.into_inner()).get(&id).cloned()
}

fn publish(app: &tauri::AppHandle, id: Uuid, session: &Session, kind: &str, text: &str) {
    let event = {
        let mut state = session.state.lock().unwrap_or_else(|e| e.into_inner());
        state.seq += 1;
        let event = LiveEvent {
            seq: state.seq,
            task_id: id,
            fingerprint: session.fingerprint.clone(),
            kind: kind.into(),
            text: text.chars().take(MAX_TEXT).collect(),
        };
        if state.events.len() == MAX_EVENTS { state.events.pop_front(); }
        state.events.push_back(event.clone());
        event
    };
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.emit(EVENT_NAME, event);
    }
}

fn validate_task(db: &Database, root: &Path, id: Uuid) -> Result<InteractiveTask, String> {
    let task = interactive::read_task(root, id)?;
    let dir = root.join(id.to_string());
    let marker = dir.join("live-mode");
    if !task.live || !std::fs::symlink_metadata(marker).is_ok_and(|m| m.is_file() && !m.file_type().is_symlink()) {
        return Err("这不是应用内会话，请在外部终端中继续".into());
    }
    let input = db.get_agent_snapshot(&task.context_fingerprint)?.ok_or("原始快照已不存在")?;
    if task.symbol.is_empty() || task.context_fingerprint.len() != 64 {
        return Err("会话元信息无效".into());
    }
    let input_path = dir.join("input.json");
    if !std::fs::symlink_metadata(&input_path).is_ok_and(|m| m.is_file() && !m.file_type().is_symlink() && m.len() <= super::MAX_INPUT_BYTES as u64) {
        return Err("会话输入文件无效".into());
    }
    let disk_input = std::fs::read_to_string(input_path).map_err(|e| e.to_string())?;
    if input != disk_input { return Err("会话输入与数据库快照不一致".into()); }
    Ok(task)
}

fn tracked(id: Uuid, fingerprint: String, can_resume: bool) -> Arc<Session> {
    let mut all = sessions().lock().unwrap_or_else(|e| e.into_inner());
    all.entry(id).or_insert_with(|| Arc::new(Session {
        fingerprint,
        state: Mutex::new(SessionState { running: false, can_resume, seq: 0, events: VecDeque::new() }),
        cancel: AtomicBool::new(false),
    })).clone()
}

pub fn start(db: &Database, root: &Path, app: tauri::AppHandle, fingerprint: String) -> Result<LiveStatus, String> {
    let agent = status(db);
    let path = agent.path.filter(|_| agent.installed).ok_or(agent.message)?;
    let input = db.get_agent_snapshot(&fingerprint)?.ok_or("分析快照不存在，请重新打开量化分析")?;
    let mut task = interactive::prepare_task(root, &fingerprint, &input)?;
    let dir = root.join(task.id.to_string());
    task.live = true;
    interactive::write_task(&dir, &task)?;
    std::fs::OpenOptions::new().write(true).create_new(true).open(dir.join("live-mode"))
        .map_err(|e| format!("无法标记应用内会话：{e}"))?;
    let initial = "请读取当前目录的 workflow.md、input.json 和 schema.json，说明股票、日期和数据局限，并等待我追问。只有在我明确要求导入时才生成 response.json。".to_string();
    let budget = turn_budget(db);
    let current = tracked(task.id, fingerprint, false);
    run_turn(app, PathBuf::from(path), dir, task.id, current, initial, budget, false)?;
    Ok(snapshot(task.id).expect("刚创建的会话必须存在"))
}

fn turn_budget(db: &Database) -> String {
    db.get_setting("agent_budget_usd").ok().flatten()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v >= 0.05)
        .map(|v| format!("{:.2}", v.min(0.20)))
        .unwrap_or_else(|| "0.20".into())
}

pub fn ask(db: &Database, root: &Path, app: tauri::AppHandle, id: Uuid, question: String) -> Result<LiveStatus, String> {
    let question = question.trim();
    if question.is_empty() || question.chars().count() > 1000 { return Err("问题须为 1–1000 个字符".into()); }
    let task = validate_task(db, root, id)?;
    let current = tracked(id, task.context_fingerprint, task.state == "opened" || task.state == "validated");
    if task.state != "opened" && task.state != "validated" {
        return Err("Claude Code 尚未正常完成会话建立，不能追问".into());
    }
    if !current.state.lock().unwrap_or_else(|e| e.into_inner()).can_resume {
        return Err("会话尚未确认创建，不能追问；请新建会话或使用外部终端".into());
    }
    let agent = status(db);
    let path = agent.path.filter(|_| agent.installed).ok_or(agent.message)?;
    let budget = turn_budget(db);
    run_turn(app, PathBuf::from(path), root.join(id.to_string()), id, current, question.into(), budget, true)?;
    Ok(snapshot(id).expect("会话必须存在"))
}

pub fn snapshot(id: Uuid) -> Option<LiveStatus> {
    session(id).map(|current| {
        let state = current.state.lock().unwrap_or_else(|e| e.into_inner());
        LiveStatus { task_id: id, fingerprint: current.fingerprint.clone(), running: state.running,
            can_resume: state.can_resume, events: state.events.iter().cloned().collect() }
    })
}

pub fn restore(db: &Database, root: &Path, id: Uuid) -> Result<LiveStatus, String> {
    let task = validate_task(db, root, id)?;
    tracked(id, task.context_fingerprint, task.state == "opened" || task.state == "validated");
    snapshot(id).ok_or("会话未找到".into())
}

pub fn cancel(id: Uuid) -> bool {
    if let Some(current) = session(id) {
        if current.state.lock().unwrap_or_else(|e| e.into_inner()).running {
            current.cancel.store(true, Ordering::SeqCst);
            return true;
        }
    }
    false
}

pub fn cancel_all() {
    for current in sessions().lock().unwrap_or_else(|e| e.into_inner()).values() {
        current.cancel.store(true, Ordering::SeqCst);
    }
}

pub fn forget(id: Uuid) {
    sessions().lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
}

fn run_turn(app: tauri::AppHandle, path: PathBuf, dir: PathBuf, id: Uuid, current: Arc<Session>, prompt: String, budget: String, resume: bool) -> Result<(), String> {
    if LIVE_BUSY.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
        return Err("已有应用内会话正在运行，请稍后重试".into());
    }
    let gate = LiveGate;
    {
        let mut state = current.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.running { return Err("该会话正在运行，请等待完成或先中断".into()); }
        state.running = true;
        current.cancel.store(false, Ordering::SeqCst);
    }
    publish(&app, id, &current, "user", if resume { &prompt } else { "开始分析当前冻结快照" });
    std::thread::spawn(move || {
        let _gate = gate;
        let result = execute(&app, &path, &dir, id, &current, &prompt, &budget, resume);
        current.state.lock().unwrap_or_else(|e| e.into_inner()).running = false;
        match result {
            Ok(()) => publish(&app, id, &current, "done", "本轮已结束；文本回复不是可导入的结构化结果"),
            Err(error) => publish(&app, id, &current, "error", &error),
        }
    });
    Ok(())
}

fn drain_stream(reader: impl std::io::Read + Send + 'static, app: tauri::AppHandle, id: Uuid, current: Arc<Session>) -> std::thread::JoinHandle<Result<bool, String>> {
    std::thread::spawn(move || {
        let mut stream = BufReader::new(reader);
        let mut consumed = 0;
        let mut saw_session = false;
        let mut streamed_text = false;
        let mut streamed_tools = false;
        loop {
            let mut line = Vec::new();
            loop {
                let buffer = stream.fill_buf().map_err(|_| "读取 Claude Code 输出失败")?;
                if buffer.is_empty() { break; }
                let n = buffer.iter().position(|b| *b == b'\n').map_or(buffer.len(), |n| n + 1);
                if line.len() + n > MAX_LINE || consumed + n > MAX_STREAM {
                    current.cancel.store(true, Ordering::SeqCst);
                    return Err("Claude Code 事件流超过安全大小限制，已终止本轮".into());
                }
                line.extend_from_slice(&buffer[..n]);
                stream.consume(n);
                consumed += n;
                if line.last() == Some(&b'\n') { break; }
            }
            if line.is_empty() { break; }
            let Ok(value) = serde_json::from_slice::<Value>(&line) else { continue; };
            if value["type"] == "system" && value["subtype"] == "init" {
                if value["session_id"].as_str() == Some(&id.to_string()) {
                    saw_session = true;
                    current.state.lock().unwrap_or_else(|e| e.into_inner()).can_resume = true;
                } else {
                    current.cancel.store(true, Ordering::SeqCst);
                    return Err("Claude Code 会话身份不匹配".into());
                }
            }
            if value["session_id"].as_str().is_some_and(|session| session != id.to_string()) {
                current.cancel.store(true, Ordering::SeqCst);
                return Err("Claude Code 输出包含另一会话事件".into());
            }
            if !saw_session { continue; }
            if value["type"] == "result" && value["is_error"] == true {
                current.cancel.store(true, Ordering::SeqCst);
                return Err("Claude Code 返回错误，请检查登录、额度或权限".into());
            }
            for (kind, text) in map_event(&value, &mut streamed_text, &mut streamed_tools) {
                publish(&app, id, &current, kind, &text);
            }
        }
        Ok(saw_session)
    })
}

fn map_event(value: &Value, streamed_text: &mut bool, streamed_tools: &mut bool) -> Vec<(&'static str, String)> {
    let mut mapped = Vec::new();
    match value["type"].as_str() {
        Some("stream_event") => {
            let event = &value["event"];
            match event["type"].as_str() {
                Some("message_start") => { *streamed_text = false; *streamed_tools = false; }
                Some("content_block_delta") => {
                    if let Some(text) = event["delta"]["text"].as_str().filter(|s| !s.is_empty()) {
                        *streamed_text = true;
                        mapped.push(("text_delta", text.to_string()));
                    }
                }
                Some("content_block_start") => {
                    if let Some(name) = event["content_block"]["name"].as_str() {
                        *streamed_tools = true;
                        mapped.push(("tool", format!("调用工具：{}（参数已隐藏）", safe_tool(name))));
                    }
                }
                _ => {}
            }
        }
        Some("assistant") => {
            if let Some(blocks) = value["message"]["content"].as_array() {
                for block in blocks {
                    if block["type"] == "text" && !*streamed_text {
                        if let Some(text) = block["text"].as_str().filter(|text| !text.is_empty()) {
                            mapped.push(("text", text.to_string()));
                        }
                    } else if block["type"] == "tool_use" && !*streamed_tools {
                        mapped.push(("tool", format!("调用工具：{}（参数已隐藏）", safe_tool(block["name"].as_str().unwrap_or("未知")))));
                    }
                }
            }
            *streamed_text = false;
            *streamed_tools = false;
        }
        Some("user") => {
            if value["message"]["content"].as_array().is_some_and(|blocks| blocks.iter().any(|b| b["type"] == "tool_result")) {
                mapped.push(("tool", "工具已返回（内容已隐藏）".into()));
            }
        }
        Some("result") if value["is_error"] == true => {
            mapped.push(("error", "Claude Code 返回错误，请检查登录、额度或网络".into()));
        }
        _ => {}
    }
    mapped
}

fn safe_tool(name: &str) -> &str {
    match name { "Read" => "Read", "Write" => "Write", "Edit" => "Edit", _ => "其他工具" }
}

fn execute(app: &tauri::AppHandle, path: &Path, dir: &Path, id: Uuid, current: &Arc<Session>, prompt: &str, budget: &str, resume: bool) -> Result<(), String> {
    let prompt_path = dir.join("live-prompt.txt");
    if let Ok(metadata) = std::fs::symlink_metadata(&prompt_path) {
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err("会话提示文件类型无效".into());
        }
        std::fs::remove_file(&prompt_path).map_err(|e| e.to_string())?;
    }
    use std::io::Write;
    std::fs::OpenOptions::new().write(true).create_new(true).open(&prompt_path)
        .and_then(|mut file| file.write_all(prompt.as_bytes()))
        .map_err(|e| format!("准备会话问题失败：{e}"))?;
    let start_path = dir.join(START_FILE);
    if let Ok(metadata) = std::fs::symlink_metadata(&start_path) {
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err("会话启动标志文件类型无效".into());
        }
        std::fs::remove_file(&start_path).map_err(|e| e.to_string())?;
    }
    let use_launcher = cfg!(windows);
    let mut command = Command::new(if use_launcher { app_launcher_path()? } else { path.to_path_buf() });
    command.current_dir(dir).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if use_launcher {
        command.arg(super::LAUNCHER_ARG).arg(path).arg(budget).arg("--live").arg(id.to_string());
        if resume { command.arg("--resume"); }
    } else {
        command.args(["--restricted", "--strict-mcp-config", "--mcp-config", "empty-mcp.json", "--disable-slash-commands", "--no-chrome",
            "--permission-mode", "dontAsk", "--permission-prompts", "none", "--tools", "Read,Write,Edit",
            "--allowedTools", "Read(input.json),Read(schema.json),Read(workflow.md),Write(response.json),Edit(response.json)",
            "--output-format", "stream-json", "--include-partial-messages", "--verbose", "--max-budget-usd", budget]);
        if resume { command.arg("--resume").arg(id.to_string()); }
        else { command.arg("--session-id").arg(id.to_string()); }
        command.arg("-p").arg(prompt);
    }
    minimal_environment(&mut command);
    hide_window(&mut command);
    let mut child = command.spawn().map_err(|e| format!("无法启动 Claude Code：{e}"))?;
    let job = match ProcessJob::assign(&child) {
        Ok(job) => job,
        Err(error) => { let _ = child.kill(); let _ = child.wait(); return Err(format!("无法限制会话进程树：{error}")); }
    };
    let reader = drain_stream(child.stdout.take().ok_or("无法读取事件流")?, app.clone(), id, current.clone());
    let diagnostic = super::collect_diagnostics(child.stderr.take().ok_or("无法读取诊断流")?);
    if use_launcher {
        if let Err(error) = std::fs::OpenOptions::new().write(true).create_new(true).open(dir.join(START_FILE)) {
            job.terminate(); let _ = child.kill(); let _ = child.wait();
            let _ = reader.join(); let _ = diagnostic.join();
            return Err(format!("无法放行会话进程：{error}"));
        }
    }
    let started = Instant::now();
    let completed = loop {
        match child.try_wait() {
            Ok(Some(code)) => break Ok(code),
            Ok(None) => {},
            Err(_) => break Err("读取 Claude Code 进程状态失败".to_string()),
        }
        if current.cancel.load(Ordering::SeqCst) { break Err("本轮会话已由用户中断".into()); }
        if started.elapsed() > TURN_TIMEOUT { break Err("本轮会话超过 300 秒，已中断".into()); }
        std::thread::sleep(Duration::from_millis(50));
    };
    job.terminate();
    let _ = child.kill(); let _ = child.wait();
    let stream_result = reader.join().map_err(|_| "会话事件读取任务异常")?;
    let _ = diagnostic.join();
    let _ = std::fs::remove_file(dir.join(START_FILE));
    let _ = std::fs::remove_file(dir.join("live-prompt.txt"));
    let saw_session = stream_result?;
    completed.and_then(|code| {
        if !code.success() { return Err(format!("Claude Code 已退出（代码：{code}）；请检查登录、额度和权限")); }
        if !saw_session { return Err("Claude Code 未确认会话身份，不能恢复".into()); }
        let mut task = interactive::read_task(&dir.parent().ok_or("会话目录无效")?, id)?;
        if task.state == "prepared" {
            task.state = "opened".into();
            interactive::write_task(dir, &task)?;
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_real_messages_are_visible() {
        let mut streamed_text = false;
        let mut streamed_tools = false;
        let init = serde_json::json!({"type":"system","subtype":"init","session_id":"test"});
        assert!(map_event(&init, &mut streamed_text, &mut streamed_tools).is_empty());
        let event = serde_json::json!({"type":"stream_event","event":{"type":"content_block_delta","delta":{"text":"你好"}}});
        assert_eq!(map_event(&event, &mut streamed_text, &mut streamed_tools)[0].1, "你好");
        let full = serde_json::json!({"type":"assistant","message":{"content":[{"type":"text","text":"你好"}]}});
        assert!(map_event(&full, &mut streamed_text, &mut streamed_tools).is_empty());
        let tool = serde_json::json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"secret":"do not expose"}}]}});
        assert_eq!(map_event(&tool, &mut streamed_text, &mut streamed_tools)[0].1, "调用工具：其他工具（参数已隐藏）");
    }
    #[test]
    fn chunked_stream_and_tool_output_are_bounded() {
        let mut text = false;
        let mut tools = false;
        let start = serde_json::json!({"type":"stream_event","event":{"type":"message_start"}});
        assert!(map_event(&start, &mut text, &mut tools).is_empty());
        let call = serde_json::json!({"type":"stream_event","event":{"type":"content_block_start","content_block":{"type":"tool_use","name":"Read","input":{"path":"secret"}}}});
        assert_eq!(map_event(&call, &mut text, &mut tools)[0].1, "调用工具：Read（参数已隐藏）");
        let delta = serde_json::json!({"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"数据有限"}}});
        assert_eq!(map_event(&delta, &mut text, &mut tools)[0].1, "数据有限");
        let final_message = serde_json::json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"path":"secret"}},{"type":"text","text":"数据有限"}]}});
        assert!(map_event(&final_message, &mut text, &mut tools).is_empty());
        let response = serde_json::json!({"type":"user","message":{"content":[{"type":"tool_result","content":"secret content"}]}});
        assert_eq!(map_event(&response, &mut text, &mut tools)[0].1, "工具已返回（内容已隐藏）");
    }

    #[test]
    fn budget_is_capped() {
        let dir = std::env::temp_dir().join(format!("bull-live-test-{}", Uuid::new_v4()));
        let db = Database::open(dir.clone()).unwrap();
        db.set_setting("agent_budget_usd", "9.99").unwrap();
        assert_eq!(turn_budget(&db), "0.20");
        drop(db);
        let _ = std::fs::remove_dir_all(dir);
    }
}
