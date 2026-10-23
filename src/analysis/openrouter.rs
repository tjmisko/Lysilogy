//! Direct `OpenRouter` chat completions.
//!
//! One request carries text and page images to a named model and asks for one
//! strict JSON answer. Every completion is billed to the spend ledger; the cost
//! `OpenRouter` reports is preferred, and the published-price estimate is used
//! only when the response carries none.

use std::time::{Duration, Instant};

use base64::Engine as _;
use chrono::Utc;
use serde::Serialize;
use serde_json::{Value, json};

use super::spend::{SpendLedger, SpendRecord, completion_cost};
use crate::{Error, Result};

mod stream;

pub use stream::StreamProgress;
use stream::StreamAccumulator;

/// `OpenRouter`'s chat completions endpoint.
pub const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1/chat/completions";

/// How long a streamed answer may go without a data event before the call is
/// abandoned. Keep-alive comments do not count; they only prove the connection
/// is open, not that the provider is producing.
pub const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(90);

/// The model used when a caller names none.
pub const DEFAULT_MODEL: &str = "deepseek/deepseek-v4.1-flash";

// Routing policy: cheapest endpoint that supports the request, falls back when a
// host is rate-limited, stays under a price ceiling, and skips low-bit
// quantization. Prices are USD per million tokens.
const PROVIDER_MAX_PROMPT_PRICE: &str = "0.5";
const PROVIDER_MAX_COMPLETION_PRICE: &str = "2.0";
const PROVIDER_MIN_THROUGHPUT: f64 = 20.0;
const PROVIDER_MAX_LATENCY_S: f64 = 60.0;

/// One part of the user message.
#[derive(Clone, Debug)]
pub enum ContentPart {
    Text(String),
    /// PNG bytes, sent as a base64 data URL.
    Png(Vec<u8>),
}

/// A single strict-JSON completion request.
#[derive(Clone, Copy, Debug)]
pub struct ChatRequest<'a> {
    pub model: &'a str,
    pub parts: &'a [ContentPart],
    pub schema: &'a Value,
    /// The substage this call belongs to, for the spend ledger.
    pub stage: &'a str,
    /// The most output tokens, reasoning included, before the call is cut off.
    pub max_tokens: Option<u32>,
    /// How much reasoning the provider should spend; `None` leaves the model
    /// default in place.
    pub reasoning: Option<ReasoningEffort>,
    /// Try these providers in order and do not fall back, so calls that share
    /// a prefix can land on the same cache. Empty leaves routing to `OpenRouter`.
    pub provider_order: &'a [String],
}

/// How much reasoning the provider should spend before answering.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReasoningEffort {
    /// Ask the provider not to reason.
    Off,
    Low,
    Medium,
    High,
}

impl ReasoningEffort {
    /// Parse a command-line value, or `None` for an unknown one.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "off" => Some(Self::Off),
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            _ => None,
        }
    }

    fn to_body(self) -> Value {
        match self {
            Self::Off => json!({"enabled": false}),
            Self::Low => json!({"effort": "low"}),
            Self::Medium => json!({"effort": "medium"}),
            Self::High => json!({"effort": "high"}),
        }
    }

    /// The lowercase name, for records and command-line output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// Where a recorded cost came from.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CostSource {
    /// `OpenRouter` reported the cost in `usage.cost`.
    Reported,
    /// Estimated from the published deepseek prices; wrong for other models.
    Estimated,
    /// A plan-backed harness; no metered cost.
    Plan,
}

/// A parsed completion before it is billed.
#[derive(Clone, Debug, PartialEq)]
pub struct ParsedCompletion {
    pub content: String,
    /// The model's reasoning text, when the provider returns it.
    pub reasoning: Option<String>,
    pub provider: Option<String>,
    pub finish_reason: Option<String>,
    pub prompt_tokens: u64,
    /// The part of `prompt_tokens` the provider served from its prefix cache.
    pub cached_tokens: u64,
    pub completion_tokens: u64,
    pub reasoning_tokens: u64,
    pub cost_usd: f64,
    pub cost_source: CostSource,
}

