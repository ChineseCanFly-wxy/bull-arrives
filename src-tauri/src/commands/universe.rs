//! 全市场股票池相关命令（漏斗 L0 获取 + L1 筛选）
//!
//! 对应前端「全市场筛选器」。业务上属于漏斗前两层：
//! 一次拉取全市场快照，然后在内存里按用户配置的条件过滤 —— **筛选本身零网络请求**。

use crate::datasource::eastmoney_universe::{
    self, count_by_board, preferred_source, preset_by_id, preset_infos, Board, FilterCapabilities,
    FilterPreset, MarketFilter, PresetInfo, SnapshotRow, SnapshotSource, DEFAULT_SNAPSHOT_TTL,
};
use crate::datasource::market_clock::MarketSession;
use crate::datasource::market_policy::MarketRequestPolicy;
use crate::db::Database;
use crate::domain::KLineData;
use crate::dynamic_filter;
use crate::quant::playbook::TradeRule;
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;
use tauri::State;
use tokio::{sync::Semaphore, task::JoinSet};

/// 板块分布项（供设置页/自检展示）
#[derive(Debug, Serialize)]
pub struct BoardCount {
    pub board: Board,
    /// 中文名，前端直接显示，不必再维护一份映射
    pub label: String,
    pub count: usize,
}

/// 全市场股票池响应
#[derive(Debug, Serialize)]
pub struct UniverseResponse {
    /// 全市场总数（未筛选）
    pub total_all: usize,
    /// 命中筛选条件的数量
    pub total_matched: usize,
    /// 本次实际返回的行数（受 limit 限制）
    pub returned: usize,
    /// 数据是否陈旧：本次刷新失败（很可能被限流），返回的是上次的旧数据
    pub stale: bool,
    /// 本次数据来自哪个通道（sina / eastmoney）
    pub source: SnapshotSource,
    /// 通道中文名，前端直接显示
    pub source_label: String,
    /// 当前通道是否提供「量比」。为 false 时前端应提示该条件当前不可用，
    /// 否则用户设了「量比 ≥ x」会得到 0 结果而不知道为什么。
    pub volume_ratio_supported: bool,
    /// 当前通道是否提供「60 日涨跌幅」。与量比同理 ——
    /// 依赖它的趋势 / 反转类策略在新浪通道下会被跳过，必须让用户看得见。
    pub change_60d_supported: bool,
    /// 当前通道是否提供上市日期
    pub listing_date_supported: bool,
    /// 因数据源不支持而被自动忽略的条件名（如 ["量比"]），供前端明确提示
    pub skipped_conditions: Vec<String>,
    /// 历史技术筛选的执行/降级说明
    pub history_notice: Option<String>,
    /// 实际完成本地历史计算的候选数
    pub history_evaluated: usize,
    /// 全市场板块分布
    pub board_counts: Vec<BoardCount>,
    /// 命中筛选的明细
    pub rows: Vec<SnapshotRow>,
}

#[derive(Debug, Serialize)]
pub struct DynamicFilterProposal {
    pub mode: String,
    pub state: String,
    pub auto_eligible: bool,
    pub observation_days: usize,
    pub source: SnapshotSource,
    pub as_of: String,
    pub date_basis: String,
    pub filter: MarketFilter,
    pub candidate_limit: usize,
    pub preview_count: usize,
    pub preview_codes: Vec<String>,
    pub clamped_fields: Vec<String>,
    pub rationale: String,
    pub guidance: String,
}

/// 生成隔离的动态筛选建议。首版只读、不写 universe_filter；缺少可审计的
/// 28 天配对样本前，confirm/auto 故意不可用。
#[tauri::command]
pub async fn generate_dynamic_filter_proposal(
    db: State<'_, Arc<Database>>,
) -> Result<DynamicFilterProposal, String> {
    let session = MarketSession::current();
    if !matches!(session, MarketSession::PreOpen | MarketSession::Closed) {
        return Err("动态筛选只在盘前或收盘后生成，盘中参数保持冻结".into());
    }
    let schedule = db
        .get_setting("quote_schedule")
        .map_err(|error| error.to_string())?;
    let policy = MarketRequestPolicy::from_quote_schedule_json(schedule.as_deref())?;
    if !policy.is_trading_day_at(chrono::Utc::now()) {
        return Err("当前为周末或已配置休市日，不生成动态筛选建议".into());
    }
    let configured = db
        .get_setting("universe_source")
        .ok()
        .flatten()
        .filter(|value| !value.trim().is_empty());
    let outcome = eastmoney_universe::market_snapshot_with_fallback(
        DEFAULT_SNAPSHOT_TTL,
        preferred_source(configured.as_deref()),
    )
    .await
    .map_err(|error| format!("获取动态筛选快照失败：{error}"))?;
    if outcome.stale {
        return Err("全市场快照为陈旧兜底数据，拒绝生成可执行参数".into());
    }
    let fixed = match db.get_setting("universe_filter").ok().flatten() {
        Some(value) if !value.trim().is_empty() => serde_json::from_str(&value)
            .map_err(|error| format!("当前固定筛选条件损坏：{error}"))?,
        _ => MarketFilter::default(),
    };
    let offset = chrono::FixedOffset::east_opt(8 * 3600).expect("UTC+8 is valid");
    let as_of = chrono::Utc::now()
        .with_timezone(&offset)
        .format("%Y-%m-%d")
        .to_string();
    let context = serde_json::json!({
        "as_of": as_of,
        "date_basis": "local_session_gate_unverified",
        "source": outcome.source,
        "fixed_filter": &fixed,
        "market": dynamic_filter::market_context(&outcome.rows),
    });
    let raw = crate::agent::propose_dynamic_filter(&db, context).await?;
    let rationale = raw.rationale.clone();
    let safe = dynamic_filter::sanitize(&fixed, &raw);
    let capabilities = FilterCapabilities::for_source(outcome.source);
    let mut candidates = safe.filter.apply_with(&outcome.rows, capabilities);
    candidates.sort_by(|a, b| {
        b.amount
            .partial_cmp(&a.amount)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.code.cmp(&b.code))
    });
    candidates.truncate(safe.candidate_limit);
    Ok(DynamicFilterProposal {
        mode: "advice".into(),
        state: "observing".into(),
        auto_eligible: false,
        observation_days: 0,
        source: outcome.source,
        as_of,
        date_basis: "local_session_gate_unverified".into(),
        filter: safe.filter,
        candidate_limit: safe.candidate_limit,
        preview_count: candidates.len(),
        preview_codes: candidates.into_iter().map(|row| row.code).collect(),
        clamped_fields: safe.clamped_fields,
        rationale,
        guidance: "仅建议：未写入真实筛选设置。需累计至少 28 天且 20 个闭合配对样本后，才能开放需确认或自动模式。".into(),
    })
}

