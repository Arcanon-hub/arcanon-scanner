//! Embedded ML Inference Layer for Arcanon.
//! Uses ONNX Runtime (ort) to run a quantized CodeBERT model locally on CPU.
//!
//! # How it works
//!
//! CodeBERT is a feature extractor — it turns code text into a 768-dimensional vector.
//! We use zero-shot classification: precompute centroid embeddings for two anchor categories
//! ("real service call" vs "config/noise"), then score each connection by cosine similarity
//! to each centroid. The ratio determines `ml_confidence` and updates `confidence`.
//!
//! # Anchor strategy
//!
//! Instead of training a classifier head, we hardcode representative examples for each
//! category. As arcanon-ml produces a fine-tuned model, drop the new `.onnx` in place —
//! the inference logic here stays the same, the model improves automatically.

use crate::core::merger::MergedResult;
use crate::types::Confidence;
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::Value;
use std::sync::Mutex;
use tokenizers::Tokenizer;
use tracing::{debug, info, warn};


/// Code snippets that represent genuine service connections.
/// These are the positive anchor class: "this code talks to an external system."
const SERVICE_ANCHORS: &[&str] = &[
    "await client.connect(url)",
    "requests.get(os.getenv('SERVICE_URL'))",
    "fetch('/api/users')",
    "conn.execute('SELECT * FROM users WHERE id = ?', [user_id])",
    "producer.send('order-events', message)",
    "stub.GetUser(GetUserRequest(id=user_id))",
    "redis_client.get(cache_key)",
    "await websocket.connect(endpoint)",
    "http_client.post(base_url + '/v1/ingest', json=payload)",
    "channel.basic_publish(exchange='events', routing_key='order.created', body=msg)",
    "await opcua_client.connect(server_url)",
    "grpc_channel = grpc.insecure_channel(host + ':' + port)",
];

/// Code snippets that represent config reads, not service connections.
/// These are the negative anchor class: "this code reads an application setting."
const NOISE_ANCHORS: &[&str] = &[
    "os.getenv('MAX_RETRIES', 3)",
    "os.getenv('LOG_LEVEL', 'INFO')",
    "int(os.getenv('WORKER_COUNT', 4))",
    "os.getenv('EVICTION_POLICY', 'lru')",
    "os.getenv('BUFFER_SIZE', 1024)",
    "bool(os.getenv('ENABLE_CACHE', True))",
    "os.getenv('FEATURE_FLAG', 'false')",
    "config.get('timeout_seconds', 30)",
    "settings.MAX_CONNECTIONS",
    "env.get('DEBUG', False)",
    "os.getenv('QUEUE_DEPTH', 100)",
    "getenv('TLS_VERIFY', 'true').lower() == 'true'",
];

/// Strategy for ML-based refinement of AST-based findings.
pub trait MLRefiner: Send + Sync {
    fn refine(&self, results: &mut MergedResult);
}

/// The main ML engine. Initializes on first use; falls back to no-op if model is missing.
pub struct ArcanonML {
    engine: Option<Box<dyn MLRefiner>>,
}

