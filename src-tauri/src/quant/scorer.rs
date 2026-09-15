// src-tauri/src/quant/scorer.rs
// 个股技术评分引擎（漏斗 L3 的核心）。
//
// 输入日 K 序列，复用 indicators 计算多因子，加权汇总为 0–100 综合分。
// 评分口径刻意保守：只做「方向 + 强弱」的规则化打分，不承诺收益，
// 供 AI 分析（L4）与人工决策参考。所有阈值集中为常量，便于回测调参。

use crate::domain::KLineData;
use super::indicators::{self, latest_finite};

/// 单个因子的评分结果
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FactorScore {
    /// 因子名（中文，直接展示给用户）
    pub name: String,
    /// 0–100 分
    pub score: f64,
    /// 权重（合计 1.0）
    pub weight: f64,
    /// 一句话说明（为什么给这个分）
    pub note: String,
}

/// 个股技术分析结果（快照 + 综合结论）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StockAnalysis {
    /// 综合评分 0–100
    pub total_score: f64,
    /// 结论：强烈关注 / 关注 / 中性 / 回避
    pub verdict: String,
    /// 各因子明细
    pub factors: Vec<FactorScore>,
    /// 关键指标快照（None 表示数据不足未算出）。
    ///
    /// ⚠️ 只放**彼此不重复**的指标。下面几项是被刻意删掉的，别再加回来：
    /// - `ma5` / `ma10`：与 MA20 高度共线，MA5 一周平均噪声极大
    ///   （趋势因子的判定内部仍然要用它们，只是不再往界面上丢）
    /// - `macd_dea`：就是 DIF 的 9 日 EMA，两者永远贴着走，看一个够了
    /// - `boll_mid`：**数学上恒等于 MA20**，同一个数字显示两遍
    /// - `kdj_k/d/j`：J = 3K−2D 纯派生，且整个 KDJ 的判据只有 4 档
    pub close: Option<f64>,
    pub ma20: Option<f64>,
    pub ma60: Option<f64>,
    pub macd_dif: Option<f64>,
    pub rsi12: Option<f64>,
    pub boll_upper: Option<f64>,
    pub boll_lower: Option<f64>,
    /// 20 日 / 60 日动量（涨跌幅，0.1 = +10%）
    pub momentum20: Option<f64>,
    pub momentum60: Option<f64>,
    /// 量比（今日量 / 过去 5 日均量）
    pub volume_ratio: Option<f64>,
    /// 操作计划（买点 / 止损 / 止盈 / 仓位）。
    ///
    /// ⚠️ 由 `analyze_stock` 命令在评分之后补上 —— 评分本身只吃 K 线，
    /// 不知道用户当前用的是哪条策略。缺省为 `None`（只做纯评分时）。
    #[serde(default)]
    pub trade_plan: Option<super::playbook::TradePlan>,
    /// 该规则在这只股票自身历史上的回测结果。缺省为 `None`。
    #[serde(default)]
    pub backtest: Option<super::backtest::BacktestStats>,
    /// 筹码分布**估算**（获利盘 / 平均成本 / 成本区间 / 筹码峰）。
    ///
    /// ⚠️ 同样由 `analyze_stock` 补上 —— 它需要流通股本（换手率要用），
    /// 纯评分路径拿不到。缺省 `None`。
    #[serde(default)]
    pub chips: Option<super::chips::ChipDistribution>,
    /// 支撑位与压力位，**已按当前交易规则加权排序**。缺省为空。
    #[serde(default)]
    pub levels: Vec<super::levels::PriceLevel>,
    /// 按**当前市场状态**自动匹配到的交易规则，以及三条规则各自的状态。
    ///
    /// ⚠️ 这是状态匹配的结果，**不是**「历史上哪条规则最赚」——
    /// 后者实测选对率只有 40%（随机挑是 33%），理由见 `playbook::match_rule`。
    #[serde(default)]
    pub rule_match: Option<super::playbook::RuleMatch>,
}

/// 至少需要多少根日 K 才能输出完整评分（MA60 需要 60 根）
const MIN_BARS: usize = 60;

