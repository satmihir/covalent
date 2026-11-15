use reagent::session_api::Source;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Serialize;
use std::path::Path;

/// Chat is one conversation as the sidebar lists it; its id is the re:agent Session ID.
#[derive(Clone, Debug, Serialize)]
pub struct Chat {
    pub id: String,
    pub title: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    /// model is the Session's model as last opened or switched here; unknown for older chats.
    pub model: Option<String>,
    /// archived_at_ms hides the chat from Covalent; its Session stays resumable from the CLI.
    pub archived_at_ms: Option<i64>,
}

/// Message is one turn as the thread shows it, independent of the model's context.
#[derive(Clone, Debug, Serialize)]
pub struct Message {
    pub id: String,
    pub chat_id: String,
    /// role is `user` or `assistant`.
    pub role: String,
    pub text: String,
    /// status is `pending`, `complete`, `failed`, `cancelled` or `interrupted`.
    pub status: String,
    pub reason: Option<String>,
    pub created_at_ms: i64,
    /// sources lists the pages a reply cites from web search, in first-cited order.
    pub sources: Vec<Source>,
}

/// Store keeps chats and their displayed messages in one SQLite file.
pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open creates the schema if needed and marks replies left pending by an earlier process.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS chats (
                id TEXT PRIMARY KEY,
                title TEXT,
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                chat_id TEXT NOT NULL REFERENCES chats(id),
                role TEXT NOT NULL,
                text TEXT NOT NULL,
                status TEXT NOT NULL,
                reason TEXT,
                created_at_ms INTEGER NOT NULL
            );",
        )?;
        // Columns added after the first release; SQLite has no ADD COLUMN IF NOT EXISTS.
        for (table, column, kind) in [
            ("chats", "model", "TEXT"),
            ("chats", "archived_at_ms", "INTEGER"),
            ("messages", "sources", "TEXT"),
        ] {
            let columns: Vec<String> = conn
                .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))?
                .query_map([], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            if !columns.iter().any(|c| c == column) {
                conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {kind}"))?;
            }
        }
        // re:agent never restarts work that was in flight, so neither does the display.
        conn.execute(
            "UPDATE messages SET status = 'interrupted',
                reason = 'The server stopped before this reply finished.'
             WHERE status = 'pending'",
            [],
        )?;
        Ok(Self { conn })
    }

    /// Chats lists every chat not archived, most recently active first.
    pub fn chats(&self) -> rusqlite::Result<Vec<Chat>> {
        let mut statement = self.conn.prepare(
            "SELECT * FROM chats WHERE archived_at_ms IS NULL
             ORDER BY updated_at_ms DESC, rowid DESC",
        )?;
        let rows = statement.query_map([], chat)?;
        rows.collect()
    }

    /// Chat returns one chat, if it exists.
    pub fn chat(&self, id: &str) -> rusqlite::Result<Option<Chat>> {
        self.conn
            .query_row("SELECT * FROM chats WHERE id = ?1", [id], chat)
            .optional()
    }

    /// Create_chat records a new untitled chat.
    pub fn create_chat(&self, id: &str, model: &str, now_ms: i64) -> rusqlite::Result<Chat> {
        self.conn.execute(
            "INSERT INTO chats (id, model, created_at_ms, updated_at_ms) VALUES (?1, ?2, ?3, ?3)",
            params![id, model, now_ms],
        )?;
        Ok(Chat {
            id: id.into(),
            title: None,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            model: Some(model.into()),
            archived_at_ms: None,
        })
    }

    /// Set_model records the model a chat's Session switched to.
    pub fn set_model(&self, chat_id: &str, model: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE chats SET model = ?2 WHERE id = ?1",
            params![chat_id, model],
        )?;
        Ok(())
    }

    /// Archive hides a chat from the list without touching its messages or its Session.
    pub fn archive(&self, chat_id: &str, now_ms: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE chats SET archived_at_ms = ?2 WHERE id = ?1",
            params![chat_id, now_ms],
        )?;
        Ok(())
    }

    /// Set_title replaces a chat's title.
    pub fn set_title(&self, chat_id: &str, title: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE chats SET title = ?2 WHERE id = ?1",
            params![chat_id, title],
        )?;
        Ok(())
    }

    /// Messages lists a chat's messages in the order they were added.
    pub fn messages(&self, chat_id: &str) -> rusqlite::Result<Vec<Message>> {
        let mut statement = self
            .conn
            .prepare("SELECT * FROM messages WHERE chat_id = ?1 ORDER BY rowid")?;
        let rows = statement.query_map([chat_id], message)?;
        rows.collect()
    }

    /// Message returns one message by ID.
    pub fn message(&self, id: &str) -> rusqlite::Result<Option<Message>> {
        self.conn
            .query_row("SELECT * FROM messages WHERE id = ?1", [id], message)
            .optional()
    }

    /// Has_message reports whether a message ID is already used, which makes sends retryable.
    pub fn has_message(&self, id: &str) -> rusqlite::Result<bool> {
        self.conn
            .query_row("SELECT 1 FROM messages WHERE id = ?1", [id], |_| Ok(()))
            .optional()
            .map(|found| found.is_some())
    }

    /// Insert_message appends a message and marks its chat as active.
    pub fn insert_message(&self, m: &Message) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO messages (id, chat_id, role, text, status, reason, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                m.id,
                m.chat_id,
                m.role,
                m.text,
                m.status,
                m.reason,
                m.created_at_ms
            ],
        )?;
        self.conn.execute(
            "UPDATE chats SET updated_at_ms = ?2 WHERE id = ?1",
            params![m.chat_id, m.created_at_ms],
        )?;
        Ok(())
    }

    /// Finish_message records a reply's final text and status and returns the updated message.
    pub fn finish_message(
        &self,
        id: &str,
        text: &str,
        status: &str,
        reason: Option<&str>,
        sources: &[Source],
    ) -> rusqlite::Result<Message> {
        let sources = (!sources.is_empty()).then(|| serde_json::to_string(sources).unwrap());
        self.conn.execute(
            "UPDATE messages SET text = ?2, status = ?3, reason = ?4, sources = ?5 WHERE id = ?1",
            params![id, text, status, reason, sources],
        )?;
        self.conn
            .query_row("SELECT * FROM messages WHERE id = ?1", [id], message)
    }
}

