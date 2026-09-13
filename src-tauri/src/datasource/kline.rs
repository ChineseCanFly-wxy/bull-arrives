// src-tauri/src/datasource/kline.rs
//! 日 K 线的**多通道适配器**（项目内唯一的日K入口）。
//!
//! # 为什么需要多通道
//!
//! 实测发现东方财富的 `push2his.../api/qt/stock/kline/get` 在部分网络下
//! **整条路径被阻断**（HTTP 000，TLS 重协商后断开），而同域的
//! `push2.../api/qt/stock/get` 却正常 —— 这与 `clist` 被阻断是同一类问题。
//! 只依赖东财会让「量化评分 / 推荐榜 / 智能监控」三个功能一起失效。
//!
//! 因此改为**三通道依次回退**：
//!
//! | 顺序 | 通道 | 复权 | 成交量单位 | 实测 |
//! |---|---|---|---|---|
//! | 1 | 腾讯 `web.ifzq.gtimg.cn` | **前复权(qfq)** | 手 | ✅ 含沪/深/创业/科创/北交所 |
//! | 2 | 新浪 `money.finance.sina.com.cn` | 不复权 | 股（需 ÷100） | ✅ 上限 3000 条 |
//! | 3 | 东财 `push2his.eastmoney.com` | 前复权 | 手 | ⚠️ 部分网络被阻断 |
//!
//! 优先腾讯是因为它的**字段口径与原来的东财完全一致**（前复权 + 成交量按手），
//! 替换后 `quant::indicators` / `quant::scorer` 的计算结果不需要任何调整。
//!
//! # 已实测的接口细节（勿凭记忆改）
//!
//! 腾讯：`?param={symbol},day,,,{count},qfq`
//! - 返回 `data.<symbol>.qfqday`，**但北交所有时只给 `day`**，两个 key 都要试
//! - 行格式：`[日期, 开, 收, 高, 低, 成交量(手)]` —— **收盘在开盘后，不是 OHLC**
//! - `count` 超过约 640 会被服务端截断
//!
//! 新浪：`?symbol={symbol}&scale=240&ma=no&datalen={count}`
//! - 返回裸数组：`[{day, open, high, low, close, volume}]`
//! - `volume` 单位是**股**，换算成手要 ÷100

use std::time::Duration;

use crate::domain::KLineData;

/// 单次请求超时
const KLINE_TIMEOUT: Duration = Duration::from_secs(20);

/// 请求条数上限。腾讯在约 640 根后会截断，这里留出余量。
const MAX_COUNT: u32 = 640;

/// 一个通道至少要给出这么多根K线才算「够用」。
///
/// 低于此值说明该通道对这只标的覆盖不足：**实测腾讯对北交所只返回 1–2 根**，
/// 而新浪能返回完整 250 根。所以不能只看「非空」，必须看「够不够」，
/// 不够就继续试下一个通道；若全都不够，则返回根数最多的那个（可能是次新股）。
const MIN_USABLE_BARS: usize = 60;

/// 腾讯前复权日K
const TENCENT_URL: &str = "https://web.ifzq.gtimg.cn/appstock/app/fqkline/get";

/// 新浪日K（不复权）
const SINA_URL: &str =
    "https://money.finance.sina.com.cn/quotes_service/api/json_v2.php/CN_MarketData.getKLineData";

/// 哪个通道最终提供了数据（供日志与 UI 提示）
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KlineSource {
    Tencent,
    Sina,
    Eastmoney,
}

impl KlineSource {
    pub fn label(self) -> &'static str {
        match self {
            KlineSource::Tencent => "腾讯财经",
            KlineSource::Sina => "新浪财经",
            KlineSource::Eastmoney => "东方财富",
        }
    }
}

/// 6 位代码 + 板块 → 项目内部完整符号（`sh600519` / `sz000001` / `bj920xxx`）。
///
/// 与前端 `toFullSymbol` 同一套规则。
pub fn full_symbol_from(code: &str, board: crate::datasource::eastmoney_universe::Board) -> String {
    use crate::datasource::eastmoney_universe::Board;
    match board {
        Board::ShMain | Board::Star => format!("sh{code}"),
        Board::SzMain | Board::ChiNext => format!("sz{code}"),
        Board::Bse => format!("bj{code}"),
        Board::BShare | Board::Other => format!("sz{code}"),
    }
}

