use std::{
    collections::HashMap,
    error::Error,
    fs::File,
    io::{BufReader, BufWriter},
    path::Path,
    sync::Arc,
};

use anyhow::{Context, Result};
use log::info;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use shared::deleted::UserStats;

use crate::db::{
    get_deleted_for_hash, get_total_deleted, get_user_stats, import_stats, increment_deleted,
    DbPool,
};
use crate::salt;

fn hash_with_salt(salt: &[u8], email: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(salt);
    hasher.update(email);
    let hash = hasher.finalize();
    format!("{:x}", hash)
}

/// Derives the `email_stats.email_hash` primary key. The salt comes from the
/// environment and must never change -- see `salt`.
pub fn email_to_hashed(email: &str) -> String {
    hash_with_salt(salt::deleted_salt().as_bytes(), email)
}

#[derive(Serialize, Deserialize)]
pub struct EmailStats {
    // For migrating from the old format
    #[serde(skip)]
    hash_to_deleted: HashMap<String, u64>,
    #[serde(skip)]
    total_deleted: u64,

    // The database connection pool
    #[serde(skip)]
    db_pool: Option<Arc<DbPool>>,

    // Flag indicating if we're currently migrating
    #[serde(skip)]
    migrating: bool,
}

impl EmailStats {
    pub fn new() -> EmailStats {
        EmailStats {
            hash_to_deleted: HashMap::new(),
            total_deleted: 0,
            db_pool: None,
            migrating: false,
        }
    }

    #[allow(dead_code)]
    pub fn with_db_pool(db_pool: Arc<DbPool>) -> EmailStats {
        EmailStats {
            hash_to_deleted: HashMap::new(),
            total_deleted: 0,
            db_pool: Some(db_pool),
            migrating: false,
        }
    }

    pub fn set_db_pool(&mut self, db_pool: Arc<DbPool>) {
        self.db_pool = Some(db_pool);
    }

    // For testing purposes only
    #[cfg(test)]
    pub fn set_migrating(&mut self, migrating: bool) {
        self.migrating = migrating;
    }

    pub fn add_for_email_address(&mut self, email_address: &str) -> Result<()> {
        let hash = email_to_hashed(email_address);
        self.inc(hash)
    }

    pub fn get_deleted_count_for_address(&self, email_address: &str) -> Result<u64> {
        let hash = email_to_hashed(email_address);
        self.get_count(&hash)
    }

    pub fn get_user_stats(&self, email_address: Option<&str>) -> Result<UserStats> {
        if let Some(pool) = &self.db_pool {
            let email_hash = email_address.map(email_to_hashed);
            let mut conn = pool.get().context("Failed to get DB connection")?;
            get_user_stats(&mut conn, email_hash.as_deref())
        } else {
            // Fallback to in-memory stats (shouldn't happen in normal operation)
            let total_deleted = self.total_deleted;
            let user_deleted =
                email_address.map(|email| self.get_deleted_count_for_address(email).unwrap_or(0));
            Ok(UserStats::new(total_deleted, user_deleted))
        }
    }

    pub fn inc(&mut self, hash: String) -> Result<()> {
        if let Some(pool) = &self.db_pool {
            // Use the database if available
            let mut conn = pool.get().context("Failed to get DB connection")?;
            increment_deleted(&mut conn, &hash)?;
        } else if self.migrating {
            // Only update in-memory if we're migrating
            let count = self.hash_to_deleted.entry(hash).or_insert(0);
            *count += 1;
            self.total_deleted += 1;
        } else {
            return Err(anyhow::anyhow!("No database pool available"));
        }

        Ok(())
    }

    pub fn get_count(&self, hash: &str) -> Result<u64> {
        if let Some(pool) = &self.db_pool {
            // Use the database if available
            let mut conn = pool.get().context("Failed to get DB connection")?;
            get_deleted_for_hash(&mut conn, hash)
        } else {
            // Fallback to in-memory stats
            Ok(self.hash_to_deleted.get(hash).copied().unwrap_or(0))
        }
    }

    pub fn load(path: &Path) -> Result<EmailStats, Box<dyn Error>> {
        // Try to open the file
        match File::open(path) {
            Ok(file) => {
                let reader = BufReader::new(file);
                match serde_json::from_reader(reader) {
                    Ok::<HashMap<String, u64>, _>(hash_map) => {
                        // Calculate total from the hash map
                        let total = hash_map.values().sum();
                        Ok(EmailStats {
                            hash_to_deleted: hash_map,
                            total_deleted: total,
                            db_pool: None,
                            migrating: true, // Mark as migrating to retain data
                        })
                    }
                    Err(_) => {
                        // Try the old format
                        let file = File::open(path)?;
                        let reader = BufReader::new(file);
                        let stats: EmailStats = serde_json::from_reader(reader)?;
                        Ok(EmailStats {
                            hash_to_deleted: stats.hash_to_deleted,
                            total_deleted: stats.total_deleted,
                            db_pool: None,
                            migrating: true, // Mark as migrating to retain data
                        })
                    }
                }
            }
            Err(_) => {
                // File doesn't exist, return empty stats
                Ok(EmailStats::new())
            }
        }
    }