fn chat(row: &Row) -> rusqlite::Result<Chat> {
    Ok(Chat {
        id: row.get("id")?,
        title: row.get("title")?,
        created_at_ms: row.get("created_at_ms")?,
        updated_at_ms: row.get("updated_at_ms")?,
        model: row.get("model")?,
        archived_at_ms: row.get("archived_at_ms")?,
    })
}

fn message(row: &Row) -> rusqlite::Result<Message> {
    Ok(Message {
        id: row.get("id")?,
        chat_id: row.get("chat_id")?,
        role: row.get("role")?,
        text: row.get("text")?,
        status: row.get("status")?,
        reason: row.get("reason")?,
        created_at_ms: row.get("created_at_ms")?,
        sources: row
            .get::<_, Option<String>>("sources")?
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(id: &str, status: &str) -> Message {
        Message {
            id: id.into(),
            chat_id: "c".into(),
            role: "assistant".into(),
            text: String::new(),
            status: status.into(),
            reason: None,
            created_at_ms: 1,
            sources: vec![],
        }
    }

    #[test]
    fn pending_replies_become_interrupted_when_the_store_reopens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("covalent.db");
        {
            let store = Store::open(&path).unwrap();
            store.create_chat("c", "m", 1).unwrap();
            store.insert_message(&reply("pending", "pending")).unwrap();
            store.insert_message(&reply("done", "complete")).unwrap();
        }
        let store = Store::open(&path).unwrap();
        let statuses: Vec<_> = store
            .messages("c")
            .unwrap()
            .into_iter()
            .map(|m| (m.id, m.status))
            .collect();
        assert_eq!(
            statuses,
            [
                ("pending".into(), "interrupted".into()),
                ("done".into(), "complete".into())
            ]
        );
    }

    #[test]
    fn chats_are_listed_by_latest_activity() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("covalent.db")).unwrap();
        store.create_chat("older", "m", 1).unwrap();
        store.create_chat("newer", "m", 2).unwrap();
        let mut m = reply("m", "complete");
        m.chat_id = "older".into();
        m.created_at_ms = 3;
        store.insert_message(&m).unwrap();
        let ids: Vec<_> = store.chats().unwrap().into_iter().map(|c| c.id).collect();
        assert_eq!(ids, ["older", "newer"]);
        store.archive("older", 4).unwrap();
        let ids: Vec<_> = store.chats().unwrap().into_iter().map(|c| c.id).collect();
        assert_eq!(ids, ["newer"]);
        assert_eq!(
            store.chat("older").unwrap().unwrap().archived_at_ms,
            Some(4)
        );
    }

    #[test]
    fn a_store_from_before_models_and_archiving_gains_both_columns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("covalent.db");
        Connection::open(&path)
            .unwrap()
            .execute_batch(
                "CREATE TABLE chats (id TEXT PRIMARY KEY, title TEXT,
                    created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL);
                 INSERT INTO chats VALUES ('old', 'Old chat', 1, 1);",
            )
            .unwrap();
        let store = Store::open(&path).unwrap();
        let old = store.chat("old").unwrap().unwrap();
        assert_eq!((old.model, old.archived_at_ms), (None, None));
        drop(store);
        Store::open(&path).unwrap();
    }
}
