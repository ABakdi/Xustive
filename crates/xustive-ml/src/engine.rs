//! Local GGUF inference through llama.cpp.
//!
//! # Threading
//!
//! A llama context is not shareable and generation is a long, CPU-bound loop, so each slot gets
//! its own OS thread and its own context. Jobs reach them over a **bounded** queue: when it is
//! full the request is refused immediately rather than queued, because a summary that arrives
//! after the user has read the results is worth less than no summary at all — and queueing turns
//! a load spike into a latency cliff for everyone behind it.
//!
//! The model itself is shared. It is the large allocation (about 2 GB for the 3B Q4_K_M), and
//! loading it per slot would exceed the memory budget on the reference hardware for no gain.
//!
//! # Device
//!
//! The layer count comes from [`crate::device::resolve`], so the admin page's device setting is
//! what actually determines whether weights land on the GPU. Settings are read at load time,
//! which is why changing them takes effect on the next load rather than immediately.

use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaChatMessage, LlamaChatTemplate, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;

use crate::device::{self, DeviceConfig, Resolved};
use crate::prompt::Prompt;

/// Slots that generate concurrently. Memory-bound: each context carries its own KV cache, which
/// on a 4 GB card is the scarce resource, not compute.
pub const DEFAULT_SLOTS: usize = 2;
/// Queue depth beyond the slots. Small on purpose — see the module note on refusing early.
pub const QUEUE_CAPACITY: usize = 8;
/// Context window. Wide enough for the passage budget plus instructions and the answer, and no
/// wider: KV cache scales linearly with it.
const N_CTX: u32 = 4096;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("model file not found: {0}")]
    ModelMissing(PathBuf),
    #[error("failed to load model: {0}")]
    Load(String),
    #[error("the summariser is busy")]
    Busy,
    #[error("generation timed out")]
    Timeout,
    #[error("generation failed: {0}")]
    Generation(String),
    #[error("the summariser is shut down")]
    Shutdown,
}

/// Sampling parameters.
///
/// Low temperature is deliberate: this is extraction from passages, not writing. A creative
/// summariser invents the details it cannot find.
#[derive(Debug, Clone, Copy)]
pub struct Sampling {
    pub temperature: f32,
    pub top_p: f32,
    pub repeat_penalty: f32,
    pub max_tokens: usize,
}

impl Default for Sampling {
    fn default() -> Self {
        Self {
            temperature: 0.2,
            top_p: 0.9,
            repeat_penalty: 1.05,
            max_tokens: 120,
        }
    }
}

struct Job {
    prompt: Prompt,
    sampling: Sampling,
    deadline: Instant,
    reply: tokio::sync::oneshot::Sender<Result<Generated, EngineError>>,
}

/// Raw model output plus the timings the budget is measured against.
#[derive(Debug, Clone)]
pub struct Generated {
    pub text: String,
    pub tokens: usize,
    pub time_to_first_token: Duration,
    pub total: Duration,
    /// True when generation stopped on the deadline rather than at an end-of-generation token.
    pub truncated: bool,
}

/// A loaded model with its worker slots.
pub struct Engine {
    queue: SyncSender<Job>,
    resolved: Resolved,
    model_path: PathBuf,
    workers: Vec<std::thread::JoinHandle<()>>,
}

impl Engine {
    /// Load a model and start its slots.
    ///
    /// Blocking and slow — seconds for a multi-gigabyte file — so callers should do this once at
    /// startup, off the request path.
    pub fn load(
        model_path: impl AsRef<Path>,
        config: &DeviceConfig,
        slots: usize,
    ) -> Result<Self, EngineError> {
        let model_path = model_path.as_ref().to_path_buf();
        let size_mib = std::fs::metadata(&model_path)
            .map(|m| m.len() / (1024 * 1024))
            .map_err(|_| EngineError::ModelMissing(model_path.clone()))?;

        let resolved = device::resolve(config, size_mib);
        tracing::info!(
            model = %model_path.display(),
            device = resolved.active.as_str(),
            gpu_layers = resolved.gpu_layers,
            reason = %resolved.reason,
            "loading summariser model"
        );

        let backend = LlamaBackend::init().map_err(|e| EngineError::Load(e.to_string()))?;

        let params = LlamaModelParams::default().with_n_gpu_layers(resolved.gpu_layers);
        let model = LlamaModel::load_from_file(&backend, &model_path, &params)
            .map_err(|e| EngineError::Load(e.to_string()))?;

        let backend = Arc::new(backend);
        let model = Arc::new(model);
        // The template baked into the GGUF, not a guess. Qwen's ChatML differs from the generic
        // one in ways that visibly degrade instruction following when mismatched.
        let template = model.chat_template(None).ok();

        let (queue, rx) = sync_channel::<Job>(QUEUE_CAPACITY);
        let rx = Arc::new(std::sync::Mutex::new(rx));

        let slots = slots.max(1);
        let workers = (0..slots)
            .map(|slot| {
                let backend = Arc::clone(&backend);
                let model = Arc::clone(&model);
                let rx = Arc::clone(&rx);
                let template = template.clone();
                std::thread::Builder::new()
                    .name(format!("summariser-{slot}"))
                    .spawn(move || worker(slot, slots, &backend, &model, template.as_ref(), &rx))
                    .expect("failed to spawn summariser thread")
            })
            .collect();

        Ok(Self {
            queue,
            resolved,
            model_path,
            workers,
        })
    }