fn val_f64(v: &serde_json::Value) -> f64 {
    match v {
        serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0),
        serde_json::Value::String(s) => s.trim().parse::<f64>().unwrap_or(0.0),
        _ => 0.0,
    }
}

fn val_str(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// 解析腾讯日K。行格式 `[日期, 开, 收, 高, 低, 量]`。
pub fn parse_tencent(text: &str, symbol: &str) -> Result<Vec<KLineData>, String> {
    let json: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("腾讯日K JSON 解析失败: {e}"))?;
    let node = json
        .get("data")
        .and_then(|d| d.get(symbol))
        .ok_or_else(|| format!("腾讯日K返回中无 {symbol} 节点"))?;

    // 前复权优先；北交所等标的只返回不复权的 `day`
    let arr = node
        .get("qfqday")
        .or_else(|| node.get("day"))
        .and_then(|v| v.as_array())
        .ok_or_else(|| "腾讯日K返回中无 qfqday/day 数组".to_string())?;

    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let Some(row) = item.as_array() else { continue };
        if row.len() < 6 {
            continue;
        }
        let date = val_str(&row[0]);
        if date.is_empty() {
            continue;
        }
        let open = val_f64(&row[1]);
        let close = val_f64(&row[2]);
        let high = val_f64(&row[3]);
        let low = val_f64(&row[4]);
        let volume = val_f64(&row[5]);
        if close <= 0.0 {
            continue;
        }
        out.push(KLineData {
            date,
            open,
            high,
            low,
            close,
            volume: volume.max(0.0) as u64,
            turnover: 0.0,
        });
    }
    Ok(out)
}

/// 解析新浪日K。`volume` 是股，换算成手。
pub fn parse_sina(text: &str) -> Result<Vec<KLineData>, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed == "null" {
        return Ok(Vec::new());
    }
    let arr: Vec<serde_json::Value> =
        serde_json::from_str(trimmed).map_err(|e| format!("新浪日K JSON 解析失败: {e}"))?;

    let mut out = Vec::with_capacity(arr.len());
    for item in &arr {
        let date = val_str(&item["day"]);
        if date.is_empty() {
            continue;
        }
        let close = val_f64(&item["close"]);
        if close <= 0.0 {
            continue;
        }
        out.push(KLineData {
            date,
            open: val_f64(&item["open"]),
            high: val_f64(&item["high"]),
            low: val_f64(&item["low"]),
            close,
            // 股 → 手
            volume: (val_f64(&item["volume"]) / 100.0).max(0.0) as u64,
            turnover: 0.0,
        });
    }
    Ok(out)
}