/// A billed completion.
#[derive(Clone, Debug)]
pub struct Completion {
    pub parsed: ParsedCompletion,
    pub latency: Duration,
}

/// The request body for `request`.
#[must_use]
pub fn request_body(request: &ChatRequest<'_>) -> Value {
    // A single text part keeps the plain-string content the text-only stages
    // have always sent.
    let content = match request.parts {
        [ContentPart::Text(text)] => Value::String(text.clone()),
        parts => Value::Array(parts.iter().map(content_part).collect()),
    };
    let mut body = json!({
        "model": request.model,
        "messages": [{"role": "user", "content": content}],
        "response_format": {
            "type": "json_schema",
            "json_schema": {"name": "result", "strict": true, "schema": request.schema},
        },
        "stream": true,
        "usage": {"include": true},
    });
    if let Some(max_tokens) = request.max_tokens {
        body["max_tokens"] = json!(max_tokens);
    }
    if let Some(effort) = request.reasoning {
        body["reasoning"] = effort.to_body();
    }
    let mut provider = json!({
        "require_parameters": true,
        "allow_fallbacks": true,
        "max_price": {
            "prompt": PROVIDER_MAX_PROMPT_PRICE,
            "completion": PROVIDER_MAX_COMPLETION_PRICE,
        },
        "preferred_min_throughput": PROVIDER_MIN_THROUGHPUT,
        "preferred_max_latency": PROVIDER_MAX_LATENCY_S,
        "quantizations": ["fp8", "mxfp8", "fp16", "bf16", "fp32", "unknown"],
    });
    if request.provider_order.is_empty() {
        provider["sort"] = json!("price");
    } else {
        let order: Vec<String> = request
            .provider_order
            .iter()
            .map(|provider| provider.trim().to_ascii_lowercase())
            .collect();
        provider["order"] = json!(order);
    }
    body["provider"] = provider;
    body
}

fn content_part(part: &ContentPart) -> Value {
    match part {
        ContentPart::Text(text) => json!({"type": "text", "text": text}),
        ContentPart::Png(bytes) => {
            let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
            json!({
                "type": "image_url",
                "image_url": {"url": format!("data:image/png;base64,{encoded}")},
            })
        }
    }
}

/// Parse a successful response body.
///
/// # Errors
/// Fails when the body is not JSON, carries an `error`, or has no message content.
pub fn parse_response(text: &str) -> Result<ParsedCompletion> {
    let value: Value = serde_json::from_str(text)?;
    if let Some(error) = value.get("error").filter(|error| !error.is_null()) {
        return Err(Error::Task(format!(
            "OpenRouter returned an error: {}",
            truncate(&error.to_string())
        )));
    }
    let choice = value.get("choices").and_then(|choices| choices.get(0));
    let message = choice.and_then(|choice| choice.get("message"));
    let content = message
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            Error::InvalidAnalysis("OpenRouter returned no message content".to_owned())
        })?;
    let reasoning = message_reasoning(message);
    let billing = Billing::from_usage(value.get("usage"));
    let text_field = |container: Option<&Value>, key: &str| {
        container
            .and_then(|container| container.get(key))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    Ok(ParsedCompletion {
        content: content.to_owned(),
        reasoning,
        provider: text_field(Some(&value), "provider"),
        finish_reason: text_field(choice, "finish_reason"),
        prompt_tokens: billing.prompt_tokens,
        cached_tokens: billing.cached_tokens,
        completion_tokens: billing.completion_tokens,
        reasoning_tokens: billing.reasoning_tokens,
        cost_usd: billing.cost_usd,
        cost_source: billing.cost_source,
    })
}

/// The token counts and cost a response's `usage` block carries.
struct Billing {
    prompt_tokens: u64,
    cached_tokens: u64,
    completion_tokens: u64,
    reasoning_tokens: u64,
    cost_usd: f64,
    cost_source: CostSource,
}

