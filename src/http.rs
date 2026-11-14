use crate::chats::{Chats, Failure};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::convert::Infallible;
use std::sync::Arc;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};
use tower_http::services::{ServeDir, ServeFile};

/// Router serves the JSON API under `/api` and the built frontend from `web` for every other path.
pub fn router(chats: Arc<Chats>, web: &std::path::Path) -> Router {
    let frontend = ServeDir::new(web).fallback(ServeFile::new(web.join("index.html")));
    Router::new()
        .route("/api/chats", get(list).post(create))
        .route("/api/chats/{id}", get(chat))
        .route("/api/chats/{id}/messages", post(send))
        .route("/api/chats/{id}/cancel", post(cancel))
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

async fn create(State(chats): State<Arc<Chats>>) -> impl IntoResponse {
    blocking(move || Ok(json!({"chat": chats.create()?}))).await
}

async fn chat(State(chats): State<Arc<Chats>>, Path(id): Path<String>) -> impl IntoResponse {
    blocking(move || {
        let (chat, messages) = chats.get(&id)?;
        Ok(json!({"chat": chat, "messages": messages}))
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
    use serde_json::Value;
    use std::time::{Duration, Instant};
    use tower::ServiceExt;

    const CHILD: &str = "COVALENT_TEST_CHILD";

    /// Isolated reruns a test in a child process whose HOME is a temporary directory,
    /// because re:agent writes traces below the user's cache directory.
    fn isolated(name: &str) -> bool {
        if std::env::var_os(CHILD).is_some() {
            return true;
        }
        let home = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name, "--nocapture"])
            .env(CHILD, "1")
            .env("HOME", home.path())
            .env("XDG_CACHE_HOME", home.path().join("cache"))
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
        if !isolated("http::tests::a_new_chat_records_the_reply_and_a_title") {
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
            .map(|m| (m["role"].as_str().unwrap(), m["status"].as_str().unwrap()))
            .collect();
        assert_eq!(roles, [("user", "complete"), ("assistant", "pending")]);

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
        if !isolated("http::tests::a_retried_send_adds_no_messages") {
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
        if !isolated("http::tests::requests_for_unknown_chats_and_empty_messages_are_rejected") {
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
}
