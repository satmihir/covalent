use crate::chats::{Chats, Failure};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::convert::Infallible;
use std::sync::Arc;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};
use tower_http::services::{ServeDir, ServeFile};

/// Router serves the JSON API under `/api` and the built frontend from `web` for every other path.
pub fn router(chats: Arc<Chats>, web: &std::path::Path) -> Router {
    let frontend = ServeDir::new(web).fallback(ServeFile::new(web.join("index.html")));
    Router::new()
        .route("/api/models", get(models))
        .route("/api/chats", get(list).post(create))
        .route("/api/chats/{id}", get(chat))
        .route("/api/chats/{id}/messages", post(send))
        .route("/api/chats/{id}/cancel", post(cancel))
        .route("/api/chats/{id}/model", post(switch_model))
        .route("/api/chats/{id}/archive", post(archive))
        .route("/api/events", get(events))
        .fallback_service(frontend)
        .with_state(chats)
}

#[derive(Deserialize)]
struct SendBody {
    request_id: String,
    text: String,
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
        let messages = chats.send(&id, &body.request_id, &body.text)?;
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

/// Events streams every chat's changes; a client that falls behind reloads what it shows.
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

    fn app(dir: &std::path::Path, replies: &[&str]) -> (Router, Arc<Chats>) {
        let script = dir.join("script.json");
        let responses: Vec<Value> = replies
            .iter()
            .map(|text| json!({"blocks": [{"kind": "text", "text": text}]}))
            .collect();
        std::fs::write(&script, serde_json::to_vec(&responses).unwrap()).unwrap();
        let launch = Launch {
            scripted: Some(script),
            ..Launch::default()
        };
        let chats = Chats::new(Store::open(&dir.join("covalent.db")).unwrap(), launch);
        (router(chats.clone(), &dir.join("web")), chats)
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

    /// Stream_provider answers Responses requests on a loopback port: title requests at once,
    /// and the chat reply as "Sourdough", then " rises." once the release file exists.
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
            ..Launch::default()
        };
        let chats = Chats::new(
            Store::open(&dir.path().join("covalent.db")).unwrap(),
            launch,
        );
        let app = router(chats.clone(), &dir.path().join("web"));
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

        let end = Instant::now() + Duration::from_secs(10);
        loop {
            let (_, body) = call(&app, "GET", &format!("/api/chats/{id}"), Value::Null).await;
            if body["previews"][&reply]["text"] == "Sourdough" {
                assert_eq!(body["messages"][1]["status"], "pending");
                // Search start and end were previews 1 and 2.
                assert_eq!(body["previews"][&reply]["seq"], 3);
                assert_eq!(body["previews"][&reply]["searching"], false);
                break;
            }
            assert!(Instant::now() < end, "no preview: {body}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        std::fs::write(std::env::var("COVALENT_TEST_RELEASE").unwrap(), b"").unwrap();
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
            Launch::default(),
        );
        let app = router(chats, &dir.path().join("web"));
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
}