    /// What the device resolution actually landed on when this model was loaded.
    ///
    /// The admin page shows both this and the current setting: they differ exactly when the
    /// operator has changed the preference and no model has been reloaded since.
    pub fn resolved(&self) -> &Resolved {
        &self.resolved
    }

    pub fn model_path(&self) -> &Path {
        &self.model_path
    }

    /// Submit a job. Returns [`EngineError::Busy`] immediately when every slot and the queue are
    /// occupied.
    pub async fn generate(
        &self,
        prompt: Prompt,
        sampling: Sampling,
        budget: Duration,
    ) -> Result<Generated, EngineError> {
        let (reply, wait) = tokio::sync::oneshot::channel();
        let job = Job {
            prompt,
            sampling,
            deadline: Instant::now() + budget,
            reply,
        };

        match self.queue.try_send(job) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => return Err(EngineError::Busy),
            Err(TrySendError::Disconnected(_)) => return Err(EngineError::Shutdown),
        }

        // The extra margin covers time spent waiting in the queue: the worker enforces the real
        // deadline, and this only guards against a worker dying without replying.
        match tokio::time::timeout(budget + Duration::from_secs(5), wait).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(EngineError::Shutdown),
            Err(_) => Err(EngineError::Timeout),
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        // Closing the queue ends every worker's receive loop. Joining matters because the workers
        // hold Arcs to the model, and llama.cpp frees GPU memory in the model's destructor.
        let (dead, _) = sync_channel(1);
        let queue = std::mem::replace(&mut self.queue, dead);
        drop(queue);
        for handle in self.workers.drain(..) {
            let _ = handle.join();
        }
    }
}

/// One slot: owns a context for its lifetime and serves jobs one at a time.
fn worker(
    slot: usize,
    slots: usize,
    backend: &LlamaBackend,
    model: &LlamaModel,
    template: Option<&LlamaChatTemplate>,
    rx: &std::sync::Mutex<Receiver<Job>>,
) {
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    // Split the machine between the slots that actually exist, not the default count.
    // Oversubscribing makes every slot slower than one alone would be; undersubscribing leaves
    // half the machine idle, which on CPU is the difference between a usable summary and a slow
    // one — prefill is thread-bound and dominates time to first token.
    let per_slot = (threads / slots).max(1) as i32;

    let params = LlamaContextParams::default()
        .with_n_ctx(NonZeroU32::new(N_CTX))
        .with_n_threads(per_slot)
        .with_n_threads_batch(per_slot);

    let mut ctx = match model.new_context(backend, params) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(slot, error = %e, "summariser slot failed to start");
            return;
        }
    };
    tracing::info!(slot, threads = per_slot, "summariser slot ready");

    loop {
        // The lock is held only for the receive, not for generation, so slots genuinely run in
        // parallel; it exists because a std receiver has a single consumer.
        let job = {
            let guard = match rx.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            match guard.recv() {
                Ok(job) => job,
                // The queue was dropped: shut down.
                Err(_) => break,
            }
        };

        // The caller may have gone away while this job sat in the queue.
        if job.reply.is_closed() || Instant::now() >= job.deadline {
            continue;
        }

        let result = generate_one(&mut ctx, model, template, &job);
        let _ = job.reply.send(result);
    }
    tracing::debug!(slot, "summariser slot stopped");
}

