//! SQLite storage for paired devices and chat messages.

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, Row, params};
use std::path::Path;
use std::sync::Mutex;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS devices (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  token_hash TEXT NOT NULL,
  can_write INTEGER NOT NULL DEFAULT 1,
  created_at INTEGER NOT NULL,
  last_seen_at INTEGER
);
CREATE TABLE IF NOT EXISTS messages (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
  from_host INTEGER NOT NULL,
  client_id TEXT,
  kind TEXT NOT NULL,
  text TEXT,
  file_path TEXT,
  file_name TEXT,
  file_size INTEGER,
  mime TEXT,
  created_at INTEGER NOT NULL,
  file_root TEXT
);
CREATE INDEX IF NOT EXISTS messages_by_device ON messages(device_id, id);
CREATE UNIQUE INDEX IF NOT EXISTS messages_client_id
  ON messages(device_id, from_host, client_id) WHERE client_id IS NOT NULL;
"#;

#[derive(Clone, Debug)]
pub struct DeviceRow {
    pub id: String,
    pub name: String,
    pub token_hash: String,
    pub can_write: bool,
    pub created_at: i64,
    pub last_seen_at: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct MessageRow {
    pub id: i64,
    pub device_id: String,
    pub from_host: bool,
    pub client_id: Option<String>,
    pub kind: String,
    pub text: Option<String>,
    pub file_path: Option<String>,
    pub file_name: Option<String>,
    pub file_size: Option<i64>,
    pub mime: Option<String>,
    pub created_at: i64,
    /// Shared-folder root the attachment was saved under (absolute path).
    pub file_root: Option<String>,
}

pub struct NewMessage<'a> {
    pub device_id: &'a str,
    pub from_host: bool,
    pub client_id: Option<&'a str>,
    pub kind: &'a str,
    pub text: Option<&'a str>,
    pub file_path: Option<&'a str>,
    pub file_name: Option<&'a str>,
    pub file_size: Option<i64>,
    pub mime: Option<&'a str>,
    pub created_at: i64,
    pub file_root: Option<&'a str>,
}

pub struct Db {
    conn: Mutex<Connection>,
}

const DEVICE_COLS: &str = "id, name, token_hash, can_write, created_at, last_seen_at";
const MESSAGE_COLS: &str = "id, device_id, from_host, client_id, kind, text, file_path, file_name, file_size, mime, created_at, file_root";

fn device_from(row: &Row) -> rusqlite::Result<DeviceRow> {
    Ok(DeviceRow {
        id: row.get(0)?,
        name: row.get(1)?,
        token_hash: row.get(2)?,
        can_write: row.get::<_, i64>(3)? != 0,
        created_at: row.get(4)?,
        last_seen_at: row.get(5)?,
    })
}

fn message_from(row: &Row) -> rusqlite::Result<MessageRow> {
    Ok(MessageRow {
        id: row.get(0)?,
        device_id: row.get(1)?,
        from_host: row.get::<_, i64>(2)? != 0,
        client_id: row.get(3)?,
        kind: row.get(4)?,
        text: row.get(5)?,
        file_path: row.get(6)?,
        file_name: row.get(7)?,
        file_size: row.get(8)?,
        mime: row.get(9)?,
        created_at: row.get(10)?,
        file_root: row.get(11)?,
    })
}