impl Billing {
    fn from_usage(usage: Option<&Value>) -> Self {
        let count = |key: &str| {
            usage
                .and_then(|usage| usage.get(key))
                .and_then(Value::as_u64)
                .unwrap_or(0)
        };
        let prompt_tokens = count("prompt_tokens");
        let cached_tokens = usage
            .and_then(|usage| usage.get("prompt_tokens_details"))
            .and_then(|details| details.get("cached_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let completion_tokens = count("completion_tokens");
        let reasoning_tokens = usage
            .and_then(|usage| usage.get("completion_tokens_details"))
            .and_then(|details| details.get("reasoning_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let reported = usage
            .and_then(|usage| usage.get("cost"))
            .and_then(Value::as_f64)
            .filter(|cost| *cost > 0.0);
        let (cost_usd, cost_source) = reported.map_or_else(
            || {
                (
                    completion_cost(prompt_tokens, completion_tokens),
                    CostSource::Estimated,
                )
            },
            |cost| (cost, CostSource::Reported),
        );
        Self {
            prompt_tokens,
            cached_tokens,
            completion_tokens,
            reasoning_tokens,
            cost_usd,
            cost_source,
        }
    }
}

fn message_reasoning(message: Option<&Value>) -> Option<String> {
    let message = message?;
    // Providers normalize the trace differently: `reasoning` (OpenRouter),
    // `reasoning_content` (DeepSeek), or a `reasoning_details` array.
    for key in ["reasoning", "reasoning_content"] {
        if let Some(text) = message.get(key).and_then(Value::as_str)
            && !text.is_empty()
        {
            return Some(text.to_owned());
        }
    }
    let details = message.get("reasoning_details").and_then(Value::as_array)?;
    let parts = details
        .iter()
        .filter_map(|detail| detail.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>();
    (!parts.is_empty()).then(|| parts.join("\n"))
}

// The key from `OPENROUTER_API_KEY`, or from the file `OR_KEY_PATH` names.
fn api_key() -> Result<String> {
    if let Some(key) = std::env::var("OPENROUTER_API_KEY")
        .ok()
        .filter(|value| !value.is_empty())
    {
        return Ok(key);
    }
    if let Ok(path) = std::env::var("OR_KEY_PATH") {
        let key = std::fs::read_to_string(&path)
            .map_err(|error| Error::ProgramUnavailable(format!("OR_KEY_PATH {path}: {error}")))?
            .trim()
            .to_owned();
        if key.is_empty() {
            return Err(Error::ProgramUnavailable(format!(
                "OR_KEY_PATH {path} is empty"
            )));
        }
        return Ok(key);
    }
    Err(Error::ProgramUnavailable("OPENROUTER_API_KEY".to_owned()))
}

/// Receives progress while a streamed answer arrives.
#[derive(Clone)]
pub struct ProgressSink(std::sync::Arc<dyn Fn(StreamProgress) + Send + Sync>);

impl ProgressSink {
    pub fn new(report: impl Fn(StreamProgress) + Send + Sync + 'static) -> Self {
        Self(std::sync::Arc::new(report))
    }

    fn report(&self, progress: StreamProgress) {
        (self.0)(progress);
    }
}

impl std::fmt::Debug for ProgressSink {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ProgressSink")
    }
}

/// Send `request`, bill it to `ledger`, and return the completion.
///
/// # Errors
/// Fails without a key, when the budget is spent, on a transport or HTTP
/// error, when the stream goes quiet for [`STREAM_IDLE_TIMEOUT`], or when the
/// response cannot be parsed or recorded.
pub async fn complete(
    client: &reqwest::Client,
    ledger: &SpendLedger,
    request: &ChatRequest<'_>,
) -> Result<Completion> {
    complete_with_progress(client, ledger, request, None).await
}

/// [`complete`], reporting to `progress` as the answer streams in.
///
/// # Errors
/// As [`complete`].
pub async fn complete_with_progress(
    client: &reqwest::Client,
    ledger: &SpendLedger,
    request: &ChatRequest<'_>,
    progress: Option<&ProgressSink>,
) -> Result<Completion> {
    let api_key = api_key()?;
    ledger.ensure_available()?;
    let started = Instant::now();
    let mut response = client
        .post(OPENROUTER_URL)
        .bearer_auth(api_key)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header("HTTP-Referer", "https://github.com/tjmisko/lysilogy")
        .header("X-Title", "Lysilogy")
        .body(serde_json::to_vec(&request_body(request))?)
        .send()
        .await
        .map_err(|error| Error::Task(format!("OpenRouter request failed: {error}")))?;
    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.map_err(|error| {
            Error::Task(format!("OpenRouter response could not be read: {error}"))
        })?;
        return Err(Error::Task(format!(
            "OpenRouter returned {status}: {}",
            truncate(&text)
        )));
    }
    let mut accumulator = StreamAccumulator::default();
    let mut last_event = Instant::now();
    loop {
        let wait = STREAM_IDLE_TIMEOUT.saturating_sub(last_event.elapsed());
        let chunk = tokio::time::timeout(wait, response.chunk())
            .await
            .map_err(|_| {
                Error::Task(format!(
                    "OpenRouter stream sent no data for {}s",
                    STREAM_IDLE_TIMEOUT.as_secs()
                ))
            })?
            .map_err(|error| Error::Task(format!("OpenRouter stream failed: {error}")))?;
        let Some(chunk) = chunk else {
            break;
        };
        if accumulator.feed(&chunk)? {
            last_event = Instant::now();
            if let Some(sink) = progress {
                sink.report(accumulator.progress(request.stage));
            }
        }
    }
    let latency = started.elapsed();
    let parsed = accumulator.finish()?;
    ledger.record(&SpendRecord {
        at: Utc::now(),
        stage: request.stage.to_owned(),
        model: request.model.to_owned(),
        prompt_tokens: parsed.prompt_tokens,
        completion_tokens: parsed.completion_tokens,
        cost_usd: parsed.cost_usd,
    })?;
    Ok(Completion { parsed, latency })
}

/// An HTTP client for `OpenRouter` with an overall request timeout.
///
/// # Errors
/// Fails when the TLS backend cannot be initialized.
pub fn client(timeout: Duration) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|error| {
            Error::Task(format!(
                "could not build the OpenRouter HTTP client: {error}"
            ))
        })
}

fn truncate(value: &str) -> String {
    const MAXIMUM: usize = 4_000;
    if value.chars().count() <= MAXIMUM {
        return value.trim().to_owned();
    }
    format!(
        "{}…",
        value.chars().take(MAXIMUM).collect::<String>().trim()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> Value {
        json!({"type": "object", "properties": {}, "required": [], "additionalProperties": false})
    }

    #[test]
    fn should_request_a_stream_that_reports_usage_when_a_request_is_built() {
        let schema = schema();
        let parts = [ContentPart::Text("hello".to_owned())];
        let body = request_body(&ChatRequest {
            model: "m",
            parts: &parts,
            schema: &schema,
            stage: "s",
            max_tokens: None,
            reasoning: None,
            provider_order: &[],
        });
        assert_eq!(body["stream"], true);
        assert_eq!(body["usage"]["include"], true);
    }

    #[test]
    fn should_send_plain_string_content_when_the_request_is_one_text_part() {
        let schema = schema();
        let parts = [ContentPart::Text("hello".to_owned())];
        let body = request_body(&ChatRequest {
            model: DEFAULT_MODEL,
            parts: &parts,
            schema: &schema,
            stage: "test",
            max_tokens: None,
            reasoning: None,
            provider_order: &[],
        });
        assert_eq!(body["model"], DEFAULT_MODEL);
        assert_eq!(body["messages"][0]["content"], "hello");
        assert_eq!(body["response_format"]["json_schema"]["strict"], true);
        assert_eq!(body["usage"]["include"], true);
        assert!(body.get("reasoning").is_none());
    }

    #[test]
    fn should_send_reasoning_effort_when_it_is_set() {
        let schema = schema();
        let parts = [ContentPart::Text("hello".to_owned())];
        for (effort, expected) in [
            (ReasoningEffort::Low, json!({"effort": "low"})),
            (ReasoningEffort::Off, json!({"enabled": false})),
        ] {
            let body = request_body(&ChatRequest {
                model: DEFAULT_MODEL,
                parts: &parts,
                schema: &schema,
                stage: "test",
                max_tokens: None,
                reasoning: Some(effort),
                provider_order: &[],
            });
            assert_eq!(body["reasoning"], expected);
        }
    }

    #[test]
    fn should_send_images_as_png_data_urls_in_order_when_parts_are_mixed() {
        let schema = schema();
        let parts = [
            ContentPart::Text("page 1".to_owned()),
            ContentPart::Png(vec![0x89, b'P', b'N', b'G']),
            ContentPart::Text("page 2".to_owned()),
        ];
        let body = request_body(&ChatRequest {
            model: "google/gemini-x",
            parts: &parts,
            schema: &schema,
            stage: "test",
            max_tokens: None,
            reasoning: None,
            provider_order: &[],
        });
        let content = body["messages"][0]["content"]
            .as_array()
            .expect("array content");
        assert_eq!(content.len(), 3);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "image_url");
        assert_eq!(
            content[1]["image_url"]["url"],
            "data:image/png;base64,iVBORw=="
        );
        assert_eq!(content[2]["text"], "page 2");
        assert_eq!(body["model"], "google/gemini-x");
    }

    #[test]
    fn should_send_an_image_only_request_as_an_array() {
        let schema = schema();
        let parts = [ContentPart::Png(vec![1, 2, 3])];
        let body = request_body(&ChatRequest {
            model: DEFAULT_MODEL,
            parts: &parts,
            schema: &schema,
            stage: "test",
            max_tokens: None,
            reasoning: None,
            provider_order: &[],
        });
        assert!(body["messages"][0]["content"].is_array());
    }

    #[test]
    fn should_send_max_tokens_only_when_a_ceiling_is_set() {
        let schema = schema();
        let parts = [ContentPart::Text("hello".to_owned())];
        let mut request = ChatRequest {
            model: DEFAULT_MODEL,
            parts: &parts,
            schema: &schema,
            stage: "test",
            max_tokens: None,
            reasoning: None,
            provider_order: &[],
        };
        assert!(request_body(&request).get("max_tokens").is_none());
        request.max_tokens = Some(32_768);
        assert_eq!(request_body(&request)["max_tokens"], 32_768);
    }

    #[test]
    fn should_prefer_the_reported_cost_when_usage_carries_one() {
        let parsed = parse_response(
            r#"{"provider":"Together","choices":[{"finish_reason":"stop","message":{"content":"{}"}}],
                "usage":{"prompt_tokens":763,"completion_tokens":425,"cost":0.0007389,
                         "completion_tokens_details":{"reasoning_tokens":287}}}"#,
        )
        .expect("parsed");
        assert_eq!(parsed.cost_source, CostSource::Reported);
        assert!((parsed.cost_usd - 0.000_738_9).abs() < 1e-12);
        assert_eq!(parsed.reasoning_tokens, 287);
        assert_eq!(parsed.provider.as_deref(), Some("Together"));
        assert_eq!(parsed.finish_reason.as_deref(), Some("stop"));
    }

