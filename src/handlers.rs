use axum::{Json, extract::State};
use axum::http::StatusCode;
use crate::models::{LLMRequest, LLMResponse};
use crate::client::call_llm;
use crate::cache::{RedisCache, generate_cache_key};
use reqwest::Client;

pub async fn health_check() -> &'static str {

    "OK"

}

pub async fn proxy_handler(
    State((cache, http_client, groq_api_key)): State<(RedisCache, Client, String)>, // extract cache and http client
    Json(request): Json<LLMRequest>
) -> Result<Json<LLMResponse>, (StatusCode, String)> {

    // generate cache key
    let cache_key = generate_cache_key(&request);
    println!("Cache key: {}", cache_key);

    // check the cache
    match cache.get(&cache_key).await {
        Ok(Some(cache_response)) => {
            println!("Cache Hit");
            // deserialize the cachen JSON string back to LLMResponse
            let response = serde_json::from_str(&cache_response)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Cache deserialization error: {}", e)))?;
            return Ok(Json(response));
        }
        Ok(None) => {
            println!("Cache Miss - calling LLM");
        }
        Err(e) => {
            println!("Cache Error: {} - treating as miss", e);
        }
    }

    // cache miss, call the llm 
    let response = call_llm(&http_client, &groq_api_key, request)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("LLM API error: {}", e)))?;

    let response_json = serde_json::to_string(&response)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Serialization error: {}", e)))?;
    
    
    if let Err(e) = cache.set(&cache_key, &response_json).await {
        println!("Warning: Failed to cache response: {}", e);
    } else {
        println!("Stored in cache");
    }

    Ok(Json(response))

}