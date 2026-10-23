//! SQLite storage for paired devices and chat messages.

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, Row, params};
use std::path::Path;
use std::sync::Mutex;

use super::util::now_ms;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS devices (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  token_hash TEXT NOT NULL,
  can_write INTEGER NOT NULL DEFAULT 1,
  created_at INTEGER NOT NULL,
  last_seen_at INTEGER,
  -- The computer has read this device's messages up to this id (unread counts).
  read_through INTEGER NOT NULL DEFAULT 0
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
  file_root TEXT,
  file_created INTEGER
);
CREATE INDEX IF NOT EXISTS messages_by_device ON messages(device_id, id);
CREATE UNIQUE INDEX IF NOT EXISTS messages_client_id
  ON messages(device_id, from_host, client_id) WHERE client_id IS NOT NULL;
-- Client ids of deleted messages (no content): a retry that arrives later,
-- such as a phone's queued text, is refused instead of bringing it back.
CREATE TABLE IF NOT EXISTS deleted_messages (
  device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
  from_host INTEGER NOT NULL,
  client_id TEXT NOT NULL,
  deleted_at INTEGER NOT NULL,
  PRIMARY KEY (device_id, from_host, client_id)
);
-- Ids of single deleted messages (no content): event streams that open
-- later, from a phone that was away, start with them, also after a restart.
CREATE TABLE IF NOT EXISTS deleted_ids (
  message_id INTEGER PRIMARY KEY,
  device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
  deleted_at INTEGER NOT NULL
);
-- Shared folders that chat files were saved in. Their 收件箱 and 已发送 stay
-- the computer's alone for good, also after the messages are deleted and
-- another folder is shared (see AppState::is_chat_location).
CREATE TABLE IF NOT EXISTS chat_roots (
  root TEXT PRIMARY KEY,
  added_at INTEGER NOT NULL
);
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
    /// When the attachment's file was created (ms): tells it apart from a
    /// later file that took the same name, while edits to it keep it.
    pub file_created: Option<i64>,
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
    pub file_created: Option<i64>,
}

pub struct Db {
    conn: Mutex<Connection>,
}