/// 返回行数默认上限。
/// 全市场 5900 行一次 IPC 传过去约 3MB，会明显拖慢前端渲染，
/// 所以默认截断；调用方可按需提高。
const DEFAULT_ROW_LIMIT: usize = 500;

/// 服务端分页的默认每页条数（与前端默认值一致）
const DEFAULT_PAGE_SIZE: u32 = 20;

/// 服务端分页的每页上限（与前端可选的最大值一致）
const MAX_PAGE_SIZE: u32 = 100;

/// 快照粗筛后最多读取这些候选的本地历史。在线请求始终只有全市场快照，
/// 不会随股票总数退化成逐只联网。
const HISTORY_CANDIDATE_LIMIT: usize = 120;
const HISTORY_CONCURRENCY: usize = 8;

fn history_bars_needed(filter: &MarketFilter) -> usize {
    [
        filter.above_ma_days.unwrap_or(0),
        filter.new_high_days.unwrap_or(0),
        filter.rise_from_low_days.unwrap_or(0),
        if filter.macd_bullish { 40 } else { 0 },
        if filter.kdj_bullish { 9 } else { 0 },
        if filter.volume_price_rising { 6 } else { 0 },
    ]
    .into_iter()
    .max()
    .unwrap_or(0)
    .clamp(2, 250) as usize
}

fn accepts_history(filter: &MarketFilter, rows: &[KLineData]) -> bool {
    let closes: Vec<f64> = rows.iter().map(|row| row.close).collect();
    let highs: Vec<f64> = rows.iter().map(|row| row.high).collect();
    let lows: Vec<f64> = rows.iter().map(|row| row.low).collect();
    let Some(&latest_close) = closes.last() else {
        return false;
    };

    if let Some(days) = filter.above_ma_days {
        let days = (days as usize).clamp(2, 250);
        let ma = crate::quant::indicators::sma(&closes, days);
        if !ma
            .last()
            .is_some_and(|value| value.is_finite() && latest_close >= *value)
        {
            return false;
        }
    }
    if let Some(days) = filter.new_high_days {
        let days = (days as usize).clamp(2, 250);
        if highs.len() < days
            || highs[highs.len() - days..]
                .iter()
                .copied()
                .fold(f64::NEG_INFINITY, f64::max)
                > *highs.last().unwrap_or(&f64::NEG_INFINITY)
        {
            return false;
        }
    }
    if filter.macd_bullish {
        let macd = crate::quant::indicators::macd(&closes, 12, 26, 9);
        let Some((&dif, &dea, &hist)) = macd
            .dif
            .last()
            .zip(macd.dea.last())
            .zip(macd.hist.last())
            .map(|((dif, dea), hist)| (dif, dea, hist))
        else {
            return false;
        };
        if !dif.is_finite() || !dea.is_finite() || dif <= dea || hist <= 0.0 {
            return false;
        }
    }
    if filter.kdj_bullish {
        let kdj = crate::quant::indicators::kdj(&highs, &lows, &closes, 9);
        if !kdj
            .k
            .last()
            .zip(kdj.d.last())
            .is_some_and(|(k, d)| k.is_finite() && d.is_finite() && k > d)
        {
            return false;
        }
    }
    if filter.volume_price_rising {
        if rows.len() < 6 {
            return false;
        }
        let latest = &rows[rows.len() - 1];
        let previous = &rows[rows.len() - 2];
        let average = rows[rows.len() - 6..rows.len() - 1]
            .iter()
            .map(|row| row.volume as f64)
            .sum::<f64>()
            / 5.0;
        if latest.close <= previous.close || latest.volume as f64 <= average {
            return false;
        }
    }
    if let Some(days) = filter.rise_from_low_days {
        let days = (days as usize).clamp(2, 250);
        if lows.len() < days {
            return false;
        }
        let low = lows[lows.len() - days..]
            .iter()
            .copied()
            .fold(f64::INFINITY, f64::min);
        let rise = (latest_close / low - 1.0) * 100.0;
        if !MarketFilter::within(rise, filter.rise_from_low_min, filter.rise_from_low_max) {
            return false;
        }
    }
    true
}

