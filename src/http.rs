use crate::chats::{Chats, Failure, Reply};
use crate::speech::{self, Voice};
use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, FromRef, Path, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::convert::Infallible;
use std::sync::Arc;
use tokio_stream::wrappers::{BroadcastStream, ReceiverStream};
use tokio_stream::{Stream, StreamExt};
use tower_http::services::{ServeDir, ServeFile};

/// RECORDING caps one dictated turn, several minutes of compressed speech.
const RECORDING: usize = 10 * 1024 * 1024;

/// App is what handlers share: the chats, and voice when an OpenAI key is configured.
#[derive(Clone)]
struct App {
    chats: Arc<Chats>,
    voice: Option<Voice>,
}

impl FromRef<App> for Arc<Chats> {
    fn from_ref(app: &App) -> Self {
        app.chats.clone()
    }
}

/// Router serves the JSON API under `/api` and the built frontend from `web` for every other path.
pub fn router(chats: Arc<Chats>, voice: Option<Voice>, web: &std::path::Path) -> Router {
    let frontend = ServeDir::new(web).fallback(ServeFile::new(web.join("index.html")));
    Router::new()
        .route("/api/voice", get(voice_available))
        .route(
            "/api/transcribe",
            post(transcribe).layer(DefaultBodyLimit::max(RECORDING)),
        )
        .route("/api/chats/{id}/messages/{message}/speech", get(speak))
        .route("/api/images/{name}", get(image))
        .route("/api/chats/{id}/files", get(files))
        .route(
            "/api/chats/{id}/files/{*path}",
            get(file)
                .put(upload)
                .layer(DefaultBodyLimit::max(crate::files::MAX_UPLOAD)),
        )
        .route("/api/models", get(models))
        .route("/api/chats", get(list).post(create))
        .route("/api/chats/{id}", get(chat))
        .route("/api/chats/{id}/messages", post(send))
        .route("/api/chats/{id}/cancel", post(cancel))
        .route("/api/chats/{id}/model", post(switch_model))
        .route("/api/chats/{id}/archive", post(archive))
        .route("/api/events", get(events))
        .fallback_service(frontend)
        .layer(middleware::from_fn(local))
        .with_state(App { chats, voice })
}

/// Local refuses a request a browser addressed to, or sent from, any site but this machine,
/// so a web page can neither point its own name at Covalent nor post to it from elsewhere.
/// Browsers always send Host, and Origin on a cross-site post; a request without them did
/// not come from a page.
async fn local(request: Request, next: Next) -> Response {
    if addressed_here(request.headers()) {
        return next.run(request).await;
    }
    let body = json!({"error": "forbidden", "message": "Covalent answers only this machine."});
    (StatusCode::FORBIDDEN, Json(body)).into_response()
}

fn addressed_here(headers: &HeaderMap) -> bool {
    let header = |name| headers.get(name).map(|v| v.to_str().unwrap_or(""));
    let origin = header(header::ORIGIN).map(|origin| {
        let origin = origin.split_once("://").map_or("", |(_, rest)| rest);
        origin.split('/').next().unwrap_or("")
    });
    [header(header::HOST), origin]
        .into_iter()
        .flatten()
        .all(loopback)
}

/// Loopback accepts this machine's own names, with or without a port.
fn loopback(host: &str) -> bool {
    let name = match host.rsplit_once(':') {
        Some((name, port)) if port.bytes().all(|b| b.is_ascii_digit()) => name,
        _ => host,
    };
    matches!(name, "127.0.0.1" | "localhost" | "[::1]")
}

#[derive(Deserialize)]
struct SendBody {
    request_id: String,
    text: String,
    /// spoken marks a turn said aloud in voice mode, whose reply will be read aloud.
    #[serde(default)]
    spoken: bool,
    /// attachments names files already uploaded to the chat's folder.
    #[serde(default)]
    attachments: Vec<String>,
}

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let body = json!({"error": self.code, "message": self.message});
        (status, Json(body)).into_response()
    }
}

/// Blocking runs Session and store work off the async runtime, which they would otherwise stall.
async fn blocking<T, F>(work: F) -> Result<Json<T>, Failure>
where
    T: Serialize + Send + 'static,
    F: FnOnce() -> Result<T, Failure> + Send + 'static,
{
    match tokio::task::spawn_blocking(work).await {
        Ok(result) => result.map(Json),
        Err(error) => Err(Failure {
            status: 500,
            code: "internal".into(),
            message: format!("request worker failed: {error}"),
        }),
    }
}

async fn list(State(chats): State<Arc<Chats>>) -> impl IntoResponse {
    blocking(move || Ok(json!({"chats": chats.list()?}))).await
}

async fn models(State(chats): State<Arc<Chats>>) -> impl IntoResponse {
    blocking(move || {
        let (models, default) = chats.models();
        Ok(json!({"models": models, "default": default}))
    })
    .await
}

/// Create takes an optional `{"model": id}`; without one the chat uses Covalent's default.
async fn create(State(chats): State<Arc<Chats>>, Json(body): Json<Value>) -> impl IntoResponse {
    blocking(move || {
        let chat = chats.create(body["model"].as_str())?;
        Ok(json!({ "chat": chat }))
    })
    .await
}

#[derive(Deserialize)]
struct ModelBody {
    model: String,
}

async fn switch_model(
    State(chats): State<Arc<Chats>>,
    Path(id): Path<String>,
    Json(body): Json<ModelBody>,
) -> impl IntoResponse {
    blocking(move || Ok(json!({"chat": chats.switch_model(&id, &body.model)?}))).await
}

async fn archive(State(chats): State<Arc<Chats>>, Path(id): Path<String>) -> impl IntoResponse {
    blocking(move || {
        chats.archive(&id)?;
        Ok(json!({}))
    })
    .await
}

async fn chat(State(chats): State<Arc<Chats>>, Path(id): Path<String>) -> impl IntoResponse {
    blocking(move || {
        let (chat, messages) = chats.get(&id)?;
        let previews = chats.previews(&id);
        Ok(json!({"chat": chat, "messages": messages, "previews": previews}))
    })
    .await
}

async fn send(
    State(chats): State<Arc<Chats>>,
    Path(id): Path<String>,
    Json(body): Json<SendBody>,
) -> impl IntoResponse {
    blocking(move || {
        let messages = chats.send(
            &id,
            &body.request_id,
            &body.text,
            body.spoken,
            &body.attachments,
        )?;
        Ok(json!({"messages": messages}))
    })
    .await
}

async fn cancel(State(chats): State<Arc<Chats>>, Path(id): Path<String>) -> impl IntoResponse {
    blocking(move || {
        chats.cancel(&id)?;
        Ok(json!({}))
    })
    .await
}

async fn voice_available(State(app): State<App>) -> Json<Value> {
    Json(json!({"available": app.voice.is_some()}))
}

fn voice(app: &App) -> Result<Voice, Failure> {
    app.voice.clone().ok_or_else(|| {
        Failure::new(
            503,
            "voice_unavailable",
            "Voice needs OPENAI_API_KEY where Covalent starts.",
        )
    })
}

/// Transcribe takes one recording as the body, typed by its Content-Type, and returns its words.
async fn transcribe(
    State(app): State<App>,
    headers: HeaderMap,
    audio: Bytes,
) -> Result<Json<Value>, Failure> {
    let voice = voice(&app)?;
    let media_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let text = voice.transcribe(audio.to_vec(), media_type).await?;
    Ok(Json(json!({ "text": text })))
}

