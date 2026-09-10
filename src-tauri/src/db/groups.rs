use super::{Database, WatchItem};
use rusqlite::{params, Result as SqliteResult};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct WatchGroup {
    pub id: i64,
    pub name: String,
    pub sort_order: i64,
    pub item_count: i64,
}

#[derive(Debug, Serialize)]
pub struct GroupSnapshot {
    pub active_group_id: i64,
    pub groups: Vec<WatchGroup>,
    pub items: Vec<WatchItem>,
}

impl Database {
    /// 0 是固定“全部”的协议标识，自定义组 ID 始终为正数。
    pub fn migrate_groups(&self) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute_batch("CREATE TABLE IF NOT EXISTS watch_groups (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL UNIQUE,
            sort_order INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS watch_group_members (
            group_id INTEGER NOT NULL,
            watch_id INTEGER NOT NULL,
            sort_order INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY(group_id, watch_id)
        );
        CREATE INDEX IF NOT EXISTS group_members_watch ON watch_group_members(watch_id);
        INSERT OR IGNORE INTO settings(key,value) VALUES ('active_group_id','0');")?;
        Ok(())
    }

    pub fn get_current_watch_codes(&self) -> SqliteResult<Vec<(String, String)>> {
        Ok(self.get_current_watchlist()?.into_iter().map(|item| (item.code, item.market)).collect())
    }

