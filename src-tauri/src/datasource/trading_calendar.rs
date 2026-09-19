//! 交易日历：由真实行情推断，不再依赖每年手工录入的交易所休市表。
//!
//! 证据链（按可信度降序）：
//! 1. **指数日 K 的日期序列** —— 上证指数（`sh000001`）不会停牌，它的每根日 K
//!    都是一次真实交易。一次请求 640 根可覆盖约两年半，跨年自动向前滚动，
//!    不需要任何人工维护，也不依赖用户本地是否每天更新数据。
//! 2. **盘中指数实时报价** —— 日 K 里还没有今天时，用一次实时报价判断今天是否开市：
//!    报价时间戳落在今天即开市；10:00 之后仍然只拿到上一个交易日的时间戳，
//!    说明今天根本没有行情 → 休市（节假日自动识别）。
//! 3. **交易所公告硬编码**（[`super::a_share_calendar`]）—— 行情通道全部不可用时的
//!    离线兜底，只覆盖它写明的年份。超出覆盖又拿不到任何行情证据时返回错误，
//!    调用方按「暂停」处理，绝不猜一个交易日去做撮合。
//!
//! 结论：有网就有日历。stockdb 是否更新、有没有人手填休市日，都不再影响判定。

use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, Timelike, Utc, Weekday};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{OnceLock, RwLock};

/// 充当日历的指数：上证指数，从不停止交易。
pub const CALENDAR_SYMBOL: &str = "sh000001";
/// 每次同步拉取的日 K 根数（约两年半，覆盖跨年判定）。
pub const CALENDAR_BARS: u32 = 640;
/// 低于这个数量说明日 K 残缺，不把它当作「覆盖区间」使用。
const MIN_TRUSTED_DATES: usize = 60;
/// 盘中探测的起点：09:25（含集合竞价，此时指数已有行情）。
const PROBE_FROM_MINUTE: u32 = 9 * 60 + 25;
/// 盘中探测的终点：15:05（收盘集合竞价结束之后）。
const PROBE_TO_MINUTE: u32 = 15 * 60 + 5;
/// 收盘后可以用「今天没有日 K」直接判定休市。
const AFTER_CLOSE_MINUTE: u32 = 15 * 60 + 10;
/// 开盘后多久仍拿不到今日行情才判定休市（避免网络抖动误判）。
const CLOSED_CONFIRM_MINUTE: u32 = 10 * 60;
/// 无新证据时的常规刷新周期。
const REFRESH_HOURS: i64 = 6;
/// 盘中未确认时的刷新周期（分钟）。
const SESSION_RETRY_MINUTES: i64 = 15;
/// 两次取证之间的最小间隔（分钟）：通道全挂时不刷屏式重试。
const MIN_ATTEMPT_MINUTES: i64 = 5;
/// 只有实时证据、缺少历史日 K 时，多久补一次历史。
const BACKFILL_RETRY_MINUTES: i64 = 30;

#[derive(Debug, Clone)]
pub struct CalendarSnapshot {
    /// 已知发生过交易的日期。
    pub dates: BTreeSet<NaiveDate>,
    /// 本次证据的采集时间（UTC）。
    pub synced_at: DateTime<Utc>,
    /// 证据来源（通道名）。
    pub source: String,
    /// 采集到的日 K 根数。
    pub bars: usize,
    /// 已确认「今天不开市」的日期。
    pub closed_confirmed: Option<NaiveDate>,
}

static SNAPSHOT: OnceLock<RwLock<Option<CalendarSnapshot>>> = OnceLock::new();
/// 上一次真正发起取证的时刻（Unix 秒），用于限制重试频率。
static LAST_ATTEMPT: AtomicI64 = AtomicI64::new(0);

fn slot() -> &'static RwLock<Option<CalendarSnapshot>> {
    SNAPSHOT.get_or_init(|| RwLock::new(None))
}

fn cst() -> FixedOffset {
    FixedOffset::east_opt(8 * 3600).expect("UTC+8 is a valid offset")
}

fn read<R>(f: impl FnOnce(Option<&CalendarSnapshot>) -> R) -> R {
    let guard = slot().read().unwrap_or_else(|error| error.into_inner());
    f(guard.as_ref())
}

fn write<R>(f: impl FnOnce(&mut Option<CalendarSnapshot>) -> R) -> R {
    let mut guard = slot().write().unwrap_or_else(|error| error.into_inner());
    f(&mut guard)
}

