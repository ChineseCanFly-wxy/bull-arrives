//! Only future, timestamped A-share quotes can execute paper orders. No OHLC replay.
use crate::{
    domain::{Depth, Quote},
    simulation::{FeeConfig, FillQuote, Side, SCALE},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveTick {
    pub quote: Quote,
    pub depth: Depth,
    pub source: String,
    pub received_at: i64,
}
pub fn scaled(v: f64) -> Result<i64, String> {
    if !v.is_finite() || v <= 0.0 || v > 1_000_000.0 {
        return Err("报价价格无效".into());
    }
    Ok((v * SCALE as f64).round() as i64)
}
pub fn validate(tick: &LiveTick, now: DateTime<Utc>) -> Result<(), String> {
    crate::datasource::a_share_calendar::continuous(now)?;
    let q = &tick.quote;
    if q.market != "CN" || !is_a_share(&q.code) {
        return Err("只允许沪深北 A 股股票，不支持港美股、基金、债券或指数".into());
    }
    // 涨跌幅由代码所属板块决定；退市整理期与上市前 5 日没有可套用的比例，直接拒绝。
    crate::market_rules::ensure_simulatable(&q.code, &q.name)?;
    if q.timestamp <= 0
        || tick.depth.timestamp <= 0
        || now.timestamp() - q.timestamp > 10
        || now.timestamp() - tick.depth.timestamp > 10
        || q.timestamp > now.timestamp() + 1
        || tick.depth.timestamp > now.timestamp() + 1
        || now.timestamp() - tick.received_at > 3
        || tick.received_at > now.timestamp() + 1
    {
        return Err("报价或盘口已过期/时间无效，等待新行情".into());
    }
    if tick.depth.code != q.code || (q.timestamp - tick.depth.timestamp).abs() > 3 {
        return Err("报价与盘口标的或时间不一致".into());
    }
    if q.volume == 0 {
        return Err("当日无成交或停牌，等待行情".into());
    }
    scaled(q.price)?;
    scaled(q.prev_close)?;
    Ok(())
}
pub fn is_a_share(symbol: &str) -> bool {
    let (prefix, code) = if symbol.len() == 8 && symbol.is_ascii() {
        symbol.split_at(2)
    } else {
        return false;
    };
    if !code.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    use crate::datasource::eastmoney_universe::Board;
    matches!(
        (prefix, Board::from_code(code)),
        ("sh", Board::ShMain | Board::Star)
            | ("sz", Board::SzMain | Board::ChiNext)
            | ("bj", Board::Bse)
    )
}
pub fn quote_fill(
    tick: &LiveTick,
    side: Side,
    quantity: i64,
    limit: i64,
    submitted: i64,
    available: i64,
    cash: i64,
    fees: FeeConfig,
    limit_bps: i64,
    now: DateTime<Utc>,
) -> Result<FillQuote, String> {
    validate(tick, now)?;
    // 涨跌幅按板块规则重算：主板 10%（含 2026-07-06 起并轨的主板 ST / *ST）、
    // 创业板/科创板 20%、北交所 30%。
    let expected = crate::market_rules::ensure_simulatable(&tick.quote.code, &tick.quote.name)?;
    if limit_bps != expected {
        return Err("涨跌幅参数与 A 股板块规则不符，特殊状态不自动成交".into());
    }
    if tick.quote.timestamp <= submitted || tick.depth.timestamp <= submitted {
        return Err("等待委托建立之后的新报价，禁止事后补成交".into());
    }
    if quantity <= 0 {
        return Err("委托数量必须大于 0".into());
    }
    match side {
        Side::Buy => crate::market_rules::validate_buy_quantity(&tick.quote.code, quantity)?,
        Side::Sell => {
            if quantity > available {
                return Err("T+1：今日买入股份尚不可卖，等待下一 A 股交易日".into());
            }
        }
    }
    let levels = if side == Side::Buy {
        &tick.depth.asks
    } else {
        &tick.depth.bids
    };
    let best = levels
        .iter()
        .filter(|l| l.price.is_finite() && l.price > 0.0 && l.volume > 0)
        .min_by(|a, b| {
            if side == Side::Buy {
                a.price.total_cmp(&b.price)
            } else {
                b.price.total_cmp(&a.price)
            }
        })
        .ok_or("没有可见对手盘，暂不成交")?;
    if best.volume < (quantity as u64) {
        return Err("最优档可见挂单量不足，整笔等待（不虚构全部成交）".into());
    }
    let price = scaled(best.price)?;
    let previous = scaled(tick.quote.prev_close)?;
    let upper = ((previous as i128 * (10000 + limit_bps) as i128 / 10000 + 50) / 100 * 100) as i64;
    let lower = ((previous as i128 * (10000 - limit_bps) as i128 / 10000 + 50) / 100 * 100) as i64;
    if !(1..=3000).contains(&limit_bps) || price > upper || price < lower {
        return Err("报价超出已知涨跌幅范围，特殊交易状态不模拟".into());
    }
    // 涨跌停边界不再一律拒绝：若盘口在该价位仍有足额对手盘，说明封板已被打开
    // （真正封死的形态是对手档缺失或挂单量不足，上面两处检查已经挡下），
    // 此时按盘口可见量成交即可。旧实现无条件拒绝，会把「盘中开板」也误杀成不可交易。
    if [
        fees.commission_bps,
        fees.stamp_tax_bps,
        fees.transfer_fee_bps,
        fees.slippage_bps,
    ]
    .iter()
    .any(|n| !(0..=1000).contains(n))
        || fees.min_commission < 0
    {
        return Err("费用参数无效".into());
    }
    let slipped = if side == Side::Buy {
        (price as i128 * (10000 + fees.slippage_bps) as i128 + 9999) / 10000
    } else {
        price as i128 * (10000 - fees.slippage_bps) as i128 / 10000
    };
    let price = if side == Side::Buy {
        ((slipped + 99) / 100 * 100) as i64
    } else {
        (slipped / 100 * 100) as i64
    };
    if price <= 0
        || price > upper
        || price < lower
        || side == Side::Buy && price > limit
        || side == Side::Sell && price < limit
    {
        return Err("含滑点的对手价未达到限价，等待".into());
    }
    let gross = price.checked_mul(quantity).ok_or("金额超限")?;
    let fee = crate::simulation::fee(gross, side, fees)?;
    let cash_delta = if side == Side::Buy {
        -gross.checked_add(fee).ok_or("费用超限")?
    } else {
        gross.checked_sub(fee).ok_or("费用超限")?
    };
    if cash.checked_add(cash_delta).is_none_or(|n| n < 0) {
        return Err("可用资金不足（含费用）".into());
    }
    Ok(FillQuote {
        price,
        gross,
        fee,
        cash_delta,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Level;
    use chrono::TimeZone;
    fn tick(price: f64, now: DateTime<Utc>) -> LiveTick {
        LiveTick {
            quote: Quote {
                code: "sh600000".into(),
                market: "CN".into(),
                name: "测试A股".into(),
                price,
                change: 0.0,
                change_pct: 0.0,
                prev_close: 34.0,
                open: 34.0,
                high: 35.0,
                low: 33.0,
                volume: 100000,
                turnover: 1000000.0,
                turnover_rate: None,
                timestamp: now.timestamp(),
            },
            depth: Depth {
                code: "sh600000".into(),
                bids: vec![Level {
                    price,
                    volume: 1000,
                }],
                asks: vec![Level {
                    price,
                    volume: 1000,
                }],
                timestamp: now.timestamp(),
            },
            source: "fixture".into(),
            received_at: now.timestamp(),
        }
    }
    fn fees() -> FeeConfig {
        FeeConfig {
            commission_bps: 0,
            min_commission: 0,
            stamp_tax_bps: 0,
            transfer_fee_bps: 0,
            slippage_bps: 0,
        }
    }
    #[test]
    fn buy_34_sell_35_requires_future_quote_and_t_plus_one() {
        let now = Utc.with_ymd_and_hms(2026, 9, 21, 2, 0, 0).unwrap();
        let t = tick(34.0, now);
        assert!(quote_fill(
            &t,
            Side::Buy,
            100,
            340000,
            now.timestamp(),
            0,
            100000000,
            fees(),
            1000,
            now
        )
        .is_err());
        let buy = quote_fill(
            &t,
            Side::Buy,
            100,
            340000,
            now.timestamp() - 1,
            0,
            100000000,
            fees(),
            1000,
            now,
        )
        .unwrap();
        assert_eq!(buy.price, 340000);
        let sell = tick(35.0, now);
        assert!(quote_fill(
            &sell,
            Side::Sell,
            100,
            350000,
            now.timestamp() - 1,
            0,
            100000000,
            fees(),
            1000,
            now
        )
        .unwrap_err()
        .contains("T+1"));
        let next = Utc.with_ymd_and_hms(2026, 9, 22, 2, 0, 0).unwrap();
        let sold = quote_fill(
            &tick(35.0, next),
            Side::Sell,
            100,
            350000,
            next.timestamp() - 1,
            100,
            100000000,
            fees(),
            1000,
            next,
        )
        .unwrap();
        assert_eq!(sold.gross - buy.gross, 1000000);
    }
    #[test]
    fn stale_closed_depth_insufficient_and_slippage_never_fake_fills() {
        let now = Utc.with_ymd_and_hms(2026, 9, 21, 2, 0, 0).unwrap();
        let mut t = tick(34.0, now);
        t.depth.timestamp -= 11;
        assert!(quote_fill(
            &t,
            Side::Buy,
            100,
            340000,
            now.timestamp() - 20,
            0,
            100000000,
            fees(),
            1000,
            now
        )
        .is_err());
        t = tick(34.0, now);
        t.depth.asks[0].volume = 99;
        assert!(quote_fill(
            &t,
            Side::Buy,
            100,
            340000,
            now.timestamp() - 1,
            0,
            100000000,
            fees(),
            1000,
            now
        )
        .is_err());
        let mut f = fees();
        f.slippage_bps = 5;
        assert!(quote_fill(
            &tick(34.0, now),
            Side::Buy,
            100,
            340000,
            now.timestamp() - 1,
            0,
            100000000,
            f,
            1000,
            now
        )
        .is_err());
        let closed = Utc.with_ymd_and_hms(2026, 9, 25, 2, 0, 0).unwrap();
        assert!(validate(&tick(34.0, closed), closed).is_err());
        assert!(!is_a_share("sh000300"));
        assert!(!is_a_share("sh510300"));
        assert!(!is_a_share("usAAPL"));
        assert!(is_a_share("bj920001"));
    }

    #[test]
    fn limit_price_is_tradeable_when_depth_shows_counterparty() {
        let now = Utc.with_ymd_and_hms(2026, 9, 21, 2, 0, 0).unwrap();
        // 涨停价 37.40 = 34.00 × 1.1，盘口在涨停价上仍有足额卖盘 → 封板已开，可买入。
        let opened = tick(37.4, now);
        let fill = quote_fill(
            &opened,
            Side::Buy,
            100,
            374000,
            now.timestamp() - 1,
            0,
            100000000,
            fees(),
            1000,
            now,
        )
        .expect("涨停价上有足额卖盘，应视为开板可买");
        assert_eq!(fill.price, 374000);

        // 跌停价 30.60 上有足额买盘 → 开板可卖（与买入侧对称）。
        let sold = quote_fill(
            &tick(30.6, now),
            Side::Sell,
            100,
            306000,
            now.timestamp() - 1,
            100,
            100000000,
            fees(),
            1000,
            now,
        )
        .expect("跌停价上有足额买盘，应视为开板可卖");
        assert_eq!(sold.price, 306000);

        // 真封死：对手档缺失时依旧不虚构成交。
        let mut sealed = tick(37.4, now);
        sealed.depth.asks.clear();
        assert!(quote_fill(
            &sealed,
            Side::Buy,
            100,
            374000,
            now.timestamp() - 1,
            0,
            100000000,
            fees(),
            1000,
            now
        )
        .is_err());
    }
}
