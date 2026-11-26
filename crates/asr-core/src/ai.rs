//! Talks to the AI providers the user holds keys for. Each provider has its own wire format
//! behind one `LlmClient` seam; a request never moves to another provider by itself.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const MAX_OUTPUT_TOKENS: u32 = 16_000;
const SUMMARY_TIMEOUT: Duration = Duration::from_secs(600);
const PING_TIMEOUT: Duration = Duration::from_secs(12);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(4);
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Provider {
    Claude,
    ChatGpt,
    Gemini,
}

impl Provider {
    pub const ALL: [Provider; 3] = [Provider::Claude, Provider::ChatGpt, Provider::Gemini];

    pub fn default_model(self) -> &'static str {
        match self {
            Provider::Claude => "claude-sonnet-4-5",
            Provider::ChatGpt => "gpt-4.1-mini",
            Provider::Gemini => "gemini-2.5-flash",
        }
    }

    pub fn recommended_models(self) -> &'static [&'static str] {
        match self {
            Provider::Claude => &[
                "claude-sonnet-4-5",
                "claude-opus-4-5",
                "claude-haiku-4-5",
                "claude-sonnet-4-20250514",
                "claude-3-7-sonnet-latest",
            ],
            Provider::ChatGpt => &[
                "gpt-4.1-mini",
                "gpt-4.1",
                "gpt-4o-mini",
                "gpt-4o",
                "o4-mini",
                "gpt-5-mini",
                "gpt-5",
            ],
            Provider::Gemini => &[
                "gemini-2.5-flash",
                "gemini-2.5-flash-lite",
                "gemini-2.5-pro",
                "gemini-2.0-flash",
                "gemini-2.0-flash-lite",
            ],
        }
    }

    fn base_url(self) -> &'static str {
        match self {
            Provider::Claude => "https://api.anthropic.com/v1",
            Provider::ChatGpt => "https://api.openai.com/v1",
            Provider::Gemini => "https://generativelanguage.googleapis.com/v1beta",
        }
    }
}

#[derive(Debug, Clone, thiserror::Error, PartialEq)]
pub enum AiError {
    #[error("API key không đúng hoặc không có quyền dùng model này ({0})")]
    Unauthorized(String),
    #[error("Nhà cung cấp đang giới hạn lượt gọi hoặc đã hết hạn mức ({0})")]
    RateLimited(String),
    #[error("Nhà cung cấp báo lỗi {status}: {message}")]
    Rejected { status: u16, message: String },
    #[error("Câu trả lời bị cắt vì quá dài, nên không dùng được")]
    Truncated,
    #[error("AI từ chối xử lý nội dung này: {0}")]
    Refused(String),
    #[error("Không kết nối được: {0}")]
    Network(String),
    #[error("Câu trả lời không đúng định dạng: {0}")]
    Malformed(String),
    #[error("Không đọc hoặc ghi được file API key: {0}")]
    KeyFile(String),
    #[error("Chưa có API key cho nhà cung cấp này")]
    MissingKey,
}

pub trait LlmClient: Send + Sync {
    /// The model's text. `schema` constrains it to JSON of that shape.
    fn complete(&self, system: &str, user: &str, schema: &Value) -> Result<String, AiError>;
}

pub struct HttpClient {
    provider: Provider,
    model: String,
    key: String,
    base_url: String,
}

impl HttpClient {
    pub fn new(provider: Provider, model: &str, key: &str) -> Self {
        Self::with_base_url(provider, model, key, provider.base_url())
    }

    fn with_base_url(provider: Provider, model: &str, key: &str, base_url: &str) -> Self {
        Self {
            provider,
            model: model.trim().to_owned(),
            key: key.to_owned(),
            base_url: base_url.to_owned(),
        }
    }

    /// Spends a few tokens to check the key and the model name. Only the status matters.
    pub fn ping(&self) -> Result<(), AiError> {
        let (url, headers, body) = self.request("Reply OK", "OK", None, 8);
        self.send(&url, &headers, &body, PING_TIMEOUT).map(|_| ())
    }

    /// Text models this key can use, as the provider lists them (Claude: newest first).
    pub fn list_models(&self) -> Result<Vec<String>, AiError> {
        let mut models = Vec::new();
        let mut page: Option<String> = None;
        loop {
            let (url, headers) = match self.provider {
                Provider::Claude => {
                    let after = page
                        .as_ref()
                        .map_or(String::new(), |id| format!("&after_id={id}"));
                    (
                        format!("{}/models?limit=1000{after}", self.base_url),
                        vec![
                            ("x-api-key", self.key.clone()),
                            ("anthropic-version", ANTHROPIC_VERSION.to_owned()),
                        ],
                    )
                }
                Provider::ChatGpt => (
                    format!("{}/models", self.base_url),
                    vec![("authorization", format!("Bearer {}", self.key))],
                ),
                Provider::Gemini => {
                    let token = page
                        .as_ref()
                        .map_or(String::new(), |t| format!("&pageToken={t}"));
                    (
                        format!("{}/models?pageSize=1000{token}", self.base_url),
                        vec![("x-goog-api-key", self.key.clone())],
                    )
                }
            };
            let reply = self.get(&url, &headers)?;
            page = match self.provider {
                Provider::Claude => {
                    models.extend(ids(&reply["data"], "id"));
                    reply["has_more"]
                        .as_bool()
                        .unwrap_or(false)
                        .then(|| reply["last_id"].as_str().map(str::to_owned))
                        .flatten()
                }
                Provider::ChatGpt => {
                    models.extend(ids(&reply["data"], "id").filter(|id| is_chat_model(id)));
                    // Newest-looking first, as the other two providers order theirs.
                    models.sort_by(|a, b| b.cmp(a));
                    None
                }
                Provider::Gemini => {
                    let usable = reply["models"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|m| {
                            m["supportedGenerationMethods"]
                                .as_array()
                                .is_some_and(|methods| {
                                    methods.iter().any(|x| x == "generateContent")
                                })
                        });
                    models.extend(usable.filter_map(|m| {
                        m["name"]
                            .as_str()
                            .map(|n| n.trim_start_matches("models/").to_owned())
                    }));
                    reply["nextPageToken"]
                        .as_str()
                        .filter(|t| !t.is_empty())
                        .map(str::to_owned)
                }
            };
            if page.is_none() {
                return Ok(models);
            }
        }
    }