/// Speak streams a reply read aloud as raw PCM; closing the request stops the reading.
async fn speak(
    State(app): State<App>,
    Path((id, message)): Path<(String, String)>,
) -> Result<Response, Failure> {
    let voice = voice(&app)?;
    let (chats, chat, reply) = (app.chats.clone(), id.clone(), message.clone());
    let found = tokio::task::spawn_blocking(move || chats.reply(&chat, &reply))
        .await
        .map_err(|e| Failure::new(500, "internal", format!("request worker failed: {e}")))??;
    if found == Reply::Ended {
        return Err(Failure::new(
            409,
            "not_speakable",
            "This reply did not finish, so there is nothing to read.",
        ));
    }
    let reader = voice.open().await?;
    let (out, audio) = tokio::sync::mpsc::channel(64);
    tokio::spawn(speech::read_aloud(
        app.chats.clone(),
        reader,
        id,
        message,
        out,
    ));
    let headers = [
        (
            header::CONTENT_TYPE,
            format!("audio/pcm;rate={}", speech::RATE),
        ),
        (header::CACHE_CONTROL, "no-store".to_owned()),
    ];
    Ok((headers, Body::from_stream(ReceiverStream::new(audio))).into_response())
}

/// Image serves one generated image; its name never changes what it holds, so browsers keep it.
async fn image(
    State(chats): State<Arc<Chats>>,
    Path(name): Path<String>,
) -> Result<Response, Failure> {
    let bytes = tokio::task::spawn_blocking(move || chats.image(&name))
        .await
        .map_err(|e| Failure::new(500, "internal", format!("request worker failed: {e}")))??;
    let headers = [
        (header::CONTENT_TYPE, "image/png"),
        (
            header::CACHE_CONTROL,
            "private, max-age=31536000, immutable",
        ),
    ];
    Ok((headers, bytes).into_response())
}

/// Events streams every chat's changes; a client that falls behind reloads what it shows.
async fn files(State(chats): State<Arc<Chats>>, Path(id): Path<String>) -> impl IntoResponse {
    blocking(move || chats.files(&id)).await
}

/// Upload saves the request body as a new file in a chat's folder, under the name in the path
/// or a numbered one beside it, and answers with the path it used.
async fn upload(
    State(chats): State<Arc<Chats>>,
    Path((id, name)): Path<(String, String)>,
    body: Bytes,
) -> impl IntoResponse {
    blocking(move || chats.upload(&id, &name, &body)).await
}

/// File serves one file from a chat's folder. The model wrote these files, so only pictures
/// are shown in the page; anything else is a download that cannot run as part of Covalent.
async fn file(
    State(chats): State<Arc<Chats>>,
    Path((id, path)): Path<(String, String)>,
) -> Result<Response, Failure> {
    let name = path.rsplit('/').next().unwrap_or("").to_owned();
    let bytes = tokio::task::spawn_blocking(move || chats.file(&id, &path))
        .await
        .map_err(|e| Failure::new(500, "internal", format!("request worker failed: {e}")))??;
    let extension = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    let picture = match extension.as_deref() {
        Some("png") => Some("image/png"),
        Some("jpg" | "jpeg") => Some("image/jpeg"),
        Some("webp") => Some("image/webp"),
        Some("gif") => Some("image/gif"),
        _ => None,
    };
    let mut response = bytes.into_response();
    let headers = response.headers_mut();
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, "nosniff".parse().unwrap());
    headers.insert(header::CACHE_CONTROL, "no-cache".parse().unwrap());
    if let Some(kind) = picture {
        headers.insert(header::CONTENT_TYPE, kind.parse().unwrap());
        return Ok(response);
    }
    headers.insert(
        header::CONTENT_TYPE,
        "application/octet-stream".parse().unwrap(),
    );
    let disposition = format!("attachment; filename*=UTF-8''{}", percent(&name));
    headers.insert(
        header::CONTENT_DISPOSITION,
        disposition
            .parse()
            .map_err(|_| Failure::new(500, "internal", "unusable file name"))?,
    );
    Ok(response)
}

