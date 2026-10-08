//! Scoped three-valued condition trees; unavailable evidence stays unknown.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConditionTree {
    And { children: Vec<ConditionTree> },
    Or { children: Vec<ConditionTree> },
    Mainline {},
    ChangeAbove { value: f64 },
    ChangeBelow { value: f64 },
}
#[derive(Default)]
pub struct Facts {
    pub mainline: Option<bool>,
    pub change_pct: Option<f64>,
}
impl ConditionTree {
    pub fn validate(&self) -> Result<(), String> {
        fn visit(node: &ConditionTree, depth: usize, total: &mut usize) -> Result<(), String> {
            *total += 1;
            if depth > 4 || *total > 20 {
                return Err("自定义条件最多4层、20项".into());
            }
            match node {
                ConditionTree::And { children } | ConditionTree::Or { children } => {
                    if children.len() < 2 || children.len() > 4 {
                        return Err("每组请选择2至4项条件".into());
                    }
                    for child in children {
                        visit(child, depth + 1, total)?;
                    }
                }
                ConditionTree::ChangeAbove { value } | ConditionTree::ChangeBelow { value } => {
                    if !value.is_finite() || !(-30.0..=30.0).contains(value) {
                        return Err("涨跌幅条件范围为−30%至30%".into());
                    }
                }
                _ => {}
            }
            Ok(())
        }
        visit(self, 1, &mut 0)
    }
    pub fn needs_mainline(&self) -> bool {
        match self {
            Self::Mainline { .. } => true,
            Self::And { children } | Self::Or { children } => {
                children.iter().any(Self::needs_mainline)
            }
            _ => false,
        }
    }
    pub fn evaluate(&self, facts: &Facts) -> Option<bool> {
        match self {
            Self::And { children } => {
                let values = children
                    .iter()
                    .map(|c| c.evaluate(facts))
                    .collect::<Vec<_>>();
                if values.contains(&Some(false)) {
                    Some(false)
                } else if values.iter().all(|v| *v == Some(true)) {
                    Some(true)
                } else {
                    None
                }
            }
            Self::Or { children } => {
                let values = children
                    .iter()
                    .map(|c| c.evaluate(facts))
                    .collect::<Vec<_>>();
                if values.contains(&Some(true)) {
                    Some(true)
                } else if values.iter().all(|v| *v == Some(false)) {
                    Some(false)
                } else {
                    None
                }
            }
            Self::Mainline { .. } => facts.mainline,
            Self::ChangeAbove { value } => facts
                .change_pct
                .filter(|p| p.is_finite())
                .map(|p| p >= *value),
            Self::ChangeBelow { value } => facts
                .change_pct
                .filter(|p| p.is_finite())
                .map(|p| p <= *value),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_is_not_false_and_scope_respects_grouping() {
        let node = ConditionTree::And {
            children: vec![
                ConditionTree::ChangeBelow { value: 3.0 },
                ConditionTree::Or {
                    children: vec![ConditionTree::Mainline {}, ConditionTree::ChangeAbove { value: 1.0 }],
                },
            ],
        };
        node.validate().unwrap();
        assert_eq!(node.evaluate(&Facts { mainline: None, change_pct: Some(2.0) }), Some(true));
        assert_eq!(node.evaluate(&Facts { mainline: None, change_pct: Some(0.0) }), None);
        assert_eq!(node.evaluate(&Facts { mainline: None, change_pct: Some(4.0) }), Some(false));
        assert_eq!(node.evaluate(&Facts::default()), None);
    }
    #[test]
    fn bounds_and_unknown_fields_are_rejected() {
        assert!(serde_json::from_str::<ConditionTree>(r#"{"op":"intraday"}"#).is_err());
        assert!(serde_json::from_str::<ConditionTree>(r#"{"op":"mainline","script":"run"}"#).is_err());
        assert!(ConditionTree::And { children: vec![] }.validate().is_err());
        assert!(ConditionTree::ChangeAbove { value: f64::NAN }.validate().is_err());
        let mut tree = ConditionTree::Mainline {};
        for _ in 0..5 {
            tree = ConditionTree::Or { children: vec![tree, ConditionTree::Mainline {}] };
        }
        assert!(tree.validate().is_err());
    }
}