impl Db {
    pub fn open(path: &Path) -> Result<Db> {
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Db> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Db> {
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")?;
        conn.execute_batch(SCHEMA)?;
        // Databases created before attachments remembered their root.
        let has_root: i64 =
            conn.query_row("SELECT COUNT(*) FROM pragma_table_info('messages') WHERE name = 'file_root'", [], |r| r.get(0))?;
        if has_root == 0 {
            conn.execute_batch("ALTER TABLE messages ADD COLUMN file_root TEXT;")?;
        }
        Ok(Db { conn: Mutex::new(conn) })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }

    // ---- devices ----

    pub fn insert_device(&self, d: &DeviceRow) -> Result<()> {
        self.conn().execute(
            &format!("INSERT INTO devices ({DEVICE_COLS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6)"),
            params![d.id, d.name, d.token_hash, d.can_write as i64, d.created_at, d.last_seen_at],
        )?;
        Ok(())
    }

    pub fn device(&self, id: &str) -> Result<Option<DeviceRow>> {
        Ok(self
            .conn()
            .query_row(&format!("SELECT {DEVICE_COLS} FROM devices WHERE id = ?1"), params![id], device_from)
            .optional()?)
    }

    pub fn devices(&self) -> Result<Vec<DeviceRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!("SELECT {DEVICE_COLS} FROM devices ORDER BY created_at"))?;
        let rows = stmt.query_map([], device_from)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Returns false if the device does not exist.
    pub fn update_device(&self, id: &str, name: Option<&str>, can_write: Option<bool>) -> Result<bool> {
        let conn = self.conn();
        let mut changed = 0;
        if let Some(name) = name {
            changed += conn.execute("UPDATE devices SET name = ?2 WHERE id = ?1", params![id, name])?;
        }
        if let Some(w) = can_write {
            changed += conn.execute("UPDATE devices SET can_write = ?2 WHERE id = ?1", params![id, w as i64])?;
        }
        if name.is_none() && can_write.is_none() {
            changed = conn.query_row("SELECT COUNT(*) FROM devices WHERE id = ?1", params![id], |r| r.get::<_, i64>(0))? as usize;
        }
        Ok(changed > 0)
    }

    /// Records activity, at most once a minute per device to avoid write churn.
    pub fn touch_device(&self, id: &str, now: i64) -> Result<()> {
        self.conn().execute(
            "UPDATE devices SET last_seen_at = ?2 WHERE id = ?1 AND (last_seen_at IS NULL OR last_seen_at < ?2 - 60000)",
            params![id, now],
        )?;
        Ok(())
    }

    /// Deletes the device and (via ON DELETE CASCADE) its messages.
    pub fn delete_device(&self, id: &str) -> Result<bool> {
        Ok(self.conn().execute("DELETE FROM devices WHERE id = ?1", params![id])? > 0)
    }

    // ---- messages ----

