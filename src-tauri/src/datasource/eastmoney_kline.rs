// src-tauri/src/datasource/eastmoney_kline.rs
// 东方财富日 K 线适配器 —— 为量化层（quant::scorer）提供历史 K 线。
//
// 接口：`push2his.eastmoney.com/api/qt/stock/kline/get`
// 关键点（真机实测）：
// - 字段顺序：`日期,开盘,收盘,最高,最低,成交量(手),成交额(元),振幅,涨跌幅,涨跌额,换手率(%)`
//   ⚠️ 东财把「收盘」放在「开盘」之后，不是常见的 OHLC 顺序，解析时注意下标。
//   ⚠️ `KLineData.volume` 全通道统一为**手**，`turnover` 为**换手率 %**（只有本通道拿得到，
//   腾讯/新浪恒为 0。v1.5.1 移除筹码分布后，已没有功能依赖它 —— 保留字段是为了
//   不破坏三个通道的统一解析口径，将来要用换手率时直接读即可）。
// - `fqt=1` 为前复权，早期价格可能为负（高分红送转导致），属正常，指标计算不受影响。
// - `beg=0&end=20500101` 表示取全区间，`lmt=N` 限制条数（返回按时间升序）。
// - 复用了 `eastmoney_universe::universe_client()`（带 `.no_proxy()`），避免系统代理切断 TLS。
//
// ⚠️ 实测警告：`push2his.eastmoney.com/api/qt/stock/kline/get` 在部分网络下
// **整条路径被阻断**（HTTP 000，TLS 重协商后断开），而同域的 `stock/get` 正常。
// 业务代码**不要直接调用本模块**，应走 `crate::datasource::kline::fetch_daily_kline`，
// 它会依次回退腾讯 → 新浪 → 东财；本模块仅作为第三顺位通道保留。

use crate::domain::KLineData;

const KLINE_URL: &str = "https://push2his.eastmoney.com/api/qt/stock/kline/get";

/// 完整符号（`sh600519`/`sz000001`/`bj920xxx`）→ 东财 secid（`1.600519`/`0.000001`）
pub fn secid_from_full_symbol(symbol: &str) -> Result<String, String> {
    if symbol.len() < 3 {
        return Err(format!("无效的股票符号: {symbol}"));
    }
    let (prefix, code) = symbol.split_at(2);
    let secid = match prefix {
        "sh" => format!("1.{code}"),
        "sz" | "bj" => format!("0.{code}"),
        _ => return Err(format!("无法识别的交易所前缀: {prefix}")),
    };
    Ok(secid)
}

/// 拉取日 K（前复权）。返回按时间升序的 K 线序列。
pub async fn fetch_daily_kline(symbol: &str, count: u32) -> Result<Vec<KLineData>, String> {
    let secid = secid_from_full_symbol(symbol)?;
    let client = crate::datasource::eastmoney_universe::universe_client();
    let url = format!(
        "{KLINE_URL}?secid={secid}&fields1=f1,f2,f3,f4,f5,f6&fields2=f51,f52,f53,f54,f55,f56,f57,f58,f59,f60,f61&klt=101&fqt=1&beg=0&end=20500101&lmt={count}"
    );
    let text = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("请求日K失败: {e}"))?
        .text()
        .await
        .map_err(|e| format!("读取日K响应失败: {e}"))?;
    parse_kline(&text)
}

/// 解析东财 kline 返回的 JSON 文本
pub fn parse_kline(text: &str) -> Result<Vec<KLineData>, String> {
    let json: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("解析JSON失败: {e}"))?;
    let arr = json["data"]["klines"]
        .as_array()
        .ok_or_else(|| format!("返回中无 klines 字段: {}", &text[..text.len().min(200)]))?;

    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let Some(s) = item.as_str() else { continue };
        let parts: Vec<&str> = s.split(',').collect();
        if parts.len() < 7 {
            continue;
        }
        let f = |idx: usize| -> f64 { parts[idx].parse::<f64>().unwrap_or(0.0) };
        let vol_hands = parts[5].parse::<u64>().unwrap_or(0);
        // 第 11 个字段（下标 10）才是换手率 %。下标 6 是成交额（元）——
        // 这里**曾经**把成交额当成换手率填进去，已修正；字段缺失时记 0（由调用方决定是否兜底推）。
        let turnover = parts
            .get(10)
            .and_then(|v| v.trim().parse::<f64>().ok())
            .unwrap_or(0.0);
        out.push(KLineData {
            date: parts[0].to_string(),
            open: f(1),
            close: f(2),
            high: f(3),
            low: f(4),
            // ⚠️ 单位统一为「手」：腾讯与新浪两个通道给的都是手，这里若 ×100 换成股，
            // 同一只股票走不同通道就会得到相差 100 倍的成交量。量比是比值看不出来，
            // 但换手率 / 筹码这类绝对量会直接算错——所以必须和那两个通道对齐。
            volume: vol_hands,
            turnover,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secid_mapping() {
        assert_eq!(secid_from_full_symbol("sh600519").unwrap(), "1.600519");
        assert_eq!(secid_from_full_symbol("sz000001").unwrap(), "0.000001");
        assert_eq!(secid_from_full_symbol("bj920185").unwrap(), "0.920185");
        assert!(secid_from_full_symbol("hk00700").is_err());
    }

    #[test]
    fn parse_kline_orders_ohlc_correctly() {
        // 东财顺序：日期,开盘,收盘,最高,最低,成交量(手),成交额(元),振幅,涨跌幅,涨跌额,换手率(%)
        let text = r#"{"data":{"code":"600519","name":"贵州茅台","klines":[
            "2024-01-02,1700.00,1710.00,1720.00,1690.00,406318,1410347000.00,0.83,0.26,-0.31,0.23",
            "2024-01-03,1710.00,1705.00,1715.00,1700.00,300000,1020000000.00,0.50,-0.29,-0.05,0.30"
        ]}}"#;
        let k = parse_kline(text).unwrap();
        assert_eq!(k.len(), 2);
        assert_eq!(k[0].date, "2024-01-02");
        assert_eq!(k[0].open, 1700.0);
        assert_eq!(k[0].close, 1710.0);
        assert_eq!(k[0].high, 1720.0);
        assert_eq!(k[0].low, 1690.0);
        // 成交量保持「手」，不换算成股（与腾讯/新浪通道对齐）
        assert_eq!(k[0].volume, 406318);
        // turnover 是换手率(%)，取自第 11 个字段，**不是**第 7 个的成交额
        assert_eq!(k[0].turnover, 0.23);
        assert_eq!(k[1].turnover, 0.30);
    }

    #[test]
    fn parse_kline_tolerates_missing_turnover_column() {
        // 老接口/裁剪过的响应可能只有前 7 列：不该整个丢弃，只把换手率记 0
        let text = r#"{"data":{"klines":["2024-01-02,1700.00,1710.00,1720.00,1690.00,406318,1410347000.00"]}}"#;
        let k = parse_kline(text).unwrap();
        assert_eq!(k.len(), 1);
        assert_eq!(k[0].turnover, 0.0);
        assert_eq!(k[0].volume, 406318);
    }

    #[test]
    fn parse_kline_empty_returns_empty() {
        let k = parse_kline(r#"{"data":null}"#).unwrap_or_default();
        assert!(k.is_empty());
    }
}
