// ============================================================================
// Simple Server Example
// ============================================================================
//
// This is a simplified implementation for demonstration purposes.
// It showcases basic HTTP server setup using Axum with a health check endpoint.
//
// ============================================================================

use axum::{routing::{get, Router}};
use std::net::SocketAddr;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() {

    // build our application with routes
    let app = Router::new().route("/health", get(health_check));

    // run it
    let addr: SocketAddr = ([0, 0, 0, 0], 3000).into();
    let listener = TcpListener::bind(addr).await.unwrap();
    println!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();

}

async fn health_check() -> &'static str {

    "OK"

}