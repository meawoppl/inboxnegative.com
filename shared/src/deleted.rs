use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, PartialEq, Eq, Clone)]
pub struct UserStats {
    pub total_deleted: u64,
    pub user_deleted: Option<u64>,
}

impl UserStats {
    pub fn new(total_deleted: u64, user_deleted: Option<u64>) -> UserStats {
        UserStats {
            total_deleted,
            user_deleted,
        }
    }
}
