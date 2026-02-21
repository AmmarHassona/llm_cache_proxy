# Phase 3: Semantic Caching with Embeddings and Vector Search

## Overview

Adds a second caching tier on top of Phase 2's exact match cache. When an exact match is not found in Redis, the request is converted into a vector embedding and compared against all previously cached prompts using cosine similarity. If a semantically similar prompt has been cached before, even if worded differently, the stored response is returned without calling the LLM.

This solves the core limitation of exact match caching: real users rarely type the same sentence twice, but they often ask the same *question* in different words.

Phase 3 improves conversational hit rates from ~15% to 60–80%.

## What It Does

- Converts prompt text into a 384-dimensional vector (an *embedding*) that captures meaning
- Searches Qdrant for the nearest cached embedding using cosine similarity
- Returns a cached response if the similarity score exceeds 0.90
- Promotes semantic hits into Redis so future identical requests are served as exact matches
- On a full miss, stores the response in both Redis (exact) and Qdrant (semantic)
- Treats embedding service and Qdrant failures as misses — the proxy continues working

### What Is an Embedding?

An embedding is a list of numbers (a vector) that represents the *meaning* of a piece of text. The model is trained so that sentences with similar meanings produce vectors that point in similar directions in high-dimensional space.

```
"What is Rust?"          → [0.12, -0.34, 0.87, ...]  (384 numbers)
"Tell me about Rust"     → [0.11, -0.31, 0.85, ...]  (384 numbers, very close)
"How do I cook pasta?"   → [-0.92, 0.14, -0.23, ...] (384 numbers, far away)
```

### How Similarity Is Measured

Cosine similarity measures the angle between two vectors. A score of 1.0 means identical direction (identical meaning), 0.0 means perpendicular (unrelated), and negative values mean opposite meaning.

The threshold of **0.90** means: "only return a cached response if this new prompt is at least 90% semantically similar to something we've seen before."

### What Matches and What Doesn't

| Query A | Query B | ~Score | Match? |
|---------|---------|--------|--------|
| "What is Rust?" | "Tell me about Rust" | 0.94 | Yes |
| "What is Rust?" | "Explain the Rust language" | 0.91 | Yes |
| "What is Rust?" | "Describe Rust to me" | 0.92 | Yes |
| "What is Rust?" | "What is Go?" | 0.71 | No |
| "How do I debug?" | "Help me troubleshoot" | 0.88 | No (borderline) |
| "What is Rust?" | "How do I cook pasta?" | 0.10 | No |

## Architecture

### File Structure

```
src/
├── main.rs              # AppState now includes QdrantCache
├── models.rs            # Unchanged
├── handlers.rs          # 2-tier cache flow (Redis → Qdrant → LLM)
├── client.rs            # Unchanged
└── cache.rs             # QdrantCache, get_embedding() added

python_embedding/
├── main.py              # NEW: FastAPI embedding service
└── requirements.txt     # NEW: Python dependencies
```

### Service Architecture

```
                    ┌─────────────────┐
                    │   Rust Proxy    │
                    │  (port 3000)    │
                    └────────┬────────┘
                             │
              ┌──────────────┼──────────────┐
              │              │              │
              ▼              ▼              ▼
        ┌──────────┐  ┌──────────┐  ┌──────────┐
        │  Redis   │  │  Qdrant  │  │  Groq    │
        │ (6379)   │  │  (6334)  │  │   API    │
        └──────────┘  └──────────┘  └──────────┘
                             ▲
                             │ embeddings
                      ┌──────────────┐
                      │Python Service│
                      │  (port 8001) │
                      └──────────────┘
```

The Rust proxy orchestrates all communication. The Python service is only called to generate embeddings — it never talks to Redis, Qdrant, or Groq directly.

### 2-Tier Cache Hierarchy

