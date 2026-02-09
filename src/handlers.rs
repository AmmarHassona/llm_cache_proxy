use axum::Json;
use axum::http::StatusCode;
use crate::models::{LLMRequest, LLMResponse};
use crate::client::call_llm;

pub async fn health_check() -> &'static str {

    "OK"

}

pub async fn proxy_handler(Json(request): Json<LLMRequest>) -> Result<Json<LLMResponse>, (StatusCode, String)> {

    println!("Received request: {:?}", request);

    let response = call_llm(request)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("LLM API error: {}", e)))?;

    println!("Got Response {:?}", response);

    Ok(Json(response))

}