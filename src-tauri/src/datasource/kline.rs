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

use std::{
    collections::BTreeMap,
    sync::OnceLock,
    time::{Duration, Instant},
};

use crate::db::Database;
use crate::domain::HistoryMeta;
use crate::domain::KLineData;

#[derive(Debug, Clone)]
pub struct HistoryData {
    pub klines: Vec<KLineData>,
    /// 与 `klines` 同日期的未复权日 K；仅本地 stockdb 能提供。
    pub raw_klines: Option<Vec<KLineData>>,
    pub meta: HistoryMeta,
}

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

/// 上证指数实际日线充当交易日历；成功结果缓存 6 小时，避免每只股票重复探测。
static TRADING_CALENDAR: OnceLock<tokio::sync::Mutex<Option<(Instant, String)>>> = OnceLock::new();

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
    parse_tencent_mode(text, symbol, true)
}

fn parse_tencent_mode(text: &str, symbol: &str, allow_raw: bool) -> Result<Vec<KLineData>, String> {
    let json: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("腾讯日K JSON 解析失败: {e}"))?;
    let node = json
        .get("data")
        .and_then(|d| d.get(symbol))
        .ok_or_else(|| format!("腾讯日K返回中无 {symbol} 节点"))?;

    // 前复权优先；北交所等标的只返回不复权的 `day`
    let arr = node
        .get("qfqday")
        .or_else(|| allow_raw.then(|| node.get("day")).flatten())
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            if allow_raw {
                "腾讯日K返回中无 qfqday/day 数组"
            } else {
                "腾讯日K未提供前复权 qfqday"
            }
            .to_string()
        })?;

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

async fn tencent_daily_qfq(
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
    parse_tencent_mode(&text, symbol, false)
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
    let better = best
        .as_ref()
        .map_or(true, |(cur, _)| rows.len() > cur.len());
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

/// 量化/回测专用的在线后路：只接受明确的前复权结果。
/// 新浪和腾讯 `day` 都是未复权，能画图但不能静默进入回测。
pub async fn fetch_qfq_daily_kline_with_source(
    symbol: &str,
    count: u32,
) -> Result<(Vec<KLineData>, KlineSource), String> {
    let count = count.clamp(1, MAX_COUNT);
    let client = crate::datasource::eastmoney_universe::universe_client();
    let mut errors = Vec::new();
    let mut best = None;
    match tencent_daily_qfq(client, symbol, count).await {
        Ok(rows) => {
            if let Some(hit) = keep_best(&mut best, rows, KlineSource::Tencent) {
                return Ok(hit);
            }
        }
        Err(error) => errors.push(format!("腾讯前复权：{error}")),
    }
    match crate::datasource::eastmoney_kline::fetch_daily_kline(symbol, count).await {
        Ok(rows) => {
            if let Some(hit) = keep_best(&mut best, rows, KlineSource::Eastmoney) {
                return Ok(hit);
            }
        }
        Err(error) => errors.push(format!("东方财富前复权：{error}")),
    }
    best.ok_or_else(|| format!("前复权日K获取失败：{}", errors.join("；")))
}

fn history_config(
    db: &Database,
) -> Result<Option<crate::datasource::history::LocalHistoryConfig>, String> {
    let enabled = db
        .get_setting("local_history_enabled")
        .map_err(|e| e.to_string())?
        .as_deref()
        != Some("0");
    if !enabled {
        return Ok(None);
    }
    let url = db
        .get_setting("local_history_url")
        .map_err(|e| e.to_string())?
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "http://127.0.0.1:7899".to_string());
    Ok(Some(crate::datasource::history::LocalHistoryConfig::new(
        url,
    )))
}

/// 批量历史任务的单次可用性探测。先探测再扇出，避免本地服务停掉时让
/// 120 个候选各自等待连接超时。
pub async fn probe_local_history(db: &Database) -> Result<(), String> {
    let config = history_config(db)?.ok_or_else(|| "本地历史数据已关闭".to_string())?;
    crate::datasource::history::probe(&config).await.map(|_| ())
}

fn lag_warning(end_date: Option<&str>) -> Option<String> {
    let end =
        end_date.and_then(|value| chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())?;
    let days = (chrono::Local::now().date_naive() - end).num_days();
    (days > 10).then(|| format!("本地历史数据截止到 {end}，可能尚未更新"))
}

