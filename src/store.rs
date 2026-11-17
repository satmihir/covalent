use crate::activity::Action;
use crate::files::Attachment;
use crate::personalization::Personalization;
use reagent::session_api::Source;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
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
    /// workspace names the chat's folder under `workspaces/`; chats made before folders have none.
    pub workspace: Option<String>,
}

/// Image is one picture a reply made: a path in the chat's folder, or the file name of a
/// picture made before chats had folders.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Image {
    pub name: String,
    pub width: u32,
    pub height: u32,
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
    /// images lists the pictures a reply made, in the order it made them.
    pub images: Vec<Image>,
    /// activity lists the tool calls a reply made, in the order it made them.
    pub activity: Vec<Action>,
    /// attachments lists the files the user sent with a message.
    pub attachments: Vec<Attachment>,
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
            );
            CREATE TABLE IF NOT EXISTS personalization (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                fields TEXT NOT NULL
            );",
        )?;
        // Columns added after the first release; SQLite has no ADD COLUMN IF NOT EXISTS.
        for (table, column, kind) in [
            ("chats", "model", "TEXT"),
            ("chats", "archived_at_ms", "INTEGER"),
            ("messages", "sources", "TEXT"),
            ("messages", "images", "TEXT"),
            ("chats", "workspace", "TEXT"),
            ("messages", "activity", "TEXT"),
            ("messages", "attachments", "TEXT"),
            ("chats", "personalization", "TEXT"),
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

    /// Create_chat records a new untitled chat and the name of its folder.
    pub fn create_chat(
        &self,
        id: &str,
        model: &str,
        workspace: Option<&str>,
        now_ms: i64,
    ) -> rusqlite::Result<Chat> {
        self.conn.execute(
            "INSERT INTO chats (id, model, workspace, created_at_ms, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?4)",
            params![id, model, workspace, now_ms],
        )?;
        Ok(Chat {
            id: id.into(),
            title: None,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            model: Some(model.into()),
            archived_at_ms: None,
            workspace: workspace.map(str::to_owned),
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

    /// Personalization returns what the user set for all chats, empty until they set it.
    pub fn personalization(&self) -> rusqlite::Result<Personalization> {
        let fields: Option<String> = self
            .conn
            .query_row("SELECT fields FROM personalization", [], |row| row.get(0))
            .optional()?;
        Ok(fields
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default())
    }

    /// Set_personalization replaces what the user set for all chats.
    pub fn set_personalization(&self, p: &Personalization) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO personalization (id, fields) VALUES (1, ?1)",
            [serde_json::to_string(p).unwrap()],
        )?;
        Ok(())
    }

    /// Personalization_sent returns the personalization note a chat's model last received.
    pub fn personalization_sent(&self, chat_id: &str) -> rusqlite::Result<Option<String>> {
        self.conn.query_row(
            "SELECT personalization FROM chats WHERE id = ?1",
            [chat_id],
            |row| row.get(0),
        )
    }

    /// Set_personalization_sent records the personalization note a chat's model now has,
    /// or none once it was told the personalization was cleared.
    pub fn set_personalization_sent(
        &self,
        chat_id: &str,
        note: Option<&str>,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE chats SET personalization = ?2 WHERE id = ?1",
            params![chat_id, note],
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

    /// Has_image reports whether some message records an image by this file name.
    pub fn has_image(&self, name: &str) -> rusqlite::Result<bool> {
        let recorded = serde_json::to_string(name).unwrap();
        self.conn
            .query_row(
                "SELECT 1 FROM messages WHERE instr(images, ?1) > 0 LIMIT 1",
                [format!("\"name\":{recorded}")],
                |_| Ok(()),
            )
            .optional()
            .map(|found| found.is_some())
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
            "INSERT INTO messages (id, chat_id, role, text, status, reason, created_at_ms,
                attachments)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                m.id,
                m.chat_id,
                m.role,
                m.text,
                m.status,
                m.reason,
                m.created_at_ms,
                list(&m.attachments)
            ],
        )?;
        self.conn.execute(
            "UPDATE chats SET updated_at_ms = ?2 WHERE id = ?1",
            params![m.chat_id, m.created_at_ms],
        )?;
        Ok(())
    }

    /// Finish_message records a finished reply, whose ID names the message to update, and
    /// returns the updated message.
    pub fn finish_message(&self, reply: &Message) -> rusqlite::Result<Message> {
        let id = &reply.id;
        self.conn.execute(
            "UPDATE messages SET text = ?2, status = ?3, reason = ?4, sources = ?5, images = ?6,
                activity = ?7
             WHERE id = ?1",
            params![
                id,
                reply.text,
                reply.status,
                reply.reason,
                list(&reply.sources),
                list(&reply.images),
                list(&reply.activity)
            ],
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
        workspace: row.get("workspace")?,
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
        sources: column(row, "sources")?,
        images: column(row, "images")?,
        activity: column(row, "activity")?,
        attachments: column(row, "attachments")?,
    })
}

/// List writes a JSON list column, unset when empty.
fn list<T: Serialize>(items: &[T]) -> Option<String> {
    (!items.is_empty()).then(|| serde_json::to_string(items).unwrap())
}

/// Column reads a JSON list column, empty when unset or unreadable.
fn column<T: serde::de::DeserializeOwned>(row: &Row, column: &str) -> rusqlite::Result<Vec<T>> {
    Ok(row
        .get::<_, Option<String>>(column)?
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default())
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
            images: vec![],
            activity: vec![],
            attachments: vec![],
        }
    }

    #[test]
    fn pending_replies_become_interrupted_when_the_store_reopens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("covalent.db");
        {
            let store = Store::open(&path).unwrap();
            store.create_chat("c", "m", None, 1).unwrap();
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
        store.create_chat("older", "m", None, 1).unwrap();
        store.create_chat("newer", "m", None, 2).unwrap();
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
    fn personalization_and_what_each_chat_was_told_survive_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("covalent.db");
        let set = Personalization {
            nickname: "Sam".into(),
            about: "Bakes on weekends.".into(),
            ..Personalization::default()
        };
        {
            let store = Store::open(&path).unwrap();
            assert_eq!(store.personalization().unwrap(), Personalization::default());
            store.create_chat("c", "m", None, 1).unwrap();
            assert_eq!(store.personalization_sent("c").unwrap(), None);
            store.set_personalization(&set).unwrap();
            store.set_personalization_sent("c", Some("note")).unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.personalization().unwrap(), set);
        assert_eq!(
            store.personalization_sent("c").unwrap().as_deref(),
            Some("note")
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