    /// Probes up to 6 prioritized modern models in parallel with tiny fast calls and returns
    /// only the ones this API key actually has quota/permission to invoke.
    /// Pass `prefetched` from an earlier `list_models()` call to avoid a duplicate network round-trip.
    pub fn probe_accessible_models(
        &self,
        prefetched: Option<&[String]>,
    ) -> Result<Vec<String>, AiError> {
        const MAX_PROBES: usize = 6;
        let available = match prefetched {
            Some(list) if !list.is_empty() => list.to_vec(),
            _ => filter_modern_models(self.provider, self.list_models()?),
        };
        let mut candidates: Vec<String> = Vec::new();
        for &rec in self.provider.recommended_models() {
            if available.iter().any(|m| m == rec) && !candidates.iter().any(|c| c == rec) {
                candidates.push(rec.to_owned());
            }
        }
        for m in available {
            if candidates.len() >= MAX_PROBES {
                break;
            }
            if !candidates.iter().any(|c| c == &m) {
                candidates.push(m);
            }
        }
        candidates.truncate(MAX_PROBES);

        let probe_client = reqwest::blocking::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(PROBE_TIMEOUT)
            .build()
            .map_err(|e| AiError::Network(e.without_url().to_string()))?;

        let results: Vec<(String, bool)> = std::thread::scope(|scope| {
            let handles: Vec<_> = candidates
                .into_iter()
                .map(|id| {
                    let probe = HttpClient {
                        provider: self.provider,
                        model: id.clone(),
                        key: self.key.clone(),
                        base_url: self.base_url.clone(),
                    };
                    let client_ref = &probe_client;
                    scope.spawn(move || {
                        let (url, headers, body) = probe.request("Reply OK", "OK", None, 8);
                        (id, probe.send_with(client_ref, &url, &headers, &body).is_ok())
                    })
                })
                .collect();
            handles
                .into_iter()
                .filter_map(|h| h.join().ok())
                .collect()
        });

        Ok(results
            .into_iter()
            .filter_map(|(id, ok)| ok.then_some(id))
            .collect())
    }

    /// Short plain-text completion (e.g. naming a custom Markdown profile).
    pub fn quick_text(&self, system: &str, user: &str, max_tokens: u32) -> Result<String, AiError> {
        let (url, headers, body) = self.request(system, user, None, max_tokens);
        let reply = self.send(&url, &headers, &body, PING_TIMEOUT)?;
        self.read_text(&reply)
    }