    /// Inserts a message; a retry with the same `client_id` returns the
    /// original row instead of creating a duplicate. The bool is true when
    /// a new row was created.
    pub fn insert_message(&self, m: &NewMessage) -> Result<(MessageRow, bool)> {
        let conn = self.conn();
        let inserted = conn.execute(
            "INSERT OR IGNORE INTO messages (device_id, from_host, client_id, kind, text, file_path, file_name, file_size, mime, created_at, file_root)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                m.device_id,
                m.from_host as i64,
                m.client_id,
                m.kind,
                m.text,
                m.file_path,
                m.file_name,
                m.file_size,
                m.mime,
                m.created_at,
                m.file_root
            ],
        )?;
        let row = if inserted > 0 {
            let id = conn.last_insert_rowid();
            conn.query_row(&format!("SELECT {MESSAGE_COLS} FROM messages WHERE id = ?1"), params![id], message_from)?
        } else {
            conn.query_row(
                &format!("SELECT {MESSAGE_COLS} FROM messages WHERE device_id = ?1 AND from_host = ?2 AND client_id = ?3"),
                params![m.device_id, m.from_host as i64, m.client_id],
                message_from,
            )?
        };
        Ok((row, inserted > 0))
    }

    pub fn message(&self, id: i64) -> Result<Option<MessageRow>> {
        Ok(self
            .conn()
            .query_row(&format!("SELECT {MESSAGE_COLS} FROM messages WHERE id = ?1"), params![id], message_from)
            .optional()?)
    }

    /// Finds a message by the sender's client id (used to make file uploads idempotent).
    pub fn message_by_client_id(&self, device_id: &str, from_host: bool, client_id: &str) -> Result<Option<MessageRow>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {MESSAGE_COLS} FROM messages WHERE device_id = ?1 AND from_host = ?2 AND client_id = ?3"),
                params![device_id, from_host as i64, client_id],
                message_from,
            )
            .optional()?)
    }

    /// Pages through a conversation, returned oldest-first.
    /// `after` reads forward (catch-up); otherwise reads back from `before` (or the newest).
    pub fn messages(&self, device_id: &str, before: Option<i64>, after: Option<i64>, limit: usize) -> Result<(Vec<MessageRow>, bool)> {
        let conn = self.conn();
        let take = (limit + 1) as i64;
        let mut rows = if let Some(after) = after {
            let mut stmt = conn.prepare(&format!(
                "SELECT {MESSAGE_COLS} FROM messages WHERE device_id = ?1 AND id > ?2 ORDER BY id ASC LIMIT ?3"
            ))?;
            stmt.query_map(params![device_id, after, take], message_from)?.collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            let before = before.unwrap_or(i64::MAX);
            let mut stmt = conn.prepare(&format!(
                "SELECT {MESSAGE_COLS} FROM messages WHERE device_id = ?1 AND id < ?2 ORDER BY id DESC LIMIT ?3"
            ))?;
            let mut v = stmt.query_map(params![device_id, before, take], message_from)?.collect::<rusqlite::Result<Vec<_>>>()?;
            v.reverse();
            v
        };
        let has_more = rows.len() > limit;
        if has_more {
            if after.is_some() {
                rows.truncate(limit);
            } else {
                rows.remove(0);
            }
        }
        Ok((rows, has_more))
    }

    pub fn last_message(&self, device_id: &str) -> Result<Option<MessageRow>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {MESSAGE_COLS} FROM messages WHERE device_id = ?1 ORDER BY id DESC LIMIT 1"),
                params![device_id],
                message_from,
            )
            .optional()?)
    }

    pub fn clear_messages(&self) -> Result<usize> {
        Ok(self.conn().execute("DELETE FROM messages", [])?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(db: &Db, id: &str) {
        db.insert_device(&DeviceRow {
            id: id.into(),
            name: "iPhone".into(),
            token_hash: "h".into(),
            can_write: true,
            created_at: 1,
            last_seen_at: None,
        })
        .unwrap();
    }

    fn text<'a>(device_id: &'a str, client_id: Option<&'a str>, body: &'a str) -> NewMessage<'a> {
        NewMessage {
            device_id,
            from_host: false,
            client_id,
            kind: "text",
            text: Some(body),
            file_path: None,
            file_name: None,
            file_size: None,
            mime: None,
            created_at: 10,
            file_root: None,
        }
    }

    #[test]
    fn dedupes_by_client_id() {
        let db = Db::open_in_memory().unwrap();
        device(&db, "d1");
        let (a, new_a) = db.insert_message(&text("d1", Some("c1"), "hi")).unwrap();
        let (b, new_b) = db.insert_message(&text("d1", Some("c1"), "hi")).unwrap();
        assert!(new_a && !new_b);
        assert_eq!(a.id, b.id);
        let (c, new_c) = db.insert_message(&text("d1", None, "x")).unwrap();
        let (d, new_d) = db.insert_message(&text("d1", None, "x")).unwrap();
        assert!(new_c && new_d && c.id != d.id);
    }

    #[test]
    fn pages_and_cascades() {
        let db = Db::open_in_memory().unwrap();
        device(&db, "d1");
        for i in 0..5 {
            let cid = format!("c{i}");
            db.insert_message(&text("d1", Some(&cid), "m")).unwrap();
        }
        let (latest, more) = db.messages("d1", None, None, 3).unwrap();
        assert_eq!(latest.iter().map(|m| m.id).collect::<Vec<_>>(), vec![3, 4, 5]);
        assert!(more);
        let (older, more) = db.messages("d1", Some(3), None, 3).unwrap();
        assert_eq!(older.iter().map(|m| m.id).collect::<Vec<_>>(), vec![1, 2]);
        assert!(!more);
        let (newer, more) = db.messages("d1", None, Some(2), 2).unwrap();
        assert_eq!(newer.iter().map(|m| m.id).collect::<Vec<_>>(), vec![3, 4]);
        assert!(more);
        assert!(db.delete_device("d1").unwrap());
        assert!(db.message(1).unwrap().is_none());
    }

    #[test]
    fn updates_device() {
        let db = Db::open_in_memory().unwrap();
        device(&db, "d1");
        assert!(db.update_device("d1", Some("新名字"), Some(false)).unwrap());
        let d = db.device("d1").unwrap().unwrap();
        assert_eq!(d.name, "新名字");
        assert!(!d.can_write);
        assert!(!db.update_device("nope", Some("x"), None).unwrap());
    }
}
