use anyhow::{Context, Result};
use diesel::prelude::*;
use std::collections::HashMap;

use crate::db::models::{EmailStat, SYSTEM_TOTAL_KEY};
use crate::db::{schema::email_stats, DbConnection};
use shared::deleted::UserStats;

/// Get total deleted emails across all users
pub fn get_total_deleted(conn: &mut DbConnection) -> Result<u64> {
    use crate::db::schema::email_stats::dsl::*;

    let total = email_stats
        .filter(email_hash.eq(SYSTEM_TOTAL_KEY))
        .select(deleted_count)
        .first::<i64>(conn)
        .optional()
        .context("Failed to get total deleted count")?;

    Ok(total.unwrap_or(0) as u64)
}

/// Get deleted count for a specific email hash
pub fn get_deleted_for_hash(conn: &mut DbConnection, hash: &str) -> Result<u64> {
    use crate::db::schema::email_stats::dsl::*;

    let count = email_stats
        .filter(email_hash.eq(hash))
        .select(deleted_count)
        .first::<i64>(conn)
        .optional()
        .context("Failed to get deleted count for email")?;

    Ok(count.unwrap_or(0) as u64)
}

/// Increment deleted count for a specific email hash
pub fn increment_deleted(conn: &mut DbConnection, hash: &str) -> Result<()> {
    use crate::db::schema::email_stats::dsl::*;

    // Start a transaction
    conn.transaction(|conn| {
        // Increment/insert the user's deleted count
        diesel::insert_into(email_stats)
            .values(EmailStat::new(hash.to_string(), 1))
            .on_conflict(email_hash)
            .do_update()
            .set(deleted_count.eq(deleted_count + 1))
            .execute(conn)
            .context("Failed to increment user deleted count")?;

        // Increment/insert the system total
        diesel::insert_into(email_stats)
            .values(EmailStat::new(SYSTEM_TOTAL_KEY.to_string(), 1))
            .on_conflict(email_hash)
            .do_update()
            .set(deleted_count.eq(deleted_count + 1))
            .execute(conn)
            .context("Failed to increment total deleted count")?;

        Result::<_, anyhow::Error>::Ok(())
    })?;

    Ok(())
}

/// Get user stats, optionally including the user's deleted count
pub fn get_user_stats(conn: &mut DbConnection, user_email_hash: Option<&str>) -> Result<UserStats> {
    let total = get_total_deleted(conn)?;

    let user_deleted = match user_email_hash {
        Some(hash) => Some(get_deleted_for_hash(conn, hash)?),
        None => None,
    };

    Ok(UserStats::new(total, user_deleted))
}

/// Import stats from the old format to the database
pub fn import_stats(
    conn: &mut DbConnection,
    hash_to_deleted: &HashMap<String, u64>,
    total_deleted: u64,
) -> Result<()> {
    // Start a transaction
    conn.transaction(|conn| {
        // Import each hash and its count
        for (hash, count) in hash_to_deleted {
            diesel::insert_into(email_stats::table)
                .values(EmailStat::new(hash.to_string(), *count as i64))
                .on_conflict(email_stats::email_hash)
                .do_update()
                .set(email_stats::deleted_count.eq(*count as i64))
                .execute(conn)
                .with_context(|| format!("Failed to import stats for hash {}", hash))?;
        }

        // Set the system total
        diesel::insert_into(email_stats::table)
            .values(EmailStat::new(
                SYSTEM_TOTAL_KEY.to_string(),
                total_deleted as i64,
            ))
            .on_conflict(email_stats::email_hash)
            .do_update()
            .set(email_stats::deleted_count.eq(total_deleted as i64))
            .execute(conn)
            .context("Failed to import total deleted count")?;

        Result::<_, anyhow::Error>::Ok(())
    })?;

    Ok(())
}