/// Percent encodes a file name for a Content-Disposition header.
fn percent(name: &str) -> String {
    name.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'-' | b'_' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

async fn events(
    State(chats): State<Arc<Chats>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = BroadcastStream::new(chats.subscribe()).map(|item| {
        let data = match item {
            Ok(event) => serde_json::to_string(&event).unwrap(),
            Err(_) => json!({"type": "lagged"}).to_string(),
        };
        Ok(Event::default().data(data))
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chats::Launch;
    use crate::store::Store;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};
    use tower::ServiceExt;

    const CHILD: &str = "COVALENT_TEST_CHILD";

    /// Isolated reruns a test in a child process whose HOME is a temporary directory,
    /// because re:agent writes traces below the user's cache directory.
    fn isolated(name: &str, env: &[(&str, String)]) -> bool {
        if std::env::var_os(CHILD).is_some() {
            return true;
        }
        let home = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name, "--nocapture"])
            .env(CHILD, "1")
            .env("HOME", home.path())
            .env("XDG_CACHE_HOME", home.path().join("cache"))
            .envs(env.iter().map(|(k, v)| (k, v)))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        false
    }

    /// Launch is a chat launch that keeps its folders in `dir` and makes no images.
    fn launch(dir: &std::path::Path) -> Launch {
        Launch {
            model: None,
            effort: None,
            title_model: None,
            title_effort: None,
            scripted: None,
            data: dir.into(),
            images: false,
        }
    }

    fn app(dir: &std::path::Path, replies: &[&str]) -> (Router, Arc<Chats>) {
        let script = dir.join("script.json");
        let responses: Vec<Value> = replies
            .iter()
            .map(|text| json!({"blocks": [{"kind": "text", "text": text}]}))
            .collect();
        std::fs::write(&script, serde_json::to_vec(&responses).unwrap()).unwrap();
        let launch = Launch {
            scripted: Some(script),
            ..launch(dir)
        };
        let chats = Chats::new(Store::open(&dir.join("covalent.db")).unwrap(), launch);
        (router(chats.clone(), None, &dir.join("web")), chats)
    }

    async fn call(app: &Router, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn settled(app: &Router, id: &str) -> Value {
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            let (_, body) = call(app, "GET", &format!("/api/chats/{id}"), Value::Null).await;
            let pending = body["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m["status"] == "pending");
            if !pending && !body["chat"]["title"].is_null() {
                return body;
            }
            assert!(Instant::now() < end, "chat never settled: {body}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn a_new_chat_records_the_reply_and_a_title() {
        if !isolated("http::tests::a_new_chat_records_the_reply_and_a_title", &[]) {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let (app, chats) = app(dir.path(), &["Sourdough needs a starter.", "unused"]);
        let mut events = chats.subscribe();
        let (status, created) = call(&app, "POST", "/api/chats", Value::Null).await;
        assert_eq!(status, StatusCode::OK, "{created}");
        let id = created["chat"]["id"].as_str().unwrap().to_owned();
        let (status, sent) = call(
            &app,
            "POST",
            &format!("/api/chats/{id}/messages"),
            json!({"request_id": "r1", "text": "How do I bake bread?"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{sent}");
        let roles: Vec<_> = sent["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["role"].as_str().unwrap())
            .collect();
        assert_eq!(roles, ["user", "assistant"]);
        // A scripted reply can finish before the send response is built.
        assert!(matches!(
            sent["messages"][1]["status"].as_str(),
            Some("pending" | "complete")
        ));

        let body = settled(&app, &id).await;
        let reply = &body["messages"][1];
        assert_eq!(reply["status"], "complete", "{reply}");
        assert_eq!(reply["text"], "Sourdough needs a starter.");
        // A scripted title Session reads the same script from its start.
        assert_eq!(body["chat"]["title"], "Sourdough needs a starter");

        let mut kinds = Vec::new();
        while let Ok(event) = events.try_recv() {
            kinds.push(serde_json::to_value(event).unwrap()["type"].clone());
        }
        kinds.sort_by_key(|k| k.to_string());
        assert_eq!(kinds, [json!("message"), json!("title")]);
        let (_, listed) = call(&app, "GET", "/api/chats", Value::Null).await;
        assert_eq!(listed["chats"][0]["id"], id.as_str());
    }

    #[tokio::test]
    async fn a_retried_send_adds_no_messages() {
        if !isolated("http::tests::a_retried_send_adds_no_messages", &[]) {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let (app, _) = app(dir.path(), &["first", "second"]);
        let (_, created) = call(&app, "POST", "/api/chats", Value::Null).await;
        let id = created["chat"]["id"].as_str().unwrap().to_owned();
        let uri = format!("/api/chats/{id}/messages");
        let body = json!({"request_id": "same", "text": "hello"});
        call(&app, "POST", &uri, body.clone()).await;
        let (status, again) = call(&app, "POST", &uri, body).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(again["messages"].as_array().unwrap().len(), 2);
        assert_eq!(
            settled(&app, &id).await["messages"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn requests_for_unknown_chats_and_empty_messages_are_rejected() {
        if !isolated(
            "http::tests::requests_for_unknown_chats_and_empty_messages_are_rejected",
            &[],
        ) {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let (app, _) = app(dir.path(), &["unused"]);
        let (status, body) = call(&app, "GET", "/api/chats/0000000000000000", Value::Null).await;
        assert_eq!(
            (status, body["error"].clone()),
            (StatusCode::NOT_FOUND, json!("not_found"))
        );
        let (_, created) = call(&app, "POST", "/api/chats", Value::Null).await;
        let id = created["chat"]["id"].as_str().unwrap();
        let (status, body) = call(
            &app,
            "POST",
            &format!("/api/chats/{id}/messages"),
            json!({"request_id": "r", "text": "  \n"}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }

    /// PIXEL is a one-pixel PNG, base64-encoded as the Images API returns images.
    const PIXEL: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

    /// Stream_provider answers Responses requests on a loopback port: title requests at once,
    /// and the chat reply as a web search that ends once the `searched` file next to the release
    /// file exists, then "Sourdough", then " rises." once the release file exists. Each chat
    /// request's input is appended as one line to `requests.log` next to the release file.
    /// A chat message containing "draw" is answered with a generate_image call, then with
    /// "Here is your boat." once the tool's result comes back; the same port plays the
    /// Images API, answering with a one-pixel PNG.
    fn stream_provider(release: std::path::PathBuf) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let release = release.clone();
                std::thread::spawn(move || {
                    let mut request = vec![];
                    let mut byte = [0];
                    while !request.ends_with(b"\r\n\r\n") {
                        stream.read_exact(&mut byte).unwrap();
                        request.push(byte[0]);
                    }
                    let length: usize = String::from_utf8_lossy(&request)
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse().unwrap())
                        })
                        .unwrap();
                    let mut body = vec![0; length];
                    stream.read_exact(&mut body).unwrap();
                    let body: Value = serde_json::from_slice(&body).unwrap();
                    if String::from_utf8_lossy(&request).contains("/images/generations") {
                        let reply = json!({"data": [{"b64_json": PIXEL}]}).to_string();
                        let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len());
                        return;
                    }
                    let title = body["instructions"]
                        .as_str()
                        .unwrap_or("")
                        .contains("titles");
                    if !write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n").is_ok() {
                        return;
                    }
                    let mut send = |event: Value| {
                        let kind = event["type"].as_str().unwrap().to_owned();
                        write!(stream, "event: {kind}\ndata: {event}\n\n").is_ok()
                            && stream.flush().is_ok()
                    };
                    // A chat reply searches once and cites one page; a title does neither.
                    let done = |text: &str, cited: bool| {
                        let annotations = if cited {
                            json!([{"type":"url_citation","start_index":0,"end_index":9,"url":"https://bread.example/sourdough","title":"Sourdough basics"}])
                        } else {
                            json!([])
                        };
                        let message = json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":text,"annotations":annotations}]});
                        let output = if cited {
                            json!([{"type":"web_search_call","id":"ws","status":"completed","action":{"type":"search","query":"sourdough"}}, message])
                        } else {
                            json!([message])
                        };
                        json!({"type":"response.completed","response":{"id":"fixture","status":"completed","model":"gpt-6.1-sol","output":output,"usage":{"input_tokens":5,"output_tokens":2}}})
                    };
                    let delta = |text: &str| json!({"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":text});
                    if title {
                        send(done("Baking Bread", false));
                        return;
                    }
                    let mut log = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(release.with_file_name("requests.log"))
                        .unwrap();
                    writeln!(log, "{}", body["input"]).unwrap();
                    let input = body["input"].to_string();
                    if input.contains("function_call_output") {
                        let text = if input.contains("write_file") {
                            "Saved your list."
                        } else {
                            "Here is your boat."
                        };
                        send(done(text, false));
                        return;
                    }
                    if input.contains("attached these files") {
                        send(done("Got your files.", false));
                        return;
                    }
                    if input.contains("packing list") {
                        let call = json!({"type":"function_call","id":"fc_1","call_id":"call_1","name":"write_file","arguments":"{\"path\":\"list.md\",\"content\":\"- socks\\n\"}","status":"completed"});
                        send(
                            json!({"type":"response.completed","response":{"id":"fixture","status":"completed","model":"gpt-6.1-sol","output":[call],"usage":{"input_tokens":5,"output_tokens":2}}}),
                        );
                        return;
                    }
                    if input.contains("draw") {
                        let offered = body["tools"].to_string().contains("generate_image");
                        let call = json!({"type":"function_call","id":"fc_1","call_id":"call_1","name":"generate_image","arguments":"{\"prompt\":\"a red boat\",\"output_path\":\"boat.png\"}","status":"completed"});
                        let output = if offered { json!([call]) } else { json!([]) };
                        send(
                            json!({"type":"response.completed","response":{"id":"fixture","status":"completed","model":"gpt-6.1-sol","output":output,"usage":{"input_tokens":5,"output_tokens":2}}}),
                        );
                        return;
                    }
                    if !body["tools"]
                        .as_array()
                        .is_some_and(|tools| tools.contains(&json!({"type":"web_search"})))
                    {
                        send(done("NO SEARCH TOOL", false));
                        return;
                    }
                    send(
                        json!({"type":"response.web_search_call.searching","item_id":"ws","output_index":0}),
                    );
                    while !release.with_file_name("searched").exists() {
                        if !send(json!({"type":"keepalive"})) {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    send(
                        json!({"type":"response.web_search_call.completed","item_id":"ws","output_index":0}),
                    );
                    send(delta("Sourdough"));
                    while !release.exists() {
                        if !send(json!({"type":"keepalive"})) {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    send(delta(" rises."));
                    send(done("Sourdough rises.", true));
                });
            }
        });
        endpoint
    }

    #[tokio::test]
    async fn a_pending_reply_shows_its_text_so_far_and_then_the_final_reply() {
        let release = tempfile::tempdir().unwrap();
        let release_file = release.path().join("release");
        let env = [
            ("OPENAI_API_KEY", "covalent-test-key".to_owned()),
            (
                "REAGENT_OPENAI_BASE_URL",
                stream_provider(release_file.clone()),
            ),
            ("COVALENT_TEST_RELEASE", release_file.display().to_string()),
        ];
        if !isolated(
            "http::tests::a_pending_reply_shows_its_text_so_far_and_then_the_final_reply",
            &env,
        ) {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let launch = Launch {
            model: Some("gpt-6.1-sol".into()),
            effort: Some("low".into()),
            ..launch(dir.path())
        };
        let chats = Chats::new(
            Store::open(&dir.path().join("covalent.db")).unwrap(),
            launch,
        );
        let app = router(chats.clone(), None, &dir.path().join("web"));
        let mut events = chats.subscribe();
        let (_, created) = call(&app, "POST", "/api/chats", Value::Null).await;
        let id = created["chat"]["id"].as_str().unwrap().to_owned();
        let (_, sent) = call(
            &app,
            "POST",
            &format!("/api/chats/{id}/messages"),
            json!({"request_id": "r1", "text": "Tell me about bread."}),
        )
        .await;
        let reply = sent["messages"][1]["id"].as_str().unwrap().to_owned();
        let release = PathBuf::from(std::env::var("COVALENT_TEST_RELEASE").unwrap());

        let preview = |done: fn(&Value) -> bool| {
            let (app, id, reply) = (app.clone(), id.clone(), reply.clone());
            async move {
                let end = Instant::now() + Duration::from_secs(10);
                loop {
                    let (_, body) =
                        call(&app, "GET", &format!("/api/chats/{id}"), Value::Null).await;
                    if done(&body["previews"][&reply]) {
                        assert_eq!(body["messages"][1]["status"], "pending");
                        return;
                    }
                    assert!(Instant::now() < end, "no such preview: {body}");
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
        };
        preview(|p| p["searching"] == true).await;
        std::fs::write(release.with_file_name("searched"), b"").unwrap();
        preview(|p| p["text"] == "Sourdough" && p["searching"] == false).await;
        std::fs::write(&release, b"").unwrap();
        let body = settled(&app, &id).await;
        assert_eq!(body["messages"][1]["text"], "Sourdough rises.");
        assert_eq!(
            body["messages"][1]["sources"],
            json!([{"url":"https://bread.example/sourdough","title":"Sourdough basics"}])
        );
        assert_eq!(body["previews"], json!({}));

        let mut previews = vec![];
        let mut searched = false;
        while let Ok(event) = events.try_recv() {
            let event = serde_json::to_value(event).unwrap();
            if event["type"] == "preview" {
                searched |= event["searching"] == true;
                previews.push((
                    event["seq"].as_u64().unwrap(),
                    event["text"].as_str().unwrap().to_owned(),
                ));
            }
        }
        assert!(searched, "no preview reported the search");
        let text: String = previews.iter().map(|(_, text)| text.as_str()).collect();
        assert_eq!(text, "Sourdough rises.");
        assert!(previews
            .iter()
            .enumerate()
            .all(|(i, (seq, _))| *seq == i as u64 + 1));
    }

    #[tokio::test]
    async fn an_archived_chat_leaves_the_list_and_refuses_new_messages() {
        if !isolated(
            "http::tests::an_archived_chat_leaves_the_list_and_refuses_new_messages",
            &[],
        ) {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let (app, _) = app(dir.path(), &["first", "unused"]);
        let (_, created) = call(&app, "POST", "/api/chats", Value::Null).await;
        let id = created["chat"]["id"].as_str().unwrap().to_owned();
        let send = json!({"request_id": "r1", "text": "hello"});
        call(&app, "POST", &format!("/api/chats/{id}/messages"), send).await;
        settled(&app, &id).await;
        let (status, _) = call(
            &app,
            "POST",
            &format!("/api/chats/{id}/archive"),
            Value::Null,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (_, listed) = call(&app, "GET", "/api/chats", Value::Null).await;
        assert_eq!(listed["chats"], json!([]));
        let (_, body) = call(&app, "GET", &format!("/api/chats/{id}"), Value::Null).await;
        assert!(body["chat"]["archived_at_ms"].is_i64(), "{body}");
        assert_eq!(body["messages"].as_array().unwrap().len(), 2);
        let again = json!({"request_id": "r2", "text": "more"});
        let (status, body) = call(&app, "POST", &format!("/api/chats/{id}/messages"), again).await;
        assert_eq!(
            (status, body["error"].clone()),
            (StatusCode::CONFLICT, json!("archived"))
        );
    }

    #[tokio::test]
    async fn the_picker_offers_credentialed_models_and_switches_an_idle_chat() {
        let release = tempfile::tempdir().unwrap();
        let env = [
            ("OPENAI_API_KEY", "covalent-test-key".to_owned()),
            ("ANTHROPIC_API_KEY", String::new()),
            ("REAGENT_MODEL", String::new()),
            (
                "REAGENT_OPENAI_BASE_URL",
                stream_provider(release.path().join("unused")),
            ),
        ];
        if !isolated(
            "http::tests::the_picker_offers_credentialed_models_and_switches_an_idle_chat",
            &env,
        ) {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let chats = Chats::new(
            Store::open(&dir.path().join("covalent.db")).unwrap(),
            launch(dir.path()),
        );
        let app = router(chats, None, &dir.path().join("web"));
        let (_, models) = call(&app, "GET", "/api/models", Value::Null).await;
        let ids: Vec<_> = models["models"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["id"].as_str().unwrap())
            .collect();
        assert!(
            ids.contains(&"gpt-6-luna") && ids.contains(&"gpt-6.1-sol"),
            "{models}"
        );
        assert!(!ids.iter().any(|id| id.starts_with("claude-")), "{models}");
        assert_eq!(models["default"], "gpt-6.1-sol");

        let (status, body) = call(
            &app,
            "POST",
            "/api/chats",
            json!({"model": "no-such-model"}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        let (_, created) = call(&app, "POST", "/api/chats", json!({"model": "gpt-6-luna"})).await;
        assert_eq!(created["chat"]["model"], "gpt-6-luna");
        let id = created["chat"]["id"].as_str().unwrap().to_owned();
        let (status, switched) = call(
            &app,
            "POST",
            &format!("/api/chats/{id}/model"),
            json!({"model": "gpt-6.1-sol"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{switched}");
        assert_eq!(switched["chat"]["model"], "gpt-6.1-sol");
        let (_, body) = call(&app, "GET", &format!("/api/chats/{id}"), Value::Null).await;
        assert_eq!(body["chat"]["model"], "gpt-6.1-sol");
    }

    /// Openai_fake serves the two OpenAI routes voice uses, on a loopback port. Its "audio" is
    /// the text it was asked to read followed by "|", so a test can hear what was said. A text
    /// containing SLOW streams until the connection closes. With `reject`, both routes refuse
    /// the key, and transcription repeats it the way OpenAI repeats part of a wrong key.
    async fn openai_fake(reject: bool) -> (String, Arc<std::sync::Mutex<Vec<Value>>>) {
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let fake = Router::new()
            .route("/v1/audio/transcriptions", post(fake_transcription))
            .route("/v1/realtime", get(fake_realtime))
            .with_state((seen.clone(), reject));
        tokio::spawn(async move { axum::serve(listener, fake).await.unwrap() });
        (base, seen)
    }

    type Fake = State<(Arc<std::sync::Mutex<Vec<Value>>>, bool)>;

    fn authorization(headers: &HeaderMap) -> String {
        headers
            .get(header::AUTHORIZATION)
            .map(|v| v.to_str().unwrap().to_owned())
            .unwrap_or_default()
    }

    async fn fake_transcription(
        State((seen, reject)): Fake,
        headers: HeaderMap,
        body: Bytes,
    ) -> Response {
        let form = String::from_utf8_lossy(&body).into_owned();
        let after = |marker: &str, end: &str| {
            let start = form.find(marker).map(|i| i + marker.len())?;
            Some(form[start..].split(end).next()?.to_owned())
        };
        let bearer = authorization(&headers);
        seen.lock().unwrap().push(json!({
            "route": "transcription",
            "authorization": bearer,
            "model": after("name=\"model\"\r\n\r\n", "\r\n"),
            "filename": after("filename=\"", "\""),
            "audio": form.contains("FAKE-AUDIO"),
        }));
        if reject {
            let message = format!(
                "Incorrect API key provided: {}",
                bearer.trim_start_matches("Bearer ")
            );
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": {"message": message}})),
            )
                .into_response();
        }
        Json(json!({"text": "  Hello there.  ", "languages": [{"code": "en"}]})).into_response()
    }

    async fn fake_realtime(
        State((seen, reject)): Fake,
        headers: HeaderMap,
        axum::extract::RawQuery(query): axum::extract::RawQuery,
        upgrade: axum::extract::ws::WebSocketUpgrade,
    ) -> Response {
        seen.lock().unwrap().push(json!({
            "route": "realtime",
            "authorization": authorization(&headers),
            "query": query,
        }));
        if reject {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        upgrade.on_upgrade(move |socket| fake_session(socket, seen))
    }

    async fn fake_session(
        mut socket: axum::extract::ws::WebSocket,
        seen: Arc<std::sync::Mutex<Vec<Value>>>,
    ) {
        use axum::extract::ws::Message as Ws;
        use base64::Engine;
        let say = |event: Value| Ws::Text(event.to_string().into());
        let delta = |audio: &[u8]| {
            let audio = base64::engine::general_purpose::STANDARD.encode(audio);
            say(json!({"type": "response.output_audio.delta", "delta": audio}))
        };
        let _ = socket.send(say(json!({"type": "session.created"}))).await;
        while let Some(Ok(Ws::Text(frame))) = socket.recv().await {
            let event: Value = serde_json::from_str(&frame).unwrap();
            seen.lock()
                .unwrap()
                .push(json!({"route": "event", "event": event}));
            match event["type"].as_str().unwrap() {
                "session.update" => {
                    let _ = socket.send(say(json!({"type": "session.updated"}))).await;
                }
                "response.create" => {
                    let text = event["response"]["input"][0]["content"][0]["text"]
                        .as_str()
                        .unwrap();
                    if text.contains("SLOW") {
                        loop {
                            tokio::select! {
                                frame = socket.recv() => if !matches!(frame, Some(Ok(Ws::Text(_)))) { break },
                                _ = tokio::time::sleep(Duration::from_millis(10)) => {
                                    if socket.send(delta(b"..")).await.is_err() { break }
                                }
                            }
                        }
                        break;
                    }
                    let (first, second) = text.as_bytes().split_at(text.len() / 2);
                    let _ = socket.send(delta(first)).await;
                    let _ = socket.send(delta(&[second, b"|"].concat())).await;
                    let done =
                        json!({"type": "response.done", "response": {"status": "completed"}});
                    let _ = socket.send(say(done)).await;
                }
                _ => {}
            }
        }
        seen.lock().unwrap().push(json!({"route": "closed"}));
    }

    /// Stored serves one chat whose second message is a reply with the given text and status.
    fn stored(dir: &std::path::Path, text: &str, status: &str, voice: Option<Voice>) -> Router {
        use crate::store::Message;
        let store = Store::open(&dir.join("covalent.db")).unwrap();
        store.create_chat("chat1", "gpt-6.1-sol", None, 1).unwrap();
        for (id, role, text, status) in [
            ("ask", "user", "Tell me.", "complete"),
            ("reply", "assistant", text, status),
        ] {
            let message = Message {
                id: id.into(),
                chat_id: "chat1".into(),
                role: role.into(),
                text: text.into(),
                status: status.into(),
                reason: None,
                created_at_ms: 1,
                sources: vec![],
                images: vec![],
                activity: vec![],
                attachments: vec![],
            };
            store.insert_message(&message).unwrap();
        }
        router(Chats::new(store, launch(dir)), voice, &dir.join("web"))
    }

    async fn upload(app: &Router, media_type: &str, audio: &[u8]) -> (StatusCode, String) {
        let request = Request::post("/api/transcribe")
            .header("content-type", media_type)
            .body(Body::from(audio.to_vec()))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn listen(app: &Router, uri: &str) -> (StatusCode, String) {
        let response = app
            .clone()
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    fn spoken(seen: &std::sync::Mutex<Vec<Value>>) -> Vec<Value> {
        seen.lock()
            .unwrap()
            .iter()
            .filter(|r| r["event"]["type"] == "response.create")
            .map(|r| r["event"]["response"].clone())
            .collect()
    }

    /// Closed waits for the fake to see its Realtime session end.
    async fn closed(seen: &std::sync::Mutex<Vec<Value>>) {
        let end = Instant::now() + Duration::from_secs(5);
        while !seen.lock().unwrap().iter().any(|r| r["route"] == "closed") {
            assert!(Instant::now() < end, "the OpenAI session stayed open");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    const SPEECH: &str = "/api/chats/chat1/messages/reply/speech";

    #[tokio::test]
    async fn without_a_key_voice_is_off_and_its_routes_say_why() {
        let dir = tempfile::tempdir().unwrap();
        let app = stored(dir.path(), "Hello.", "complete", None);
        let (_, body) = call(&app, "GET", "/api/voice", Value::Null).await;
        assert_eq!(body, json!({"available": false}));
        let (status, body) = upload(&app, "audio/webm", b"FAKE-AUDIO").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(
            body.contains("voice_unavailable") && body.contains("OPENAI_API_KEY"),
            "{body}"
        );
        let (status, _) = listen(&app, SPEECH).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn a_recording_reaches_the_transcription_model_and_returns_its_words() {
        let (base, seen) = openai_fake(false).await;
        let dir = tempfile::tempdir().unwrap();
        let app = stored(
            dir.path(),
            "Hello.",
            "complete",
            Some(Voice::new("voice-test-key".into(), base)),
        );
        let (_, body) = call(&app, "GET", "/api/voice", Value::Null).await;
        assert_eq!(body, json!({"available": true}));

        let (status, body) = upload(&app, "audio/webm;codecs=opus", b"FAKE-AUDIO").await;
        assert_eq!(
            (status, body.as_str()),
            (StatusCode::OK, r#"{"text":"Hello there."}"#)
        );
        assert_eq!(
            seen.lock().unwrap()[0],
            json!({"route": "transcription", "authorization": "Bearer voice-test-key",
                   "model": "gpt-transcribe", "filename": "audio.webm", "audio": true})
        );
        let (status, body) = upload(&app, "text/plain", b"FAKE-AUDIO").await;
        assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE, "{body}");
        let (status, body) = upload(&app, "audio/mp4", b"").await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        let (status, _) = upload(&app, "audio/wav", &vec![0; 11 * 1024 * 1024]).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(
            seen.lock().unwrap().len(),
            1,
            "only the good recording reached OpenAI"
        );
    }

    #[tokio::test]
    async fn a_stored_reply_is_read_aloud_as_plain_sentences() {
        let (base, seen) = openai_fake(false).await;
        let dir = tempfile::tempdir().unwrap();
        let text = "## Rising\nSourdough needs **time**. Keep it warm ([bread.example](https://bread.example/x?utm_source=openai)).\n```\nmix()\n```\nBake at noon?";
        let app = stored(
            dir.path(),
            text,
            "complete",
            Some(Voice::new("voice-test-key".into(), base)),
        );
        let (status, audio) = listen(&app, SPEECH).await;
        assert_eq!(status, StatusCode::OK, "{audio}");
        let fake = seen;
        assert_eq!(audio, "Rising. Sourdough needs time. Keep it warm. The code is shown on screen. Bake at noon?|");

        let seen = fake.lock().unwrap().clone();
        assert_eq!(seen[0]["authorization"], "Bearer voice-test-key");
        assert_eq!(seen[0]["query"], "model=gpt-realtime-2.1-mini");
        let session = &seen[1]["event"]["session"];
        assert_eq!(
            session["audio"]["output"],
            json!({"format": {"type": "audio/pcm", "rate": 24000}, "voice": "marin"})
        );
        assert!(session["audio"]["input"]["turn_detection"].is_null());
        let response = &seen[2]["event"]["response"];
        assert_eq!(response["conversation"], "none");
        assert!(response["instructions"]
            .as_str()
            .unwrap()
            .contains("exactly as written"));
        drop(seen);
        closed(&fake).await;
    }

    #[tokio::test]
    async fn only_finished_replies_of_the_chat_can_be_read() {
        let (base, seen) = openai_fake(false).await;
        let dir = tempfile::tempdir().unwrap();
        let app = stored(
            dir.path(),
            "Half a rep",
            "cancelled",
            Some(Voice::new("k".into(), base)),
        );
        let (status, body) = listen(&app, SPEECH).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        for uri in [
            "/api/chats/chat1/messages/ask/speech",
            "/api/chats/other/messages/reply/speech",
            "/api/chats/chat1/messages/none/speech",
        ] {
            let (status, _) = listen(&app, uri).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        }
        assert!(seen.lock().unwrap().is_empty(), "nothing reached OpenAI");
    }

    #[tokio::test]
    async fn closing_a_speech_request_closes_the_openai_session() {
        let (base, seen) = openai_fake(false).await;
        let dir = tempfile::tempdir().unwrap();
        let app = stored(
            dir.path(),
            "A SLOW reply.",
            "complete",
            Some(Voice::new("k".into(), base)),
        );
        let response = app
            .clone()
            .oneshot(Request::get(SPEECH).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let mut body = response.into_body();
        let first = body.frame().await.unwrap().unwrap().into_data().unwrap();
        assert_eq!(&first[..], b"..");
        drop(body);
        closed(&seen).await;
    }

    #[tokio::test]
    async fn a_rejected_key_is_reported_without_repeating_it() {
        let (base, _) = openai_fake(true).await;
        let dir = tempfile::tempdir().unwrap();
        let key = "sk-test-SECRET-0123456789";
        let app = stored(
            dir.path(),
            "Hello.",
            "complete",
            Some(Voice::new(key.into(), base)),
        );
        let (status, body) = upload(&app, "audio/webm", b"FAKE-AUDIO").await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(
            body.contains("OpenAI rejected OPENAI_API_KEY") && !body.contains("SECRET"),
            "{body}"
        );
        let (status, body) = listen(&app, SPEECH).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(
            body.contains("OpenAI rejected OPENAI_API_KEY") && !body.contains("SECRET"),
            "{body}"
        );
    }

    #[tokio::test]
    async fn a_pending_reply_is_read_aloud_as_it_is_written() {
        let release = tempfile::tempdir().unwrap();
        let release_file = release.path().join("release");
        let env = [
            ("OPENAI_API_KEY", "covalent-test-key".to_owned()),
            (
                "REAGENT_OPENAI_BASE_URL",
                stream_provider(release_file.clone()),
            ),
            ("COVALENT_TEST_RELEASE", release_file.display().to_string()),
        ];
        if !isolated(
            "http::tests::a_pending_reply_is_read_aloud_as_it_is_written",
            &env,
        ) {
            return;
        }
        let release = PathBuf::from(std::env::var("COVALENT_TEST_RELEASE").unwrap());
        std::fs::write(release.with_file_name("searched"), b"").unwrap();
        let (base, seen) = openai_fake(false).await;
        let dir = tempfile::tempdir().unwrap();
        let launch = Launch {
            model: Some("gpt-6.1-sol".into()),
            effort: Some("low".into()),
            ..launch(dir.path())
        };
        let chats = Chats::new(
            Store::open(&dir.path().join("covalent.db")).unwrap(),
            launch,
        );
        let app = router(
            chats,
            Some(Voice::new("k".into(), base)),
            &dir.path().join("web"),
        );
        let (_, created) = call(&app, "POST", "/api/chats", Value::Null).await;
        let id = created["chat"]["id"].as_str().unwrap().to_owned();
        let send = json!({"request_id": "r1", "text": "Tell me about bread."});
        let (_, sent) = call(&app, "POST", &format!("/api/chats/{id}/messages"), send).await;
        let reply = sent["messages"][1]["id"].as_str().unwrap().to_owned();
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            let (_, body) = call(&app, "GET", &format!("/api/chats/{id}"), Value::Null).await;
            if body["previews"][&reply]["text"] == "Sourdough" {
                break;
            }
            assert!(Instant::now() < end, "no preview: {body}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let uri = format!("/api/chats/{id}/messages/{reply}/speech");
        let response = app
            .clone()
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let audio =
            tokio::spawn(async move { response.into_body().collect().await.unwrap().to_bytes() });
        std::fs::write(&release, b"").unwrap();
        let audio = tokio::time::timeout(Duration::from_secs(10), audio)
            .await
            .unwrap()
            .unwrap();
        // The unfinished sentence waited for the rest of the reply and was read once, whole.
        assert_eq!(&audio[..], b"Sourdough rises.|");
        assert_eq!(spoken(&seen).len(), 1);
    }

    #[tokio::test]
    async fn a_spoken_turn_asks_for_a_reply_meant_to_be_heard_and_a_typed_one_does_not() {
        let release = tempfile::tempdir().unwrap();
        let release_file = release.path().join("release");
        std::fs::write(&release_file, b"").unwrap();
        std::fs::write(release.path().join("searched"), b"").unwrap();
        let env = [
            ("OPENAI_API_KEY", "covalent-test-key".to_owned()),
            (
                "REAGENT_OPENAI_BASE_URL",
                stream_provider(release_file.clone()),
            ),
            ("COVALENT_TEST_RELEASE", release_file.display().to_string()),
        ];
        if !isolated(
            "http::tests::a_spoken_turn_asks_for_a_reply_meant_to_be_heard_and_a_typed_one_does_not",
            &env,
        ) {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let launch = Launch {
            model: Some("gpt-6.1-sol".into()),
            effort: Some("low".into()),
            ..launch(dir.path())
        };
        let chats = Chats::new(
            Store::open(&dir.path().join("covalent.db")).unwrap(),
            launch,
        );
        let app = router(chats, None, &dir.path().join("web"));
        let (_, created) = call(&app, "POST", "/api/chats", Value::Null).await;
        let id = created["chat"]["id"].as_str().unwrap().to_owned();
        let uri = format!("/api/chats/{id}/messages");
        let spoken = json!({"request_id": "r1", "text": "what rises", "spoken": true});
        call(&app, "POST", &uri, spoken).await;
        settled(&app, &id).await;
        call(
            &app,
            "POST",
            &uri,
            json!({"request_id": "r2", "text": "and typed"}),
        )
        .await;
        settled(&app, &id).await;

        let release = PathBuf::from(std::env::var("COVALENT_TEST_RELEASE").unwrap());
        let log = std::fs::read_to_string(release.with_file_name("requests.log")).unwrap();
        let inputs: Vec<Value> = log
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(inputs.len(), 2, "{log}");
        let note = "The user said this message aloud in voice mode";
        assert_eq!(
            inputs[0].to_string().matches(note).count(),
            1,
            "{}",
            inputs[0]
        );
        // The typed turn carries no note; only the earlier spoken one in history does.
        assert_eq!(
            inputs[1].to_string().matches(note).count(),
            1,
            "{}",
            inputs[1]
        );
        let last = inputs[1].as_array().unwrap().last().unwrap().to_string();
        assert!(last.contains("and typed") && !last.contains(note), "{last}");
        let (_, body) = call(&app, "GET", &format!("/api/chats/{id}"), Value::Null).await;
        assert_eq!(body["messages"][0]["text"], "what rises");
    }

    /// Fetch gets a path from the app with the given headers, returning the response's
    /// status, headers and body.
    async fn fetch(
        app: &Router,
        uri: &str,
        headers: &[(&str, &str)],
    ) -> (StatusCode, HeaderMap, Bytes) {
        let mut request = Request::get(uri);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let (parts, body) = response.into_parts();
        (
            parts.status,
            parts.headers,
            body.collect().await.unwrap().to_bytes(),
        )
    }

    /// Fake_chat opens a chat whose Session talks to the streaming fake provider, with images
    /// on, and returns the app, the chats, their data folder and the new chat's ID.
    async fn fake_chat(dir: &std::path::Path) -> (Router, Arc<Chats>, String) {
        let launch = Launch {
            model: Some("gpt-6.1-sol".into()),
            effort: Some("low".into()),
            images: true,
            ..launch(dir)
        };
        let chats = Chats::new(Store::open(&dir.join("covalent.db")).unwrap(), launch);
        let app = router(chats.clone(), None, &dir.join("web"));
        let (_, created) = call(&app, "POST", "/api/chats", Value::Null).await;
        let id = created["chat"]["id"].as_str().unwrap().to_owned();
        (app, chats, id)
    }

    #[tokio::test]
    async fn a_request_for_a_file_writes_it_in_the_chats_folder_and_lists_the_work() {
        let release = tempfile::tempdir().unwrap();
        let env = [
            ("OPENAI_API_KEY", "covalent-test-key".to_owned()),
            (
                "REAGENT_OPENAI_BASE_URL",
                stream_provider(release.path().join("release")),
            ),
        ];
        if !isolated(
            "http::tests::a_request_for_a_file_writes_it_in_the_chats_folder_and_lists_the_work",
            &env,
        ) {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let (app, chats, id) = fake_chat(dir.path()).await;
        let mut events = chats.subscribe();
        let send = json!({"request_id": "r1", "text": "make me a packing list"});
        call(&app, "POST", &format!("/api/chats/{id}/messages"), send).await;
        let body = settled(&app, &id).await;

        let reply = &body["messages"][1];
        assert_eq!(reply["text"], "Saved your list.", "{reply}");
        let row = json!({"call_id":"call_1","tool":"write_file","target":"list.md",
                         "state":"done","operation":"create","error":null});
        assert_eq!(reply["activity"], json!([row]));
        let mut announced = false;
        while let Ok(event) = events.try_recv() {
            let event = serde_json::to_value(event).unwrap();
            announced |= event["type"] == "activity" && event["activity"][0]["target"] == "list.md";
        }
        assert!(
            announced,
            "no event reported the call while the reply was pending"
        );

        let folder = dir
            .path()
            .join("workspaces")
            .join(body["chat"]["workspace"].as_str().unwrap());
        assert_eq!(std::fs::read(folder.join("list.md")).unwrap(), b"- socks\n");
        let (status, headers, bytes) =
            fetch(&app, &format!("/api/chats/{id}/files/list.md"), &[]).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(&bytes[..], b"- socks\n");
        // A file the model wrote is never shown as a page of Covalent's own.
        assert_eq!(headers[header::CONTENT_TYPE], "application/octet-stream");
        assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert!(headers[header::CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .starts_with("attachment"));

        // Nothing outside the chat's folder is served, by a path or through a link.
        std::os::unix::fs::symlink(dir.path().join("covalent.db"), folder.join("link")).unwrap();
        for path in [
            "..%2F..%2Fcovalent.db",
            "link",
            "missing.md",
            "%2Fetc%2Fhosts",
        ] {
            let (status, _, _) = fetch(&app, &format!("/api/chats/{id}/files/{path}"), &[]).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        }
    }

    #[tokio::test]
    async fn a_request_for_a_picture_makes_an_image_that_the_reply_shows_and_the_server_serves() {
        let release = tempfile::tempdir().unwrap();
        let env = [
            ("OPENAI_API_KEY", "covalent-test-key".to_owned()),
            (
                "REAGENT_OPENAI_BASE_URL",
                stream_provider(release.path().join("release")),
            ),
        ];
        if !isolated(
            "http::tests::a_request_for_a_picture_makes_an_image_that_the_reply_shows_and_the_server_serves",
            &env,
        ) {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let (app, chats, id) = fake_chat(dir.path()).await;
        let mut events = chats.subscribe();
        let send = json!({"request_id": "r1", "text": "draw me a boat"});
        call(&app, "POST", &format!("/api/chats/{id}/messages"), send).await;
        let body = settled(&app, &id).await;

        let reply = &body["messages"][1];
        assert_eq!(reply["status"], "complete", "{reply}");
        assert_eq!(reply["text"], "Here is your boat.");
        assert_eq!(
            reply["images"],
            json!([{"name": "boat.png", "width": 1, "height": 1}])
        );
        assert_eq!(reply["activity"][0]["tool"], "generate_image");
        let mut generating = false;
        while let Ok(event) = events.try_recv() {
            let event = serde_json::to_value(event).unwrap();
            generating |= event["type"] == "preview" && event["generating"] == true;
        }
        assert!(generating, "no preview said an image was being made");

        let folder = dir
            .path()
            .join("workspaces")
            .join(body["chat"]["workspace"].as_str().unwrap());
        let (status, headers, bytes) =
            fetch(&app, &format!("/api/chats/{id}/files/boat.png"), &[]).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[header::CONTENT_TYPE], "image/png");
        assert_eq!(
            bytes[..],
            std::fs::read(folder.join("boat.png")).unwrap()[..]
        );
    }

    #[tokio::test]
    async fn a_picture_made_before_chats_had_folders_is_served_only_when_recorded() {
        use crate::store::{Image, Message};
        let dir = tempfile::tempdir().unwrap();
        let images = dir.path().join("images");
        std::fs::create_dir(&images).unwrap();
        for name in ["image-ab12.png", "image-0000.png"] {
            std::fs::write(images.join(name), name).unwrap();
        }
        let store = Store::open(&dir.path().join("covalent.db")).unwrap();
        store.create_chat("old", "gpt-6.1-sol", None, 1).unwrap();
        let mut reply = Message {
            id: "reply".into(),
            chat_id: "old".into(),
            role: "assistant".into(),
            text: String::new(),
            status: "pending".into(),
            reason: None,
            created_at_ms: 1,
            sources: vec![],
            images: vec![],
            activity: vec![],
            attachments: vec![],
        };
        store.insert_message(&reply).unwrap();
        reply.status = "complete".into();
        reply.images = vec![Image {
            name: "image-ab12.png".into(),
            width: 1,
            height: 1,
        }];
        store.finish_message(&reply).unwrap();
        let app = router(
            Chats::new(store, launch(dir.path())),
            None,
            &dir.path().join("web"),
        );
        let (status, headers, bytes) = fetch(&app, "/api/images/image-ab12.png", &[]).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[header::CONTENT_TYPE], "image/png");
        assert_eq!(&bytes[..], b"image-ab12.png");
        // A file in the folder that no message records, and a name that is not an image's.
        for name in ["image-0000.png", "image-zz.png", "..%2Fcovalent.db"] {
            let (status, _, _) = fetch(&app, &format!("/api/images/{name}"), &[]).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{name}");
        }
        // An older chat has no folder to serve files from.
        let (status, _, _) = fetch(&app, "/api/chats/old/files/image-ab12.png", &[]).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_chats_files_are_listed_in_path_order_up_to_the_bound() {
        if !isolated(
            "http::tests::a_chats_files_are_listed_in_path_order_up_to_the_bound",
            &[],
        ) {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let (app, _) = app(dir.path(), &["unused"]);
        let (_, created) = call(&app, "POST", "/api/chats", Value::Null).await;
        let id = created["chat"]["id"].as_str().unwrap();
        let folder = dir
            .path()
            .join("workspaces")
            .join(created["chat"]["workspace"].as_str().unwrap());
        let (status, empty) =
            call(&app, "GET", &format!("/api/chats/{id}/files"), Value::Null).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(empty, json!({"files": [], "more": false}));

        std::fs::create_dir(folder.join("notes")).unwrap();
        std::fs::write(folder.join("notes/b.md"), "bee").unwrap();
        std::fs::write(folder.join("a.txt"), "a").unwrap();
        std::os::unix::fs::symlink(dir.path().join("covalent.db"), folder.join("link")).unwrap();
        let (_, listed) = call(&app, "GET", &format!("/api/chats/{id}/files"), Value::Null).await;
        let files: Vec<_> = listed["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| (f["path"].as_str().unwrap(), f["size"].as_u64().unwrap()))
            .collect();
        assert_eq!(files, [("a.txt", 1), ("notes/b.md", 3)]);
        assert!(listed["files"][0]["modified_at_ms"].as_i64().unwrap() > 0);

        for n in 0..crate::files::MAX_LISTED {
            std::fs::write(folder.join(format!("many-{n:03}.txt")), "").unwrap();
        }
        let (_, listed) = call(&app, "GET", &format!("/api/chats/{id}/files"), Value::Null).await;
        let files = listed["files"].as_array().unwrap();
        assert_eq!(files.len(), crate::files::MAX_LISTED);
        assert_eq!(listed["more"], true);
        assert_eq!(files[0]["path"], "a.txt");

        let (status, _) = call(&app, "GET", "/api/chats/missing/files", Value::Null).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    /// Put uploads bytes as a file and returns the response's status and JSON body.
    async fn put(app: &Router, uri: &str, bytes: Vec<u8>) -> (StatusCode, Value) {
        let request = Request::put(uri).body(Body::from(bytes)).unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    #[tokio::test]
    async fn uploads_land_in_the_folder_without_replacing_and_the_message_names_them() {
        let release = tempfile::tempdir().unwrap();
        let release_file = release.path().join("release");
        let env = [
            ("OPENAI_API_KEY", "covalent-test-key".to_owned()),
            (
                "REAGENT_OPENAI_BASE_URL",
                stream_provider(release_file.clone()),
            ),
            ("COVALENT_TEST_RELEASE", release_file.display().to_string()),
        ];
        if !isolated(
            "http::tests::uploads_land_in_the_folder_without_replacing_and_the_message_names_them",
            &env,
        ) {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let (app, _, id) = fake_chat(dir.path()).await;
        let files = format!("/api/chats/{id}/files");
        let (status, saved) = put(&app, &format!("{files}/notes.md"), b"hello".to_vec()).await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        assert_eq!(saved, json!({"path": "notes.md", "size": 5}));
        let (_, again) = put(&app, &format!("{files}/notes.md"), b"other".to_vec()).await;
        assert_eq!(again["path"], "notes-2.md");
        use base64::Engine;
        let png = base64::engine::general_purpose::STANDARD
            .decode(PIXEL)
            .unwrap();
        let (_, photo) = put(&app, &format!("{files}/photo.png"), png).await;
        assert_eq!(photo["path"], "photo.png");

        // Hidden names, paths and oversized bodies are refused, and nothing is replaced.
        for name in [".env", "a%2Fb.txt", "%2E%2E"] {
            let (status, _) = put(&app, &format!("{files}/{name}"), b"x".to_vec()).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{name}");
        }
        let big = vec![0; crate::files::MAX_UPLOAD + 1];
        let (status, _) = put(&app, &format!("{files}/big.bin"), big).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        let folder = dir.path().join("workspaces").join(
            call(&app, "GET", &format!("/api/chats/{id}"), Value::Null)
                .await
                .1["chat"]["workspace"]
                .as_str()
                .unwrap(),
        );
        assert_eq!(std::fs::read(folder.join("notes.md")).unwrap(), b"hello");
        assert!(!folder.join("big.bin").exists());

        // A message of files alone reaches the model with a note naming each one.
        let send =
            json!({"request_id": "r1", "text": "", "attachments": ["notes.md", "photo.png"]});
        let (status, sent) = call(&app, "POST", &format!("/api/chats/{id}/messages"), send).await;
        assert_eq!(status, StatusCode::OK, "{sent}");
        let body = settled(&app, &id).await;
        assert_eq!(
            body["messages"][0]["attachments"],
            json!([{"path": "notes.md", "size": 5}, {"path": "photo.png", "size": photo["size"]}])
        );
        assert_eq!(body["messages"][1]["text"], "Got your files.");
        let release = PathBuf::from(std::env::var("COVALENT_TEST_RELEASE").unwrap());
        let seen = std::fs::read_to_string(release.with_file_name("requests.log")).unwrap();
        assert!(seen.contains("notes.md (text, 5 B)"), "{seen}");
        assert!(seen.contains("photo.png (image, "), "{seen}");

        // Files outside the folder, or too many, are not attached.
        let many: Vec<_> = (0..11).map(|_| "notes.md").collect();
        for attachments in [
            json!(["../covalent.db"]),
            json!(["missing.md"]),
            json!(many),
        ] {
            let send = json!({"request_id": "r2", "text": "x", "attachments": attachments});
            let (status, _) = call(&app, "POST", &format!("/api/chats/{id}/messages"), send).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{attachments}");
        }
    }

    #[tokio::test]
    async fn requests_addressed_to_or_sent_from_another_site_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _) = app(dir.path(), &[]);
        for headers in [
            &[("host", "127.0.0.1:7420")][..],
            &[
                ("host", "localhost:5173"),
                ("origin", "http://localhost:5173"),
            ],
            &[("host", "[::1]:7420")],
            &[],
        ] {
            let (status, _, _) = fetch(&app, "/api/chats", headers).await;
            assert_eq!(status, StatusCode::OK, "{headers:?}");
        }
        for headers in [
            &[("host", "covalent.example:7420")][..],
            &[
                ("host", "127.0.0.1:7420"),
                ("origin", "https://elsewhere.example"),
            ],
            &[("host", "127.0.0.1:7420"), ("origin", "null")],
            &[("host", "localhost.example")],
        ] {
            let (status, _, _) = fetch(&app, "/api/chats", headers).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{headers:?}");
        }
    }
}
