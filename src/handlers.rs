use axum::{Json, extract::State, http::HeaderMap, response::{Html, IntoResponse}};
use axum::http::StatusCode;
use chrono::Utc;
use crate::models::{LLMRequest, LLMResponse};
use crate::client::call_llm;
use crate::cache::{generate_cache_key, get_embedding};
use crate::AppState;
use serde_json::json;
use crate::logger::log_request;

/// Returns (input_cost_per_1m_tokens, output_cost_per_1m_tokens) for Groq models
fn get_groq_model_pricing(model: &str) -> (f64, f64) {
    match model {
        // Llama models
        "llama-3.3-70b-versatile" => (0.59, 0.79),
        "llama-3.1-8b-instant" => (0.05, 0.08),
        "llama-4-scout" => (0.11, 0.34),
        "llama-4-maverick" => (0.20, 0.60),
        
        // Qwen models
        "qwen3-32b" => (0.29, 0.59),
        
        // Kimi models
        "kimi-k2-0905-1t" => (1.00, 3.00),
        
        // GPT OSS models
        "gpt-oss-20b" => (0.075, 0.30),
        "gpt-oss-safeguard-20b" => (0.075, 0.30),
        "gpt-oss-120b" => (0.15, 0.60),
        
        // Default to Llama 3.3 70B pricing (most common)
        _ => {
            eprintln!("Warning: Unknown model '{}', using Llama 3.3 70B pricing", model);
            (0.59, 0.79)
        }
    }
}

fn calculate_cost(model: &str, tokens: u64) -> f64 {

    let (input_price, output_price) = get_groq_model_pricing(model);
    let avg_price = (input_price + output_price) / 2.0;
    (tokens as f64 / 1_000_000.0) * avg_price

}

pub async fn health_check(State(state): State<AppState>) -> impl IntoResponse {
    use crate::cache::check_embedding_service;

    let (redis_up, qdrant_up, embeddings_up) = tokio::join!(
        state.redis_cache.health_check(),
        state.qdrant_cache.health_check(),
        check_embedding_service(&state.http_client, &state.embedding_url)
    );

    let all_healthy = redis_up && qdrant_up && embeddings_up;
    let status = if all_healthy { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };

    let body = json!({
        "status": if all_healthy { "healthy" } else { "unhealthy" },
        "services": {
            "redis":      { "status": if redis_up      { "up" } else { "down" } },
            "qdrant":     { "status": if qdrant_up     { "up" } else { "down" } },
            "embeddings": { "status": if embeddings_up { "up" } else { "down" } }
        },
        "timestamp": Utc::now().to_rfc3339()
    });

    (status, Json(body))
}

pub async fn dashboard() -> Html<&'static str> {
    Html(include_str!("../dashboard.html"))
}

pub async fn proxy_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<LLMRequest>
) -> Result<Json<LLMResponse>, (StatusCode, String)> {

    let temperature = request.temperature.unwrap_or(0.0);

    let model = request.model.clone();

    let bypass_cache = headers
        .get("x-bypass-cache")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_lowercase() == "true")
        .unwrap_or(false);

    // Optional: Custom TTL
    let custom_ttl = headers
        .get("x-cache-ttl")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());

    if bypass_cache {
        println!("Cache bypass requested - skipping cache");
    }

    // generate cache key
    let cache_key = generate_cache_key(&request);
    println!("Cache key: {}", cache_key);

    // Tier 1: Exact match cache (Redis)
    if !bypass_cache {
        match state.redis_cache.get(&cache_key).await {
            Ok(Some(cache_response)) => {
                println!("Exact Cache Hit");

                state.metrics.record_exact_hit();

                log_request("EXACT_HIT", &model, 0, 0.0);

                // deserialize the cache JSON string back to LLMResponse
                let response = serde_json::from_str(&cache_response)
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Cache deserialization error: {}", e)))?;
                
                return Ok(Json(response));
            }
            Ok(None) => {
                println!("Exact Cache Miss");
            }
            Err(e) => {
                println!("Redis Error: {} - continuing", e);
            }
        }
    }
    // Tier 2: Semantic cache (Qdrant)
    // extract prompt text for embedding
    
    let prompt_text: String = request.messages.iter()
        .map(|m| format!("{}: {}", m.role, m.content))
        .collect::<Vec<_>>()
        .join("\n");

    // get embedding — stored so it can be reused for Qdrant storage on a cache miss
    let maybe_embedding = get_embedding(&state.http_client, &state.embedding_url, &prompt_text).await;
    
    if !bypass_cache {
        match &maybe_embedding {
            Ok(embedding) => {
                // Search for similar cached responses
                match state.qdrant_cache.search_similar(embedding.clone(), 0.90).await {
                    Ok(Some(cached_response)) => {
                        println!("Semantic Cache Hit");

                        let cached_llm_response: LLMResponse = serde_json::from_str(&cached_response)
                            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Cache deserialization error: {}", e)))?;
                        
                        let tokens = cached_llm_response.usage.total_tokens as u64;
                        state.metrics.record_semantic_hit(tokens);

                        let cost = calculate_cost(&model, tokens); 
                        log_request("SEMANTIC_HIT", &model, 0, cost); 
                        
                        // Store in Redis for faster future lookups
                        let _ = state.redis_cache.set(&cache_key, &cached_response).await;
                        
                        return Ok(Json(cached_llm_response));
                    }
                    Ok(None) => {
                        println!("Semantic cache miss");
                    }
                    Err(e) => {
                        println!("Qdrant search error: {} - continuing", e);
                    }
                }
            }
            Err(e) => {
                println!("Embedding error: {} - skipping semantic cache", e);
            }
        }
    }

    // Tier 3: Cache miss - call LLM
    println!("Cache Miss - calling LLM"); 

    let response = call_llm(&state.http_client, &state.groq_api_key, request)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("LLM API error: {}", e)))?;

    let tokens = response.usage.total_tokens as u64;
    state.metrics.record_miss(tokens);

    let cost = calculate_cost(&model, tokens); 
    log_request("MISS", &model, tokens, cost); 

    // store in both caches
    let response_json = serde_json::to_string(&response)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Serialization error: {}", e)))?;
    
    // store in redis with custom TTL if given
    let ttl = custom_ttl.unwrap_or_else(|| {
        if temperature > 0.7 {
            3600  // 1 hour for creative
        } else {
            86400  // 24 hours for deterministic
        }
    });
    
    if let Err(e) = state.redis_cache.set_with_ttl(&cache_key, &response_json, ttl).await {
        println!("Warning: Failed to cache in Redis: {}", e);
    } else {
        if custom_ttl.is_some() {
            println!("Stored in Redis (TTL: {}s)", ttl);
        }
        else {
            println!("Stored in Redis");
        }
    }

    // store in Qdrant — reuse embedding from semantic search, avoid a second HTTP call
    if let Ok(embedding) = maybe_embedding {
        if let Err(e) = state.qdrant_cache.store(&cache_key, embedding, &response_json).await {
            println!("Failed to cache in Qdrant: {}", e);
        } else {
            println!("Stored in Qdrant");
        }
    }

    Ok(Json(response))

}

