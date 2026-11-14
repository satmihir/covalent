use crate::store::{Chat, Message, Store};
use reagent::session_api::{
    self, ApiError, CommandState, HistoryMode, PlanMode, PreviewCursor, PreviewPage, RunKind,
    RunRef, SessionApi, SessionChange, SessionOptions,
};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

/// PROMPT is the instruction every chat Session opens with, in place of re:agent's own.
const PROMPT: &str = include_str!("prompt.txt");

/// TITLE_PROMPT is the whole instruction for a title Session.
const TITLE_PROMPT: &str = "You write titles for chats. Given the first message of a chat, reply \
    with a title of at most six words and nothing else: no quotes, no trailing punctuation.";

/// Launch holds the settings every chat Session opens with.
#[derive(Clone, Debug, Default)]
pub struct Launch {
    pub model: Option<String>,
    pub effort: Option<String>,
    pub title_model: Option<String>,
    pub title_effort: Option<String>,
    /// scripted replaces inference with a re:agent response script, for offline tests.
    pub scripted: Option<PathBuf>,
}

impl Launch {
    /// Options returns the Session settings for a chat, or for a title request.
    /// A model chosen in the picker runs at its own default effort.
    pub fn options(&self, title: bool, chosen: Option<&str>) -> SessionOptions {
        let (model, effort) = match (title, chosen) {
            (true, _) => (self.title_model.clone(), self.title_effort.clone()),
            (false, Some(model)) => (Some(model.to_owned()), None),
            (false, None) => (self.model.clone(), self.effort.clone()),
        };
        SessionOptions {
            model,
            reasoning_effort: effort,
            scripted: self.scripted.clone(),
            tool_free: true,
            instructions: Some(if title { TITLE_PROMPT } else { PROMPT }.into()),
            agents: false,
            checkpoint: !title,
            web_search: !title,
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
    /// Archived removes a chat from every open sidebar.
    Archived { chat_id: String },
    /// Preview carries reply text the model wrote since the previous preview of that message.
    Preview {
        chat_id: String,
        message_id: String,
        /// seq numbers a message's previews from one; a client that misses one reloads.
        seq: u64,
        /// restarted means the model started its reply over: text replaces what was shown.
        restarted: bool,
        text: String,
        /// searching is true while the provider runs a web search for this reply.
        searching: bool,
    },
}

/// Model is one catalog model the picker offers.
#[derive(Clone, Debug, Serialize)]
pub struct Model {
    pub id: String,
    pub provider: String,
    pub note: String,
}

/// Preview is a pending reply's text so far, as numbered by its latest preview event.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Preview {
    pub seq: u64,
    pub text: String,
    pub searching: bool,
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
    /// previews holds each pending reply's chat ID and text so far, keyed by message ID.
    previews: Mutex<HashMap<String, (String, Preview)>>,
    events: broadcast::Sender<Event>,
    launch: Launch,
}

impl Chats {
    /// New serves chats from an open store; Sessions are opened or resumed on first use.
    pub fn new(store: Store, launch: Launch) -> Arc<Self> {
        Arc::new(Self {
            store: Mutex::new(store),
            live: Mutex::new(HashMap::new()),
            previews: Mutex::new(HashMap::new()),
            // Previews arrive many times a second; a reader that still falls behind reloads.
            events: broadcast::channel(1024).0,
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

    /// Previews returns the text so far of a chat's pending replies, keyed by message ID.
    pub fn previews(&self, chat_id: &str) -> HashMap<String, Preview> {
        let previews = self.previews.lock().unwrap();
        previews
            .iter()
            .filter(|(_, (chat, _))| chat == chat_id)
            .map(|(id, (_, preview))| (id.clone(), preview.clone()))
            .collect()
    }

    /// Models lists the catalog models whose provider has credentials here, and the default.
    /// Checking is local: re:agent reads environment keys and its login file, nothing remote.
    pub fn models(&self) -> (Vec<Model>, Option<String>) {
        let usable = |model: &str| {
            let mut config = serde_json::json!({ "model": model });
            reagent::provider::configure(&mut config)
                .ok()
                .map(|_| config["model"].as_str().unwrap_or("").to_owned())
        };
        let models = reagent::catalog::models()
            .as_array()
            .into_iter()
            .flatten()
            .filter(|entry| usable(entry["id"].as_str().unwrap_or("")).is_some())
            .map(|entry| Model {
                id: entry["id"].as_str().unwrap_or("").into(),
                provider: entry["provider"].as_str().unwrap_or("").into(),
                note: entry["note"].as_str().unwrap_or("").into(),
            })
            .collect();
        (models, usable(self.launch.model.as_deref().unwrap_or("")))
    }

    /// Create opens a new Session, on a chosen catalog model if given, and records an untitled chat.
    pub fn create(&self, model: Option<&str>) -> Result<Chat, Failure> {
        if let Some(model) = model {
            known(model)?;
        }
        let api = session_api::open_session(self.launch.options(false, model))?;
        let snapshot = api.get_snapshot()?;
        let id = snapshot.session.session_id;
        let chat = self
            .store
            .lock()
            .unwrap()
            .create_chat(&id, &snapshot.model, now_ms())?;
        self.live.lock().unwrap().insert(id, api);
        Ok(chat)
    }

    /// Switch_model moves an idle chat to another catalog model, carrying its history over.
    pub fn switch_model(&self, chat_id: &str, model: &str) -> Result<Chat, Failure> {
        known(model)?;
        let (chat, _) = self.get(chat_id)?;
        if chat.archived_at_ms.is_some() {
            return Err(archived());
        }
        let api = self.session(chat_id)?;
        let snapshot = api.get_snapshot()?;
        if snapshot.model != model {
            let request = reagent::types::new_id();
            let change = SessionChange::SwitchModel {
                model: model.into(),
                history: HistoryMode::Carry,
            };
            api.change_session(&request, snapshot.revision, change)?;
            let command = api.wait_command(&request)?;
            if command.state != CommandState::Applied {
                let reason = command
                    .reason
                    .unwrap_or_else(|| "the switch did not apply".into());
                return Err(Failure::new(409, "not_applied", reason));
            }
        }
        self.store.lock().unwrap().set_model(chat_id, model)?;
        Ok(self.get(chat_id)?.0)
    }

    /// Archive hides an idle chat and closes its Session here, so `reagent --resume ID` can open it.
    pub fn archive(&self, chat_id: &str) -> Result<(), Failure> {
        let (_, messages) = self.get(chat_id)?;
        if messages.iter().any(|m| m.status == "pending") {
            return Err(Failure::new(
                409,
                "busy",
                "Stop the reply before archiving this chat.",
            ));
        }
        let api = self.live.lock().unwrap().remove(chat_id);
        if let Some(api) = api {
            api.close()?;
        }
        self.store.lock().unwrap().archive(chat_id, now_ms())?;
        let _ = self.events.send(Event::Archived {
            chat_id: chat_id.into(),
        });
        Ok(())
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
        let (chat, messages) = self.get(chat_id)?;
        if chat.archived_at_ms.is_some() {
            return Err(archived());
        }
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
            sources: vec![],
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
        let reply_chat = chat_id.to_owned();
        std::thread::spawn(move || chats.await_reply(&reply_chat, &api, &run));
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

    fn await_reply(&self, chat_id: &str, api: &SessionApi, run: &RunRef) {
        let mut cursor = None;
        // Any read error leaves the reply to arrive whole from wait_run.
        while let Ok(page) = api.read_preview(run, cursor, 30_000, None) {
            if page.finished {
                break;
            }
            let searched = cursor.is_some_and(|c: PreviewCursor| c.searching) != page.searching;
            cursor = Some(page.next);
            if page.restarted || !page.text.is_empty() || searched {
                self.preview(chat_id, &run.run_id, &page);
            }
        }
        let mut sources = Vec::new();
        let (text, status, reason) = match api.wait_run(run).map(|info| info.result) {
            Ok(Some(result)) => {
                let text = String::from_utf8_lossy(&result.reply).into_owned();
                sources = result.sources;
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
        // The final reply replaces the preview; a stopped reply shows none of its partial text.
        self.previews.lock().unwrap().remove(&run.run_id);
        match store.finish_message(&run.run_id, &text, status, reason.as_deref(), &sources) {
            Ok(message) => {
                let _ = self.events.send(Event::Message { message });
            }
            Err(error) => eprintln!("covalent: record reply {}: {error}", run.run_id),
        }
    }

    /// Preview records a pending reply's new text and announces it under the same lock,
    /// so a reload sees either the text or the event that follows it.
    fn preview(&self, chat_id: &str, message_id: &str, page: &PreviewPage) {
        let mut previews = self.previews.lock().unwrap();
        let (_, preview) = previews
            .entry(message_id.to_owned())
            .or_insert_with(|| (chat_id.to_owned(), Preview::default()));
        preview.seq += 1;
        if page.restarted {
            preview.text.clear();
        }
        preview.text.push_str(&page.text);
        preview.searching = page.searching;
        let _ = self.events.send(Event::Preview {
            chat_id: chat_id.into(),
            message_id: message_id.into(),
            seq: preview.seq,
            restarted: page.restarted,
            text: page.text.clone(),
            searching: page.searching,
        });
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

fn archived() -> Failure {
    Failure::new(
        409,
        "archived",
        "This chat is archived. Continue it in a terminal with reagent --resume.",
    )
}

/// Known accepts only catalog models, since re:agent takes any name and fails later.
fn known(model: &str) -> Result<(), Failure> {
    if reagent::catalog::model_info(model).is_null() {
        return Err(Failure::new(
            400,
            "invalid_argument",
            format!("{model} is not an offered model"),
        ));
    }
    Ok(())
}

/// Now_ms is the current Unix time in milliseconds.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}
