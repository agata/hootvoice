use anyhow::{anyhow, Context, Result};
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE, USER_AGENT};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::min;
use std::collections::HashSet;
use std::sync::Mutex;
use std::time::{Duration, Instant};

mod history;

/// Default API base URL for OpenAI 互換ローカルエンドポイント (例: Ollama)。
pub const DEFAULT_LOCAL_BASE_URL: &str = "http://localhost:11434/v1";
/// Local provider default model.
pub const DEFAULT_LOCAL_MODEL: &str = "llama3.1:8b";
/// Default maximum number of input characters sent to the LLM.
pub const DEFAULT_MAX_INPUT_CHARS: usize = 4_000;
/// Default request timeout in seconds.
pub const DEFAULT_TIMEOUT_SECS: u64 = 30;

const USER_AGENT_VALUE: &str = concat!("hootvoice/", env!("CARGO_PKG_VERSION"));
const CHAT_COMPLETIONS_PATH: &str = "chat/completions";
const MODELS_PATH: &str = "models";
const BACKOFF_FAILURES: u32 = 3;
const BACKOFF_SECS: u64 = 60;
const MAX_ERROR_BODY_PREVIEW: usize = 300;
const GLOBAL_LOCALE: &str = "global";
const LOCALE_JA_JP: &str = "ja-JP";
const LOCALE_EN_US: &str = "en-US";
const PLACEHOLDER_TRANSCRIPT: &str = "{{transcript}}";
const PLACEHOLDER_DICTIONARY: &str = "{{dictionary}}";
const FORMAT_SYSTEM_JA: &str = "ユーザーは文字起こしされたテキストを送ってくるので内容を確認して、文字起こしで欠損したり誤変換した単語などを全体の文脈を考慮して修正してください。段落ごとに改行や空行を積極的に使って、読みやすい構造にしてください。結果は修正後のテキストのみを返却します。修正が必要ない場合は元の文章のみを返します。出力する文字列には校正後の文章以外は一切含まないこと。「えーと」「あー」などの人が話す際に発した不要な情報は除去します。";
const FORMAT_SYSTEM_EN: &str = "You receive an automatic transcript. Fix recognition mistakes, add punctuation, keep a neutral narrator style, and remove filler words such as \"um\" or \"uh\". Return only the corrected text.";
const FORMAT_SYSTEM_GLOBAL: &str = "You receive an automatic transcript. Clean it up, fix recognition mistakes, add punctuation, and remove filler words. Return only the corrected text in the same language as the input.";
const SUMMARY_SYSTEM_JA: &str = "以下の文字起こしを最大5つの簡潔な箇条書きで日本語のまま要約してください。各行は \"- \" で開始し、余計な前置きや感想は入れないでください。";
const SUMMARY_SYSTEM_EN: &str = "Summarize the transcript into at most five concise bullet points written in English. Start each bullet with \"- \" and avoid any commentary.";
const SUMMARY_SYSTEM_GLOBAL: &str = "Summarize the transcript into at most five concise bullet points. Prefer the transcript language when obvious, otherwise use English. Start each bullet with \"- \".";
const FORMAT_USER_JA: &str = "校正対象:\n{{transcript}}";
const FORMAT_USER_EN: &str = "Transcript to revise:\n{{transcript}}";
const FORMAT_USER_GLOBAL: &str = "Transcript:\n{{transcript}}";
const SUMMARY_USER_DEFAULT: &str = "{{transcript}}";
pub const PRESET_ID_FORMAT: &str = "preset:format";
pub const PRESET_ID_SUMMARY: &str = "preset:summary";
pub const MODE_ID_CUSTOM_DRAFT: &str = "custom:draft";

pub use history::{
    history_file_path, history_modified_time, load_entries as load_history_entries,
    record_entry as record_history, LlmHistoryEntry, MAX_HISTORY_ENTRIES,
};

fn default_mode_id() -> String {
    PRESET_ID_FORMAT.to_string()
}

fn is_builtin_mode(id: &str) -> bool {
    matches!(id, PRESET_ID_FORMAT | PRESET_ID_SUMMARY)
}

