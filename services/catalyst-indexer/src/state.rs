use crate::events::CatalystEvent;
use crate::repositories::TriggerEventRepo;
use axum::extract::FromRef;
use sqlx::SqlitePool;
use tokio::sync::broadcast;

pub const BROADCAST_CAPACITY: usize = 1024;

#[derive(Clone, FromRef)]
pub struct AppState {
    pub database: SqlitePool,
    pub trigger_event_repo: TriggerEventRepo,
    pub event_tx: broadcast::Sender<CatalystEvent>,
}

impl AppState {
    pub async fn new(database_url: &str) -> Result<Self, sqlx::Error> {
        let database = crate::database::connect(database_url).await?;
        let trigger_event_repo = TriggerEventRepo::new(database.clone());
        let (event_tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        Ok(Self {
            database,
            trigger_event_repo,
            event_tx,
        })
    }
}
