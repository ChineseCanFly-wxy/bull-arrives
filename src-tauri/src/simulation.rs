//! 模拟交易的纯撮合规则。历史库目前只有前复权 K 线，因此自动任务只能生成指令；
//! 只有调用方显式传入未复权 `RawBar` 时，才允许改变账本。

use serde::{Deserialize, Serialize};

pub const SCALE: i64 = 10_000;
pub const MAX_MONEY: i64 = 9_000_000_000_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    pub fn as_str(self) -> &'static str {
        if self == Self::Buy {
            "buy"
        } else {
            "sell"
        }
    }
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "buy" => Ok(Self::Buy),
            "sell" => Ok(Self::Sell),
            _ => Err("买卖方向无效".into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawBar {
    pub date: String,
    pub open: String,
    pub high: String,
    pub low: String,
    pub close: String,
    pub prev_close: String,
    pub volume: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct FeeConfig {
    pub commission_bps: i64,
    pub min_commission: i64,
    pub stamp_tax_bps: i64,
    pub transfer_fee_bps: i64,
    pub slippage_bps: i64,
}

#[derive(Debug, Clone)]
pub struct MatchRequest {
    pub side: Side,
    pub quantity: i64,
    pub signal_date: String,
    pub available_quantity: i64,
    pub cash: i64,
    pub limit_bps: i64,
    pub fees: FeeConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FillQuote {
    pub price: i64,
    pub gross: i64,
    pub fee: i64,
    pub cash_delta: i64,
}

pub fn parse_scaled(value: &str, field: &str) -> Result<i64, String> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!("{field}应为非负的定点整数文本"));
    }
    let value = value
        .parse::<i64>()
        .map_err(|_| format!("{field}超出安全范围"))?;
    if value > MAX_MONEY {
        return Err(format!("{field}超出安全范围"));
    }
    Ok(value)
}

fn mul_div(value: i64, multiplier: i64, divisor: i64) -> Result<i64, String> {
    value
        .checked_mul(multiplier)
        .ok_or_else(|| "金额超出安全范围".to_string())
        .map(|v| v / divisor)
}

fn rounded_cent(value: i64) -> i64 {
    ((value + 50) / 100) * 100
}

fn fee(gross: i64, side: Side, config: FeeConfig) -> Result<i64, String> {
    let commission = mul_div(gross, config.commission_bps, 10_000)?.max(config.min_commission);
    let transfer = mul_div(gross, config.transfer_fee_bps, 10_000)?;
    let stamp = if side == Side::Sell {
        mul_div(gross, config.stamp_tax_bps, 10_000)?
    } else {
        0
    };
    commission
        .checked_add(transfer)
        .and_then(|v| v.checked_add(stamp))
        .ok_or_else(|| "费用超出安全范围".into())
}

