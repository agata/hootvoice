use crate::core::{LogCallback, SimpleRecState};
use crate::llm::{
    history_file_path, record_history, LlmPostProcessSettings, LlmPostProcessor,
    MAX_HISTORY_ENTRIES,
};
use std::sync::{Arc, Mutex};

pub struct PostProcessResult {
    pub final_text: String,
    pub llm_latency_secs: f32,
}

#[derive(Clone)]
pub struct PostProcessEngine {
    settings: Arc<Mutex<LlmPostProcessSettings>>,
    processor: Arc<LlmPostProcessor>,
    state: Arc<Mutex<SimpleRecState>>,
}

impl PostProcessEngine {
    pub fn new(
        settings: Arc<Mutex<LlmPostProcessSettings>>,
        processor: Arc<LlmPostProcessor>,
        state: Arc<Mutex<SimpleRecState>>,
    ) -> Self {
        Self {
            settings,
            processor,
            state,
        }
    }

    pub fn set_settings(&self, settings: LlmPostProcessSettings) {
        *self.settings.lock().unwrap() = settings;
    }

    pub fn process(
        &self,
        base_text: &str,
        dictionary_hint: Option<&str>,
        language_hint: Option<&str>,
        log: &Arc<Mutex<Option<LogCallback>>>,
    ) -> PostProcessResult {
        let snapshot = self.settings.lock().unwrap().clone();
        if !snapshot.enabled {
            return PostProcessResult {
                final_text: base_text.to_string(),
                llm_latency_secs: 0.0,
            };
        }

        log_message(log, &format!("[llm][input] {}", base_text));
        log_message(
            log,
            &format!(
                "[llm] Processing via {} (model: {}).",
                snapshot.effective_base_url(),
                snapshot.model
            ),
        );

        if let Ok(mut state) = self.state.lock() {
            *state = SimpleRecState::PostProcessing;
        }

        let mut final_text = base_text.to_string();
        let mut llm_latency_secs = 0.0f32;
        let mut llm_output_for_log: Option<String> = None;

        let mut history_payload: Option<(String, bool, u128)> = None;
        match self
            .processor
            .process(&snapshot, base_text, dictionary_hint, language_hint)
        {
            Ok(outcome) => {
                let content = outcome.content;
                let truncated_input = outcome.truncated_input;
                let latency_ms = outcome.latency_ms;
                history_payload = Some((content.clone(), truncated_input, latency_ms));
                if outcome.truncated_input {
                    log_message(
                        log,
                        &format!(
                            "[llm] Input truncated to {} chars.",
                            snapshot.max_input_chars
                        ),
                    );
                }
                llm_latency_secs = latency_ms as f32 / 1000.0;
                log_message(
                    log,
                    &format!("[llm] Completed in {:.2}s.", llm_latency_secs),
                );
                log_message(log, &format!("[llm][output] {}", content));
                llm_output_for_log = Some(content.clone());
                if snapshot.apply_to_autopaste {
                    final_text = content.clone();
                } else {
                    log_message(
                        log,
                        "[llm] Auto paste uses Whisper text (setting disabled).",
                    );
                }
            }
            Err(err) => {
                let mut message = err.message;
                if let Some(status) = err.status {
                    message = format!("{} (status {})", message, status);
                }
                if let Some(wait) = err.retry_after_secs {
                    message = format!("{} (retry after {}s)", message, wait);
                }
                log_message(log, &format!("[llm][error] {}", message));
                log_message(log, "[llm] Falling back to Whisper text.");
            }
        }

        if let Some((llm_output, truncated_input, latency_ms)) = history_payload {
            match record_history(
                base_text,
                &llm_output,
                truncated_input,
                latency_ms,
                &snapshot,
            ) {
                Ok(outcome) => {
                    log_message(
                        log,
                        &format!(
                            "[llm][history] Saved entry {}/{} → {}",
                            outcome.total_entries,
                            MAX_HISTORY_ENTRIES,
                            history_file_path().display()
                        ),
                    );
                }
                Err(err) => {
                    log_message(
                        log,
                        &format!("[Warning] Failed to persist LLM history: {}", err),
                    );
                }
            }
        }

        if let Some(output_text) = llm_output_for_log.as_ref() {
            if output_text == base_text {
                log_message(log, "[llm] Output identical to Whisper text.");
            } else {
                log_message(log, "[llm] Output differs from Whisper text.");
            }
        }

        if let Ok(mut state) = self.state.lock() {
            *state = SimpleRecState::Processing;
        }

        PostProcessResult {
            final_text,
            llm_latency_secs,
        }
    }
}

fn log_message(log_callback: &Arc<Mutex<Option<LogCallback>>>, message: &str) {
    if let Some(ref callback) = *log_callback.lock().unwrap() {
        callback(message);
    }
    if let Some(rest) = message.strip_prefix("[Error]") {
        tracing::error!("{}", rest.trim());
    } else if let Some(rest) = message.strip_prefix("[Warning]") {
        tracing::warn!("{}", rest.trim());
    } else if let Some(rest) = message.strip_prefix("[Info]") {
        tracing::info!("{}", rest.trim());
    } else {
        tracing::info!("{}", message);
    }
}
