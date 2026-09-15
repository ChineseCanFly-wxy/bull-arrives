use crate::datasource::sector::{self, SectorKind, SectorMemberPage, SectorSummaryPage};

/// 获取行业或概念板块排行。
///
/// `keyword` 非空时会在板块目录中做全量名称/代码搜索，再按分页返回结果；
/// 普通翻页只请求当前页，避免打开板块中心时拉取所有成分股。
#[tauri::command]
pub async fn get_sector_summaries(
    kind: String,
    page: Option<u32>,
    page_size: Option<u32>,
    keyword: Option<String>,
    force_refresh: Option<bool>,
) -> Result<SectorSummaryPage, String> {
    let kind = SectorKind::parse(&kind)?;
    let keyword: String = keyword
        .unwrap_or_default()
        .trim()
        .chars()
        .take(30)
        .collect();
    sector::fetch_summaries(
        kind,
        page.unwrap_or(1),
        page_size.unwrap_or(50),
        &keyword,
        force_refresh.unwrap_or(false),
    )
    .await
}

/// 按板块代码获取成分股。只接受东方财富板块代码，避免把任意用户输入拼进 URL。
#[tauri::command]
pub async fn get_sector_members(
    kind: String,
    sector_code: String,
    page: Option<u32>,
    page_size: Option<u32>,
    force_refresh: Option<bool>,
) -> Result<SectorMemberPage, String> {
    let kind = SectorKind::parse(&kind)?;
    sector::fetch_members(
        kind,
        &sector_code,
        page.unwrap_or(1),
        page_size.unwrap_or(50),
        force_refresh.unwrap_or(false),
    )
    .await
}
