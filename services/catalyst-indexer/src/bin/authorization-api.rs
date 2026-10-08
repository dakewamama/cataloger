use cataloger::{Catalog, ProgramVersion};
use catalyst_indexer::{Journal, Snapshot, router};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let database_url = std::env::var("DATABASE_URL")?;
    let journal = Journal::open(&database_url).await?;
    match args.as_slice() {
        [command, catalog_path, snapshot_path] if command == "import" => {
            let versions: Vec<ProgramVersion> =
                serde_json::from_slice(&std::fs::read(catalog_path)?)?;
            let snapshot: Snapshot = serde_json::from_slice(&std::fs::read(snapshot_path)?)?;
            let record = journal.ingest(snapshot, &Catalog::new(versions)?).await?;
            println!("{}", serde_json::to_string(&record)?);
        }
        [command, id] if command == "replay" => {
            println!("{}", serde_json::to_string(&journal.replay(id).await?)?);
        }
        [command, address] if command == "serve" => {
            let listener = tokio::net::TcpListener::bind(address).await?;
            axum::serve(listener, router(journal))
                .with_graceful_shutdown(async {
                    let _ = tokio::signal::ctrl_c().await;
                })
                .await?;
        }
        _ => {
            return Err(
                "usage: authorization-api import CATALOG SNAPSHOT | replay ID | serve ADDRESS"
                    .into(),
            );
        }
    }
    Ok(())
}