async fn apply_history_filter(
    db: Arc<Database>,
    rows: Vec<SnapshotRow>,
    filter: &MarketFilter,
) -> (Vec<SnapshotRow>, usize, Option<String>) {
    if !filter.has_history_conditions() || rows.is_empty() {
        return (rows, 0, None);
    }
    if let Err(error) = crate::datasource::kline::probe_local_history(&db).await {
        return (
            rows,
            0,
            Some(format!(
                "本地历史不可用，历史技术条件已跳过（未逐只回退在线）：{error}"
            )),
        );
    }
    let coarse_count = rows.len();
    let candidates: Vec<_> = rows.iter().take(HISTORY_CANDIDATE_LIMIT).cloned().collect();
    let semaphore = Arc::new(Semaphore::new(HISTORY_CONCURRENCY));
    let filter = Arc::new(filter.clone());
    let bars = history_bars_needed(&filter);
    let mut tasks = JoinSet::new();
    for row in candidates {
        let db = db.clone();
        let semaphore = semaphore.clone();
        let filter = filter.clone();
        tasks.spawn(async move {
            let _permit = semaphore
                .acquire_owned()
                .await
                .expect("semaphore not closed");
            let symbol = crate::datasource::kline::full_symbol_from(&row.code, row.board);
            let result = crate::datasource::kline::fetch_history(&db, &symbol, Some(bars), false)
                .await
                .map(|history| accepts_history(&filter, &history.klines));
            (row, result)
        });
    }

    let mut passed = Vec::new();
    let mut evaluated = 0usize;
    let mut failed = 0usize;
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok((row, Ok(true))) => {
                evaluated += 1;
                passed.push(row);
            }
            Ok((_, Ok(false))) => evaluated += 1,
            Ok((_, Err(_))) | Err(_) => failed += 1,
        }
    }

    if evaluated == 0 {
        return (
            rows,
            0,
            Some("本地历史不可用，历史技术条件已跳过（未逐只回退在线）".into()),
        );
    }
    let mut notes = vec![format!("本地历史已计算 {evaluated} 只")];
    if coarse_count > HISTORY_CANDIDATE_LIMIT {
        notes.push(format!("仅检查成交额前 {HISTORY_CANDIDATE_LIMIT} 只候选"));
    }
    if failed > 0 {
        notes.push(format!("{failed} 只本地数据缺失，未纳入结果"));
    }
    (passed, evaluated, Some(notes.join("；")))
}

/// 翻页必须继续使用首屏对应的快照；强制刷新始终优先拿新数据。
fn snapshot_ttl(force_refresh: bool, reuse_snapshot: bool) -> Duration {
    if force_refresh {
        Duration::ZERO
    } else if reuse_snapshot {
        Duration::MAX
    } else {
        DEFAULT_SNAPSHOT_TTL
    }
}