fn generate_custom_mode_id(existing: &HashSet<String>, name: &str) -> String {
    let mut slug = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else if c.is_whitespace() || c == '-' || c == '_' {
                '-'
            } else {
                '-'
            }
        })
        .collect::<String>();

    if slug.trim_matches('-').is_empty() {
        slug = "custom-mode".to_string();
    } else {
        while slug.contains("--") {
            slug = slug.replace("--", "-");
        }
        slug = slug.trim_matches('-').to_string();
        if slug.is_empty() {
            slug = "custom-mode".to_string();
        }
    }

    let mut candidate = format!("custom:{}", slug);
    let mut counter = 2;
    while existing.contains(&candidate) {
        candidate = format!("custom:{}-{}", slug, counter);
        counter += 1;
    }
    candidate
}

pub fn builtin_prompt_preview(mode_id: &str, locales: &[String]) -> Option<(String, String)> {
    if !is_builtin_mode(mode_id) {
        return None;
    }
    match mode_id {
        PRESET_ID_FORMAT => Some(format_prompt_strings(locales)),
        PRESET_ID_SUMMARY => Some(summary_prompt_strings(locales)),
        _ => None,
    }
}

/// User configurable LLM post processing settings persisted in settings.toml.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LlmPostProcessSettings {
    pub enabled: bool,
    pub api_base_url: String,
    pub model: String,
    #[serde(default = "default_mode_id", alias = "mode")]
    pub mode_id: String,
    #[serde(default)]
    pub custom_prompts: Vec<CustomPromptMode>,
    #[serde(default)]
    pub custom_prompt_name: String,
    pub language_override: Option<String>,
    #[serde(default)]
    pub custom_prompt_system: String,
    #[serde(default)]
    pub custom_prompt: String,
    pub max_input_chars: usize,
    pub timeout_secs: u64,
    pub apply_to_autopaste: bool,
}

/// User defined custom prompt mode stored in settings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CustomPromptMode {
    pub id: String,
    pub name: String,
    pub system_prompt: Option<String>,
    pub user_prompt: String,
}

impl Default for LlmPostProcessSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            api_base_url: DEFAULT_LOCAL_BASE_URL.to_string(),
            model: DEFAULT_LOCAL_MODEL.to_string(),
            mode_id: default_mode_id(),
            custom_prompts: Vec::new(),
            custom_prompt_name: "Custom prompt".to_string(),
            language_override: None,
            custom_prompt_system: String::new(),
            custom_prompt: "{{transcript}}".to_string(),
            max_input_chars: DEFAULT_MAX_INPUT_CHARS,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            apply_to_autopaste: true,
        }
    }
}

/// Metadata describing a chat-completion capable model.
#[derive(Debug, Clone)]
pub struct LlmModelInfo {
    pub id: String,
    pub label: String,
}

/// Result of running a connection test against the configured endpoint.
#[derive(Debug, Clone)]
pub struct ConnectionTestOutcome {
    pub status: Option<u16>,
    pub duration_ms: u128,
    pub message: String,
}

/// Successful response from the LLM post-processing step.
#[derive(Debug, Clone)]
pub struct PostProcessOutcome {
    pub content: String,
    pub truncated_input: bool,
    pub latency_ms: u128,
}

/// Error information returned when an LLM request fails.
#[derive(Debug, Clone)]
pub struct LlmRequestError {
    pub message: String,
    pub status: Option<u16>,
    pub retry_after_secs: Option<u64>,
}

pub type LlmResult<T> = std::result::Result<T, LlmRequestError>;

#[derive(Default, Debug)]
struct RetryState {
    consecutive_failures: u32,
    next_retry_at: Option<Instant>,
}

/// Simple retry/backoff manager for LLM post-processing calls.
#[derive(Debug, Default)]
pub struct LlmPostProcessor {
    state: Mutex<RetryState>,
}

impl LlmPostProcessSettings {
    /// Returns the effective API base URL, forcing the OpenAI default when that provider is used.
    pub fn effective_base_url(&self) -> String {
        if self.api_base_url.trim().is_empty() {
            DEFAULT_LOCAL_BASE_URL.to_string()
        } else {
            self.api_base_url.trim().to_string()
        }
    }