    fn get(&self, url: &str, headers: &[(&'static str, String)]) -> Result<Value, AiError> {
        let network = |e: reqwest::Error| AiError::Network(e.without_url().to_string());
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(PING_TIMEOUT)
            .build()
            .map_err(network)?;
        let mut request = client.get(url);
        for (name, value) in headers {
            request = request.header(*name, value);
        }
        let response = request.send().map_err(network)?;
        let status = response.status().as_u16();
        let text = response.text().map_err(network)?;
        check_status(status, &text)
    }

    fn request(
        &self,
        system: &str,
        user: &str,
        schema: Option<&Value>,
        max_tokens: u32,
    ) -> (String, Vec<(&'static str, String)>, Value) {
        match self.provider {
            Provider::Claude => {
                let headers = vec![
                    ("x-api-key", self.key.clone()),
                    ("anthropic-version", ANTHROPIC_VERSION.to_owned()),
                ];
                let mut body = json!({
                    "model": self.model,
                    "max_tokens": max_tokens,
                    "system": system,
                    "messages": [{ "role": "user", "content": user }],
                });
                if let Some(schema) = schema {
                    body["output_config"] = json!({
                        "effort": "high",
                        "format": { "type": "json_schema", "schema": schema },
                    });
                }
                (format!("{}/messages", self.base_url), headers, body)
            }
            Provider::ChatGpt => {
                let mut body = json!({
                    "model": self.model,
                    "max_completion_tokens": max_tokens,
                    "messages": [
                        { "role": "developer", "content": system },
                        { "role": "user", "content": user },
                    ],
                });
                if let Some(schema) = schema {
                    body["response_format"] = json!({
                        "type": "json_schema",
                        "json_schema": { "name": "meeting_note", "strict": true, "schema": schema },
                    });
                }
                let headers = vec![("authorization", format!("Bearer {}", self.key))];
                (format!("{}/chat/completions", self.base_url), headers, body)
            }
            Provider::Gemini => {
                let mut config = json!({ "maxOutputTokens": max_tokens });
                if let Some(schema) = schema {
                    config["responseMimeType"] = json!("application/json");
                    config["responseJsonSchema"] = schema.clone();
                }
                let body = json!({
                    "systemInstruction": { "parts": [{ "text": system }] },
                    "contents": [{ "role": "user", "parts": [{ "text": user }] }],
                    "generationConfig": config,
                });
                let url = format!("{}/models/{}:generateContent", self.base_url, self.model);
                (url, vec![("x-goog-api-key", self.key.clone())], body)
            }
        }
    }

    fn send(
        &self,
        url: &str,
        headers: &[(&'static str, String)],
        body: &Value,
        timeout: Duration,
    ) -> Result<Value, AiError> {
        let network = |e: reqwest::Error| AiError::Network(e.without_url().to_string());
        let client = reqwest::blocking::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(network)?;
        self.send_with(&client, url, headers, body)
    }

    fn send_with(
        &self,
        client: &reqwest::blocking::Client,
        url: &str,
        headers: &[(&'static str, String)],
        body: &Value,
    ) -> Result<Value, AiError> {
        let network = |e: reqwest::Error| AiError::Network(e.without_url().to_string());
        let mut request = client
            .post(url)
            .header("content-type", "application/json")
            .body(body.to_string());
        for (name, value) in headers {
            request = request.header(*name, value);
        }
        let response = request.send().map_err(network)?;
        let status = response.status().as_u16();
        let text = response.text().map_err(network)?;
        check_status(status, &text)
    }

    fn read_text(&self, reply: &Value) -> Result<String, AiError> {
        let text_of = |parts: Option<&Vec<Value>>, key_filter: bool| -> String {
            parts
                .into_iter()
                .flatten()
                .filter(|p| !key_filter || p["type"] == "text")
                .filter_map(|p| p["text"].as_str())
                .collect()
        };
        let text = match self.provider {
            Provider::Claude => {
                match reply["stop_reason"].as_str() {
                    Some("max_tokens") => return Err(AiError::Truncated),
                    Some("refusal") => {
                        let why = reply["stop_details"]["explanation"].as_str().unwrap_or("");
                        return Err(AiError::Refused(why.to_owned()));
                    }
                    _ => {}
                }
                // Thinking and fallback blocks sit among the text blocks.
                text_of(reply["content"].as_array(), true)
            }
            Provider::ChatGpt => {
                let choice = &reply["choices"][0];
                if let Some(refusal) = choice["message"]["refusal"].as_str() {
                    return Err(AiError::Refused(refusal.to_owned()));
                }
                match choice["finish_reason"].as_str() {
                    Some("length") => return Err(AiError::Truncated),
                    Some("content_filter") => {
                        return Err(AiError::Refused("bộ lọc nội dung".to_owned()));
                    }
                    _ => {}
                }
                choice["message"]["content"]
                    .as_str()
                    .unwrap_or("")
                    .to_owned()
            }
            Provider::Gemini => {
                if let Some(blocked) = reply["promptFeedback"]["blockReason"].as_str() {
                    return Err(AiError::Refused(blocked.to_owned()));
                }
                let candidate = &reply["candidates"][0];
                match candidate["finishReason"].as_str() {
                    Some("MAX_TOKENS") => return Err(AiError::Truncated),
                    Some("STOP") | None => {}
                    Some(other) => return Err(AiError::Refused(other.to_owned())),
                }
                text_of(candidate["content"]["parts"].as_array(), false)
            }
        };
        if text.trim().is_empty() {
            return Err(AiError::Malformed("câu trả lời rỗng".to_owned()));
        }
        Ok(text)
    }
}

impl LlmClient for HttpClient {
    fn complete(&self, system: &str, user: &str, schema: &Value) -> Result<String, AiError> {
        let (url, headers, body) = self.request(system, user, Some(schema), MAX_OUTPUT_TOKENS);
        let reply = self.send(&url, &headers, &body, SUMMARY_TIMEOUT)?;
        self.read_text(&reply)
    }
}

fn check_status(status: u16, text: &str) -> Result<Value, AiError> {
    if (200..300).contains(&status) {
        return serde_json::from_str(text).map_err(|e| AiError::Malformed(e.to_string()));
    }
    let message = error_message(text);
    Err(match status {
        401 | 403 => AiError::Unauthorized(message),
        429 if message.contains("limit: 0") => AiError::RateLimited(
            "gói hiện tại không có hạn mức cho model này (limit: 0). Bấm \"Chọn model\" → \"Lọc model gọi được\" để chọn model tài khoản được phép dùng.".into(),
        ),
        429 => AiError::RateLimited(message),
        _ => AiError::Rejected { status, message },
    })
}

fn ids<'a>(list: &'a Value, field: &'a str) -> impl Iterator<Item = String> + 'a {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(move |m| m[field].as_str().map(str::to_owned))
}

/// OpenAI lists every model it serves; only chat models can write a meeting note.
fn is_chat_model(id: &str) -> bool {
    const NOT_CHAT: [&str; 11] = [
        "embedding",
        "tts",
        "whisper",
        "dall-e",
        "image",
        "audio",
        "realtime",
        "moderation",
        "transcribe",
        "search",
        "davinci",
    ];
    (id.starts_with("gpt-") || id.starts_with("chatgpt-") || id.starts_with('o'))
        && !NOT_CHAT.iter().any(|word| id.contains(word))
}

/// Keeps only modern text/chat models so legacy versions don't clutter the picker.
pub fn is_modern_model(provider: Provider, id: &str) -> bool {
    let s = id.to_ascii_lowercase();
    match provider {
        Provider::Claude => {
            if s.contains("claude-2") || s.contains("claude-instant") {
                return false;
            }
            if s.contains("claude-3-") && !(s.contains("3-5") || s.contains("3-7")) {
                return false;
            }
            s.starts_with("claude-")
        }
        Provider::ChatGpt => {
            const SKIP: [&str; 7] = [
                "realtime",
                "audio",
                "transcribe",
                "tts",
                "image",
                "search-preview",
                "instruct",
            ];
            if SKIP.iter().any(|w| s.contains(w)) {
                return false;
            }
            s.starts_with("gpt-4o")
                || s.starts_with("gpt-4.1")
                || s.starts_with("gpt-5")
                || s.starts_with("o3")
                || s.starts_with("o4")
        }
        Provider::Gemini => {
            if s.contains("gemini-1.")
                || s == "gemini-pro"
                || s == "gemini-ultra"
            {
                return false;
            }
            const SKIP: [&str; 10] = [
                "embedding",
                "aqa",
                "imagen",
                "veo",
                "learnlm",
                "gemma",
                "tts",
                "native-audio",
                "robotics",
                "computer-use",
            ];
            if SKIP.iter().any(|w| s.contains(w)) {
                return false;
            }
            s.contains("gemini-2.0")
                || s.contains("gemini-2.5")
                || s.contains("gemini-3")
        }
    }
}

/// Filters a provider's model list to modern models, falling back to the raw list if empty.
pub fn filter_modern_models(provider: Provider, models: Vec<String>) -> Vec<String> {
    let filtered: Vec<String> = models
        .iter()
        .filter(|id| is_modern_model(provider, id))
        .cloned()
        .collect();
    if filtered.is_empty() { models } else { filtered }
}

/// All three providers put the human-readable text at `error.message`.
fn error_message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(str::to_owned))
        .unwrap_or_else(|| body.chars().take(300).collect())
}

/// Writing style applied when summarizing transcripts, shared across all AI providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SummaryStyle {
    #[default]
    WorkEmail,
    AdministrativeNotice,
    CasualShare,
    ProductIntro,
    HowToGuide,
    CommunityNewsletter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryStyleInfo {
    pub id: SummaryStyle,
    pub label: &'static str,
    pub description: &'static str,
    pub before: &'static str,
    pub after: &'static str,
}

impl SummaryStyle {
    pub const ALL: [SummaryStyle; 6] = [
        SummaryStyle::WorkEmail,
        SummaryStyle::AdministrativeNotice,
        SummaryStyle::CasualShare,
        SummaryStyle::ProductIntro,
        SummaryStyle::HowToGuide,
        SummaryStyle::CommunityNewsletter,
    ];

    pub fn catalog() -> Vec<SummaryStyleInfo> {
        Self::ALL
            .into_iter()
            .map(|id| SummaryStyleInfo {
                id,
                label: id.label(),
                description: id.description(),
                before: id.example_before(),
                after: id.example_after(),
            })
            .collect()
    }

    pub fn example_before(self) -> &'static str {
        match self {
            SummaryStyle::WorkEmail => {
                "Mình muốn gửi thông tin về việc bản nháp đã được Lan gửi vào sáng nay. Mình nhờ bạn thực hiện việc kiểm tra bảng chi phí trước 15:00 thứ Năm; phần còn lại chưa cần duyệt."
            }
            SummaryStyle::AdministrativeNotice => {
                "Để phục vụ cho việc tổng hợp nhiệm vụ quý IV, Phòng Tổng hợp đề nghị các đơn vị thực hiện việc rà soát số liệu và tiến hành gửi báo cáo về Phòng Tổng hợp trước 16 giờ ngày 12/11/2026."
            }
            SummaryStyle::CasualShare => {
                "Hôm nay nhóm mình đã có một buổi trao đổi vô cùng hiệu quả và ý nghĩa nhằm mục đích thống nhất lịch bàn giao vào cuối tuần này để mọi người chủ động sắp xếp."
            }
            SummaryStyle::ProductIntro => {
                "Chúng tôi tự hào mang đến sổ bìa vải A5 giá 89.000đ/cuốn, lựa chọn vô cùng lý tưởng cho nhu cầu ghi chép. Giảm 10% cho đơn từ 3 cuốn đến hết ngày 15/11/2026."
            }
            SummaryStyle::HowToGuide => {
                "Trong trường hợp mã phản hồi là 429, việc thử lại yêu cầu cần được thực hiện sau ít nhất 30 giây. Việc thử lại không được thực hiện tự động với yêu cầu POST."
            }
            SummaryStyle::CommunityNewsletter => {
                "Mình rất vui mừng khi được chia sẻ với mọi người một thông tin vô cùng ý nghĩa: Chủ nhật này nhóm mình đổi sách lúc 9h ở thư viện phường. Ai có sách muốn đổi thì mang theo nhé."
            }
        }
    }

    pub fn example_after(self) -> &'static str {
        match self {
            SummaryStyle::WorkEmail => {
                "Lan đã gửi bản nháp sáng nay. Mình nhờ bạn kiểm tra bảng chi phí trước 15:00 thứ Năm; phần còn lại chưa cần duyệt."
            }
            SummaryStyle::AdministrativeNotice => {
                "Để tổng hợp nhiệm vụ quý IV, Phòng Tổng hợp đề nghị các đơn vị rà soát số liệu và gửi báo cáo về Phòng Tổng hợp trước 16 giờ ngày 12/11/2026."
            }
            SummaryStyle::CasualShare => {
                "Hôm nay nhóm đã chốt lịch bàn giao vào cuối tuần này để mọi người chủ động sắp xếp."
            }
            SummaryStyle::ProductIntro => {
                "Sổ bìa vải A5: 89.000đ/cuốn. Giảm 10% cho đơn từ 3 cuốn đến hết ngày 15/11/2026."
            }
            SummaryStyle::HowToGuide => {
                "Nếu mã phản hồi là 429, hãy đợi ít nhất 30 giây trước khi thử lại yêu cầu. Không tự động thử lại yêu cầu POST."
            }
            SummaryStyle::CommunityNewsletter => {
                "Chủ nhật này nhóm mình đổi sách lúc 9h ở thư viện phường. Ai có sách muốn đổi thì mang theo nhé."
            }
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SummaryStyle::WorkEmail => "Email công việc",
            SummaryStyle::AdministrativeNotice => "Thông báo hành chính",
            SummaryStyle::CasualShare => "Chia sẻ đời thường",
            SummaryStyle::ProductIntro => "Bài giới thiệu sản phẩm",
            SummaryStyle::HowToGuide => "Hướng dẫn thao tác",
            SummaryStyle::CommunityNewsletter => "Bản tin cộng đồng",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            SummaryStyle::WorkEmail => {
                "Lịch sự, trực tiếp. Dùng câu ngắn và nêu rõ quyết định, việc cần làm, người phụ trách và thời hạn nếu có căn cứ."
            }
            SummaryStyle::AdministrativeNotice => {
                "Trang trọng, dùng từ nhất quán. Làm rõ đối tượng áp dụng, thời điểm và điều kiện đã nêu."
            }
            SummaryStyle::CasualShare => {
                "Gần gũi, dùng từ thông dụng và câu ngắn. Giữ nguyên các thuật ngữ cần thiết."
            }
            SummaryStyle::ProductIntro => {
                "Nêu tính năng và lợi ích đã được nhắc đến. Giữ đúng mức độ chắc chắn của từng nhận định."
            }
            SummaryStyle::HowToGuide => {
                "Dùng động từ rõ nghĩa. Trình bày theo thứ tự các bước đã có, kèm điều kiện cần để thực hiện."
            }
            SummaryStyle::CommunityNewsletter => {
                "Rõ ràng, bình tĩnh. Đưa thông tin có ích cho người đọc lên trước, kèm thời gian và cách tham gia nếu cuộc họp có nêu."
            }
        }
    }

    /// Offline Vietnamese Writing Skills reference bundled directly into the app binary.
    pub fn skill_guide(self) -> &'static str {
        match self {
            SummaryStyle::WorkEmail => include_str!(
                "../prompts/vietnamese-humanizer/skills/humanizer-vi/references/business-writing.md"
            ),
            SummaryStyle::AdministrativeNotice => include_str!(
                "../prompts/vietnamese-humanizer/skills/humanizer-vi/references/administrative-writing.md"
            ),
            SummaryStyle::CasualShare => include_str!(
                "../prompts/vietnamese-humanizer/skills/humanizer-vi/references/conversational.md"
            ),
            SummaryStyle::ProductIntro => include_str!(
                "../prompts/vietnamese-humanizer/skills/humanizer-vi/references/marketing-writing.md"
            ),
            SummaryStyle::HowToGuide => include_str!(
                "../prompts/vietnamese-humanizer/skills/humanizer-vi/references/technical-writing.md"
            ),
            SummaryStyle::CommunityNewsletter => include_str!(
                "../prompts/vietnamese-humanizer/skills/humanizer-vi/references/social-writing.md"
            ),
        }
    }

    /// Shared offline preservation rules (keep facts, numbers, quotes, conditions intact).
    pub fn preservation_rules() -> &'static str {
        include_str!(
            "../prompts/vietnamese-humanizer/skills/humanizer-vi/references/preservation-rules.md"
        )
    }

    /// Vietnamese writing guidance injected into the summarization system prompt.
    pub fn prompt_rules(self) -> &'static str {
        match self {
            SummaryStyle::WorkEmail => {
                "Văn phong Email công việc: lịch sự, trực tiếp, tự nhiên như người đi làm viết tiếng Việt (tránh câu sáo rỗng). Dùng câu ngắn và nêu rõ quyết định, việc cần làm, người phụ trách và thời hạn nếu có căn cứ."
            }
            SummaryStyle::AdministrativeNotice => {
                "Văn phong Thông báo hành chính: trang trọng, khách quan, dùng từ nhất quán. Làm rõ đối tượng áp dụng, thời điểm và điều kiện đã nêu."
            }
            SummaryStyle::CasualShare => {
                "Văn phong Chia sẻ đời thường: gần gũi, dễ đọc, dùng từ thông dụng và câu ngắn tự nhiên. Giữ nguyên các thuật ngữ cần thiết."
            }
            SummaryStyle::ProductIntro => {
                "Văn phong Bài giới thiệu sản phẩm: nêu rõ tính năng và lợi ích đã được nhắc đến trong cuộc họp. Giữ đúng mức độ chắc chắn của từng nhận định, không phóng đại."
            }
            SummaryStyle::HowToGuide => {
                "Văn phong Hướng dẫn thao tác: dùng động từ rõ nghĩa, trực tiếp. Trình bày theo thứ tự các bước đã có, kèm điều kiện cần để thực hiện."
            }
            SummaryStyle::CommunityNewsletter => {
                "Văn phong Bản tin cộng đồng: rõ ràng, bình tĩnh, tự nhiên. Đưa thông tin có ích cho người đọc lên trước, kèm thời gian và cách tham gia nếu cuộc họp có nêu."
            }
        }
    }
}