/// 获取全市场股票池并应用筛选条件。
///
/// 筛选条件的优先级：`filter` > `preset` > 默认条件。
///
/// - `preset` 传预设 id（如 `"strong_breakout"`），未知 id 回退为「全部」
/// - `filter` 传完整的筛选条件（前端微调预设后传这个）
/// - `page` / `page_size`：**服务端分页**。传了 `page` 就只返回那一页
///   （`page_size` 缺省 20，上限 100）；翻页由前端逐页请求，避免一次传输几千行
/// - `reuse_snapshot`：翻页时复用当前快照，即使常规 60 秒 TTL 已过，保证页间一致
/// - 不传 `page` 时保持旧行为：按 `limit`（默认 500）从头截断
/// - `force_refresh` 为 `true` 时忽略缓存强制刷新
/// - `source` 指定取数通道（`sina` / `eastmoney` / `auto`），不传则读设置项 `universe_source`
///
/// ⚠️ 分页的前提是**顺序稳定**：这里按「成交额降序 + 代码升序」排序，
/// 否则两次请求之间顺序漂移会导致翻页出现重复或漏行。
#[tauri::command]
pub async fn get_market_universe(
    db: State<'_, Arc<Database>>,
    preset: Option<String>,
    filter: Option<MarketFilter>,
    page: Option<u32>,
    page_size: Option<u32>,
    limit: Option<usize>,
    force_refresh: Option<bool>,
    reuse_snapshot: Option<bool>,
    source: Option<String>,
) -> Result<UniverseResponse, String> {
    let ttl = snapshot_ttl(
        force_refresh.unwrap_or(false),
        reuse_snapshot.unwrap_or(false),
    );
    let started = std::time::Instant::now();

    // 通道优先级：调用方显式指定 > 设置项 > auto（新浪优先，东财兜底）
    let configured = source.or_else(|| {
        db.get_setting("universe_source")
            .ok()
            .flatten()
            .filter(|value| !value.trim().is_empty())
    });
    let preferred = preferred_source(configured.as_deref());

    let outcome = eastmoney_universe::market_snapshot_with_fallback(ttl, preferred)
        .await
        .map_err(|error| {
            format!(
                "获取全市场快照失败：{error}\
                 （已尝试新浪与东方财富两个通道；请检查网络，或在设置里切换数据源后重试）"
            )
        })?;

    let filter = filter.unwrap_or_else(|| {
        preset
            .as_deref()
            .map(preset_by_id)
            .unwrap_or(eastmoney_universe::FilterPreset::All)
            .build()
    });

    let capabilities = FilterCapabilities::for_source(outcome.source);
    let mut matched = filter.apply_with(&outcome.rows, capabilities);
    let skipped_conditions = filter.skipped_conditions(capabilities);

    // 稳定排序：成交额降序，同额按代码升序。没有这一步，分页就会重复/漏行。
    matched.sort_by(|a, b| {
        b.amount
            .partial_cmp(&a.amount)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.code.cmp(&b.code))
    });

    let (mut matched, history_evaluated, history_notice) =
        apply_history_filter(db.inner().clone(), matched, &filter).await;
    // 并发本地读取会打乱完成顺序，筛完后恢复稳定分页顺序。
    matched.sort_by(|a, b| {
        b.amount
            .partial_cmp(&a.amount)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.code.cmp(&b.code))
    });

    let board_counts = count_by_board(&outcome.rows)
        .into_iter()
        .map(|(board, count)| BoardCount {
            board,
            label: board.label().to_owned(),
            count,
        })
        .collect();

    let total_matched = matched.len();

    // 分页：传了 page 就按页取；否则退回旧的 limit 截断行为
    let rows: Vec<SnapshotRow> = match (page, page_size) {
        (Some(requested_page), _) => {
            let size = page_size
                .unwrap_or(DEFAULT_PAGE_SIZE)
                .clamp(1, MAX_PAGE_SIZE) as usize;
            let current = requested_page.max(1) as usize;
            let start = current.saturating_sub(1).saturating_mul(size);
            matched.into_iter().skip(start).take(size).collect()
        }
        (None, Some(size)) => {
            let size = size.clamp(1, MAX_PAGE_SIZE) as usize;
            matched.into_iter().take(size).collect()
        }
        _ => {
            let take = limit.unwrap_or(DEFAULT_ROW_LIMIT).min(total_matched);
            matched.into_iter().take(take).collect()
        }
    };

    log::info!(
        "[universe] get_market_universe: page={:?} size={:?} reuse_snapshot={} 快照={}({}) 全市场={} 命中={} 返回={} 耗时={}ms",
        page,
        page_size,
        reuse_snapshot.unwrap_or(false),
        outcome.source.label(),
        if outcome.stale { "陈旧" } else { "新鲜" },
        outcome.rows.len(),
        total_matched,
        rows.len(),
        started.elapsed().as_millis()
    );

    Ok(UniverseResponse {
        total_all: outcome.rows.len(),
        total_matched,
        returned: rows.len(),
        stale: outcome.stale,
        source: outcome.source,
        source_label: outcome.source.label().to_owned(),
        volume_ratio_supported: outcome.source.has_volume_ratio(),
        change_60d_supported: outcome.source.has_change_60d(),
        listing_date_supported: matches!(outcome.source, SnapshotSource::Eastmoney),
        skipped_conditions,
        history_notice,
        history_evaluated,
        board_counts,
        rows,
    })
}

/// 用户自建策略在 settings 表里的 key。
///
/// 复用现有 KV 表存一个 JSON 数组，**不需要动表结构、也不需要迁移** ——
/// 策略数量在几十条量级，整存整取足够，不值得为它单开一张表。
const CUSTOM_PRESETS_SETTING_KEY: &str = "universe_custom_presets";

/// 自定义策略数量上限。防止无节制堆积把设置接口的响应撑大。
const MAX_CUSTOM_PRESETS: usize = 30;

/// 策略名称最大字数（按字符数算，中文一个字算一个）
const MAX_LABEL_CHARS: usize = 16;

/// 策略说明最大字数
const MAX_DESCRIPTION_CHARS: usize = 80;

