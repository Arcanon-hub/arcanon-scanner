# Arcanon Scanner — BERT-tiny Connection Classifier

Design for an embedded ONNX classifier that filters noise from scanner output
and improves automatically as more repos are scanned — without any manual labeling.

**Status:** Design complete. Implementation planned for v1.3 milestone.
**Design originated:** opcua-adapter session, 2026-04-07
**Source documents:** `scanner-classifier-training.md`, `scanner-improvements.md` (management/opcua-adapter)

---

## Why we need this

Scanning the opcua-adapter repo produced 86 raw connections. After manual review, only
18 were real connections to external systems — **79% noise**.

| Source | Raw count | Real | Noise |
|--------|-----------|------|-------|
| Wrapper traces | 44 | 0 | 44 (internal call graph traversal) |
| Env pattern detections | 31 | 7 | 24 (paths, flags, identifiers, not endpoints) |
| Pattern + spec detections | 11 | 11 | 0 |
| **Total** | **86** | **18** | **68** |

No static rule can fix this generically. Whether `CERTIFICATE_PATH` is an endpoint depends on
naming conventions that vary by org. Whether `connector.connect()` is an external call or an
internal abstraction requires understanding the codebase — not just matching strings.

The 18 real connections were:
- 2 opcua (asyncua `Client` instantiation)
- 6 rest (fetch() calls, `create_client`, `CoreV1Api`)
- 7 env (URL-suffix vars only: `EDGEWORKS_JOURNAL_URL`, etc.)
- 1 kubernetes

---

## The core insight: the scanner already labels its own data

Every scan run produces two types of connections that are almost always correct without
human verification:

| Signal | Label | Confidence | Reasoning |
|--------|-------|------------|-----------|
| `extraction_method: pattern:*` + resolved target | **KEEP** | High | Direct library call with a URL/hostname |
| `extraction_method: wrapper_trace:*` | **DROP** | High | Internal call graph — not a boundary |
| `protocol: env` + `_URL`/`_HOST`/`_ENDPOINT` in evidence | **KEEP** | Medium | Var name semantically indicates an endpoint |
| `protocol: env` + `_PATH`/`_ID`/`_KEY`/`_LEVEL` in evidence | **DROP** | High | Var name indicates config, not an endpoint |
| `extraction_method: spec:openapi` or `spec:asyncapi` | **KEEP** | High | Extracted from a contract document |
| `extraction_method: ast_*` | **KEEP** | High | Framework route detected by AST plugin |

This is **weak supervision** — approximate labels from existing metadata, not human annotation.
Zero manual work. Scales automatically with every new scan.

---

## Architecture

### Model

**BERT-tiny** fine-tuned for binary sequence classification (KEEP or DROP per connection).

| Property | Value |
|----------|-------|
| Base model | BERT-tiny (L=2, H=128, A=2) |
| Parameters | 4.4M |
| Size (fp32) | 17MB |
| Size (int8 quantized) | **~5MB** |
| Inference on CPU (batch=100) | **<5ms** |
| Training compute | ~1 GPU-hour on 50k examples |
| Rust integration | `ort` crate (ONNX Runtime) + `tokenizers` crate |

BERT-tiny is the right choice because the classification signal is surface-level syntactic,
not semantic. `"= Client("` vs `"async def connect("` doesn't require deep language understanding.
The feature space is small and clean — tiny model generalises well.

### What the model sees (inputs)

Three inputs per connection:

1. **Evidence text** — the actual line of code (tokenized, ~20 tokens average)
2. **Protocol** — categorical: `rest`, `opcua`, `env`, `kubernetes`, `grpc`, etc.
3. **Extraction method prefix** — categorical: `pattern`, `wrapper_trace`, `spec`, `ast`, `library_resolution`

The model does NOT receive the full `extraction_method` value. It receives only the
prefix category (e.g. `wrapper_trace`, not `wrapper_trace:connect→= Client`). This forces
the model to learn from the evidence text — not to memorize method→label mappings.
If it just learned "wrapper_trace = DROP" it would be no better than a hardcoded rule.

**Examples:**

```
Input:
  evidence:  "async def connect(self) -> None:"
  protocol:  "opcua"
  em_prefix: "wrapper_trace"
Output: P(KEEP) = 0.03  →  DROP

Input:
  evidence:  "client = Client(url=endpoint, timeout=self._config.connection_timeout_s)"
  protocol:  "opcua"
  em_prefix: "pattern"
Output: P(KEEP) = 0.97  →  KEEP
```

### Output: three zones

| Probability | Action | Notes |
|-------------|--------|-------|
| ≥ 0.85 KEEP | Include in output | High confidence real connection |
| ≤ 0.15 KEEP | Exclude from output | High confidence noise |
| 0.15–0.85   | Include with `confidence: low` | Borderline — let hub decide |