    /// Returns the effective model placeholder default.
    pub fn default_model() -> &'static str {
        DEFAULT_LOCAL_MODEL
    }

    /// Returns the model name ensuring defaults when settings still carry a previous provider.
    pub fn effective_model(&self) -> String {
        if self.model.trim().is_empty() {
            Self::default_model().to_string()
        } else {
            self.model.trim().to_string()
        }
    }

    /// Returns locale priority list for prompt resolution.
    pub fn locale_priority(&self, language_hint: Option<&str>) -> Vec<String> {
        let mut locales = Vec::new();
        if let Some(locale) = self
            .language_override
            .as_deref()
            .and_then(normalize_locale_code)
        {
            locales.push(locale);
        } else if let Some(locale) = language_hint.and_then(normalize_locale_code) {
            locales.push(locale);
        }
        if !locales
            .iter()
            .any(|l| l.eq_ignore_ascii_case(GLOBAL_LOCALE))
        {
            locales.push(GLOBAL_LOCALE.to_string());
        }
        locales
    }

    fn unique_custom_name(&self, raw_name: &str) -> String {
        let base = {
            let trimmed = raw_name.trim();
            if trimmed.is_empty() {
                "Custom prompt".to_string()
            } else {
                trimmed.to_string()
            }
        };

        if !self.custom_prompts.iter().any(|mode| mode.name == base) {
            return base;
        }

        let mut counter = 2;
        loop {
            let candidate = format!("{} ({})", base, counter);
            if !self
                .custom_prompts
                .iter()
                .any(|mode| mode.name == candidate)
            {
                return candidate;
            }
            counter += 1;
        }
    }

    pub fn custom_prompt(&self, id: &str) -> Option<&CustomPromptMode> {
        self.custom_prompts.iter().find(|mode| mode.id == id)
    }

    pub fn custom_prompt_mut(&mut self, id: &str) -> Option<&mut CustomPromptMode> {
        self.custom_prompts.iter_mut().find(|mode| mode.id == id)
    }

    pub fn ensure_mode_valid(&mut self) {
        if self.mode_id.is_empty() {
            self.mode_id = default_mode_id();
        }
        if self.mode_id == "custom" || self.mode_id == "preset:custom" {
            self.mode_id = MODE_ID_CUSTOM_DRAFT.to_string();
        }
        if self.mode_id != MODE_ID_CUSTOM_DRAFT
            && !is_builtin_mode(&self.mode_id)
            && self.custom_prompt(&self.mode_id).is_none()
        {
            self.mode_id = MODE_ID_CUSTOM_DRAFT.to_string();
        }
    }

    pub fn begin_custom_draft(&mut self, language_hint: Option<&str>) {
        self.mode_id = MODE_ID_CUSTOM_DRAFT.to_string();
        self.custom_prompt_name.clear();
        let locales = self.locale_priority(language_hint);
        let (system, user) = format_prompt_strings(&locales);
        self.custom_prompt_system = system;
        self.custom_prompt = user;
    }

    pub fn create_custom_mode(
        &mut self,
        name: &str,
        system_prompt: &str,
        user_prompt: &str,
    ) -> String {
        let final_name = self.unique_custom_name(name);
        let mut existing: HashSet<String> = self
            .custom_prompts
            .iter()
            .map(|mode| mode.id.clone())
            .collect();
        existing.insert(PRESET_ID_FORMAT.to_string());
        existing.insert(PRESET_ID_SUMMARY.to_string());
        existing.insert(MODE_ID_CUSTOM_DRAFT.to_string());

        let id = generate_custom_mode_id(&existing, &final_name);
        let mode = CustomPromptMode {
            id: id.clone(),
            name: final_name,
            system_prompt: if system_prompt.trim().is_empty() {
                None
            } else {
                Some(system_prompt.to_string())
            },
            user_prompt: user_prompt.to_string(),
        };
        self.custom_prompts.push(mode);
        id
    }

    pub fn update_custom_mode(
        &mut self,
        id: &str,
        name: &str,
        system_prompt: &str,
        user_prompt: &str,
    ) -> Result<(), ()> {
        if let Some(mode) = self.custom_prompt_mut(id) {
            mode.name = name.to_string();
            mode.system_prompt = if system_prompt.trim().is_empty() {
                None
            } else {
                Some(system_prompt.to_string())
            };
            mode.user_prompt = user_prompt.to_string();
            Ok(())
        } else {
            Err(())
        }
    }

    pub fn remove_custom_mode(&mut self, id: &str) -> bool {
        let before = self.custom_prompts.len();
        self.custom_prompts.retain(|mode| mode.id != id);
        if before != self.custom_prompts.len() {
            if self.mode_id == id {
                self.mode_id = default_mode_id();
            }
            true
        } else {
            false
        }
    }
}

fn normalize_locale_code(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_lowercase();
    if lower == "auto" {
        return None;
    }
    let parts: Vec<&str> = trimmed
        .split(|c| c == '-' || c == '_')
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        return None;
    }
    let language = parts[0].to_lowercase();
    if parts.len() >= 2 {
        let region = parts[1].to_uppercase();
        return Some(format!("{}-{}", language, region));
    }
    match language.as_str() {
        "ja" => Some(LOCALE_JA_JP.to_string()),
        "en" => Some(LOCALE_EN_US.to_string()),
        _ => Some(language),
    }
}

