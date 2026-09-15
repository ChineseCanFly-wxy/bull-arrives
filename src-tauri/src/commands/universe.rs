//! 全市场股票池相关命令（漏斗 L0 获取 + L1 筛选）
//!
//! 对应前端「全市场筛选器」。业务上属于漏斗前两层：
//! 一次拉取全市场快照，然后在内存里按用户配置的条件过滤 —— **筛选本身零网络请求**。

use crate::datasource::eastmoney_universe::{
    self, count_by_board, preferred_source, preset_by_id, preset_infos, Board, FilterCapabilities,
    FilterPreset, MarketFilter, PresetInfo, SnapshotRow, SnapshotSource, DEFAULT_SNAPSHOT_TTL,
};
use crate::db::Database;
use crate::quant::playbook::TradeRule;
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;
use tauri::State;

/// 板块分布项（供设置页/自检展示）
#[derive(Debug, Serialize)]
pub struct BoardCount {
    pub board: Board,
    /// 中文名，前端直接显示，不必再维护一份映射
    pub label: String,
    pub count: usize,
}

/// 全市场股票池响应
#[derive(Debug, Serialize)]
pub struct UniverseResponse {
    /// 全市场总数（未筛选）
    pub total_all: usize,
    /// 命中筛选条件的数量
    pub total_matched: usize,
    /// 本次实际返回的行数（受 limit 限制）
    pub returned: usize,
    /// 数据是否陈旧：本次刷新失败（很可能被限流），返回的是上次的旧数据
    pub stale: bool,
    /// 本次数据来自哪个通道（sina / eastmoney）
    pub source: SnapshotSource,
    /// 通道中文名，前端直接显示
    pub source_label: String,
    /// 当前通道是否提供「量比」。为 false 时前端应提示该条件当前不可用，
    /// 否则用户设了「量比 ≥ x」会得到 0 结果而不知道为什么。
    pub volume_ratio_supported: bool,
    /// 当前通道是否提供「60 日涨跌幅」。与量比同理 ——
    /// 依赖它的趋势 / 反转类策略在新浪通道下会被跳过，必须让用户看得见。
    pub change_60d_supported: bool,
    /// 因数据源不支持而被自动忽略的条件名（如 ["量比"]），供前端明确提示
    pub skipped_conditions: Vec<String>,
    /// 全市场板块分布
    pub board_counts: Vec<BoardCount>,
    /// 命中筛选的明细
    pub rows: Vec<SnapshotRow>,
}

/// 返回行数默认上限。
/// 全市场 5900 行一次 IPC 传过去约 3MB，会明显拖慢前端渲染，
/// 所以默认截断；调用方可按需提高。
const DEFAULT_ROW_LIMIT: usize = 500;

/// 服务端分页的默认每页条数（与前端默认值一致）
const DEFAULT_PAGE_SIZE: u32 = 20;

/// 服务端分页的每页上限（与前端可选的最大值一致）
const MAX_PAGE_SIZE: u32 = 100;

/// 翻页必须继续使用首屏对应的快照；强制刷新始终优先拿新数据。
fn snapshot_ttl(force_refresh: bool, reuse_snapshot: bool) -> Duration {
    if force_refresh {
        Duration::ZERO
    } else if reuse_snapshot {
        Duration::MAX
    } else {
        DEFAULT_SNAPSHOT_TTL
    }
}

