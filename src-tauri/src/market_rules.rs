//! A 股交易规则：按代码板块自动判定涨跌幅与买卖数量单位。
//!
//! 这里的规则**不读账户配置**。模拟必须执行交易所规则，不能让用户用 10% 的参数
//! 去撮合创业板 20% 的股票，也不能用 100 股整手去卡科创板 200 股起的要求。
//! 全部为纯函数，便于单测与在日线 / 实时两条撮合链路中复用。
//!
//! 现行规则（2020-08-24 创业板注册制、2021-11-15 北交所开市、2023-04-10 主板注册制后）：
//! - 沪深主板 ±10%，**主板 ST / *ST 同为 ±10%**（2026-07-06 起由 5% 放宽，见下）
//! - 创业板（300/301）、科创板（688/689）±20%；注册制下 ST 不改变 20%
//! - 北交所（43x/83x/87x/920）±30%，含 ST
//! - 新股上市后前 5 个交易日不设涨跌幅限制（首日简称前加 `N`，其后 4 日加 `C`）
//! - 退市整理期规则独立（首日不设限、其后 10%）
//!
//! 2026-07-06 起，沪深交易所《交易规则（2026 年修订）》把主板风险警示股票（ST / *ST）
//! 涨跌幅由 5% 放宽至 10%，与主板普通股票并轨；创业板 / 科创板 20%、北交所 30% 维持不变。
//! 因此主板 ST 不再需要单独阈值，但常量与分支保留，便于日后规则再变时只改一处。
//!
//! 最后两类（新股前 5 日、退市整理期）无法用「前收盘 ± 比例」建模，因此**拒绝模拟**
//! 而不是套用错误参数。

use crate::datasource::eastmoney_universe::Board;

/// 沪深主板涨跌幅（基点）
pub const MAIN_LIMIT_BPS: i64 = 1_000;
/// 沪深主板 ST / *ST 涨跌幅。2026-07-06 起与主板普通股票并轨为 10%。
pub const ST_MAIN_LIMIT_BPS: i64 = 1_000;
/// 创业板 / 科创板涨跌幅
pub const GROWTH_LIMIT_BPS: i64 = 2_000;
/// 北交所涨跌幅
pub const BSE_LIMIT_BPS: i64 = 3_000;

/// 解析完整符号（`sh600519` / `sz300750` / `bj920001`）对应的 A 股板块。
/// 指数、ETF、B 股、港美股一律返回 `None`。
pub fn board_of(symbol: &str) -> Option<Board> {
    let symbol = symbol.trim();
    if symbol.len() != 8 || !symbol.is_ascii() {
        return None;
    }
    let (prefix, code) = symbol.split_at(2);
    if !code.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let board = Board::from_code(code);
    let recognized = matches!(
        (prefix, board),
        ("sh", Board::ShMain | Board::Star)
            | ("sz", Board::SzMain | Board::ChiNext)
            | ("bj", Board::Bse)
    );
    recognized.then_some(board)
}

/// 简称是否带 ST / *ST 标记。2026-07-06 起主板 ST 涨跌幅已与普通股票并轨为 10%，
/// 本函数仍保留：注册制板块的 ST 需要排除在「特殊状态」判断之外，且便于日后规则回摆。
pub fn is_st(name: &str) -> bool {
    name.trim().to_uppercase().contains("ST")
}

/// 退市整理期（简称含「退」）。首日不设涨跌幅、其后 10%，与普通股票不同。
pub fn is_delisting(name: &str) -> bool {
    name.contains('退')
}

/// 上市未满 5 个交易日的次新股：首日简称前加 `N`，其后 4 日加 `C`。
/// 这 5 个交易日不设涨跌幅限制，无法用前收盘价推断可成交区间。
pub fn is_new_listing(name: &str) -> bool {
    let name = name.trim();
    matches!(name.chars().next(), Some('N') | Some('C')) && name.chars().count() > 1
}

