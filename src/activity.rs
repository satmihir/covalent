use reagent::session_api::{BlockView, ContentBytes, EntryContent};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Action is one tool call a reply made, as the thread lists it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Action {
    /// call_id ties a call to its outcome.
    pub call_id: String,
    pub tool: String,
    /// target is the path or search text the call names, when it names one.
    pub target: Option<String>,
    /// state is `working`, `done`, `failed`, or `stopped` when the reply ended first.
    pub state: String,
    /// operation is what a file tool reported doing, such as `create` or `update`.
    pub operation: Option<String>,
    /// error is a failed call's message.
    pub error: Option<String>,
}

/// Record adds a Run's new tool calls, and the outcomes of earlier ones, from one Entry.
/// It reports whether anything changed; an Entry seen twice changes nothing.
pub fn record(activity: &mut Vec<Action>, content: &EntryContent) -> bool {
    match content {
        EntryContent::Assistant { blocks, .. } => {
            let before = activity.len();
            for block in blocks {
                let BlockView::ToolCall {
                    call_id,
                    name,
                    arguments,
                } = block
                else {
                    continue;
                };
                let call_id = text(call_id);
                if activity.iter().any(|a| a.call_id == call_id) {
                    continue;
                }
                let tool = text(name);
                activity.push(Action {
                    target: target(&tool, &arguments.bytes),
                    call_id,
                    tool,
                    state: "working".into(),
                    operation: None,
                    error: None,
                });
            }
            activity.len() > before
        }
        EntryContent::Tool {
            call_id, outcome, ..
        } => {
            let call_id = text(call_id);
            let Some(action) = activity
                .iter_mut()
                .find(|a| a.call_id == call_id && a.state == "working")
            else {
                return false;
            };
            // A file read's data holds the file and is cut short; only small results parse.
            let data: Value = serde_json::from_slice(&outcome.data.bytes).unwrap_or_default();
            action.state = if outcome.ok { "done" } else { "failed" }.into();
            action.operation = data["operation"].as_str().map(str::to_owned);
            if action.target.is_none() {
                action.target = data["path"].as_str().map(str::to_owned);
            }
            if !outcome.ok {
                action.error = Some(text(&outcome.message));
            }
            true
        }
        _ => false,
    }
}

/// Stop marks calls that never reported an outcome, because the reply ended first.
pub fn stop(activity: &mut [Action]) {
    for action in activity.iter_mut().filter(|a| a.state == "working") {
        action.state = "stopped".into();
    }
}

fn text(bytes: &ContentBytes) -> String {
    String::from_utf8_lossy(&bytes.bytes).into_owned()
}

/// Target reads the argument a row names: a search's text, an image's path, or a file's path.
fn target(tool: &str, arguments: &[u8]) -> Option<String> {
    let key = match tool {
        "search_text" => "query",
        "generate_image" => "output_path",
        _ => "path",
    };
    if let Ok(arguments) = serde_json::from_slice::<Value>(arguments) {
        return arguments[key].as_str().map(str::to_owned);
    }
    // Long arguments, such as a whole file's text, arrive cut short; the path usually comes first.
    let arguments = String::from_utf8_lossy(arguments);
    let start = arguments.find(&format!("\"{key}\":"))? + key.len() + 3;
    let mut value = serde_json::Deserializer::from_str(arguments[start..].trim_start());
    String::deserialize(&mut value).ok()
}
