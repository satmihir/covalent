use crate::store::{Chat, Message, Store};
use reagent::session_api::{self, ApiError, PlanMode, RunKind, RunRef, SessionApi, SessionOptions};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

/// Launch holds the settings every chat Session opens with.
#[derive(Clone, Debug, Default)]
pub struct Launch {
    /// workspace is an empty directory: chats have no files to work on yet.
    pub workspace: PathBuf,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub title_model: Option<String>,
    pub title_effort: Option<String>,
    /// scripted replaces inference with a re:agent response script, for offline tests.
    pub scripted: Option<PathBuf>,
}

impl Launch {
    /// Options returns the Session settings for a chat, or for a title request.
    pub fn options(&self, title: bool) -> SessionOptions {
        SessionOptions {
            workspace: self.workspace.clone(),
            model: if title {
                self.title_model.clone()
            } else {
                self.model.clone()
            },
            reasoning_effort: if title {
                self.title_effort.clone()
            } else {
                self.effort.clone()
            },
            scripted: self.scripted.clone(),
            // Until re:agent has tool-free conversations, read-only on an empty directory
            // leaves the model nothing it can change or read.
            read_only: true,
            agents: false,
            checkpoint: !title,
            origin: Some("covalent".into()),
            ..SessionOptions::default()
        }
    }
}

/// Event is one change the browser hears about over server-sent events.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// Message carries a reply that finished, failed or was cancelled.
    Message { message: Message },
    /// Title carries a chat's new title.
    Title { chat_id: String, title: String },
}

/// Failure is a request error with the HTTP status it should produce.
#[derive(Debug)]
pub struct Failure {
    pub status: u16,
    pub code: String,
    pub message: String,
}

impl Failure {
    fn new(status: u16, code: &str, message: impl Into<String>) -> Self {
        Self {
            status,
            code: code.into(),
            message: message.into(),
        }
    }
}

impl From<ApiError> for Failure {
    fn from(error: ApiError) -> Self {
        let status = match error.code {
            "invalid_argument" => 400,
            "not_found" => 404,
            "busy" | "blocked" | "conflict" => 409,
            "resource_exhausted" => 429,
            "unavailable" => 503,
            _ => 500,
        };
        Self::new(status, error.code, error.message)
    }
}

impl From<rusqlite::Error> for Failure {
    fn from(error: rusqlite::Error) -> Self {
        Self::new(500, "store", format!("chat store: {error}"))
    }
}

/// Chats owns the store and one live Session handle per chat in use.
pub struct Chats {
    store: Mutex<Store>,
    live: Mutex<HashMap<String, SessionApi>>,
    events: broadcast::Sender<Event>,
    launch: Launch,
}

impl Chats {
    /// New serves chats from an open store; Sessions are opened or resumed on first use.
    pub fn new(store: Store, launch: Launch) -> Arc<Self> {
        Arc::new(Self {
            store: Mutex::new(store),
            live: Mutex::new(HashMap::new()),
            events: broadcast::channel(256).0,
            launch,
        })
    }

    /// Subscribe returns a receiver for every later Event.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    /// List returns all chats, most recently active first.
    pub fn list(&self) -> Result<Vec<Chat>, Failure> {
        Ok(self.store.lock().unwrap().chats()?)
    }

    /// Get returns a chat and its messages.
    pub fn get(&self, chat_id: &str) -> Result<(Chat, Vec<Message>), Failure> {
        let store = self.store.lock().unwrap();
        let chat = store
            .chat(chat_id)?
            .ok_or_else(|| Failure::new(404, "not_found", "no such chat"))?;
        Ok((chat, store.messages(chat_id)?))
    }

    /// Create opens a new Session and records it as an untitled chat.
    pub fn create(&self) -> Result<Chat, Failure> {
        let api = session_api::open_session(self.launch.options(false))?;
        let id = api.get_snapshot()?.session.session_id;
        let chat = self.store.lock().unwrap().create_chat(&id, now_ms())?;
        self.live.lock().unwrap().insert(id, api);
        Ok(chat)
    }

