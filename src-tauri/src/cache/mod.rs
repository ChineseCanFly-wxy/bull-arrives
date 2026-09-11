use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use tauri::{Emitter, Manager};
use tokio::sync::Notify;
use crate::domain::{Quote, IndexQuote};
use crate::datasource::market_clock::MarketSession;

#[derive(Default)]
struct AlertScope {
    initialized: bool,
    codes: HashSet<String>,
    pending_rearm: HashSet<String>,
}

/// Quote cache (in-memory + SQLite dual-write)
pub struct QuoteCache {
    quotes: Mutex<HashMap<String, Quote>>,
    indices: Mutex<Vec<IndexQuote>>,
    alert_scope: Mutex<AlertScope>,
    db: Arc<crate::db::Database>,
}

impl QuoteCache {
    pub fn new(db: Arc<crate::db::Database>) -> Self {
        Self {
            quotes: Mutex::new(HashMap::new()),
            indices: Mutex::new(Vec::new()),
            alert_scope: Mutex::new(AlertScope::default()),
            db,
        }
    }

    /// Restore cache from SQLite (called on startup). Restored rows are display
    /// data only and intentionally never enter the alert evaluator.
    pub fn restore_from_db(&self) {
        if let Ok(cached) = self.db.get_cached_quotes() {
            let mut quotes = self.quotes.lock().unwrap_or_else(|e| e.into_inner());
            for q in cached {
                let key = format!("{}:{}", q.market, q.code);
                quotes.insert(key, q);
            }
        }
        if let Ok(cached) = self.db.get_cached_indices() {
            *self.indices.lock().unwrap_or_else(|e| e.into_inner()) = cached;
        }
    }

    /// Evaluate datasource-fresh quotes only. A code newly entering the current
    /// group is observed once to rearm crossing state, never compared with cache.
    fn evaluate_alerts_at(
        &self,
        quotes: &[Quote],
        rearm_codes: &HashSet<String>,
        now: chrono::DateTime<chrono::FixedOffset>,
        app_handle: &tauri::AppHandle,
    ) {
        crate::alerts::evaluate_fresh_quotes(
            &self.db,
            quotes,
            now,
            rearm_codes,
            |event| {
                let title = format!("行情提醒 · {} {}", event.name, event.code);
                let body = if event.alert_type == "change_pct" {
                    format!("较昨收 {:+.2}%，达到 {:+.2}% 提醒条件；现价 {:.4}", event.value, event.threshold, event.current_price)
                } else {
                    format!("{}穿越目标价 {:.4}；现价 {:.4}", if event.direction == "up" { "向上" } else { "向下" }, event.threshold, event.current_price)
                };
                match serde_json::to_value(event) {
                    Ok(mut payload) => {
                        payload["title"] = serde_json::Value::String(title);
                        payload["body"] = serde_json::Value::String(body);
                        crate::notifications::publish(app_handle, payload);
                    }
                    Err(error) => log::warn!("提醒事件序列化失败: {}", error),
                }
            },
        );
    }

    fn sync_alert_scope(&self, codes: &[(String, String)]) {
        let current: HashSet<String> = codes
            .iter()
            .map(|(code, market)| format!("{}:{}", market, code))
            .collect();
        let mut scope = self.alert_scope.lock().unwrap_or_else(|e| e.into_inner());
        let newly_visible: HashSet<String> = if scope.initialized {
            current.difference(&scope.codes).cloned().collect()
        } else {
            // Startup must establish live state before any crossing can trigger.
            current.clone()
        };
        scope.pending_rearm.extend(newly_visible);
        scope.pending_rearm.retain(|key| current.contains(key));
        scope.codes = current;
        scope.initialized = true;
    }

    fn pending_rearm_codes(&self) -> HashSet<String> {
        self.alert_scope
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pending_rearm
            .clone()
    }

    fn finish_rearm(&self, quotes: &[Quote], now: chrono::DateTime<chrono::FixedOffset>) {
        let observed: HashSet<String> = quotes
            .iter()
            .filter(|quote| crate::alerts::is_fresh_trading_quote(quote, now))
            .map(|quote| format!("{}:{}", quote.market, quote.code))
            .collect();
        self.alert_scope
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pending_rearm
            .retain(|key| !observed.contains(key));
    }

