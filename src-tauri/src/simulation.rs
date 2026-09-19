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
    pub symbol: String,
    pub name: String,
    pub side: Side,
    pub quantity: i64,
    pub signal_date: String,
    pub available_quantity: i64,
    /// 该标的的持仓总量（含当日买入、尚未满足 T+1 的部分）。
    pub position_quantity: i64,
    pub cash: i64,
    /// 账户里记录的涨跌幅配置。**不用于撮合**：撮合一律按代码所属板块的
    /// 交易所规则重算（见 [`crate::market_rules::ensure_simulatable`]），
    /// 这里的值只用来提示配置与实际规则不一致。
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

pub(crate) fn fee(gross: i64, side: Side, config: FeeConfig) -> Result<i64, String> {
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
///
/// 涨跌幅与申报数量一律按代码所属板块的交易所规则判定，账户里配错了参数
/// 也不会改变撮合结果 —— 主板 10%（含 2026-07-06 起并轨的主板 ST / *ST）、
/// 创业板/科创板 20%、北交所 30%。
pub fn match_raw_bar(request: &MatchRequest, bar: &RawBar) -> Result<FillQuote, String> {
    let limit_bps = crate::market_rules::ensure_simulatable(&request.symbol, &request.name)?;
    if request.limit_bps != limit_bps {
        log::warn!(
            "[simulation] {} 账户配置涨跌幅 {} 与板块规则 {} 不符，已按交易所规则撮合",
            request.symbol,
            request.limit_bps,
            limit_bps
        );
    }
    match request.side {
        Side::Buy => crate::market_rules::validate_buy_quantity(&request.symbol, request.quantity)?,
        Side::Sell => crate::market_rules::validate_sell_quantity(
            request.quantity,
            request.position_quantity,
            request.available_quantity,
        )?,
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

    let open = rounded_cent(parse_scaled(&bar.open, "开盘价")?);
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
    let upper = rounded_cent(mul_div(prev, 10_000 + limit_bps, 10_000)?);
    let lower = rounded_cent(mul_div(prev, 10_000 - limit_bps, 10_000)?);
    if open > upper || open < lower {
        // 未复权开盘价不可能落在涨跌停之外；出现说明数据本身有问题，宁可拒撮合。
        return Err("开盘价超出当日涨跌停区间，数据异常".into());
    }
    // 只有「一字板」才是真正买不进 / 卖不掉：以涨跌停价开盘且全天振幅为 0，
    // 说明封单自始至终没有被打开过。
    //
    // 以涨停价开盘但盘中开板（high > low）时**不再拒绝**：封单被打开后板上挂单会成交，
    // 买方能够进场。此时成交价仍按开盘价（即涨停价）计算 —— 这是当日买方可能付出的
    // 最差价格，偏保守，不会虚增策略收益。卖出侧对跌停开盘的同理。
    if request.side == Side::Buy && open >= upper && high == low {
        return Err("一字涨停无法买入".into());
    }
    if request.side == Side::Sell && open <= lower && high == low {
        return Err("一字跌停无法卖出".into());
    }

    let slipped = match request.side {
        Side::Buy => rounded_cent(mul_div(open, 10_000 + request.fees.slippage_bps, 10_000)?.min(upper)),
        Side::Sell => rounded_cent(mul_div(open, 10_000 - request.fees.slippage_bps, 10_000)?.max(lower)),
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
            symbol: "sh600000".into(),
            name: "浦发银行".into(),
            side,
            quantity: 100,
            signal_date: "2026-01-02".into(),
            available_quantity: 100,
            position_quantity: 100,
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

    #[test]
    fn board_limits_and_lots_override_account_configuration() {
        // 创业板 20%：开盘 +12% 不是涨停，正常成交；同一根 K 线放到主板就是开盘涨停。
        let mut growth = request(Side::Buy);
        growth.symbol = "sz300750".into();
        growth.name = "宁德时代".into();
        let mut growth_bar = bar();
        growth_bar.open = "11200".into();
        growth_bar.high = "11800".into();
        growth_bar.low = "11000".into();
        growth_bar.close = "11500".into();
        assert!(match_raw_bar(&growth, &growth_bar).is_ok());

        // 主板 10%：同一根 K 线放到主板，开盘 +12% 已超出涨停区间，属数据异常，拒绝。
        let mut main = request(Side::Buy);
        main.symbol = "sh600000".into();
        assert!(match_raw_bar(&main, &growth_bar)
            .unwrap_err()
            .contains("涨跌停区间"));

        // 创业板 +21% 超过 20% 涨停 → 拒绝，说明没有沿用账户里的 10% 配置。
        growth_bar.open = "12100".into();
        growth_bar.high = "12100".into();
        growth_bar.low = "12100".into();
        growth_bar.close = "12100".into();
        assert!(match_raw_bar(&growth, &growth_bar).is_err());
        assert_eq!(growth.limit_bps, 1_000, "账户配置仍是 10%，但撮合按板块规则走");

        // 主板 ST 自 2026-07-06 起放宽到 10%，+6% 已不再是涨停，可以正常成交。
        let mut st = request(Side::Buy);
        st.name = "ST 红星".into();
        let mut st_bar = bar();
        st_bar.open = "10600".into();
        st_bar.high = "10700".into();
        st_bar.low = "10500".into();
        st_bar.close = "10600".into();
        let st_fill = match_raw_bar(&st, &st_bar).expect("主板 ST 已并轨 10%，+6% 应可成交");
        assert_eq!(st_fill.price, 10_600);

        // 主板 ST 真正一字板时（+10% 且振幅 0）仍然买不进。
        let mut st_sealed = request(Side::Buy);
        st_sealed.name = "*ST 红星".into();
        let mut st_sealed_bar = bar();
        st_sealed_bar.open = "11000".into();
        st_sealed_bar.high = "11000".into();
        st_sealed_bar.low = "11000".into();
        st_sealed_bar.close = "11000".into();
        assert!(match_raw_bar(&st_sealed, &st_sealed_bar)
            .unwrap_err()
            .contains("一字涨停"));

        // 科创板买入至少 200 股，且 200 股以上可以 1 股递增。
        let mut star = request(Side::Buy);
        star.symbol = "sh688981".into();
        star.name = "中芯国际".into();
        star.quantity = 100;
        assert!(match_raw_bar(&star, &bar()).unwrap_err().contains("200"));
        star.quantity = 300;
        star.cash = 10_000_000;
        assert!(match_raw_bar(&star, &bar()).is_ok());

        // 退市整理期与上市前 5 日不套用任何涨跌幅参数，直接拒绝模拟。
        let mut delisting = request(Side::Buy);
        delisting.name = "退市红星".into();
        assert!(match_raw_bar(&delisting, &bar()).unwrap_err().contains("退市整理期"));
        let mut newborn = request(Side::Buy);
        newborn.name = "N 新能".into();
        assert!(match_raw_bar(&newborn, &bar()).unwrap_err().contains("无涨跌幅"));
    }

    #[test]
    fn limit_open_board_is_tradeable_but_one_word_board_is_not() {
        // 以涨停价开盘、盘中开板（有振幅）→ 可以买入；成交价仍按涨停价，
        // 即当日买方可能付出的最差价格，偏保守。
        let mut opened = bar();
        opened.open = "11000".into();
        opened.high = "11000".into();
        opened.low = "10400".into();
        opened.close = "10600".into();
        let fill = match_raw_bar(&request(Side::Buy), &opened).expect("盘中开板应可买入");
        assert_eq!(fill.price, 11_000);

        // 全天封死在涨停价（振幅为 0）→ 买不进。
        let mut sealed = opened.clone();
        sealed.low = "11000".into();
        sealed.close = "11000".into();
        assert!(match_raw_bar(&request(Side::Buy), &sealed)
            .unwrap_err()
            .contains("一字涨停"));

        // 卖出侧对称：跌停开盘后开板可卖出，成交价按跌停价（卖方最差价）。
        let mut down_open = bar();
        down_open.open = "9000".into();
        down_open.high = "9600".into();
        down_open.low = "9000".into();
        down_open.close = "9400".into();
        let sold = match_raw_bar(&request(Side::Sell), &down_open).expect("盘中开板应可卖出");
        assert_eq!(sold.price, 9_000);

        let mut down_sealed = down_open.clone();
        down_sealed.high = "9000".into();
        down_sealed.close = "9000".into();
        assert!(match_raw_bar(&request(Side::Sell), &down_sealed)
            .unwrap_err()
            .contains("一字跌停"));

        // 开盘价落在涨跌停区间之外属数据异常，拒绝而不是猜一个价。
        let mut broken = bar();
        broken.open = "11200".into();
        broken.high = "11800".into();
        broken.low = "11000".into();
        broken.close = "11200".into();
        assert!(match_raw_bar(&request(Side::Buy), &broken)
            .unwrap_err()
            .contains("涨跌停区间"));
    }
}