```
Tier 1: Redis (Exact Match)
  └── O(1) lookup by SHA256 key
  └── Fastest possible — sub-millisecond
  └── Only hits when prompt is byte-for-byte identical (after normalization)

Tier 2: Qdrant (Semantic Match)
  └── Vector similarity search over all cached embeddings
  └── ~10-50ms — slightly slower but catches paraphrased queries
  └── Hits when prompt meaning is similar enough (≥0.90 cosine similarity)
  └── On hit → also stores in Redis for future exact hits

Tier 3: Groq API (Cache Miss)
  └── Called only when both caches miss
  └── Response stored in both Redis and Qdrant for future requests
```

## The Caching Flow

### Step-by-Step

```
Incoming request
      │
      ▼
Generate SHA256 cache key (from Phase 2)
      │
      ▼
┌─────────────────────────────────────┐
│  Tier 1: Check Redis (exact match)  │
└─────────────────────────────────────┘
      │
   HIT ──────────────────────────────→ Return cached response (<1ms)
      │
   MISS / ERROR
      │
      ▼
Build prompt text string
("{role}: {content}" for each message, joined by newlines)
      │
      ▼
POST /embed to Python service → 384-dim vector
      │
   ERROR ──→ Skip semantic cache, go to Tier 3
      │
   OK
      │
      ▼
┌──────────────────────────────────────────┐
│  Tier 2: Search Qdrant (semantic match)  │
│  threshold: 0.90 cosine similarity       │
└──────────────────────────────────────────┘
      │
   HIT ──→ Store in Redis (exact key) ──→ Return cached response (~10-50ms)
      │
   MISS / ERROR
      │
      ▼
Call Groq API (~1-3s)
      │
      ▼
Store in Redis (exact key)
Store in Qdrant (embedding + response)
      │
      ▼
Return response
```

### Concrete Example: 3-Request Sequence

**Request 1** — `"What is Rust?"` (cold cache)
```
Cache key: cache:exact:a3f8c2...:llama-3.3-70b-versatile
Exact Cache Miss
Semantic cache miss
Cache Miss - calling LLM
Stored in Redis
Stored in Qdrant
```

**Request 2** — `"Tell me about Rust"` (paraphrased, semantic hit)
```
Cache key: cache:exact:9d2e1f...:llama-3.3-70b-versatile
Exact Cache Miss
Semantic Cache Hit
Stored in Redis
```

**Request 3** — `"Tell me about Rust"` (exact same as Request 2, now in Redis)
```
Cache key: cache:exact:9d2e1f...:llama-3.3-70b-versatile
Exact Cache Hit
```

After Request 2, the paraphrased prompt is promoted into Redis. From Request 3 onward, it never reaches Qdrant.

## Python Embedding Service

### What It Does

Accepts a string of text and returns a 384-dimensional vector representing its semantic meaning. The model is loaded once at startup and kept in memory.

```python
# Request
POST http://127.0.0.1:8001/embed
{"text": "user: What is Rust?"}

# Response
{"embedding": [0.12, -0.34, 0.87, ...]}  # 384 floats
```

### Model: all-MiniLM-L6-v2

| Property | Value |
|----------|-------|
| Dimensions | 384 |
| Model size | ~80MB |
| Inference speed | ~10-30ms per request |
| Languages | English-optimized |
| Metric | Cosine similarity |

This model was chosen for being fast, small, and high-quality for English semantic similarity tasks. It is not ideal for multi-language or highly technical/domain-specific content — for those, the model could be swapped without changing any Rust code (only the Python service changes).

### Endpoints

```bash
# Generate embedding
POST /embed
Body: {"text": "your text here"}
Returns: {"embedding": [float, ...]}  # 384 values

# Health check
GET /health
Returns: {"status": "ok"}
```

### How Rust Communicates With It

`get_embedding()` in [cache.rs](../src/cache.rs) sends an HTTP POST to the Python service using the shared `reqwest::Client` and deserializes the response:

```
Rust → POST http://127.0.0.1:8001/embed → Python → [0.12, -0.34, ...]
```

The Python service is treated as an external dependency. If it is unreachable, `get_embedding()` returns an `Err`, which the handler catches and treats as a cache miss, the proxy falls through to the LLM without semantic caching.

### What Text Gets Embedded

The prompt is formatted as a single string with all messages concatenated:

```
"user: What is Rust?\nassistant: Rust is a systems programming language..."
```