impl ArcanonML {
    /// `model_path`: path to `codebert-v1.onnx`.
    /// Pass `None` to use the default: `~/.arcanon/models/codebert-v1.onnx`.
    pub fn new(model_path: Option<&std::path::Path>) -> Self {
        let resolved_path = match model_path {
            Some(p) => p.to_path_buf(),
            None => {
                let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
                home.join(".arcanon").join("models").join("codebert-v1.onnx")
            }
        };

        if !resolved_path.exists() {
            warn!(
                "ML Refinement disabled: model file not found at {}. \
                 Download it with `arcanon fetch-model` or set [scanner].ml_model_path in .arcanon.toml.",
                resolved_path.display()
            );
            return Self { engine: None };
        }

        let tokenizer_path = resolved_path
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join("tokenizer.json");

        let tokenizer_bytes = match std::fs::read(&tokenizer_path) {
            Ok(b) => b,
            Err(e) => {
                warn!(
                    "ML Refinement disabled: could not read tokenizer file {}: {}",
                    tokenizer_path.display(),
                    e
                );
                return Self { engine: None };
            }
        };

        match std::fs::read(&resolved_path) {
            Err(e) => {
                warn!(
                    "ML Refinement disabled: could not read model file {}: {}",
                    resolved_path.display(),
                    e
                );
                Self { engine: None }
            }
            Ok(model_bytes) => match CodeBERTRefiner::new_from_bytes(&model_bytes, &tokenizer_bytes) {
                Ok(engine) => {
                    info!("ML Refinement: CodeBERT engine loaded successfully.");
                    Self {
                        engine: Some(Box::new(engine)),
                    }
                }
                Err(e) => {
                    // Distinguish model-file corruption from missing ONNX Runtime library.
                    let reason = if e.to_string().contains("libonnxruntime")
                        || e.to_string().contains("onnxruntime.dll")
                        || e.to_string().contains("OrtGetApiBase")
                    {
                        format!(
                            "ONNX Runtime library not found (libonnxruntime.so / onnxruntime.dll). \
                             Install it from https://github.com/microsoft/onnxruntime/releases. \
                             Error: {}",
                            e
                        )
                    } else {
                        format!("model file may be corrupt or incompatible. Error: {}", e)
                    };
                    warn!("ML Refinement disabled: {}", reason);
                    Self { engine: None }
                }
            },
        }
    }

    pub fn refine(&self, results: &mut MergedResult) {
        if let Some(ref engine) = self.engine {
            engine.refine(results);
        }
    }
}

/// Zero-shot CodeBERT classifier using cosine similarity to anchor centroids.
pub struct CodeBERTRefiner {
    session: Mutex<Session>,
    tokenizer: Tokenizer,
    /// Centroid embedding of SERVICE_ANCHORS — the "real connection" pole.
    service_centroid: Vec<f32>,
    /// Centroid embedding of NOISE_ANCHORS — the "config value" pole.
    noise_centroid: Vec<f32>,
}

impl CodeBERTRefiner {
    pub fn new_from_bytes(model_bytes: &[u8], tokenizer_bytes: &[u8]) -> anyhow::Result<Self> {
        let session = Mutex::new(
            Session::builder()
                .map_err(|e| anyhow::anyhow!("Session builder error: {:?}", e))?
                .with_optimization_level(GraphOptimizationLevel::Level3)
                .map_err(|e| anyhow::anyhow!("Optimization level error: {:?}", e))?
                .with_intra_threads(4)
                .map_err(|e| anyhow::anyhow!("Thread config error: {:?}", e))?
                .commit_from_memory(model_bytes)
                .map_err(|e| anyhow::anyhow!("Model load error: {:?}", e))?,
        );

        let tokenizer = Tokenizer::from_bytes(tokenizer_bytes)
            .map_err(|e| anyhow::anyhow!("Tokenizer error: {}", e))?;

        // Precompute anchor centroids once at startup.
        // This runs the model 24 times (12 service + 12 noise anchors).
        // Cost is paid once; all connection scoring is incremental after this.
        info!(
            "ML Refinement: Precomputing anchor centroids ({} service + {} noise)...",
            SERVICE_ANCHORS.len(),
            NOISE_ANCHORS.len()
        );
        let service_centroid = Self::compute_centroid(&session, &tokenizer, SERVICE_ANCHORS)?;
        let noise_centroid = Self::compute_centroid(&session, &tokenizer, NOISE_ANCHORS)?;

        Ok(Self {
            session,
            tokenizer,
            service_centroid,
            noise_centroid,
        })
    }

