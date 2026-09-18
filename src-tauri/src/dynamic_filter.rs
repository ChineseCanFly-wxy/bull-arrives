use crate::agent::DynamicFilterSuggestion;
use crate::datasource::eastmoney_universe::{Board, MarketFilter, SnapshotRow};
use serde::Serialize;

pub const CANDIDATE_LIMIT_MAX: usize = 100;

#[derive(Debug, Clone, Serialize)]
pub struct SanitizedFilter {
    pub filter: MarketFilter,
    pub candidate_limit: usize,
    pub clamped_fields: Vec<String>,
}

fn clamped(value: f64, min: f64, max: f64, field: &str, report: &mut Vec<String>) -> f64 {
    let next = value.clamp(min, max);
    if (next - value).abs() > f64::EPSILON {
        report.push(field.into());
    }
    next
}

fn ordered(mut min: f64, mut max: f64, field: &str, report: &mut Vec<String>) -> (f64, f64) {
    if min > max {
        std::mem::swap(&mut min, &mut max);
        report.push(field.into());
    }
    (min, max)
}

pub fn sanitize(base: &MarketFilter, value: &DynamicFilterSuggestion) -> SanitizedFilter {
    let mut report = Vec::new();
    let mut filter = base.clone();
    filter.boards.retain(|board| {
        matches!(
            board,
            Board::ShMain | Board::SzMain | Board::ChiNext | Board::Star | Board::Bse
        )
    });
    if filter.boards.is_empty() {
        filter.boards = MarketFilter::default().boards;
        report.push("boards".into());
    }
    for (field, previous) in [
        ("exclude_st", filter.exclude_st),
        ("exclude_delisting", filter.exclude_delisting),
        ("exclude_suspended", filter.exclude_suspended),
        ("exclude_limit_locked", filter.exclude_limit_locked),
    ] {
        if !previous {
            report.push(field.into());
        }
    }
    filter.exclude_st = true;
    filter.exclude_delisting = true;
    filter.exclude_suspended = true;
    filter.exclude_limit_locked = true;

    let price = ordered(
        clamped(value.price_min, 1.0, 200.0, "price_min", &mut report),
        clamped(value.price_max, 2.0, 500.0, "price_max", &mut report),
        "price_range",
        &mut report,
    );
    let cap = ordered(
        clamped(
            value.market_cap_min_yi,
            10.0,
            2_000.0,
            "market_cap_min_yi",
            &mut report,
        ),
        clamped(
            value.market_cap_max_yi,
            50.0,
            10_000.0,
            "market_cap_max_yi",
            &mut report,
        ),
        "market_cap_range",
        &mut report,
    );
    let turnover = ordered(
        clamped(value.turnover_min, 0.0, 15.0, "turnover_min", &mut report),
        clamped(value.turnover_max, 1.0, 30.0, "turnover_max", &mut report),
        "turnover_range",
        &mut report,
    );
    let change = ordered(
        clamped(
            value.change_pct_min,
            -10.0,
            0.0,
            "change_pct_min",
            &mut report,
        ),
        clamped(
            value.change_pct_max,
            0.0,
            10.0,
            "change_pct_max",
            &mut report,
        ),
        "change_pct_range",
        &mut report,
    );
    filter.price_min = Some(price.0);
    filter.price_max = Some(price.1);
    filter.market_cap_min_yi = Some(cap.0);
    filter.market_cap_max_yi = Some(cap.1);
    filter.turnover_min = Some(turnover.0);
    filter.turnover_max = Some(turnover.1);
    filter.volume_ratio_min = Some(clamped(
        value.volume_ratio_min,
        0.5,
        5.0,
        "volume_ratio_min",
        &mut report,
    ));
    filter.change_pct_min = Some(change.0);
    filter.change_pct_max = Some(change.1);
    filter.amount_min_wan = Some(clamped(
        value.amount_min_wan,
        500.0,
        100_000.0,
        "amount_min_wan",
        &mut report,
    ));
    filter.amplitude_max = Some(clamped(
        value.amplitude_max,
        1.0,
        15.0,
        "amplitude_max",
        &mut report,
    ));
    let candidate_limit = value.candidate_limit.clamp(10, CANDIDATE_LIMIT_MAX);
    if candidate_limit != value.candidate_limit {
        report.push("candidate_limit".into());
    }
    report.sort();
    report.dedup();
    SanitizedFilter {
        filter,
        candidate_limit,
        clamped_fields: report,
    }
}

pub fn market_context(rows: &[SnapshotRow]) -> serde_json::Value {
    let count = rows.len().max(1) as f64;
    serde_json::json!({
        "row_count": rows.len(),
        "advancers": rows.iter().filter(|row| row.change_pct > 0.0).count(),
        "decliners": rows.iter().filter(|row| row.change_pct < 0.0).count(),
        "average_change_pct": rows.iter().map(|row| row.change_pct).sum::<f64>() / count,
        "average_turnover_pct": rows.iter().map(|row| row.turnover_rate).sum::<f64>() / count,
        "average_amplitude_pct": rows.iter().map(|row| row.amplitude_pct).sum::<f64>() / count,
        "paired_performance": { "status": "insufficient", "observations": 0 }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_soft_fields_and_cannot_disable_hard_exclusions() {
        let mut base = MarketFilter::default();
        base.exclude_st = false;
        base.exclude_delisting = false;
        base.boards = vec![Board::BShare];
        let result = sanitize(
            &base,
            &DynamicFilterSuggestion {
                price_min: 999.0,
                price_max: -2.0,
                market_cap_min_yi: 20_000.0,
                market_cap_max_yi: 1.0,
                turnover_min: 99.0,
                turnover_max: -1.0,
                volume_ratio_min: 99.0,
                change_pct_min: -99.0,
                change_pct_max: 99.0,
                amount_min_wan: 1.0,
                amplitude_max: 99.0,
                candidate_limit: 999,
                rationale: "test".into(),
            },
        );
        assert!(result.filter.exclude_st && result.filter.exclude_delisting);
        assert!(result.filter.exclude_suspended && result.filter.exclude_limit_locked);
        assert!(!result.filter.boards.contains(&Board::BShare));
        assert!(result.filter.price_min <= result.filter.price_max);
        assert!(result.filter.market_cap_min_yi <= result.filter.market_cap_max_yi);
        assert_eq!(result.candidate_limit, CANDIDATE_LIMIT_MAX);
        assert!(!result.clamped_fields.is_empty());
    }
}