/// 读取用户自建策略。
///
/// 容错策略：**逐条校验，坏数据只丢这一条**，不能让一条脏数据把整条策略栏清空。
/// 数据整体解析失败时返回 `Err`，由调用方决定是否降级为「只有内置策略」。
fn load_custom_presets(db: &Database) -> Result<Vec<PresetInfo>, String> {
    let Some(raw) = db
        .get_setting(CUSTOM_PRESETS_SETTING_KEY)
        .map_err(|e| e.to_string())?
    else {
        return Ok(Vec::new());
    };
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }

    let parsed: Vec<PresetInfo> =
        serde_json::from_str(&raw).map_err(|e| format!("自定义策略数据解析失败：{e}"))?;

    let mut cleaned = Vec::with_capacity(parsed.len());
    for mut preset in parsed {
        // builtin 由后端裁定：用户数据里写什么都不作数
        preset.builtin = false;
        preset.id = preset.id.trim().to_owned();
        preset.label = preset.label.trim().to_owned();
        preset.description = preset.description.trim().to_owned();

        if preset.id.is_empty() || preset.label.is_empty() {
            log::warn!(
                "[universe] 跳过残缺的自定义策略：id={:?} label={:?}",
                preset.id,
                preset.label
            );
            continue;
        }
        if FilterPreset::is_builtin_id(&preset.id) {
            log::warn!(
                "[universe] 跳过 id 与内置预设冲突的自定义策略：{}",
                preset.id
            );
            continue;
        }
        // 规则 id 收敛到已知值：坏数据不能让「打开分析」拿到一个无法解析的规则
        if TradeRule::try_from_id(&preset.rule).is_err() {
            log::warn!(
                "[universe] 自定义策略 {} 的 rule={} 无效，按旧数据兼容为 trend_follow",
                preset.id,
                preset.rule
            );
        }
        preset.rule = TradeRule::from_id(&preset.rule).id().to_owned();
        preset.strategy_version_id = None;
        preset.strategy_version = 0;
        preset.strategy_status.clear();
        preset.filter.normalize_ranges();
        cleaned.push(preset);
    }
    Ok(cleaned)
}

/// 写回用户自建策略（整体覆盖）。
fn store_custom_presets(db: &Database, presets: &[PresetInfo]) -> Result<(), String> {
    db.replace_custom_strategy_presets(presets, None)
}

/// 生成一个没被占用的自定义策略 id
fn next_custom_id(existing: &[PresetInfo]) -> String {
    let stamp = chrono::Utc::now().timestamp_millis();
    let mut candidate = format!("custom_{stamp}");
    let mut suffix = 2u32;
    while existing.iter().any(|preset| preset.id == candidate) {
        candidate = format!("custom_{stamp}_{suffix}");
        suffix += 1;
    }
    candidate
}

/// 校验并规范化用户填的名称 / 说明。
///
/// 返回规范化后的 `(label, description)`；名称与内置或其它自定义策略重名时直接拒绝 ——
/// 允许重名的话，策略栏会出现两个一模一样的标签，用户根本分不清点的是哪个。
fn normalize_meta(
    label: &str,
    description: Option<&str>,
    self_id: &str,
    others: &[PresetInfo],
) -> Result<(String, String), String> {
    let label = label.trim();
    if label.is_empty() {
        return Err("策略名称不能为空".into());
    }
    let label_chars = label.chars().count();
    if label_chars > MAX_LABEL_CHARS {
        return Err(format!(
            "策略名称最多 {MAX_LABEL_CHARS} 个字，当前 {label_chars} 个"
        ));
    }

    if let Some(clash) = others
        .iter()
        .find(|preset| preset.id != self_id && preset.label == label)
    {
        return Err(format!("已有同名策略「{}」，换个名字吧", clash.label));
    }

    let description = description.unwrap_or_default().trim();
    let desc_chars = description.chars().count();
    if desc_chars > MAX_DESCRIPTION_CHARS {
        return Err(format!(
            "策略说明最多 {MAX_DESCRIPTION_CHARS} 个字，当前 {desc_chars} 个"
        ));
    }

    Ok((label.to_owned(), description.to_owned()))
}

/// 列出全部可用策略的**纯逻辑**：**内置预设在前，用户自建策略在后**。
///
/// 之所以 `Result` 不往外抛：内置策略是产品底线，
/// 不能因为用户自建数据坏了就让整条策略栏消失 —— 读不出来就只下发内置的。
fn all_presets_impl(db: &Database) -> Vec<PresetInfo> {
    let mut presets = preset_infos();
    let builtin_count = presets.len();

    match load_custom_presets(db) {
        Ok(custom) => presets.extend(custom),
        Err(error) => {
            log::warn!("[universe] 读取自定义策略失败，仅下发内置预设：{error}");
        }
    }

    if let Err(error) = db.sync_strategy_presets(&presets) {
        log::error!("[strategy] 策略版本库同步失败：{error}");
    } else if let Ok(library) = db.strategy_library() {
        let versions: std::collections::HashMap<_, _> = library
            .cards
            .into_iter()
            .map(|card| (card.id.clone(), card))
            .collect();
        for preset in &mut presets {
            if let Some(card) = versions.get(&preset.id) {
                preset.strategy_version_id = Some(card.version_id);
                preset.strategy_version = card.current_version;
                preset.strategy_status = card.status.clone();
            }
        }
    }

    log::info!(
        "[universe] get_filter_presets -> 内置 {} + 自建 {}",
        builtin_count,
        presets.len() - builtin_count
    );
    presets
}

pub(crate) fn sync_strategy_registry(db: &Database) {
    let _ = all_presets_impl(db);
}

#[tauri::command]
pub fn get_filter_presets(db: State<'_, Arc<Database>>) -> Vec<PresetInfo> {
    all_presets_impl(db.inner().as_ref())
}