/// 获取全市场股票池并应用筛选条件。
///
/// 筛选条件的优先级：`filter` > `preset` > 默认条件。
///
/// - `preset` 传预设 id（如 `"strong_breakout"`），未知 id 回退为「全部」
/// - `filter` 传完整的筛选条件（前端微调预设后传这个）
/// - `page` / `page_size`：**服务端分页**。传了 `page` 就只返回那一页
///   （`page_size` 缺省 20，上限 100）；翻页由前端逐页请求，避免一次传输几千行
/// - `reuse_snapshot`：翻页时复用当前快照，即使常规 60 秒 TTL 已过，保证页间一致
/// - 不传 `page` 时保持旧行为：按 `limit`（默认 500）从头截断
/// - `force_refresh` 为 `true` 时忽略缓存强制刷新
/// - `source` 指定取数通道（`sina` / `eastmoney` / `auto`），不传则读设置项 `universe_source`
///
/// ⚠️ 分页的前提是**顺序稳定**：这里按「成交额降序 + 代码升序」排序，
/// 否则两次请求之间顺序漂移会导致翻页出现重复或漏行。
#[tauri::command]
pub async fn get_market_universe(
    db: State<'_, Arc<Database>>,
    preset: Option<String>,
    filter: Option<MarketFilter>,
    page: Option<u32>,
    page_size: Option<u32>,
    limit: Option<usize>,
    force_refresh: Option<bool>,
    reuse_snapshot: Option<bool>,
    source: Option<String>,
) -> Result<UniverseResponse, String> {
    let ttl = snapshot_ttl(
        force_refresh.unwrap_or(false),
        reuse_snapshot.unwrap_or(false),
    );
    let started = std::time::Instant::now();

    // 通道优先级：调用方显式指定 > 设置项 > auto（新浪优先，东财兜底）
    let configured = source.or_else(|| {
        db.get_setting("universe_source")
            .ok()
            .flatten()
            .filter(|value| !value.trim().is_empty())
    });
    let preferred = preferred_source(configured.as_deref());

    let outcome = eastmoney_universe::market_snapshot_with_fallback(ttl, preferred)
        .await
        .map_err(|error| {
            format!(
                "获取全市场快照失败：{error}\
                 （已尝试新浪与东方财富两个通道；请检查网络，或在设置里切换数据源后重试）"
            )
        })?;

    let filter = filter.unwrap_or_else(|| {
        preset
            .as_deref()
            .map(preset_by_id)
            .unwrap_or(eastmoney_universe::FilterPreset::All)
            .build()
    });

    let capabilities = FilterCapabilities::for_source(outcome.source);
    let mut matched = filter.apply_with(&outcome.rows, capabilities);
    let skipped_conditions = filter.skipped_conditions(capabilities);

    // 稳定排序：成交额降序，同额按代码升序。没有这一步，分页就会重复/漏行。
    matched.sort_by(|a, b| {
        b.amount
            .partial_cmp(&a.amount)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.code.cmp(&b.code))
    });

    let board_counts = count_by_board(&outcome.rows)
        .into_iter()
        .map(|(board, count)| BoardCount {
            board,
            label: board.label().to_owned(),
            count,
        })
        .collect();

    let total_matched = matched.len();

    // 分页：传了 page 就按页取；否则退回旧的 limit 截断行为
    let rows: Vec<SnapshotRow> = match (page, page_size) {
        (Some(requested_page), _) => {
            let size = page_size.unwrap_or(DEFAULT_PAGE_SIZE).clamp(1, MAX_PAGE_SIZE) as usize;
            let current = requested_page.max(1) as usize;
            let start = current.saturating_sub(1).saturating_mul(size);
            matched.into_iter().skip(start).take(size).collect()
        }
        (None, Some(size)) => {
            let size = size.clamp(1, MAX_PAGE_SIZE) as usize;
            matched.into_iter().take(size).collect()
        }
        _ => {
            let take = limit.unwrap_or(DEFAULT_ROW_LIMIT).min(total_matched);
            matched.into_iter().take(take).collect()
        }
    };

    log::info!(
        "[universe] get_market_universe: page={:?} size={:?} reuse_snapshot={} 快照={}({}) 全市场={} 命中={} 返回={} 耗时={}ms",
        page,
        page_size,
        reuse_snapshot.unwrap_or(false),
        outcome.source.label(),
        if outcome.stale { "陈旧" } else { "新鲜" },
        outcome.rows.len(),
        total_matched,
        rows.len(),
        started.elapsed().as_millis()
    );

    Ok(UniverseResponse {
        total_all: outcome.rows.len(),
        total_matched,
        returned: rows.len(),
        stale: outcome.stale,
        source: outcome.source,
        source_label: outcome.source.label().to_owned(),
        volume_ratio_supported: outcome.source.has_volume_ratio(),
        change_60d_supported: outcome.source.has_change_60d(),
        skipped_conditions,
        board_counts,
        rows,
    })
}

