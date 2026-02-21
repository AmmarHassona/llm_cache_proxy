# LLM Cache Proxy

A caching proxy for LLM APIs, written in Rust. Sits between your application and the LLM API, returning cached responses instead of making redundant API calls. Compatible with any client that uses the OpenAI API format.

## How It Works

Requests pass through two cache tiers before reaching the LLM:

```
Request
  │
  ▼
Tier 1: Redis (exact match)
  Hit  ──────────────────────────────→ Return cached response (~1ms)
  Miss
  │
  ▼
Tier 2: Qdrant (semantic similarity)
  Hit  ──→ Promote to Redis ─────────→ Return cached response (~10-50ms)
  Miss
  │
  ▼
Tier 3: Groq API
         Store in Redis + Qdrant ────→ Return response (~1-3s)
```

**Tier 1 — Exact match (Redis):** The request is normalized (whitespace, case) and hashed with SHA256. If the same prompt has been seen before, the response is returned immediately.

**Tier 2 — Semantic match (Qdrant):** If exact match fails, the prompt is converted into a 384-dimensional vector embedding and compared against all previously cached prompts using cosine similarity. If a semantically similar prompt is found (score ≥ 0.90), its response is returned. The result is also stored in Redis so the next identical request skips this tier entirely.

**Tier 3 — LLM call (Groq):** If both caches miss, the request is forwarded to Groq. The response is stored in both Redis and Qdrant for future use.

## Services

| Service | Role | Port |
|---------|------|------|
| Rust proxy | Request handling, cache orchestration | 3000 |
| Redis | Exact match cache | 6379 |
| Qdrant | Vector store for semantic search | 6333/6334 |
| Python FastAPI | Text embedding service (`all-MiniLM-L6-v2`) | 8001 |

All four services run together via Docker Compose.

## Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/v1/chat/completions` | Main proxy — OpenAI-compatible |
| `GET`  | `/health` | Health check for all services |
| `GET`  | `/metrics` | Cache performance and cost metrics |
| `GET`  | `/dashboard` | Live web dashboard |
| `POST` | `/admin/cache/clear` | Clear the Redis cache |
| `GET`  | `/admin/stats` | Metrics + service status combined |

## Prerequisites

- [Docker](https://docs.docker.com/get-docker/) and Docker Compose
- A [Groq API key](https://console.groq.com) (free tier available)

## Setup

**1. Clone the repository**

```bash
git clone https://github.com/AmmarHassona/llm_cache_proxy.git
cd llm_cache_proxy
```

**2. Create your `.env` file**

```bash
cp .env.example .env
```

Open `.env` and set your Groq API key:

```env
GROQ_API_KEY=your-api-key-here
```

The other variables have working defaults and do not need to be changed when running via Docker Compose.

**3. Create the logs directory**

```bash
mkdir -p logs
```

**4. Start everything**

```bash
docker-compose up --build
```

The proxy starts on port 3000 once Redis, Qdrant, and the embedding service all pass their health checks. The embedding service downloads the `all-MiniLM-L6-v2` model (~80MB) on first run — this takes a minute.

## Usage

The proxy is a drop-in replacement for any OpenAI-compatible client. Point `base_url` at `http://localhost:3000/v1`.

**With the OpenAI Python SDK:**

```python
from openai import OpenAI

client = OpenAI(
    base_url="http://localhost:3000/v1",
    api_key="any-string",  # the proxy uses its own Groq key
)

response = client.chat.completions.create(
    model="llama-3.3-70b-versatile",
    messages=[{"role": "user", "content": "What is Rust?"}],
    temperature=0.0,
)
print(response.choices[0].message.content)
```

**With curl:**

```bash
curl -X POST http://localhost:3000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "llama-3.3-70b-versatile",
    "messages": [{"role": "user", "content": "What is Rust?"}],
    "temperature": 0.0
  }'
```

### Supported Models

Any Groq model can be used. Pricing in `/metrics` is accurate for:

- `llama-3.3-70b-versatile`
- `llama-3.1-8b-instant`
- `llama-4-scout`
- `llama-4-maverick`
- `qwen3-32b`
- `kimi-k2-0905-1t`
- `gpt-oss-20b`, `gpt-oss-120b`

### Optional Request Headers

| Header | Example | Effect |
|--------|---------|--------|
| `x-bypass-cache` | `true` | Skip cache read and write, always call LLM |
| `x-cache-ttl` | `3600` | Override Redis TTL for this response (seconds) |

## Dashboard

Open [http://localhost:3000/dashboard](http://localhost:3000/dashboard) in your browser. It polls `/metrics` every 5 seconds and displays hit rate, token savings, cost savings, and cache distribution charts.

## Testing

A test script is included that uses the OpenAI Python SDK:

```bash
cd python_embedding
source venv/bin/activate  # or create one: python3 -m venv venv && pip install -r requirements.txt
python test_proxy.py
```

The script sends 5 requests designed to exercise each cache tier and prints a metrics summary at the end.

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `GROQ_API_KEY` | — | **Required.** Your Groq API key |
| `REDIS_URL` | `redis://127.0.0.1:6379` | Redis connection URL |
| `QDRANT_URL` | `http://127.0.0.1:6334` | Qdrant gRPC endpoint |
| `EMBEDDING_URL` | `http://127.0.0.1:8001/embed` | Embedding service endpoint |
| `LOG_PATH` | `./requests.log` | Path for the request log file |

When running via Docker Compose, the internal service hostnames (`redis`, `qdrant`, `embeddings`) are set automatically.

## Project Structure

```
.
├── src/
│   ├── main.rs        # App state, router setup
│   ├── handlers.rs    # HTTP handlers for all endpoints
│   ├── cache.rs       # Redis and Qdrant cache logic
│   ├── client.rs      # Groq API client
│   ├── models.rs      # Request/response types
│   ├── metrics.rs     # In-memory metrics counters
│   └── logger.rs      # Request log writer
├── python_embedding/
│   ├── main.py        # FastAPI embedding service
│   ├── test_proxy.py  # Integration test script
│   ├── Dockerfile
│   └── requirements.txt
├── documentation/
│   ├── phase-1-basic-proxy.md
│   ├── phase-2-exact-match-caching.md
│   ├── phase-3-semantic-caching.md
│   └── phase-4-production-polish.md
├── dashboard.html     # Single-page dashboard (served at /dashboard)
├── docker-compose.yml
├── Dockerfile
├── Cargo.toml
└── .env.example
```

## Tech Stack

- **Rust** — proxy server ([Axum](https://github.com/tokio-rs/axum), [Tokio](https://tokio.rs), [reqwest](https://github.com/seanmonstar/reqwest))
- **Redis** — exact match cache
- **Qdrant** — vector database for semantic search
- **Python / FastAPI** — embedding service ([sentence-transformers](https://www.sbert.net/), `all-MiniLM-L6-v2`)
- **Docker Compose** — multi-service orchestration