    /// Update in-memory cache only (fast, no I/O)
    pub fn update_quotes_memory(&self, quotes: &[Quote]) {
        let mut cache = self.quotes.lock().unwrap_or_else(|e| e.into_inner());
        for q in quotes {
            let key = format!("{}:{}", q.market, q.code);
            cache.insert(key, q.clone());
        }
    }

    /// Persist quotes to SQLite (call via spawn_blocking to avoid blocking tokio)
    pub fn persist_quotes(&self, quotes: &[Quote]) {
        if let Err(e) = self.db.cache_quotes(quotes) {
            log::warn!("Failed to persist quotes to DB: {}", e);
        }
    }

    /// Update cache with fresh quotes (combines memory update + best-effort DB write)
    /// Prefer update_quotes_memory + spawn_blocking persist_quotes in async contexts.
    pub fn update_quotes(&self, quotes: &[Quote]) {
        self.update_quotes_memory(quotes);
        // Best-effort sync write — use update_quotes_memory + spawn_blocking persist_quotes
        // in async contexts to avoid blocking tokio worker threads.
        self.persist_quotes(quotes);
    }

    /// Get all cached quotes
    pub fn get_all_quotes(&self) -> Vec<Quote> {
        let cache = self.quotes.lock().unwrap_or_else(|e| e.into_inner());
        cache.values().cloned().collect()
    }

    fn quotes_for_codes(&self, codes: &[(String, String)]) -> Vec<Quote> {
        let cache = self.quotes.lock().unwrap_or_else(|e| e.into_inner());
        codes
            .iter()
            .filter_map(|(code, market)| {
                cache.get(&format!("{}:{}", market, code)).cloned()
            })
            .collect()
    }

    /// Update indices in memory.
    pub fn update_indices(&self, indices: Vec<IndexQuote>) {
        *self.indices.lock().unwrap_or_else(|e| e.into_inner()) = indices;
    }

    pub fn persist_indices(&self, indices: &[IndexQuote]) {
        if let Err(error) = self.db.cache_indices(indices) {
            log::warn!("Failed to persist indices to DB: {}", error);
        }
    }

    /// Get cached indices
    pub fn get_indices(&self) -> Vec<IndexQuote> {
        self.indices.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Snapshot current prices (code→price) for change detection
    pub fn get_price_snapshot(&self) -> HashMap<String, f64> {
        let cache = self.quotes.lock().unwrap_or_else(|e| e.into_inner());
        cache
            .iter()
            .map(|(k, v)| (k.clone(), v.price))
            .collect()
    }
}

// ── Adaptive polling constants ──

/// Number of probes at session start to detect if market is actually open
const PROBE_COUNT: u32 = 3;
/// Interval used during probing phase (seconds)
const PROBE_INTERVAL: u64 = 2;
/// Number of consecutive unchanged polls before switching to idle
const STREAK_THRESHOLD: u32 = 10;
/// Idle polling interval when market is detected as closed (seconds)
const IDLE_INTERVAL: u64 = 30;

/// Adaptive polling state machine — detects holidays via price stasis
#[derive(Debug, Clone, Copy, PartialEq)]
enum PollingState {
    /// Probing at session start to determine if market is actually open
    Probing { remaining: u32 },
    /// Market is open, normal frequency. Tracks consecutive unchanged polls.
    Normal { unchanged_streak: u32 },
    /// Market detected as closed (holiday), throttled to idle frequency.
    Idle,
}

impl PollingState {
    fn new() -> Self {
        Self::Probing { remaining: PROBE_COUNT }
    }

    /// Reset to probing when entering a trading session
    fn on_session_enter(&mut self) {
        *self = Self::Probing { remaining: PROBE_COUNT };
    }