    #[allow(dead_code)]
    pub fn save(&self, path: &Path) -> Result<(), Box<dyn Error>> {
        // If we're using the database, we don't need to save anymore
        if self.db_pool.is_some() && !self.migrating {
            return Ok(());
        }

        let file = File::create(path)?;
        let writer = BufWriter::new(file);
        serde_json::to_writer(writer, &self.hash_to_deleted)?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn get_total_deleted(&self) -> Result<u64> {
        if let Some(pool) = &self.db_pool {
            // Use the database if available
            let mut conn = pool.get().context("Failed to get DB connection")?;
            get_total_deleted(&mut conn)
        } else {
            // Fallback to in-memory stats
            Ok(self.total_deleted)
        }
    }

    /// Migrate data from file-based storage to the database
    pub fn migrate_to_db(&mut self) -> Result<()> {
        if let Some(pool) = &self.db_pool {
            if self.migrating && !self.hash_to_deleted.is_empty() {
                let mut conn = pool.get().context("Failed to get DB connection")?;
                import_stats(&mut conn, &self.hash_to_deleted, self.total_deleted)?;

                // Clear the in-memory data and mark as not migrating
                self.hash_to_deleted.clear();
                self.total_deleted = 0;
                self.migrating = false;

                info!("Successfully migrated stats to database");
            }
            Ok(())
        } else {
            Err(anyhow::anyhow!("No database pool available for migration"))
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::NamedTempFile;

    use super::*;

    /// Pins the hash construction to SHA256(salt || email). Note this is the
    /// opposite order from `addresses::hash_email`, which is SHA256(email ||
    /// salt). Swapping them would silently orphan all 1400-odd stats rows, so
    /// the ordering is pinned explicitly rather than left to inspection.
    #[test]
    fn test_hash_with_salt_is_salt_then_email() {
        assert_eq!(
            hash_with_salt(b"TestDeletedSalt", "test@example.com"),
            "b26235973befd822baf8d2725561aa52157f9070900caffb9baf5aa055b1d9a0"
        );
    }

    #[test]
    fn test_email_to_hashed_uses_configured_salt() {
        // Under test, `salt` supplies the fixed test-only salt.
        assert_eq!(
            email_to_hashed("test@example.com"),
            hash_with_salt(b"TestDeletedSalt", "test@example.com")
        );
    }

    #[test]
    fn test_email_stats() {
        let mut stats = EmailStats::new();
        // In test mode with no DB, this should update the hash_to_deleted map
        stats.set_migrating(true);

        stats.inc("hash1".to_string()).unwrap();
        stats.inc("hash2".to_string()).unwrap();
        stats.inc("hash1".to_string()).unwrap();
        stats.inc("hash1".to_string()).unwrap();
        stats.inc("hash2".to_string()).unwrap();
        stats.inc("hash2".to_string()).unwrap();
        stats.inc("hash2".to_string()).unwrap();

        assert_eq!(stats.hash_to_deleted.get("hash1"), Some(&3));
        assert_eq!(stats.hash_to_deleted.get("hash2"), Some(&4));
        assert_eq!(stats.hash_to_deleted.get("hash3"), None);
    }

    #[test]
    fn test_email_stats_save_load() {
        let mut stats = EmailStats::new();
        stats.set_migrating(true);

        stats.inc("hash1".to_string()).unwrap();
        stats.inc("hash2".to_string()).unwrap();
        stats.inc("hash1".to_string()).unwrap();
        stats.inc("hash1".to_string()).unwrap();
        stats.inc("hash2".to_string()).unwrap();
        stats.inc("hash2".to_string()).unwrap();
        stats.inc("hash2".to_string()).unwrap();

        let file = NamedTempFile::new().unwrap();
        let path = file.path();

        stats.save(path).unwrap();

        let loaded_stats = EmailStats::load(path).unwrap();
        assert_eq!(loaded_stats.hash_to_deleted.get("hash1"), Some(&3));
        assert_eq!(loaded_stats.hash_to_deleted.get("hash2"), Some(&4));
        assert_eq!(loaded_stats.hash_to_deleted.get("hash3"), None);
    }
}