    #[test]
    fn should_harvest_reasoning_from_any_provider_shape() {
        let call = |body: &str| parse_response(body).expect("parsed").reasoning;
        assert_eq!(
            call(r#"{"choices":[{"message":{"content":"{}","reasoning":"step 1"}}]}"#).as_deref(),
            Some("step 1")
        );
        assert_eq!(
            call(r#"{"choices":[{"message":{"content":"{}","reasoning_content":"step 2"}}]}"#)
                .as_deref(),
            Some("step 2")
        );
        assert_eq!(
            call(r#"{"choices":[{"message":{"content":"{}","reasoning_details":[{"text":"x"},{"text":"y"}]}}]}"#).as_deref(),
            Some("x\ny")
        );
        assert_eq!(call(r#"{"choices":[{"message":{"content":"{}"}}]}"#), None);
    }

    #[test]
    fn should_read_cached_prompt_tokens_when_the_provider_reports_them() {
        let parsed = parse_response(
            r#"{"choices":[{"message":{"content":"{}"}}],"usage":{"prompt_tokens":900,"prompt_tokens_details":{"cached_tokens":768},"completion_tokens":10}}"#,
        )
        .expect("parsed");
        assert_eq!(parsed.cached_tokens, 768);
    }

    #[test]
    fn should_pin_providers_only_when_one_is_named() {
        let schema = schema();
        let parts = [ContentPart::Text("hello".to_owned())];
        let pinned = ["DeepSeek".to_owned()];
        let body = request_body(&ChatRequest {
            model: DEFAULT_MODEL,
            parts: &parts,
            schema: &schema,
            stage: "test",
            max_tokens: None,
            reasoning: None,
            provider_order: &pinned,
        });
        assert_eq!(body["provider"]["order"][0], "deepseek");
        assert_eq!(body["provider"]["allow_fallbacks"], true);
        assert_eq!(body["provider"]["require_parameters"], true);
        assert!(body["provider"].get("sort").is_none());
    }

    #[test]
    fn should_route_cheap_with_a_price_ceiling_when_no_provider_is_pinned() {
        let schema = schema();
        let parts = [ContentPart::Text("hello".to_owned())];
        let body = request_body(&ChatRequest {
            model: DEFAULT_MODEL,
            parts: &parts,
            schema: &schema,
            stage: "test",
            max_tokens: None,
            reasoning: None,
            provider_order: &[],
        });
        let provider = &body["provider"];
        assert_eq!(provider["sort"], "price");
        assert_eq!(provider["allow_fallbacks"], true);
        assert_eq!(provider["require_parameters"], true);
        assert_eq!(provider["max_price"]["completion"], "2.0");
        assert!(provider.get("order").is_none());
        assert!(
            provider["quantizations"]
                .as_array()
                .is_some_and(|q| q.contains(&json!("fp8")))
        );
    }

    #[test]
    fn should_estimate_the_cost_when_usage_has_no_cost() {
        let parsed = parse_response(
            r#"{"choices":[{"message":{"content":"{}"}}],"usage":{"prompt_tokens":1000000,"completion_tokens":0}}"#,
        )
        .expect("parsed");
        assert_eq!(parsed.cost_source, CostSource::Estimated);
        assert!((parsed.cost_usd - completion_cost(1_000_000, 0)).abs() < 1e-12);
    }

    #[test]
    fn should_fail_when_the_body_carries_an_error_despite_a_success_status() {
        let error = parse_response(r#"{"error":{"code":400,"message":"schema rejected"}}"#)
            .expect_err("error body");
        assert!(error.to_string().contains("schema rejected"), "{error}");
    }

    #[test]
    fn should_fail_when_there_is_no_message_content() {
        assert!(parse_response(r#"{"choices":[{"message":{"content":null}}]}"#).is_err());
        assert!(parse_response(r#"{"choices":[]}"#).is_err());
        assert!(parse_response("not json").is_err());
    }

    #[test]
    fn should_report_a_length_finish_when_output_was_cut_off() {
        let parsed = parse_response(
            r#"{"choices":[{"finish_reason":"length","message":{"content":"{\"a\":"}}]}"#,
        )
        .expect("parsed");
        assert_eq!(parsed.finish_reason.as_deref(), Some("length"));
    }
}
