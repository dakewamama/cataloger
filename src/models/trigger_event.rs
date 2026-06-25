use sqlx::FromRow;

#[derive(Debug, FromRow)]
pub struct TriggerEvent {
    pub id: i64,
    pub signature: String,
    pub program_id: String,
    pub discriminator: i64,
    pub raw_data: Vec<u8>,
    pub created_at: String,
}

#[derive(Debug)]
pub struct NewTriggerEvent {
    pub signature: String,
    pub program_id: String,
    pub discriminator: u8,
    pub raw_data: Vec<u8>,
}