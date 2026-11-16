mod activity;
mod chats;
mod files;
mod http;
mod speakable;
mod speech;
mod store;
mod title;

use std::path::PathBuf;

#[tokio::main]
async fn main() {
    if let Err(error) = serve().await {
        eprintln!("covalent: {error}");
        std::process::exit(1);
    }
}

async fn serve() -> Result<(), String> {
    let data = match std::env::var_os("COVALENT_DATA_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(std::env::var_os("HOME").ok_or("HOME is not set")?).join(".covalent"),
    };
    std::fs::create_dir_all(&data).map_err(|e| format!("create {}: {e}", data.display()))?;
    let store =
        store::Store::open(&data.join("covalent.db")).map_err(|e| format!("open store: {e}"))?;
    let env = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    let launch = chats::Launch {
        model: env("COVALENT_MODEL"),
        effort: env("COVALENT_EFFORT"),
        title_model: env("COVALENT_TITLE_MODEL"),
        title_effort: env("COVALENT_TITLE_EFFORT"),
        scripted: None,
        data: data.clone(),
        // Images are made with the OpenAI key, as voice is; without it chats make none.
        images: env("OPENAI_API_KEY").is_some(),
    };
    let web = env("COVALENT_WEB_DIR").map_or_else(|| PathBuf::from("web/dist"), PathBuf::from);
    let port = env("COVALENT_PORT").unwrap_or_else(|| "7420".into());
    // Loopback only: re:agent's Session API is trusted-local, with no authorization of its own.
    let address = format!("127.0.0.1:{port}");
    let listener = tokio::net::TcpListener::bind(&address)
        .await
        .map_err(|e| format!("listen on {address}: {e}"))?;
    println!("covalent: serving http://{address}");
    let voice = speech::Voice::from_env();
    if voice.is_none() {
        println!("covalent: voice is off; set OPENAI_API_KEY to turn it on");
    }
    // Extra names to answer to, such as the Tailscale name a phone reaches this Mac by.
    let hosts: Vec<String> = env("COVALENT_HOSTS")
        .map(|v| {
            v.split([',', ' '])
                .filter(|h| !h.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let app = http::router(chats::Chats::new(store, launch), voice, &web, &hosts);
    // No graceful shutdown: open event streams never finish, and re:agent has already saved
    // every accepted step, so Ctrl-C ending the process loses nothing a restart cannot report.
    axum::serve(listener, app)
        .await
        .map_err(|e| format!("serve: {e}"))
}