/// 用户自建策略在 settings 表里的 key。
///
/// 复用现有 KV 表存一个 JSON 数组，**不需要动表结构、也不需要迁移** ——
/// 策略数量在几十条量级，整存整取足够，不值得为它单开一张表。
const CUSTOM_PRESETS_SETTING_KEY: &str = "universe_custom_presets";

/// 自定义策略数量上限。防止无节制堆积把设置接口的响应撑大。
const MAX_CUSTOM_PRESETS: usize = 30;

/// 策略名称最大字数（按字符数算，中文一个字算一个）
const MAX_LABEL_CHARS: usize = 16;

/// 策略说明最大字数
const MAX_DESCRIPTION_CHARS: usize = 80;

/// 读取用户自建策略。
///
/// 容错策略：**逐条校验，坏数据只丢这一条**，不能让一条脏数据把整条策略栏清空。
/// 数据整体解析失败时返回 `Err`，由调用方决定是否降级为「只有内置策略」。
fn load_custom_presets(db: &Database) -> Result<Vec<PresetInfo>, String> {
    let Some(raw) = db
        .get_setting(CUSTOM_PRESETS_SETTING_KEY)
        .map_err(|e| e.to_string())?
    else {
        return Ok(Vec::new());
    };
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }

    let parsed: Vec<PresetInfo> =
        serde_json::from_str(&raw).map_err(|e| format!("自定义策略数据解析失败：{e}"))?;

    let mut cleaned = Vec::with_capacity(parsed.len());
    for mut preset in parsed {
        // builtin 由后端裁定：用户数据里写什么都不作数
        preset.builtin = false;
        preset.id = preset.id.trim().to_owned();
        preset.label = preset.label.trim().to_owned();
        preset.description = preset.description.trim().to_owned();

        if preset.id.is_empty() || preset.label.is_empty() {
            log::warn!(
                "[universe] 跳过残缺的自定义策略：id={:?} label={:?}",
                preset.id,
                preset.label
            );
            continue;
        }
        if FilterPreset::is_builtin_id(&preset.id) {
            log::warn!(
                "[universe] 跳过 id 与内置预设冲突的自定义策略：{}",
                preset.id
            );
            continue;
        }
        // 规则 id 收敛到已知值：坏数据不能让「打开分析」拿到一个无法解析的规则
        preset.rule = TradeRule::from_id(&preset.rule).id().to_owned();
        preset.filter.normalize_ranges();
        cleaned.push(preset);
    }
    Ok(cleaned)
}

/// 写回用户自建策略（整体覆盖）。
fn store_custom_presets(db: &Database, presets: &[PresetInfo]) -> Result<(), String> {
    let json = serde_json::to_string(presets).map_err(|e| format!("自定义策略序列化失败：{e}"))?;
    db.set_setting(CUSTOM_PRESETS_SETTING_KEY, &json)
        .map_err(|e| e.to_string())
}

/// 生成一个没被占用的自定义策略 id
fn next_custom_id(existing: &[PresetInfo]) -> String {
    let stamp = chrono::Utc::now().timestamp_millis();
    let mut candidate = format!("custom_{stamp}");
    let mut suffix = 2u32;
    while existing.iter().any(|preset| preset.id == candidate) {
        candidate = format!("custom_{stamp}_{suffix}");
        suffix += 1;
    }
    candidate
}

