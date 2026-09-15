// src-tauri/src/commands/analysis.rs
// 量化分析命令 —— 把「数据接入（eastmoney_kline）+ 指标打分（quant::scorer）
// + 操作计划（quant::playbook）+ 规则回测（quant::backtest）」暴露给前端。

use crate::quant::playbook::TradeRule;
use crate::quant::scorer::StockAnalysis;

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
