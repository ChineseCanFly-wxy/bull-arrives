// src-tauri/src/datasource/network_smoke.rs
//! **联网冒烟测试**：用生产代码真实访问数据源，验证通道是否可用。
//!
//! 这些测试默认带 `#[ignore]`，不会在常规 `cargo test` 里跑（避免离线环境失败、
//! 也避免给数据源添压力）。需要排查「界面拉不到数据」时手动执行：
//!
//! ```text
//! cargo test --release --lib network_smoke -- --ignored --nocapture
//! ```
//!
//! 它们存在的意义：界面出问题时，能一眼区分是**数据通道挂了**还是**前端渲染问题**。

#![allow(clippy::print_stdout)]

use super::eastmoney_universe::{self, FilterCapabilities, FilterPreset, SnapshotSource};
use super::{kline, sina_universe};

/// 全市场快照：新浪通道必须能拿到完整市场
#[tokio::test]
#[ignore]
async fn sina_snapshot_returns_full_market() {
    let rows = sina_universe::fetch_market_snapshot(sina_universe::client())
        .await
        .expect("新浪全市场快照失败");
    println!("新浪快照行数 = {}", rows.len());
    println!("前 3 行:");
    for row in rows.iter().take(3) {
        println!(
            "  {} {} 价={} 涨跌={}% 成交额={} 换手={} 量比={}",
            row.code,
            row.name,
            row.price,
            row.change_pct,
            row.amount,
            row.turnover_rate,
            row.volume_ratio
        );
    }
    let mut counts = std::collections::BTreeMap::new();
    for row in &rows {
        *counts.entry(row.board.label().to_string()).or_insert(0) += 1;
    }
    println!("板块分布: {counts:?}");
    assert!(
        rows.len() > 3000,
        "全市场应超过 3000 只，实际 {}",
        rows.len()
    );
}

/// 自动通道选择：至少有一个通道可用
#[tokio::test]
#[ignore]
async fn auto_snapshot_picks_a_working_channel() {
    match eastmoney_universe::fetch_snapshot_auto(None).await {
        Ok((rows, source)) => {
            println!("通道 = {} 行数 = {}", source.label(), rows.len());
            assert!(!rows.is_empty());
        }
        Err(error) => panic!("两个通道都失败: {error}"),
    }
}

/// 日K：各板块都要能拿到足够长的历史
#[tokio::test]
#[ignore]
async fn kline_covers_all_boards() {
    let cases = [
        ("sh600519", "沪主板-贵州茅台", 60usize),
        ("sz000001", "深主板-平安银行", 60),
        ("sz300750", "创业板-宁德时代", 60),
        ("sh688111", "科创板-金山办公", 60),
        ("bj920000", "北交所-安徽凤凰", 60),
    ];
    let mut failures = Vec::new();
    for (symbol, label, min_bars) in cases {
        match kline::fetch_daily_kline_with_source(symbol, 250).await {
            Ok((rows, source)) => {
                let last = rows.last().map(|k| k.close).unwrap_or(0.0);
                println!(
                    "{label:<20} {symbol} -> {:<8} {} 根  最新收盘={last}",
                    source.label(),
                    rows.len()
                );
                if rows.len() < min_bars {
                    failures.push(format!("{label} 只有 {} 根", rows.len()));
                }
            }
            Err(error) => failures.push(format!("{label} 失败: {error}")),
        }
    }
    assert!(failures.is_empty(), "日K 通道存在缺口: {failures:?}");
}