async fn tencent_daily(
    client: &reqwest::Client,
    symbol: &str,
    count: u32,
) -> Result<Vec<KLineData>, String> {
    let url = format!("{TENCENT_URL}?param={symbol},day,,,{count},qfq");
    let text = client
        .get(&url)
        .header("Referer", "https://gu.qq.com/")
        .timeout(KLINE_TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?
        .text()
        .await
        .map_err(|e| format!("读取响应失败: {e}"))?;
    parse_tencent(&text, symbol)
}

async fn sina_daily(
    client: &reqwest::Client,
    symbol: &str,
    count: u32,
) -> Result<Vec<KLineData>, String> {
    let url = format!("{SINA_URL}?symbol={symbol}&scale=240&ma=no&datalen={count}");
    let text = client
        .get(&url)
        .header("Referer", "https://finance.sina.com.cn/")
        .timeout(KLINE_TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?
        .text()
        .await
        .map_err(|e| format!("读取响应失败: {e}"))?;
    parse_sina(&text)
}

/// 拉取日K（多通道自动回退）。
///
/// 依次尝试腾讯 → 新浪 → 东财，任一通道拿到非空数据即返回。
/// 全部失败时返回聚合后的错误，便于定位到底是哪个通道出问题。
pub async fn fetch_daily_kline(symbol: &str, count: u32) -> Result<Vec<KLineData>, String> {
    fetch_daily_kline_with_source(symbol, count)
        .await
        .map(|(rows, _)| rows)
}

/// 记录候选结果：只在根数更多时替换。
/// 返回 `Some(结果)` 表示「已够用，可以立即返回」；`None` 表示继续试下一个通道。
fn keep_best(
    best: &mut Option<(Vec<KLineData>, KlineSource)>,
    rows: Vec<KLineData>,
    source: KlineSource,
) -> Option<(Vec<KLineData>, KlineSource)> {
    if rows.is_empty() {
        return None;
    }
    let enough = rows.len() >= MIN_USABLE_BARS;
    let better = best.as_ref().map_or(true, |(cur, _)| rows.len() > cur.len());
    if better {
        *best = Some((rows, source));
    }
    if enough {
        best.take()
    } else {
        None
    }
}

/// 同 [`fetch_daily_kline`]，额外返回数据来自哪个通道。
pub async fn fetch_daily_kline_with_source(
    symbol: &str,
    count: u32,
) -> Result<(Vec<KLineData>, KlineSource), String> {
    let count = count.clamp(1, MAX_COUNT);
    let client = crate::datasource::eastmoney_universe::universe_client();
    let mut errors: Vec<String> = Vec::new();
    let mut best: Option<(Vec<KLineData>, KlineSource)> = None;

    // 1) 腾讯（前复权，口径与原东财一致，绝大多数标的走这条）
    match tencent_daily(client, symbol, count).await {
        Ok(rows) => {
            if let Some(hit) = keep_best(&mut best, rows, KlineSource::Tencent) {
                return Ok(hit);
            }
        }
        Err(error) => {
            log::warn!("腾讯日K失败（{symbol}），尝试新浪：{error}");
            errors.push(format!("腾讯：{error}"));
        }
    }

    // 2) 新浪（不复权，但覆盖最全 —— 北交所只有这里有数据）
    match sina_daily(client, symbol, count).await {
        Ok(rows) => {
            if let Some(hit) = keep_best(&mut best, rows, KlineSource::Sina) {
                return Ok(hit);
            }
        }
        Err(error) => {
            log::warn!("新浪日K失败（{symbol}），尝试东方财富：{error}");
            errors.push(format!("新浪：{error}"));
        }
    }

    // 3) 东财（在部分网络下会被阻断，保留以兼容其他环境）
    match crate::datasource::eastmoney_kline::fetch_daily_kline(symbol, count).await {
        Ok(rows) => {
            if let Some(hit) = keep_best(&mut best, rows, KlineSource::Eastmoney) {
                return Ok(hit);
            }
        }
        Err(error) => errors.push(format!("东方财富：{error}")),
    }

    // 三个通道都不够 60 根：可能是次新股，返回根数最多的那个，别浪费已拿到的数据
    if let Some(fallback) = best {
        log::info!(
            "{symbol} 各通道K线均不足 {MIN_USABLE_BARS} 根，返回 {}（{} 根）",
            fallback.1.label(),
            fallback.0.len()
        );
        return Ok(fallback);
    }

    Err(format!(
        "日K 获取失败（已尝试腾讯 / 新浪 / 东方财富）：{}",
        errors.join("；")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datasource::eastmoney_universe::Board;

    #[test]
    fn parses_tencent_qfqday_in_open_close_high_low_order() {
        // 真实响应节选：贵州茅台 2026-09-11
        let text = r#"{"code":0,"msg":"","data":{"sh600519":{
            "qfqday":[["2026-09-10","1291.000","1285.130","1294.990","1282.000","18900.000"],
                      ["2026-09-11","1285.150","1275.160","1286.150","1263.010","34801.000"]],
            "qt":{}}}}"#;
        let rows = parse_tencent(text, "sh600519").unwrap();
        assert_eq!(rows.len(), 2);
        let last = &rows[1];
        assert_eq!(last.date, "2026-09-11");
        // 第 2 位是「收盘」而不是「最高」—— 这是最容易搞错的地方
        assert!((last.open - 1285.150).abs() < 1e-9);
        assert!((last.close - 1275.160).abs() < 1e-9);
        assert!((last.high - 1286.150).abs() < 1e-9);
        assert!((last.low - 1263.010).abs() < 1e-9);
        assert_eq!(last.volume, 34801);
    }

    #[test]
    fn tencent_falls_back_to_day_key_for_bse() {
        // 北交所在带 qfq 参数时也只返回 `day`
        let text = r#"{"code":0,"data":{"bj920000":{
            "day":[["2026-09-11","13.88","13.55","13.96","13.49","5428"]]}}}"#;
        let rows = parse_tencent(text, "bj920000").unwrap();
        assert_eq!(rows.len(), 1);
        assert!((rows[0].close - 13.55).abs() < 1e-9);
        assert_eq!(rows[0].volume, 5428);
    }

    #[test]
    fn parses_sina_and_converts_volume_to_lots() {
        let text = r#"[{"day":"2026-09-10","open":"1291.000","high":"1294.990",
                        "low":"1282.000","close":"1285.130","volume":"1890022"},
                       {"day":"2026-09-11","open":"1285.150","high":"1286.150",
                        "low":"1263.010","close":"1275.160","volume":"3480100"}]"#;
        let rows = parse_sina(text).unwrap();
        assert_eq!(rows.len(), 2);
        assert!((rows[0].close - 1285.130).abs() < 1e-9);
        // 股 → 手
        assert_eq!(rows[1].volume, 34801);
    }

    #[test]
    fn empty_and_null_bodies_are_not_errors() {
        assert!(parse_sina("").unwrap().is_empty());
        assert!(parse_sina("null").unwrap().is_empty());
    }

    #[test]
    fn malformed_rows_are_skipped_not_fatal() {
        let text = r#"{"data":{"sh600519":{"qfqday":[
            ["2026-09-11","1285.150","1275.160","1286.150","1263.010","34801.000"],
            ["bad"],
            ["2026-09-12","1","0","1","1","1"]]}}}"#;
        let rows = parse_tencent(text, "sh600519").unwrap();
        // 只有第一条有效：第二条长度不足，第三条收盘为 0
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn full_symbol_matches_exchange_rules() {
        assert_eq!(full_symbol_from("600519", Board::ShMain), "sh600519");
        assert_eq!(full_symbol_from("688111", Board::Star), "sh688111");
        assert_eq!(full_symbol_from("000001", Board::SzMain), "sz000001");
        assert_eq!(full_symbol_from("300750", Board::ChiNext), "sz300750");
        assert_eq!(full_symbol_from("920000", Board::Bse), "bj920000");
    }

    fn bars(n: usize) -> Vec<KLineData> {
        (0..n)
            .map(|i| KLineData {
                date: format!("2026-01-{:02}", i % 28 + 1),
                open: 10.0,
                high: 11.0,
                low: 9.0,
                close: 10.5,
                volume: 100,
                turnover: 0.0,
            })
            .collect()
    }

    #[test]
    fn short_result_does_not_stop_the_fallback_chain() {
        // 腾讯对北交所只给 2 根 —— 必须继续试下一个通道，而不是当成成功
        let mut best = None;
        assert!(keep_best(&mut best, bars(2), KlineSource::Tencent).is_none());
        assert_eq!(best.as_ref().unwrap().1, KlineSource::Tencent);

        // 新浪给 250 根 —— 立即返回
        let hit = keep_best(&mut best, bars(250), KlineSource::Sina).unwrap();
        assert_eq!(hit.1, KlineSource::Sina);
        assert_eq!(hit.0.len(), 250);
    }

    #[test]
    fn short_results_keep_the_longest_one() {
        let mut best = None;
        assert!(keep_best(&mut best, Vec::new(), KlineSource::Tencent).is_none());
        assert!(keep_best(&mut best, bars(3), KlineSource::Tencent).is_none());
        // 更短的候选不覆盖更长的
        assert!(keep_best(&mut best, bars(2), KlineSource::Sina).is_none());
        let kept = best.unwrap();
        assert_eq!(kept.1, KlineSource::Tencent);
        assert_eq!(kept.0.len(), 3);
    }
}