    /// Get the [CLS] token embedding (768-dim) for a text snippet.
    fn embed(session: &Mutex<Session>, tokenizer: &Tokenizer, text: &str) -> anyhow::Result<Vec<f32>> {
        let encoding = tokenizer
            .encode(text, true)
            .map_err(|e| anyhow::anyhow!("Encoding error: {}", e))?;

        let input_ids: Vec<i64> = encoding.get_ids().iter().map(|&x| x as i64).collect();
        let attention_mask: Vec<i64> = encoding
            .get_attention_mask()
            .iter()
            .map(|&x| x as i64)
            .collect();

        let seq_len = input_ids.len();
        let shape = vec![1usize, seq_len];

        let input_ids_val = Value::from_array((shape.clone(), input_ids))
            .map_err(|e| anyhow::anyhow!("Input IDs value error: {:?}", e))?;
        let attn_mask_val = Value::from_array((shape, attention_mask))
            .map_err(|e| anyhow::anyhow!("Attention mask value error: {:?}", e))?;

        let sess = session
            .lock()
            .map_err(|e| anyhow::anyhow!("Session lock poisoned: {}", e))?;

        let outputs = sess
            .run(ort::inputs![
                "input_ids" => input_ids_val,
                "attention_mask" => attn_mask_val,
            ].map_err(|e| anyhow::anyhow!("Inputs build error: {:?}", e))?)
            .map_err(|e| anyhow::anyhow!("Inference run error: {:?}", e))?;

        let output_tensor = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| anyhow::anyhow!("Tensor extraction error: {:?}", e))?;

        // try_extract_tensor returns an ndarray::ArrayViewD — flatten to a slice.
        let data = output_tensor
            .as_slice()
            .ok_or_else(|| anyhow::anyhow!("Tensor data is not contiguous"))?;

        // [CLS] token is first 768 dimensions of the last hidden state.
        if data.len() < 768 {
            return Err(anyhow::anyhow!(
                "Model output has {} dimensions; expected at least 768. \
                 Check that the loaded model is a CodeBERT-compatible encoder.",
                data.len()
            ));
        }
        Ok(data[..768].to_vec())
    }

    /// Compute the mean embedding across a set of text examples.
    fn compute_centroid(
        session: &Mutex<Session>,
        tokenizer: &Tokenizer,
        texts: &[&str],
    ) -> anyhow::Result<Vec<f32>> {
        let mut centroid = vec![0.0f32; 768];
        for text in texts {
            let emb = Self::embed(session, tokenizer, text)?;
            for (c, e) in centroid.iter_mut().zip(emb.iter()) {
                *c += e;
            }
        }
        let n = texts.len() as f32;
        for c in &mut centroid {
            *c /= n;
        }
        Ok(centroid)
    }

    /// Cosine similarity between two equal-length vectors.
    fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
        let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
        let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm_a == 0.0 || norm_b == 0.0 {
            return 0.0;
        }
        dot / (norm_a * norm_b)
    }
}