/// 各因子权重（合计 1.0）。
///
/// ⚠️ 权重按**彼此独立的维度**分配，不是每个指标平均分一点。
///
/// 实测（20 只股票 / 8600 个交易日样本）四类信息两两相关性：
/// 趋势↔RSI 0.76、趋势↔MACD 0.71、趋势↔动量 0.70、RSI↔动量 0.69；
/// 而**量能与所有因子都是 0.00**。
///
/// 也就是说「趋势 / MACD / 动量 / 旧版 RSI」原本是同一个维度的四种说法，
/// 旧版权重却把它们加起来给了 0.75 —— 等于把趋势偷偷放大了四倍，
/// 分数看起来是「6 因子综合」，实际主要在回答「均线多头不强」。
///
/// 现在：
/// - 只留最能表达趋势结构的那一个（趋势本身），删掉 MACD 与 KDJ
/// - RSI 改为**只判超买超卖极值** —— 那才是它独有的信息；
///   旧版奖励「50–70 多头区」，而那正是趋势因子在说的话
/// - 量能是唯一真正独立的维度，权重从 0.15 提到 0.25
const W_TREND: f64 = 0.40;
const W_VOLUME: f64 = 0.25;
const W_MOMENTUM: f64 = 0.20;
const W_RSI: f64 = 0.15;

/// 趋势因子：均线多头排列程度
fn score_trend(ma5: f64, ma10: f64, ma20: f64, ma60: f64, close: f64) -> (f64, String) {
    let bullish = ma5 > ma10 && ma10 > ma20 && ma20 > ma60;
    let above20 = close > ma20;
    let above60 = close > ma60;
    if bullish && above20 {
        (100.0, "均线完全多头排列，且站上 MA20".into())
    } else if ma5 > ma20 && above20 {
        (78.0, "短期均线在中期之上，站上 MA20".into())
    } else if above20 {
        (58.0, "站上 MA20 但均线未完全多头".into())
    } else if above60 {
        (40.0, "跌破 MA20，仍守住 MA60".into())
    } else {
        (20.0, "空头排列，均线压制".into())
    }
}

/// 动量因子：20 日涨跌幅，偏好「健康上涨」，警惕超买
fn score_momentum(mom20: f64) -> (f64, String) {
    if mom20 > 0.0 && mom20 <= 0.15 {
        (100.0, "20 日涨幅温和（0~15%），趋势健康".into())
    } else if mom20 > 0.15 && mom20 <= 0.30 {
        (70.0, "20 日涨幅偏大（15~30%），注意节奏".into())
    } else if mom20 > 0.30 {
        (40.0, "20 日涨幅过大（>30%），短期超买".into())
    } else if mom20 >= -0.05 {
        (50.0, "近 20 日小幅回撤（0~-5%）".into())
    } else {
        (25.0, "近 20 日明显下跌（<-5%）".into())
    }
}

/// 量能因子：量比，偏好温和放量，警惕天量
fn score_volume(vr: f64) -> (f64, String) {
    if vr >= 1.2 && vr < 3.0 {
        (100.0, "温和放量（量比 1.2~3）".into())
    } else if vr >= 0.8 && vr < 1.2 {
        (70.0, "量能平稳（量比 0.8~1.2）".into())
    } else if vr >= 3.0 && vr < 5.0 {
        (55.0, "明显放量（量比 3~5），波动加剧".into())
    } else if vr >= 5.0 {
        (35.0, "天量（量比 ≥5），谨防冲高回落".into())
    } else {
        (40.0, "缩量（量比 <0.8），关注度低".into())
    }
}

/// RSI 因子（12 日）—— **只判超买超卖极值**。
///
/// 为什么不再奖励「50~70 多头区」：实测那套判据与趋势因子相关 0.76，
/// 等于把趋势维度又数了一遍。RSI 真正独有的信息是**极端值**；
/// 常态区间本就不构成买卖信号，给中性分才诚实。
fn score_rsi(rsi: f64) -> (f64, String) {
    if rsi > 85.0 {
        (20.0, "RSI 极度超买（>85），追高风险大".into())
    } else if rsi > 75.0 {
        (45.0, "RSI 超买（>75），短期动能透支".into())
    } else if rsi < 20.0 {
        (35.0, "RSI 极度超卖（<20），超跌但需等企稳信号，别直接接".into())
    } else if rsi < 30.0 {
        (50.0, "RSI 超卖（<30），偏弱".into())
    } else {
        (70.0, "RSI 处于常态区间（30~75），不构成买卖信号".into())
    }
}