fn generate_one(
    ctx: &mut llama_cpp_2::context::LlamaContext,
    model: &LlamaModel,
    template: Option<&LlamaChatTemplate>,
    job: &Job,
) -> Result<Generated, EngineError> {
    let started = Instant::now();

    let text = render_chat(model, template, &job.prompt)?;
    let tokens = model
        .str_to_token(&text, AddBos::Always)
        .map_err(|e| EngineError::Generation(format!("tokenise: {e}")))?;

    if tokens.len() >= N_CTX as usize {
        // Should not happen — the passage budget is set well below this — but a prompt that
        // overflows the window silently drops the instructions off the front, which turns the
        // grounding rules into suggestions.
        return Err(EngineError::Generation(format!(
            "prompt is {} tokens, over the {N_CTX} context window",
            tokens.len()
        )));
    }

    // Each job starts from a clean slate. Reusing KV state across requests would leak one user's
    // query into another's summary.
    ctx.clear_kv_cache();

    let mut batch = LlamaBatch::new(tokens.len().max(1), 1);
    let last = tokens.len() - 1;
    for (i, token) in tokens.iter().enumerate() {
        batch
            .add(*token, i as i32, &[0], i == last)
            .map_err(|e| EngineError::Generation(format!("batch: {e}")))?;
    }
    ctx.decode(&mut batch)
        .map_err(|e| EngineError::Generation(format!("prefill: {e}")))?;

    let s = job.sampling;
    let mut sampler = LlamaSampler::chain_simple([
        LlamaSampler::penalties(64, s.repeat_penalty, 0.0, 0.0),
        LlamaSampler::top_p(s.top_p, 1),
        LlamaSampler::temp(s.temperature),
        // Seeded rather than random so a given prompt gives a given summary. Reproducibility is
        // worth more here than variety: it makes the faithfulness evaluation meaningful.
        LlamaSampler::dist(0x5553_5449),
    ]);

    let mut out = String::new();
    let mut decoder = encoding_rs::UTF_8.new_decoder();
    let mut n_cur = batch.n_tokens();
    let mut produced = 0usize;
    let mut first_token_at = None;
    let mut truncated = false;

    while produced < s.max_tokens {
        let token = sampler.sample(ctx, batch.n_tokens() - 1);
        if model.is_eog_token(token) {
            break;
        }

        let piece = model
            .token_to_piece(token, &mut decoder, false, None)
            .map_err(|e| EngineError::Generation(format!("detokenise: {e}")))?;
        out.push_str(&piece);
        produced += 1;
        first_token_at.get_or_insert_with(Instant::now);

        // Checked after appending so a deadline hit still returns the text generated so far,
        // which the validator may well accept.
        if Instant::now() >= job.deadline {
            truncated = true;
            break;
        }

        batch.clear();
        batch
            .add(token, n_cur, &[0], true)
            .map_err(|e| EngineError::Generation(format!("batch: {e}")))?;
        n_cur += 1;
        ctx.decode(&mut batch)
            .map_err(|e| EngineError::Generation(format!("decode: {e}")))?;
    }

    if produced >= s.max_tokens {
        truncated = true;
    }

    Ok(Generated {
        text: out,
        tokens: produced,
        time_to_first_token: first_token_at.unwrap_or(started).duration_since(started),
        total: started.elapsed(),
        truncated,
    })
}

/// Apply the model's chat template, falling back to ChatML.
///
/// The fallback exists because a GGUF without an embedded template would otherwise be unusable,
/// and Qwen's own format is ChatML — so for the models in the registry the fallback is correct
/// rather than merely tolerable.
fn render_chat(
    model: &LlamaModel,
    template: Option<&LlamaChatTemplate>,
    prompt: &Prompt,
) -> Result<String, EngineError> {
    let messages = [
        LlamaChatMessage::new("system".into(), prompt.system.clone()),
        LlamaChatMessage::new("user".into(), prompt.user.clone()),
    ];
    let messages: Result<Vec<_>, _> = messages.into_iter().collect();
    let messages = messages.map_err(|e| EngineError::Generation(format!("chat message: {e}")))?;

    if let Some(tmpl) = template {
        if let Ok(rendered) = model.apply_chat_template(tmpl, &messages, true) {
            return Ok(rendered);
        }
    }

    Ok(format!(
        "<|im_start|>system\n{}<|im_end|>\n<|im_start|>user\n{}<|im_end|>\n<|im_start|>assistant\n",
        prompt.system, prompt.user
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampling_defaults_favour_extraction_over_writing() {
        let s = Sampling::default();
        assert!(
            s.temperature <= 0.3,
            "a creative summariser invents details it cannot find"
        );
        assert!(s.repeat_penalty > 1.0, "guards against degenerate loops");
        assert_eq!(s.max_tokens, 120);
    }

    #[test]
    fn a_missing_model_file_is_reported_not_panicked() {
        let err = Engine::load("/nonexistent/model.gguf", &DeviceConfig::default(), 1);
        assert!(matches!(err, Err(EngineError::ModelMissing(_))));
    }

    #[test]
    fn the_queue_is_small_enough_to_refuse_early() {
        // A deep queue converts a load spike into a latency cliff: every waiting request still
        // gets served, long after its user has stopped caring.
        const { assert!(QUEUE_CAPACITY <= 16) };
    }
}
