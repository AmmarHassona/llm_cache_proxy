use sha2::{Sha256, Digest};
use crate::models::LLMRequest;
use redis::aio::ConnectionManager;
use redis::AsyncCommands;

const CACHE_TTL_SECONDS: u64 = 86400;

pub fn generate_cache_key(request: &LLMRequest) -> String {
    
    // Request contains model, temperature, max_tokens, messages
    let normalized_messages: Vec<String> = request.messages
        .iter() // iterate through each message
        .map(|message| {
            // for each message create a "role:content" string 
            let normalized_content = message.content.trim().to_lowercase();
            format!("{}:{}", message.role.to_lowercase(), normalized_content)
        })
        .collect(); // collect into a vector of strings

    let combined_messages = normalized_messages.join("|");

    let model = request.model.trim().to_lowercase();

    // convert temperature and max_tokens to string while handling case that they might be empty
    let temp_str = match request.temperature {
        Some(t) => format!("temp:{}", t),
        None => "temp:none".to_string()
    };

    let tokens_str = match request.max_tokens {
        Some(t) => format!("tokens:{}", t),
        None => "tokens:none".to_string()
    };

    // concatenate all strings into one hash string
    let to_hash = format!("{}|model:{}|{}|{}",
        combined_messages,
        model,
        temp_str,
        tokens_str
    );

    // initialize a new sha256 variable
    let mut hasher = Sha256::new();
    hasher.update(to_hash.as_bytes());
    let hash_bytes = hasher.finalize();

    // convert bytes to hash string
    let hash_hex = format!("{:x}", hash_bytes);

    // return formatted cache key
    format!("cache:exact:{}:{}", hash_hex, model)

}

#[derive(Clone)]
pub struct RedisCache {
    conn_manager: ConnectionManager
}

impl RedisCache {

    pub async fn new(redis_url: &str) -> Result<Self, redis::RedisError> {

        let client = redis::Client::open(redis_url)?;
        let conn_manager = ConnectionManager::new(client).await?;
        Ok(RedisCache { conn_manager })

    }

    pub async fn get(&self, key: &str) -> Result<Option<String>, redis::RedisError> {
        
        let mut connection = self.conn_manager.clone();
        connection.get(key).await

    }

    pub async fn set(&self, key: &str, value: &str) -> Result<(), redis::RedisError> {

        let mut connection = self.conn_manager.clone();
        connection.set_ex(key, value, CACHE_TTL_SECONDS).await

    }

}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::models::{LLMRequest, Message};

    #[test]
    fn test_same_prompts_same_key() {

        let req1 = LLMRequest {
            messages: vec![
                    Message {
                        role: "user".to_string(),
                        content: "What is Rust?".to_string()
                    }
            ],
            model: "gpt-4".to_string(),
            temperature: Some(0.7),
            max_tokens: None
        };

        let req2 = LLMRequest {
            messages: vec![
                Message {
                    role: "user".to_string(),
                    content: "   what is Rust?     ".to_string()
                }
            ],
            model: "gpt-4".to_string(),
            temperature: Some(0.7),
            max_tokens: None
        };

        let key1 = generate_cache_key(&req1);
        let key2 = generate_cache_key(&req2);

        assert_eq!(key1, key2, "Normalized prompts should generate same key");

    }

}