impl LlmPostProcessor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn process(
        &self,
        settings: &LlmPostProcessSettings,
        transcript: &str,
        dictionary_hint: Option<&str>,
        language_hint: Option<&str>,
    ) -> LlmResult<PostProcessOutcome> {
        if !settings.enabled {
            return Err(LlmRequestError {
                message: "LLM post-processing is disabled".to_string(),
                status: None,
                retry_after_secs: None,
            });
        }

        if let Some(wait) = self.check_backoff() {
            return Err(LlmRequestError {
                message: format!("Backoff active. Retry after {}s.", wait),
                status: None,
                retry_after_secs: Some(wait),
            });
        }

        let trimmed = transcript.trim();
        if trimmed.is_empty() {
            return Err(LlmRequestError {
                message: "Transcript is empty".to_string(),
                status: None,
                retry_after_secs: None,
            });
        }

        let (prepared, truncated) = prepare_transcript(trimmed, settings.max_input_chars);
        if prepared.is_empty() {
            return Err(LlmRequestError {
                message: "Transcript is empty after trimming".to_string(),
                status: None,
                retry_after_secs: None,
            });
        }

        let payload = build_chat_payload(settings, &prepared, dictionary_hint, language_hint);
        match execute_chat_completion(settings, &payload) {
            Ok((response, status, latency_ms)) => {
                if let Some(content) = extract_first_choice_text(&response) {
                    self.note_success();
                    let polished = content.trim().to_string();
                    return Ok(PostProcessOutcome {
                        content: if polished.is_empty() {
                            content
                        } else {
                            polished
                        },
                        truncated_input: truncated,
                        latency_ms,
                    });
                }
                let mut err = LlmRequestError {
                    message: "Missing content field".to_string(),
                    status: Some(status.as_u16()),
                    retry_after_secs: None,
                };
                if let Some(wait) = self.register_failure() {
                    err.retry_after_secs = err.retry_after_secs.or(Some(wait));
                }
                Err(err)
            }
            Err(mut err) => {
                if let Some(wait) = self.register_failure() {
                    err.retry_after_secs = err.retry_after_secs.or(Some(wait));
                }
                Err(err)
            }
        }
    }

    fn check_backoff(&self) -> Option<u64> {
        let mut state = self.state.lock().unwrap();
        if let Some(next) = state.next_retry_at {
            if let Some(remaining) = next.checked_duration_since(Instant::now()) {
                return Some(remaining.as_secs().max(1));
            }
            state.next_retry_at = None;
        }
        None
    }

    fn register_failure(&self) -> Option<u64> {
        let mut state = self.state.lock().unwrap();
        state.consecutive_failures += 1;
        if state.consecutive_failures >= BACKOFF_FAILURES {
            state.consecutive_failures = 0;
            let wait = Duration::from_secs(BACKOFF_SECS);
            state.next_retry_at = Some(Instant::now() + wait);
            Some(wait.as_secs())
        } else {
            None
        }
    }

    fn note_success(&self) {
        let mut state = self.state.lock().unwrap();
        state.consecutive_failures = 0;
        state.next_retry_at = None;
    }
}

#[derive(Serialize)]
struct ChatCompletionPayload {
    model: String,
    messages: Vec<ChatMessagePayload>,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Serialize)]
struct ChatMessagePayload {
    role: &'static str,
    content: String,
}

#[derive(Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: Option<ChatMessage>,
}

#[derive(Deserialize)]
struct ChatMessage {
    content: Option<Value>,
}

fn prepare_transcript(text: &str, max_chars: usize) -> (String, bool) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return (String::new(), false);
    }

    if max_chars == 0 {
        return (trimmed.to_string(), false);
    }

    let mut out = String::with_capacity(min(trimmed.len(), max_chars));
    let mut truncated = false;
    for (idx, ch) in trimmed.chars().enumerate() {
        if idx == max_chars {
            truncated = true;
            break;
        }
        out.push(ch);
    }
    (out, truncated)
}

struct PromptTemplateResolved {
    system: Option<String>,
    user: String,
}

impl PromptTemplateResolved {
    fn apply_dictionary(mut self, dictionary: &str) -> Self {
        if let Some(ref mut system) = self.system {
            let replaced = inject_dictionary(std::mem::take(system), dictionary);
            *system = replaced;
        }
        self.user = inject_dictionary(self.user, dictionary);
        self
    }
}