/// 用未复权日 K 的开盘价撮合。所有拒绝都在账本事务开始前发生。
pub fn match_raw_bar(request: &MatchRequest, bar: &RawBar) -> Result<FillQuote, String> {
    if request.quantity <= 0 {
        return Err("委托数量必须大于 0".into());
    }
    if request.side == Side::Buy && request.quantity % 100 != 0 {
        return Err("买入数量必须是 100 股的整倍数".into());
    }
    let signal_date = chrono::NaiveDate::parse_from_str(&request.signal_date, "%Y-%m-%d")
        .map_err(|_| "信号日期无效".to_string())?;
    let trade_date = chrono::NaiveDate::parse_from_str(&bar.date, "%Y-%m-%d")
        .map_err(|_| "成交日期无效".to_string())?;
    if trade_date <= signal_date {
        return Err("信号当日不得成交，需等下一交易日开盘".into());
    }
    if bar.volume == 0 {
        return Err("停牌无法成交".into());
    }
    if !(1..=3_000).contains(&request.limit_bps) {
        return Err("涨跌幅限制未知，为避免错误成交已阻断".into());
    }
    for (name, value) in [
        ("佣金费率", request.fees.commission_bps),
        ("印花税", request.fees.stamp_tax_bps),
        ("过户费", request.fees.transfer_fee_bps),
        ("滑点", request.fees.slippage_bps),
    ] {
        if !(0..=1_000).contains(&value) {
            return Err(format!("{name}应在 0~1000 个基点之间"));
        }
    }

    let open = parse_scaled(&bar.open, "开盘价")?;
    let high = parse_scaled(&bar.high, "最高价")?;
    let low = parse_scaled(&bar.low, "最低价")?;
    let close = parse_scaled(&bar.close, "收盘价")?;
    let prev = parse_scaled(&bar.prev_close, "前收盘价")?;
    if [open, high, low, close, prev].contains(&0)
        || low > open
        || open > high
        || low > close
        || close > high
    {
        return Err("未复权 OHLC 数据无效".into());
    }
    let upper = rounded_cent(mul_div(prev, 10_000 + request.limit_bps, 10_000)?);
    let lower = rounded_cent(mul_div(prev, 10_000 - request.limit_bps, 10_000)?);
    if request.side == Side::Buy && open >= upper && high == low {
        return Err("一字涨停无法买入".into());
    }
    if request.side == Side::Sell && open <= lower && high == low {
        return Err("一字跌停无法卖出".into());
    }
    if request.side == Side::Buy && open >= upper {
        return Err("开盘涨停无法买入".into());
    }
    if request.side == Side::Sell && open <= lower {
        return Err("开盘跌停无法卖出".into());
    }
    if request.side == Side::Sell && request.quantity > request.available_quantity {
        return Err("可用持仓不足（T+1 持仓当日不可卖）".into());
    }

    let slipped = match request.side {
        Side::Buy => mul_div(open, 10_000 + request.fees.slippage_bps, 10_000)?.min(upper),
        Side::Sell => mul_div(open, 10_000 - request.fees.slippage_bps, 10_000)?.max(lower),
    };
    let gross = slipped
        .checked_mul(request.quantity)
        .ok_or_else(|| "成交金额超出安全范围".to_string())?;
    let charge = fee(gross, request.side, request.fees)?;
    let cash_delta = match request.side {
        Side::Buy => gross
            .checked_add(charge)
            .and_then(|v| v.checked_neg())
            .ok_or_else(|| "成交金额超出安全范围".to_string())?,
        Side::Sell => gross
            .checked_sub(charge)
            .ok_or_else(|| "费用超过成交金额".to_string())?,
    };
    if request.side == Side::Buy && request.cash.checked_add(cash_delta).is_none_or(|v| v < 0) {
        return Err("可用资金不足".into());
    }
    Ok(FillQuote {
        price: slipped,
        gross,
        fee: charge,
        cash_delta,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(side: Side) -> MatchRequest {
        MatchRequest {
            side,
            quantity: 100,
            signal_date: "2026-01-02".into(),
            available_quantity: 100,
            cash: 2_000_000,
            limit_bps: 1_000,
            fees: FeeConfig {
                commission_bps: 3,
                min_commission: 50_000,
                stamp_tax_bps: 5,
                transfer_fee_bps: 1,
                slippage_bps: 0,
            },
        }
    }
    fn bar() -> RawBar {
        RawBar {
            date: "2026-01-05".into(),
            open: "10000".into(),
            high: "10500".into(),
            low: "9900".into(),
            close: "10200".into(),
            prev_close: "10000".into(),
            volume: 1000,
        }
    }

    #[test]
    fn enforces_cash_lot_t_plus_one_and_market_blocks() {
        let fill = match_raw_bar(&request(Side::Buy), &bar()).unwrap();
        assert_eq!(fill.gross, 1_000_000);
        assert_eq!(fill.fee, 50_100);
        assert_eq!(fill.cash_delta, -1_050_100);

        let mut bad = request(Side::Buy);
        bad.quantity = 99;
        assert!(match_raw_bar(&bad, &bar()).unwrap_err().contains("100"));
        bad = request(Side::Buy);
        bad.cash = 1;
        assert!(match_raw_bar(&bad, &bar()).unwrap_err().contains("资金"));
        let mut same_day = bar();
        same_day.date = "2026-01-02".into();
        assert!(match_raw_bar(&request(Side::Buy), &same_day)
            .unwrap_err()
            .contains("当日"));
        let mut suspended = bar();
        suspended.volume = 0;
        assert!(match_raw_bar(&request(Side::Buy), &suspended)
            .unwrap_err()
            .contains("停牌"));
        let mut limit_up = bar();
        limit_up.open = "11000".into();
        limit_up.high = "11000".into();
        limit_up.low = "11000".into();
        limit_up.close = "11000".into();
        assert!(match_raw_bar(&request(Side::Buy), &limit_up)
            .unwrap_err()
            .contains("一字涨停"));
        let mut limit_down = bar();
        limit_down.open = "9000".into();
        limit_down.high = "9000".into();
        limit_down.low = "9000".into();
        limit_down.close = "9000".into();
        assert!(match_raw_bar(&request(Side::Sell), &limit_down)
            .unwrap_err()
            .contains("一字跌停"));
        let mut frozen = request(Side::Sell);
        frozen.available_quantity = 0;
        assert!(match_raw_bar(&frozen, &bar()).unwrap_err().contains("T+1"));
    }
}
