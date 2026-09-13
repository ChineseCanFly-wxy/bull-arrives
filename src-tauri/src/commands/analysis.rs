// src-tauri/src/commands/analysis.rs
// 量化分析命令 —— 把「数据接入（eastmoney_kline）+ 指标打分（quant::scorer）」暴露给前端。

use crate::quant::scorer::StockAnalysis;

/// 分析单只股票：拉取日 K 并给出技术评分。
///
/// 接收**完整符号**（`sh600519` / `sz000001` / `bj920xxx`），
/// 拉取 250 根前复权日 K（约一年，足以让 MA60 / 60 日动量等指标收敛），
/// 输出多因子加权评分与各因子明细。
///
/// 说明：日 K 分析是盘前/盘后也要用的能力，因此**不走** `DataSourceManager`
/// 的交易时段门禁（`ensure_request_allowed`），直接走东财历史 K 线接口。
#[tauri::command]
pub async fn analyze_stock(symbol: String) -> Result<StockAnalysis, String> {
    let klines = crate::datasource::kline::fetch_daily_kline(&symbol, 250).await?;
    crate::quant::scorer::analyze(&klines).ok_or_else(|| {
        format!("K 线数据不足（{} 根，至少需 60 根），无法评分", klines.len())
    })
}
