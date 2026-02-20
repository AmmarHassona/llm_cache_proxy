# Phase 2: Exact Match Caching with Redis

## Overview

Adds persistent, exact-match caching on top of the Phase 1 proxy. Before forwarding a request to Groq, the server checks Redis for a cached response. If one exists, it is returned immediately without calling the LLM. If not, the LLM is called, the response is stored in Redis with a 24-hour TTL, and returned to the client.

Repeated identical requests become free and near-instant.

## What It Does

- Generates a deterministic cache key from each request using SHA256
- Normalizes request content (trim whitespace, lowercase) before hashing
- Checks Redis before every LLM call
- Stores LLM responses in Redis with a 24-hour TTL
- Treats Redis failures as cache misses — the proxy continues working even if Redis goes down
- Persists cache across server restarts (Redis survives process exits)

**What "exact match" means**: The request must be byte-for-byte identical after normalization. `"What is Rust?"` and `"Tell me about Rust"` are different keys. Semantic similarity matching comes in Phase 3.

**Hit rate depends entirely on your use case.** Exact match caching is most effective when the same prompt text is reused repeatedly. For free-form conversational use (where every user types something slightly different), real-world hit rates can be as low as 5–15%. For templated or structured applications, hit rates can reach 70–90%. See [Performance Impact](#performance-impact) for details.

## Architecture

### File Structure

```
src/
├── main.rs       # Server setup, shared state (cache, http_client, api_key)
├── models.rs     # Data structures (Message, LLMRequest, LLMResponse)
├── handlers.rs   # HTTP handlers — now includes cache check logic
├── client.rs     # Groq API client — now accepts shared Client + api_key
└── cache.rs      # NEW: Cache key generation, RedisCache struct
```

### Request Flow

```
Client → Axum Server → Handler → Generate Cache Key
                                        ↓
                                  Check Redis
                                 ↙          ↘
                             HIT             MISS / ERROR
                              ↓                   ↓
                        Return cached        Call Groq API
                         response                 ↓
                                           Store in Redis
                                                  ↓
                                           Return response
```

1. Client POSTs JSON to `/v1/chat/completions`
2. `proxy_handler` generates a cache key from the request
3. Redis is checked for that key
4. **Cache hit**: deserialize stored JSON, return immediately
5. **Cache miss**: call Groq, serialize response, store in Redis, return
6. **Redis error**: log warning, treat as miss, call Groq normally

### Shared State

Three values are initialized once at startup and shared across all requests via Axum state:

```
(RedisCache, reqwest::Client, String)
   ↑              ↑               ↑
Redis conn     HTTP conn       Groq API
pool           pool            key
```

Sharing these avoids recreating connection pools per request.

## Cache Key Generation

### What Gets Hashed

Every field that affects the LLM response is included:

| Field | Example input | Normalized |
|-------|--------------|------------|
| `messages[].role` | `"User"` | `"user"` |
| `messages[].content` | `"  What is Rust?  "` | `"what is rust?"` |
| `model` | `"GPT-4"` | `"gpt-4"` |
| `temperature` | `0.7` | `"temp:0.7"` |
| `max_tokens` | `None` | `"tokens:none"` |

### Normalization Rules

- **Whitespace**: leading and trailing whitespace is trimmed (`trim()`)
- **Case**: both `role` and `content` are lowercased
- **Punctuation**: kept as-is — `"What is Rust?"` and `"What is Rust"` produce **different keys**
- **Internal whitespace**: not normalized — `"What  is  Rust"` and `"What is Rust"` produce **different keys**
- **Optional fields**: `None` values hash as `"temp:none"` / `"tokens:none"` — absent and zero are distinct

### Pre-Hash String Format

```
{role}:{content}|{role}:{content}|...|model:{model}|temp:{temperature}|tokens:{max_tokens}
```

Example for a single-message request:
```
user:what is rust?|model:llama-3.3-70b-versatile|temp:0.7|tokens:none
```

Multi-message conversations include all messages separated by `|`:
```
system:you are a helpful assistant|user:what is rust?|model:llama-3.3-70b-versatile|temp:0.7|tokens:100
```

### Cache Key Format

```
cache:exact:{sha256_hex}:{model}
```

Example:
```
cache:exact:a3f8c2d1e4b7a9f0c2e1d4b7a9f0c2e1d4b7a9f0c2e1d4b7a9f0c2e1d4b7a9:llama-3.3-70b-versatile
```

The model is appended after the hash for human readability when inspecting Redis directly.

### Normalization Example

These two requests produce the **same** cache key:

```json
// Request 1
{"model": "gpt-4", "messages": [{"role": "user", "content": "What is Rust?"}], "temperature": 0.7}

// Request 2 — extra whitespace, different casing
{"model": "gpt-4", "messages": [{"role": "user", "content": "   what is Rust?   "}], "temperature": 0.7}
```

These two requests produce **different** cache keys:

```json
// Different punctuation
{"messages": [{"role": "user", "content": "What is Rust?"}]}
{"messages": [{"role": "user", "content": "What is Rust"}]}

// Different temperature
{"messages": [{"role": "user", "content": "What is Rust?"}], "temperature": 0.7}
{"messages": [{"role": "user", "content": "What is Rust?"}], "temperature": 0.9}
```

## Caching Flow

### Cache Hit

```
→ generate_cache_key(request)
→ cache.get(key)          → Ok(Some(json_string))
→ serde_json::from_str()  → LLMResponse
→ return Ok(Json(response))   ← returned in <1ms, no API call
```

### Cache Miss

```
→ generate_cache_key(request)
→ cache.get(key)          → Ok(None)
→ call_llm(client, api_key, request)  → LLMResponse   (~1-3s)
→ serde_json::to_string() → json_string
→ cache.set(key, json_string, TTL=86400)
→ return Ok(Json(response))
```

### Redis Error

```
→ generate_cache_key(request)
→ cache.get(key)          → Err(redis_error)
→ log "Cache Error: {e} - treating as miss"
→ call_llm(...)           → LLMResponse   (falls through to normal path)
→ cache.set(...)          → likely also fails, warning logged
→ return Ok(Json(response))
```

Redis errors are **non-fatal**. The proxy degrades gracefully to a pass-through proxy rather than returning 500 errors to clients.

### Result Type for Redis Get

`cache.get()` returns `Result<Option<String>, redis::RedisError>`, which has three distinct states handled explicitly:

```rust
match cache.get(&cache_key).await {
    Ok(Some(json)) => { /* cache hit  — return immediately   */ }
    Ok(None)       => { /* cache miss — fall through to LLM  */ }
    Err(e)         => { /* Redis down — log, fall through     */ }
}
```

Using `if let Ok(Some(...))` would silently discard errors, making Redis outages invisible in logs.

## Key Design Decisions

**Redis over in-memory HashMap**: A `HashMap` lives only for the server process lifetime. Redis persists across restarts and, if needed, can be shared across multiple proxy instances. It also has built-in TTL support.

**24-hour TTL**: LLM responses for the same prompt are deterministic (at `temperature: 0`), or close enough to be useful cached, for at least a day. Adjust `CACHE_TTL_SECONDS` in `cache.rs` for your use case.

**Exact match before semantic**: Exact matching is O(1) — one Redis lookup. Semantic matching requires embedding generation and vector search, adding latency and cost to every request. Exact match catches the common case (repeated queries) cheaply. Phase 3 adds semantic matching on top.

**Redis failures as warnings, not errors**: A caching layer should never be a single point of failure for the proxy. If Redis goes down, clients continue to get responses — just slower and at cost.

**Connection pooling for Redis**: `ConnectionManager` handles reconnection automatically and is designed to be cloned cheaply (it wraps an internal `Arc`). Each `get`/`set` call clones the manager rather than opening a new TCP connection.

**Connection pooling for HTTP**: `reqwest::Client` maintains an internal connection pool to Groq. Creating a new `Client` per request would discard this pool, adding TLS handshake overhead to every LLM call. The client is created once in `main` and shared via state.

**Shared API key in state**: The Groq API key is read once at startup, validated with `.expect()`, and passed through state to the handler. This is cleaner than calling `env::var` on every request.

## Configuration

### Environment Variables

Create a `.env` file in the project root:

```env
GROQ_API_KEY=gsk_your_key_here
REDIS_URL=redis://127.0.0.1:6379
```

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `GROQ_API_KEY` | Yes | — | Groq API key. Server panics at startup if missing. |
| `REDIS_URL` | No | `redis://127.0.0.1:6379` | Redis connection URL. Falls back to localhost if unset. |

### TTL

Cache expiry is controlled by a module-level constant in `cache.rs`:

```rust
const CACHE_TTL_SECONDS: u64 = 86400; // 24 hours
```

Change this value and recompile to adjust TTL. There is no runtime configuration for TTL.

## Testing

### Setup

1. Create `.env` file:
   ```
   GROQ_API_KEY=your_key_here
   ```

2. Start Redis:
   ```bash
   redis-server
   ```

3. Start server:
   ```bash
   cargo run
   # Output: listening on 0.0.0.0:3000
   ```

### Test Cache Miss then Hit

Send the same request twice:

```bash
# First request — cache miss
curl -X POST http://localhost:3000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "llama-3.3-70b-versatile",
    "messages": [{"role": "user", "content": "What is Rust?"}],
    "temperature": 0.7
  }'
```

Server logs:
```
Cache key: cache:exact:a3f8c2....:llama-3.3-70b-versatile
Cache Miss - calling LLM
Stored in cache
```

```bash
# Second request — identical, should hit cache
curl -X POST http://localhost:3000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "llama-3.3-70b-versatile",
    "messages": [{"role": "user", "content": "What is Rust?"}],
    "temperature": 0.7
  }'
```

Server logs:
```
Cache key: cache:exact:a3f8c2....:llama-3.3-70b-versatile
Cache Hit
```

### Test Normalization

This request should produce a **cache hit** for the above (whitespace trimmed, content lowercased):

```bash
curl -X POST http://localhost:3000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "llama-3.3-70b-versatile",
    "messages": [{"role": "user", "content": "   what is Rust?   "}],
    "temperature": 0.7
  }'
```

### Test Cache Persistence

1. Send a request (cache miss + store)
2. Stop the server (`Ctrl+C`)
3. Start the server again (`cargo run`)
4. Send the same request — should be a cache hit

### Verify Redis Directly

```bash
# List all cache keys
redis-cli KEYS "cache:exact:*"

# Inspect a specific key
redis-cli GET "cache:exact:a3f8c2....:llama-3.3-70b-versatile"

# Check TTL remaining (in seconds)
redis-cli TTL "cache:exact:a3f8c2....:llama-3.3-70b-versatile"

# Count cached entries
redis-cli KEYS "cache:exact:*" | wc -l

# Clear all cache entries (useful for testing)
redis-cli KEYS "cache:exact:*" | xargs redis-cli DEL
```

### Run Unit Tests

```bash
cargo test
```

The `test_same_prompts_same_key` test verifies that whitespace and casing differences produce identical cache keys.

## Performance Impact

| Scenario | Latency | Cost |
|----------|---------|------|
| Phase 1 (no cache) | ~1–3s per request | Every request billed |
| Phase 2 cache miss | ~1–3s (same as Phase 1) | Billed once |
| Phase 2 cache hit | <1ms | Free |

The first request for any unique prompt pays the full cost. Every subsequent identical request is free.

### Expected Hit Rates by Use Case

Hit rate depends on how much prompt variance exists in your workload:

| Use Case | Expected Hit Rate | Why |
|----------|-----------------|-----|
| FAQ bot (fixed question set) | 70–90% | Users select from predefined prompts |
| Templated code generation | 60–80% | Structured inputs repeat frequently |
| Document summarization pipeline | 50–70% | Same documents processed multiple times |
| General-purpose assistant | 5–15% | Every user types something different |
| Free-form chat | <5% | Conversational context is unique per session |

**For conversational applications, exact match caching has limited impact.** Two users asking about the same topic will almost never send byte-for-byte identical messages. `"What is Rust?"`, `"what is rust"`, `"Can you explain Rust?"`, and `"Tell me about Rust"` are all different cache keys — only the first two will hit each other (after normalization).

This is the core motivation for Phase 3: semantic caching catches paraphrased queries that exact matching misses entirely.

## What's Not Implemented Yet (Phase 3 Preview)

**Semantic caching**: `"What is Rust?"` and `"Tell me about the Rust programming language"` are different cache keys today. Phase 3 will use text embeddings + vector similarity search (Qdrant) to match semantically equivalent prompts.

**Cache analytics**: Hit/miss rates are logged but not tracked numerically. Phase 3 will add metrics.

**Multi-tier cache (L1/L2)**: Currently every request — including hits — makes a network call to Redis. An in-memory L1 cache (e.g., `moka`) in front of Redis would serve hot keys in nanoseconds without any I/O.

**Cache invalidation**: There is no way to invalidate a specific cached entry short of deleting it directly in Redis or waiting for TTL expiry.

## Dependencies Added

| Crate | Version | Features | Purpose |
|-------|---------|----------|---------|
| `redis` | 0.27 | `tokio-comp`, `connection-manager` | Async Redis client with connection pooling |
| `sha2` | 0.10.9 | — | SHA256 hashing for cache key generation |

Full dependency list:

| Crate | Version | Purpose |
|-------|---------|---------|
| `axum` | 0.7 | Web framework |
| `tokio` | 1.49.0 | Async runtime |
| `reqwest` | 0.13.2 | HTTP client (connection pooled) |
| `serde` | 1.0.228 | JSON serialization |
| `serde_json` | 1.0.149 | JSON parsing |
| `dotenvy` | 0.15.7 | Load `.env` files |
| `sha2` | 0.10.9 | SHA256 hashing |
| `redis` | 0.27 | Redis client |

## Troubleshooting

**Server won't start — "Failed to connect to Redis"**
Redis is not running. Start it with `redis-server`. The server requires a Redis connection at startup by design — it validates the connection before accepting traffic.

**Cache never hits**
- Verify the request body is byte-for-byte identical (same model, temperature, max_tokens)
- Check that `temperature` or `max_tokens` aren't being added/omitted between requests — `None` and `0` are different keys
- Inspect the logged cache key — if the keys differ between requests, the inputs differ after normalization
- Run `redis-cli TTL <key>` to confirm the entry hasn't expired

**Redis error logged, proxy still works**
Expected behavior. Redis connection was lost mid-runtime. `ConnectionManager` will attempt to reconnect automatically on the next operation.

**Cached response looks stale**
The TTL is 24 hours. Delete the key manually with `redis-cli DEL <key>` or clear all cache entries and let them repopulate.

**Want a shorter/longer TTL**
Change `CACHE_TTL_SECONDS` in `cache.rs` and recompile. This only affects newly stored entries — existing entries keep their original TTL.

## Next Phase

Phase 3 adds semantic caching:
- Generate text embeddings for each request
- Store embeddings in Qdrant (vector database)
- On incoming request, find the nearest cached embedding by cosine similarity
- If similarity exceeds a threshold, return the cached response
- This catches paraphrased queries: `"What is Rust?"` ≈ `"Tell me about Rust"`