impl MLRefiner for CodeBERTRefiner {
    fn refine(&self, results: &mut MergedResult) {
        info!(
            "ML Refinement: Scoring {} connections...",
            results.connections.len()
        );

        for conn in &mut results.connections {
            let evidence = match &conn.evidence {
                Some(e) if !e.trim().is_empty() => e.clone(),
                _ => {
                    // No evidence text — leave confidence unchanged, mark as unscored.
                    conn.ml_confidence = Some(0.5);
                    conn.ml_reasoning = Some("no_evidence".to_string());
                    continue;
                }
            };

            // Combine protocol + dependency + evidence so CodeBERT sees full context.
            // Example: "protocol:env dependency:py-env-getenv code:os.getenv('EVICTION_POLICY', ...)"
            // The variable name and protocol together are usually enough to distinguish
            // service locators from config values.
            let input = format!(
                "protocol:{} dependency:{} code:{}",
                conn.protocol,
                conn.dependency.as_deref().unwrap_or("unknown"),
                evidence
            );

            match Self::embed(&self.session, &self.tokenizer, &input) {
                Ok(emb) => {
                    let sim_service =
                        Self::cosine_similarity(&emb, &self.service_centroid);
                    let sim_noise =
                        Self::cosine_similarity(&emb, &self.noise_centroid);

                    // Normalize: what fraction of total similarity is the service pole?
                    let total = sim_service + sim_noise;
                    let ml_score = if total > 0.0 {
                        (sim_service / total).clamp(0.0, 1.0)
                    } else if sim_service > sim_noise {
                        // Both similarities are non-positive but service pole is less negative —
                        // this embedding leans toward a real connection; score high.
                        0.75
                    } else {
                        0.5
                    };

                    conn.ml_confidence = Some(ml_score);
                    conn.ml_reasoning = Some(format!(
                        "service_sim:{:.3} noise_sim:{:.3}",
                        sim_service, sim_noise
                    ));

                    // Update confidence: ML score overrides AST confidence.
                    // Thresholds tuned to bias toward recall — we'd rather keep a
                    // borderline finding than drop a real connection.
                    conn.confidence = if ml_score >= 0.68 {
                        Confidence::High
                    } else if ml_score >= 0.42 {
                        Confidence::Medium
                    } else {
                        Confidence::Low
                    };

                    debug!(
                        "ML: {} | score={:.3} | {:?}",
                        conn.source_file, ml_score, conn.confidence
                    );
                }
                Err(e) => {
                    warn!(
                        "ML embedding failed for {}: {}",
                        conn.source_file, e
                    );
                    conn.ml_confidence = Some(0.5);
                    conn.ml_reasoning = Some(format!("embedding_error:{}", e));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity_identical() {
        let v = vec![1.0f32, 0.0, 0.0];
        assert!((CodeBERTRefiner::cosine_similarity(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0f32, 0.0, 0.0];
        let b = vec![0.0f32, 1.0, 0.0];
        assert!(CodeBERTRefiner::cosine_similarity(&a, &b).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_zero_vector() {
        let zero = vec![0.0f32; 3];
        let a = vec![1.0f32, 0.0, 0.0];
        assert_eq!(CodeBERTRefiner::cosine_similarity(&zero, &a), 0.0);
    }

    #[test]
    fn test_score_high_when_service_wins() {
        // sim_service = 0.8, sim_noise = 0.2  =>  total = 1.0  =>  score = 0.8
        // Expect: score >= 0.68 => Confidence::High
        let sim_service = 0.8f32;
        let sim_noise = 0.2f32;
        let total = sim_service + sim_noise;
        let ml_score = (sim_service / total).clamp(0.0, 1.0);
        assert!(ml_score >= 0.68, "expected High confidence threshold, got {}", ml_score);
    }

    #[test]
    fn test_score_medium_when_ambiguous() {
        // sim_service = 0.55, sim_noise = 0.45  =>  total = 1.0  =>  score = 0.55
        // Expect: 0.42 <= score < 0.68 => Confidence::Medium
        let sim_service = 0.55f32;
        let sim_noise = 0.45f32;
        let total = sim_service + sim_noise;
        let ml_score = (sim_service / total).clamp(0.0, 1.0);
        assert!(
            ml_score >= 0.42 && ml_score < 0.68,
            "expected Medium confidence band, got {}",
            ml_score
        );
    }

    #[test]
    fn test_score_total_zero_service_wins() {
        // total <= 0.0 but sim_service > sim_noise  =>  score = 0.75 (high-ish)
        let sim_service = -0.1f32;
        let sim_noise = -0.3f32;
        let total = sim_service + sim_noise; // -0.4, not > 0
        let ml_score = if total > 0.0 {
            (sim_service / total).clamp(0.0, 1.0)
        } else if sim_service > sim_noise {
            0.75
        } else {
            0.5
        };
        assert!(
            (ml_score - 0.75).abs() < 1e-6,
            "expected 0.75 for total<=0 with service winning, got {}",
            ml_score
        );
    }

    #[test]
    #[ignore = "loads 473MB ONNX model — run manually with `cargo test -- --ignored`"]
    fn test_ml_new_no_crash() {
        // Verifies ArcanonML initializes and computes anchor centroids without panicking.
        let _ml = ArcanonML::new(None);
    }
}
