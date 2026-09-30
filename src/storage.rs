use crate::models::{ParseRecord, ParseStatus};
use rusqlite::{params, Connection};
use std::path::Path;

pub struct Storage {
    conn: Connection,
}

impl Storage {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| format!("创建数据目录失败：{err}"))?;
        }
        let conn = Connection::open(path).map_err(|err| format!("打开数据库失败：{err}"))?;
        let storage = Self { conn };
        storage.migrate()?;
        Ok(storage)
    }

    fn migrate(&self) -> Result<(), String> {
        self.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS parse_records (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    file_name TEXT NOT NULL,
                    file_path TEXT NOT NULL,
                    file_size TEXT NOT NULL,
                    modified_at TEXT NOT NULL,
                    detected_at TEXT NOT NULL,
                    parsed_at TEXT NOT NULL,
                    parser_id TEXT NOT NULL,
                    transformer_id TEXT NOT NULL,
                    status TEXT NOT NULL,
                    error_message TEXT NOT NULL,
                    raw_json TEXT NOT NULL,
                    final_json TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_parse_records_created_at ON parse_records(created_at);
                CREATE INDEX IF NOT EXISTS idx_parse_records_file_path ON parse_records(file_path);",
            )
            .map_err(|err| format!("初始化数据库失败：{err}"))
    }

    pub fn list_records(&self) -> Result<Vec<ParseRecord>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, file_name, file_path, file_size, modified_at, detected_at, parsed_at,
                        parser_id, transformer_id, status, error_message, raw_json, final_json
                 FROM parse_records
                 ORDER BY id DESC
                 LIMIT 500",
            )
            .map_err(|err| format!("查询记录失败：{err}"))?;

        let rows = stmt
            .query_map([], |row| {
                let status: String = row.get(9)?;
                Ok(ParseRecord {
                    id: row.get::<_, i64>(0)? as u64,
                    file_name: row.get(1)?,
                    file_path: row.get(2)?,
                    file_size: row.get(3)?,
                    modified_at: row.get(4)?,
                    detected_at: row.get(5)?,
                    parsed_at: row.get(6)?,
                    parser_id: row.get(7)?,
                    transformer_id: row.get(8)?,
                    status: ParseStatus::from_code(&status),
                    error_message: row.get(10)?,
                    raw_json: row.get(11)?,
                    final_json: row.get(12)?,
                })
            })
            .map_err(|err| format!("读取记录失败：{err}"))?;

        let mut records = Vec::new();
        for row in rows {
            records.push(row.map_err(|err| format!("转换记录失败：{err}"))?);
        }
        Ok(records)
    }

    pub fn update_record(&self, record: &ParseRecord) -> Result<(), String> {
        let updated_at = chrono::Local::now().to_rfc3339();
        self.conn
            .execute(
                "UPDATE parse_records
                 SET file_name = ?1,
                     file_path = ?2,
                     file_size = ?3,
                     modified_at = ?4,
                     detected_at = ?5,
                     parsed_at = ?6,
                     parser_id = ?7,
                     transformer_id = ?8,
                     status = ?9,
                     error_message = ?10,
                     raw_json = ?11,
                     final_json = ?12,
                     updated_at = ?13
                 WHERE id = ?14",
                params![
                    record.file_name,
                    record.file_path,
                    record.file_size,
                    record.modified_at,
                    record.detected_at,
                    record.parsed_at,
                    record.parser_id,
                    record.transformer_id,
                    record.status.as_code(),
                    record.error_message,
                    record.raw_json,
                    record.final_json,
                    updated_at,
                    record.id as i64,
                ],
            )
            .map_err(|err| format!("更新记录失败：{err}"))?;
        Ok(())
    }

    pub fn insert_record(&self, record: &ParseRecord) -> Result<u64, String> {
        let now = chrono::Local::now().to_rfc3339();
        self.conn
            .execute(
                "INSERT INTO parse_records (
                    file_name, file_path, file_size, modified_at, detected_at, parsed_at,
                    parser_id, transformer_id, status, error_message, raw_json, final_json,
                    created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                params![
                    record.file_name,
                    record.file_path,
                    record.file_size,
                    record.modified_at,
                    record.detected_at,
                    record.parsed_at,
                    record.parser_id,
                    record.transformer_id,
                    record.status.as_code(),
                    record.error_message,
                    record.raw_json,
                    record.final_json,
                    now,
                    now,
                ],
            )
            .map_err(|err| format!("新增记录失败：{err}"))?;
        Ok(self.conn.last_insert_rowid() as u64)
    }

    pub fn delete_record(&self, id: u64) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM parse_records WHERE id = ?1", params![id as i64])
            .map_err(|err| format!("删除记录失败：{err}"))?;
        Ok(())
    }

    pub fn prune_records(&self, retention_days: u32) -> Result<usize, String> {
        let cutoff = chrono::Local::now()
            .checked_sub_signed(chrono::Duration::days(retention_days.max(1) as i64))
            .unwrap_or_else(chrono::Local::now)
            .to_rfc3339();
        self.conn
            .execute("DELETE FROM parse_records WHERE created_at < ?1", params![cutoff])
            .map_err(|err| format!("清理过期记录失败：{err}"))
    }
}