/// Default provider, model names, and shared summary writing style. Keys live in `ApiKeys`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AiSettings {
    pub default_provider: Provider,
    pub models: BTreeMap<Provider, String>,
    pub summary_style: SummaryStyle,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            default_provider: Provider::Claude,
            models: Provider::ALL
                .iter()
                .map(|p| (*p, p.default_model().to_owned()))
                .collect(),
            summary_style: SummaryStyle::default(),
        }
    }
}

impl AiSettings {
    pub fn load(path: &Path) -> Self {
        let mut settings: Self = fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        // Migrate old placeholder default IDs that don't exist on public APIs.
        const STALE_DEFAULTS: [(Provider, &str); 3] = [
            (Provider::Claude, "claude-opus-5-5"),
            (Provider::ChatGpt, "gpt-6-astra"),
            (Provider::Gemini, "gemini-3.6-flash"),
        ];
        for (provider, stale) in STALE_DEFAULTS {
            if settings.models.get(&provider).map(String::as_str) == Some(stale) {
                settings
                    .models
                    .insert(provider, provider.default_model().to_owned());
            }
        }
        settings
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        crate::output::save_atomic(path, &bytes).map_err(|e| e.to_string())
    }

    pub fn model(&self, provider: Provider) -> &str {
        self.models
            .get(&provider)
            .map(String::as_str)
            .filter(|m| !m.trim().is_empty())
            .unwrap_or(provider.default_model())
    }
}