pub async fn metrics(State(state): State<AppState>) -> Json<serde_json::Value> {
    let snapshot = state.metrics.snapshot();
    
    let hit_rate = snapshot.cache_hit_rate();
    let total_hits = snapshot.exact_hits + snapshot.semantic_hits;
    
    // Use default model for cost calculation
    // In production, you'd want to track which model was actually used
    let default_model = "llama-3.3-70b-versatile";
    let (input_price, output_price) = get_groq_model_pricing(default_model);
    
    let input_cost_per_token = input_price / 1_000_000.0;
    let output_cost_per_token = output_price / 1_000_000.0;
    
    // Rough estimate: assume 50/50 input/output split
    let avg_cost_per_token = (input_cost_per_token + output_cost_per_token) / 2.0;
    
    let cost_saved = snapshot.tokens_saved as f64 * avg_cost_per_token;
    let cost_spent = snapshot.tokens_used as f64 * avg_cost_per_token;
    let total_cost_without_cache = (snapshot.tokens_saved + snapshot.tokens_used) as f64 * avg_cost_per_token;
    
    Json(json!({
        "cache_performance": {
            "exact_hits": snapshot.exact_hits,
            "semantic_hits": snapshot.semantic_hits,
            "total_hits": total_hits,
            "misses": snapshot.misses,
            "total_requests": snapshot.total_requests,
            "hit_rate_percent": format!("{:.2}%", hit_rate)
        },
        "token_usage": {
            "tokens_saved": snapshot.tokens_saved,
            "tokens_used": snapshot.tokens_used,
            "total_tokens_without_cache": snapshot.tokens_saved + snapshot.tokens_used
        },
        "cost_analysis": {
            "cost_saved_usd": format!("${:.4}", cost_saved),
            "cost_spent_usd": format!("${:.4}", cost_spent),
            "total_cost_without_cache_usd": format!("${:.4}", total_cost_without_cache),
            "savings_percent": if total_cost_without_cache > 0.0 {
                format!("{:.2}%", (cost_saved / total_cost_without_cache) * 100.0)
            } else {
                "0.00%".to_string()
            },
            "note": format!("Costs calculated using {} pricing. Actual costs may vary if different models were used.", default_model)
        },
        "pricing": {
            "model_assumed": default_model,
            "input_per_1m_tokens": format!("${:.2}", input_price),
            "output_per_1m_tokens": format!("${:.2}", output_price),
            "supported_models": [
                "llama-3.3-70b-versatile", "llama-3.1-8b-instant",
                "llama-4-scout", "llama-4-maverick",
                "qwen3-32b", "kimi-k2-0905-1t",
                "gpt-oss-20b", "gpt-oss-safeguard-20b", "gpt-oss-120b"
            ]
        }
    }))
}

pub async fn admin_clear_cache(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    state.redis_cache.flush_all()
        .await
        .map_err(|e| (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to flush Redis: {}", e)}))
        ))?;

    println!("Admin: Redis cache cleared");

    Ok(Json(json!({
        "status": "success",
        "message": "Redis cache cleared"
    })))
}

pub async fn admin_stats(
    State(state): State<AppState>,
) -> Json<serde_json::Value> {
    use crate::cache::check_embedding_service;

    let (redis_up, qdrant_up, embeddings_up) = tokio::join!(
        state.redis_cache.health_check(),
        state.qdrant_cache.health_check(),
        check_embedding_service(&state.http_client, &state.embedding_url)
    );

    let snapshot = state.metrics.snapshot();

    Json(json!({
        "cache_stats": {
            "exact_hits": snapshot.exact_hits,
            "semantic_hits": snapshot.semantic_hits,
            "misses": snapshot.misses,
            "total_requests": snapshot.total_requests,
            "hit_rate": snapshot.cache_hit_rate()
        },
        "services": {
            "redis":      if redis_up      { "up" } else { "down" },
            "qdrant":     if qdrant_up     { "up" } else { "down" },
            "embeddings": if embeddings_up { "up" } else { "down" }
        }
    }))
}