/// 返回 `(买入最小数量, 买入递增单位)`。
pub fn buy_lot(symbol: &str) -> (i64, i64) {
    match board_of(symbol) {
        // 科创板：200 股起，超出部分 1 股递增
        Some(Board::Star) => (200, 1),
        // 北交所：100 股起，超出部分 1 股递增
        Some(Board::Bse) => (100, 1),
        // 沪深主板、创业板：100 股整数倍
        _ => (100, 100),
    }
}

/// 校验买入数量是否符合该板块的申报单位。
pub fn validate_buy_quantity(symbol: &str, quantity: i64) -> Result<(), String> {
    if quantity <= 0 {
        return Err("委托数量必须大于 0".into());
    }
    let (minimum, step) = buy_lot(symbol);
    if quantity < minimum {
        return Err(match board_of(symbol) {
            Some(Board::Star) => "科创板买入至少 200 股".to_string(),
            Some(Board::Bse) => "北交所买入至少 100 股".to_string(),
            _ => "买入数量必须是 100 股的整数倍".to_string(),
        });
    }
    if (quantity - minimum) % step != 0 {
        return Err(match board_of(symbol) {
            Some(Board::Star) => "科创板超过 200 股的部分必须以 1 股递增".to_string(),
            Some(Board::Bse) => "北交所超过 100 股的部分必须以 1 股递增".to_string(),
            _ => "买入数量必须是 100 股的整数倍".to_string(),
        });
    }
    Ok(())
}

/// 校验卖出数量与持仓的关系：允许零股，但零股必须一次性全部卖出，且不得超过持仓。
/// 这是「实际不能卖的不能卖出去」里与 T+1 无关的那一半。
pub fn validate_sell_position(quantity: i64, position: i64) -> Result<(), String> {
    if quantity <= 0 {
        return Err("委托数量必须大于 0".into());
    }
    if quantity > position {
        return Err("卖出数量超过持仓总量".into());
    }
    // A 股允许卖出零股，但持有一手以内时必须一次性卖出全部余额。
    if position < 100 && quantity != position {
        return Err("持仓不足 100 股时，零股必须一次性全部卖出".into());
    }
    Ok(())
}

/// 完整校验卖出：先看持仓结构（[`validate_sell_position`]），再看 T+1 可用数量。
pub fn validate_sell_quantity(quantity: i64, position: i64, available: i64) -> Result<(), String> {
    validate_sell_position(quantity, position)?;
    if quantity > available {
        return Err("可用持仓不足（T+1 持仓当日不可卖）".into());
    }
    Ok(())
}

/// 按板块与 ST 状态返回当日涨跌幅（基点）。
///
/// **这是全项目唯一的涨跌幅阈值来源**：模拟撮合（[`ensure_simulatable`]）、
/// 全市场筛选器的「一字板 / 涨停」判定都走这里，避免两处各写一份而漂移。
/// 注册制板块（创业板 / 科创板 / 北交所）的 ST 不改变比例。
pub fn limit_bps_for(board: Board, is_st: bool) -> i64 {
    match board {
        Board::Star | Board::ChiNext => GROWTH_LIMIT_BPS,
        Board::Bse => BSE_LIMIT_BPS,
        Board::ShMain | Board::SzMain if is_st => ST_MAIN_LIMIT_BPS,
        _ => MAIN_LIMIT_BPS,
    }
}

/// 该标的当日适用的涨跌幅（基点）。特殊状态返回 `Err`，调用方必须拒绝模拟。
pub fn ensure_simulatable(symbol: &str, name: &str) -> Result<i64, String> {
    let board = board_of(symbol)
        .ok_or_else(|| format!("{symbol} 不是沪深北 A 股股票，模拟不支持该品种"))?;
    if is_delisting(name) {
        return Err(format!(
            "{name} 处于退市整理期，涨跌幅规则与普通股票不同，模拟不支持"
        ));
    }
    if is_new_listing(name) {
        return Err(format!(
            "{name} 上市未满 5 个交易日，无涨跌幅限制，无法用前收盘价建模，模拟不支持"
        ));
    }
    Ok(limit_bps_for(board, is_st(name)))
}