/// 综合评分 → 结论
fn verdict_for(score: f64) -> String {
    if score >= 75.0 {
        "强烈关注".into()
    } else if score >= 60.0 {
        "关注".into()
    } else if score >= 45.0 {
        "中性".into()
    } else {
        "回避".into()
    }
}

/// 分析一只股票。数据不足（< MIN_BARS 根）返回 None。
pub fn analyze(klines: &[KLineData]) -> Option<StockAnalysis> {
    if klines.len() < MIN_BARS {
        return None;
    }

    let closes: Vec<f64> = klines.iter().map(|k| k.close).collect();
    let volumes: Vec<f64> = klines.iter().map(|k| k.volume as f64).collect();

    let ma5 = latest_finite(&indicators::sma(&closes, 5));
    let ma10 = latest_finite(&indicators::sma(&closes, 10));
    let ma20 = latest_finite(&indicators::sma(&closes, 20));
    let ma60 = latest_finite(&indicators::sma(&closes, 60));

    let macd = indicators::macd(&closes, 12, 26, 9);
    // DEA 不再下发（它只是 DIF 的 9 日 EMA），但 MACD 整体仍要算才能拿到 DIF
    let dif = latest_finite(&macd.dif);

    let rsi12 = latest_finite(&indicators::rsi(&closes, 12));

    // KDJ 已退出评分（判据只有 4 档、且与 RSI 表达的信息重叠），连算都不用算了
    let boll = indicators::boll(&closes, 20, 2.0);
    let boll_upper = latest_finite(&boll.upper);
    let boll_lower = latest_finite(&boll.lower);

    let mom20 = latest_finite(&indicators::momentum(&closes, 20));
    let mom60 = latest_finite(&indicators::momentum(&closes, 60));
    let vr = latest_finite(&indicators::volume_ratio(&volumes, 5));

    // 只对**判据真正用到**的指标做非空校验：用不到的指标不再拖累整次分析
    let (ma5, ma10, ma20, ma60, dif, rsi12, mom20) =
        (ma5?, ma10?, ma20?, ma60?, dif?, rsi12?, mom20?);

    let close = *closes.last()?;

    let mut factors = Vec::new();
    let mut total = 0.0;

    let (s, note) = score_trend(ma5, ma10, ma20, ma60, close);
    total += s * W_TREND;
    factors.push(FactorScore { name: "趋势".into(), score: s, weight: W_TREND, note });

    // 量能因子依赖 vr，可能为 None（早期数据不足），缺失时按中性 55 分处理
    let (s, note) = match vr {
        Some(v) => score_volume(v),
        None => (55.0, "量比数据不足".to_string()),
    };
    total += s * W_VOLUME;
    factors.push(FactorScore { name: "量能".into(), score: s, weight: W_VOLUME, note });

    let (s, note) = score_momentum(mom20);
    total += s * W_MOMENTUM;
    factors.push(FactorScore { name: "动量".into(), score: s, weight: W_MOMENTUM, note });

    let (s, note) = score_rsi(rsi12);
    total += s * W_RSI;
    factors.push(FactorScore { name: "RSI".into(), score: s, weight: W_RSI, note });

    let total_score = (total * 10.0).round() / 10.0; // 保留 1 位小数

    Some(StockAnalysis {
        total_score,
        verdict: verdict_for(total_score),
        factors,
        close: Some(close),
        ma20: Some(ma20),
        ma60: Some(ma60),
        macd_dif: Some(dif),
        rsi12: Some(rsi12),
        boll_upper,
        boll_lower,
        momentum20: Some(mom20),
        momentum60: mom60,
        volume_ratio: vr,
        // 评分只吃 K 线，派生不出「用户当前用哪条策略」；
        // 操作计划与回测由 analyze_stock 命令按策略补上（见 commands/analysis.rs）
        trade_plan: None,
        backtest: None,
        // 筹码与支撑压力位由 analyze_stock 按策略补上（见 commands/analysis.rs）
        chips: None,
        levels: Vec::new(),
        rule_match: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(close: f64) -> KLineData {
        KLineData {
            date: String::new(),
            open: close,
            high: close + 1.0,
            low: close - 1.0,
            close,
            volume: 1_000_000,
            turnover: 0.0,
        }
    }

    /// 生成一段缓步上涨的 90 根日 K（每根 +0.5%）
    fn uptrend_bars(n: usize) -> Vec<KLineData> {
        (0..n).map(|i| bar(10.0 * 1.005f64.powi(i as i32))).collect()
    }

    /// 生成一段缓步下跌的 90 根日 K（每根 -0.5%）
    fn downtrend_bars(n: usize) -> Vec<KLineData> {
        (0..n).map(|i| bar(10.0 * 0.995f64.powi(i as i32))).collect()
    }

    #[test]
    fn insufficient_data_returns_none() {
        assert!(analyze(&uptrend_bars(30)).is_none());
    }

    #[test]
    fn uptrend_scores_higher_than_downtrend() {
        let up = analyze(&uptrend_bars(90)).unwrap();
        let down = analyze(&downtrend_bars(90)).unwrap();
        assert!(
            up.total_score > down.total_score,
            "上涨应高于下跌：up={} down={}",
            up.total_score,
            down.total_score
        );
        assert!(up.total_score >= 60.0, "上涨趋势应至少「关注」档");
    }

    #[test]
    fn trend_factor_bullish_alignment() {
        let (s, _) = score_trend(15.0, 14.0, 13.0, 12.0, 16.0);
        assert_eq!(s, 100.0);
        let (s, _) = score_trend(11.0, 12.0, 13.0, 14.0, 10.0);
        assert_eq!(s, 20.0);
    }

    #[test]
    fn rsi_only_flags_extremes() {
        // 常态区间一律给中性分 —— 不再奖励「50~70 多头区」，
        // 那套判据与趋势因子相关 0.76，等于把趋势数第二遍
        assert_eq!(score_rsi(60.0).0, 70.0);
        assert_eq!(score_rsi(45.0).0, 70.0);
        assert_eq!(score_rsi(30.0).0, 70.0);
        // 只有极端值才产生偏离
        assert_eq!(score_rsi(90.0).0, 20.0);
        assert_eq!(score_rsi(78.0).0, 45.0);
        assert_eq!(score_rsi(15.0).0, 35.0);
        assert_eq!(score_rsi(25.0).0, 50.0);
    }

    #[test]
    fn factor_weights_sum_to_one() {
        let sum = W_TREND + W_VOLUME + W_MOMENTUM + W_RSI;
        assert!((sum - 1.0).abs() < 1e-9, "权重合计应为 1，实际 {sum}");
    }

    #[test]
    fn factors_are_independent_dimensions_only() {
        let a = analyze(&uptrend_bars(90)).unwrap();
        let names: Vec<&str> = a.factors.iter().map(|f| f.name.as_str()).collect();
        // 实测与趋势因子相关 0.71 的 MACD、判据只有 4 档的 KDJ，都已删
        assert!(!names.contains(&"MACD"), "MACD 与趋势因子共线，不该再出现在因子里");
        assert!(!names.contains(&"KDJ"), "KDJ 判据太粗，不该再出现在因子里");
        assert_eq!(names.len(), 4, "应只剩四个彼此独立的维度：{names:?}");
        // 量能是唯一与其他因子相关 0.00 的维度，必须在
        assert!(names.contains(&"量能"));
    }

    #[test]
    fn volume_factor_ranges() {
        assert_eq!(score_volume(2.0).0, 100.0);
        assert_eq!(score_volume(1.0).0, 70.0);
        assert_eq!(score_volume(6.0).0, 35.0);
    }

    #[test]
    fn verdict_thresholds() {
        assert_eq!(verdict_for(80.0), "强烈关注");
        assert_eq!(verdict_for(65.0), "关注");
        assert_eq!(verdict_for(50.0), "中性");
        assert_eq!(verdict_for(30.0), "回避");
    }
}
