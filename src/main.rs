use axum::{Router, routing::{get, post}, Json};
use tokio::net::TcpListener;


mod constants;
mod types;
mod extractor;

use types::WebhookPayload;
use extractor::extract_events;

mod database;
mod models;
mod repositories;

#[tokio::main]
async fn main(){
    tracing_subscriber::fmt::init();

    let app = Router::new()
        .route("/health", get(health))
        .route("/webhook/helius", post(webhook));

    let listener = TcpListener::bind("0.0.0.0:3000").await.unwrap();
    tracing::info!("catalyst-indexer listening on port 3000");
    axum::serve(listener, app).await.unwrap();
}

async fn health() -> &'static str {
    "ok"
}

async fn webhook(Json(payload): Json<WebhookPayload>) -> &'static str {
    let events = extract_events(&payload);
    for event in &events {
        tracing::info!(discriminator = event[8], "catalyst event recieved");
    }
    tracing::info!(count = events.len(), "webhook processed");
    "ok"
}