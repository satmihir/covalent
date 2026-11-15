//! Speakable turns a reply's Markdown, as it grows, into plain sentences to read aloud.

use std::collections::VecDeque;

/// PIECE is roughly how many characters one spoken request carries once a reply is flowing.
const PIECE: usize = 600;

/// CODE and TABLE stand in for what cannot be read aloud.
const CODE: &str = "The code is shown on screen.";
const TABLE: &str = "The table is shown on screen.";

/// Cutter remembers how much of a reply has become speech, so it can be fed the whole reply
/// each time it grows.
#[derive(Debug, Default)]
pub struct Cutter {
    /// seen is the reply text already cut; later text must extend it.
    seen: String,
    /// mid_line means `seen` stops after a sentence inside a line, not at a line start.
    mid_line: bool,
    code: bool,
    table: bool,
    ready: VecDeque<String>,
}

impl Cutter {
    /// Feed cuts every sentence of `text` that is complete, or all of it when `done`.
    /// It returns false when `text` does not extend what was already cut, as when the model
    /// started its reply over; nothing is cut then.
    pub fn feed(&mut self, text: &str, done: bool) -> bool {
        let Some(rest) = text.strip_prefix(self.seen.as_str()) else {
            return false;
        };
        let mut rest = rest;
        while !rest.is_empty() {
            let (line, complete, used) = match rest.find('\n') {
                Some(end) => (&rest[..end], true, end + 1),
                None if done => (rest, true, rest.len()),
                None => match self.partial(rest) {
                    Some(end) => (&rest[..end], false, end),
                    None => break,
                },
            };
            self.line(line, complete);
            self.seen.push_str(&rest[..used]);
            rest = &rest[used..];
        }
        true
    }

    /// Take removes the next piece to speak: at least one sentence, and more while they fit.
    pub fn take(&mut self) -> Option<String> {
        let mut piece = self.ready.pop_front()?;
        while let Some(next) = self.ready.front() {
            if piece.len() + 1 + next.len() > PIECE {
                break;
            }
            piece.push(' ');
            piece.push_str(next);
            self.ready.pop_front();
        }
        Some(piece)
    }

    /// Partial finds where an unfinished line can be cut: after its last complete sentence,
    /// and only in prose, since a fence, table row or heading must be seen whole.
    fn partial(&self, rest: &str) -> Option<usize> {
        let start = rest.trim_start();
        let held = !self.mid_line && (start.is_empty() || start.starts_with(['`', '~', '|', '#']));
        if self.code || held {
            return None;
        }
        sentence_ends(rest).last()
    }

    fn line(&mut self, line: &str, complete: bool) {
        let continuing = std::mem::replace(&mut self.mid_line, !complete);
        if continuing {
            self.say(line, complete);
            return;
        }
        let start = line.trim_start();
        if start.starts_with("```") || start.starts_with("~~~") {
            self.code = !self.code;
            if self.code {
                self.ready.push_back(CODE.into());
            }
            return;
        }
        if self.code {
            return;
        }
        if start.starts_with('|') {
            if !self.table {
                self.ready.push_back(TABLE.into());
            }
            self.table = true;
            return;
        }
        self.table = false;
        let rule = start.len() >= 3 && start.chars().all(|c| matches!(c, '-' | '*' | '_' | ' '));
        if rule {
            return;
        }
        self.say(strip_marker(start), complete);
    }

    /// Say adds a line's sentences; a complete line ends with punctuation so it is read
    /// as the end of a thought, as headings and list items are.
    fn say(&mut self, text: &str, complete: bool) {
        let mut spoken = plain(text);
        if spoken.is_empty() {
            return;
        }
        if complete && spoken.ends_with(|c: char| c.is_alphanumeric() || c == ')') {
            spoken.push('.');
        }
        let mut start = 0;
        for end in sentence_ends(&spoken).chain([spoken.len()]) {
            let sentence = spoken[start..end].trim();
            if !sentence.is_empty() {
                self.ready.push_back(sentence.into());
            }
            start = end;
        }
    }
}

/// Sentence_ends yields the byte offsets just after each sentence end that is followed by a
/// space: the punctuation and any closing quotes, brackets or emphasis markers.
fn sentence_ends(text: &str) -> impl Iterator<Item = usize> + '_ {
    let bytes = text.as_bytes();
    (0..bytes.len()).filter_map(move |i| {
        if !matches!(bytes[i], b'.' | b'!' | b'?') {
            return None;
        }
        let mut end = i + 1;
        while end < bytes.len() && matches!(bytes[end], b')' | b'"' | b'\'' | b'*' | b'_' | b']') {
            end += 1;
        }
        let closing = text[end..].starts_with(['”', '’']);
        if closing {
            end += 3;
        }
        (end < bytes.len() && bytes[end] == b' ').then_some(end)
    })
}

/// Strip_marker drops a heading, quote or list marker from the start of a line.
fn strip_marker(line: &str) -> &str {
    let mut line = line.trim_start_matches('>').trim_start();
    line = line.trim_start_matches('#').trim_start();
    for bullet in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(bullet) {
            return rest.trim_start();
        }
    }
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    let numbered = digits > 0 && matches!(line[digits..].as_bytes(), [b'.' | b')', b' ', ..]);
    if numbered {
        return line[digits + 2..].trim_start();
    }
    line
}