/// 校验并规范化用户填的名称 / 说明。
///
/// 返回规范化后的 `(label, description)`；名称与内置或其它自定义策略重名时直接拒绝 ——
/// 允许重名的话，策略栏会出现两个一模一样的标签，用户根本分不清点的是哪个。
fn normalize_meta(
    label: &str,
    description: Option<&str>,
    self_id: &str,
    others: &[PresetInfo],
) -> Result<(String, String), String> {
    let label = label.trim();
    if label.is_empty() {
        return Err("策略名称不能为空".into());
    }
    let label_chars = label.chars().count();
    if label_chars > MAX_LABEL_CHARS {
        return Err(format!(
            "策略名称最多 {MAX_LABEL_CHARS} 个字，当前 {label_chars} 个"
        ));
    }

    if let Some(clash) = others
        .iter()
        .find(|preset| preset.id != self_id && preset.label == label)
    {
        return Err(format!("已有同名策略「{}」，换个名字吧", clash.label));
    }

    let description = description.unwrap_or_default().trim();
    let desc_chars = description.chars().count();
    if desc_chars > MAX_DESCRIPTION_CHARS {
        return Err(format!(
            "策略说明最多 {MAX_DESCRIPTION_CHARS} 个字，当前 {desc_chars} 个"
        ));
    }

    Ok((label.to_owned(), description.to_owned()))
}

/// 列出全部可用策略的**纯逻辑**：**内置预设在前，用户自建策略在后**。
///
/// 之所以 `Result` 不往外抛：内置策略是产品底线，
/// 不能因为用户自建数据坏了就让整条策略栏消失 —— 读不出来就只下发内置的。
fn all_presets_impl(db: &Database) -> Vec<PresetInfo> {
    let mut presets = preset_infos();
    let builtin_count = presets.len();

    match load_custom_presets(db) {
        Ok(custom) => presets.extend(custom),
        Err(error) => {
            log::warn!("[universe] 读取自定义策略失败，仅下发内置预设：{error}");
        }
    }

    log::info!(
        "[universe] get_filter_presets -> 内置 {} + 自建 {}",
        builtin_count,
        presets.len() - builtin_count
    );
    presets
}

#[tauri::command]
pub fn get_filter_presets(db: State<'_, Arc<Database>>) -> Vec<PresetInfo> {
    all_presets_impl(db.inner().as_ref())
}

/// 保存策略的**纯逻辑**（不依赖 Tauri `State`，便于直接测试落库往返）。
///
/// - `id` 为空 → 新建（自动分配 `custom_*` 的 id）
/// - `id` 指向已有自建策略 → 覆盖它的名称、说明与条件
/// - `id` 指向内置策略 → 拒绝，并提示改用「另存为我的策略」
fn save_preset_impl(
    db: &Database,
    id: Option<&str>,
    label: &str,
    description: Option<&str>,
    rule: Option<&str>,
    filter: MarketFilter,
) -> Result<PresetInfo, String> {
    let mut customs = load_custom_presets(db)?;

    let requested = id.map(str::trim).filter(|value| !value.is_empty());

    let target_id = match requested {
        Some(existing) => {
            if FilterPreset::is_builtin_id(existing) {
                return Err(format!(
                    "「{existing}」是内置策略，不能直接改；请用「另存为我的策略」存一份自己的副本再改"
                ));
            }
            if !customs.iter().any(|preset| preset.id == existing) {
                return Err("要修改的策略不存在，可能已经在别处被删掉了".into());
            }
            existing.to_owned()
        }
        None => {
            if customs.len() >= MAX_CUSTOM_PRESETS {
                return Err(format!(
                    "自定义策略最多 {MAX_CUSTOM_PRESETS} 个，请先删掉一些不再用的"
                ));
            }
            next_custom_id(&customs)
        }
    };

    // 重名检查要连内置一起看：用户把自建策略叫「强势突破」同样会撞车
    let mut others = preset_infos();
    others.extend(customs.iter().cloned());
    let (label, description) = normalize_meta(label, description, &target_id, &others)?;

    let mut filter = filter;
    filter.normalize_ranges();

    let saved = PresetInfo {
        id: target_id.clone(),
        label,
        description,
        filter,
        // 未知 / 缺省一律回落为趋势跟随，保证「打开分析」永远有规则可用
        rule: TradeRule::from_id(rule.unwrap_or_default()).id().to_owned(),
        builtin: false,
    };

    match customs.iter_mut().find(|preset| preset.id == target_id) {
        Some(slot) => *slot = saved.clone(),
        None => customs.push(saved.clone()),
    }
    store_custom_presets(db, &customs)?;
    Ok(saved)
}