Borderline connections are surfaced in the hub UI flagged for review. User confirms or
dismisses → those decisions become high-confidence training labels → model improves on
the exact edge cases that are genuinely hard.

---

## Training data pipeline

### Weak labeling (runs on hub, per scan upload)

```python
def weak_label(conn: RawConnection) -> tuple[str, float] | None:
    method = conn.extraction_method or ""
    evidence = conn.evidence or ""

    # High-confidence DROP
    if method.startswith("wrapper_trace:"):
        return ("DROP", 0.95)

    # High-confidence KEEP
    if method.startswith("spec:"):
        return ("KEEP", 0.95)
    if method.startswith("ast_"):
        return ("KEEP", 0.90)

    # Pattern match with resolved target → strong KEEP
    if method.startswith("pattern:") and conn.target and not conn.target.startswith("env:"):
        return ("KEEP", 0.90)

    # Env protocol: classify by variable name suffix in evidence
    if conn.protocol == "env":
        if any(s in evidence for s in ('_URL"', '_HOST"', '_ENDPOINT"', '_ADDR"', '_DSN"', '_BROKERS"')):
            return ("KEEP", 0.80)
        if any(s in evidence for s in ('_PATH"', '_DIR"', '_ID"', '_KEY"', '_LEVEL"',
                                        '_MODE"', '_POLICY"', '_PORT"', '_JSON"',
                                        '_SA"', '_NAMESPACE"', '_TIMEOUT"')):
            return ("DROP", 0.85)

    # Uncertain — exclude from training set
    return None
```

Connections where `weak_label` returns `None` are excluded from training. Only
high-confidence examples are kept. This keeps the training set clean.

### Training loop (weekly, fully automated)

```
Week N:
  Hub accumulates weak-labeled examples from all scan uploads
  → Filter to high-confidence labels only (confidence ≥ 0.80)
  → Deduplicate by (evidence_normalized, label)
  → Split 80/10/10 train/val/test

  Fine-tune BERT-tiny:
  → Start from HuggingFace pre-trained weights
  → 3 epochs, lr=2e-5, batch=32
  → Early stop on val F1

  Quality gates (ALL must pass before publishing):
  → F1 (KEEP) ≥ 0.92 on held-out set
  → F1 (DROP) ≥ 0.92 on held-out set
  → Precision (KEEP) ≥ 0.95  [asymmetric: better to show fewer than wrong]
  → ≤ 1% F1 regression vs previous version on any individual repo
  → ≥ 75% of connections classified with high confidence

  If all gates pass:
  → Export to ONNX (opset 14) → int8 quantize
  → Publish to models.arcanon.dev/classifier-v{N}.onnx
  → Update models.arcanon.dev/latest.json

Week N+1:
  Scanner checks latest.json on startup
  → Downloads new model if version changed
  → Caches at ~/.arcanon/models/classifier.onnx
  → Uses new model for all subsequent scans
```

### Bootstrapping: getting to the first model

The weekly loop needs a starter dataset. Three sources:

| Source | Examples | Quality |
|--------|----------|---------|
| opcua-adapter manual review (18 KEEP, 68 DROP) | 86 | Human-verified — held-out test set |
| The Stack (HuggingFace) — weak labels | ~50k | Weak but diverse, zero API calls |
| Synthetic DROP examples (wrapper trace templates) | ~5k | Balances class distribution |

**Do NOT clone GitHub repos directly** — hits rate limits, disk and time issues.

Use **The Stack** (HuggingFace) instead — 6TB of permissively-licensed source code,
streamable, no GitHub API calls, no rate limits:

1. Stream Python, TypeScript, Go, Java, Rust, Ruby files from The Stack
2. Filter to files containing known library imports (`asyncua`, `redis`, `psycopg2`,
   `boto3`, `kubernetes`, `ioredis`, etc.)
3. Apply scanner CDN pattern matching inline in Python → generates KEEP labels directly
4. Generate DROP labels from structural patterns:
   - `async def {name}(self) -> None:` → DROP
   - `await self.{something}.{method}()` → DROP
   - `task = asyncio.create_task({func}({arg}))` → DROP
5. Expected yield: ~50k labeled examples in ~2 hours, zero API calls

**Validation set (catches distribution shift):**
Separately scan 50–100 real repos with the actual arcanon scanner (shallow clone + delete).
Use these ~1,000 scanner-output examples as an evaluation set. Validates that The Stack
training distribution matches actual scanner output — important because The Stack is raw
source files, not scanner output.

Bootstrap step: stream The Stack → apply weak labeling → fine-tune BERT-tiny →
evaluate on held-out 86 human labels + validation set → publish v1 if gates pass.

---

## The flywheel

