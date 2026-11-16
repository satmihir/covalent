use crate::activity::Action;
use crate::files::Attachment;
use crate::store::{Chat, Image, Message, Store};
use reagent::session_api::{
    self, ApiError, CommandState, EntryCursor, EventCursor, EventKind, HistoryMode, PlanMode,
    PreviewCursor, PreviewPage, RunKind, RunRef, RunState, SessionApi, SessionChange,
    SessionOptions, SessionSnapshot,
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

/// IMAGES is added to a chat's instructions when it can make images.
const IMAGES: &str = "\n\nYou can create images with the generate_image tool. Use it only when \
    the user asks for a picture, drawing or illustration, and write the tool's prompt as a full \
    description of the image. Save each image as a new .png file with a short descriptive name. \
    To change an image, generate a new one with the original as a reference image, under a new \
    name, and say which file is the result. The image is shown to the user right above your \
    reply, so answer with one short sentence: do not describe the image at length, link to it, \
    or repeat the prompt.";

/// MAX_TOOL_CALLS and MAX_STEPS bound one reply's work, and so what a runaway reply can cost.
const MAX_TOOL_CALLS: u64 = 40;
const MAX_STEPS: u64 = 30;

/// SPOKEN is the note a dictated turn carries, so the reply suits being heard.
const SPOKEN: &str = "The user said this message aloud in voice mode, so it may contain \
    transcription errors; answer what they most likely meant. Your reply will be read aloud: \
    answer in short, natural spoken sentences, without Markdown, lists, tables, code or URLs, \
    and keep it brief unless they ask for detail. They are waiting to hear you, so answer from \
    what you know; search the web only when the answer depends on recent or local facts you \
    do not have, and then search once.";

/// MAX_ATTACHMENTS bounds the files one message can bring, and so the note that lists them.
const MAX_ATTACHMENTS: usize = 10;

/// Launch holds the settings every chat Session opens with.
#[derive(Clone, Debug)]
pub struct Launch {
    pub model: Option<String>,
    pub effort: Option<String>,
    pub title_model: Option<String>,
    pub title_effort: Option<String>,
    /// scripted replaces inference with a re:agent response script, for offline tests.
    pub scripted: Option<PathBuf>,
    /// data is Covalent's data folder: each chat's folder is in `workspaces/`, and the
    /// pictures chats made before they had folders are in `images/`.
    pub data: PathBuf,
    /// images lets new chats create pictures, which needs the OpenAI key.
    pub images: bool,
}

impl Launch {
    /// Title_options returns the settings of a throwaway Session that writes a chat's title.
    pub fn title_options(&self) -> SessionOptions {
        SessionOptions {
            model: self.title_model.clone(),
            reasoning_effort: self.title_effort.clone(),
            scripted: self.scripted.clone(),
            tool_free: true,
            instructions: Some(TITLE_PROMPT.into()),
            agents: false,
            checkpoint: false,
            origin: Some("covalent".into()),
            ..SessionOptions::default()
        }
    }

    /// Chat_options returns the settings of a chat's Session, which works with files in
    /// `workspace` and runs no commands. A model chosen in the picker runs at its own default effort.
    pub fn chat_options(&self, chosen: Option<&str>, workspace: PathBuf) -> SessionOptions {
        let (model, effort) = match chosen {
            Some(model) => (Some(model.to_owned()), None),
            None => (self.model.clone(), self.effort.clone()),
        };
        let instructions = if self.images {
            format!("{}{IMAGES}", PROMPT.trim_end())
        } else {
            PROMPT.to_owned()
        };
        SessionOptions {
            workspace,
            model,
            reasoning_effort: effort,
            scripted: self.scripted.clone(),
            instructions: Some(instructions),
            agents: false,
            exec: false,
            web_search: true,
            image_generation: self.images,
            max_steps: MAX_STEPS,
            max_tool_calls: MAX_TOOL_CALLS,
            origin: Some("covalent".into()),
            ..SessionOptions::default()
        }
    }

    /// Workspace is the path of a chat's folder, from the name the store records.
    fn workspace(&self, chat: &Chat) -> Option<PathBuf> {
        Some(self.data.join("workspaces").join(chat.workspace.as_ref()?))
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
        /// generating is true from when this reply starts making an image until it finishes.
        generating: bool,
    },
    /// Admitted carries a new user message and its pending reply, so every open page shows a
    /// message sent from elsewhere, such as another tab or the phone app's voice mode.
    Admitted {
        chat_id: String,
        messages: Vec<Message>,
    },
    /// Activity carries all of a pending reply's tool calls so far, each time one changes.
    Activity {
        chat_id: String,
        message_id: String,
        activity: Vec<Action>,
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
    pub generating: bool,
    pub activity: Vec<Action>,
}

/// Reply is an assistant message's text as speech follows it.
#[derive(Debug, PartialEq)]
pub enum Reply {
    /// Pending carries the text written so far, which a newer attempt may replace.
    Pending(String),
    /// Complete carries the final text.
    Complete(String),
    /// Ended means the reply failed or was stopped, so nothing more should be said.
    Ended,
}

/// Failure is a request error with the HTTP status it should produce.
#[derive(Debug)]
pub struct Failure {
    pub status: u16,
    pub code: String,
    pub message: String,
}

impl Failure {
    /// New builds a Failure from its HTTP status, error code and sentence.
    pub fn new(status: u16, code: &str, message: impl Into<String>) -> Self {
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

    /// Reply returns one of a chat's replies for reading aloud.
    pub fn reply(&self, chat_id: &str, message_id: &str) -> Result<Reply, Failure> {
        // The store lock comes first, as in await_reply, which removes a preview and records
        // the final text under it: a read sees one or the other.
        let store = self.store.lock().unwrap();
        let message = store
            .message(message_id)?
            .filter(|m| m.chat_id == chat_id && m.role == "assistant")
            .ok_or_else(|| Failure::new(404, "not_found", "no such reply"))?;
        Ok(match message.status.as_str() {
            "pending" => {
                let previews = self.previews.lock().unwrap();
                Reply::Pending(
                    previews
                        .get(message_id)
                        .map(|(_, p)| p.text.clone())
                        .unwrap_or_default(),
                )
            }
            "complete" => Reply::Complete(message.text),
            _ => Reply::Ended,
        })
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
        // The folder is named before the Session, whose ID names the chat, can exist.
        let folder = reagent::types::new_id();
        let workspace = self.launch.data.join("workspaces").join(&folder);
        std::fs::create_dir_all(&workspace).map_err(|e| {
            Failure::new(500, "workspace", format!("create the chat's folder: {e}"))
        })?;
        let api =
            match session_api::open_session(self.launch.chat_options(model, workspace.clone())) {
                Ok(api) => api,
                Err(error) => {
                    let _ = std::fs::remove_dir(&workspace);
                    return Err(error.into());
                }
            };
        let snapshot = api.get_snapshot()?;
        let id = snapshot.session.session_id;
        let chat = self.store.lock().unwrap().create_chat(
            &id,
            &snapshot.model,
            Some(&folder),
            now_ms(),
        )?;
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
    /// A spoken message carries a note asking for a reply suited to being heard.
    /// Attachments name files already uploaded to the chat's folder; the note lists them.
    pub fn send(
        self: &Arc<Self>,
        chat_id: &str,
        request_id: &str,
        text: &str,
        spoken: bool,
        attachments: &[String],
    ) -> Result<Vec<Message>, Failure> {
        if text.trim().is_empty() && attachments.is_empty() {
            return Err(Failure::new(400, "invalid_argument", "message is empty"));
        }
        let (chat, messages) = self.get(chat_id)?;
        if chat.archived_at_ms.is_some() {
            return Err(archived());
        }
        let attached = self.attached(&chat, attachments)?;
        // A retry after a restart must not reach re:agent, whose deduplication is per opening.
        if self.store.lock().unwrap().has_message(request_id)? {
            return Ok(messages);
        }
        let first = messages.is_empty();
        let api = self.session(chat_id)?;
        let note = note(spoken, &attached);
        let start = start(&api.get_snapshot()?);
        let submit = |api: &SessionApi| match &note {
            Some(note) => {
                api.send_message_with_note(request_id, text.as_bytes(), note, PlanMode::Keep)
            }
            None => api.send_message(request_id, text.as_bytes(), PlanMode::Keep),
        };
        let run = match submit(&api) {
            Err(error) if error.code == "resource_exhausted" => {
                // One opening keeps every receipt; a fresh opening of the same Session starts empty.
                api.close()?;
                self.live.lock().unwrap().remove(chat_id);
                submit(&self.session(chat_id)?)?
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
            images: vec![],
            activity: vec![],
            attachments: attached.into_iter().map(|(a, _)| a).collect(),
        };
        let reply = Message {
            id: run.run_id.clone(),
            role: "assistant".into(),
            text: String::new(),
            status: "pending".into(),
            attachments: vec![],
            ..user.clone()
        };
        {
            let store = self.store.lock().unwrap();
            store.insert_message(&user)?;
            store.insert_message(&reply)?;
        }
        // The reply's preview exists until it finishes, so nothing can announce it afterwards.
        self.previews
            .lock()
            .unwrap()
            .insert(run.run_id.clone(), (chat_id.into(), Preview::default()));
        // Announced before the reply's work starts, so it comes before any event about the reply.
        let _ = self.events.send(Event::Admitted {
            chat_id: chat_id.into(),
            messages: vec![user, reply],
        });
        let chats = self.clone();
        let api = self.session(chat_id)?;
        let reply_chat = chat_id.to_owned();
        std::thread::spawn(move || chats.await_reply(&reply_chat, &api, &run, start));
        if first {
            let chats = self.clone();
            // A message of files alone is titled from their names.
            let text = if text.trim().is_empty() {
                attachments.join(", ")
            } else {
                text.to_owned()
            };
            let chat_id = chat_id.to_owned();
            std::thread::spawn(move || chats.write_title(&chat_id, &text));
        }
        Ok(self.get(chat_id)?.1)
    }

    /// Attached checks that each named file is in the chat's folder, with its size and kind.
    fn attached(
        &self,
        chat: &Chat,
        paths: &[String],
    ) -> Result<Vec<(Attachment, &'static str)>, Failure> {
        if paths.is_empty() {
            return Ok(vec![]);
        }
        let refuse = |message: String| Failure::new(400, "invalid_argument", message);
        if paths.len() > MAX_ATTACHMENTS {
            return Err(refuse(format!(
                "At most {MAX_ATTACHMENTS} files can be attached to one message."
            )));
        }
        let folder = self
            .launch
            .workspace(chat)
            .ok_or_else(|| refuse("This chat has no folder for files.".into()))?;
        paths
            .iter()
            .map(|path| {
                let file = crate::files::inside(&folder, path).ok_or_else(|| {
                    refuse(format!("{path} is not a file in this chat's folder."))
                })?;
                let size = file.metadata().map_or(0, |m| m.len());
                let attachment = Attachment {
                    path: path.clone(),
                    size,
                };
                Ok((attachment, crate::files::kind(&file)))
            })
            .collect()
    }

    /// Upload saves a file the user is about to send into the chat's folder.
    pub fn upload(&self, chat_id: &str, name: &str, bytes: &[u8]) -> Result<Attachment, Failure> {
        let chat = self
            .store
            .lock()
            .unwrap()
            .chat(chat_id)?
            .ok_or_else(|| Failure::new(404, "not_found", "no such chat"))?;
        if chat.archived_at_ms.is_some() {
            return Err(archived());
        }
        let folder = self.launch.workspace(&chat).ok_or_else(|| {
            Failure::new(
                400,
                "invalid_argument",
                "This chat has no folder for files.",
            )
        })?;
        if !crate::files::uploadable(name) {
            return Err(Failure::new(
                400,
                "invalid_argument",
                format!("{name} can't be uploaded: a file name can't be empty, start with a dot, or contain a slash."),
            ));
        }
        crate::files::save(&folder, name, bytes)
            .map_err(|e| Failure::new(500, "workspace", format!("save {name}: {e}")))
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

    fn await_reply(
        self: &Arc<Self>,
        chat_id: &str,
        api: &SessionApi,
        run: &RunRef,
        (entries, events): (EntryCursor, EventCursor),
    ) {
        let follower = {
            let (chats, chat_id, api, run) =
                (self.clone(), chat_id.to_owned(), api.clone(), run.clone());
            std::thread::spawn(move || chats.follow(&chat_id, &api, &run, Some(entries), events))
        };
        let mut cursor = None;
        // Any read error leaves the reply to arrive whole from wait_run.
        while let Ok(page) = api.read_preview(run, cursor, 30_000, None) {
            if page.finished {
                break;
            }
            let before = cursor.map_or((false, false), |c: PreviewCursor| {
                (c.searching, c.generating)
            });
            let busy = before != (page.searching, page.generating);
            cursor = Some(page.next);
            if page.restarted || !page.text.is_empty() || busy {
                self.preview(chat_id, &run.run_id, &page);
            }
        }
        let mut sources = Vec::new();
        let mut images = Vec::new();
        let (text, status, reason) = match api.wait_run(run).map(|info| info.result) {
            Ok(Some(result)) => {
                let text = String::from_utf8_lossy(&result.reply).into_owned();
                sources = result.sources;
                // An image that was saved is shown however the reply ended: it exists.
                images = result
                    .effects
                    .iter()
                    .filter(|e| e.tool == "generate_image" && e.operation == "create")
                    .filter_map(|e| self.saved_image(chat_id, std::str::from_utf8(&e.path).ok()?))
                    .collect();
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
        // The follower stops once the Run is over, having read all of its Entries.
        let mut activity = follower.join().unwrap_or_default();
        crate::activity::stop(&mut activity);
        let store = self.store.lock().unwrap();
        // The final reply replaces the preview; a stopped reply shows none of its partial text.
        self.previews.lock().unwrap().remove(&run.run_id);
        let Some(Ok(mut reply)) = store.message(&run.run_id).transpose() else {
            eprintln!(
                "covalent: record reply {}: the message is missing",
                run.run_id
            );
            return;
        };
        reply.text = text;
        reply.status = status.into();
        reply.reason = reason;
        reply.sources = sources;
        reply.images = images;
        reply.activity = activity;
        match store.finish_message(&reply) {
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
        let Some((_, preview)) = previews.get_mut(message_id) else {
            return;
        };
        preview.seq += 1;
        if page.restarted {
            preview.text.clear();
        }
        preview.text.push_str(&page.text);
        preview.searching = page.searching;
        // The image itself arrives with the finished reply, so its placeholder stays until then.
        preview.generating |= page.generating;
        let _ = self.events.send(Event::Preview {
            chat_id: chat_id.into(),
            message_id: message_id.into(),
            seq: preview.seq,
            restarted: page.restarted,
            text: page.text.clone(),
            searching: page.searching,
            generating: preview.generating,
        });
    }

    /// Follow announces a reply's tool calls as they are made and finish, until its Run ends,
    /// and returns them.
    fn follow(
        &self,
        chat_id: &str,
        api: &SessionApi,
        run: &RunRef,
        mut entries: Option<EntryCursor>,
        mut events: EventCursor,
    ) -> Vec<Action> {
        let mut activity = Vec::new();
        loop {
            let Ok(page) = api.read_events(events.clone(), 100, 30_000, None) else {
                // Events were missed or belong to an earlier opening; the Entries stay readable.
                let _ = api.wait_run(run);
                read_activity(api, run, &mut entries, &mut activity);
                return activity;
            };
            events = page.next_cursor.clone();
            let ended = page.end_of_opening || page.events.iter().any(|e| {
                matches!(&e.change, EventKind::RunChanged { run: r, state: RunState::Terminal, .. }
                        if r.run_id == run.run_id)
            });
            let added = page.events.iter().any(|e| e.entries_available.is_some());
            if (added || ended) && read_activity(api, run, &mut entries, &mut activity) {
                let mut previews = self.previews.lock().unwrap();
                if let Some((_, preview)) = previews.get_mut(&run.run_id) {
                    preview.activity = activity.clone();
                    let _ = self.events.send(Event::Activity {
                        chat_id: chat_id.into(),
                        message_id: run.run_id.clone(),
                        activity: activity.clone(),
                    });
                }
            }
            if ended {
                return activity;
            }
        }
    }

    /// Saved_image describes a generated file by the size in its PNG header.
    fn saved_image(&self, chat_id: &str, name: &str) -> Option<Image> {
        let chat = self.store.lock().unwrap().chat(chat_id).ok()??;
        let path = match self.launch.workspace(&chat) {
            Some(folder) => crate::files::inside(&folder, name)?,
            None => self.launch.data.join("images").join(image_name(name)?),
        };
        let bytes = std::fs::read(path).ok()?;
        let size = |at: usize| Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
        let png = bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.get(12..16) == Some(b"IHDR");
        Some(Image {
            name: name.to_owned(),
            width: size(16).filter(|_| png)?,
            height: size(20)?,
        })
    }

    /// Image returns the bytes of a picture made before chats had folders, only for a name
    /// some message records.
    pub fn image(&self, name: &str) -> Result<Vec<u8>, Failure> {
        let missing = || Failure::new(404, "not_found", "no such image");
        let name = image_name(name).ok_or_else(missing)?;
        if !self.store.lock().unwrap().has_image(name)? {
            return Err(missing());
        }
        std::fs::read(self.launch.data.join("images").join(name)).map_err(|_| missing())
    }

    /// Files lists a chat's folder; a chat from before folders has none.
    pub fn files(&self, chat_id: &str) -> Result<crate::files::Listing, Failure> {
        let missing = || Failure::new(404, "not_found", "This chat has no folder.");
        let chat = self
            .store
            .lock()
            .unwrap()
            .chat(chat_id)?
            .ok_or_else(missing)?;
        let folder = self.launch.workspace(&chat).ok_or_else(missing)?;
        crate::files::list(&folder)
            .map_err(|e| Failure::new(500, "workspace", format!("list the chat's folder: {e}")))
    }

    /// File returns one file from a chat's folder, refusing any path that leaves it.
    pub fn file(&self, chat_id: &str, path: &str) -> Result<Vec<u8>, Failure> {
        let missing = || Failure::new(404, "not_found", "no such file");
        let chat = self
            .store
            .lock()
            .unwrap()
            .chat(chat_id)?
            .ok_or_else(missing)?;
        let folder = self.launch.workspace(&chat).ok_or_else(missing)?;
        let path = crate::files::inside(&folder, path).ok_or_else(missing)?;
        std::fs::read(path).map_err(|_| missing())
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

/// Note is what the model is told beside a message: that it was spoken, and which files the
/// user attached.
fn note(spoken: bool, attached: &[(Attachment, &str)]) -> Option<String> {
    let mut parts = Vec::new();
    if spoken {
        parts.push(SPOKEN.to_owned());
    }
    if !attached.is_empty() {
        let mut files = String::from(
            "The user attached these files to this message; they are in the chat's folder \
             under these names:",
        );
        for (attachment, kind) in attached {
            files.push_str(&format!(
                "\n- {} ({kind}, {})",
                attachment.path,
                size(attachment.size)
            ));
        }
        parts.push(files);
    }
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

/// Size writes a byte count the way a file list does.
fn size(bytes: u64) -> String {
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{:.1} KB", bytes as f64 / 1024.0),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}

/// Start is where a reply's activity is read from: history and events as they stand before it.
fn start(snapshot: &SessionSnapshot) -> (EntryCursor, EventCursor) {
    let entries = EntryCursor {
        session: snapshot.session.clone(),
        generation: snapshot.context.history_generation,
        position: snapshot.context.entry_count,
    };
    (entries, snapshot.event_cursor.clone())
}

/// Read_activity adds a Run's new tool calls and outcomes to `activity` and reports a change.
/// Rewritten history, or a cursor from an earlier opening, restarts from the beginning.
fn read_activity(
    api: &SessionApi,
    run: &RunRef,
    entries: &mut Option<EntryCursor>,
    activity: &mut Vec<Action>,
) -> bool {
    let mut changed = false;
    loop {
        let page = match api.read_entries(entries.clone(), 100) {
            Ok(page) => page,
            Err(error)
                if entries.is_some() && matches!(error.code, "cursor_expired" | "conflict") =>
            {
                *entries = None;
                continue;
            }
            Err(_) => return changed,
        };
        for entry in &page.entries {
            if entry.run.as_ref().is_some_and(|r| r.run_id == run.run_id) {
                changed |= crate::activity::record(activity, &entry.content);
            }
        }
        *entries = Some(page.next_cursor);
        if !page.has_more {
            return changed;
        }
    }
}

/// Image_name accepts only the file names re:agent gives generated images.
fn image_name(name: &str) -> Option<&str> {
    let id = name.strip_prefix("image-")?.strip_suffix(".png")?;
    (!id.is_empty() && id.bytes().all(|b| b.is_ascii_hexdigit())).then_some(name)
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
