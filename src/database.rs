use sqlx::SqlitePool;

pub async fn connect(database_url: &str) -> Result<SqlitePool, sqlx::Error> {
    let pool = SqlitePool::connect(database_url).await?;
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