fn build_chat_payload(
    settings: &LlmPostProcessSettings,
    transcript: &str,
    dictionary_hint: Option<&str>,
    language_hint: Option<&str>,
) -> ChatCompletionPayload {
    let dictionary = dictionary_hint.unwrap_or_default();
    let resolved = resolve_prompt(settings, transcript, dictionary, language_hint);
    ChatCompletionPayload {
        model: settings.effective_model(),
        messages: prompt_to_messages(resolved),
        temperature: 0.2,
        max_tokens: Some(1024),
    }
}

fn resolve_prompt(
    settings: &LlmPostProcessSettings,
    transcript: &str,
    dictionary: &str,
    language_hint: Option<&str>,
) -> PromptTemplateResolved {
    let mut mode_id = settings.mode_id.trim();
    if mode_id.is_empty() {
        mode_id = PRESET_ID_FORMAT;
    }

    if let Some(resolved) =
        resolve_builtin_prompt(mode_id, settings, transcript, dictionary, language_hint)
    {
        return resolved;
    }

    if mode_id == MODE_ID_CUSTOM_DRAFT {
        return custom_prompt_to_resolved(
            settings.custom_prompt_system.as_str(),
            settings.custom_prompt.as_str(),
            transcript,
            dictionary,
        );
    }

    if let Some(custom) = settings.custom_prompt(mode_id) {
        return custom_prompt_to_resolved(
            custom.system_prompt.as_deref().unwrap_or_default(),
            custom.user_prompt.as_str(),
            transcript,
            dictionary,
        );
    }

    custom_prompt_to_resolved(
        settings.custom_prompt_system.as_str(),
        settings.custom_prompt.as_str(),
        transcript,
        dictionary,
    )
}

fn resolve_builtin_prompt(
    mode_id: &str,
    settings: &LlmPostProcessSettings,
    transcript: &str,
    dictionary: &str,
    language_hint: Option<&str>,
) -> Option<PromptTemplateResolved> {
    if !is_builtin_mode(mode_id) {
        return None;
    }
    let locales = settings.locale_priority(language_hint);
    let resolved = match mode_id {
        PRESET_ID_FORMAT => format_prompt_for_locales(&locales, transcript),
        PRESET_ID_SUMMARY => summary_prompt_for_locales(&locales, transcript),
        _ => return None,
    };
    Some(resolved.apply_dictionary(dictionary))
}

fn format_prompt_for_locales(locales: &[String], transcript: &str) -> PromptTemplateResolved {
    for locale in locales {
        if let Some(resolved) = try_format_prompt(locale, transcript) {
            return resolved;
        }
    }
    PromptTemplateResolved {
        system: Some(FORMAT_SYSTEM_GLOBAL.to_string()),
        user: format!("Transcript:\n{}", transcript),
    }
}

fn format_prompt_strings(locales: &[String]) -> (String, String) {
    for locale in locales {
        if let Some(pair) = format_prompt_template_for_locale(locale) {
            return pair;
        }
    }
    (
        FORMAT_SYSTEM_GLOBAL.to_string(),
        FORMAT_USER_GLOBAL.to_string(),
    )
}

fn format_prompt_template_for_locale(locale: &str) -> Option<(String, String)> {
    let normalized = locale.to_ascii_lowercase();
    match normalized.as_str() {
        "ja-jp" => Some((FORMAT_SYSTEM_JA.to_string(), FORMAT_USER_JA.to_string())),
        "en-us" => Some((FORMAT_SYSTEM_EN.to_string(), FORMAT_USER_EN.to_string())),
        "global" => Some((
            FORMAT_SYSTEM_GLOBAL.to_string(),
            FORMAT_USER_GLOBAL.to_string(),
        )),
        _ => None,
    }
}

fn try_format_prompt(locale: &str, transcript: &str) -> Option<PromptTemplateResolved> {
    let normalized = locale.to_ascii_lowercase();
    match normalized.as_str() {
        "ja-jp" => Some(PromptTemplateResolved {
            system: Some(FORMAT_SYSTEM_JA.to_string()),
            user: format!("校正対象:\n{}", transcript),
        }),
        "en-us" => Some(PromptTemplateResolved {
            system: Some(FORMAT_SYSTEM_EN.to_string()),
            user: format!("Transcript to revise:\n{}", transcript),
        }),
        "global" => Some(PromptTemplateResolved {
            system: Some(FORMAT_SYSTEM_GLOBAL.to_string()),
            user: format!("Transcript:\n{}", transcript),
        }),
        _ => None,
    }
}

