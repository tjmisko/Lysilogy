//! Server-sent-event parsing for streamed chat completions.
//!
//! The accumulator is fed raw body bytes as they arrive, so it can be tested
//! without a network. It rebuilds the same [`ParsedCompletion`] a blocking
//! response carries and reports how far the model has got along the way.

use serde_json::Value;

use super::{Billing, ParsedCompletion, message_reasoning, truncate};
use crate::{Error, Result};

/// How far a streamed answer has got.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StreamProgress {
    /// The substage the call belongs to.
    pub stage: String,
    pub content_chars: usize,
    pub reasoning_chars: usize,
    /// How many top-level keys of the JSON answer have been started.
    pub top_level_keys: usize,
    /// The most recent top-level key.
    pub last_key: Option<String>,
}

#[derive(Default)]
pub(super) struct StreamAccumulator {
    buffer: Vec<u8>,
    content: String,
    reasoning: String,
    provider: Option<String>,
    finish_reason: Option<String>,
    usage: Option<Value>,
    error: Option<String>,
    keys: KeyScanner,
}

impl StreamAccumulator {
    /// Take the next bytes of the body; true when they completed a data event.
    /// Comment lines (keep-alives) and blank lines are not data events.
    pub(super) fn feed(&mut self, bytes: &[u8]) -> Result<bool> {
        self.buffer.extend_from_slice(bytes);
        let mut received = false;
        while let Some(end) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line);
            let Some(payload) = line.trim_end_matches(['\r', '\n']).strip_prefix("data:") else {
                continue;
            };
            let payload = payload.trim();
            if payload == "[DONE]" {
                received = true;
                continue;
            }
            let event: Value = serde_json::from_str(payload).map_err(|error| {
                Error::Task(format!(
                    "OpenRouter stream sent an unreadable event: {error}: {}",
                    truncate(payload)
                ))
            })?;
            self.apply(&event);
            received = true;
        }
        Ok(received)
    }

    fn apply(&mut self, event: &Value) {
        if let Some(error) = event.get("error").filter(|error| !error.is_null()) {
            self.error = Some(truncate(&error.to_string()));
        }
        if let Some(provider) = event.get("provider").and_then(Value::as_str) {
            self.provider = Some(provider.to_owned());
        }
        if let Some(usage) = event.get("usage").filter(|usage| !usage.is_null()) {
            self.usage = Some(usage.clone());
        }
        let choice = event.get("choices").and_then(|choices| choices.get(0));
        if let Some(reason) = choice
            .and_then(|choice| choice.get("finish_reason"))
            .and_then(Value::as_str)
        {
            self.finish_reason = Some(reason.to_owned());
        }
        let delta = choice.and_then(|choice| choice.get("delta"));
        if let Some(text) = delta
            .and_then(|delta| delta.get("content"))
            .and_then(Value::as_str)
        {
            self.content.push_str(text);
            self.keys.feed(text);
        }
        if let Some(text) = message_reasoning(delta) {
            self.reasoning.push_str(&text);
        }
    }

    pub(super) fn progress(&self, stage: &str) -> StreamProgress {
        StreamProgress {
            stage: stage.to_owned(),
            content_chars: self.content.chars().count(),
            reasoning_chars: self.reasoning.chars().count(),
            top_level_keys: self.keys.count,
            last_key: self.keys.last.clone(),
        }
    }

    /// The finished completion.
    ///
    /// # Errors
    /// Fails when the stream carried an error, or ended with no content.
    pub(super) fn finish(self) -> Result<ParsedCompletion> {
        if let Some(error) = self.error {
            return Err(Error::Task(format!(
                "OpenRouter returned an error: {error}"
            )));
        }
        if self.content.is_empty() && self.finish_reason.is_none() {
            return Err(Error::InvalidAnalysis(
                "OpenRouter returned no message content".to_owned(),
            ));
        }
        let billing = Billing::from_usage(self.usage.as_ref());
        Ok(ParsedCompletion {
            content: self.content,
            reasoning: (!self.reasoning.is_empty()).then_some(self.reasoning),
            provider: self.provider,
            finish_reason: self.finish_reason,
            prompt_tokens: billing.prompt_tokens,
            cached_tokens: billing.cached_tokens,
            completion_tokens: billing.completion_tokens,
            reasoning_tokens: billing.reasoning_tokens,
            cost_usd: billing.cost_usd,
            cost_source: billing.cost_source,
        })
    }
}

