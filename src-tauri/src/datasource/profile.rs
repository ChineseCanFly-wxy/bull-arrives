// src-tauri/src/datasource/profile.rs
//! 个股股本信息。
//!
//! 只有一件事：拿到**流通股本**。
//!
//! # 当前没有调用方（v1.5.1 起）
//!
//! 它原本是给筹码分布算换手率用的 —— 腾讯/新浪两个日K通道都不给历史换手率，
//! 只能用 `换手率 = 成交量 ÷ 流通股本` 反推。v1.5.1 移除了筹码分布
//! （A 股没有公开的筹码原始数据，自算的结果与各软件对不上、反而误导），
//! 这条请求也就跟着从 `analyze_stock` 里撤掉了 —— 顺带少一次网络往返。
//!
//! **代码保留**：接口已实测可用（见下方测试的固化响应），是"这台机器上确实拿得到
//! 流通股本"的证据；以后要做换手率反推、流通市值校验之类的需求可以直接用，
//! 不必重新摸一遍东财的字段（f85）。真用不上时删掉这个文件即可，没有别处引用它。
//!
//! 走东财 `push2/api/qt/stock/get`：`kline.rs` 的模块注释里记着
//! **同域的这条路径是通的**（被阻断的是 `clist` 与 `push2his` 的 kline 路径），
//! 所以它比日K通道更稳。
//!
//! 拿不到时**不要编**：返回 `Ok` 但字段为 `None`，由调用方决定降级方式。

const PROFILE_URL: &str = "https://push2.eastmoney.com/api/qt/stock/get";

/// 大盘/个股通用快照接口的超时。比日K短 —— 这只是个附带的元数据请求，
/// 拿不到就算了，不该拖住整次分析。
const PROFILE_TIMEOUT_SECS: u64 = 8;

/// 个股股本信息（单位：股）。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct StockProfile {
    /// 流通股本（股）。原本是给筹码分布算换手率用的（见模块头注释）。
    pub circulating_shares: Option<f64>,
    /// 总股本（股）
    pub total_shares: Option<f64>,
}

/// 拉取股本信息。网络/接口失败返回 `Err`；接口通了但字段缺失则字段为 `None`。
pub async fn fetch_profile(symbol: &str) -> Result<StockProfile, String> {
    let secid = crate::datasource::eastmoney_kline::secid_from_full_symbol(symbol)?;
    let client = crate::datasource::eastmoney_universe::universe_client();
    // f84 总股本 / f85 流通股 / f116 总市值 / f117 流通市值
    let url = format!("{PROFILE_URL}?secid={secid}&fields=f57,f58,f84,f85,f116,f117");
    let text = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(PROFILE_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|e| format!("请求个股信息失败: {e}"))?
        .text()
        .await
        .map_err(|e| format!("读取个股信息失败: {e}"))?;
    parse_profile(&text)
}

/// 解析东财 `stock/get` 返回。字段可能是数字，也可能是 `"-"` 这类占位字符串。
pub fn parse_profile(text: &str) -> Result<StockProfile, String> {
    let json: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("个股信息 JSON 解析失败: {e}"))?;
    let data = json
        .get("data")
        .ok_or_else(|| "个股信息返回中无 data 节点".to_string())?;
    if data.is_null() {
        return Err("个股信息返回 data 为空".to_string());
    }

    let pick = |key: &str| -> Option<f64> {
        match data.get(key) {
            Some(serde_json::Value::Number(n)) => n.as_f64(),
            Some(serde_json::Value::String(s)) => s.trim().parse::<f64>().ok(),
            _ => None,
        }
        // 东财用 "-" / 0 表示缺失 —— 0 股本的股票不存在，当成没拿到更诚实
        .filter(|v| v.is_finite() && *v > 0.0)
    };

    Ok(StockProfile {
        circulating_shares: pick("f85"),
        total_shares: pick("f84"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_share_counts() {
        let text = r#"{"rc":0,"data":{"f57":"600519","f58":"贵州茅台",
            "f84":1256197800,"f85":1256197800,"f116":1599000000000,"f117":1599000000000}}"#;
        let p = parse_profile(text).unwrap();
        assert_eq!(p.circulating_shares, Some(1_256_197_800.0));
        assert_eq!(p.total_shares, Some(1_256_197_800.0));
    }

    #[test]
    fn placeholder_dash_is_treated_as_missing() {
        // 指数 / 无股本数据的标的会返回 "-"，不能当成 0 传下去
        let text = r#"{"data":{"f57":"000001","f84":"-","f85":"-"}}"#;
        let p = parse_profile(text).unwrap();
        assert_eq!(p.circulating_shares, None);
        assert_eq!(p.total_shares, None);
    }

    #[test]
    fn zero_is_treated_as_missing() {
        let text = r#"{"data":{"f84":0,"f85":0}}"#;
        let p = parse_profile(text).unwrap();
        assert_eq!(p.circulating_shares, None);
    }

    #[test]
    fn null_data_is_an_error() {
        assert!(parse_profile(r#"{"rc":0,"data":null}"#).is_err());
        assert!(parse_profile(r#"{}"#).is_err());
        assert!(parse_profile("not json").is_err());
    }
}