fn summary_prompt_for_locales(locales: &[String], transcript: &str) -> PromptTemplateResolved {
    for locale in locales {
        if let Some(resolved) = try_summary_prompt(locale, transcript) {
            return resolved;
        }
    }
    PromptTemplateResolved {
        system: Some(SUMMARY_SYSTEM_GLOBAL.to_string()),
        user: transcript.to_string(),
    }
}

fn summary_prompt_strings(locales: &[String]) -> (String, String) {
    for locale in locales {
        if let Some(pair) = summary_prompt_template_for_locale(locale) {
            return pair;
        }
    }
    (
        SUMMARY_SYSTEM_GLOBAL.to_string(),
        SUMMARY_USER_DEFAULT.to_string(),
    )
}

fn summary_prompt_template_for_locale(locale: &str) -> Option<(String, String)> {
    let normalized = locale.to_ascii_lowercase();
    match normalized.as_str() {
        "ja-jp" => Some((
            SUMMARY_SYSTEM_JA.to_string(),
            SUMMARY_USER_DEFAULT.to_string(),
        )),
        "en-us" => Some((
            SUMMARY_SYSTEM_EN.to_string(),
            SUMMARY_USER_DEFAULT.to_string(),
        )),
        "global" => Some((
            SUMMARY_SYSTEM_GLOBAL.to_string(),
            SUMMARY_USER_DEFAULT.to_string(),
        )),
        _ => None,
    }
}

fn try_summary_prompt(locale: &str, transcript: &str) -> Option<PromptTemplateResolved> {
    let normalized = locale.to_ascii_lowercase();
    match normalized.as_str() {
        "ja-jp" => Some(PromptTemplateResolved {
            system: Some(SUMMARY_SYSTEM_JA.to_string()),
            user: transcript.to_string(),
        }),
        "en-us" => Some(PromptTemplateResolved {
            system: Some(SUMMARY_SYSTEM_EN.to_string()),
            user: transcript.to_string(),
        }),
        "global" => Some(PromptTemplateResolved {
            system: Some(SUMMARY_SYSTEM_GLOBAL.to_string()),
            user: transcript.to_string(),
        }),
        _ => None,
    }
}

fn custom_prompt_to_resolved(
    system_prompt: &str,
    user_prompt: &str,
    transcript: &str,
    dictionary: &str,
) -> PromptTemplateResolved {
    let system = if system_prompt.trim().is_empty() {
        None
    } else {
        Some(render_with_placeholders(system_prompt, transcript, dictionary).0)
    };

    let (rendered_user, had_transcript) =
        render_with_placeholders(user_prompt, transcript, dictionary);
    if had_transcript {
        PromptTemplateResolved {
            system,
            user: rendered_user,
        }
    } else {
        let mut content = rendered_user;
        if content.trim().is_empty() {
            content.push_str("Transcript:\n");
        } else {
            content.push_str("\n\nTranscript:\n");
        }
        content.push_str(transcript);
        PromptTemplateResolved {
            system,
            user: content,
        }
    }
}

fn prompt_to_messages(resolved: PromptTemplateResolved) -> Vec<ChatMessagePayload> {
    let mut messages = Vec::new();
    if let Some(system) = resolved.system {
        messages.push(ChatMessagePayload {
            role: "system",
            content: system,
        });
    }
    messages.push(ChatMessagePayload {
        role: "user",
        content: resolved.user,
    });
    messages
}

fn render_with_placeholders(template: &str, transcript: &str, dictionary: &str) -> (String, bool) {
    let mut rendered = template.to_string();
    let has_transcript = rendered.contains(PLACEHOLDER_TRANSCRIPT);
    if has_transcript {
        rendered = rendered.replace(PLACEHOLDER_TRANSCRIPT, transcript);
    }
    if rendered.contains(PLACEHOLDER_DICTIONARY) {
        rendered = rendered.replace(PLACEHOLDER_DICTIONARY, dictionary);
    }
    (rendered, has_transcript)
}

fn inject_dictionary(text: String, dictionary: &str) -> String {
    if text.contains(PLACEHOLDER_DICTIONARY) {
        text.replace(PLACEHOLDER_DICTIONARY, dictionary)
    } else {
        text
    }
}

