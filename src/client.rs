use reqwest::Client;
use crate::models::{LLMRequest, LLMResponse};
use std::env;

pub async fn call_llm(request: LLMRequest) -> Result<LLMResponse, reqwest::Error> {

    let api_key = env::var("GROQ_API_KEY")
        .expect("GROQ_API_KEY checked at startup");

    let client = Client::new();

    let response = client
        .post("https://api.groq.com/openai/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&request)
        .send()
        .await?;

    let llm_response: LLMResponse = response
        .error_for_status()?
        .json()
        .await?;

    Ok(llm_response) 

}