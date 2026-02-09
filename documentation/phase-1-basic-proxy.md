# Phase 1: Basic HTTP Proxy

## Overview

A simple pass-through HTTP proxy that forwards chat completion requests to the Groq API. No caching implemented - every request hits the external API.

## What It Does

- Accepts chat completion requests at `/v1/chat/completions`
- Forwards requests to Groq's OpenAI-compatible API
- Returns LLM responses to clients
- Provides `/health` endpoint for monitoring
- Logs requests and responses for debugging

## Architecture

### File Structure

```
src/
├── main.rs       # Server setup, routing, env validation
├── models.rs     # Data structures (Message, LLMRequest, LLMResponse)
├── handlers.rs   # HTTP handlers (health_check, proxy_handler)
└── client.rs     # Groq API client (call_llm)
```

### Request Flow

```
Client → Axum Server → Handler → Groq API Client → Groq API → Response
```

1. Client POSTs JSON to `/v1/chat/completions`
2. `proxy_handler` validates and logs request
3. `call_llm` forwards to Groq with API key
4. Response parsed and returned to client

## API Endpoints

### Health Check
```bash
GET /health
# Returns: "OK"
```

### Chat Completions
```bash
curl -X POST http://localhost:3000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "llama-3.3-70b-versatile",
    "messages": [{"role": "user", "content": "Hello"}],
    "temperature": 0.7,
    "max_tokens": 100
  }'
```

**Required fields**: `model`, `messages`
**Optional fields**: `temperature`, `max_tokens`

## Key Design Decisions

**Separate files**: Easy debugging and future extensibility. Each file has one responsibility.

**Groq API**: Fast inference, OpenAI-compatible format, good model selection, free-tier.

**Error handling**: Uses `Result<Json<T>, (StatusCode, String)>` to return proper HTTP errors with messages.

**Environment config**: API key loaded from `.env` file, validated at startup.

**Optional parameters**: `temperature` and `max_tokens` are optional to allow Groq's defaults.

## Testing

### Setup
1. Create `.env` file:
   ```
   GROQ_API_KEY=your_key_here
   ```

2. Start server:
   ```bash
   cargo run
   # Output: listening on 0.0.0.0:3000
   ```

### Test Health Check
```bash
curl http://localhost:3000/health
# Expected: OK
```

### Test Chat Completion
```bash
curl -X POST http://localhost:3000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "llama-3.3-70b-versatile",
    "messages": [{"role": "user", "content": "Say hello"}]
  }'
```

Check terminal for logs:
```
Received request: LLMRequest { ... }
Got Response: LLMResponse { ... }
```

## Dependencies

| Crate | Purpose |
|-------|---------|
| `axum` 0.7 | Web framework |
| `tokio` 1.49.0 | Async runtime |
| `reqwest` 0.13.2 | HTTP client for Groq API |
| `serde` 1.0.228 | JSON serialization |
| `serde_json` 1.0.149 | JSON parsing |
| `dotenvy` 0.15.7 | Load `.env` files |

## Next Phase

Phase 2 adds in-memory caching with HashMap to avoid redundant API calls and save costs.
