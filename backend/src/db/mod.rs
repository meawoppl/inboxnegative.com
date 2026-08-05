pub mod models;
pub mod operations;
pub mod schema;

use anyhow::{Context, Result};
use diesel::pg::PgConnection;
use diesel::r2d2::{self, ConnectionManager};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use std::env;
use std::sync::Arc;

/// Migrations embedded into the binary at compile time, applied on startup.
pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

// Re-export common operations
pub use operations::*;

// Type alias for the database connection pool
pub type DbPool = r2d2::Pool<ConnectionManager<PgConnection>>;
// Type alias for a connection from the pool
pub type DbConnection = r2d2::PooledConnection<ConnectionManager<PgConnection>>;

/// Create a new database connection pool
pub fn create_pool() -> Result<Arc<DbPool>> {
    // Load environment variables from .env file if present. `.ok()` is load-bearing:
    // production runs on real environment variables with no .env file, so a missing
    // file must stay non-fatal.
    dotenvy::dotenv().ok();

    // Get the database URL from environment variables
    let database_url = env::var("DATABASE_URL").context("DATABASE_URL must be set")?;

    // Create a connection manager for PostgreSQL
    let manager = ConnectionManager::<PgConnection>::new(database_url);

    // Create the connection pool with appropriate options for cloud DB platforms like Neon
    let pool = r2d2::Pool::builder()
        .max_size(5) // Limit connections to avoid hitting cloud DB limits
        .build(manager)
        .context("Failed to create database connection pool")?;

    // Create the initial database schema using a connection from the pool
    run_migrations(&pool)?;

    Ok(Arc::new(pool))
}

/// Run embedded migrations to ensure the database schema is up to date.
fn run_migrations(pool: &DbPool) -> Result<()> {
    let mut conn = pool.get().context("Failed to get connection from pool")?;

    let applied = conn
        .run_pending_migrations(MIGRATIONS)
        .map_err(|e| anyhow::anyhow!("Failed to run migrations: {}", e))?;

    if applied.is_empty() {
        log::debug!("No pending migrations");
    } else {
        for m in &applied {
            log::info!("Applied migration: {}", m);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    /// Every migration directory must follow the Diesel naming convention so
    /// `embed_migrations!` and `scripts/check-migration-names.sh` agree:
    ///   - 00000000000000_<snake_case>            (initial)
    ///   - YYYY-MM-DD-HHMMSS_<snake_case>         (timestamped)
    #[test]
    fn migration_dirs_follow_naming_convention() {
        let initial = regex_lite_initial();
        let migrations_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        let mut checked = 0;
        for entry in std::fs::read_dir(&migrations_dir).expect("migrations dir exists") {
            let entry = entry.unwrap();
            if !entry.file_type().unwrap().is_dir() {
                continue;
            }
            let name = entry.file_name().into_string().unwrap();
            assert!(
                initial(&name) || timestamped(&name),
                "migration dir `{name}` does not match the naming convention"
            );
            checked += 1;
        }
        assert!(checked > 0, "expected at least one migration directory");
    }

    // Minimal validators (kept dependency-free).
    fn regex_lite_initial() -> impl Fn(&str) -> bool {
        |name: &str| {
            name.strip_prefix("00000000000000_")
                .is_some_and(is_snake_case)
        }
    }

    fn timestamped(name: &str) -> bool {
        // YYYY-MM-DD-HHMMSS_<snake>
        let Some((ts, rest)) = name.split_once('_') else {
            return false;
        };
        let parts: Vec<&str> = ts.split('-').collect();
        let ok_shape = matches!(parts.as_slice(), [y, mo, d, hms]
            if y.len() == 4 && mo.len() == 2 && d.len() == 2 && hms.len() == 6
                && [y, mo, d, hms].iter().all(|p| p.chars().all(|c| c.is_ascii_digit())));
        ok_shape && is_snake_case(rest)
    }

    fn is_snake_case(s: &str) -> bool {
        !s.is_empty()
            && s.chars().next().unwrap().is_ascii_lowercase()
            && s.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    }
}
