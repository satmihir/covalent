use crate::chats::Launch;
use reagent::session_api::{open_session, PlanMode, SessionApi};

const LONGEST: usize = 60;

/// Generate asks a throwaway Session for a short title, falling back to the message's opening words.
pub fn generate(launch: &Launch, message: &str) -> String {
    match ask(launch, message) {
        Ok(title) => title,
        Err(reason) => {
            eprintln!("covalent: title fell back to the message: {reason}");
            fallback(message)
        }
    }
}

fn ask(launch: &Launch, message: &str) -> Result<String, String> {
    let api = open_session(launch.options(true, None)).map_err(|e| e.message)?;
    let result = request(&api, message);
    let _ = api.close();
    result
}

fn request(api: &SessionApi, message: &str) -> Result<String, String> {
    let excerpt: String = message.chars().take(2000).collect();
    let run = api
        .send_message("title", excerpt.as_bytes(), PlanMode::Keep)
        .map_err(|e| e.message)?
        .run
        .ok_or("title request admitted without a Run")?;
    let result = api
        .wait_run(&run)
        .map_err(|e| e.message)?
        .result
        .ok_or("title Run ended without a result")?;
    if result.status != "completed" {
        return Err(format!(
            "title Run ended with {}: {}",
            result.status, result.reason
        ));
    }
    clean(&String::from_utf8_lossy(&result.reply)).ok_or_else(|| "empty title".into())
}

/// Clean keeps the first line of a model reply, without decoration, bounded to a sidebar's width.
fn clean(reply: &str) -> Option<String> {
    let line = reply.lines().map(str::trim).find(|l| !l.is_empty())?;
    let decoration = |c: char| c.is_whitespace() || "\"'`*#“”‘’".contains(c);
    let line = line.trim_matches(decoration);
    let line = line
        .strip_prefix("Title:")
        .map_or(line, |rest| rest.trim_matches(decoration))
        .trim_end_matches(['.', '!', ':', ';', ',']);
    let title = shorten(line);
    (!title.is_empty()).then_some(title)
}

fn fallback(message: &str) -> String {
    let title = shorten(message);
    if title.is_empty() {
        "New chat".into()
    } else {
        title
    }
}

/// Shorten collapses whitespace and cuts at a word boundary within LONGEST characters.
fn shorten(text: &str) -> String {
    let mut out = String::new();
    for word in text.split_whitespace() {
        let extra = usize::from(!out.is_empty()) + word.chars().count();
        if out.chars().count() + extra > LONGEST {
            if out.is_empty() {
                out = word.chars().take(LONGEST - 1).collect();
                out.push('…');
            }
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_keeps_the_first_line_without_decoration() {
        for (reply, expected) in [
            ("Planning a Trip to Kyoto", Some("Planning a Trip to Kyoto")),
            (
                "\"Rust Lifetimes Explained.\"\n",
                Some("Rust Lifetimes Explained"),
            ),
            (
                "\n\n**Title:** Sourdough Basics\nmore text",
                Some("Sourdough Basics"),
            ),
            ("   \n  ", None),
            ("\"\"", None),
        ] {
            assert_eq!(clean(reply).as_deref(), expected, "{reply:?}");
        }
    }

    #[test]
    fn long_text_is_cut_at_a_word_boundary() {
        let title = shorten(&"word ".repeat(40));
        assert!(title.chars().count() <= LONGEST);
        assert!(title.ends_with("word"));
        let single = shorten(&"x".repeat(100));
        assert_eq!(single.chars().count(), LONGEST);
        assert!(single.ends_with('…'));
    }

    #[test]
    fn fallback_uses_the_message_and_never_returns_empty() {
        assert_eq!(fallback("  how do   I bake bread  "), "how do I bake bread");
        assert_eq!(fallback("   "), "New chat");
    }
}