/// 安装一批交易日（来自指数日 K 的日期序列），覆盖旧快照。
///
/// `synced_at` 是这次证据的采集时间：调用方用当前时刻，测试用固定时刻。
pub fn install(
    dates: impl IntoIterator<Item = NaiveDate>,
    source: &str,
    bars: usize,
    synced_at: DateTime<Utc>,
) -> usize {
    let dates: BTreeSet<NaiveDate> = dates.into_iter().collect();
    let count = dates.len();
    write(|cell| {
        let closed = cell
            .as_ref()
            .and_then(|previous| previous.closed_confirmed)
            .filter(|date| !dates.contains(date));
        *cell = Some(CalendarSnapshot {
            dates,
            synced_at,
            source: source.to_string(),
            bars,
            closed_confirmed: closed,
        });
    });
    count
}

/// 记录一笔「今天确实有行情」的正面证据。
pub fn mark_session_open(date: NaiveDate) {
    write(|cell| {
        let snapshot = cell.get_or_insert_with(empty_snapshot);
        snapshot.dates.insert(date);
        snapshot.synced_at = Utc::now();
        if snapshot.closed_confirmed == Some(date) {
            snapshot.closed_confirmed = None;
        }
    });
}

/// 记录「今天经行情确认没有开市」。
pub fn mark_session_closed(date: NaiveDate) {
    write(|cell| {
        let snapshot = cell.get_or_insert_with(empty_snapshot);
        snapshot.closed_confirmed = Some(date);
        snapshot.synced_at = Utc::now();
    });
}

fn empty_snapshot() -> CalendarSnapshot {
    CalendarSnapshot {
        dates: BTreeSet::new(),
        synced_at: Utc::now(),
        source: "实时报价".to_string(),
        bars: 0,
        closed_confirmed: None,
    }
}

pub fn coverage() -> Option<(NaiveDate, NaiveDate)> {
    read(|cell| {
        let snapshot = cell?;
        let first = snapshot.dates.iter().next().copied()?;
        let last = snapshot.dates.iter().next_back().copied()?;
        Some((first, last))
    })
}

pub fn status_text() -> String {
    read(|cell| match cell {
        None => "交易日历尚未同步：正在用行情推断，暂时回退到交易所公告日历".to_string(),
        Some(snapshot) => match (snapshot.dates.iter().next(), snapshot.dates.iter().next_back()) {
            (Some(first), Some(last)) => format!(
                "交易日历 {} → {}（{} 个交易日，来源 {}，{} 根指数日K）",
                first, last, snapshot.dates.len(), snapshot.source, snapshot.bars
            ),
            _ => "交易日历为空，回退到交易所公告日历".to_string(),
        },
    })
}

/// 判定 `date` 是否为交易日。
///
/// `now` 为当前时刻，用于区分「历史日期」「今天」和「未来日期」的证据强度。
pub fn is_trading_day_at(now: DateTime<Utc>, date: NaiveDate) -> Result<bool, String> {
    if matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
        return Ok(false);
    }
    let local = now.with_timezone(&cst());
    let today = local.date_naive();
    let minutes = local.num_seconds_from_midnight() / 60;
    let verdict = read(|cell| {
        let snapshot = cell?;
        if snapshot.dates.contains(&date) {
            return Some(true);
        }
        if snapshot.closed_confirmed == Some(date) {
            return Some(false);
        }
        // 日 K 覆盖区间内却没有这一天 —— 就是休市（区间由真实成交日期构成）。
        if snapshot.dates.len() >= MIN_TRUSTED_DATES {
            let (first, last) = (
                snapshot.dates.iter().next().copied(),
                snapshot.dates.iter().next_back().copied(),
            );
            if let (Some(first), Some(last)) = (first, last) {
                if date >= first && date <= last {
                    return Some(false);
                }
            }
        }
        // 收盘之后当天日 K 必定已经产生；没有它说明今天没开市。
        let synced_today = snapshot.synced_at.with_timezone(&cst()).date_naive() == today;
        if date == today && synced_today && minutes >= AFTER_CLOSE_MINUTE {
            return Some(false);
        }
        None
    });
    match verdict {
        Some(value) => Ok(value),
        None => offline_fallback(date),
    }
}

/// 用当前时刻判定「现在（北京时间的今天）是否交易日」。
pub fn is_trading_day_now(now: DateTime<Utc>) -> Result<bool, String> {
    let date = now.with_timezone(&cst()).date_naive();
    is_trading_day_at(now, date)
}

/// 行情完全不可用时的兜底：交易所公告日历（只覆盖写明年份）。
fn offline_fallback(date: NaiveDate) -> Result<bool, String> {
    super::a_share_calendar::trading_day(date).map_err(|error| {
        format!("{error}；指数行情日历也尚未同步成功，无法用行情推断")
    })
}

