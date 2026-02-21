use sha2::{Sha256, Digest};
use crate::models::LLMRequest;
use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use reqwest::Client;
use serde_json::{Value, json};
use qdrant_client::Qdrant;
use qdrant_client::qdrant::{
    CreateCollectionBuilder, Distance, VectorParamsBuilder,
    SearchPointsBuilder, PointStruct, UpsertPointsBuilder
};
use qdrant_client::qdrant::value::Kind;
use uuid::Uuid;

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

#[derive(Clone)]
pub struct QdrantCache {
    client: Qdrant,
    collection_name: String
}

impl QdrantCache {

    pub async fn new(qdrant_url: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {

        // connect to qdrant
        let client = Qdrant::from_url(qdrant_url).build()?;
        let collection_name = "llm_cache".to_string();

        // create collection if it doesn't exist (ignore error if it does)
        let _ = client.create_collection(CreateCollectionBuilder::new(&collection_name)
            .vectors_config(VectorParamsBuilder::new(384, Distance::Cosine)))
            .await;

        Ok(QdrantCache {
            client,
            collection_name
        })

    }

    pub async fn store(
        &self,
        cache_key: &str,
        embedding: Vec<f32>,
        cached_response: &str
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {

        let point = PointStruct::new(
            Uuid::new_v4().to_string(),
            embedding,
            [
                ("cache_key", cache_key.into()),
                ("response", cached_response.into())
            ]
        );

        self.client
            .upsert_points(
                UpsertPointsBuilder::new(&self.collection_name, vec![point])
            )
            .await?;

        Ok(())

    }

    pub async fn search_similar(
        &self,
        embedding: Vec<f32>,
        similarity_threshold: f32
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {

        let search_result = self.client.search_points(
            SearchPointsBuilder::new(&self.collection_name, embedding, 1)
            .with_payload(true)
            .score_threshold(similarity_threshold)
        ).await?;

        if let Some(point) = search_result.result.first() {
            if let Some(response_value) = point.payload.get("response") {
                if let Some(kind) = &response_value.kind {
                    if let Kind::StringValue(s) = kind {
                        return Ok(Some(s.clone()));
                    }
                }
            }
        }

        Ok(None) // no match found

    }

}

pub async fn get_embedding(
    http_client: &Client,
    embedding_url: &str,
    text: &str
) -> Result<Vec<f32>, Box<dyn std::error::Error + Send + Sync>> {

    let response = http_client
        .post(embedding_url)
        .json(&json!({"text": text}))
        .send()
        .await?;

    let result: Value = response.json().await?;

    let embedding: Vec<f32> = result["embedding"]
        .as_array()
        .ok_or("No embedding in response")?
        .iter()
        .filter_map(|v| v.as_f64().map(|f| f as f32))
        .collect();

    Ok(embedding)

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

    #[tokio::test]
    async fn test_get_embedding() {
        let client = Client::new();
        let embedding = get_embedding(&client, "http://127.0.0.1:8001/embed", "What is Rust?")
            .await
            .expect("Failed to get embedding");

        assert_eq!(embedding.len(), 384, "Embedding should have 384 dimensions");
        println!("First 5 values: {:?}", &embedding[0..5]);
    }

    #[tokio::test]
    async fn test_qdrant_store_and_search() {
        let qdrant = QdrantCache::new("http://127.0.0.1:6334").await
            .expect("Failed to connect to Qdrant");
        
        let client = Client::new();
        
        // Get embedding for "What is Rust?"
        let embedding1 = get_embedding(&client, "http://127.0.0.1:8001/embed", "What is Rust?")
            .await
            .expect("Failed to get embedding");
        
        // Store it with a fake response
        qdrant.store(
            "test_key_1",
            embedding1.clone(),
            "Rust is a programming language"
        ).await.expect("Failed to store");
        
        // Search with same embedding (should find exact match)
        let result = qdrant.search_similar(embedding1, 0.99)
            .await
            .expect("Search failed");
        
        assert!(result.is_some(), "Should find the stored embedding");
        assert_eq!(result.unwrap(), "Rust is a programming language");
        
        println!("✅ Qdrant store and search working!");
    }

}