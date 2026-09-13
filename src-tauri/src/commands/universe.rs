//! 全市场股票池相关命令（漏斗 L0 获取 + L1 筛选）
//!
//! 对应前端「全市场筛选器」。业务上属于漏斗前两层：
//! 一次拉取全市场快照，然后在内存里按用户配置的条件过滤 —— **筛选本身零网络请求**。

use crate::datasource::eastmoney_universe::{
    self, count_by_board, preferred_source, preset_by_id, preset_infos, Board, FilterCapabilities,
    MarketFilter, PresetInfo, SnapshotRow, SnapshotSource, DEFAULT_SNAPSHOT_TTL,
};
use crate::db::Database;
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;
use tauri::State;

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
    /// 快照已存在多少秒（`stale` 为 true 时这个值会比较大）
    pub fetched_age_secs: u64,
    /// 数据是否陈旧：本次刷新失败（很可能被限流），返回的是上次的旧数据
    pub stale: bool,
    /// 本次数据来自哪个通道（sina / eastmoney）
    pub source: SnapshotSource,
    /// 通道中文名，前端直接显示
    pub source_label: String,
    /// 当前通道是否提供「量比」。为 false 时前端应提示该条件当前不可用，
    /// 否则用户设了「量比 ≥ x」会得到 0 结果而不知道为什么。
    pub volume_ratio_supported: bool,
    /// 因数据源不支持而被自动忽略的条件名（如 ["量比"]），供前端明确提示
    pub skipped_conditions: Vec<String>,
    /// 全市场板块分布
    pub board_counts: Vec<BoardCount>,
    /// 命中筛选的明细
    pub rows: Vec<SnapshotRow>,
}

/// 返回行数默认上限。
/// 全市场 5900 行一次 IPC 传过去约 3MB，会明显拖慢前端渲染，
/// 所以默认截断；调用方可按需提高。
const DEFAULT_ROW_LIMIT: usize = 500;

/// 服务端分页的默认每页条数（与前端默认值一致）
const DEFAULT_PAGE_SIZE: u32 = 20;

/// 服务端分页的每页上限（与前端可选的最大值一致）
const MAX_PAGE_SIZE: u32 = 100;

/// 获取全市场股票池并应用筛选条件。
///
/// 筛选条件的优先级：`filter` > `preset` > 默认条件。
///
/// - `preset` 传预设 id（如 `"strong_breakout"`），未知 id 回退为「全部」
/// - `filter` 传完整的筛选条件（前端微调预设后传这个）
/// - `page` / `page_size`：**服务端分页**。传了 `page` 就只返回那一页
///   （`page_size` 缺省 20，上限 100）；翻页由前端逐页请求，避免一次传输几千行
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
    source: Option<String>,
) -> Result<UniverseResponse, String> {
    let ttl = if force_refresh.unwrap_or(false) {
        // ttl=0 使缓存立即过期，从而强制真实拉取
        Duration::ZERO
    } else {
        DEFAULT_SNAPSHOT_TTL
    };
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
            let size = page_size.unwrap_or(DEFAULT_PAGE_SIZE).clamp(1, MAX_PAGE_SIZE) as usize;
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
        "[universe] get_market_universe: page={:?} size={:?} 快照={}({}) 全市场={} 命中={} 返回={} 耗时={}ms",
        page,
        page_size,
        outcome.source.label(),
        if outcome.stale { "陈旧" } else { "新鲜" },
        outcome.rows.len(),
        total_matched,
        rows.len(),
        started.elapsed().as_millis()
    );

    let fetched_age_secs = eastmoney_universe::snapshot_cache_age()
        .await
        .unwrap_or_default()
        .as_secs();

    Ok(UniverseResponse {
        total_all: outcome.rows.len(),
        total_matched,
        returned: rows.len(),
        fetched_age_secs,
        stale: outcome.stale,
        source: outcome.source,
        source_label: outcome.source.label().to_owned(),
        volume_ratio_supported: outcome.source.has_volume_ratio(),
        skipped_conditions,
        board_counts,
        rows,
    })
}

/// 列出全部内置预设方案（含展开后的筛选条件），供前端渲染一键切换条。
///
/// 预设定义在 Rust 侧而非前端，好处是：**规则只有一份**，
/// 后续要做盘后扫描、AI 参数拟定都复用同一套，不会前后端各写一遍导致不一致。
#[tauri::command]
pub fn get_filter_presets() -> Vec<PresetInfo> {
    let presets = preset_infos();
    log::info!("[universe] get_filter_presets -> {} 个预设", presets.len());
    presets
}