/// 删除策略的**纯逻辑**（内置策略一律拒绝）
fn delete_preset_impl(db: &Database, id: &str) -> Result<(), String> {
    let id = id.trim();
    if FilterPreset::is_builtin_id(id) {
        return Err("内置策略不能删除；想改造它的话，用「另存为我的策略」存一份副本再改".into());
    }

    let mut customs = load_custom_presets(db)?;
    let before = customs.len();
    customs.retain(|preset| preset.id != id);
    if customs.len() == before {
        return Err("要删除的策略不存在，可能已经被删掉了".into());
    }

    store_custom_presets(db, &customs)
}

/// 保存一条策略，**新建 / 重命名 / 用当前条件覆盖**三种动作共用这一个命令。
#[tauri::command]
pub fn save_filter_preset(
    db: State<'_, Arc<Database>>,
    id: Option<String>,
    label: String,
    description: Option<String>,
    rule: Option<String>,
    filter: MarketFilter,
) -> Result<PresetInfo, String> {
    let saved = save_preset_impl(
        db.inner().as_ref(),
        id.as_deref(),
        &label,
        description.as_deref(),
        rule.as_deref(),
        filter,
    )?;
    log::info!(
        "[universe] save_filter_preset -> {} 「{}」规则={}",
        saved.id,
        saved.label,
        saved.rule
    );
    Ok(saved)
}