    /// Send admits a user message and returns the chat's messages, ending with the pending reply.
    pub fn send(
        self: &Arc<Self>,
        chat_id: &str,
        request_id: &str,
        text: &str,
    ) -> Result<Vec<Message>, Failure> {
        if text.trim().is_empty() {
            return Err(Failure::new(400, "invalid_argument", "message is empty"));
        }
        let (_, messages) = self.get(chat_id)?;
        // A retry after a restart must not reach re:agent, whose deduplication is per opening.
        if self.store.lock().unwrap().has_message(request_id)? {
            return Ok(messages);
        }
        let first = messages.is_empty();
        let api = self.session(chat_id)?;
        let run = match api.send_message(request_id, text.as_bytes(), PlanMode::Keep) {
            Err(error) if error.code == "resource_exhausted" => {
                // One opening keeps every receipt; a fresh opening of the same Session starts empty.
                api.close()?;
                self.live.lock().unwrap().remove(chat_id);
                self.session(chat_id)?
                    .send_message(request_id, text.as_bytes(), PlanMode::Keep)?
            }
            other => other?,
        }
        .run
        .ok_or_else(|| Failure::new(500, "internal", "message admitted without a Run"))?;
        let now = now_ms();
        let user = Message {
            id: request_id.into(),
            chat_id: chat_id.into(),
            role: "user".into(),
            text: text.into(),
            status: "complete".into(),
            reason: None,
            created_at_ms: now,
        };
        let reply = Message {
            id: run.run_id.clone(),
            role: "assistant".into(),
            text: String::new(),
            status: "pending".into(),
            ..user.clone()
        };
        {
            let store = self.store.lock().unwrap();
            store.insert_message(&user)?;
            store.insert_message(&reply)?;
        }
        let chats = self.clone();
        let api = self.session(chat_id)?;
        std::thread::spawn(move || chats.await_reply(&api, &run));
        if first {
            let chats = self.clone();
            let (chat_id, text) = (chat_id.to_owned(), text.to_owned());
            std::thread::spawn(move || chats.write_title(&chat_id, &text));
        }
        Ok(self.get(chat_id)?.1)
    }

    /// Cancel asks every unfinished conversation Run in a chat to stop.
    pub fn cancel(&self, chat_id: &str) -> Result<(), Failure> {
        let Some(api) = self.live.lock().unwrap().get(chat_id).cloned() else {
            return Ok(());
        };
        for run in api.get_snapshot()?.live_runs {
            if run.kind == RunKind::Conversation {
                api.cancel_run(&run.run)?;
            }
        }
        Ok(())
    }

    fn session(&self, chat_id: &str) -> Result<SessionApi, Failure> {
        let mut live = self.live.lock().unwrap();
        if let Some(api) = live.get(chat_id) {
            return Ok(api.clone());
        }
        let api = session_api::resume_session(chat_id)?;
        live.insert(chat_id.into(), api.clone());
        Ok(api)
    }

    fn await_reply(&self, api: &SessionApi, run: &RunRef) {
        let (text, status, reason) = match api.wait_run(run).map(|info| info.result) {
            Ok(Some(result)) => {
                let text = String::from_utf8_lossy(&result.reply).into_owned();
                match result.status.as_str() {
                    "completed" | "refused" => (text, "complete", None),
                    "cancelled" => (text, "cancelled", None),
                    status => {
                        let reason = if result.reason.is_empty() {
                            status.to_owned()
                        } else {
                            result.reason
                        };
                        (text, "failed", Some(reason))
                    }
                }
            }
            Ok(None) => (
                String::new(),
                "failed",
                Some("the Run ended without a result".into()),
            ),
            Err(error) => (String::new(), "failed", Some(error.message)),
        };
        let store = self.store.lock().unwrap();
        match store.finish_message(&run.run_id, &text, status, reason.as_deref()) {
            Ok(message) => {
                let _ = self.events.send(Event::Message { message });
            }
            Err(error) => eprintln!("covalent: record reply {}: {error}", run.run_id),
        }
    }

    fn write_title(&self, chat_id: &str, text: &str) {
        let title = crate::title::generate(&self.launch, text);
        if let Err(error) = self.store.lock().unwrap().set_title(chat_id, &title) {
            eprintln!("covalent: record title for {chat_id}: {error}");
            return;
        }
        let _ = self.events.send(Event::Title {
            chat_id: chat_id.into(),
            title,
        });
    }
}

/// Now_ms is the current Unix time in milliseconds.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}
