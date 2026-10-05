//! Fixed nested technical research, isolated from registered live model identities.
use super::research::{self, ModelRunnerConfig};
use crate::db::Database;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
const SCRIPT: &[u8] = include_bytes!("../../../research/nested-technical-v1/search.py");
pub const SEARCH_ID: &str = "technical-nested-v1";
pub const YEARS: [i64; 5] = [2022, 2023, 2024, 2025, 2026];
fn hash(path: &Path) -> Result<String, String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|e| format!("研究文件缺失 {}: {e}", path.display()))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("研究输入必须为普通文件".into());
    }
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut state = Sha256::new();
    let mut buffer = vec![0; 4 * 1024 * 1024];
    use std::io::Read;
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        state.update(&buffer[..n]);
    }
    Ok(hex::encode(state.finalize()))
}
fn identity(c: &ModelRunnerConfig) -> Result<(String, Value, PathBuf), String> {
    research::trusted_runner(c)?;
    let root = Path::new(&c.research_root);
    let script = root.join("research/nested-technical-v1/search.py");
    if hash(&script)? != hex::encode(Sha256::digest(SCRIPT)) {
        return Err("嵌套研究程序与当前应用受信版本不一致".into());
    }
    let sources = json!({"runner":hash(&script)?,"snapshot":hash(&Path::new(&c.snapshot).join("matrices.npz"))?,"metadata":hash(&Path::new(&c.snapshot).join("matrices-metadata.json"))?,"index":hash(Path::new(&c.index))?,"registry":hash(&root.join("research/research-center-runner/registry.json"))?,"frozen_runner":hash(&root.join("research/research-center-runner/model_runner.py"))?,"paths":c,"search":SEARCH_ID,"years":YEARS});
    Ok((
        hex::encode(Sha256::digest(sources.to_string())),
        sources,
        script,
    ))
}
fn read_checked(path: &Path) -> Result<Value, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 32 * 1024 * 1024 {
        return Err("研究结果文件类型或大小无效".into());
    }
    serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}
