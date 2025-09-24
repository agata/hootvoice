use std::sync::{Arc, Mutex};
use std::thread;
use std::collections::VecDeque;
use whisper_rs::{WhisperContext, FullParams, SamplingStrategy};
use anyhow::Result;
use crate::transcription::WhisperOptimizationParams;
use crate::audio::VadStrategy;

#[derive(Clone)]
pub struct ChunkResult {
    pub text: String,
    pub start_ms: i64,
    pub end_ms: i64,
}

pub struct ChunkProcessor {
    ctx: Arc<WhisperContext>,
    sample_rate: u32,
    language: Option<String>,
    optimization: Option<WhisperOptimizationParams>,
    strategy: VadStrategy,
    logger: Option<Arc<dyn Fn(&str) + Send + Sync>>,
    worker_thread: Option<thread::JoinHandle<()>>,
    queue: Arc<Mutex<VecDeque<Vec<f32>>>>,
    stop_flag: Arc<Mutex<bool>>,
    results: Arc<Mutex<Vec<ChunkResult>>>,
}

impl ChunkProcessor {
    pub fn new(
        ctx: Arc<WhisperContext>,
        sample_rate: u32,
        language: Option<String>,
        optimization: Option<WhisperOptimizationParams>,
        strategy: VadStrategy,
    ) -> Self {
        Self {
            ctx,
            sample_rate,
            language,
            optimization,
            strategy,
            logger: None,
            worker_thread: None,
            queue: Arc::new(Mutex::new(VecDeque::new())),
            stop_flag: Arc::new(Mutex::new(false)),
            results: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn set_logger(&mut self, logger: Arc<dyn Fn(&str) + Send + Sync>) {
        self.logger = Some(logger);
    }

    pub fn start_worker(&mut self) {
        let ctx = self.ctx.clone();
        let queue = self.queue.clone();
        let stop = self.stop_flag.clone();
        let results = self.results.clone();
        let lang = self.language.clone();
        let opt = self.optimization.clone();
        let logger = self.logger.clone();
        
        let handle = thread::spawn(move || {
            let mut state = ctx.create_state().unwrap();
            
            loop {
                if *stop.lock().unwrap() {
                    break;
                }
                
                let chunk = {
                    let mut q = queue.lock().unwrap();
                    q.pop_front()
                };
                
                if let Some(audio) = chunk {
                    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
                    
                    if let Some(ref l) = lang {
                        params.set_language(Some(l));
                    } else {
                        params.set_language(Some("ja"));
                    }
                    
                    params.set_print_special(false);
                    params.set_print_progress(false);
                    params.set_print_realtime(false);
                    params.set_print_timestamps(false);
                    params.set_suppress_blank(true);
                    params.set_token_timestamps(false);
                    
                    if let Some(ref o) = opt {
                        params.set_temperature(o.temperature);
                        params.set_no_timestamps(o.no_timestamps);
                        params.set_token_timestamps(o.token_timestamps);
                        params.set_n_max_text_ctx(o.n_max_text_ctx);
                        params.set_no_context(o.no_context);
                        
                        let n_threads = if o.use_physical_cores {
                            num_cpus::get_physical() as i32
                        } else {
                            num_cpus::get() as i32
                        };
                        params.set_n_threads(n_threads.max(1));
                    }
                    
                    if let Ok(_) = state.full(params, &audio) {
                        let mut text = String::new();
                        
                        for seg in state.as_iter() {
                            text.push_str(&seg.to_string());
                            text.push(' ');
                        }
                        
                        if !text.trim().is_empty() {
                            if let Some(ref log) = logger {
                                log(&format!("[chunk] recognized: {}", text));
                            }
                            
                            results.lock().unwrap().push(ChunkResult {
                                text: text.clone(),
                                start_ms: 0,
                                end_ms: 0,
                            });
                        }
                    }
                } else {
                    thread::sleep(std::time::Duration::from_millis(10));
                }
            }
        });
        
        self.worker_thread = Some(handle);
    }

    pub fn process_audio(&self, audio: &[f32], _sample_rate: u32) {
        self.queue.lock().unwrap().push_back(audio.to_vec());
    }

    pub fn process_chunk(&self, audio: Vec<f32>) {
        self.queue.lock().unwrap().push_back(audio);
    }

    pub fn stop_worker(&mut self) {
        *self.stop_flag.lock().unwrap() = true;
        if let Some(handle) = self.worker_thread.take() {
            let _ = handle.join();
        }
    }

    pub fn get_results(&self) -> Vec<ChunkResult> {
        self.results.lock().unwrap().clone()
    }

    pub fn finish(&mut self) -> Vec<ChunkResult> {
        self.stop_worker();
        self.get_results()
    }

    pub fn combine_results(results: &[ChunkResult]) -> String {
        fn merge_with_overlap(mut acc: String, next: &str) -> String {
            let next_trim = next.trim();
            if next_trim.is_empty() { return acc; }
            if acc.contains(next_trim) { return acc; }
            let max_k = acc.len().min(next_trim.len()).min(64);
            let mut cuts: Vec<usize> = next_trim.char_indices().map(|(i, _)| i).collect();
            if *cuts.last().unwrap_or(&0) != next_trim.len() { cuts.push(next_trim.len()); }
            let mut best_k = 0usize;
            for &i in cuts.iter().rev() {
                if i == 0 || i > max_k { continue; }
                if acc.ends_with(&next_trim[..i]) { best_k = i; break; }
            }
            acc.push_str(&next_trim[best_k..]);
            acc
        }
        let mut acc = String::new();
        for r in results.iter() {
            acc = merge_with_overlap(acc, r.text.as_str());
        }
        acc
    }
}

impl Drop for ChunkProcessor {
    fn drop(&mut self) {
        self.stop_worker();
    }
}
