mod models;
mod handlers;
mod client;

use axum::{routing::{get, post, Router}};
use std::net::SocketAddr;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() {

    dotenvy::dotenv().ok();

    std::env::var("GROQ_API_KEY").expect("GROQ_API_KEY must be set");

    let app = Router::new()
        .route("/health", get(handlers::health_check))
        .route("/v1/chat/completions", post(handlers::proxy_handler));

    let addr: SocketAddr = ([0, 0, 0, 0], 3000).into();
    let listener = TcpListener::bind(addr).await.unwrap();
    println!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();

}