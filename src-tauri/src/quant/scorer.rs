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
    /// 关键指标快照（None 表示数据不足未算出）
    pub close: Option<f64>,
    pub ma5: Option<f64>,
    pub ma10: Option<f64>,
    pub ma20: Option<f64>,
    pub ma60: Option<f64>,
    pub macd_dif: Option<f64>,
    pub macd_dea: Option<f64>,
    pub rsi12: Option<f64>,
    pub kdj_k: Option<f64>,
    pub kdj_d: Option<f64>,
    pub kdj_j: Option<f64>,
    pub boll_upper: Option<f64>,
    pub boll_mid: Option<f64>,
    pub boll_lower: Option<f64>,
    /// 20 日 / 60 日动量（涨跌幅，0.1 = +10%）
    pub momentum20: Option<f64>,
    pub momentum60: Option<f64>,
    /// 量比（今日量 / 过去 5 日均量）
    pub volume_ratio: Option<f64>,
}

/// 至少需要多少根日 K 才能输出完整评分（MA60 需要 60 根）
const MIN_BARS: usize = 60;

/// 各因子权重（合计 1.0）
const W_TREND: f64 = 0.30;
const W_MACD: f64 = 0.20;
const W_MOMENTUM: f64 = 0.15;
const W_VOLUME: f64 = 0.15;
const W_RSI: f64 = 0.10;
const W_KDJ: f64 = 0.10;

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

/// MACD 因子
fn score_macd(dif: f64, dea: f64) -> (f64, String) {
    if dif > dea && dif > 0.0 {
        (100.0, "DIF 在零轴上方且金叉向上".into())
    } else if dif > dea {
        (70.0, "金叉但仍在零轴下方（反弹初期）".into())
    } else {
        (30.0, "死叉，动能走弱".into())
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

/// RSI 因子（12 日），偏好多头区、警惕超买超卖
fn score_rsi(rsi: f64) -> (f64, String) {
    if rsi >= 50.0 && rsi <= 70.0 {
        (100.0, "RSI 处于多头区（50~70）".into())
    } else if (rsi >= 40.0 && rsi < 50.0) || (rsi > 70.0 && rsi <= 80.0) {
        (60.0, "RSI 中性偏强/偏热（40~50 或 70~80）".into())
    } else if rsi >= 30.0 && rsi < 40.0 {
        (45.0, "RSI 偏弱（30~40）".into())
    } else if rsi > 80.0 {
        (30.0, "RSI 超买（>80），回调风险".into())
    } else {
        (40.0, "RSI 超卖（<30），弱势或反弹前夜".into())
    }
}

/// KDJ 因子
fn score_kdj(k: f64, d: f64, j: f64) -> (f64, String) {
    if k > d && j < 100.0 {
        (100.0, "KDJ 金叉且未超买".into())
    } else if k > d {
        (50.0, "KDJ 金叉但 J 已超买".into())
    } else if j < 0.0 {
        (35.0, "KDJ 死叉且超卖".into())
    } else {
        (30.0, "KDJ 死叉".into())
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
    let highs: Vec<f64> = klines.iter().map(|k| k.high).collect();
    let lows: Vec<f64> = klines.iter().map(|k| k.low).collect();
    let volumes: Vec<f64> = klines.iter().map(|k| k.volume as f64).collect();

    let ma5 = latest_finite(&indicators::sma(&closes, 5));
    let ma10 = latest_finite(&indicators::sma(&closes, 10));
    let ma20 = latest_finite(&indicators::sma(&closes, 20));
    let ma60 = latest_finite(&indicators::sma(&closes, 60));

    let macd = indicators::macd(&closes, 12, 26, 9);
    let dif = latest_finite(&macd.dif);
    let dea = latest_finite(&macd.dea);

    let rsi12 = latest_finite(&indicators::rsi(&closes, 12));

    let kdj = indicators::kdj(&highs, &lows, &closes, 9);
    let k = latest_finite(&kdj.k);
    let d = latest_finite(&kdj.d);
    let j = latest_finite(&kdj.j);

    let boll = indicators::boll(&closes, 20, 2.0);
    let boll_upper = latest_finite(&boll.upper);
    let boll_mid = latest_finite(&boll.mid);
    let boll_lower = latest_finite(&boll.lower);

    let mom20 = latest_finite(&indicators::momentum(&closes, 20));
    let mom60 = latest_finite(&indicators::momentum(&closes, 60));
    let vr = latest_finite(&indicators::volume_ratio(&volumes, 5));

    // 关键指标若缺失（理论上 >= MIN_BARS 不会），则返回 None 避免输出半截结果
    let (ma5, ma10, ma20, ma60, dif, dea, rsi12, k, d, j, mom20) =
        (ma5?, ma10?, ma20?, ma60?, dif?, dea?, rsi12?, k?, d?, j?, mom20?);

    let close = *closes.last()?;

    let mut factors = Vec::new();
    let mut total = 0.0;

    let (s, note) = score_trend(ma5, ma10, ma20, ma60, close);
    total += s * W_TREND;
    factors.push(FactorScore { name: "趋势".into(), score: s, weight: W_TREND, note });

    let (s, note) = score_macd(dif, dea);
    total += s * W_MACD;
    factors.push(FactorScore { name: "MACD".into(), score: s, weight: W_MACD, note });

    let (s, note) = score_momentum(mom20);
    total += s * W_MOMENTUM;
    factors.push(FactorScore { name: "动量".into(), score: s, weight: W_MOMENTUM, note });

    // 量能因子依赖 vr，可能为 None（早期数据不足），缺失时按中性 55 分处理
    let (s, note) = match vr {
        Some(v) => score_volume(v),
        None => (55.0, "量比数据不足".to_string()),
    };
    total += s * W_VOLUME;
    factors.push(FactorScore { name: "量能".into(), score: s, weight: W_VOLUME, note });

    let (s, note) = score_rsi(rsi12);
    total += s * W_RSI;
    factors.push(FactorScore { name: "RSI".into(), score: s, weight: W_RSI, note });

    let (s, note) = score_kdj(k, d, j);
    total += s * W_KDJ;
    factors.push(FactorScore { name: "KDJ".into(), score: s, weight: W_KDJ, note });

    let total_score = (total * 10.0).round() / 10.0; // 保留 1 位小数

    Some(StockAnalysis {
        total_score,
        verdict: verdict_for(total_score),
        factors,
        close: Some(close),
        ma5: Some(ma5),
        ma10: Some(ma10),
        ma20: Some(ma20),
        ma60: Some(ma60),
        macd_dif: Some(dif),
        macd_dea: Some(dea),
        rsi12: Some(rsi12),
        kdj_k: Some(k),
        kdj_d: Some(d),
        kdj_j: Some(j),
        boll_upper,
        boll_mid,
        boll_lower,
        momentum20: Some(mom20),
        momentum60: mom60,
        volume_ratio: vr,
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
    fn rsi_factor_boundaries() {
        assert_eq!(score_rsi(60.0).0, 100.0);
        assert_eq!(score_rsi(90.0).0, 30.0);
        assert_eq!(score_rsi(20.0).0, 40.0);
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