/// 是否需要立刻发起一次同步。
pub fn needs_sync(now: DateTime<Utc>) -> bool {
    let local = now.with_timezone(&cst());
    let today = local.date_naive();
    let minutes = local.num_seconds_from_midnight() / 60;
    let since_attempt = now.timestamp() - LAST_ATTEMPT.load(Ordering::Acquire);
    if since_attempt < MIN_ATTEMPT_MINUTES * 60 {
        return false; // 刚试过，别刷屏
    }
    read(|cell| {
        let Some(snapshot) = cell else { return true };
        if snapshot.synced_at > now {
            return true; // 系统时间回拨，重新取证
        }
        let elapsed = now.signed_duration_since(snapshot.synced_at);
        if elapsed >= chrono::Duration::hours(REFRESH_HOURS) {
            return true;
        }
        // 只有实时证据、历史日 K 还没补上时，过一段时间再补一次。
        if snapshot.bars < MIN_TRUSTED_DATES
            && elapsed >= chrono::Duration::minutes(BACKFILL_RETRY_MINUTES)
        {
            return true;
        }
        if snapshot.dates.contains(&today) || snapshot.closed_confirmed == Some(today) {
            return false; // 今天的结论已经确定
        }
        (PROBE_FROM_MINUTE..=PROBE_TO_MINUTE).contains(&minutes)
            && elapsed >= chrono::Duration::minutes(SESSION_RETRY_MINUTES)
    })
}

/// 拉一次指数行情，把证据写进日历。
///
/// 两条通道互相独立：日 K 负责历史与跨年，实时报价负责「今天开不开市」。
/// 任何一条失败都不清空已有证据；日 K 失败时仍然尝试实时探测，避免
/// 「拿不到日 K → 判定休市 → 不敢请求行情」的死锁。
pub async fn sync_from_market(
    manager: &crate::datasource::DataSourceManager,
) -> Result<usize, String> {
    LAST_ATTEMPT.store(Utc::now().timestamp(), Ordering::Release);
    let kline_result =
        crate::datasource::kline::fetch_qfq_daily_kline_with_source(CALENDAR_SYMBOL, CALENDAR_BARS)
            .await;
    let mut count = 0;
    let mut failure = None;
    match kline_result {
        Ok((rows, source)) => {
            let dates = rows
                .iter()
                .filter_map(|bar| NaiveDate::parse_from_str(&bar.date, "%Y-%m-%d").ok())
                .collect::<Vec<_>>();
            if dates.len() < MIN_TRUSTED_DATES {
                failure = Some(format!(
                    "指数日K仅返回 {} 根，样本不足，保留上一次交易日历",
                    dates.len()
                ));
            } else {
                count = install(dates, source.label(), rows.len(), Utc::now());
                log::info!(
                    "交易日历已由行情推断：{}（{}）",
                    status_text(),
                    source.label()
                );
            }
        }
        Err(error) => failure = Some(format!("指数日K获取失败：{error}")),
    }
    if let Some(reason) = &failure {
        log::warn!("[calendar] {reason}");
    }
    probe_today(manager).await;
    match (failure, count) {
        (None, count) => Ok(count),
        (Some(reason), 0) => Err(reason),
        (Some(_), count) => Ok(count),
    }
}

