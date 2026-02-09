use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct Message {
    pub role: String,
    pub content: String
}

#[derive(Debug, Deserialize, Serialize)]
pub struct LLMRequest {
    pub messages: Vec<Message>,
    pub model: String,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>
}

#[derive(Debug, Deserialize, Serialize)]
pub struct LLMResponse {
    pub choices: Vec<Choice>
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Choice {
    pub message: Message
}