/// **最关键的一条**：用真实快照跑一遍所有内置预设，确认不会筛出 0 只。
///
/// 这条测试直接对应「点了筛选什么都没有」的问题：新浪通道不提供量比，
/// 若字段能力判定失效，4 套带量比条件的预设会全部筛空。
#[tokio::test]
#[ignore]
async fn presets_match_something_on_real_snapshot() {
    let (rows, source) = eastmoney_universe::fetch_snapshot_auto(None)
        .await
        .expect("快照失败");
    let caps = FilterCapabilities::for_source(source);
    println!(
        "通道 = {}  全市场 = {} 只  量比可用 = {}",
        source.label(),
        rows.len(),
        caps.volume_ratio
    );

    for preset in FilterPreset::ALL {
        let filter = preset.build();
        let matched = filter.apply_with(&rows, caps);
        let skipped = filter.skipped_conditions(caps);
        println!(
            "  {:<8} 命中 {:>5} 只  忽略条件={:?}",
            preset.label(),
            matched.len(),
            skipped
        );
        assert!(
            !matched.is_empty(),
            "预设「{}」在真实数据上筛出 0 只 —— 筛选条件与数据源字段不匹配",
            preset.label()
        );
    }
}

/// 通道降级行为：即使指定一个不可用的通道，也应自动落到可用通道
#[tokio::test]
#[ignore]
async fn falls_back_when_preferred_channel_is_dead() {
    // 东财在部分网络被阻断；即使显式指定它，也应回退到新浪
    let result = eastmoney_universe::fetch_snapshot_auto(Some(SnapshotSource::Eastmoney)).await;
    match result {
        Ok((rows, source)) => {
            println!(
                "指定东财 -> 实际通道 = {} 行数 = {}",
                source.label(),
                rows.len()
            );
            assert!(!rows.is_empty());
        }
        Err(error) => println!("两个通道都不可用（可接受，取决于网络）: {error}"),
    }
}

/// 原始 HTTP 探针：把新浪两个接口的**原始响应**打出来，用于定位
/// 「curl 能通、Rust 拿不到」这类差异（状态码 / 响应头 / 响应体长度）。
#[tokio::test]
#[ignore]
async fn raw_probe_sina_endpoints() {
    let client = sina_universe::client();
    let count_url = "https://vip.stock.finance.sina.com.cn/quotes_service/api/json_v2.php/\
                     Market_Center.getHQNodeStockCount?node=hs_a";
    probe(client, "总数接口", count_url).await;

    let page_url = "https://vip.stock.finance.sina.com.cn/quotes_service/api/json_v2.php/\
                    Market_Center.getHQNodeData?page=1&num=100&sort=symbol&asc=1&node=hs_a";
    probe(client, "列表第1页", page_url).await;

    // 并发压力：模拟全市场分页（4 路并发 × 若干页），看是否被限流
    let mut handles = Vec::new();
    for page in 1..=12u32 {
        let client = client.clone();
        handles.push(tokio::spawn(async move {
            let url = format!(
                "https://vip.stock.finance.sina.com.cn/quotes_service/api/json_v2.php/\
                 Market_Center.getHQNodeData?page={page}&num=100&sort=symbol&asc=1&node=hs_a"
            );
            let resp = client
                .get(&url)
                .header("Referer", "https://finance.sina.com.cn/")
                .send()
                .await;
            match resp {
                Ok(r) => {
                    let status = r.status();
                    let len = r.text().await.map(|t| t.len()).unwrap_or(0);
                    (page, format!("status={status} body_len={len}"))
                }
                Err(e) => (page, format!("ERR {e}")),
            }
        }));
    }
    let mut results = Vec::new();
    for handle in handles {
        if let Ok(item) = handle.await {
            results.push(item);
        }
    }
    results.sort_by_key(|(page, _)| *page);
    for (page, info) in results {
        println!("  并发第 {page:>2} 页 -> {info}");
    }
}

async fn probe(client: &reqwest::Client, label: &str, url: &str) {
    match client
        .get(url)
        .header("Referer", "https://finance.sina.com.cn/")
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status();
            let headers = resp.headers().clone();
            let text = resp.text().await.unwrap_or_default();
            println!("[{label}] status={status} body_len={}", text.len());
            println!(
                "  content-type={:?} content-encoding={:?} transfer-encoding={:?}",
                headers.get("content-type"),
                headers.get("content-encoding"),
                headers.get("transfer-encoding")
            );
            println!(
                "  body 头部 = {:?}",
                &text.chars().take(180).collect::<String>()
            );
        }
        Err(error) => println!("[{label}] 请求失败: {error}"),
    }
}