```
More repos scanned
      ↓
More weak-labeled training examples generated automatically
      ↓
Larger, more diverse training set
      ↓
Better classifier (higher F1, fewer false positives on novel repos)
      ↓
Cleaner scan output sent to hub
      ↓
Better dependency graphs in hub UI
      ↓
More teams adopt arcanon
      ↓
More repos scanned  ←── (loop)
```

The second, higher-quality loop:

```
Uncertain prediction (0.15–0.85 probability)
      ↓
Surfaced in hub UI flagged for user review
      ↓
User confirms / dismisses
      ↓
High-confidence label stored
      ↓
Added to training set for next cycle
      ↓
Model learns the borderline cases
      ↓
Fewer uncertain predictions over time
```

Over time the uncertain zone shrinks. The model becomes more decisive on edge cases
precisely because humans only need to label those — not the obvious ones.

---

## Rust integration

```rust
// src/transform/classifier.rs

pub struct Classifier {
    session: ort::Session,
    tokenizer: tokenizers::Tokenizer,
    threshold: f32,
}

impl Classifier {
    /// Load from ~/.arcanon/models/classifier.onnx
    /// Downloads from CDN if not present or outdated
    pub async fn load(config: &ClassifierConfig) -> Result<Self, ClassifierError> { ... }

    /// Returns filtered connections. Borderline cases included with confidence=low.
    pub fn filter(&self, connections: Vec<ConnectionInfo>) -> Vec<ConnectionInfo> {
        let inputs = self.prepare_inputs(&connections);
        let outputs = self.session.run(inputs)?;
        let probs = extract_keep_probabilities(outputs);

        connections.into_iter().zip(probs).filter_map(|(conn, p)| {
            if p >= self.threshold {
                Some(conn)
            } else if p >= 0.15 {
                Some(ConnectionInfo { confidence: Confidence::Low, ..conn })
            } else {
                None
            }
        }).collect()
    }
}
```

The classifier runs as a post-assembly, pre-serialization stage in `src/core/payload.rs`
(or a new `src/transform/`). It is **additive** — if the model is unavailable (no cache,
no CDN, network error), the scanner logs a warning and writes raw unfiltered output.
Never blocking.

### Configuration

```toml
# .arcanon.toml
[scanner.classifier]
enabled       = true       # default: true once model is published
threshold     = 0.85       # keep connections above this probability
model_version = "latest"   # or pin: "v3"
```

### Model delivery

- CDN: `models.arcanon.dev/classifier-v{N}.onnx` + `latest.json`
- Cache: `~/.arcanon/models/classifier.onnx`
- Same CDN infrastructure as pattern distribution (`patterns.arcanon.dev`)

---

## Privacy and data governance

Evidence lines are snippets of source code. Two options:

**Option A — Opt-in contribution** (`contribute = true` in `.arcanon.toml`, default off):
Raw connection list (with evidence) stored on hub for training. Opted-out repos
send only the filtered output — evidence lines not retained.

**Option B — Open source repos only (recommended for v1):**
Train exclusively on public GitHub repos. Private repo scans benefit from the model
but don't contribute to training. Cleanest approach, no consent complexity.

**Recommendation:** Option B for the first release, opt-in contribution as a v2 feature.

---

## Expected impact

| Scenario | Total connections | Real connections | Noise |
|----------|-----------------|-----------------|-------|
| Raw scanner output (today, opcua-adapter) | 86 | 18 | 79% |
| After classifier at bootstrap (~90% F1) | ~22 | ~18 | ~18% |
| After classifier at scale (~97% F1, 10k repos) | ~19 | ~18 | ~5% |

---

## What comes before this (prerequisites)

The classifier needs `extraction_method` and `evidence` fields on every `ConnectionPayload`
to operate. These were delivered in v1.2:

- `extraction_method` on every connection: **DQ-01** (Phase 13) ✓
- `evidence` field: present in `ConnectionInfo` (set by pattern engine) ✓
- Dependency field for lineage: **DQ-02** (Phase 13) ✓

The classifier is ready to be implemented once v1.2 ships.

---

## Implementation phases (v1.3 milestone)

| Phase | What | Notes |
|-------|------|-------|
| Phase A | `src/transform/classifier.rs` skeleton with `ort` + `tokenizers` | CDN download, cache management, fallback |
| Phase B | Bootstrap training script (Python) | The Stack streaming → weak labels → BERT-tiny fine-tune → ONNX export |
| Phase C | Integration in `src/core/payload.rs` | Post-assembly filter stage, config gate |
| Phase D | Hub-side weak labeling + training loop | Weekly automated retraining, quality gates |
| Phase E | Hub UI: borderline connection review | Human-in-the-loop for uncertain zone |

Phases A and C are scanner changes (this repo). Phases B, D, E are hub/infra work.

---

*Design originated: 2026-04-07 (opcua-adapter session)*
*Written to arcanon-scanner: 2026-04-08*
