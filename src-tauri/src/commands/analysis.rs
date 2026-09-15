// src-tauri/src/commands/analysis.rs
// 量化分析命令 —— 把「数据接入（eastmoney_kline）+ 指标打分（quant::scorer）
// + 操作计划（quant::playbook）+ 规则回测（quant::backtest）」暴露给前端。

use crate::datasource::kline;
use crate::quant::playbook::TradeRule;
use crate::quant::scorer::StockAnalysis;
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

/// 分析单只股票：拉取日 K，给出技术评分、操作计划与规则回测。
///
/// 接收**完整符号**（`sh600519` / `sz000001` / `bj920xxx`），
/// 拉取 250 根前复权日 K（约一年，足以让 MA60 / 60 日动量等指标收敛）。
///
/// `rule` 指定用哪条交易规则算买卖点（`trend_follow` / `mean_reversion` / `breakout`），
/// 不传或传未知值即回落为趋势跟随。前端把它与当前策略绑定 ——
/// **筛选器负责粗筛（单日快照），规则负责精确点位（日 K）**，
/// 因为快照里既没有均线也没有 RSI，买点只能逐只看日 K 才准。
///
/// 说明：日 K 分析是盘前/盘后也要用的能力，因此**不走** `DataSourceManager`
/// 的交易时段门禁（`ensure_request_allowed`），直接走东财历史 K 线接口。
#[tauri::command]
pub async fn analyze_stock(symbol: String, rule: Option<String>) -> Result<StockAnalysis, String> {
    let klines = crate::datasource::kline::fetch_daily_kline(&symbol, 250).await?;
    let rule = rule
        .as_deref()
        .map(TradeRule::from_id)
        .unwrap_or(TradeRule::TrendFollow);

    let mut analysis = crate::quant::scorer::analyze(&klines).ok_or_else(|| {
        format!(
            "K 线数据不足（{} 根，至少需 60 根），无法评分",
            klines.len()
        )
    })?;

    // 操作计划：算不出就说算不出（K 线不足 / 价格异常时返回 None），
    // 绝不给一个凑出来的止损价
    analysis.trade_plan = crate::quant::playbook::plan(&klines, rule);
    // 规则回测：让「胜率」这个词落到这只股票自己的历史上，而不是引一个别人的数字
    analysis.backtest = crate::quant::backtest::run(&klines, rule);

    if analysis.trade_plan.is_none() {
        log::warn!("[analysis] {symbol} 未能生成操作计划（K 线不足或价格异常）");
    }
    log::info!(
        "[analysis] {symbol} 规则={} 评分={:.1} K线={} 计划={} 回测触发={}",
        rule.id(),
        analysis.total_score,
        klines.len(),
        if analysis.trade_plan.is_some() { "有" } else { "无" },
        analysis
            .backtest
            .as_ref()
            .map(|b| b.trades.to_string())
            .unwrap_or_else(|| "-".to_string())
    );

    Ok(analysis)
}

/// 筛选器结果表里「评分」与「入场」两列的状态。
///
/// 为什么要有批量命令：筛选器一页最多 100 只股票，若前端逐只调 `analyze_stock`，
/// 就要发 100 次 IPC + 100 次网络往返，且并发完全由前端拍脑袋决定。
/// 这里复用推荐榜的并发池（信号量 5）在 Rust 侧一次跑完，前端只发一次调用。
#[derive(Debug, Serialize)]
pub struct StockStatusItem {
    /// 完整符号（`sh600519`），前端用它回填到对应行
    pub symbol: String,
    /// 量化评分；拉 K 线失败或数据不足时为 None
    pub score: Option<f64>,
    /// 当前是否已满足入场条件；无计划（K 线不足 / 价格异常）时为 None
    pub ready: Option<bool>,
    /// 未满足时缺什么条件
    pub waiting_for: Option<String>,
    /// 建议建仓区间（无计划时为 None）
    pub buy_low: Option<f64>,
    pub buy_high: Option<f64>,
    pub stop_loss: Option<f64>,
    pub take_profit: Option<f64>,
    pub risk_reward: Option<f64>,
    /// 失败原因；成功时为 None
    pub error: Option<String>,
}

