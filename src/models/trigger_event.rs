use sqlx::FromRow;

#[derive(Debug, FromRow)]
pub struct TriggerEvent {
    pub id: i64,
    pub signature: String,
    pub program_id: String,
    pub discriminator: i64,
    pub raw_data: Vec<u8>,
    pub created_at: String,
    pub plan: Option<String>,
    pub subscriber: Option<String>,
    pub mint: Option<String>,
    pub amount: Option<i64>,
    pub period_start_ts: Option<i64>,
    pub period_end_ts: Option<i64>,
    pub delegation: Option<String>,
}

#[derive(Debug)]
pub struct NewTriggerEvent {
    pub signature: String,
    pub program_id: String,
    pub discriminator: u8,
    pub raw_data: Vec<u8>,
    pub plan: Option<String>,
    pub subscriber: Option<String>,
    pub mint: Option<String>,
    pub amount: Option<i64>,
    pub period_start_ts: Option<i64>,
    pub period_end_ts: Option<i64>,
    pub delegation: Option<String>,
}
