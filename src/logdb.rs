//! SQLite 日志落库（阶段 C）：stdout/stderr 行级日志经 UI 拉取后同步写入本地 SQLite，
//! 支持按来源（服务器 / 隧道）查询、行数上限轮转（超限自动删除最旧行），替代内存日志的
//! 持久化缺口。HTTP API 不做（本地开放端口有安全风险且无明确用例）。

use rusqlite::{Connection, params};
use std::path::Path;

/// 日志库默认行数上限（轮转水位）
pub const DEFAULT_MAX_ROWS: i64 = 50_000;

pub struct LogDb {
    conn: Connection,
    max_rows: i64,
}

#[derive(Debug, Clone)]
pub struct LogRow {
    pub id: i64,
    pub ts: String,
    pub src: String,
    pub line: String,
}

impl LogDb {
    /// 打开（不存在则创建）数据库，WAL 模式 + 日志表 + 来源索引。
    pub fn open(path: &Path, max_rows: i64) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;\
             CREATE TABLE IF NOT EXISTS logs(\
               id INTEGER PRIMARY KEY AUTOINCREMENT,\
               ts TEXT NOT NULL,\
               src TEXT NOT NULL,\
               line TEXT NOT NULL\
             );\
             CREATE INDEX IF NOT EXISTS idx_logs_src ON logs(src);\
             CREATE INDEX IF NOT EXISTS idx_logs_id ON logs(id);",
        )
        .map_err(|e| e.to_string())?;
        Ok(LogDb {
            conn,
            max_rows: if max_rows > 0 { max_rows } else { DEFAULT_MAX_ROWS },
        })
    }

    /// 批量写入一行或多行；写入前执行行数轮转（仅当超限时删除最旧行）。
    pub fn insert_batch(&mut self, src: &str, lines: &[String]) {
        if lines.is_empty() {
            return;
        }
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let max = self.max_rows;
        let src = src.to_string();
        let _ = (|| -> Result<(), rusqlite::Error> {
            let tx = self.conn.transaction()?;
            {
                let mut stmt = tx.prepare("INSERT INTO logs(ts, src, line) VALUES(?1, ?2, ?3)")?;
                for l in lines {
                    stmt.execute(params![now, src, l])?;
                }
            }
            let cnt: i64 = tx.query_row("SELECT COUNT(*) FROM logs", [], |r| r.get(0))?;
            if cnt > max {
                let del = cnt - max;
                tx.execute(
                    "DELETE FROM logs WHERE id IN (SELECT id FROM logs ORDER BY id ASC LIMIT ?1)",
                    params![del],
                )?;
            }
            tx.commit()
        })();
    }

    /// 查询最近 N 条；可选按来源过滤（src 精确匹配）。
    pub fn recent(&self, limit: i64, src_filter: Option<&str>) -> Vec<LogRow> {
        let limit = if limit <= 0 { 100 } else { limit };
        let mut out = Vec::new();
        fn map_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<LogRow> {
            Ok(LogRow {
                id: r.get(0)?,
                ts: r.get(1)?,
                src: r.get(2)?,
                line: r.get(3)?,
            })
        }
        let collected: Vec<LogRow> = match src_filter {
            Some(src) => {
                let mut stmt = match self.conn.prepare(
                    "SELECT id, ts, src, line FROM logs WHERE src = ?1 ORDER BY id DESC LIMIT ?2",
                ) {
                    Ok(s) => s,
                    Err(_) => return out,
                };
                let mut rows = match stmt.query_map(params![src, limit], map_row) {
                    Ok(r) => r,
                    Err(_) => return out,
                };
                rows.collect::<rusqlite::Result<Vec<_>>>().unwrap_or_default()
            }
            None => {
                let mut stmt = match self
                    .conn
                    .prepare("SELECT id, ts, src, line FROM logs ORDER BY id DESC LIMIT ?1")
                {
                    Ok(s) => s,
                    Err(_) => return out,
                };
                let mut rows = match stmt.query_map(params![limit], map_row) {
                    Ok(r) => r,
                    Err(_) => return out,
                };
                rows.collect::<rusqlite::Result<Vec<_>>>().unwrap_or_default()
            }
        };
        out.extend(collected);
                out.reverse(); // 时间正序（旧→新）
        out
    }

    /// 当前总行数。
    pub fn count(&self) -> i64 {
        self.conn
            .query_row("SELECT COUNT(*) FROM logs", [], |r| r.get(0))
            .unwrap_or(0)
    }

    /// 清空日志库（整表删除，保留表结构）。
    pub fn clear(&mut self) {
        let _ = self.conn.execute("DELETE FROM logs", []);
    }
}
