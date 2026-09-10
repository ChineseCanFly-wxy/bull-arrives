use chrono::{DateTime, Datelike, FixedOffset, NaiveTime, Utc, Weekday};

/// A-share market trading session
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MarketSession {
    /// Before 09:15 — pre-market
    PreOpen,
    /// 09:15–11:30 — opening auction and morning trading
    MorningTrade,
    /// 11:30–13:00 — lunch break
    LunchBreak,
    /// 13:00–15:00 — afternoon trading
    AfternoonTrade,
    /// After 15:00 or weekend/configured holiday — closed
    Closed,
}

impl MarketSession {
    /// Determine the current A-share market session (China Standard Time / UTC+8).
    pub fn current() -> Self {
        Self::at_utc(Utc::now())
    }

    /// Testable session calculation for an explicit UTC instant.
    /// This clock handles weekdays only; configured closure dates are enforced by
    /// `MarketRequestPolicy` and are not a claim of a verified exchange calendar.
    pub fn at_utc(now: DateTime<Utc>) -> Self {
        let cst_offset = FixedOffset::east_opt(8 * 3600).expect("UTC+8 is a valid offset");
        let now = now.with_timezone(&cst_offset);

        if matches!(now.weekday(), Weekday::Sat | Weekday::Sun) {
            return Self::Closed;
        }

        let time = now.time();
        let morning_start = NaiveTime::from_hms_opt(9, 15, 0).expect("valid time constant");
        let morning_end = NaiveTime::from_hms_opt(11, 30, 0).expect("valid time constant");
        let afternoon_start = NaiveTime::from_hms_opt(13, 0, 0).expect("valid time constant");
        let afternoon_end = NaiveTime::from_hms_opt(15, 0, 0).expect("valid time constant");

        if time < morning_start {
            Self::PreOpen
        } else if time < morning_end {
            Self::MorningTrade
        } else if time < afternoon_start {
            Self::LunchBreak
        } else if time < afternoon_end {
            Self::AfternoonTrade
        } else {
            Self::Closed
        }
    }

    /// Recommended polling interval in seconds for this session
    pub fn recommended_interval(&self) -> u64 {
        match self {
            Self::MorningTrade | Self::AfternoonTrade => 2,
            Self::PreOpen => 5,
            Self::LunchBreak => 10,
            Self::Closed => 30,
        }
    }

    /// Human-readable session name
    pub fn name(&self) -> &str {
        match self {
            Self::PreOpen => "盘前",
            Self::MorningTrade => "早盘",
            Self::LunchBreak => "午休",
            Self::AfternoonTrade => "午盘",
            Self::Closed => "休市",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn utc(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
            .single()
            .unwrap()
    }

    #[test]
    fn auction_opens_at_0915_beijing() {
        assert_eq!(MarketSession::at_utc(utc(2026, 9, 9, 1, 14)), MarketSession::PreOpen);
        assert_eq!(MarketSession::at_utc(utc(2026, 9, 9, 1, 15)), MarketSession::MorningTrade);
    }

    #[test]
    fn boundaries_and_weekends_are_classified() {
        assert_eq!(MarketSession::at_utc(utc(2026, 9, 9, 3, 30)), MarketSession::LunchBreak);
        assert_eq!(MarketSession::at_utc(utc(2026, 9, 9, 5, 0)), MarketSession::AfternoonTrade);
        assert_eq!(MarketSession::at_utc(utc(2026, 9, 9, 7, 0)), MarketSession::Closed);
        assert_eq!(MarketSession::at_utc(utc(2026, 9, 12, 2, 0)), MarketSession::Closed);
    }
}