    pub fn get_current_watchlist(&self) -> SqliteResult<Vec<WatchItem>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let id: i64 = conn.query_row("SELECT COALESCE(CAST((SELECT value FROM settings WHERE key='active_group_id') AS INTEGER),0)", [], |r| r.get(0))?;
        let valid = id > 0 && conn.query_row("SELECT EXISTS(SELECT 1 FROM watch_groups WHERE id=?1)", [id], |r| r.get::<_, bool>(0))?;
        let sql = if valid {
            "SELECT w.id,w.code,w.market,w.name,m.sort_order,w.added_at FROM watchlist w JOIN watch_group_members m ON m.watch_id=w.id WHERE m.group_id=?1 ORDER BY m.sort_order,w.id"
        } else {
            "SELECT id,code,market,name,sort_order,added_at FROM watchlist WHERE ?1 IS NOT NULL ORDER BY sort_order,id"
        };
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map([id], |row| Ok(WatchItem {
            id: row.get(0)?, code: row.get(1)?, market: row.get(2)?, name: row.get(3)?, sort_order: row.get(4)?, added_at: row.get(5)?,
        }))?;
        rows.collect()
    }

    pub fn get_group_snapshot(&self) -> SqliteResult<GroupSnapshot> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let groups = {
            let mut stmt = conn.prepare("SELECT g.id,g.name,g.sort_order,COUNT(w.id) FROM watch_groups g LEFT JOIN watch_group_members m ON m.group_id=g.id LEFT JOIN watchlist w ON w.id=m.watch_id GROUP BY g.id ORDER BY g.sort_order,g.id")?;
            let rows = stmt.query_map([], |r| Ok(WatchGroup { id: r.get(0)?, name: r.get(1)?, sort_order: r.get(2)?, item_count: r.get(3)? }))?;
            rows.collect::<SqliteResult<Vec<_>>>()?
        };
        let saved: i64 = conn.query_row("SELECT COALESCE(CAST((SELECT value FROM settings WHERE key='active_group_id') AS INTEGER),0)", [], |row| row.get(0))?;
        let active_group_id = if groups.iter().any(|g| g.id == saved) { saved } else { 0 };
        let sql = if active_group_id == 0 {
            "SELECT id,code,market,name,sort_order,added_at FROM watchlist WHERE ?1 IS NOT NULL ORDER BY sort_order,id"
        } else {
            "SELECT w.id,w.code,w.market,w.name,m.sort_order,w.added_at FROM watchlist w JOIN watch_group_members m ON m.watch_id=w.id WHERE m.group_id=?1 ORDER BY m.sort_order,w.id"
        };
        let mut stmt = conn.prepare(sql)?;
        let items = stmt.query_map([active_group_id], |row| Ok(WatchItem {
            id: row.get(0)?, code: row.get(1)?, market: row.get(2)?, name: row.get(3)?, sort_order: row.get(4)?, added_at: row.get(5)?,
        }))?.collect::<SqliteResult<Vec<_>>>()?;
        Ok(GroupSnapshot { active_group_id, groups, items })
    }

    pub fn move_current_group_item(&self, watch_id: i64, direction: &str) -> Result<(), String> {
        let snapshot = self.get_group_snapshot().map_err(|e| e.to_string())?;
        if snapshot.active_group_id == 0 {
            return match direction {
                "top" => self.move_watch_top(watch_id),
                "up" => self.move_watch_up(watch_id),
                _ => self.move_watch_down(watch_id),
            }.map_err(|e| e.to_string());
        }
        let mut ids: Vec<i64> = snapshot.items.iter().map(|item| item.id).collect();
        let index = ids.iter().position(|id| *id == watch_id).ok_or("股票不在当前分组")?;
        let target = match direction { "top" => 0, "up" => index.saturating_sub(1), _ => (index + 1).min(ids.len() - 1) };
        ids.remove(index);
        ids.insert(target,watch_id);
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        for (order,id) in ids.iter().enumerate() {
            tx.execute("UPDATE watch_group_members SET sort_order=?1 WHERE group_id=?2 AND watch_id=?3", params![order as i64,snapshot.active_group_id,id]).map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }

    pub fn create_watch_group(&self, name: &str) -> Result<i64, String> {
        let name = validate_name(name)?;
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute("INSERT INTO watch_groups(name,sort_order) VALUES (?1,(SELECT COALESCE(MAX(sort_order),-1)+1 FROM watch_groups))", [name]).map_err(|e| e.to_string())?;
        Ok(conn.last_insert_rowid())
    }

    pub fn rename_watch_group(&self, id: i64, name: &str) -> Result<(), String> {
        if id <= 0 { return Err("全部分组不能重命名".into()); }
        let name = validate_name(name)?;
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let count = conn.execute("UPDATE watch_groups SET name=?1 WHERE id=?2", params![name,id]).map_err(|e| e.to_string())?;
        if count == 0 { return Err("分组不存在".into()); }
        Ok(())
    }

    pub fn select_watch_group(&self, id: i64) -> Result<(), String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        if id < 0 || (id != 0 && !conn.query_row("SELECT EXISTS(SELECT 1 FROM watch_groups WHERE id=?1)", [id], |r| r.get::<_, bool>(0)).map_err(|e| e.to_string())?) {
            return Err("分组不存在".into());
        }
        conn.execute("INSERT OR REPLACE INTO settings(key,value) VALUES('active_group_id',?1)", [id.to_string()]).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn delete_watch_group(&self, id: i64) -> Result<(), String> {
        if id <= 0 { return Err("全部分组不能删除".into()); }
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM watch_group_members WHERE group_id=?1", [id]).map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM watch_groups WHERE id=?1", [id]).map_err(|e| e.to_string())?;
        tx.execute("UPDATE settings SET value='0' WHERE key='active_group_id' AND value=?1", [id.to_string()]).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }

    pub fn add_watch_to_group(&self, code: &str, market: &str, name: &str, group_id: i64) -> Result<(), String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        if group_id < 0 || (group_id > 0 && !tx.query_row("SELECT EXISTS(SELECT 1 FROM watch_groups WHERE id=?1)", [group_id], |row| row.get::<_,bool>(0)).map_err(|e|e.to_string())?) {
            return Err("目标分组不存在".into());
        }
        tx.execute("INSERT OR IGNORE INTO watchlist(code,market,name,sort_order,added_at) VALUES (?1,?2,?3,(SELECT COALESCE(MAX(sort_order),-1)+1 FROM watchlist),?4)",params![code,market,name,chrono::Utc::now().to_rfc3339()]).map_err(|e|e.to_string())?;
        if group_id > 0 {
            let watch_id:i64 = tx.query_row("SELECT id FROM watchlist WHERE code=?1 AND market=?2",params![code,market],|row|row.get(0)).map_err(|e|e.to_string())?;
            tx.execute("INSERT OR IGNORE INTO watch_group_members(group_id,watch_id,sort_order) VALUES (?1,?2,(SELECT COALESCE(MAX(sort_order),-1)+1 FROM watch_group_members WHERE group_id=?1))",params![group_id,watch_id]).map_err(|e|e.to_string())?;
        }
        tx.commit().map_err(|e|e.to_string())
    }

    pub fn set_group_member(&self, group_id: i64, watch_id: i64, included: bool) -> Result<(), String> {
        if group_id <= 0 { return Err("请操作自定义分组；全部自动包含所有自选".into()); }
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let exists: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM watch_groups WHERE id=?1) AND EXISTS(SELECT 1 FROM watchlist WHERE id=?2)", params![group_id,watch_id], |r| r.get(0)).map_err(|e| e.to_string())?;
        if !exists { return Err("分组或股票不存在".into()); }
        if included {
            conn.execute("INSERT OR IGNORE INTO watch_group_members(group_id,watch_id,sort_order) VALUES (?1,?2,(SELECT COALESCE(MAX(sort_order),-1)+1 FROM watch_group_members WHERE group_id=?1))", params![group_id,watch_id]).map_err(|e| e.to_string())?;
        } else {
            conn.execute("DELETE FROM watch_group_members WHERE group_id=?1 AND watch_id=?2", params![group_id,watch_id]).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

fn validate_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.is_empty() || name == "全部" || name.chars().count() > 30 {
        return Err("分组名称应为 1–30 个字符，且不能命名为全部".into());
    }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    fn database() -> Database {
        let db = Database { conn: Mutex::new(rusqlite::Connection::open_in_memory().unwrap()) };
        db.migrate().unwrap();
        db.migrate_groups().unwrap();
        db
    }
    #[test]
    fn groups_share_stocks_and_keep_all_on_delete() {
        let db = database();
        db.add_watch("sz000001", "CN", "测试股票").unwrap();
        let id = db.get_watchlist().unwrap()[0].id;
        let a = db.create_watch_group("观察").unwrap();
        let b = db.create_watch_group("持有").unwrap();
        db.set_group_member(a,id,true).unwrap();
        db.set_group_member(b,id,true).unwrap();
        db.set_group_member(a,id,true).unwrap();
        db.select_watch_group(a).unwrap();
        assert_eq!(db.get_current_watch_codes().unwrap().len(),1);
        db.set_group_member(a,id,false).unwrap();
        assert!(db.get_current_watch_codes().unwrap().is_empty());
        db.delete_watch_group(a).unwrap();
        assert_eq!(db.get_group_snapshot().unwrap().active_group_id,0);
        assert_eq!(db.get_current_watch_codes().unwrap().len(),1);
        db.select_watch_group(b).unwrap();
        assert_eq!(db.get_current_watch_codes().unwrap().len(),1);
        db.migrate_groups().unwrap();
        assert_eq!(db.get_watchlist().unwrap().len(),1);
    }
    #[test]
    fn rejects_reserved_and_invalid_groups() {
        let db = database();
        assert!(db.create_watch_group(" 全部 ").is_err());
        assert!(db.create_watch_group(" ").is_err());
        assert!(db.select_watch_group(999).is_err());
        assert!(db.delete_watch_group(0).is_err());
    }
}