fn execute_chat_completion(
    settings: &LlmPostProcessSettings,
    payload: &ChatCompletionPayload,
) -> LlmResult<(ChatCompletionResponse, StatusCode, u128)> {
    let client = build_client_with_timeout(settings.timeout_secs).map_err(|e| LlmRequestError {
        message: format!("Failed to create HTTP client: {}", e),
        status: None,
        retry_after_secs: None,
    })?;

    let headers = create_headers(settings, true).map_err(|e| LlmRequestError {
        message: e.to_string(),
        status: None,
        retry_after_secs: None,
    })?;

    let url = join_url(&settings.effective_base_url(), CHAT_COMPLETIONS_PATH);
    let start = Instant::now();
    let response = client
        .post(&url)
        .headers(headers)
        .json(payload)
        .send()
        .map_err(map_reqwest_error)?;

    let status = response.status();
    let headers_snapshot = response.headers().clone();
    let elapsed_ms = start.elapsed().as_millis();
    let body = response.text().unwrap_or_default();

    if !status.is_success() {
        let mut retry_after_secs = parse_retry_after_secs(&headers_snapshot);
        if status.as_u16() == 429 && retry_after_secs.is_none() {
            retry_after_secs = Some(BACKOFF_SECS);
        }
        let snippet = preview_body(&body);
        return Err(LlmRequestError {
            message: format!("HTTP {} {}", status.as_u16(), snippet),
            status: Some(status.as_u16()),
            retry_after_secs,
        });
    }

    let parsed: ChatCompletionResponse =
        serde_json::from_str(&body).map_err(|e| LlmRequestError {
            message: format!("Failed to parse JSON: {}", e),
            status: Some(status.as_u16()),
            retry_after_secs: None,
        })?;

    Ok((parsed, status, elapsed_ms))
}

fn extract_first_choice_text(resp: &ChatCompletionResponse) -> Option<String> {
    resp.choices
        .get(0)
        .and_then(|choice| choice.message.as_ref())
        .and_then(|msg| msg.content.as_ref())
        .and_then(extract_content_value)
}

fn extract_content_value(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.trim().to_string()),
        Value::Array(parts) => {
            let mut buf = String::new();
            for part in parts {
                if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                    buf.push_str(text);
                    buf.push('\n');
                } else if let Some(text) = part.get("content").and_then(|v| v.as_str()) {
                    buf.push_str(text);
                    buf.push('\n');
                }
            }
            if buf.is_empty() {
                None
            } else {
                Some(buf.trim().to_string())
            }
        }
        Value::Object(map) => {
            if let Some(text) = map.get("text").and_then(|v| v.as_str()) {
                return Some(text.trim().to_string());
            }
            if let Some(content) = map.get("content").and_then(|v| v.as_str()) {
                return Some(content.trim().to_string());
            }
            if let Some(resp) = map.get("response").and_then(|v| v.as_str()) {
                return Some(resp.trim().to_string());
            }
            None
        }
        _ => None,
    }
}

#[derive(Deserialize)]
struct ModelsResponse {
    data: Vec<ModelItem>,
}

#[derive(Deserialize, Default)]
struct ModelItem {
    id: String,
    owned_by: Option<String>,
    hidden: Option<bool>,
    format: Option<String>,
    details: Option<ModelDetails>,
}

#[derive(Deserialize, Default)]
struct ModelDetails {
    #[serde(default)]
    parameter_size: Option<String>,
}

pub fn run_connection_test(settings: &LlmPostProcessSettings) -> Result<ConnectionTestOutcome> {
    run_connection_test_local(settings)
}

fn run_connection_test_chat(settings: &LlmPostProcessSettings) -> Result<ConnectionTestOutcome> {
    let payload = ChatCompletionPayload {
        model: settings.effective_model(),
        messages: vec![
            ChatMessagePayload {
                role: "system",
                content: "Reply with the word 'pong'.".to_string(),
            },
            ChatMessagePayload {
                role: "user",
                content: "ping".to_string(),
            },
        ],
        temperature: 0.0,
        max_tokens: Some(8),
    };

    let (response, status, latency_ms) =
        execute_chat_completion(settings, &payload).map_err(|err| anyhow!(err.message))?;

    let content =
        extract_first_choice_text(&response).ok_or_else(|| anyhow!("Missing content field"))?;
    let message = format!("Chat completion OK: {}", preview_body(&content));

    Ok(ConnectionTestOutcome {
        status: Some(status.as_u16()),
        duration_ms: latency_ms,
        message,
    })
}

