use diesel::prelude::*;

use crate::db::schema::email_stats;

// Special key to track total deleted emails across all users
pub const SYSTEM_TOTAL_KEY: &str = "SYSTEM_TOTAL";

#[derive(Queryable, Insertable, AsChangeset)]
#[diesel(table_name = email_stats)]
pub struct EmailStat {
    pub email_hash: String,
    pub deleted_count: i64,
}

impl EmailStat {
    pub fn new(email_hash: String, deleted_count: i64) -> Self {
        Self {
            email_hash,
            deleted_count,
        }
    }
}
