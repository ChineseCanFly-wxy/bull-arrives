//! 交易所公告休市日（**离线兜底**，不再是主判据）。
//!
//! 日常判定走 [`super::trading_calendar`]：由上证指数日 K 与盘中实时报价推断，
//! 跨年自动滚动，不需要每年改代码。只有行情通道全部不可用时才会落到这里，
//! 因此它只覆盖写明的年份，超出范围就返回错误，由调用方按「暂停」处理 ——
//! 宁可暂停，也不猜一个交易日去做撮合。
//!
//! Source: https://www.sse.com.cn/disclosure/dealinstruc/closed/c/c_20251222_10802510.shtml
use chrono::{DateTime, Datelike, NaiveDate, Timelike, Utc, Weekday};
pub const VERIFIED_YEAR: i32 = 2026;
pub fn trading_day(date: NaiveDate) -> Result<bool, String> {
    if date.year() != VERIFIED_YEAR {
        return Err(format!(
            "尚未载入 {} 年 A 股交易所休市日历",
            date.year()
        ));
    }
    if matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
        return Ok(false);
    }
    let md = date.month() * 100 + date.day();
    Ok(!matches!(md,101..=103|215..=223|404..=406|501..=505|619..=621|925..=927|1001..=1007))
}
pub fn continuous(now: DateTime<Utc>) -> Result<(), String> {
    let local = now.with_timezone(&chrono::FixedOffset::east_opt(28800).unwrap());
    match super::trading_calendar::is_trading_day_now(now) {
        Ok(true) => {}
        Ok(false) => {
            return Err(format!(
                "A 股休市：{} 不是交易日（周末或休市日）",
                local.date_naive()
            ))
        }
        Err(error) => {
            return Err(format!(
                "无法确认 {} 是否交易日，实时模拟暂停：{error}",
                local.date_naive()
            ))
        }
    }
    let seconds = local.num_seconds_from_midnight();
    if !(34200..41400).contains(&seconds) && !(46800..53820).contains(&seconds) {
        return Err(
            "等待 A 股连续竞价时段 09:30–11:30 / 13:00–14:57（北京时间）；集合竞价不模拟成交"
                .into(),
        );
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    #[test]
    fn exchange_holidays_and_makeup_weekends_are_closed() {
        for date in ["2026-02-23", "2026-09-25", "2026-10-01", "2026-10-10"] {
            assert!(!trading_day(NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap()).unwrap());
        }
        assert!(trading_day(NaiveDate::from_ymd_opt(2026, 9, 28).unwrap()).unwrap());
        assert!(trading_day(NaiveDate::from_ymd_opt(2027, 1, 4).unwrap()).is_err());
    }
    #[test]
    fn cst_continuous_only() {
        for (h, m) in [(1, 29), (3, 30), (4, 0), (6, 57), (7, 0)] {
            assert!(continuous(Utc.with_ymd_and_hms(2026, 9, 21, h, m, 0).unwrap()).is_err());
        }
        assert!(continuous(Utc.with_ymd_and_hms(2026, 9, 21, 1, 30, 0).unwrap()).is_ok());
    }
}
