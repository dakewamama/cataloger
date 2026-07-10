use axum::{Json, Router, extract::State, http::StatusCode, middleware as axum_middleware, routing::{get, post}};
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

mod events;
mod decoder;

mod middleware;


#[tokio::main]
async fn main(){
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();

    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "sqlite://catalyst.db".to_string());
    let state: AppState = AppState::new(&database_url).await.unwrap();

    let app = Router::new()
        .route("/health", get(health))
        .route("/webhook/helius",
         post(webhook).layer(axum_middleware::from_fn(middleware::verify_helius_signature)))
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
) -> StatusCode {
    let raw_events = extract_events(&payload);

    for event in &raw_events {
        let raw = &event.bytes;
        let discriminator = raw[8];
        let decoded = decoder::decode_event(raw);

        let (plan, subscriber, mint, amount, period_start_ts, period_end_ts) = match &decoded {
            Some(events::CatalystEvent::SubscriptionCreated(e)) =>
                (Some(e.plan.clone()), Some(e.subscriber.clone()), Some(e.mint.clone()), None, None, None),
            Some(events::CatalystEvent::SubscriptionCancelled(e)) =>
                (Some(e.plan.clone()), Some(e.subscriber.clone()), None, None, None, Some(e.expires_at_ts)),
            Some(events::CatalystEvent::SubscriptionTransfer(e)) =>
                (Some(e.plan.clone()), Some(e.delegator.clone()), Some(e.mint.clone()), Some(e.amount as i64), Some(e.period_start_ts), Some(e.period_end_ts)),
            Some(events::CatalystEvent::FixedTransfer(e)) =>
                (None, Some(e.delegator.clone()), Some(e.mint.clone()), Some(e.amount as i64), None, None),
            Some(events::CatalystEvent::RecurringTransfer(e)) =>
                (None, Some(e.delegator.clone()), Some(e.mint.clone()), Some(e.amount as i64), Some(e.period_start_ts), Some(e.period_end_ts)),
            Some(events::CatalystEvent::SubscriptionResumed(e)) =>
                (Some(e.plan.clone()), Some(e.subscriber.clone()), None, None, None, None),
            None => (None, None, None, None, None, None),
        };

        let new_event = NewTriggerEvent {
            signature: event.signature.clone(),
            program_id: SUBSCRIPTIONS_PROGRAM_ID.to_string(),
            discriminator,
            raw_data: raw.clone(),
            plan,
            subscriber,
            mint,
            amount,
            period_start_ts,
            period_end_ts,
        };

        match state.trigger_event_repo.insert(&new_event).await {
            Ok(Some(e)) => {
                tracing::info!(id = e.id, discriminator, "event persisted");
                if let Some(ev) = decoded {
                    let _ = state.event_tx.send(ev);
                }
            }
            Ok(None) => {
                tracing::info!(discriminator, "duplicate event skipped");
            }
            Err(err) => {
                tracing::error!(error = %err, "failed to persist event");
                return StatusCode::INTERNAL_SERVER_ERROR;
            }
        }
    }

    tracing::info!(count = raw_events.len(), "webhook processed");
    StatusCode::OK
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
        let body = r#"[{
            "slot": 123,
            "meta": { "innerInstructions": [] },
            "transaction": {
                "message": { "accountKeys": [] },
                "signatures": ["sig123"]
            }
        }]"#;
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