    /// Update state based on fetch result. Returns the interval for the next cycle.
    fn update(&mut self, prices_changed: bool, session: MarketSession) -> u64 {
        match self {
            Self::Probing { remaining } => {
                if prices_changed {
                    log::info!("Probe detected price change — market is open");
                    *self = Self::Normal { unchanged_streak: 0 };
                    return session.recommended_interval();
                }
                *remaining -= 1;
                if *remaining == 0 {
                    log::info!("All probes returned no price change — switching to idle (holiday/closure)");
                    *self = Self::Idle;
                    return IDLE_INTERVAL;
                }
                PROBE_INTERVAL
            }
            Self::Normal { unchanged_streak } => {
                if prices_changed {
                    *unchanged_streak = 0;
                } else {
                    *unchanged_streak += 1;
                    if *unchanged_streak >= STREAK_THRESHOLD {
                        log::info!(
                            "{} consecutive polls with no price change — switching to idle",
                            *unchanged_streak
                        );
                        *self = Self::Idle;
                        return IDLE_INTERVAL;
                    }
                }
                session.recommended_interval()
            }
            Self::Idle => {
                if prices_changed {
                    log::info!("Price change detected in idle mode — resuming normal polling");
                    *self = Self::Normal { unchanged_streak: 0 };
                    return session.recommended_interval();
                }
                IDLE_INTERVAL
            }
        }
    }
}

/// Outcome of a fetch cycle — indicates whether quote prices changed vs the cache
struct FetchOutcome {
    prices_changed: bool,
}

/// Runtime-mutable polling configuration, shared between the settings command
/// (writer) and the scheduler loop (reader).
///
/// `interval_secs == 0` means AUTO: the scheduler follows the trading
/// session's recommended interval.  Any other value pins the poll interval.
/// `changed` wakes the scheduler so a new interval takes effect immediately
/// instead of only after the currently-sleeping tick elapses.
pub struct PollingConfig {
    interval_secs: AtomicU64,
    changed: Notify,
}

impl PollingConfig {
    pub fn new(interval_secs: u64) -> Self {
        Self {
            interval_secs: AtomicU64::new(interval_secs),
            changed: Notify::new(),
        }
    }

    pub fn interval_secs(&self) -> u64 {
        self.interval_secs.load(Ordering::Relaxed)
    }

    /// Update the configured interval and wake the scheduler immediately.
    pub fn set_interval_secs(&self, secs: u64) {
        self.interval_secs.store(secs, Ordering::Relaxed);
        self.changed.notify_one();
    }