const DEVICE_COLS: &str = "id, name, token_hash, can_write, created_at, last_seen_at";
const MESSAGE_COLS: &str = "id, device_id, from_host, client_id, kind, text, file_path, file_name, file_size, mime, created_at, file_root, file_created";

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
        file_created: row.get(12)?,
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
        // secure_delete overwrites deleted rows with zeros, so cleared chats
        // cannot be read back out of the database file.
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000; PRAGMA secure_delete=ON;")?;
        conn.execute_batch(SCHEMA)?;
        // Databases created before attachments remembered their root and their file.
        for (column, kind) in [("file_root", "TEXT"), ("file_created", "INTEGER")] {
            let has: i64 = conn.query_row("SELECT COUNT(*) FROM pragma_table_info('messages') WHERE name = ?1", params![column], |r| r.get(0))?;
            if has == 0 {
                conn.execute_batch(&format!("ALTER TABLE messages ADD COLUMN {column} {kind};"))?;
            }
        }
        let has_read_through: i64 = conn.query_row("SELECT COUNT(*) FROM pragma_table_info('devices') WHERE name = 'read_through'", [], |r| r.get(0))?;
        if has_read_through == 0 {
            // What was there before counts as read: an upgrade starts with no unread messages.
            conn.execute_batch(
                "ALTER TABLE devices ADD COLUMN read_through INTEGER NOT NULL DEFAULT 0;
                 UPDATE devices SET read_through = (SELECT COALESCE(MAX(id), 0) FROM messages);",
            )?;
        }
        // Upgrades (and any start): the folders that attachments remember held chat files.
        conn.execute(
            "INSERT OR IGNORE INTO chat_roots (root, added_at) SELECT DISTINCT file_root, ?1 FROM messages WHERE file_root IS NOT NULL",
            params![now_ms()],
        )?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version < 1 {
            // Builds before secure_delete left deleted text in free pages:
            // rewrite the file once.
            conn.execute_batch("VACUUM;")?;
            Self::purge(&conn);
            conn.execute_batch("PRAGMA user_version = 1;")?;
        }
        Ok(Db { conn: Mutex::new(conn) })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Runs after every delete. In WAL mode older copies of a page stay in the
    /// -wal file until a checkpoint; TRUNCATE copies the zeroed pages into the
    /// database and empties the -wal file, so deleted text is gone from both.
    /// Best effort: the delete itself has already been committed, and the
    /// next purge (or SQLite's own checkpoint) finishes the job.
    fn purge(conn: &Connection) {
        if let Err(e) = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(())) {
            tracing::warn!("could not compact the database after a delete: {e}");
        }
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
        let conn = self.conn();
        let deleted = conn.execute("DELETE FROM devices WHERE id = ?1", params![id])? > 0;
        if deleted {
            Self::purge(&conn);
        }
        Ok(deleted)
    }

    // ---- messages ----

    /// Inserts a message; a retry with the same `client_id` returns the
    /// original row instead of creating a duplicate. The bool is true when
    /// a new row was created.
    pub fn insert_message(&self, m: &NewMessage) -> Result<(MessageRow, bool)> {
        let conn = self.conn();
        let inserted = conn.execute(
            "INSERT OR IGNORE INTO messages (device_id, from_host, client_id, kind, text, file_path, file_name, file_size, mime, created_at, file_root, file_created)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
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
                m.file_root,
                m.file_created
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

    /// Marks a device's messages so far as read on the computer.
    pub fn mark_read(&self, device_id: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE devices SET read_through = MAX(read_through, (SELECT COALESCE(MAX(id), 0) FROM messages WHERE device_id = ?1)) WHERE id = ?1",
            params![device_id],
        )?;
        Ok(())
    }

    /// Unread messages from each device: those after the last one read.
    pub fn unread_counts(&self) -> Result<Vec<(String, u32)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT d.id, (SELECT COUNT(*) FROM messages m WHERE m.device_id = d.id AND m.from_host = 0 AND m.id > d.read_through) FROM devices d",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Devices (id, name), the one with the most recent message first, then
    /// the newest pairing (the order of the host's conversation list).
    pub fn devices_by_recent(&self) -> Result<Vec<(String, String)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT d.id, d.name FROM devices d LEFT JOIN messages m ON m.device_id = d.id
             GROUP BY d.id ORDER BY MAX(m.id) IS NULL, MAX(m.id) DESC, d.created_at DESC",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// The newest text a phone sent to the computer.
    pub fn latest_received_text(&self) -> Result<Option<MessageRow>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {MESSAGE_COLS} FROM messages WHERE from_host = 0 AND kind = 'text' ORDER BY id DESC LIMIT 1"),
                [],
                message_from,
            )
            .optional()?)
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

    /// Deletes the messages `filter` selects, remembering their client ids
    /// (see `deleted_messages`) and, with `with_ids`, their ids (`deleted_ids`).
    fn delete_where(&self, filter: &str, p: &[&dyn rusqlite::ToSql], with_ids: bool) -> Result<usize> {
        let now = now_ms();
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            &format!(
                "INSERT OR IGNORE INTO deleted_messages (device_id, from_host, client_id, deleted_at)
                 SELECT device_id, from_host, client_id, {now} FROM messages WHERE client_id IS NOT NULL AND ({filter})"
            ),
            p,
        )?;
        if with_ids {
            tx.execute(
                &format!("INSERT OR IGNORE INTO deleted_ids (message_id, device_id, deleted_at) SELECT id, device_id, {now} FROM messages WHERE {filter}"),
                p,
            )?;
        }
        let n = tx.execute(&format!("DELETE FROM messages WHERE {filter}"), p)?;
        tx.commit()?;
        if n > 0 {
            Self::purge(&conn);
        }
        Ok(n)
    }

    /// Deletes one message (its attachment file is left alone).
    pub fn delete_message(&self, id: i64) -> Result<bool> {
        Ok(self.delete_where("id = ?1", params![id], true)? > 0)
    }

    /// Deletes every message so far; returns how many, and the highest id
    /// among them. A clear is passed on as that id (`Config::cleared_through`),
    /// which also covers the single deletes before it. A message that arrives
    /// meanwhile has a higher id and stays.
    pub fn clear_messages(&self) -> Result<(usize, i64)> {
        let through: i64 = self.conn().query_row("SELECT COALESCE(MAX(id), 0) FROM messages", [], |r| r.get(0))?;
        let n = self.delete_where("id <= ?1", params![through], false)?;
        self.conn().execute("DELETE FROM deleted_ids WHERE message_id <= ?1", params![through])?;
        Ok((n, through))
    }

    /// Deletes messages created before `before` (ms); used by automatic cleanup.
    pub fn delete_messages_before(&self, before: i64) -> Result<usize> {
        self.delete_where("created_at < ?1", params![before], false)
    }

    /// The most recent `limit` single deletes, oldest first: all of them, or one device's.
    pub fn deleted_ids(&self, device_id: Option<&str>, limit: usize) -> Result<Vec<i64>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT message_id FROM (SELECT message_id, deleted_at FROM deleted_ids WHERE ?1 IS NULL OR device_id = ?1
             ORDER BY deleted_at DESC, message_id DESC LIMIT ?2) ORDER BY deleted_at, message_id",
        )?;
        let rows = stmt.query_map(params![device_id, limit as i64], |r| r.get(0))?.collect::<rusqlite::Result<Vec<i64>>>()?;
        Ok(rows)
    }

    /// Whether a message with this client id was deleted.
    pub fn was_deleted(&self, device_id: &str, from_host: bool, client_id: &str) -> Result<bool> {
        Ok(self.conn().query_row(
            "SELECT EXISTS (SELECT 1 FROM deleted_messages WHERE device_id = ?1 AND from_host = ?2 AND client_id = ?3)",
            params![device_id, from_host as i64, client_id],
            |r| r.get(0),
        )?)
    }

    /// Forgets deletions older than `before` (ms): no retry or returning phone comes that late.
    pub fn forget_deleted_before(&self, before: i64) -> Result<usize> {
        let conn = self.conn();
        let n = conn.execute("DELETE FROM deleted_messages WHERE deleted_at < ?1", params![before])?;
        Ok(n + conn.execute("DELETE FROM deleted_ids WHERE deleted_at < ?1", params![before])?)
    }

    /// Records that chat files are saved under `root`; true the first time.
    pub fn remember_chat_root(&self, root: &str) -> Result<bool> {
        Ok(self.conn().execute("INSERT OR IGNORE INTO chat_roots (root, added_at) VALUES (?1, ?2)", params![root, now_ms()])? > 0)
    }

    /// Attachments saved outside the given chat folders, as (root, path):
    /// earlier versions followed a chat folder that was a link.
    pub fn attachments_outside(&self, chat_dirs: &[&str]) -> Result<Vec<(String, String)>> {
        let conn = self.conn();
        let mut sql = "SELECT DISTINCT file_root, file_path FROM messages WHERE kind = 'file' AND file_root IS NOT NULL AND file_path IS NOT NULL".to_string();
        for i in 1..=chat_dirs.len() {
            sql.push_str(&format!(" AND file_path NOT LIKE ?{i}"));
        }
        let prefixes: Vec<String> = chat_dirs.iter().map(|d| format!("{d}/%")).collect();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(prefixes.iter()), |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<(String, String)>>>()?;
        Ok(rows)
    }

    /// Shared folders chat files were ever saved under, and those the
    /// remaining attachments name.
    pub fn chat_roots(&self) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT root FROM chat_roots UNION SELECT file_root FROM messages WHERE file_root IS NOT NULL")?;
        let rows = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(rows)
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
            file_created: None,
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

    /// Deleted text must be in neither the database file nor its -wal file,
    /// whether it had already been checkpointed or still sat in the WAL.
    #[test]
    fn deleted_text_is_gone_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lanshare.db");
        let on_disk = |marker: &str| {
            let mut bytes = std::fs::read(&path).unwrap();
            bytes.extend(std::fs::read(dir.path().join("lanshare.db-wal")).unwrap_or_default());
            bytes.windows(marker.len()).any(|w| w == marker.as_bytes())
        };
        let db = Db::open(&path).unwrap();
        device(&db, "d1");
        device(&db, "d2");

        let checkpointed = "MARKER-IN-MAIN-FILE";
        db.insert_message(&text("d1", None, checkpointed)).unwrap();
        Db::purge(&db.conn());
        let in_wal = "MARKER-ONLY-IN-WAL";
        db.insert_message(&text("d1", None, in_wal)).unwrap();
        assert!(on_disk(checkpointed) && on_disk(in_wal), "markers should be on disk before the delete");
        db.clear_messages().unwrap();
        assert!(!on_disk(checkpointed) && !on_disk(in_wal));

        let removed_device = "MARKER-REMOVED-DEVICE";
        db.insert_message(&text("d2", None, removed_device)).unwrap();
        assert!(db.delete_device("d2").unwrap());
        assert!(!on_disk(removed_device));

        let (one, _) = db.insert_message(&text("d1", None, "MARKER-ONE-MESSAGE")).unwrap();
        db.insert_message(&text("d1", None, "neighbour")).unwrap();
        assert!(db.delete_message(one.id).unwrap());
        assert!(!db.delete_message(one.id).unwrap(), "already gone");
        assert!(!on_disk("MARKER-ONE-MESSAGE") && on_disk("neighbour"));

        let mut old = text("d1", None, "MARKER-OLD-MESSAGE");
        old.created_at = 5;
        db.insert_message(&old).unwrap();
        db.insert_message(&text("d1", None, "kept")).unwrap();
        assert_eq!(db.delete_messages_before(10).unwrap(), 1);
        assert!(!on_disk("MARKER-OLD-MESSAGE") && on_disk("kept"));
    }

    /// A database from a build before secure_delete gets scrubbed once.
    #[test]
    fn upgrade_scrubs_text_deleted_by_older_builds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lanshare.db");
        let marker = "MARKER-DELETED-BEFORE-UPGRADE";
        let on_disk = || std::fs::read(&path).unwrap().windows(marker.len()).any(|w| w == marker.as_bytes());
        {
            let old = Connection::open(&path).unwrap();
            old.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
            old.execute_batch(SCHEMA).unwrap();
            old.execute("INSERT INTO devices (id, name, token_hash, created_at) VALUES ('d1', 'iPhone', 'h', 1)", []).unwrap();
            old.execute("INSERT INTO messages (device_id, from_host, kind, text, created_at) VALUES ('d1', 0, 'text', ?1, 1)", params![marker]).unwrap();
            old.execute("DELETE FROM messages", []).unwrap();
            old.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(())).unwrap();
        }
        assert!(on_disk(), "without secure_delete the text stays in the file");
        let db = Db::open(&path).unwrap();
        assert!(!on_disk());
        assert_eq!(db.conn().query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert!(db.device("d1").unwrap().is_some(), "the data itself is kept");
    }

    #[test]
    fn deleted_client_ids_are_remembered() {
        let db = Db::open_in_memory().unwrap();
        device(&db, "d1");
        let (one, _) = db.insert_message(&text("d1", Some("c1"), "a")).unwrap();
        db.insert_message(&text("d1", Some("c2"), "b")).unwrap();
        db.insert_message(&text("d1", None, "c")).unwrap();
        assert!(!db.was_deleted("d1", false, "c1").unwrap());
        db.delete_message(one.id).unwrap();
        assert!(db.was_deleted("d1", false, "c1").unwrap());
        assert_eq!(db.deleted_ids(Some("d1"), 10).unwrap(), [one.id]);
        assert!(db.deleted_ids(Some("d2"), 10).unwrap().is_empty());
        assert!(!db.was_deleted("d1", true, "c1").unwrap(), "the computer's own ids are kept apart");
        db.clear_messages().unwrap();
        assert!(db.was_deleted("d1", false, "c2").unwrap());
        assert!(db.deleted_ids(None, 10).unwrap().is_empty(), "a clear is passed on as a cutoff");
        assert_eq!(db.forget_deleted_before(i64::MAX).unwrap(), 2);
        // Removing the device forgets them as well.
        db.insert_message(&text("d1", Some("c3"), "d")).unwrap();
        db.clear_messages().unwrap();
        let (four, _) = db.insert_message(&text("d1", Some("c4"), "e")).unwrap();
        db.delete_message(four.id).unwrap();
        db.delete_device("d1").unwrap();
        assert!(!db.was_deleted("d1", false, "c3").unwrap());
        assert!(db.deleted_ids(None, 10).unwrap().is_empty());
    }

    /// A database from before attachments kept their folder and time.
    #[test]
    fn adds_new_columns_to_an_older_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lanshare.db");
        Connection::open(&path)
            .unwrap()
            .execute_batch(
                "CREATE TABLE devices (id TEXT PRIMARY KEY, name TEXT NOT NULL, token_hash TEXT NOT NULL, can_write INTEGER NOT NULL DEFAULT 1, created_at INTEGER NOT NULL, last_seen_at INTEGER);
                 CREATE TABLE messages (id INTEGER PRIMARY KEY AUTOINCREMENT, device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
                   from_host INTEGER NOT NULL, client_id TEXT, kind TEXT NOT NULL, text TEXT, file_path TEXT, file_name TEXT, file_size INTEGER, mime TEXT, created_at INTEGER NOT NULL);
                 INSERT INTO devices (id, name, token_hash, created_at) VALUES ('d0', 'old phone', 'h', 1);
                 INSERT INTO messages (device_id, from_host, kind, text, created_at) VALUES ('d0', 0, 'text', 'before the upgrade', 1);",
            )
            .unwrap();
        let db = Db::open(&path).unwrap();
        assert_eq!(db.unread_counts().unwrap(), [("d0".to_string(), 0)], "an upgrade starts with nothing unread");
        db.insert_message(&text("d0", None, "after the upgrade")).unwrap();
        assert_eq!(db.unread_counts().unwrap(), [("d0".to_string(), 1)]);
        device(&db, "d1");
        let mut m = text("d1", None, "x");
        m.file_root = Some("/shared");
        m.file_created = Some(42);
        let (row, _) = db.insert_message(&m).unwrap();
        assert_eq!((row.file_root.as_deref(), row.file_created), (Some("/shared"), Some(42)));
        db.mark_read("d0").unwrap();
        assert!(db.unread_counts().unwrap().contains(&("d0".to_string(), 0)));
        assert_eq!(db.devices_by_recent().unwrap().iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["d1", "d0"]);
    }

    #[test]
    fn chat_roots_outlive_their_messages() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lanshare.db");
        {
            // A database from before chat roots were recorded: only attachments remember theirs.
            let db = Db::open(&path).unwrap();
            device(&db, "d1");
            let mut m = text("d1", None, "x");
            m.file_root = Some("/old/share");
            db.insert_message(&m).unwrap();
            db.conn().execute_batch("DROP TABLE chat_roots;").unwrap();
        }
        let db = Db::open(&path).unwrap();
        db.clear_messages().unwrap();
        assert_eq!(db.chat_roots().unwrap(), ["/old/share"], "seeded once on upgrade, kept after the messages go");
        assert!(db.remember_chat_root("/new/share").unwrap());
        assert!(!db.remember_chat_root("/new/share").unwrap(), "remembered once");
        assert_eq!(db.chat_roots().unwrap().len(), 2);
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
