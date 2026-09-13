// src-tauri/src/commands/rank.rs
//! 推荐榜：对筛选结果里最活跃的 N 只股票批量拉日 K + 评分，按总分排序。
//!
//! 这是五层漏斗 L3 的「批量落地」——把单只 `analyze_stock` 扩展成对一篮子股票
//! 的扫描，产出「今日推荐榜」。为控制成本与限流：
//! - 只对「成交额最大的前 N 只」评分（最活跃的票才值得算，而不是全市场 5000 只）
//! - 日 K 请求用 Semaphore 限制并发，结果按总分降序

use crate::datasource::kline;
use crate::datasource::eastmoney_universe::{
    self, preferred_source, Board, FilterCapabilities, MarketFilter, SnapshotRow, SnapshotSource,
    DEFAULT_SNAPSHOT_TTL,
};
use crate::db::Database;
use crate::quant::scorer::{self, StockAnalysis};
use serde::Serialize;
use std::cmp::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tauri::State;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

/// 推荐榜的一行
#[derive(Debug, Serialize)]
pub struct RankItem {
    /// 6 位代码
    pub code: String,
    pub name: String,
    pub board: Board,
    /// 当日涨跌幅 %
    pub change_pct: f64,
    /// 成交额（元）
    pub amount: f64,
    /// 量化评分（拉日 K 失败或数据不足时为 None）
    pub analysis: Option<StockAnalysis>,
    /// 评分失败原因（成功时为 None）
    pub error: Option<String>,
}

/// 推荐榜响应
#[derive(Debug, Serialize)]
pub struct RankResponse {
    /// 实际参与筛选的股票池大小（轻量扫描时是「最活跃的一批」而非全市场）
    pub scanned: usize,
    /// 命中筛选的总数
    pub total_matched: usize,
    /// 参与评分的数量（= 取前 N 只最活跃股）
    pub candidates: usize,
    /// 成功评分的数量
    pub scored: usize,
    /// 失败数量
    pub failed: usize,
    /// 快照是否陈旧（刷新失败回退旧数据）
    pub stale: bool,
    /// 本次数据来自哪个通道
    pub source: SnapshotSource,
    /// 因数据源不支持而被自动忽略的条件名（如 ["量比"]）
    pub skipped_conditions: Vec<String>,
    /// 按总分降序的榜单
    pub items: Vec<RankItem>,
}

/// 日 K 请求并发上限。push2his 比 clist 宽松，但仍要节制，避免触发东财 IP 限流。
const KLINE_CONCURRENCY: usize = 5;

/// 默认参与评分的股票数量
const DEFAULT_TOP_N: usize = 30;

/// 轻量扫描页数（每页 100 只，按成交额降序）。
///
/// 全市场遍历要 60 次请求，既慢又极易触发东财 IP 限流——这是推荐榜失败的首要原因。
/// 而推荐榜只关心「最活跃的一批」，成交额前 1000 名已经绰绰有余。
const RANK_SCAN_PAGES: u32 = 10;

fn snapshot_error(error: eastmoney_universe::EmError) -> String {
    format!(
        "获取全市场快照失败：{error}。\
         常见原因：网络不通、东方财富临时限流（连续请求过多）、或系统代理拦截。请稍后重试。"
    )
}

