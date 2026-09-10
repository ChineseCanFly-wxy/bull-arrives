use chrono::{DateTime, Datelike, FixedOffset, Timelike, Utc};
use serde::Serialize;

use crate::db::{Database, PriceAlert};
use crate::domain::Quote;

const CHINA_OFFSET_SECONDS: i32 = 8 * 60 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertDirection {
    Up,
    Down,
}

impl AlertDirection {
    fn as_str(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AlertDecision {
    pub value: f64,
    pub value_day: Option<String>,
    pub trigger: Option<AlertDirection>,
}

/// Stable, serializable contract emitted as `price-alert-triggered`.
#[derive(Debug, Clone, Serialize)]
pub struct PriceAlertEvent {
    pub id: i64,
    pub code: String,
    pub market: String,
    pub name: String,
    pub alert_type: String,
    pub threshold: f64,
    pub repeat_mode: String,
    pub cooldown_minutes: i64,
    pub current_price: f64,
    pub change_pct: f64,
    pub value: f64,
    pub direction: String,
    pub triggered_at: String,
    pub triggered_day: String,
}

pub fn is_alert_time(now: DateTime<FixedOffset>) -> bool {
    if now.weekday().number_from_monday() > 5 {
        return false;
    }
    let seconds = now.hour() * 3600 + now.minute() * 60 + now.second();
    (9 * 3600 + 15 * 60..11 * 3600 + 30 * 60).contains(&seconds)
        || (13 * 3600..15 * 3600).contains(&seconds)
}

pub(crate) fn is_fresh_trading_quote(quote: &Quote, now: DateTime<FixedOffset>) -> bool {
    let Some(quote_utc) = DateTime::<Utc>::from_timestamp(quote.timestamp, 0) else {
        return false;
    };
    let quote_time = quote_utc.with_timezone(now.offset());
    quote_time.date_naive() == now.date_naive()
        && is_alert_time(quote_time)
        && quote_time <= now
        && now.signed_duration_since(quote_time).num_seconds() <= 5 * 60
}

pub fn decide(
    alert: &PriceAlert,
    current_price: f64,
    previous_close: f64,
    now: DateTime<FixedOffset>,
    notifications_enabled: bool,
    rearm_only: bool,
) -> Option<AlertDecision> {
    if !current_price.is_finite() || current_price <= 0.0 {
        return None;
    }

    let today = now.date_naive().to_string();
    let (value, value_day) = match alert.alert_type.as_str() {
        "change_pct" => {
            if !previous_close.is_finite() || previous_close <= 0.0 {
                return None;
            }
            (
                (current_price - previous_close) / previous_close * 100.0,
                Some(today.clone()),
            )
        }
        "fixed_price" => (current_price, None),
        _ => return None,
    };
    if !value.is_finite() {
        return None;
    }

    // Percentage observations belong to one trading day. Fixed-price observations
    // deliberately survive day boundaries so a restart/overnight gap can cross.
    let previous = if alert.alert_type == "change_pct"
        && alert.last_value_day.as_deref() != Some(today.as_str())
    {
        None
    } else {
        alert.last_value.filter(|candidate| candidate.is_finite())
    };

    let condition = condition_met(alert, value);
    let entry_direction = condition_entry(alert, previous, value);
    let price_crossing = if alert.alert_type == "fixed_price" {
        crossing(previous, value, alert.threshold)
    } else {
        entry_direction
    };

    let eligible = notifications_enabled && alert.enabled && !rearm_only;
    let trigger = if !eligible {
        None
    } else {
        match alert.alert_type.as_str() {
            // A target price always represents a crossing, including daily and
            // cooldown modes. Merely remaining at the target never repeats.
            "fixed_price" => price_crossing.filter(|_| repeat_allows(alert, now, &today)),
            "change_pct" => match alert.repeat_mode.as_str() {
                "daily" => condition
                    .then_some(direction_for_percentage(alert))
                    .filter(|_| alert.last_triggered_day.as_deref() != Some(today.as_str())),
                "crossing" => entry_direction,
                "cooldown" => condition
                    .then_some(direction_for_percentage(alert))
                    .filter(|_| cooldown_elapsed(alert, now)),
                _ => None,
            },
            _ => None,
        }
    };

    Some(AlertDecision {
        value,
        value_day,
        trigger,
    })
}

fn condition_met(alert: &PriceAlert, value: f64) -> bool {
    if alert.alert_type == "change_pct" {
        if alert.threshold >= 0.0 {
            value >= alert.threshold
        } else {
            value <= alert.threshold
        }
    } else {
        false
    }
}

fn condition_entry(
    alert: &PriceAlert,
    previous: Option<f64>,
    current: f64,
) -> Option<AlertDirection> {
    let direction = direction_for_percentage(alert);
    match previous {
        Some(previous) if !condition_met(alert, previous) && condition_met(alert, current) => {
            Some(direction)
        }
        _ => None,
    }
}

fn direction_for_percentage(alert: &PriceAlert) -> AlertDirection {
    if alert.threshold >= 0.0 {
        AlertDirection::Up
    } else {
        AlertDirection::Down
    }
}

fn crossing(previous: Option<f64>, current: f64, target: f64) -> Option<AlertDirection> {
    let previous = previous?;
    if previous < target && current >= target {
        Some(AlertDirection::Up)
    } else if previous > target && current <= target {
        Some(AlertDirection::Down)
    } else {
        None
    }
}

fn repeat_allows(alert: &PriceAlert, now: DateTime<FixedOffset>, today: &str) -> bool {
    match alert.repeat_mode.as_str() {
        "daily" => alert.last_triggered_day.as_deref() != Some(today),
        "crossing" => true,
        "cooldown" => cooldown_elapsed(alert, now),
        _ => false,
    }
}

fn cooldown_elapsed(alert: &PriceAlert, now: DateTime<FixedOffset>) -> bool {
    let Some(last) = alert
        .last_triggered_at
        .as_deref()
        .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
    else {
        return true;
    };
    now.signed_duration_since(last).num_seconds() >= alert.cooldown_minutes * 60
}

/// Evaluate only quotes freshly returned by the datasource. Cache restoration and
/// cached fallback paths must never call this function.
pub fn evaluate_fresh_quotes<F>(
    db: &Database,
    quotes: &[Quote],
    now: DateTime<FixedOffset>,
    rearm_codes: &std::collections::HashSet<String>,
    mut publish: F,
) where
    F: FnMut(PriceAlertEvent),
{
    if !is_alert_time(now) {
        return;
    }

    let global_enabled = match db.get_setting("alerts_enabled") {
        Ok(Some(value)) => value != "0" && !value.eq_ignore_ascii_case("false"),
        Ok(None) => true,
        Err(error) => {
            log::warn!("Failed to read alerts_enabled: {}", error);
            return;
        }
    };
    let alerts = match db.get_all_price_alerts() {
        Ok(alerts) => alerts,
        Err(error) => {
            log::warn!("Failed to read price alerts: {}", error);
            return;
        }
    };

    let now_rfc3339 = now.to_rfc3339();
    let today = now.date_naive().to_string();
    for alert in alerts {
        let Some(quote) = quotes
            .iter()
            .find(|quote| quote.code == alert.code && quote.market == alert.market)
        else {
            continue;
        };
        if !is_fresh_trading_quote(quote, now) {
            continue;
        }
        // Use the datasource's explicit previous close; a missing legacy-cache
        // value is zero and therefore cannot trigger a percentage alert.
        let previous_close = quote.prev_close;
        let key = format!("{}:{}", alert.market, alert.code);
        let Some(decision) = decide(
            &alert,
            quote.price,
            previous_close,
            now,
            global_enabled,
            rearm_codes.contains(&key),
        ) else {
            continue;
        };
        let triggered_at = decision.trigger.map(|_| now_rfc3339.as_str());
        let triggered_day = decision.trigger.map(|_| today.as_str());
        if let Err(error) = db.persist_price_alert_evaluation(
            &alert,
            decision.value,
            decision.value_day.as_deref(),
            triggered_at,
            triggered_day,
        ) {
            log::warn!(
                "Failed to persist alert evaluation {}:{}:{}: {}",
                alert.market,
                alert.code,
                alert.alert_type,
                error
            );
            continue;
        }

        if let Some(direction) = decision.trigger {
            let change_pct = if previous_close > 0.0 {
                (quote.price - previous_close) / previous_close * 100.0
            } else {
                quote.change_pct
            };
            publish(PriceAlertEvent {
                id: alert.id,
                code: alert.code,
                market: alert.market,
                name: quote.name.clone(),
                alert_type: alert.alert_type,
                threshold: alert.threshold,
                repeat_mode: alert.repeat_mode,
                cooldown_minutes: alert.cooldown_minutes,
                current_price: quote.price,
                change_pct,
                value: decision.value,
                direction: direction.as_str().to_owned(),
                triggered_at: now_rfc3339.clone(),
                triggered_day: today.clone(),
            });
        }
    }
}

pub fn china_now() -> DateTime<FixedOffset> {
    Utc::now().with_timezone(
        &FixedOffset::east_opt(CHINA_OFFSET_SECONDS).expect("UTC+8 is a valid fixed offset"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(day_and_time: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(&format!("{day_and_time}+08:00")).unwrap()
    }

    fn alert(alert_type: &str, threshold: f64, repeat_mode: &str) -> PriceAlert {
        PriceAlert {
            id: 1,
            code: "sh600000".into(),
            market: "CN".into(),
            alert_type: alert_type.into(),
            threshold,
            enabled: true,
            repeat_mode: repeat_mode.into(),
            cooldown_minutes: 5,
            last_triggered_at: None,
            last_triggered_day: None,
            last_value: None,
            last_value_day: None,
        }
    }

    #[test]
    fn alert_time_boundaries_use_half_open_sessions() {
        assert!(!is_alert_time(at("2026-09-10T09:14:59")));
        assert!(is_alert_time(at("2026-09-10T09:15:00")));
        assert!(is_alert_time(at("2026-09-10T11:29:59")));
        assert!(!is_alert_time(at("2026-09-10T11:30:00")));
        assert!(!is_alert_time(at("2026-09-10T11:30:01")));
        assert!(is_alert_time(at("2026-09-10T13:00:00")));
        assert!(is_alert_time(at("2026-09-10T14:59:59")));
        assert!(!is_alert_time(at("2026-09-10T15:00:00")));
        assert!(!is_alert_time(at("2026-09-10T15:00:01")));
        assert!(!is_alert_time(at("2026-09-12T10:00:00")));
    }

    #[test]
    fn percentage_uses_previous_close_and_preserves_sign() {
        let up = alert("change_pct", 5.0, "daily");
        assert_eq!(decide(&up, 105.0, 100.0, at("2026-09-10T10:00:00"), true, false).unwrap().trigger, Some(AlertDirection::Up));
        assert_eq!(decide(&up, 95.0, 100.0, at("2026-09-10T10:00:00"), true, false).unwrap().trigger, None);

        let down = alert("change_pct", -5.0, "daily");
        assert_eq!(decide(&down, 95.0, 100.0, at("2026-09-10T10:00:00"), true, false).unwrap().trigger, Some(AlertDirection::Down));
        assert_eq!(decide(&down, 105.0, 100.0, at("2026-09-10T10:00:00"), true, false).unwrap().trigger, None);
    }

    #[test]
    fn fixed_price_crosses_both_directions_and_handles_jumps() {
        let mut rule = alert("fixed_price", 10.0, "crossing");
        rule.last_value = Some(9.0);
        assert_eq!(decide(&rule, 11.0, 9.0, at("2026-09-10T10:00:00"), true, false).unwrap().trigger, Some(AlertDirection::Up));
        rule.last_value = Some(11.0);
        assert_eq!(decide(&rule, 9.0, 11.0, at("2026-09-10T10:01:00"), true, false).unwrap().trigger, Some(AlertDirection::Down));
    }

    #[test]
    fn equality_does_not_repeat_until_recrossed() {
        let mut rule = alert("fixed_price", 10.0, "crossing");
        rule.last_value = Some(9.0);
        assert!(decide(&rule, 10.0, 9.0, at("2026-09-10T10:00:00"), true, false).unwrap().trigger.is_some());
        rule.last_value = Some(10.0);
        assert!(decide(&rule, 10.0, 10.0, at("2026-09-10T10:01:00"), true, false).unwrap().trigger.is_none());
        assert!(decide(&rule, 11.0, 10.0, at("2026-09-10T10:02:00"), true, false).unwrap().trigger.is_none());
        rule.last_value = Some(11.0);
        assert_eq!(decide(&rule, 10.0, 11.0, at("2026-09-10T10:03:00"), true, false).unwrap().trigger, Some(AlertDirection::Down));
    }

    #[test]
    fn daily_percentage_state_resets_but_price_state_does_not() {
        let mut pct = alert("change_pct", 5.0, "crossing");
        pct.last_value = Some(4.0);
        pct.last_value_day = Some("2026-09-09".into());
        assert!(decide(&pct, 106.0, 100.0, at("2026-09-10T10:00:00"), true, false).unwrap().trigger.is_none());

        let mut price = alert("fixed_price", 10.0, "crossing");
        price.last_value = Some(9.0);
        price.last_value_day = Some("2026-09-09".into());
        assert!(decide(&price, 11.0, 9.0, at("2026-09-10T10:00:00"), true, false).unwrap().trigger.is_some());
    }

    #[test]
    fn daily_and_cooldown_limits_are_enforced() {
        let mut daily = alert("change_pct", 5.0, "daily");
        daily.last_triggered_day = Some("2026-09-10".into());
        assert!(decide(&daily, 106.0, 100.0, at("2026-09-10T10:00:00"), true, false).unwrap().trigger.is_none());

        let mut cooldown = alert("fixed_price", 10.0, "cooldown");
        cooldown.last_value = Some(9.0);
        cooldown.last_triggered_at = Some(at("2026-09-10T10:00:00").to_rfc3339());
        assert!(decide(&cooldown, 11.0, 9.0, at("2026-09-10T10:04:59"), true, false).unwrap().trigger.is_none());
        assert!(decide(&cooldown, 11.0, 9.0, at("2026-09-10T10:05:00"), true, false).unwrap().trigger.is_some());
    }

    #[test]
    fn disabled_observations_advance_state_without_triggering() {
        let mut rule = alert("fixed_price", 10.0, "crossing");
        rule.last_value = Some(9.0);
        let decision = decide(&rule, 11.0, 9.0, at("2026-09-10T10:00:00"), false, false).unwrap();
        assert_eq!(decision.trigger, None);
        assert_eq!(decision.value, 11.0);
    }
}