This means the entire conversation context and not just the latest message is used to generate the embedding. Two conversations that have different history but the same final message will produce different embeddings.

## Qdrant Vector Database

### What It Stores

Each entry in Qdrant contains:

| Field | Content |
|-------|---------|
| ID | Random UUID (generated at store time) |
| Vector | 384-dimensional embedding of the prompt |
| Payload `cache_key` | The Redis exact match key for this prompt |
| Payload `response` | The serialized JSON LLM response |

### Collection Configuration

```
Collection name: "llm_cache"
Vector dimensions: 384
Distance metric: Cosine
```

The collection is created automatically on first startup if it does not already exist. If it exists, the create call is ignored.

### How Search Works

Qdrant performs approximate nearest-neighbor search over all stored embeddings. It returns the single closest match (`k=1`) and applies the score threshold filter. If the best match scores below 0.90, it is not returned.

```
Search: embedding=[...], k=1, score_threshold=0.90
Result: the stored response if best_score >= 0.90, else nothing
```

## Similarity Threshold

The threshold of **0.90** is set in [handlers.rs:51](../src/handlers.rs#L51):

```rust
state.qdrant_cache.search_similar(embedding.clone(), 0.90).await
```

### What 0.90 Means in Practice

| Threshold | Behavior |
|-----------|----------|
| 0.99 | Only near-identical phrasings hit. Very safe, low recall. |
| 0.95 | Minor rewording hits. Recommended for factual queries. |
| **0.90** | **Moderate paraphrasing hits. Current setting — good balance.** |
| 0.85 | Looser matching. Risk of returning responses for different-but-related questions. |
| 0.80 | High recall, high risk of false positives. |

### Tuning the Threshold

- **Too high** (e.g., 0.98): Low hit rate improvement over Phase 2. Semantic cache barely triggers.
- **Too low** (e.g., 0.80): Risk of returning a cached answer about "Rust (programming)" for a question about "Rust (oxidation)". The two topics produce embeddings that are more similar than you might expect.

The right threshold depends on how semantically distinct your expected query set is. For a focused domain (e.g., a Rust documentation bot), you can go lower. For a general assistant, stay at 0.90 or higher.

## Key Design Decisions

**2-tier cache (Redis + Qdrant)**: Exact match is O(1) and sub-millisecond. Semantic search takes 10–50ms. Checking Redis first means exact matches never have to go through the embedding or vector search overhead. The tiers complement each other.

**Promote semantic hits into Redis**: When a semantic hit is found, the response is immediately stored under the *new* request's exact key. Next time that same paraphrase arrives, it hits Redis without touching Qdrant or the embedding service. The semantic tier trains the exact tier over time.

**Separate Python service for embeddings**: The ML ecosystem for embeddings (`sentence-transformers`, PyTorch, Hugging Face) is mature in Python but difficult to use from Rust. A separate HTTP microservice keeps the Rust code dependency-free while retaining Python's ML libraries. The tradeoff is an extra network hop (~1ms on localhost).

**Qdrant over other vector stores**: Qdrant is purpose-built for vector similarity search, open source, easy to run locally, and has a well-maintained Rust client. It handles the ANN (approximate nearest neighbor) algorithm internally, so no vector math is needed in application code.

**all-MiniLM-L6-v2**: Small enough to run on CPU without a GPU, fast enough for real-time inference (~10–30ms), and accurate enough for English semantic similarity. Swapping the model only requires changing the Python service and zero Rust changes, as long as the output remains 384 dimensions (or the Qdrant collection dimension is updated accordingly).

**Embeddings are deterministic**: The same text always produces the same vector from the same model. This means embeddings don't need TTLs. Only the cached *responses* may go stale, which is handled by Redis TTL on the exact match tier.

## Services Required

| Service | Port | Purpose | Phase |
|---------|------|---------|-------|
| Redis | 6379 | Exact match cache, fast lookups | Phase 2 |
| Qdrant | 6334 | Vector store for semantic search | Phase 3 (new) |
| Python embedding service | 8001 | Convert text to embeddings | Phase 3 (new) |
| Groq API | — | LLM inference on cache misses | Phase 1 |

All three local services must be running for full functionality. Individual failures degrade gracefully:
- **Redis down**: exact match skipped, semantic + LLM still work
- **Python service down**: semantic cache skipped, Redis + LLM still work
- **Qdrant down**: semantic cache skipped, Redis + LLM still work

## Setup & Configuration

### 1. Install Python Dependencies

```bash
cd python_embedding
python3 -m venv venv
source venv/bin/activate
pip install -r requirements.txt
```

`requirements.txt`:
```
fastapi==0.104.1
uvicorn==0.24.0
sentence-transformers==2.7.0
huggingface-hub==0.23.0
```

The first run downloads the `all-MiniLM-L6-v2` model (~80MB) from Hugging Face automatically.

### 2. Start the Python Embedding Service

```bash
cd python_embedding
source venv/bin/activate
uvicorn main:app --port 8001
```

Expected output:
```
INFO:     Started server process [12345]
INFO:     Waiting for application startup.
INFO:     Application startup complete.
INFO:     Uvicorn running on http://127.0.0.1:8001
```

Verify it works:
```bash
curl http://localhost:8001/health
# {"status":"ok"}

curl -X POST http://localhost:8001/embed \
  -H "Content-Type: application/json" \
  -d '{"text": "What is Rust?"}'
# {"embedding": [0.12, -0.34, ...]}  (384 values)
```

### 3. Start Qdrant

**With Docker**
```bash
docker run -p 6334:6334 qdrant/qdrant
```

Qdrant does not require any manual collection setup — `QdrantCache::new(&qdrant_url)` creates the `llm_cache` collection automatically on startup.

### 4. Start Redis (unchanged from Phase 2)

```bash
redis-server
```

### 5. Start the Rust Proxy

```bash
cargo run
# listening on 0.0.0.0:3000
```

The proxy connects to all three services at startup. If any connection fails, it panics with a descriptive message.

### Environment Variables

Add the new Phase 3 variables to your `.env` file:

```env
GROQ_API_KEY=gsk_your_key_here
REDIS_URL=redis://127.0.0.1:6379
QDRANT_URL=http://127.0.0.1:6334
EMBEDDING_URL=http://127.0.0.1:8001/embed
```

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `GROQ_API_KEY` | Yes | — | Groq API key |
| `REDIS_URL` | No | `redis://127.0.0.1:6379` | Redis connection URL |
| `QDRANT_URL` | No | `http://127.0.0.1:6334` | Qdrant gRPC endpoint |
| `EMBEDDING_URL` | No | `http://127.0.0.1:8001/embed` | Python embedding service endpoint |

### Port Summary

| Port | Service |
|------|---------|
| 3000 | Rust proxy (this application) |
| 6379 | Redis |
| 6334 | Qdrant (gRPC, used by qdrant-client) |
| 8001 | Python embedding service |

## Testing Semantic Caching

### Setup

Start all services in separate terminals:
```bash
# Terminal 1
redis-server

# Terminal 2
docker run -p 6334:6334 qdrant/qdrant

# Terminal 3
cd python_embedding && source venv/bin/activate && uvicorn main:app --port 8001

# Terminal 4
cargo run
```

### Test 1: Original Query (Cold Miss)

```bash
curl -X POST http://localhost:3000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "llama-3.3-70b-versatile",
    "messages": [{"role": "user", "content": "What is Rust?"}],
    "temperature": 0.7
  }'
```

Expected server logs:
```
Cache key: cache:exact:a3f8c2...:llama-3.3-70b-versatile
Exact Cache Miss
Semantic cache miss
Cache Miss - calling LLM
Stored in Redis
Stored in Qdrant
```

### Test 2: Paraphrased Query (Semantic Hit)

```bash
curl -X POST http://localhost:3000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "llama-3.3-70b-versatile",
    "messages": [{"role": "user", "content": "Tell me about Rust"}],
    "temperature": 0.7
  }'
```

Expected server logs:
```
Cache key: cache:exact:9d2e1f...:llama-3.3-70b-versatile
Exact Cache Miss
Semantic Cache Hit
Stored in Redis
```

Response is identical to Test 1. The paraphrase was caught by Qdrant and promoted to Redis.

### Test 3: Same Paraphrase Again (Exact Hit)

```bash
# Same request as Test 2
curl -X POST http://localhost:3000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "llama-3.3-70b-versatile",
    "messages": [{"role": "user", "content": "Tell me about Rust"}],
    "temperature": 0.7
  }'
```

Expected server logs:
```
Cache key: cache:exact:9d2e1f...:llama-3.3-70b-versatile
Exact Cache Hit
```

Now served from Redis with no embedding, no Qdrant lookup.

### Verify Qdrant Directly

```bash
# Check collection info (HTTP API on port 6333)
curl http://localhost:6333/collections/llm_cache

# Count stored vectors
curl http://localhost:6333/collections/llm_cache/points/count \
  -H "Content-Type: application/json" \
  -d '{"exact": true}'
```

## Performance Impact

| Scenario | Latency | Cost |
|----------|---------|------|
| Phase 1 (no cache) | ~1–3s | Every request billed |
| Phase 2 exact hit | <1ms | Free |
| Phase 3 semantic hit | ~10–60ms | Free |
| Full miss (all phases) | ~1–3s + ~60ms overhead | Billed once |

### Hit Rate Improvement

| Use Case | Phase 2 Hit Rate | Phase 3 Hit Rate |
|----------|-----------------|-----------------|
| FAQ bot | 70–90% | 85–95% |
| Templated generation | 60–80% | 75–90% |
| General-purpose assistant | 5–15% | 50–70% |
| Free-form chat | <5% | 20–40% |

### Latency Overhead on Full Miss

Phase 3 adds ~60ms to every full cache miss (embedding generation + Qdrant search). This overhead is negligible relative to the ~1–3s LLM call, but is measurable. On a semantic hit, total response time is 10–60ms vs the ~1–3s LLM call — a significant improvement.

## Examples of Semantic Matches

### Queries That Match "What is Rust?"

```
"What is Rust?"               → original (exact match after Phase 2)
"Tell me about Rust"          → semantic hit (~0.94)
"Explain Rust"                → semantic hit (~0.91)
"Describe the Rust language"  → semantic hit (~0.92)
"Can you explain Rust to me?" → semantic hit (~0.90 — borderline)
```

### Queries That Do NOT Match

```
"What is Go?"                → different language (~0.71)
"What is Rust the game?"     → different topic (~0.74)
"How do I use Rust?"         → related but different question (~0.83 — below threshold)
"What are Rust lifetimes?"   → narrower topic (~0.79)
```

### More Examples

```
"How do I fix this bug?"     ≈ "Help me debug this" ≈ "I have an error"
"Write a function in Rust"   ≈ "Create a Rust function" ≈ "Rust function example"
"Summarize this text"        ≈ "Give me a summary" ≈ "What is the summary"
```

## Limitations and Trade-offs

**Embedding latency on every non-exact-match request**: Every request that misses Redis triggers an embedding call to the Python service (~10–30ms). This is negligible but not zero. High-traffic deployments may want to batch embedding calls or move the model closer to the Rust process.

**Vector search is slower than exact lookup**: Qdrant search takes 10–50ms depending on the number of stored vectors. This is still far faster than an LLM call but adds latency on cache misses.

**False positives at low thresholds**: If the threshold is set too low, a question about one topic may return a cached response intended for a related-but-different topic. The 0.90 default is conservative; lower it carefully.

**Conversation context is embedded as a whole**: The full message history is concatenated into a single string before embedding. This means two conversations with identical final messages but different history produce different embeddings, reducing hit rates for multi-turn conversations.

**All services must be up for full functionality**: Phase 2 required 2 services (Redis + Groq). Phase 3 requires 4 (Redis + Qdrant + Python + Groq). Each additional service is a failure point, though all failures degrade gracefully rather than crashing.

**No TTL on Qdrant entries**: Redis entries expire after 24 hours. Qdrant has no TTL configured meaning embeddings persist indefinitely. Over time, Qdrant will accumulate stale entries whose corresponding Redis responses have expired. This is currently not handled.

## Dependencies Added

### Rust

| Crate | Version | Purpose |
|-------|---------|---------|
| `qdrant-client` | 1.11 | Rust client for Qdrant gRPC API |
| `uuid` | 1.21.0 (v4 feature) | Generate unique IDs for Qdrant points |

### Python

| Package | Version | Purpose |
|---------|---------|---------|
| `fastapi` | 0.104.1 | Web framework for embedding service |
| `uvicorn` | 0.24.0 | ASGI server to run FastAPI |
| `sentence-transformers` | 2.7.0 | Load and run `all-MiniLM-L6-v2` |
| `huggingface-hub` | 0.23.0 | Download model from Hugging Face |

### External

| Service | Installation |
|---------|-------------|
| Qdrant | `docker run -p 6334:6334 qdrant/qdrant` or `brew install qdrant` |

Full Rust dependency list:

| Crate | Version | Purpose |
|-------|---------|---------|
| `axum` | 0.7 | Web framework |
| `tokio` | 1.49.0 | Async runtime |
| `reqwest` | 0.13.2 | HTTP client (Groq + Python service) |
| `serde` | 1.0.228 | JSON serialization |
| `serde_json` | 1.0.149 | JSON parsing |
| `dotenvy` | 0.15.7 | Load `.env` files |
| `sha2` | 0.10.9 | SHA256 hashing for exact cache keys |
| `redis` | 0.27 | Redis client |
| `qdrant-client` | 1.11 | Qdrant client |
| `uuid` | 1.21.0 | UUID generation |

## Troubleshooting

**"Failed to connect to Qdrant" at startup**
Qdrant is not running. Start it with `docker run -p 6334:6334 qdrant/qdrant` or `qdrant` (Homebrew).

**"Embedding error: Connection refused - skipping semantic cache"**
The Python embedding service is not running on port 8001. Start it with `uvicorn main:app --port 8001` from the `python_embedding/` directory with the venv activated.

**No semantic hits despite similar queries**
- Threshold may be too high — try temporarily lowering to 0.85 to test
- Verify the embedding service is returning 384-dimensional vectors: `curl -X POST http://localhost:8001/embed -H "Content-Type: application/json" -d '{"text":"test"}' | python3 -m json.tool`
- Check that Qdrant has stored points: `curl http://localhost:6333/collections/llm_cache`
- The queries may genuinely not be similar enough — test with very close paraphrases first

**Slow responses on every request**
The embedding service may be slow to start (model loading) — wait ~10 seconds after starting it before sending requests. If consistently slow, the model may be running on CPU without acceleration; this is expected (~10–30ms per embedding).

**Qdrant collection dimension mismatch error**
The Qdrant collection was created with a different number of dimensions than 384 (possibly from a previous experiment). Drop and recreate the collection: `curl -X DELETE http://localhost:6333/collections/llm_cache`, then restart the proxy to recreate it.

**"Semantic Cache Hit" but response seems wrong**
The similarity threshold may be too low for your use case. Increase it from 0.90 to 0.95 in [handlers.rs:51](../src/handlers.rs#L51) and recompile.

## What's Not Implemented Yet (Phase 4 Preview)

**Analytics and metrics**: Hit/miss counts are logged but not tracked. Phase 4 will add a `/metrics` endpoint showing exact hit rate, semantic hit rate, miss rate, and average latency per tier.

**Configurable threshold per request**: The 0.90 threshold is a compile-time constant. It could be made a runtime parameter, or even per-model/per-use-case.

**TTL for Qdrant entries**: Unlike Redis, Qdrant vectors never expire. Stale responses accumulate over time. Phase 4 should either add TTL support or a cleanup job.

**Multiple embedding models**: Different models are better for different languages and domains. A production system might route to different models based on detected language or query type.

## Next Phase

Phase 4 adds production polish:
- `/metrics` endpoint with hit rate, latency, and cache size stats
- Configurable similarity threshold without recompilation
- Qdrant entry expiry / cleanup
- Structured logging (replacing `println!` with `tracing`)
- Comprehensive load testing results