fn run_connection_test_local(settings: &LlmPostProcessSettings) -> Result<ConnectionTestOutcome> {
    let start = Instant::now();
    match fetch_models(settings) {
        Ok(models) => {
            let mut message = format!("Model list OK ({} models)", models.len());
            if let Some(first) = models.first() {
                message = format!("Model list OK (first: {})", first.id);
            }
            Ok(ConnectionTestOutcome {
                status: Some(200),
                duration_ms: start.elapsed().as_millis(),
                message,
            })
        }
        Err(first_err) => {
            let first_msg = first_err.to_string();
            match run_connection_test_chat(settings) {
                Ok(mut outcome) => {
                    outcome.message = format!(
                        "Chat completion OK (model fetch failed: {})",
                        preview_body(&first_msg)
                    );
                    Ok(outcome)
                }
                Err(second_err) => Err(first_err.context(second_err)),
            }
        }
    }
}

pub fn fetch_models(settings: &LlmPostProcessSettings) -> Result<Vec<LlmModelInfo>> {
    let client = build_client_with_timeout(settings.timeout_secs)?;
    let headers = create_headers(settings, false)?;
    let url = join_url(&settings.effective_base_url(), MODELS_PATH);
    let response = client
        .get(&url)
        .headers(headers)
        .send()
        .with_context(|| format!("GET {}", url))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        return Err(anyhow!("HTTP {} {}", status.as_u16(), preview_body(&body)));
    }

    let parsed: ModelsResponse = response.json().context("parse models response")?;
    let mut models: Vec<LlmModelInfo> = parsed.data.iter().filter_map(map_model_item).collect();
    models.sort_by(|a, b| a.id.cmp(&b.id));
    if models.is_empty() {
        return Err(anyhow!("No chat-capable models reported"));
    }
    Ok(models)
}

fn map_model_item(item: &ModelItem) -> Option<LlmModelInfo> {
    if item.id.trim().is_empty() {
        return None;
    }

    if item.hidden.unwrap_or(false) {
        return None;
    }
    if let Some(format) = item.format.as_ref() {
        let lower = format.to_ascii_lowercase();
        if lower.contains("embed") {
            return None;
        }
    }

    let mut label = item.id.clone();
    if let Some(owner) = item.owned_by.as_ref() {
        if !owner.is_empty() {
            label = format!("{} ({})", item.id, owner);
        }
    }

    if let Some(details) = item.details.as_ref() {
        if let Some(size) = details.parameter_size.as_ref() {
            if !size.is_empty() {
                label = format!("{} [{}]", label, size);
            }
        }
    }

    Some(LlmModelInfo {
        id: item.id.clone(),
        label,
    })
}

fn build_client_with_timeout(timeout_secs: u64) -> Result<Client> {
    let secs = timeout_secs.max(3).min(120);
    Client::builder()
        .timeout(Duration::from_secs(secs))
        .build()
        .context("create HTTP client")
}

fn create_headers(
    _settings: &LlmPostProcessSettings,
    include_content_type: bool,
) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_VALUE));
    if include_content_type {
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    }

    Ok(headers)
}

fn join_url(base: &str, path: &str) -> String {
    let mut joined = base.trim_end_matches('/').to_string();
    joined.push('/');
    joined.push_str(path.trim_start_matches('/'));
    joined
}

fn map_reqwest_error(err: reqwest::Error) -> LlmRequestError {
    if err.is_timeout() {
        return LlmRequestError {
            message: "LLM request timed out".to_string(),
            status: None,
            retry_after_secs: None,
        };
    }
    if err.is_connect() {
        return LlmRequestError {
            message: format!("Failed to connect: {}", err),
            status: None,
            retry_after_secs: None,
        };
    }
    LlmRequestError {
        message: format!("HTTP request failed: {}", err),
        status: err.status().map(|s| s.as_u16()),
        retry_after_secs: None,
    }
}

fn parse_retry_after_secs(headers: &HeaderMap) -> Option<u64> {
    headers
        .get("Retry-After")
        .and_then(|value| value.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
}

fn preview_body(body: &str) -> String {
    let mut out = String::new();
    let mut count = 0usize;
    let mut truncated = false;
    for ch in body.chars() {
        if count >= MAX_ERROR_BODY_PREVIEW {
            truncated = true;
            break;
        }
        out.push(ch);
        count += 1;
    }
    let trimmed = out.trim();
    if truncated {
        format!("{}…", trimmed)
    } else {
        trimmed.to_string()
    }
}
