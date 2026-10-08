use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use std::str::FromStr;

pub async fn connect(database_url: &str) -> Result<SqlitePool, sqlx::Error> {
    let options = SqliteConnectOptions::from_str(database_url)?
        .busy_timeout(std::time::Duration::from_secs(5))
        .pragma("journal_mode", "WAL")
        .pragma("synchronous", "NORMAL");

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await?;

    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn connects_and_migrates() {
        let pool = connect("sqlite::memory:").await.unwrap();
        assert!(!pool.is_closed());
    }
}