impl StockStatusItem {
    /// 算不出来时的占位行 —— 让前端能区分「还没算」与「算了但算不出」。
    /// 绝不给一个凑出来的买点，与 `TradePlan` 的取舍保持一致。
    fn failed(symbol: String, error: String) -> Self {
        Self {
            symbol,
            score: None,
            ready: None,
            waiting_for: None,
            buy_low: None,
            buy_high: None,
            stop_loss: None,
            take_profit: None,
            risk_reward: None,
            error: Some(error),
        }
    }
}

/// 单次批量分析的股票数上限。
///
/// 与筛选器单页上限（100）对齐并留一点余量：这个命令是给「当前页」用的，
/// 不是全市场扫描。用户真要全量排序，走「全部量化评分排序」那条路。
const BATCH_MAX: usize = 120;

/// 日 K 请求并发上限。与推荐榜保持一致 —— push2his 比 clist 宽松，但仍要节制，
/// 否则连续翻页时会明显更容易触发东财 IP 限流。
const BATCH_CONCURRENCY: usize = 5;

/// 批量给一篮子股票算「评分 + 当前是否满足入场条件」。
///
/// 用途：筛选器结果表里的「入场」列要直接展示状态，而不是让用户逐只点开分析。
///
/// `rule` 必须与当前策略配套（与 `analyze_stock` 同一套语义），
/// 否则会出现「用超跌反弹策略选出来、却按趋势规则判断入场」的错配。
#[tauri::command]
pub async fn batch_stock_status(
    symbols: Vec<String>,
    rule: Option<String>,
) -> Result<Vec<StockStatusItem>, String> {
    if symbols.is_empty() {
        return Ok(Vec::new());
    }
    let rule = rule
        .as_deref()
        .map(TradeRule::from_id)
        .unwrap_or(TradeRule::TrendFollow);

    // 去重：同一页里不该出现重复符号，真出现了也只算一次
    let mut seen = std::collections::HashSet::new();
    let targets: Vec<String> = symbols
        .into_iter()
        .filter(|s| !s.trim().is_empty() && seen.insert(s.clone()))
        .take(BATCH_MAX)
        .collect();

    let sem = Arc::new(Semaphore::new(BATCH_CONCURRENCY));
    let mut set = JoinSet::new();

    for symbol in targets {
        let sem = sem.clone();
        set.spawn(async move {
            let _permit = sem.acquire_owned().await.expect("semaphore not closed");
            let klines = match kline::fetch_daily_kline(&symbol, 250).await {
                Ok(klines) => klines,
                Err(e) => return StockStatusItem::failed(symbol, e),
            };
            let analysis = match crate::quant::scorer::analyze(&klines) {
                Some(a) => a,
                None => {
                    return StockStatusItem::failed(
                        symbol,
                        format!("K 线数据不足（{} 根，至少需 60 根）", klines.len()),
                    )
                }
            };
            let plan = crate::quant::playbook::plan(&klines, rule);
            StockStatusItem {
                symbol,
                score: Some(analysis.total_score),
                ready: plan.as_ref().map(|p| p.ready),
                waiting_for: plan.as_ref().and_then(|p| p.waiting_for.clone()),
                buy_low: plan.as_ref().map(|p| p.buy_low),
                buy_high: plan.as_ref().map(|p| p.buy_high),
                stop_loss: plan.as_ref().map(|p| p.stop_loss),
                take_profit: plan.as_ref().map(|p| p.take_profit),
                risk_reward: plan.as_ref().map(|p| p.risk_reward),
                error: None,
            }
        });
    }

    let mut items = Vec::new();
    while let Some(res) = set.join_next().await {
        match res {
            Ok(item) => items.push(item),
            // 任务被 panic 时不能整体失败：丢掉这一只，其余照常返回
            Err(e) => log::warn!("[analysis] 批量状态任务异常：{e}"),
        }
    }
    Ok(items)
}
