use reqwest::Client;
use crate::models::{LLMRequest, LLMResponse};

pub async fn call_llm(
    client: &Client, 
    api_key: &str,
    request: LLMRequest
) -> Result<LLMResponse, reqwest::Error> {

    let response = client
        .post("https://api.groq.com/openai/v1/chat/completions")
        .timeout(std::time::Duration::from_secs(60))
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