/// 扫描并生成推荐榜。
///
/// - `filter`：筛选条件（与筛选器共用 `MarketFilter`）
/// - `top_n`：参与评分的股票数（默认 30，上限 60）
/// - `force_refresh`：是否强制刷新全市场快照
#[tauri::command]
pub async fn scan_and_rank(
    db: State<'_, Arc<Database>>,
    filter: Option<MarketFilter>,
    top_n: Option<usize>,
    force_refresh: Option<bool>,
    source: Option<String>,
) -> Result<RankResponse, String> {
    let top_n = top_n.unwrap_or(DEFAULT_TOP_N).clamp(1, 60);
    let ttl = if force_refresh.unwrap_or(false) {
        Duration::ZERO
    } else {
        DEFAULT_SNAPSHOT_TTL
    };

    let configured = source.or_else(|| {
        db.get_setting("universe_source")
            .ok()
            .flatten()
            .filter(|value| !value.trim().is_empty())
    });
    let preferred = preferred_source(configured.as_deref());

    let filter = filter.unwrap_or_default();

    // 1) 轻量路径：只扫成交额最高的前 RANK_SCAN_PAGES 页（几次请求而非全市场几十次）。
    //    两个通道都失败时才回退到带缓存的全市场快照，仍失败才报错。
    let mut stale = false;
    let mut source_used;
    let mut rows: Vec<SnapshotRow> =
        match eastmoney_universe::fetch_top_active_auto(RANK_SCAN_PAGES, preferred).await {
            Ok((rows, source)) => {
                source_used = source;
                rows
            }
            Err(error) => {
                log::warn!("[rank] 活跃股轻量扫描两个通道均失败，回退全市场快照：{error}");
                let outcome = eastmoney_universe::market_snapshot_with_fallback(ttl, preferred)
                    .await
                    .map_err(snapshot_error)?;
                stale = outcome.stale;
                source_used = outcome.source;
                outcome.rows.as_ref().clone()
            }
        };

    // 2) 轻量池较窄，条件苛刻时命中可能不足 top_n，这时补一次全市场扫描
    let mut capabilities = FilterCapabilities::for_source(source_used);
    if filter.apply_with(&rows, capabilities).len() < top_n {
        if let Ok(outcome) = eastmoney_universe::market_snapshot_with_fallback(ttl, preferred).await {
            let full = outcome.rows.as_ref().clone();
            if full.len() > rows.len() {
                stale = outcome.stale;
                source_used = outcome.source;
                capabilities = FilterCapabilities::for_source(source_used);
                rows = full;
            }
        }
    }

    let matched = filter.apply_with(&rows, capabilities);
    let total_matched = matched.len();
    let skipped_conditions = filter.skipped_conditions(capabilities);

    // 按成交额降序，取最活跃的前 top_n 只（活跃度高的票才有分析价值，也顺带控制请求量）
    let mut candidates = matched;
    candidates.sort_by(|a, b| b.amount.partial_cmp(&a.amount).unwrap_or(Ordering::Equal));
    candidates.truncate(top_n);

    let mut items = rank_candidates(&candidates).await;

    // 按总分降序；评分失败的（analysis=None）排最后
    items.sort_by(|a, b| {
        let sa = a.analysis.as_ref().map(|x| x.total_score).unwrap_or(-1.0);
        let sb = b.analysis.as_ref().map(|x| x.total_score).unwrap_or(-1.0);
        sb.partial_cmp(&sa).unwrap_or(Ordering::Equal)
    });

    let scored = items.iter().filter(|x| x.analysis.is_some()).count();
    let failed = items.len() - scored;

    Ok(RankResponse {
        scanned: rows.len(),
        total_matched,
        candidates: items.len(),
        scored,
        failed,
        stale,
        source: source_used,
        skipped_conditions,
        items,
    })
}

/// 并发拉日 K + 评分。每个任务内部拿一个信号量许可，限制同时发起的请求数。
async fn rank_candidates(candidates: &[SnapshotRow]) -> Vec<RankItem> {
    let sem = Arc::new(Semaphore::new(KLINE_CONCURRENCY));
    let mut set = JoinSet::new();

    for row in candidates {
        let row = row.clone();
        let sem = sem.clone();
        set.spawn(async move {
            let _permit = sem.acquire_owned().await.expect("semaphore not closed");
            let symbol = kline::full_symbol_from(&row.code, row.board);
            let (analysis, error) = match kline::fetch_daily_kline(&symbol, 250).await {
                Ok(klines) => match scorer::analyze(&klines) {
                    Some(a) => (Some(a), None),
                    None => (None, Some(format!("K 线不足（{} 根）", klines.len()))),
                },
                Err(e) => (None, Some(e)),
            };
            RankItem {
                code: row.code,
                name: row.name,
                board: row.board,
                change_pct: row.change_pct,
                amount: row.amount,
                analysis,
                error,
            }
        });
    }

    let mut items = Vec::with_capacity(candidates.len());
    while let Some(res) = set.join_next().await {
        if let Ok(item) = res {
            items.push(item);
        }
    }
    items
}