    /// Resolve the effective interval for a session, honouring auto mode.
    fn resolve(&self, session: MarketSession) -> u64 {
        let configured = self.interval_secs();
        if configured == 0 {
            session.recommended_interval()
        } else {
            configured
        }
    }
}

/// Background polling scheduler
pub struct Scheduler;

impl Scheduler {
    /// Spawn the global polling loop in a background tokio task.
    pub fn spawn(
        data_manager: Arc<crate::datasource::DataSourceManager>,
        cache: Arc<QuoteCache>,
        db: Arc<crate::db::Database>,
        app_handle: tauri::AppHandle,
        config: Arc<PollingConfig>,
    ) {
        tauri::async_runtime::spawn(async move {
            let mut last_session = MarketSession::current();
            let mut state = PollingState::new();

            // Fetch guard — prevents concurrent fetch_once calls from the two loops
            let fetching = Arc::new(AtomicBool::new(false));

            loop {
                // ── Session transition handling ──
                let session = MarketSession::current();
                if session != last_session {
                    let is_trading_enter = matches!(
                        (last_session, session),
                        (MarketSession::PreOpen, MarketSession::MorningTrade)
                            | (MarketSession::LunchBreak, MarketSession::AfternoonTrade)
                    );

                    if is_trading_enter {
                        state.on_session_enter();
                        log::info!(
                            "Entering trading session ({:?}), starting probe",
                            session
                        );
                    }

                    last_session = session;
                    if let Err(e) = app_handle.emit("market-session-changed", serde_json::json!({
                        "session": session.name(),
                        "interval_secs": config.resolve(session),
                        "is_trading": matches!(
                            session,
                            MarketSession::MorningTrade | MarketSession::AfternoonTrade
                        ),
                    })) {
                        log::warn!("Failed to emit market-session-changed: {}", e);
                    }
                }

                // ── Fetch data ──
                let outcome = Self::fetch_once(&data_manager, &cache, &db, &app_handle, &fetching, false).await;

                // ── Adaptive interval ──
                // Adaptive polling (probe → normal → idle) is only used during
                // MorningTrade and AfternoonTrade sessions. During PreOpen,
                // LunchBreak, and Closed, prices are expected to be static, so
                // we skip the state machine and use the fixed recommended interval.
                // A user-pinned interval (non-zero) always wins over both.
                let configured = config.interval_secs();
                let in_trading = matches!(session, MarketSession::MorningTrade | MarketSession::AfternoonTrade);
                let new_interval = if configured != 0 {
                    configured
                } else {
                    match (outcome, in_trading) {
                        (Some(o), true) => state.update(o.prices_changed, session),
                        _ => session.recommended_interval(),
                    }
                };

                // Use sleep instead of interval to avoid the "immediate first tick"
                // problem. tokio::time::interval fires immediately when created,
                // which causes a burst of polls on every state transition —
                // creating an oscillation between idle and normal modes.
                // sleep always waits the full duration before resuming.
                // `changed` cuts the wait short when the user edits the interval.
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(new_interval)) => {}
                    _ = data_manager.wakeup.notified() => { state.on_session_enter(); }
                    _ = config.changed.notified() => {
                        log::info!(
                            "[scheduler] interval changed to {:?} — refetching now",
                            if config.interval_secs() == 0 { "auto".to_string() } else { format!("{}s", config.interval_secs()) }
                        );
                    }
                }
            }
        });
    }

    /// Run one fetch cycle. Returns `FetchOutcome` with price-change info,
    /// or `None` if no quote data was fetched (empty watchlist, error, closed market).
    async fn fetch_once(
        manager: &crate::datasource::DataSourceManager,
        cache: &Arc<QuoteCache>,
        db: &std::sync::Arc<crate::db::Database>,
        app_handle: &tauri::AppHandle,
        fetching: &AtomicBool,
        _force: bool,
    ) -> Option<FetchOutcome> {
        // Skip if a fetch is already in progress (prevents duplicate API calls)
        if fetching.swap(true, Ordering::AcqRel) {
            return None;
        }
        let _guard = FetchGuard(fetching);

        let revision = manager.revision();

        // 1. Get watchlist codes
        let db_for_codes = db.clone();
        let codes = match tokio::task::spawn_blocking(move || db_for_codes.get_current_watch_codes()).await
        {
            Ok(Ok(c)) => c,
            Ok(Err(e)) => {
                log::warn!("Failed to read watchlist from DB: {}", e);
                Self::fetch_and_emit_indices(manager, cache, app_handle).await;
                return None;
            }
            Err(join_err) => {
                log::warn!("spawn_blocking join error for get_watch_codes: {}", join_err);
                Self::fetch_and_emit_indices(manager, cache, app_handle).await;
                return None;
            }
        };

        // Track membership independently from successful fetches. Leaving and later
        // re-entering a group therefore forces one fresh quote to rearm the rule.
        cache.sync_alert_scope(&codes);

        // Respect the central request policy before every network path. When
        // blocked, only current-group cache is emitted and alerts are not evaluated.
        if let Err(reason) = manager.ensure_request_allowed() {
            log::debug!("Quote refresh blocked by request policy: {}", reason);
            let cached = cache.quotes_for_codes(&codes);
            if !cached.is_empty() {
                if let Err(error) = app_handle.emit("quotes-updated", &cached) {
                    log::warn!("Failed to emit policy-blocked cached quotes: {}", error);
                }
            }
            Self::fetch_and_emit_indices(manager, cache, app_handle).await;
            return None;
        }
        if codes.is_empty() {
            Self::fetch_and_emit_indices(manager, cache, app_handle).await;
            return None;
        }

        // 2. Group by market
        let mut cn_codes: Vec<String> = Vec::new();
        for (code, market) in &codes {
            if market == "CN" {
                cn_codes.push(code.clone());
            }
        }

        if manager.revision() != revision { return None; }

        if !cn_codes.is_empty() {
            if let Some(source) = manager.active_source() {
                // Snapshot prices before fetch for change detection
                let prices_before = cache.get_price_snapshot();

                if manager.ensure_request_allowed().is_err() { return None; }
                let response = source.fetch_realtime(&cn_codes, "CN").await;
                if manager.revision() != revision || manager.ensure_request_allowed().is_err() {
                    manager.wakeup.notify_one();
                    return None;
                }
                match response {
                    Ok(quotes) => {
                        cache.update_quotes_memory(&quotes);
                        let rearm_codes = cache.pending_rearm_codes();
                        let alert_now = crate::alerts::china_now();
                        cache.evaluate_alerts_at(&quotes, &rearm_codes, alert_now, app_handle);
                        cache.finish_rearm(&quotes, alert_now);
                        if let Err(e) = app_handle.emit("quotes-updated", &quotes) {
                            log::warn!("Failed to emit quotes-updated: {}", e);
                        }
                        let cache_for_persist = cache.clone();
                        let quotes_for_db = quotes.to_vec();
                        if let Err(error) = tokio::task::spawn_blocking(move || {
                            cache_for_persist.persist_quotes(&quotes_for_db);
                        }).await {
                            log::warn!("行情缓存落盘任务失败: {}", error);
                        }

                        // Compare: did any price actually change?
                        let changed = Self::any_price_changed(&prices_before, &quotes);

                        Self::fetch_and_emit_indices(manager, cache, app_handle).await;
                        return Some(FetchOutcome { prices_changed: changed });
                    }
                    Err(e) => {
                        log::warn!("Quote fetch failed (will retry): {}", e);
                        let cached = cache.quotes_for_codes(&codes);
                        if !cached.is_empty() {
                            if let Err(e) = app_handle.emit("quotes-updated", &cached) {
                                log::warn!("Failed to emit quotes-updated (fallback): {}", e);
                            }
                        }
                        // Fetch failed — we can't determine price change, so return None
                        // to keep the state machine from making a false decision.
                        Self::fetch_and_emit_indices(manager, cache, app_handle).await;
                        return None;
                    }
                }
            }
        }

        Self::fetch_and_emit_indices(manager, cache, app_handle).await;
        None
    }

    /// Check whether any quote price differs from the snapshot.
    /// Uses a small epsilon (0.001) instead of f64::EPSILON because the stock
    /// API may return values with slightly different floating-point representation
    /// across backend servers (e.g. 3250.68 vs 3250.680000000001).
    const PRICE_CHANGE_EPSILON: f64 = 0.001;

    fn any_price_changed(snapshot: &std::collections::HashMap<String, f64>, quotes: &[crate::domain::Quote]) -> bool {
        if snapshot.is_empty() {
            return true;
        }
        for q in quotes {
            let key = format!("{}:{}", q.market, q.code);
            match snapshot.get(&key) {
                Some(&prev_price) if (prev_price - q.price).abs() > Self::PRICE_CHANGE_EPSILON => return true,
                Some(_) => {} // same price (within tolerance)
                None => return true, // new stock added
            }
        }
        false
    }

    async fn fetch_and_emit_indices(
        manager: &crate::datasource::DataSourceManager,
        cache: &Arc<QuoteCache>,
        app_handle: &tauri::AppHandle,
    ) {
        if !app_handle.get_webview_window("main").and_then(|w| w.is_visible().ok()).unwrap_or(false) {
            return;
        }
        if let Err(reason) = manager.ensure_request_allowed() {
            log::debug!("Index refresh blocked by request policy: {}", reason);
            let cached = cache.get_indices();
            if !cached.is_empty() {
                if let Err(error) = app_handle.emit("indices-updated", &cached) {
                    log::warn!("Failed to emit policy-blocked cached indices: {}", error);
                }
            }
            return;
        }
        if let Some(source) = manager.active_source() {
            let revision = manager.revision();
            match source.fetch_indices().await {
                Ok(fresh) => {
                    if manager.revision() != revision || manager.ensure_request_allowed().is_err() {
                        manager.wakeup.notify_one();
                        return;
                    }
                    let prev = cache.get_indices();
                    let changed = prev.len() != fresh.len()
                        || !fresh.iter().zip(&prev).all(|(n, p)| {
                            n.code == p.code
                                && n.price == p.price
                                && n.change == p.change
                                && n.change_pct == p.change_pct
                        });
                    cache.update_indices(fresh);
                    let current = cache.get_indices();
                    let cache_for_persist = cache.clone();
                    let indices_for_db = current.clone();
                    if let Err(error) = tokio::task::spawn_blocking(move || {
                        cache_for_persist.persist_indices(&indices_for_db);
                    }).await {
                        log::warn!("指数缓存落盘任务失败: {}", error);
                    }
                    if changed {
                        if let Err(e) = app_handle.emit("indices-updated", &current) {
                            log::warn!("Failed to emit indices-updated: {}", e);
                        }
                    } else {
                        log::debug!("Indices unchanged, skipping emit");
                    }
                }
                Err(e) => log::warn!("Index fetch failed: {}", e),
            }
        }
    }
}

/// RAII guard that clears the fetch-in-progress flag on drop.
struct FetchGuard<'a>(&'a AtomicBool);

impl<'a> Drop for FetchGuard<'a> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