fn latest_closed_date_at(rows: &[KLineData], now: chrono::DateTime<chrono::Utc>) -> Option<String> {
    let offset = chrono::FixedOffset::east_opt(8 * 3600).expect("UTC+8 is valid");
    let local = now.with_timezone(&offset);
    let today = local.date_naive().format("%Y-%m-%d").to_string();
    let include_today =
        local.time() >= chrono::NaiveTime::from_hms_opt(15, 10, 0).expect("15:10 is valid");
    rows.iter()
        .map(|row| row.date.as_str())
        .filter(|date| *date < today.as_str() || (include_today && *date == today.as_str()))
        .max()
        .map(str::to_owned)
}

async fn latest_closed_trading_date() -> Result<String, String> {
    let cache = TRADING_CALENDAR.get_or_init(|| tokio::sync::Mutex::new(None));
    let mut guard = cache.lock().await;
    if let Some((fetched_at, date)) = guard.as_ref() {
        if fetched_at.elapsed() < Duration::from_secs(6 * 60 * 60) {
            return Ok(date.clone());
        }
    }
    let (rows, source) = fetch_qfq_daily_kline_with_source("sh000001", 120).await?;
    let date = latest_closed_date_at(&rows, chrono::Utc::now())
        .ok_or_else(|| "交易日历没有已收盘日期".to_string())?;
    log::info!("交易日历更新至 {date}（{}）", source.label());
    *guard = Some((Instant::now(), date.clone()));
    Ok(date)
}

fn merge_daily_unique(local: Vec<KLineData>, online: Vec<KLineData>) -> (Vec<KLineData>, usize) {
    let local_len = local.len();
    let mut by_date: BTreeMap<String, KLineData> = local
        .into_iter()
        .map(|row| (row.date.clone(), row))
        .collect();
    for row in online {
        // 本地历史口径优先；在线只补不存在的日期。
        by_date.entry(row.date.clone()).or_insert(row);
    }
    let rows: Vec<_> = by_date.into_values().collect();
    let added = rows.len().saturating_sub(local_len);
    (rows, added)
}

