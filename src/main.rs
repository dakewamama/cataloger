use axum::{Router, routing::get};
use tokio::net::TcpListener;

#[tokio::main]
async fn main(){
    let app = Router::new().route("/health", get(health));

    let listener = TcpListener::bind("0.0.0.0:3000").await.unwrap();
    println!("catalyst-indexer listening on port 3000");
    axum::serve(listener, app).await.unwrap();
}

async fn health() -> &'static str {
    "ok"
}