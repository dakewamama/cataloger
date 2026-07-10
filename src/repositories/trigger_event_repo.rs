use sqlx::SqlitePool;
use crate::models::{NewTriggerEvent, TriggerEvent};

use crate::constants::SUBSCRIPTIONS_PROGRAM_ID;

#[derive(Clone)]
pub struct TriggerEventRepo {
    db: SqlitePool,
}

impl TriggerEventRepo {
    pub fn new(db: SqlitePool) -> Self {
        Self { db }
    }

    pub async fn insert(&self, event: &NewTriggerEvent) -> Result<Option<TriggerEvent>, sqlx::Error> {
        let row = sqlx::query_as::<_, TriggerEvent>(
            r#"
            INSERT INTO trigger_events (
                    signature, program_id, discriminator, raw_data,
                    plan, subscriber, mint, amount, period_start_ts, period_end_ts
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(signature, discriminator, raw_data) DO NOTHING
            RETURNING id, signature, program_id, discriminator, raw_data, created_at,
                      plan, subscriber, mint, amount, period_start_ts, period_end_ts
            "#,
        )
        .bind(&event.signature)
        .bind(&event.program_id)
        .bind(event.discriminator as i64)
        .bind(&event.raw_data)
        .bind(&event.plan)
        .bind(&event.subscriber)
        .bind(&event.mint)
        .bind(event.amount)
        .bind(event.period_start_ts)
        .bind(event.period_end_ts)
        .fetch_optional(&self.db)
        .await?;

        Ok(row)
    }

    pub async fn list(&self) -> Result<Vec<TriggerEvent>, sqlx::Error> {
        let rows = sqlx::query_as::<_, TriggerEvent>(
            r#"
            SELECT id, signature, program_id, discriminator, raw_data, created_at, plan, subscriber, mint, amount, period_start_ts, period_end_ts
            FROM trigger_events
            ORDER BY created_at DESC
            "#,
        )
        .fetch_all(&self.db)
        .await?;

        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::connect;

    async fn setup() -> TriggerEventRepo {
        let pool = connect("sqlite::memory:").await.unwrap();
        TriggerEventRepo::new(pool)
    }

    fn make_event() -> NewTriggerEvent {
        NewTriggerEvent {
            signature: "testsig".to_string(),
            program_id: SUBSCRIPTIONS_PROGRAM_ID.to_string(),
            discriminator: 0,
            raw_data: vec![0u8; 9],
            plan: None,
            subscriber: None,
            mint: None,
            amount: None,
            period_start_ts: None,
            period_end_ts: None,
        }
    }

    #[tokio::test]
    async fn inserts_and_lists_event() {
        let repo = setup().await;
        let event = make_event();

        let inserted = repo.insert(&event).await.unwrap();
        assert!(inserted.is_some());
        assert_eq!(inserted.unwrap().signature, "testsig");

        let all = repo.list().await.unwrap();
        assert_eq!(all.len(), 1);
    }

    #[tokio::test]
    async fn list_returns_empty_when_no_events() {
        let repo = setup().await;
        let all = repo.list().await.unwrap();
        assert_eq!(all.len(), 0);
    }

    #[tokio::test]
    async fn duplicate_insert_is_skipped_not_errored() {
        let repo = setup().await;
        let event = make_event();

        let first = repo.insert(&event).await.unwrap();
        assert!(first.is_some());

        let second = repo.insert(&event).await.unwrap();
        assert!(second.is_none());

        let all = repo.list().await.unwrap();
        assert_eq!(all.len(), 1);
    }
}