/// 撮合价最小变动单位 0.01 元：把定点价格（1/10000 元）四舍五入到分。
pub fn round_to_cent(scaled_price: i64) -> i64 {
    ((scaled_price + 50) / 100) * 100
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limit_rates_follow_board_conventions() {
        assert_eq!(ensure_simulatable("sh600519", "贵州茅台").unwrap(), 1_000);
        assert_eq!(ensure_simulatable("sz000001", "平安银行").unwrap(), 1_000);
        // 2026-07-06 起主板 ST / *ST 由 5% 放宽到 10%，与普通股票并轨。
        assert_eq!(ensure_simulatable("sh600519", "ST 红星").unwrap(), 1_000);
        assert_eq!(ensure_simulatable("sh600519", "*ST 红星").unwrap(), 1_000);
        assert_eq!(ensure_simulatable("sz000001", "*ST 平安").unwrap(), 1_000);
        // 注册制板块 ST 维持原比例
        assert_eq!(ensure_simulatable("sz300750", "ST 宁王").unwrap(), 2_000);
        assert_eq!(ensure_simulatable("sh688981", "中芯国际").unwrap(), 2_000);
        assert_eq!(ensure_simulatable("sz301001", "凯淳股份").unwrap(), 2_000);
        assert_eq!(ensure_simulatable("bj920001", "北证测试").unwrap(), 3_000);
        assert_eq!(ensure_simulatable("bj920001", "*ST 北证").unwrap(), 3_000);
    }

    #[test]
    fn threshold_source_is_shared_by_board_and_st_flags() {
        // 筛选器的一字板 / 涨停判定与模拟撮合必须同源，避免两处口径漂移。
        assert_eq!(limit_bps_for(Board::ShMain, false), 1_000);
        assert_eq!(limit_bps_for(Board::SzMain, true), 1_000);
        assert_eq!(limit_bps_for(Board::ChiNext, true), 2_000);
        assert_eq!(limit_bps_for(Board::Star, true), 2_000);
        assert_eq!(limit_bps_for(Board::Bse, true), 3_000);
    }

    #[test]
    fn special_states_are_refused_instead_of_guessed() {
        assert!(ensure_simulatable("sh600519", "退市红星").unwrap_err().contains("退市整理期"));
        assert!(ensure_simulatable("sz300750", "N 新能").unwrap_err().contains("无涨跌幅"));
        assert!(ensure_simulatable("sh688981", "C 中芯").unwrap_err().contains("无涨跌幅"));
        assert!(ensure_simulatable("sh510300", "沪深300ETF").is_err());
        assert!(ensure_simulatable("usAAPL", "苹果").is_err());
    }

    #[test]
    fn buy_lot_differs_between_main_growth_and_bse() {
        assert_eq!(buy_lot("sh600519"), (100, 100));
        assert_eq!(buy_lot("sz300750"), (100, 100));
        assert_eq!(buy_lot("sh688981"), (200, 1));
        assert_eq!(buy_lot("bj920001"), (100, 1));

        assert!(validate_buy_quantity("sh600519", 100).is_ok());
        assert!(validate_buy_quantity("sh600519", 150).is_err());
        assert!(validate_buy_quantity("sh688981", 100).is_err());
        assert!(validate_buy_quantity("sh688981", 200).is_ok());
        assert!(validate_buy_quantity("sh688981", 201).is_ok());
        assert!(validate_buy_quantity("bj920001", 101).is_ok());
        assert!(validate_buy_quantity("bj920001", 99).is_err());
    }

    #[test]
    fn odd_lots_may_be_sold_but_only_in_full() {
        assert!(validate_sell_quantity(70, 70, 70).is_ok());
        assert!(validate_sell_quantity(35, 70, 70).unwrap_err().contains("一次性"));
        assert!(validate_sell_quantity(200, 200, 100).unwrap_err().contains("T+1"));
        assert!(validate_sell_quantity(300, 200, 200).unwrap_err().contains("超过持仓"));
        assert!(validate_sell_quantity(120, 500, 500).is_ok());
    }

    #[test]
    fn fills_round_to_one_cent() {
        assert_eq!(round_to_cent(3_400_004), 3_400_000);
        assert_eq!(round_to_cent(3_400_050), 3_400_100);
        assert_eq!(round_to_cent(3_400_049), 3_400_000);
    }
}