/// 统一历史入口。单股允许在线短区间后路；批量任务必须传 `allow_online=false`，
/// 防止本地不可用时退化成全市场逐只联网。
pub async fn fetch_history(
    db: &Database,
    symbol: &str,
    count: Option<usize>,
    allow_online: bool,
) -> Result<HistoryData, String> {
    let local = history_config(db)?;
    let local_error = if let Some(config) = local {
        match crate::datasource::history::fetch_daily(&config, symbol, None, None).await {
            Ok(mut result) if !result.klines.is_empty() => {
                let mut raw_klines = result.raw_klines;
                let mut source_label = "本地 stockdb".to_string();
                let mut warning = None;
                if allow_online {
                    match latest_closed_trading_date().await {
                        Ok(latest)
                            if result
                                .end_date
                                .as_deref()
                                .is_some_and(|end| end < latest.as_str()) =>
                        {
                            match fetch_qfq_daily_kline_with_source(symbol, 120).await {
                                Ok((online, source)) => {
                                    let end = result.end_date.as_deref().unwrap_or_default();
                                    let online = online
                                        .into_iter()
                                        .filter(|row| {
                                            row.date.as_str() > end
                                                && row.date.as_str() <= latest.as_str()
                                        })
                                        .collect();
                                    let (merged, added) = merge_daily_unique(result.klines, online);
                                    result.klines = merged;
                                    if added > 0 {
                                        source_label =
                                            format!("本地 stockdb + {}增量", source.label());
                                    }
                                }
                                Err(error) => {
                                    warning = Some(format!("在线日线增量更新失败：{error}"))
                                }
                            }
                        }
                        Ok(_) => {}
                        Err(error) => {
                            warning = Some(format!("交易日历不可用，未检查在线增量：{error}"))
                        }
                    }
                }
                if let Some(limit) = count {
                    if result.klines.len() > limit {
                        result.klines.drain(..result.klines.len() - limit);
                    }
                }
                let kept_dates: std::collections::HashSet<_> =
                    result.klines.iter().map(|row| row.date.as_str()).collect();
                raw_klines.retain(|row| kept_dates.contains(row.date.as_str()));
                let start_date = result.klines.first().map(|row| row.date.clone());
                let end_date = result.klines.last().map(|row| row.date.clone());
                warning = warning.or_else(|| lag_warning(end_date.as_deref()));
                let stale = warning.is_some();
                let bars = result.klines.len();
                return Ok(HistoryData {
                    klines: result.klines,
                    raw_klines: Some(raw_klines),
                    meta: HistoryMeta {
                        source: "local_stockdb".into(),
                        source_label,
                        start_date,
                        end_date,
                        bars,
                        adjustment: "qfq".into(),
                        stale,
                        warning,
                    },
                });
            }
            Ok(_) => Some("本地历史数据为空".to_string()),
            Err(error) => Some(error),
        }
    } else {
        Some("本地历史数据已关闭".to_string())
    };

    if !allow_online {
        return Err(format!(
            "{}；批量任务不会逐只回退在线历史",
            local_error.unwrap_or_else(|| "本地历史不可用".into())
        ));
    }
    let online_count = count.unwrap_or(MAX_COUNT as usize).min(MAX_COUNT as usize) as u32;
    let (klines, source) = fetch_qfq_daily_kline_with_source(symbol, online_count).await?;
    let start_date = klines.first().map(|row| row.date.clone());
    let end_date = klines.last().map(|row| row.date.clone());
    let bars = klines.len();
    Ok(HistoryData {
        klines,
        raw_klines: None,
        meta: HistoryMeta {
            source: "online".into(),
            source_label: source.label().into(),
            start_date,
            end_date,
            bars,
            adjustment: "qfq".into(),
            stale: false,
            warning: local_error.map(|error| format!("本地历史不可用，已回退在线短区间：{error}")),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datasource::eastmoney_universe::Board;
    use chrono::TimeZone;

    fn bar(date: &str, close: f64) -> KLineData {
        KLineData {
            date: date.into(),
            open: close,
            high: close,
            low: close,
            close,
            volume: 1,
            turnover: 0.0,
        }
    }

    #[test]
    fn calendar_skips_unclosed_day_and_increment_merge_deduplicates_dates() {
        let calendar = vec![bar("2026-09-17", 1.0), bar("2026-09-18", 2.0)];
        let before_close = chrono::Utc
            .with_ymd_and_hms(2026, 9, 18, 6, 0, 0)
            .single()
            .unwrap();
        assert_eq!(
            latest_closed_date_at(&calendar, before_close).as_deref(),
            Some("2026-09-17")
        );

        let (merged, added) = merge_daily_unique(
            vec![bar("2026-09-17", 10.0)],
            vec![bar("2026-09-17", 99.0), bar("2026-09-18", 11.0)],
        );
        assert_eq!(added, 1);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].close, 10.0, "重复日期应保留本地口径");
    }

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
    fn quant_history_rejects_unadjusted_tencent_day() {
        let text = r#"{"code":0,"data":{"bj920000":{
            "day":[["2026-09-11","13.88","13.55","13.96","13.49","5428"]]}}}"#;
        assert!(parse_tencent_mode(text, "bj920000", false)
            .unwrap_err()
            .contains("前复权"));
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

    #[tokio::test]
    #[ignore = "requires local stockdb and public qfq endpoints"]
    async fn local_history_is_deeper_and_matches_online_samples() {
        let config = crate::datasource::history::LocalHistoryConfig::new("http://127.0.0.1:7899");
        for (symbol, require_deeper) in [
            ("sh600519", true),
            ("sz000001", true),
            ("sh600000", true),
            ("bj920000", false),
        ] {
            let local = crate::datasource::history::fetch_daily(&config, symbol, None, None)
                .await
                .unwrap_or_else(|error| panic!("{symbol} local: {error}"));
            let (online, source) = if require_deeper {
                fetch_qfq_daily_kline_with_source(symbol, MAX_COUNT).await
            } else {
                fetch_daily_kline_with_source(symbol, MAX_COUNT).await
            }
            .unwrap_or_else(|error| panic!("{symbol} online: {error}"));
            if require_deeper {
                assert!(
                    local.klines.len() > online.len(),
                    "{symbol}: local={} online={}",
                    local.klines.len(),
                    online.len()
                );
            }
            let pair = online
                .iter()
                .rev()
                .find_map(|right| {
                    local
                        .klines
                        .iter()
                        .rev()
                        .find(|left| left.date == right.date)
                        .map(|left| (left, right))
                })
                .unwrap_or_else(|| panic!("{symbol}: no overlapping date"));
            let price_tolerance = (pair.1.close * 0.002).max(0.02);
            for (left, right) in [
                (pair.0.open, pair.1.open),
                (pair.0.high, pair.1.high),
                (pair.0.low, pair.1.low),
                (pair.0.close, pair.1.close),
            ] {
                assert!(
                    (left - right).abs() <= price_tolerance,
                    "{symbol} {} price mismatch: {left} vs {right}",
                    pair.0.date
                );
            }
            let volume_delta = pair.0.volume.abs_diff(pair.1.volume) as f64;
            assert!(
                volume_delta <= (pair.1.volume as f64 * 0.03).max(1.0),
                "{symbol} {} volume mismatch: {} vs {}",
                pair.0.date,
                pair.0.volume,
                pair.1.volume
            );
            eprintln!(
                "{symbol}: local={} online={} ({:?}), sample={}",
                local.klines.len(),
                online.len(),
                source,
                pair.0.date
            );
        }
    }
}