/// 保存策略的**纯逻辑**（不依赖 Tauri `State`，便于直接测试落库往返）。
///
/// - `id` 为空 → 新建（自动分配 `custom_*` 的 id）
/// - `id` 指向已有自建策略 → 覆盖它的名称、说明与条件
/// - `id` 指向内置策略 → 拒绝，并提示改用「另存为我的策略」
fn save_preset_impl(
    db: &Database,
    id: Option<&str>,
    label: &str,
    description: Option<&str>,
    rule: Option<&str>,
    filter: MarketFilter,
) -> Result<PresetInfo, String> {
    let mut customs = load_custom_presets(db)?;

    let requested = id.map(str::trim).filter(|value| !value.is_empty());

    let target_id = match requested {
        Some(existing) => {
            if FilterPreset::is_builtin_id(existing) {
                return Err(format!(
                    "「{existing}」是内置策略，不能直接改；请用「另存为我的策略」存一份自己的副本再改"
                ));
            }
            if !customs.iter().any(|preset| preset.id == existing) {
                return Err("要修改的策略不存在，可能已经在别处被删掉了".into());
            }
            existing.to_owned()
        }
        None => {
            if customs.len() >= MAX_CUSTOM_PRESETS {
                return Err(format!(
                    "自定义策略最多 {MAX_CUSTOM_PRESETS} 个，请先删掉一些不再用的"
                ));
            }
            next_custom_id(&customs)
        }
    };

    // 重名检查要连内置一起看：用户把自建策略叫「强势突破」同样会撞车
    let mut others = preset_infos();
    others.extend(customs.iter().cloned());
    let (label, description) = normalize_meta(label, description, &target_id, &others)?;

    let mut filter = filter;
    filter.normalize_ranges();

    let saved = PresetInfo {
        id: target_id.clone(),
        label,
        description,
        filter,
        rule: match rule.map(str::trim).filter(|value| !value.is_empty()) {
            Some(value) => TradeRule::try_from_id(value)?.id().to_owned(),
            None => TradeRule::TrendFollow.id().to_owned(),
        },
        builtin: false,
        strategy_version_id: None,
        strategy_version: 0,
        strategy_status: String::new(),
    };

    match customs.iter_mut().find(|preset| preset.id == target_id) {
        Some(slot) => *slot = saved.clone(),
        None => customs.push(saved.clone()),
    }
    store_custom_presets(db, &customs)?;
    Ok(saved)
}

/// 删除策略的**纯逻辑**（内置策略一律拒绝）
fn delete_preset_impl(db: &Database, id: &str) -> Result<(), String> {
    let id = id.trim();
    if FilterPreset::is_builtin_id(id) {
        return Err("内置策略不能删除；想改造它的话，用「另存为我的策略」存一份副本再改".into());
    }

    let mut customs = load_custom_presets(db)?;
    let before = customs.len();
    customs.retain(|preset| preset.id != id);
    if customs.len() == before {
        return Err("要删除的策略不存在，可能已经被删掉了".into());
    }

    db.replace_custom_strategy_presets(&customs, Some(id))
}

/// 保存一条策略，**新建 / 重命名 / 用当前条件覆盖**三种动作共用这一个命令。
#[tauri::command]
pub fn save_filter_preset(
    db: State<'_, Arc<Database>>,
    id: Option<String>,
    label: String,
    description: Option<String>,
    rule: Option<String>,
    filter: MarketFilter,
) -> Result<PresetInfo, String> {
    let saved = save_preset_impl(
        db.inner().as_ref(),
        id.as_deref(),
        &label,
        description.as_deref(),
        rule.as_deref(),
        filter,
    )?;
    log::info!(
        "[universe] save_filter_preset -> {} 「{}」规则={}",
        saved.id,
        saved.label,
        saved.rule
    );
    Ok(saved)
}