/// API keys in a plain file the user chose over the Keychain. It sits in the app's data
/// folder, readable only by the user's account, and is never written anywhere else.
pub struct ApiKeys {
    path: std::path::PathBuf,
}

impl ApiKeys {
    pub fn at(path: &Path) -> Self {
        Self {
            path: path.to_owned(),
        }
    }

    fn all(&self) -> Result<BTreeMap<Provider, String>, AiError> {
        match fs::read(&self.path) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).map_err(|e| AiError::KeyFile(e.to_string()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(e) => Err(AiError::KeyFile(e.to_string())),
        }
    }

    fn write(&self, keys: &BTreeMap<Provider, String>) -> Result<(), AiError> {
        use std::io::Write;
        let fail = |e: std::io::Error| AiError::KeyFile(e.to_string());
        let bytes = serde_json::to_vec_pretty(keys).map_err(|e| AiError::KeyFile(e.to_string()))?;
        let tmp = self.path.with_extension("tmp");
        let mut opts = fs::OpenOptions::new();
        opts.create(true).write(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut file = opts.open(&tmp).map_err(fail)?;
        file.write_all(&bytes).map_err(fail)?;
        fs::rename(&tmp, &self.path).map_err(fail)
    }

    pub fn get(&self, provider: Provider) -> Result<Option<String>, AiError> {
        Ok(self.all()?.remove(&provider).filter(|k| !k.is_empty()))
    }

    pub fn set(&self, provider: Provider, key: &str) -> Result<(), AiError> {
        let mut keys = self.all()?;
        keys.insert(provider, key.trim().to_owned());
        self.write(&keys)
    }

    pub fn remove(&self, provider: Provider) -> Result<(), AiError> {
        let mut keys = self.all()?;
        if keys.remove(&provider).is_some() {
            self.write(&keys)?;
        }
        Ok(())
    }

    /// Masked hints for the providers that have a key.
    pub fn hints(&self) -> Result<BTreeMap<Provider, String>, AiError> {
        Ok(self
            .all()?
            .into_iter()
            .filter(|(_, k)| !k.is_empty())
            .map(|(p, k)| (p, key_hint(&k)))
            .collect())
    }
}

/// What the settings screen may show: "sk-…abcd", never the key.
pub fn key_hint(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 10 {
        return "••••".to_owned();
    }
    let head: String = chars[..3].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;

    use super::*;

    struct Seen {
        head: String,
        body: Value,
    }

    /// One-shot HTTP server: answers with `reply` and reports what it received.
    fn serve(status: u16, reply: Value) -> (String, mpsc::Receiver<Seen>) {
        serve_many(vec![(status, reply)])
    }

    /// Answers each request in turn with the next reply.
    fn serve_many(replies: Vec<(u16, Value)>) -> (String, mpsc::Receiver<Seen>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            for (status, reply) in replies {
                let (mut stream, _) = listener.accept().unwrap();
                let mut data = Vec::new();
                let mut buf = [0; 8192];
                let (head, length, start) = loop {
                    let n = stream.read(&mut buf).unwrap();
                    data.extend_from_slice(&buf[..n]);
                    if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&data[..end]).to_lowercase();
                        let length = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length: "))
                            .and_then(|v| v.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        break (head, length, end + 4);
                    }
                };
                while data.len() < start + length {
                    let n = stream.read(&mut buf).unwrap();
                    data.extend_from_slice(&buf[..n]);
                }
                let body =
                    serde_json::from_slice(&data[start..start + length]).unwrap_or(Value::Null);
                let text = reply.to_string();
                let response = format!(
                    "HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
                    text.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
                tx.send(Seen { head, body }).unwrap();
            }
        });
        (url, rx)
    }

    fn call(
        provider: Provider,
        model: &str,
        status: u16,
        reply: Value,
    ) -> (Result<String, AiError>, Seen) {
        let (url, rx) = serve(status, reply);
        let client = HttpClient::with_base_url(provider, model, "khoa-bi-mat", &url);
        let result = client.complete("hệ thống", "nội dung", &json!({ "type": "object" }));
        (result, rx.recv().unwrap())
    }

    #[test]
    fn claude_request_and_reply_shape() {
        let reply = json!({ "stop_reason": "end_turn", "content": [
            { "type": "thinking", "thinking": "" },
            { "type": "fallback", "from": {}, "to": {} },
            { "type": "text", "text": "{\"a\":1}" },
        ]});
        let (result, seen) = call(Provider::Claude, "claude-sonnet-4-5", 200, reply);
        assert_eq!(result.unwrap(), "{\"a\":1}");
        assert!(seen.head.starts_with("post /messages "));
        assert!(seen.head.contains("x-api-key: khoa-bi-mat"));
        assert!(seen.head.contains("anthropic-version: 2023-06-01"));
        assert_eq!(seen.body["system"], "hệ thống");
        assert_eq!(seen.body["output_config"]["format"]["type"], "json_schema");
    }

    #[test]
    fn a_typed_in_older_claude_model_gets_no_fallback_parameter() {
        let reply =
            json!({ "stop_reason": "end_turn", "content": [{ "type": "text", "text": "x" }] });
        let (_, seen) = call(Provider::Claude, "claude-haiku-4-5", 200, reply);
        assert!(seen.body.get("fallbacks").is_none());
        assert!(!seen.head.contains("anthropic-beta"));
    }

    #[test]
    fn chatgpt_request_and_reply_shape() {
        let reply = json!({ "choices": [{ "finish_reason": "stop", "message": { "content": "ok", "refusal": null } }] });
        let (result, seen) = call(Provider::ChatGpt, "gpt-4.1-mini", 200, reply);
        assert_eq!(result.unwrap(), "ok");
        assert!(seen.head.starts_with("post /chat/completions "));
        assert!(seen.head.contains("authorization: bearer khoa-bi-mat"));
        assert_eq!(seen.body["max_completion_tokens"], 16000);
        assert_eq!(seen.body["messages"][0]["role"], "developer");
        assert_eq!(seen.body["response_format"]["json_schema"]["strict"], true);
    }

    #[test]
    fn gemini_request_and_reply_shape() {
        let reply = json!({ "candidates": [{ "finishReason": "STOP", "content": { "parts": [{ "text": "o" }, { "text": "k" }] } }] });
        let (result, seen) = call(Provider::Gemini, "gemini-2.5-flash", 200, reply);
        assert_eq!(result.unwrap(), "ok");
        assert!(
            seen.head
                .starts_with("post /models/gemini-2.5-flash:generatecontent ")
        );
        assert!(seen.head.contains("x-goog-api-key: khoa-bi-mat"));
        assert_eq!(
            seen.body["generationConfig"]["responseMimeType"],
            "application/json"
        );
        assert_eq!(
            seen.body["systemInstruction"]["parts"][0]["text"],
            "hệ thống"
        );
    }

    #[test]
    fn a_cut_off_answer_is_an_error_on_every_provider() {
        let cases = [
            (
                Provider::Claude,
                json!({ "stop_reason": "max_tokens", "content": [{ "type": "text", "text": "{" }] }),
            ),
            (
                Provider::ChatGpt,
                json!({ "choices": [{ "finish_reason": "length", "message": { "content": "{" } }] }),
            ),
            (
                Provider::Gemini,
                json!({ "candidates": [{ "finishReason": "MAX_TOKENS", "content": { "parts": [{ "text": "{" }] } }] }),
            ),
        ];
        for (provider, reply) in cases {
            let (result, _) = call(provider, "m", 200, reply);
            assert_eq!(result, Err(AiError::Truncated), "{provider:?}");
        }
    }

    #[test]
    fn a_refusal_is_reported_on_every_provider() {
        let cases = [
            (
                Provider::Claude,
                json!({ "stop_reason": "refusal", "stop_details": { "explanation": "lý do" }, "content": [] }),
            ),
            (
                Provider::ChatGpt,
                json!({ "choices": [{ "finish_reason": "stop", "message": { "content": null, "refusal": "lý do" } }] }),
            ),
            (
                Provider::Gemini,
                json!({ "promptFeedback": { "blockReason": "lý do" } }),
            ),
        ];
        for (provider, reply) in cases {
            let (result, _) = call(provider, "m", 200, reply);
            assert_eq!(
                result,
                Err(AiError::Refused("lý do".into())),
                "{provider:?}"
            );
        }
    }

    #[test]
    fn http_errors_keep_the_provider_message() {
        let body = json!({ "error": { "message": "sai key" } });
        let (result, _) = call(Provider::Claude, "m", 401, body.clone());
        assert_eq!(result, Err(AiError::Unauthorized("sai key".into())));
        let (result, _) = call(Provider::Gemini, "m", 429, body.clone());
        assert_eq!(result, Err(AiError::RateLimited("sai key".into())));
        let (result, _) = call(Provider::ChatGpt, "m", 400, body);
        assert_eq!(
            result,
            Err(AiError::Rejected {
                status: 400,
                message: "sai key".into()
            })
        );
    }

    fn list(provider: Provider, replies: Vec<Value>) -> (Result<Vec<String>, AiError>, Vec<Seen>) {
        let count = replies.len();
        let (url, rx) = serve_many(replies.into_iter().map(|r| (200, r)).collect());
        let result = HttpClient::with_base_url(provider, "m", "k", &url).list_models();
        (result, rx.iter().take(count).collect())
    }

    #[test]
    fn claude_models_follow_every_page() {
        let (models, seen) = list(
            Provider::Claude,
            vec![
                json!({ "data": [{ "id": "claude-opus-5-5" }], "has_more": true, "last_id": "claude-opus-5-5" }),
                json!({ "data": [{ "id": "claude-haiku-5-5" }], "has_more": false }),
            ],
        );
        assert_eq!(models.unwrap(), ["claude-opus-5-5", "claude-haiku-5-5"]);
        assert!(seen[0].head.starts_with("get /models?limit=1000 "));
        assert!(seen[0].head.contains("x-api-key: k"));
        assert!(seen[1].head.contains("after_id=claude-opus-5-5"));
    }

    #[test]
    fn openai_models_keep_only_chat_models() {
        let (models, seen) = list(
            Provider::ChatGpt,
            vec![json!({ "data": [
                { "id": "gpt-6-astra" }, { "id": "text-embedding-3-large" }, { "id": "gpt-6-luna" },
                { "id": "whisper-1" }, { "id": "gpt-4o-mini-tts" }, { "id": "dall-e-3" }, { "id": "o4-mini" },
            ]})],
        );
        assert_eq!(models.unwrap(), ["o4-mini", "gpt-6-luna", "gpt-6-astra"]);
        assert!(seen[0].head.contains("authorization: bearer k"));
    }

    #[test]
    fn gemini_models_need_generate_content_and_follow_pages() {
        let (models, seen) = list(
            Provider::Gemini,
            vec![
                json!({ "models": [
                { "name": "models/gemini-3.6-flash", "supportedGenerationMethods": ["generateContent", "countTokens"] },
                { "name": "models/text-embedding-005", "supportedGenerationMethods": ["embedContent"] },
            ], "nextPageToken": "p2" }),
                json!({ "models": [{ "name": "models/gemini-3.1-pro", "supportedGenerationMethods": ["generateContent"] }] }),
            ],
        );
        assert_eq!(models.unwrap(), ["gemini-3.6-flash", "gemini-3.1-pro"]);
        assert!(seen[0].head.contains("x-goog-api-key: k"));
        assert!(seen[1].head.contains("pagetoken=p2"));
    }

    #[test]
    fn a_bad_key_when_listing_models_is_explained() {
        let (url, _rx) = serve(401, json!({ "error": { "message": "sai key" } }));
        let result = HttpClient::with_base_url(Provider::Gemini, "m", "k", &url).list_models();
        assert_eq!(result, Err(AiError::Unauthorized("sai key".into())));
    }

    #[test]
    fn ping_only_needs_a_success_status() {
        let (url, _rx) = serve(200, json!({ "stop_reason": "max_tokens", "content": [] }));
        assert_eq!(
            HttpClient::with_base_url(Provider::Claude, "m", "k", &url).ping(),
            Ok(())
        );
    }

    #[test]
    fn keys_round_trip_in_a_file_only_the_user_can_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys.json");
        let keys = ApiKeys::at(&path);
        assert_eq!(keys.get(Provider::Claude), Ok(None));
        keys.set(Provider::Claude, " sk-ant-1234567890abcd ")
            .unwrap();
        keys.set(Provider::Gemini, "AIzaSy-0987654321qqV0").unwrap();
        assert_eq!(
            keys.get(Provider::Claude).unwrap().as_deref(),
            Some("sk-ant-1234567890abcd")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert_eq!(
            keys.hints()
                .unwrap()
                .get(&Provider::Gemini)
                .map(String::as_str),
            Some("AIz…qqV0")
        );
        keys.remove(Provider::Claude).unwrap();
        assert_eq!(keys.get(Provider::Claude), Ok(None));
        assert_eq!(keys.hints().unwrap().len(), 1);
    }

    #[test]
    fn the_hint_never_reveals_the_key() {
        assert_eq!(key_hint("sk-ant-1234567890abcd"), "sk-…abcd");
        assert_eq!(key_hint("short"), "••••");
    }

    #[test]
    fn settings_round_trip_and_blank_model_falls_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ai.json");
        assert_eq!(AiSettings::load(&path), AiSettings::default());
        let mut settings = AiSettings {
            default_provider: Provider::Gemini,
            ..Default::default()
        };
        settings.models.insert(Provider::Claude, "  ".into());
        settings.save(&path).unwrap();
        let loaded = AiSettings::load(&path);
        assert_eq!(loaded.default_provider, Provider::Gemini);
        assert_eq!(loaded.model(Provider::Claude), "claude-sonnet-4-5");
        assert!(
            !fs::read_to_string(&path)
                .unwrap()
                .to_lowercase()
                .contains("key")
        );
    }

    #[test]
    fn legacy_settings_without_summary_style_default_to_work_email() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ai.json");
        fs::write(
            &path,
            r#"{"defaultProvider":"chatGpt","models":{"chatGpt":"gpt-4.1","gemini":"gemini-3.6-flash"}}"#,
        )
        .unwrap();
        let loaded = AiSettings::load(&path);
        assert_eq!(loaded.default_provider, Provider::ChatGpt);
        assert_eq!(loaded.model(Provider::ChatGpt), "gpt-4.1");
        assert_eq!(loaded.model(Provider::Gemini), "gemini-2.5-flash");
        assert_eq!(loaded.summary_style, SummaryStyle::WorkEmail);
    }

    #[test]
    fn summary_style_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ai.json");
        let settings = AiSettings {
            summary_style: SummaryStyle::HowToGuide,
            ..Default::default()
        };
        settings.save(&path).unwrap();
        assert_eq!(AiSettings::load(&path).summary_style, SummaryStyle::HowToGuide);
    }

    #[test]
    fn modern_model_filtering_keeps_current_and_drops_legacy() {
        assert!(is_modern_model(Provider::Claude, "claude-sonnet-4-5"));
        assert!(is_modern_model(Provider::Claude, "claude-haiku-4-5-20251001"));
        assert!(is_modern_model(Provider::Claude, "claude-3-7-sonnet-latest"));
        assert!(!is_modern_model(Provider::Claude, "claude-3-haiku-20240307"));
        assert!(!is_modern_model(Provider::Claude, "claude-2.1"));

        assert!(is_modern_model(Provider::ChatGpt, "gpt-4o"));
        assert!(is_modern_model(Provider::ChatGpt, "gpt-4.1-mini"));
        assert!(is_modern_model(Provider::ChatGpt, "o4-mini"));
        assert!(!is_modern_model(Provider::ChatGpt, "gpt-4o-realtime-preview"));
        assert!(!is_modern_model(Provider::ChatGpt, "gpt-3.5-turbo"));

        assert!(is_modern_model(Provider::Gemini, "gemini-2.5-flash"));
        assert!(is_modern_model(Provider::Gemini, "gemini-2.5-pro"));
        assert!(is_modern_model(Provider::Gemini, "gemini-2.0-flash"));
        assert!(!is_modern_model(Provider::Gemini, "gemini-1.5-pro"));
        assert!(!is_modern_model(Provider::Gemini, "gemini-2.5-flash-tts"));

        // Falls back to raw list if filtering would empty it.
        assert_eq!(
            filter_modern_models(Provider::Claude, vec!["claude-3-haiku".into()]),
            vec!["claude-3-haiku".to_owned()]
        );
    }

    #[test]
    fn limit_zero_quota_error_gives_actionable_hint() {
        let body = r#"{"error":{"message":"Quota exceeded for metric: free_tier_requests, limit: 0, model: gemini-4-carbon"}}"#;
        let err = check_status(429, body).unwrap_err();
        assert!(err.to_string().contains("limit: 0"));
        assert!(err.to_string().contains("Lọc model gọi được"));
    }
}
