use axum::{Router, routing::{get, post}, Json, extract::State};
use tokio::net::TcpListener;


mod constants;
mod types;
mod extractor;

use types::WebhookPayload;
use extractor::extract_events;

mod database;
mod models;
mod repositories;
mod state;

use state::AppState;
use models::NewTriggerEvent;
use constants::SUBSCRIPTIONS_PROGRAM_ID;

#[tokio::main]
async fn main(){
    tracing_subscriber::fmt::init();

    let state: AppState = AppState::new("sqlite://catalyst.db").await.unwrap();

    let app = Router::new()
        .route("/health", get(health))
        .route("/webhook/helius", post(webhook))
        .with_state(state);

    let listener = TcpListener::bind("0.0.0.0:3000").await.unwrap();
    tracing::info!("catalyst-indexer listening on port 3000");
    axum::serve(listener, app).await.unwrap();
}

async fn health() -> &'static str {
    "ok"
}

async fn webhook(
    State(state): State<AppState>,
    Json(payload): Json<WebhookPayload>,
) -> &'static str {
    let events = extract_events(&payload);

    for event in &events {
        let discriminator = event[8];
        let new_event = NewTriggerEvent {
            signature: payload.transactions.first()
                .map(|t| t.signature.clone())
                .unwrap_or_default(),
            program_id: SUBSCRIPTIONS_PROGRAM_ID.to_string(),
            discriminator,
            raw_data: event.clone(),
        };

        match state.trigger_event_repo.insert(&new_event).await {
            Ok(e) => tracing::info!(id = e.id, discriminator, "event persisted"),
            Err(err) => tracing::error!(error = %err, "failed to persist event"),
        }
    }
    tracing::info!(count = events.len(), "webhook processed");
    "ok"
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::util::ServiceExt;

    async fn build_app() -> Router {
        let state: AppState = AppState::new("sqlite::memory:").await.unwrap();
        Router::new()
            .route("/health", get(health))
            .route("/webhook/helius", post(webhook))
            .with_state(state)
    }

    #[tokio::test]
    async fn health_returns_ok() {
        let app = build_app().await;
        let response = app
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn webhook_with_no_events_returns_ok() {
        let app = build_app().await;
        let body = r#"{"type":"TRANSFER","transactions":[{"signature":"sig123","meta":{"innerInstructions":[]}}]}"#;
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/webhook/helius")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}