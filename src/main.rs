mod models;
mod handlers;
mod client;
mod cache;

use axum::{routing::{get, post, Router}};
use std::net::SocketAddr;
use tokio::net::TcpListener;
use cache::RedisCache;
use reqwest::Client;

#[tokio::main]
async fn main() {

    dotenvy::dotenv().ok();

    let groq_api_key = std::env::var("GROQ_API_KEY")
        .expect("GROQ_API_KEY must be set");

    let redis_url = std::env::var("REDIS_URL")
        .unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());

    // create cache
    let cache = RedisCache::new(&redis_url)
        .await
        .expect("Failed to connect to Redis");

    let http_client = Client::new();
    
    let app = Router::new()
        .route("/health", get(handlers::health_check))
        .route("/v1/chat/completions", post(handlers::proxy_handler))
        .with_state((cache, http_client, groq_api_key)); // share the cache and http client with all the handles
                            // http client is shared to avoid creating a new 
                            // HTTP client for every request

    let addr: SocketAddr = ([0, 0, 0, 0], 3000).into();
    let listener = TcpListener::bind(addr).await
        .expect("Failed to bind to port 3000");
    println!("listening on {}", listener.local_addr()
        .expect("Failed to get local address"));
    axum::serve(listener, app).await
        .expect("Server failed");

}