/// 删除一条用户自建策略。内置策略一律拒绝 —— 它们是产品随版本维护的基准方案。
#[tauri::command]
pub fn delete_filter_preset(db: State<'_, Arc<Database>>, id: String) -> Result<(), String> {
    delete_preset_impl(db.inner().as_ref(), &id)?;
    log::info!("[universe] delete_filter_preset -> {id}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{accepts_history, next_custom_id, normalize_meta, snapshot_ttl, MAX_LABEL_CHARS};
    use crate::datasource::eastmoney_universe::{
        preset_infos, MarketFilter, PresetInfo, DEFAULT_SNAPSHOT_TTL,
    };
    use crate::domain::KLineData;
    use crate::quant::playbook::TradeRule;
    use std::time::Duration;

    fn custom(id: &str, label: &str) -> PresetInfo {
        PresetInfo {
            id: id.to_owned(),
            label: label.to_owned(),
            description: String::new(),
            filter: MarketFilter::default(),
            rule: TradeRule::TrendFollow.id().to_owned(),
            builtin: false,
            strategy_version_id: None,
            strategy_version: 0,
            strategy_status: String::new(),
        }
    }

    #[test]
    fn force_refresh_overrides_pagination_snapshot_reuse() {
        assert_eq!(snapshot_ttl(false, false), DEFAULT_SNAPSHOT_TTL);
        assert_eq!(snapshot_ttl(false, true), Duration::MAX);
        assert_eq!(snapshot_ttl(true, true), Duration::ZERO);
    }

    #[test]
    fn history_conditions_accept_a_rising_breakout_and_reject_a_ma_break() {
        let mut rows: Vec<KLineData> = (0..60)
            .map(|day| {
                let close = 10.0 + (day * day) as f64 * 0.01;
                KLineData {
                    date: format!("2026-07-{:02}", day % 28 + 1),
                    open: close - 0.1,
                    high: close + 0.2,
                    low: close - 0.3,
                    close,
                    volume: if day == 59 { 3_000 } else { 1_000 },
                    turnover: 0.0,
                }
            })
            .collect();
        let filter = MarketFilter {
            above_ma_days: Some(20),
            new_high_days: Some(20),
            macd_bullish: true,
            kdj_bullish: true,
            volume_price_rising: true,
            rise_from_low_days: Some(20),
            rise_from_low_min: Some(0.0),
            rise_from_low_max: Some(200.0),
            ..MarketFilter::default()
        };
        assert!(accepts_history(&filter, &rows));
        rows.last_mut().unwrap().close = 5.0;
        assert!(!accepts_history(&filter, &rows));
    }

    /// 内置策略必须被标记为 builtin，否则前端会给出「改名/删除」入口，
    /// 用户点了才发现在后端被拒 —— 交互上很别扭。
    #[test]
    fn builtin_presets_are_flagged_and_use_prefixed_ids() {
        let infos = preset_infos();
        assert!(!infos.is_empty());
        for info in &infos {
            assert!(info.builtin, "内置预设 {} 未标记 builtin", info.id);
            assert!(
                !info.id.starts_with("custom_"),
                "内置预设 {} 不应占用 custom_ 前缀",
                info.id
            );
        }
    }

    /// 同一毫秒内连续新建也不能撞 id，否则后存的会把先存的覆盖掉。
    #[test]
    fn generated_custom_ids_never_collide() {
        let mut existing: Vec<PresetInfo> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..50 {
            let id = next_custom_id(&existing);
            assert!(seen.insert(id.clone()), "生成了重复 id：{id}");
            assert!(id.starts_with("custom_"));
            existing.push(custom(&id, &id));
        }
    }

    #[test]
    fn name_is_trimmed_and_must_not_be_blank() {
        let (label, desc) = normalize_meta("  我的策略  ", Some("  说明  "), "custom_1", &[])
            .expect("应当通过校验");
        assert_eq!(label, "我的策略");
        assert_eq!(desc, "说明");

        assert!(
            normalize_meta("   ", None, "custom_1", &[]).is_err(),
            "全空白名称应被拒"
        );
    }

    #[test]
    fn duplicate_names_are_rejected_except_for_self() {
        let others = vec![
            custom("custom_1", "挖坑策略"),
            custom("custom_2", "打板策略"),
        ];

        // 撞别人的名字 → 拒绝，且错误信息里要带上冲突的名字，便于用户判断
        let err = normalize_meta("挖坑策略", None, "custom_2", &others).unwrap_err();
        assert!(
            err.contains("挖坑策略"),
            "错误信息应指出撞了哪个名字：{err}"
        );

        // 改自己的名字（id 相同）→ 放行
        assert!(normalize_meta("挖坑策略", None, "custom_1", &others).is_ok());
    }

    /// 中文按字数算而不是按字节 —— 用 len() 的话 16 个汉字会被当成 48 而误拒。
    #[test]
    fn length_limit_counts_chars_not_bytes() {
        let just_fits = "策".repeat(MAX_LABEL_CHARS);
        assert!(normalize_meta(&just_fits, None, "custom_1", &[]).is_ok());

        let too_long = "策".repeat(MAX_LABEL_CHARS + 1);
        assert!(
            normalize_meta(&too_long, None, "custom_1", &[]).is_err(),
            "超长名称应被拒"
        );

        let long_desc = "说".repeat(81);
        assert!(
            normalize_meta("合规名称", Some(&long_desc), "custom_1", &[]).is_err(),
            "超长说明应被拒"
        );
    }

    /// 落库往返：保存 → 读回 → 改名 → 覆盖条件 → 删除。
    ///
    /// 这是「用户数据真的写进 settings 表再读回来」的唯一路径。不测它的话，
    /// 序列化 / 反序列化 / 覆盖语义上的问题只能在真机上才暴露 ——
    /// 而这正是「存了策略重启后不见了」这类问题最常见的成因。
    #[test]
    fn custom_presets_round_trip_through_the_settings_table() {
        use super::{all_presets_impl, delete_preset_impl, load_custom_presets, save_preset_impl};
        use crate::db::Database;

        let dir =
            std::env::temp_dir().join(format!("bull-arrives-preset-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let db = Database::open(dir.clone()).expect("应能建库");

        // 起点：一条自建策略都没有，但内置的齐全
        assert!(load_custom_presets(&db).expect("初始读取应成功").is_empty());
        let builtin_total = preset_infos().len();

        // 保存三条
        let mut ids = Vec::new();
        for (label, cap) in [
            ("我的低估值", 50.0),
            ("我的高换手", 80.0),
            ("我的超跌", 30.0),
        ] {
            let filter = MarketFilter {
                market_cap_min_yi: Some(cap),
                ..MarketFilter::default()
            };
            let saved = save_preset_impl(&db, None, label, Some("测试用"), None, filter)
                .expect("保存应成功");
            assert!(saved.id.starts_with("custom_"), "自建 id 应带 custom_ 前缀");
            assert!(!saved.builtin);
            ids.push(saved.id);
        }

        // 读回：内置在前、自建在后，且互不污染
        let stored = load_custom_presets(&db).expect("应能读回");
        assert_eq!(stored.len(), 3, "三条自建策略都应读回");
        assert!(stored.iter().all(|preset| !preset.builtin));
        assert_eq!(all_presets_impl(&db).len(), builtin_total + 3);

        // 改名 + 覆盖条件：id 不变、条件被替换
        let renamed = save_preset_impl(
            &db,
            Some(ids[0].as_str()),
            "改过的名字",
            Some("新说明"),
            Some("mean_reversion"),
            MarketFilter {
                pb_max: Some(1.5),
                ..MarketFilter::default()
            },
        )
        .expect("覆盖应成功");
        assert_eq!(renamed.id, ids[0], "覆盖不应改 id");
        assert_eq!(renamed.label, "改过的名字");
        assert_eq!(renamed.rule, "mean_reversion", "规则应随保存一起落库");
        assert_eq!(renamed.filter.pb_max, Some(1.5), "条件应被整体替换");
        assert_eq!(renamed.filter.market_cap_min_yi, None, "旧条件不应残留");
        assert_eq!(
            load_custom_presets(&db).expect("读取应成功").len(),
            3,
            "覆盖不该新增"
        );

        // 填反的区间在落库前被交换过来
        let swapped = save_preset_impl(
            &db,
            None,
            "填反了的",
            None,
            None,
            MarketFilter {
                price_min: Some(100.0),
                price_max: Some(10.0),
                ..MarketFilter::default()
            },
        )
        .expect("保存应成功");
        assert_eq!(swapped.filter.price_min, Some(10.0));
        assert_eq!(swapped.filter.price_max, Some(100.0));
        // 未知 / 缺省规则一律收敛为趋势跟随
        assert_eq!(swapped.rule, "trend_follow");

        // 重名要被拒（含与内置撞名）
        assert!(
            save_preset_impl(&db, None, "改过的名字", None, None, MarketFilter::default()).is_err(),
            "自建之间不应允许重名"
        );
        assert!(
            save_preset_impl(&db, None, "强势突破", None, None, MarketFilter::default()).is_err(),
            "与内置策略重名也应被拒"
        );
        // 内置策略不能被改
        assert!(
            save_preset_impl(
                &db,
                Some("all"),
                "偷改内置",
                None,
                None,
                MarketFilter::default()
            )
            .is_err(),
            "内置策略不允许直接修改"
        );

        // 删除自建：只删掉目标那一条
        delete_preset_impl(&db, &ids[1]).expect("删除应成功");
        let after = load_custom_presets(&db).expect("读取应成功");
        assert_eq!(after.len(), 3, "上面新存过一条，删一条后应为 3");
        assert!(!after.iter().any(|preset| preset.id == ids[1]));

        // 删不存在的、删内置的都要报错
        assert!(delete_preset_impl(&db, &ids[1]).is_err(), "重复删除应报错");
        assert!(
            delete_preset_impl(&db, "all").is_err(),
            "内置策略不允许删除"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 脏数据不能让整条策略栏消失：坏条目逐条丢弃，内置策略照常下发。
    #[test]
    fn corrupt_custom_presets_degrade_to_builtin_only() {
        use super::{all_presets_impl, load_custom_presets, CUSTOM_PRESETS_SETTING_KEY};
        use crate::db::Database;

        let dir = std::env::temp_dir().join(format!(
            "bull-arrives-preset-corrupt-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let db = Database::open(dir.clone()).expect("应能建库");

        // 整体不是合法 JSON → 读取失败，但内置策略仍然要能下发
        db.set_setting(CUSTOM_PRESETS_SETTING_KEY, "{ 这不是 JSON")
            .expect("写入应成功");
        assert!(load_custom_presets(&db).is_err());
        assert_eq!(
            all_presets_impl(&db).len(),
            preset_infos().len(),
            "自建数据坏掉时应只剩内置策略，而不是一条都不剩"
        );

        // 数组里混入残缺条目 → 只丢坏的那条
        db.set_setting(
            CUSTOM_PRESETS_SETTING_KEY,
            r#"[
                {"id":"custom_ok","label":"好的","description":"","filter":{}},
                {"id":"","label":"缺 id","description":"","filter":{}},
                {"id":"custom_nolabel","label":"   ","description":"","filter":{}},
                {"id":"all","label":"冒用内置 id","description":"","filter":{}}
            ]"#,
        )
        .expect("写入应成功");

        let cleaned = load_custom_presets(&db).expect("应能读回");
        assert_eq!(cleaned.len(), 1, "只应保留那一条完整的");
        assert_eq!(cleaned[0].id, "custom_ok");
        assert!(!cleaned[0].builtin);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
