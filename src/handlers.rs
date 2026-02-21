use axum::{Json, extract::State};
use axum::http::StatusCode;
use crate::models::{LLMRequest, LLMResponse};
use crate::client::call_llm;
use crate::cache::{generate_cache_key, get_embedding};
use crate::AppState;

pub async fn health_check() -> &'static str {

    "OK"

}

pub async fn proxy_handler(
    State(state): State<AppState>,
    Json(request): Json<LLMRequest>
) -> Result<Json<LLMResponse>, (StatusCode, String)> {

    // generate cache key
    let cache_key = generate_cache_key(&request);
    println!("Cache key: {}", cache_key);

    // Tier 1: Exact match cache (Redis)
    match state.redis_cache.get(&cache_key).await {
        Ok(Some(cache_response)) => {
            println!("Exact Cache Hit");
            // deserialize the cachen JSON string back to LLMResponse
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

    // Tier 2: Semantic cache (Qdrant)
    // extract prompt text for embedding
    let prompt_text: String = request.messages.iter()
        .map(|m| format!("{}: {}", m.role, m.content))
        .collect::<Vec<_>>()
        .join("\n");

    // get embedding — stored so it can be reused for Qdrant storage on a cache miss
    let maybe_embedding = get_embedding(&state.http_client, &state.embedding_url, &prompt_text).await;
    match &maybe_embedding {
        Ok(embedding) => {
            // Search for similar cached responses
            match state.qdrant_cache.search_similar(embedding.clone(), 0.90).await {
                Ok(Some(cached_response)) => {
                    println!("Semantic Cache Hit");
                    
                    // Store in Redis for faster future lookups
                    let _ = state.redis_cache.set(&cache_key, &cached_response).await;
                    
                    let response: LLMResponse = serde_json::from_str(&cached_response)
                        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Cache deserialization error: {}", e)))?;
                    return Ok(Json(response));
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

    // Tier 3: Cache miss - call LLM
    println!("Cache Miss - calling LLM"); 

    let response = call_llm(&state.http_client, &state.groq_api_key, request)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("LLM API error: {}", e)))?;

    // store in both caches
    let response_json = serde_json::to_string(&response)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Serialization error: {}", e)))?;
    
    // store in redis
    if let Err(e) = state.redis_cache.set(&cache_key, &response_json).await {
        println!("Warning: Failed to cache in Redis: {}", e);
    } else {
        println!("Stored in Redis");
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