/// 盘中确认「今天到底开不开市」，只在日 K 尚未覆盖今天时探测。
async fn probe_today(manager: &crate::datasource::DataSourceManager) {
    let now = Utc::now();
    let local = now.with_timezone(&cst());
    let today = local.date_naive();
    let minutes = local.num_seconds_from_midnight() / 60;
    if !(PROBE_FROM_MINUTE..=PROBE_TO_MINUTE).contains(&minutes) {
        return;
    }
    let uncovered = read(|cell| {
        cell.map(|snapshot| !snapshot.dates.contains(&today))
            .unwrap_or(true)
    });
    if !uncovered {
        return;
    }
    let Some(source) = manager.active_source() else {
        return;
    };
    let quotes = match source
        .fetch_realtime(&[CALENDAR_SYMBOL.to_string()], "CN")
        .await
    {
        Ok(quotes) => quotes,
        Err(error) => {
            log::warn!("交易日历：指数实时报价探测失败，今天是否开市仍未确定（{error}）");
            return;
        }
    };
    let Some(quote) = quotes
        .iter()
        .find(|quote| quote.timestamp > 0 && quote.code.ends_with("000001"))
    else {
        return;
    };
    let quote_date = DateTime::from_timestamp(quote.timestamp, 0)
        .map(|stamp| stamp.with_timezone(&cst()).date_naive());
    match quote_date {
        Some(date) if date == today => {
            log::info!("交易日历：指数 {today} 有实时行情，确认今天开市");
            mark_session_open(today);
        }
        // 开盘一小时之后仍然只有旧时间戳 → 今天没有行情，判定休市。
        Some(stale) if minutes >= CLOSED_CONFIRM_MINUTE => {
            log::info!("交易日历：指数最新报价仍停留在 {stale}，判定 {today} 休市");
            mark_session_closed(today);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn date(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    fn utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, min, 0).single().unwrap()
    }

    fn weekday(value: NaiveDate) -> bool {
        !matches!(value.weekday(), Weekday::Sat | Weekday::Sun)
    }

    /// 快照是进程级状态，全部场景放在同一个测试里顺序验证，避免并行互相覆盖。
    #[test]
    fn market_evidence_drives_the_calendar() {
        // ① 同步前：回退到交易所公告日历，2026 年内仍可用。
        assert_eq!(
            is_trading_day_at(utc(2026, 9, 21, 2, 0), date("2026-09-21")),
            Ok(true)
        );
        assert_eq!(
            is_trading_day_at(utc(2026, 9, 25, 2, 0), date("2026-09-25")),
            Ok(false)
        );
        // ② 公告日历不覆盖、行情也没同步 → 明确报错而不是猜一个交易日。
        assert!(is_trading_day_at(utc(2027, 1, 4, 2, 0), date("2027-01-04")).is_err());

        // ③ 指数日 K 提供 2026-09 → 2027-01 的真实成交日期，天然跨越年度边界。
        let mut dates: Vec<NaiveDate> = Vec::new();
        let mut cursor = date("2026-09-01");
        while cursor <= date("2027-01-29") {
            if weekday(cursor) {
                dates.push(cursor);
            }
            cursor = cursor.succ_opt().expect("date range is finite");
        }
        // 挖掉中秋与国庆，模拟交易所真实休市。
        dates.retain(|value| !(value >= &date("2026-09-25") && value <= &date("2026-09-27")));
        dates.retain(|value| !(value >= &date("2026-10-01") && value <= &date("2026-10-07")));
        install(dates, "fixture", 100, utc(2026, 9, 30, 7, 0));

        assert_eq!(
            is_trading_day_at(utc(2027, 1, 4, 2, 0), date("2027-01-04")),
            Ok(true),
            "跨年后依然由行情判定为交易日"
        );
        assert_eq!(
            is_trading_day_at(utc(2026, 10, 1, 2, 0), date("2026-10-01")),
            Ok(false),
            "覆盖区间内的缺口即休市"
        );
        assert_eq!(
            is_trading_day_at(utc(2026, 9, 28, 2, 0), date("2026-09-28")),
            Ok(true)
        );
        assert_eq!(
            is_trading_day_at(utc(2026, 9, 26, 2, 0), date("2026-09-26")),
            Ok(false),
            "周末先于任何证据判定"
        );
        // 超出行情覆盖范围又没有当日证据 → 仍然报错，不猜。
        assert!(is_trading_day_at(utc(2027, 3, 1, 2, 0), date("2027-03-01")).is_err());

        // ④ 盘中：只拿到上一个交易日的时间戳 → 判定今天休市。
        mark_session_closed(date("2027-10-01"));
        assert_eq!(
            is_trading_day_at(utc(2027, 10, 1, 3, 0), date("2027-10-01")),
            Ok(false)
        );

        // ⑤ 收盘后同步过、当天仍没有日 K → 今天没有开市。
        install(
            vec![date("2026-09-18"), date("2026-09-21")],
            "fixture",
            2,
            utc(2027, 10, 8, 7, 0),
        );
        assert_eq!(
            is_trading_day_at(utc(2027, 10, 8, 9, 0), date("2027-10-08")),
            Ok(false)
        );

        // ⑥ 盘中拿到今天时间戳的指数行情 → 确认开市（跨年也不用改代码）。
        mark_session_open(date("2027-10-08"));
        assert_eq!(
            is_trading_day_at(utc(2027, 10, 8, 9, 0), date("2027-10-08")),
            Ok(true)
        );

        // ⑦ 取证收敛：刚同步过就不再重复请求，超过刷新周期才重新取证。
        let now = Utc::now();
        let today = now.with_timezone(&cst()).date_naive();
        install(vec![today], "fixture", 1, now);
        assert!(!needs_sync(now));
        install(
            vec![today],
            "fixture",
            1,
            now - chrono::Duration::hours(REFRESH_HOURS + 1),
        );
        assert!(needs_sync(now));
    }
}

