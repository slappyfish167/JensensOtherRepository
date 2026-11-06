//! Tabletop Combat Engine server (Pathfinder 2e, Cyberpunk RED, ...).
//!
//! Environment:
//! * `PF2E_DB`         SQLite file (default `pf2e.sqlite`; `:memory:` for ephemeral)
//! * `PF2E_ADDR`       listen address (default `127.0.0.1:8080`)
//! * `PF2E_STATIC_DIR` serve a built frontend from this directory (default `frontend/dist` if present)
//! * `PF2E_NO_SEED`    set to skip inserting sample content into an empty database

mod app;
mod error;
mod routes;
mod seed;

use app::App;
use pf2e_core::builder::content::Content;
use pf2e_store::SqliteStore;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db = std::env::var("PF2E_DB").unwrap_or_else(|_| "pf2e.sqlite".into());
    let addr = std::env::var("PF2E_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".into());
    let store = if db == ":memory:" { SqliteStore::in_memory()? } else { SqliteStore::open(&db)? };
    let content = Content::bundled()?;
    let app = Arc::new(App::new(store, content));
    if std::env::var("PF2E_NO_SEED").is_err() {
        seed::seed_if_empty(&app)?;
    }

    let mut router = routes::router(app.clone()).layer(CorsLayer::permissive());
    let static_dir = std::env::var("PF2E_STATIC_DIR").unwrap_or_else(|_| "frontend/dist".into());
    if std::path::Path::new(&static_dir).join("index.html").exists() {
        let index = format!("{static_dir}/index.html");
        router = router.fallback_service(ServeDir::new(&static_dir).not_found_service(ServeFile::new(index)));
        println!("serving frontend from {static_dir}");
    }
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("Tabletop combat engine listening on http://{addr} (db: {db})");
    axum::serve(listener, router).await?;
    Ok(())
}

#[cfg(test)]
mod tests;