/// Plain reduces inline Markdown to words: a link keeps its text, a citation in parentheses
/// and a bare URL's path are dropped, and emphasis and code markers go.
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if let Some((words, used)) = link(rest) {
            let cited = out.ends_with('(') && rest[used..].starts_with(')');
            if cited {
                out.pop();
                out.truncate(out.trim_end().len());
                rest = &rest[used + 1..];
                continue;
            }
            out.push_str(&plain(words));
            rest = &rest[used..];
            continue;
        }
        if rest.starts_with("https://") || rest.starts_with("http://") {
            let end = rest
                .find(|c: char| c.is_whitespace() || matches!(c, ')' | ']' | '>'))
                .unwrap_or(rest.len());
            let url = rest[..end].trim_end_matches(['.', ',', ';', ':', '!', '?']);
            out.push_str(host(url));
            rest = &rest[url.len()..];
            continue;
        }
        let before = out.chars().next_back();
        let after = rest[c.len_utf8()..].chars().next();
        let inside_word =
            before.is_some_and(char::is_alphanumeric) && after.is_some_and(char::is_alphanumeric);
        let marker = matches!(c, '`' | '~' | '<' | '>') || (matches!(c, '*' | '_') && !inside_word);
        if !marker {
            out.push(c);
        }
        rest = &rest[c.len_utf8()..];
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Link parses `[words](target)` or `![words](target)` at the start of text, returning the
/// words and the bytes it spans.
fn link(text: &str) -> Option<(&str, usize)> {
    let skip = usize::from(text.starts_with('!'));
    let body = text[skip..].strip_prefix('[')?;
    let close = body.find("](")?;
    if body[..close].contains(['[', '\n']) {
        return None;
    }
    let target = &body[close + 2..];
    let end = target.find(')')?;
    if target[..end].contains(char::is_whitespace) {
        return None;
    }
    Some((&body[..close], skip + 1 + close + 2 + end + 1))
}

/// Host keeps a URL's host name without `www.`, which is how a person would say it.
fn host(url: &str) -> &str {
    let after = url.split_once("://").map_or(url, |(_, rest)| rest);
    let host = after.split(['/', '?', '#']).next().unwrap_or(after);
    host.strip_prefix("www.").unwrap_or(host)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pieces(text: &str) -> Vec<String> {
        let mut cutter = Cutter::default();
        assert!(cutter.feed(text, true));
        std::iter::from_fn(|| cutter.take()).collect()
    }

    #[test]
    fn markdown_becomes_words_a_person_would_say() {
        let cases = [
            ("plain prose", "It rose overnight. Bake it at noon!", "It rose overnight. Bake it at noon!"),
            ("emphasis and code", "Use **bold**, _italic_ and `cargo fmt`.", "Use bold, italic and cargo fmt."),
            ("snake case keeps its underscore", "Set max_tokens to 5.", "Set max_tokens to 5."),
            ("a link keeps its words", "Read [the guide](https://a.example/g) first.", "Read the guide first."),
            (
                "a citation is dropped",
                "It shipped Thursday ([blog.rust-lang.org](https://blog.rust-lang.org/x?utm_source=openai)).",
                "It shipped Thursday.",
            ),
            ("a bare URL becomes its host", "Visit https://www.example.com/pay today.", "Visit example.com today."),
            ("a heading ends a thought", "## Getting started\nOpen the app.", "Getting started. Open the app."),
            ("list markers go", "- two cups of flour\n2. a pinch of salt", "two cups of flour. a pinch of salt."),
            ("a quote marker goes", "> Measure twice.", "Measure twice."),
            ("a rule is silent", "Above.\n\n---\n\nBelow.", "Above. Below."),
            ("code is announced", "Run this:\n```sh\nmake check\n```\nThen wait.", "Run this: The code is shown on screen. Then wait."),
            ("a table is announced once", "Compare:\n| a | b |\n|---|---|\n| 1 | 2 |\nDone.", "Compare: The table is shown on screen. Done."),
        ];
        for (name, text, want) in cases {
            assert_eq!(pieces(text).join(" "), want, "{name}");
        }
    }

    #[test]
    fn sentences_are_cut_as_they_complete_and_never_twice() {
        let mut cutter = Cutter::default();
        assert!(cutter.feed("First one. Sec", false));
        assert_eq!(cutter.take().as_deref(), Some("First one."));
        assert_eq!(cutter.take(), None);
        // A line still unfinished stays unsaid until it ends or the reply is done.
        assert!(cutter.feed("First one. Second one is here", false));
        assert_eq!(cutter.take(), None);
        assert!(cutter.feed("First one. Second one is here.\n- Item", false));
        assert_eq!(cutter.take().as_deref(), Some("Second one is here."));
        assert!(cutter.feed("First one. Second one is here.\n- Item", true));
        assert_eq!(cutter.take().as_deref(), Some("Item."));
    }

    #[test]
    fn a_code_block_is_not_read_while_it_arrives() {
        let mut cutter = Cutter::default();
        assert!(cutter.feed("Try:\n```py\nprint('a. b. c. d')", false));
        assert_eq!(
            cutter.take().as_deref(),
            Some("Try: The code is shown on screen.")
        );
        assert!(cutter.feed(
            "Try:\n```py\nprint('a. b. c. d')\n```\nAfter that. More",
            false
        ));
        assert_eq!(cutter.take().as_deref(), Some("After that."));
    }

    #[test]
    fn text_that_no_longer_extends_the_cut_is_refused() {
        let mut cutter = Cutter::default();
        assert!(cutter.feed("Hello there. And", false));
        assert!(!cutter.feed("Hi", false));
        assert!(!cutter.feed("Hello thereX", true));
        assert!(cutter.feed("Hello there. And more.", true));
    }

    #[test]
    fn pieces_join_sentences_up_to_the_limit() {
        let sentence = format!("{}.", "word ".repeat(40).trim_end());
        let text = vec![sentence.as_str(); 8].join(" ");
        let pieces = pieces(&text);
        assert!(pieces.len() > 1, "{pieces:?}");
        assert!(pieces.iter().all(|p| p.len() <= PIECE), "{pieces:?}");
        assert_eq!(pieces.join(" "), text);
    }
}