/// 删除一条用户自建策略。内置策略一律拒绝 —— 它们是产品随版本维护的基准方案。
#[tauri::command]
pub fn delete_filter_preset(db: State<'_, Arc<Database>>, id: String) -> Result<(), String> {
    delete_preset_impl(db.inner().as_ref(), &id)?;
    log::info!("[universe] delete_filter_preset -> {id}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{next_custom_id, normalize_meta, snapshot_ttl, MAX_LABEL_CHARS};
    use crate::datasource::eastmoney_universe::{
        preset_infos, MarketFilter, PresetInfo, DEFAULT_SNAPSHOT_TTL,
    };
    use crate::quant::playbook::TradeRule;
    use std::time::Duration;

    fn custom(id: &str, label: &str) -> PresetInfo {
        PresetInfo {
            id: id.to_owned(),
            label: label.to_owned(),
            description: String::new(),
            filter: MarketFilter::default(),
            rule: TradeRule::TrendFollow.id().to_owned(),
            builtin: false,
        }
    }

    #[test]
    fn force_refresh_overrides_pagination_snapshot_reuse() {
        assert_eq!(snapshot_ttl(false, false), DEFAULT_SNAPSHOT_TTL);
        assert_eq!(snapshot_ttl(false, true), Duration::MAX);
        assert_eq!(snapshot_ttl(true, true), Duration::ZERO);
    }

    /// 内置策略必须被标记为 builtin，否则前端会给出「改名/删除」入口，
    /// 用户点了才发现在后端被拒 —— 交互上很别扭。
    #[test]
    fn builtin_presets_are_flagged_and_use_prefixed_ids() {
        let infos = preset_infos();
        assert!(!infos.is_empty());
        for info in &infos {
            assert!(info.builtin, "内置预设 {} 未标记 builtin", info.id);
            assert!(
                !info.id.starts_with("custom_"),
                "内置预设 {} 不应占用 custom_ 前缀",
                info.id
            );
        }
    }

    /// 同一毫秒内连续新建也不能撞 id，否则后存的会把先存的覆盖掉。
    #[test]
    fn generated_custom_ids_never_collide() {
        let mut existing: Vec<PresetInfo> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..50 {
            let id = next_custom_id(&existing);
            assert!(seen.insert(id.clone()), "生成了重复 id：{id}");
            assert!(id.starts_with("custom_"));
            existing.push(custom(&id, &id));
        }
    }

    #[test]
    fn name_is_trimmed_and_must_not_be_blank() {
        let (label, desc) = normalize_meta("  我的策略  ", Some("  说明  "), "custom_1", &[])
            .expect("应当通过校验");
        assert_eq!(label, "我的策略");
        assert_eq!(desc, "说明");

        assert!(normalize_meta("   ", None, "custom_1", &[]).is_err(), "全空白名称应被拒");
    }

    #[test]
    fn duplicate_names_are_rejected_except_for_self() {
        let others = vec![custom("custom_1", "挖坑策略"), custom("custom_2", "打板策略")];

        // 撞别人的名字 → 拒绝，且错误信息里要带上冲突的名字，便于用户判断
        let err = normalize_meta("挖坑策略", None, "custom_2", &others).unwrap_err();
        assert!(err.contains("挖坑策略"), "错误信息应指出撞了哪个名字：{err}");

        // 改自己的名字（id 相同）→ 放行
        assert!(normalize_meta("挖坑策略", None, "custom_1", &others).is_ok());
    }

    /// 中文按字数算而不是按字节 —— 用 len() 的话 16 个汉字会被当成 48 而误拒。
    #[test]
    fn length_limit_counts_chars_not_bytes() {
        let just_fits = "策".repeat(MAX_LABEL_CHARS);
        assert!(normalize_meta(&just_fits, None, "custom_1", &[]).is_ok());

        let too_long = "策".repeat(MAX_LABEL_CHARS + 1);
        assert!(
            normalize_meta(&too_long, None, "custom_1", &[]).is_err(),
            "超长名称应被拒"
        );

        let long_desc = "说".repeat(81);
        assert!(
            normalize_meta("合规名称", Some(&long_desc), "custom_1", &[]).is_err(),
            "超长说明应被拒"
        );
    }

    /// 落库往返：保存 → 读回 → 改名 → 覆盖条件 → 删除。
    ///
    /// 这是「用户数据真的写进 settings 表再读回来」的唯一路径。不测它的话，
    /// 序列化 / 反序列化 / 覆盖语义上的问题只能在真机上才暴露 ——
    /// 而这正是「存了策略重启后不见了」这类问题最常见的成因。
    #[test]
    fn custom_presets_round_trip_through_the_settings_table() {
        use super::{all_presets_impl, delete_preset_impl, load_custom_presets, save_preset_impl};
        use crate::db::Database;

        let dir = std::env::temp_dir().join(format!(
            "bull-arrives-preset-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let db = Database::open(dir.clone()).expect("应能建库");

        // 起点：一条自建策略都没有，但内置的齐全
        assert!(load_custom_presets(&db).expect("初始读取应成功").is_empty());
        let builtin_total = preset_infos().len();

        // 保存三条
        let mut ids = Vec::new();
        for (label, cap) in [("我的低估值", 50.0), ("我的高换手", 80.0), ("我的超跌", 30.0)] {
            let filter = MarketFilter {
                market_cap_min_yi: Some(cap),
                ..MarketFilter::default()
            };
            let saved = save_preset_impl(&db, None, label, Some("测试用"), None, filter)
                .expect("保存应成功");
            assert!(saved.id.starts_with("custom_"), "自建 id 应带 custom_ 前缀");
            assert!(!saved.builtin);
            ids.push(saved.id);
        }

        // 读回：内置在前、自建在后，且互不污染
        let stored = load_custom_presets(&db).expect("应能读回");
        assert_eq!(stored.len(), 3, "三条自建策略都应读回");
        assert!(stored.iter().all(|preset| !preset.builtin));
        assert_eq!(all_presets_impl(&db).len(), builtin_total + 3);

        // 改名 + 覆盖条件：id 不变、条件被替换
        let renamed = save_preset_impl(
            &db,
            Some(ids[0].as_str()),
            "改过的名字",
            Some("新说明"),
            Some("mean_reversion"),
            MarketFilter {
                pb_max: Some(1.5),
                ..MarketFilter::default()
            },
        )
        .expect("覆盖应成功");
        assert_eq!(renamed.id, ids[0], "覆盖不应改 id");
        assert_eq!(renamed.label, "改过的名字");
        assert_eq!(renamed.rule, "mean_reversion", "规则应随保存一起落库");
        assert_eq!(renamed.filter.pb_max, Some(1.5), "条件应被整体替换");
        assert_eq!(renamed.filter.market_cap_min_yi, None, "旧条件不应残留");
        assert_eq!(load_custom_presets(&db).expect("读取应成功").len(), 3, "覆盖不该新增");

        // 填反的区间在落库前被交换过来
        let swapped = save_preset_impl(
            &db,
            None,
            "填反了的",
            None,
            None,
            MarketFilter {
                price_min: Some(100.0),
                price_max: Some(10.0),
                ..MarketFilter::default()
            },
        )
        .expect("保存应成功");
        assert_eq!(swapped.filter.price_min, Some(10.0));
        assert_eq!(swapped.filter.price_max, Some(100.0));
        // 未知 / 缺省规则一律收敛为趋势跟随
        assert_eq!(swapped.rule, "trend_follow");

        // 重名要被拒（含与内置撞名）
        assert!(
            save_preset_impl(&db, None, "改过的名字", None, None, MarketFilter::default()).is_err(),
            "自建之间不应允许重名"
        );
        assert!(
            save_preset_impl(&db, None, "强势突破", None, None, MarketFilter::default()).is_err(),
            "与内置策略重名也应被拒"
        );
        // 内置策略不能被改
        assert!(
            save_preset_impl(&db, Some("all"), "偷改内置", None, None, MarketFilter::default())
                .is_err(),
            "内置策略不允许直接修改"
        );

        // 删除自建：只删掉目标那一条
        delete_preset_impl(&db, &ids[1]).expect("删除应成功");
        let after = load_custom_presets(&db).expect("读取应成功");
        assert_eq!(after.len(), 3, "上面新存过一条，删一条后应为 3");
        assert!(!after.iter().any(|preset| preset.id == ids[1]));

        // 删不存在的、删内置的都要报错
        assert!(delete_preset_impl(&db, &ids[1]).is_err(), "重复删除应报错");
        assert!(delete_preset_impl(&db, "all").is_err(), "内置策略不允许删除");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 脏数据不能让整条策略栏消失：坏条目逐条丢弃，内置策略照常下发。
    #[test]
    fn corrupt_custom_presets_degrade_to_builtin_only() {
        use super::{all_presets_impl, load_custom_presets, CUSTOM_PRESETS_SETTING_KEY};
        use crate::db::Database;

        let dir = std::env::temp_dir().join(format!(
            "bull-arrives-preset-corrupt-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let db = Database::open(dir.clone()).expect("应能建库");

        // 整体不是合法 JSON → 读取失败，但内置策略仍然要能下发
        db.set_setting(CUSTOM_PRESETS_SETTING_KEY, "{ 这不是 JSON")
            .expect("写入应成功");
        assert!(load_custom_presets(&db).is_err());
        assert_eq!(
            all_presets_impl(&db).len(),
            preset_infos().len(),
            "自建数据坏掉时应只剩内置策略，而不是一条都不剩"
        );

        // 数组里混入残缺条目 → 只丢坏的那条
        db.set_setting(
            CUSTOM_PRESETS_SETTING_KEY,
            r#"[
                {"id":"custom_ok","label":"好的","description":"","filter":{}},
                {"id":"","label":"缺 id","description":"","filter":{}},
                {"id":"custom_nolabel","label":"   ","description":"","filter":{}},
                {"id":"all","label":"冒用内置 id","description":"","filter":{}}
            ]"#,
        )
        .expect("写入应成功");

        let cleaned = load_custom_presets(&db).expect("应能读回");
        assert_eq!(cleaned.len(), 1, "只应保留那一条完整的");
        assert_eq!(cleaned[0].id, "custom_ok");
        assert!(!cleaned[0].builtin);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
