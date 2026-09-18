use super::Database;
use crate::domain::KLineData;
use rusqlite::{params, Result as SqliteResult};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct CalibrationBucket {
    pub confidence_min: u8,
    pub confidence_max: u8,
    pub count: usize,
    pub actual_hit_rate: f64,
}

#[derive(Debug, Serialize)]
pub struct PredictionCalibration {
    pub status: String,
    pub verified: usize,
    pub span_days: i64,
    pub required_verified: usize,
    pub required_span_days: i64,
    pub buckets: Vec<CalibrationBucket>,
}

impl Database {
    pub fn migrate_predictions(&self) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS agent_predictions(
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               fingerprint TEXT NOT NULL UNIQUE,
               symbol TEXT NOT NULL,
               as_of TEXT NOT NULL,
               entry_price REAL NOT NULL,
               conclusion TEXT NOT NULL,
               confidence INTEGER NOT NULL,
               evidence_json TEXT NOT NULL,
               model_version TEXT NOT NULL,
               predicted_at TEXT NOT NULL,
               verified_date TEXT,
               realized_return_pct REAL,
               correct INTEGER
             );
             CREATE INDEX IF NOT EXISTS idx_agent_predictions_pending
             ON agent_predictions(symbol,verified_date);",
        )
    }

    pub fn save_agent_prediction(
        &self,
        fingerprint: &str,
        symbol: &str,
        as_of: &str,
        entry_price: f64,
        conclusion: &str,
        confidence: u8,
        evidence_json: &str,
        model_version: &str,
    ) -> Result<(), String> {
        if !entry_price.is_finite() || entry_price <= 0.0 || confidence > 100 {
            return Err("预测价格或置信度无效".into());
        }
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        conn.execute(
            "INSERT OR IGNORE INTO agent_predictions
             (fingerprint,symbol,as_of,entry_price,conclusion,confidence,evidence_json,model_version,predicted_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![fingerprint,symbol,as_of,entry_price,conclusion,confidence,evidence_json,model_version,chrono::Utc::now().to_rfc3339()],
        )
        .map(|_| ())
        .map_err(|error| error.to_string())
    }

    pub fn settle_agent_predictions(
        &self,
        symbol: &str,
        klines: &[KLineData],
    ) -> Result<usize, String> {
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        let pending = {
            let mut stmt = conn
                .prepare("SELECT id,as_of,entry_price,conclusion FROM agent_predictions WHERE symbol=?1 AND verified_date IS NULL")
                .map_err(|error| error.to_string())?;
            let rows = stmt
                .query_map([symbol], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, f64>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })
                .map_err(|error| error.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?;
            rows
        };
        let mut settled = 0;
        for (id, as_of, entry_price, conclusion) in pending {
            let Some(next) = klines.iter().find(|bar| bar.date > as_of) else {
                continue;
            };
            let return_pct = (next.close / entry_price - 1.0) * 100.0;
            let correct = match conclusion.as_str() {
                "bullish" => return_pct > 0.0,
                "bearish" | "cautious" => return_pct < 0.0,
                "neutral" => return_pct.abs() <= 1.0,
                _ => false,
            };
            conn.execute(
                "UPDATE agent_predictions SET verified_date=?2,realized_return_pct=?3,correct=?4 WHERE id=?1 AND verified_date IS NULL",
                params![id,next.date,return_pct,i64::from(correct)],
            )
            .map_err(|error| error.to_string())?;
            settled += 1;
        }
        Ok(settled)
    }

    pub fn prediction_calibration(&self) -> Result<PredictionCalibration, String> {
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        let rows = {
            let mut stmt = conn
                .prepare("SELECT confidence,correct,predicted_at,verified_date FROM agent_predictions WHERE verified_date IS NOT NULL ORDER BY id")
                .map_err(|error| error.to_string())?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, u8>(0)?,
                        row.get::<_, i64>(1)? != 0,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })
                .map_err(|error| error.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?;
            rows
        };
        let span_days = rows
            .first()
            .and_then(|row| {
                chrono::NaiveDate::parse_from_str(&row.2[..10.min(row.2.len())], "%Y-%m-%d").ok()
            })
            .zip(
                rows.last()
                    .and_then(|row| chrono::NaiveDate::parse_from_str(&row.3, "%Y-%m-%d").ok()),
            )
            .map(|(first, last)| (last - first).num_days())
            .unwrap_or(0);
        let mut buckets = Vec::new();
        for start in [0u8, 20, 40, 60, 80] {
            let end = if start == 80 { 100 } else { start + 19 };
            let values: Vec<_> = rows
                .iter()
                .filter(|row| row.0 >= start && row.0 <= end)
                .collect();
            if !values.is_empty() {
                buckets.push(CalibrationBucket {
                    confidence_min: start,
                    confidence_max: end,
                    count: values.len(),
                    actual_hit_rate: values.iter().filter(|row| row.1).count() as f64
                        / values.len() as f64
                        * 100.0,
                });
            }
        }
        Ok(PredictionCalibration {
            status: if rows.len() >= 200 && span_days >= 28 {
                "ready"
            } else {
                "observing"
            }
            .into(),
            verified: rows.len(),
            span_days,
            required_verified: 200,
            required_span_days: 28,
            buckets,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn predictions_settle_on_first_later_bar_and_stay_observing() {
        let db = Database {
            conn: Mutex::new(rusqlite::Connection::open_in_memory().unwrap()),
        };
        db.migrate_predictions().unwrap();
        db.save_agent_prediction(
            "fp1",
            "sh600000",
            "2026-09-17",
            10.0,
            "bullish",
            80,
            "[]",
            "test",
        )
        .unwrap();
        let bars = vec![
            KLineData {
                date: "2026-09-17".into(),
                open: 10.0,
                high: 10.0,
                low: 10.0,
                close: 10.0,
                volume: 1,
                turnover: 0.0,
            },
            KLineData {
                date: "2026-09-18".into(),
                open: 10.0,
                high: 11.0,
                low: 10.0,
                close: 11.0,
                volume: 1,
                turnover: 0.0,
            },
        ];
        assert_eq!(db.settle_agent_predictions("sh600000", &bars).unwrap(), 1);
        let calibration = db.prediction_calibration().unwrap();
        assert_eq!(calibration.verified, 1);
        assert_eq!(calibration.status, "observing");
        assert_eq!(calibration.buckets[0].actual_hit_rate, 100.0);
    }
}