/// Counts the keys of the top-level JSON object as the text streams in.
#[derive(Default)]
struct KeyScanner {
    depth: u32,
    in_string: bool,
    escaped: bool,
    current: String,
    pending_key: Option<String>,
    count: usize,
    last: Option<String>,
}

impl KeyScanner {
    fn feed(&mut self, text: &str) {
        for character in text.chars() {
            if self.in_string {
                self.in_string_character(character);
                continue;
            }
            match character {
                '"' => {
                    self.in_string = true;
                    self.current.clear();
                    self.pending_key = None;
                }
                '{' | '[' => self.depth += 1,
                '}' | ']' => self.depth = self.depth.saturating_sub(1),
                ':' => {
                    if self.depth == 1
                        && let Some(key) = self.pending_key.take()
                    {
                        self.count += 1;
                        self.last = Some(key);
                    }
                }
                other if other.is_whitespace() => {}
                _ => self.pending_key = None,
            }
        }
    }

    fn in_string_character(&mut self, character: char) {
        if self.escaped {
            self.escaped = false;
        } else if character == '\\' {
            self.escaped = true;
            return;
        } else if character == '"' {
            self.in_string = false;
            if self.depth == 1 {
                self.pending_key = Some(std::mem::take(&mut self.current));
            }
            return;
        }
        if self.depth == 1 {
            self.current.push(character);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(delta: &str) -> String {
        format!("data: {{\"choices\":[{{\"delta\":{delta}}}]}}\n\n")
    }

    #[test]
    fn should_reassemble_content_when_an_event_is_split_mid_line() {
        let mut stream = StreamAccumulator::default();
        let full = event(r#"{"content":"{\"a\":"}"#) + &event(r#"{"content":"1}"}"#);
        let (head, tail) = full.as_bytes().split_at(20);
        assert!(!stream.feed(head).expect("head"));
        assert!(stream.feed(tail).expect("tail"));
        let parsed = stream.finish().expect("finished");
        assert_eq!(parsed.content, r#"{"a":1}"#);
    }

    #[test]
    fn should_keep_a_multibyte_character_split_across_chunks() {
        let mut stream = StreamAccumulator::default();
        let full = event(r#"{"content":"é"}"#);
        let bytes = full.as_bytes();
        let split = bytes.iter().position(|byte| *byte == 0xC3).expect("é") + 1;
        stream.feed(&bytes[..split]).expect("head");
        stream.feed(&bytes[split..]).expect("tail");
        assert_eq!(stream.finish().expect("finished").content, "é");
    }

    #[test]
    fn should_not_count_keep_alive_comments_or_blank_lines_as_data() {
        let mut stream = StreamAccumulator::default();
        let received = stream
            .feed(b": OPENROUTER PROCESSING\n\n\r\n")
            .expect("comments");
        assert!(!received);
    }

    #[test]
    fn should_accept_crlf_line_endings_and_done() {
        let mut stream = StreamAccumulator::default();
        let text = event(r#"{"content":"x"}"#).replace('\n', "\r\n") + "data: [DONE]\r\n\r\n";
        assert!(stream.feed(text.as_bytes()).expect("fed"));
        assert_eq!(stream.finish().expect("finished").content, "x");
    }

    #[test]
    fn should_read_usage_cost_provider_and_finish_from_the_final_chunks() {
        let mut stream = StreamAccumulator::default();
        let text = event(r#"{"content":"{}"}"#)
            + "data: {\"provider\":\"DeepSeek\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n"
            + "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5,\"cost\":0.002,\"completion_tokens_details\":{\"reasoning_tokens\":3}}}\n\n"
            + "data: [DONE]\n\n";
        stream.feed(text.as_bytes()).expect("fed");
        let parsed = stream.finish().expect("finished");
        assert_eq!(parsed.provider.as_deref(), Some("DeepSeek"));
        assert_eq!(parsed.finish_reason.as_deref(), Some("stop"));
        assert_eq!(parsed.prompt_tokens, 10);
        assert_eq!(parsed.reasoning_tokens, 3);
        assert!((parsed.cost_usd - 0.002).abs() < 1e-12);
    }

    #[test]
    fn should_collect_reasoning_from_every_provider_spelling() {
        let mut stream = StreamAccumulator::default();
        let text = event(r#"{"reasoning":"a"}"#)
            + &event(r#"{"reasoning_content":"b"}"#)
            + &event(r#"{"reasoning_details":[{"text":"c"}]}"#)
            + &event(r#"{"content":"{}"}"#);
        stream.feed(text.as_bytes()).expect("fed");
        assert_eq!(stream.finish().expect("finished").reasoning.as_deref(), Some("abc"));
    }

    #[test]
    fn should_fail_when_the_stream_carries_an_error_chunk() {
        let mut stream = StreamAccumulator::default();
        let text = event(r#"{"content":"{\"a\""}"#)
            + "data: {\"error\":{\"code\":502,\"message\":\"upstream died\"},\"choices\":[{\"delta\":{},\"finish_reason\":\"error\"}]}\n\n";
        stream.feed(text.as_bytes()).expect("fed");
        let error = stream.finish().expect_err("error chunk");
        assert!(error.to_string().contains("upstream died"), "{error}");
    }

    #[test]
    fn should_keep_a_length_finish_with_the_partial_content() {
        let mut stream = StreamAccumulator::default();
        let text = event(r#"{"content":"{\"a\":"}"#)
            + "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"length\"}]}\n\n";
        stream.feed(text.as_bytes()).expect("fed");
        let parsed = stream.finish().expect("finished");
        assert_eq!(parsed.finish_reason.as_deref(), Some("length"));
        assert_eq!(parsed.content, "{\"a\":");
    }

    #[test]
    fn should_fail_when_the_stream_ends_with_nothing() {
        assert!(StreamAccumulator::default().finish().is_err());
    }

    #[test]
    fn should_fail_when_an_event_is_not_json() {
        let mut stream = StreamAccumulator::default();
        assert!(stream.feed(b"data: {not json\n\n").is_err());
    }

    #[test]
    fn should_count_top_level_keys_as_they_stream_in() {
        let mut stream = StreamAccumulator::default();
        let answer = r#"{"identity":{"title":"A: b","tags":["x","y"]},"type":"empirical","ab\"stract":"has \"quotes\": and colons""#;
        for piece in answer.as_bytes().chunks(7) {
            let text = serde_json::to_string(std::str::from_utf8(piece).expect("ascii"))
                .expect("json");
            stream
                .feed(event(&format!("{{\"content\":{text}}}")).as_bytes())
                .expect("fed");
        }
        let progress = stream.progress("core");
        assert_eq!(progress.top_level_keys, 3);
        assert_eq!(progress.last_key.as_deref(), Some("ab\"stract"));
        assert_eq!(progress.stage, "core");
        assert_eq!(progress.content_chars, answer.chars().count());
    }

    #[test]
    fn should_not_mistake_a_string_value_for_a_key() {
        let mut scanner = KeyScanner::default();
        scanner.feed(r#"{"a":"b","c":["d","e"],"f":1}"#);
        assert_eq!(scanner.count, 3);
        assert_eq!(scanner.last.as_deref(), Some("f"));
    }
}