fn validate_artifact(path: &Path, expected: &str, base: &Path) -> Result<(), String> {
    let resolved = path.canonicalize().map_err(|e| e.to_string())?;
    let root = base.canonicalize().map_err(|e| e.to_string())?;
    if !resolved.starts_with(&root) || hash(path)? != expected {
        return Err("研究结果来源越界或指纹已变化".into());
    }
    Ok(())
}
fn selection_choice<'a>(rows: &'a [Value], policy: &str) -> Result<Option<&'a Value>, String> {
    let mut best: Option<(&Value, [f64; 3], &str)> = None;
    for row in rows {
        let metrics = &row["metrics"];
        let ret = metrics["net_return_pct"]
            .as_f64()
            .filter(|v| v.is_finite())
            .ok_or("内层收益无效")?;
        let dd = metrics["max_drawdown_pct"]
            .as_f64()
            .filter(|v| v.is_finite() && *v >= 0.)
            .ok_or("内层回撤无效")?;
        let cycles = metrics["completed_holding_cycles"]
            .as_u64()
            .ok_or("内层交易数量无效")?;
        let id = row["id"].as_str().ok_or("内层身份缺失")?;
        if ret <= 0. || cycles < 30 {
            continue;
        }
        let key = match policy {
            "balanced" => [ret - 0.5 * dd, ret, -dd],
            "return" => [ret, -dd, 0.],
            "defensive" => [-dd, ret, 0.],
            _ => return Err("未知选择风格".into()),
        };
        let better = best.as_ref().is_none_or(|(_, old, old_id)| {
            key.iter()
                .zip(old.iter())
                .map(|(a, b)| a.total_cmp(b))
                .find(|o| !o.is_eq())
                .unwrap_or_else(|| id.cmp(old_id))
                .is_gt()
        });
        if better {
            best = Some((row, key, id));
        }
    }
    Ok(best.map(|(row, _, _)| row))
}
fn validate_report(row: &Value, year: i64, sources: &Value, base: &Path) -> Result<(), String> {
    if row["schema"] != "ashare-nested-technical-v1"
        || row["year"] != year
        || row["bank_size"] != 12
        || row["production_admission"] != false
        || row["new_admitted_models"] != 0
        || row["later_history_already_seen"] != true
    {
        return Err("嵌套研究身份或准入标记错误".into());
    }
    for key in ["runner", "snapshot", "metadata", "index"] {
        if row["input_sha256"][key] != sources[key] {
            return Err(format!("研究输入指纹不符: {key}"));
        }
    }
    for key in [
        "holding_label_endpoints_purged",
        "selection_did_not_read_outer_metrics",
        "prefix_causality",
        "whole_ledger_reconciled",
    ] {
        if row["checks"][key] != true {
            return Err(format!("研究核验未通过: {key}"));
        }
    }
    if row["checks"]["ST_BJ_buys"] != 0 {
        return Err("研究存在ST/北交所买入".into());
    }
    let candidates = row["selection_candidates"]
        .as_array()
        .ok_or("缺少选择记录")?;
    if candidates.len() != 12 {
        return Err("候选集合不完整".into());
    }
    let mut ids = std::collections::HashSet::new();
    for candidate in candidates {
        if !ids.insert(candidate["id"].as_str().ok_or("候选身份缺失")?)
            || candidate["last_training_label_endpoint"]
                .as_i64()
                .unwrap_or(i64::MAX)
                >= (year - 1) * 10000 + 101
            || candidate["last_selection_label_endpoint"]
                .as_i64()
                .unwrap_or(i64::MAX)
                >= year * 10000 + 101
        {
            return Err("训练/选择标签越过时间边界".into());
        }
    }
    let evaluations = row["evaluations"].as_array().ok_or("缺少外层评估")?;
    if evaluations.len() != 3 {
        return Err("三种选择风格评估不完整".into());
    }
    let mut policies = std::collections::HashSet::new();
    for ev in evaluations {
        let policy = ev["policy"].as_str().ok_or("选择风格缺失")?;
        if !["balanced", "return", "defensive"].contains(&policy) || !policies.insert(policy) {
            return Err("选择风格未知或重复".into());
        }
        let selected = selection_choice(candidates, policy)?;
        if ev["status"] == "cash" {
            if selected.is_some() {
                return Err("前一年有合格候选，不能按外层亏损改成现金等待".into());
            }
            if !ev["selected_id"].is_null() || ev["metrics"]["net_return_pct"] != 0 {
                return Err("现金等待不能伪称选中模型".into());
            }
            continue;
        }
        if selected.is_none_or(|row| {
            row["id"] != ev["selected_id"] || row["metrics"] != ev["selection_metrics"]
        }) || ev["selected_on"] != format!("{}-12-31", year - 1)
        {
            return Err("选中项不符合预先登记的前一年选择目标".into());
        }
        if ev["status"] != "evaluated"
            || !ids.contains(ev["selected_id"].as_str().ok_or("选中身份缺失")?)
            || ev["refit_last_label_endpoint"].as_i64().unwrap_or(i64::MAX) >= year * 10000 + 101
        {
            return Err("外层拟合身份或边界错误".into());
        }
        validate_artifact(
            Path::new(ev["model_path"].as_str().ok_or("模型路径缺失")?),
            ev["model_sha256"].as_str().ok_or("模型指纹缺失")?,
            base,
        )?;
        for name in [
            "metrics",
            "double_cost",
            "delayed_entry",
            "matched_pool_neutral",
        ] {
            for metric in ["net_return_pct", "max_drawdown_pct"] {
                if !ev[name][metric].as_f64().is_some_and(f64::is_finite) {
                    return Err("收益核验字段缺失".into());
                }
            }
        }
    }
    let accounts = row["accounts"].as_array().ok_or("账本清单缺失")?;
    if accounts.len() < 12 || accounts.len() > 24 {
        return Err("账本数量无效".into());
    }
    for account in accounts {
        validate_artifact(
            Path::new(account["path"].as_str().ok_or("账本路径缺失")?),
            account["sha256"].as_str().ok_or("账本指纹缺失")?,
            base,
        )?;
        if !account["accounting"]["reconcile_cny"]
            .as_f64()
            .is_some_and(|v| v.is_finite() && v.abs() <= 0.02)
            || !account["accounting"]["minimum_cash_cny"]
                .as_f64()
                .is_some_and(|v| v >= -0.01)
            || account["accounting"]["ST_BJ_buys"] != 0
        {
            return Err("现金账本核对失败".into());
        }
    }
    Ok(())
}
fn run_fold(
    db: &Database,
    id: i64,
    c: &ModelRunnerConfig,
    script: &Path,
    year: i64,
    output: &Path,
) -> Result<Value, String> {
    let stdout = output.with_extension("log");
    let stderr = output.with_extension("error.log");
    let mut command = std::process::Command::new(&c.python);
    command
        .arg(script)
        .args([
            "--year",
            &year.to_string(),
            "--snapshot",
            &c.snapshot,
            "--index",
            &c.index,
            "--output",
        ])
        .arg(output)
        .current_dir(&c.research_root)
        .env("PYTHONIOENCODING", "utf-8")
        .stdin(std::process::Stdio::null())
        .stdout(std::fs::File::create(&stdout).map_err(|e| e.to_string())?)
        .stderr(std::fs::File::create(&stderr).map_err(|e| e.to_string())?);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("研究Python无法启动: {e}"))?;
    let _job = research::own_research_process(&mut child)?;
    let began = std::time::Instant::now();
    loop {
        if db.research_job_cancelled(id) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("作业已取消；已完成年份保留".into());
        }
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            if !status.success() {
                let error = std::fs::read_to_string(&stderr).unwrap_or_default();
                return Err(format!(
                    "嵌套研究停止: {}；日志 {}",
                    error
                        .chars()
                        .rev()
                        .take(1600)
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect::<String>(),
                    stderr.display()
                ));
            }
            break;
        }
        if began.elapsed() > std::time::Duration::from_secs(600) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("当前年份研究超过600秒，停止并保留之前已核验年份".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    read_checked(output)
}
pub(crate) fn run_worker(db: &Database, id: i64) -> Result<(), String> {
    let job = db.research_job(id)?;
    let c: ModelRunnerConfig = if job["context"].is_null() {
        research::model_config(db)?
    } else {
        serde_json::from_value(job["context"]["config"].clone()).map_err(|_| "研究配置断点损坏")?
    };
    let (fingerprint, sources, script) = identity(&c)?;
    let base = if job["context"].is_null() {
        Path::new(&c.research_root)
            .join("research/nested-technical-v1/runs")
            .join(format!("job-{}-{}", id, uuid::Uuid::new_v4()))
    } else {
        PathBuf::from(
            job["context"]["output_dir"]
                .as_str()
                .ok_or("研究输出断点损坏")?,
        )
    };
    let intended = Path::new(&c.research_root).join("research/nested-technical-v1/runs");
    std::fs::create_dir_all(&intended).map_err(|e| e.to_string())?;
    if base
        .parent()
        .ok_or("研究输出缺父目录")?
        .canonicalize()
        .map_err(|e| e.to_string())?
        != intended.canonicalize().map_err(|e| e.to_string())?
    {
        return Err("研究输出父目录越界".into());
    }
    std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    if !base
        .canonicalize()
        .map_err(|e| e.to_string())?
        .starts_with(intended.canonicalize().map_err(|e| e.to_string())?)
    {
        return Err("研究输出目录越界".into());
    }
    let context = json!({"config":c,"sources":sources,"output_dir":base});
    db.prepare_research_job(id, &context, &fingerprint)?;
    let completed = job["results"].as_array().ok_or("研究断点列表损坏")?;
    for (n, year) in YEARS.iter().enumerate() {
        if db.research_job_cancelled(id) {
            return Err("研究作业已取消".into());
        }
        let key = format!("{SEARCH_ID}:{year}");
        if let Some(saved) = completed.iter().find(|s| s["key"] == key) {
            if saved["input_identity"] != fingerprint {
                return Err("研究断点输入身份改变".into());
            }
            let path = Path::new(saved["report_path"].as_str().ok_or("报告路径缺失")?);
            validate_artifact(
                path,
                saved["report_sha256"].as_str().ok_or("报告指纹缺失")?,
                &base,
            )?;
            validate_report(&read_checked(path)?, *year, &sources, &base)?;
            continue;
        }
        db.research_job_phase(
            id,
            "calculating",
            &format!(
                "第{}/5年：{year}；训练与前一年选择12项技术组合，独立评估三种风格",
                n + 1
            ),
        )?;
        let output = base.join(format!("year-{year}-{}.json", uuid::Uuid::new_v4()));
        let report = run_fold(db, id, &c, &script, *year, &output)?;
        db.research_job_phase(
            id,
            "verifying",
            "核对标签时间、候选身份、现金账本和因果特征；完成后保存年份断点",
        )?;
        let (after, _, _) = identity(&c)?;
        if after != fingerprint {
            return Err("计算中输入改变，当前年份未保存".into());
        }
        validate_report(&report, *year, &sources, &base)?;
        db.checkpoint_research_job(id,&json!({"key":key,"model_id":SEARCH_ID,"comparison":"nested","holding_days":0,"year":year,"as_of":report["as_of"],"input_identity":fingerprint,"report_path":output,"report_sha256":hash(&output)?,"evaluations":report["evaluations"],"bank_size":12,"accounts":report["accounts"].as_array().map(Vec::len),"state":"research_only","production_admission":false}))?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_uses_only_inner_metrics_and_cash_is_a_real_option() {
        let rows = vec![
            json!({"id":"a","metrics":{"net_return_pct":-1.,"max_drawdown_pct":1.,"completed_holding_cycles":60}}),
            json!({"id":"b","metrics":{"net_return_pct":5.,"max_drawdown_pct":1.,"completed_holding_cycles":60}}),
            json!({"id":"c","metrics":{"net_return_pct":10.,"max_drawdown_pct":20.,"completed_holding_cycles":60}}),
        ];
        assert_eq!(
            selection_choice(&rows, "balanced").unwrap().unwrap()["id"],
            "b"
        );
        assert_eq!(
            selection_choice(&rows, "return").unwrap().unwrap()["id"],
            "c"
        );
        assert_eq!(
            selection_choice(&rows, "defensive").unwrap().unwrap()["id"],
            "b"
        );
        assert!(selection_choice(&rows[..1], "balanced").unwrap().is_none());
    }
    #[test]
    fn report_rejects_admission_and_time_leakage() {
        let source = json!({});
        let row = json!({"schema":"ashare-nested-technical-v1","year":2022,"bank_size":12,"production_admission":true,"new_admitted_models":1,"later_history_already_seen":true});
        assert!(validate_report(&row, 2022, &source, Path::new(".")).is_err());
    }
}

#[cfg(test)]
mod real_tests {
    use super::*;
    #[test]
    #[ignore = "Runs all five real nested folds in an isolated SQLite database; requires local research data"]
    fn real_nested_research_job_checkpoints_and_recovery() {
        assert_eq!(
            std::env::var("BULL_NESTED_SMOKE").ok().as_deref(),
            Some("1")
        );
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let output = root.join("research/nested-technical-v1/verification-2026-10-04");
        std::fs::create_dir_all(&output).unwrap();
        let db_path = output.join(format!("isolated-db-{}", uuid::Uuid::new_v4()));
        let db = Database::open(db_path.clone()).unwrap();
        let mut config = research::model_config(&db).unwrap();
        if let Ok(python) = std::env::var("BULL_RESEARCH_PYTHON") {
            config.python = python;
        }
        db.set_setting(
            "model_runner_config",
            &serde_json::to_string(&config).unwrap(),
        )
        .unwrap();
        let request = json!({"kind":"explore","models":[SEARCH_ID],"comparisons":["baseline"],"continuation":null});
        let id = db.create_research_job("explore", &request, 5).unwrap();
        db.activate_research_job(id).unwrap();
        run_worker(&db, id).unwrap();
        let job = db.research_job(id).unwrap();
        assert_eq!(job["completed"], 5);
        assert_eq!(job["results"].as_array().unwrap().len(), 5);
        assert!(
            db.model_runs().unwrap().as_array().unwrap().is_empty(),
            "exploration cannot silently register frozen accounts"
        );
        // Simulate an application restart before final completion; all five verified years are reused.
        assert_eq!(db.recover_research_jobs().unwrap(), 1);
        db.activate_research_job(id).unwrap();
        run_worker(&db, id).unwrap();
        db.finish_research_job(id, "complete", "五年嵌套研究核验完成")
            .unwrap();
        let reopened = Database::open(db_path.clone()).unwrap();
        assert_eq!(reopened.research_job(id).unwrap()["completed"], 5);
        let list = reopened.research_jobs().unwrap();
        assert_eq!(
            list[0]["results"][0]["evaluations"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        let verified = json!({"tested_at":chrono::Utc::now().to_rfc3339(),"database":db_path,"job":reopened.research_job(id).unwrap(),"resume_skipped_verified_years":true,"registered_model_accounts_created":0,"native_popup_observed":false});
        std::fs::write(
            output.join("job-validation.json"),
            serde_json::to_string_pretty(&verified).unwrap(),
        )
        .unwrap();
        println!("five-year research verification: {}", output.display());
    }
}

#[cfg(test)]
mod saved_acceptance_tests {
    use super::*;
    #[test]
    #[ignore = "Revalidates saved real five-year output and fingerprints with the current checker"]
    fn saved_nested_result_is_consistent_with_current_selection_checker() {
        assert_eq!(
            std::env::var("BULL_NESTED_SMOKE").ok().as_deref(),
            Some("1")
        );
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let verification = root.join("research/nested-technical-v1/verification-2026-10-04");
        let value: Value = serde_json::from_slice(
            &std::fs::read(verification.join("job-validation.json")).unwrap(),
        )
        .unwrap();
        let path = Path::new(value["database"].as_str().unwrap());
        assert!(path
            .canonicalize()
            .unwrap()
            .starts_with(verification.canonicalize().unwrap()));
        let db = Database::open(path.to_owned()).unwrap();
        let id = value["job"]["id"].as_i64().unwrap();
        let job = db.research_job(id).unwrap();
        let c: ModelRunnerConfig =
            serde_json::from_value(job["context"]["config"].clone()).unwrap();
        let (fp, sources, _) = identity(&c).unwrap();
        assert_eq!(job["fingerprint"], fp);
        let base = Path::new(job["context"]["output_dir"].as_str().unwrap());
        for step in job["results"].as_array().unwrap() {
            let report_path = Path::new(step["report_path"].as_str().unwrap());
            validate_artifact(report_path, step["report_sha256"].as_str().unwrap(), base).unwrap();
            validate_report(
                &read_checked(report_path).unwrap(),
                step["year"].as_i64().unwrap(),
                &sources,
                base,
            )
            .unwrap();
        }
        assert!(db.model_runs().unwrap().as_array().unwrap().is_empty());
        println!("All five years, selection rules and saved source fingerprints verified");
    }
}
