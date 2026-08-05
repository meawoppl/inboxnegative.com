use serde::Serialize;

#[derive(Serialize)]
pub struct DeletedStats {
    total_deleted: u64,
    user_deleted: Option<u64>,
}

impl DeletedStats {
    pub fn new(total_deleted: u64, user_deleted: Option<u64>) -> DeletedStats {
        DeletedStats {
            total_deleted,
            user_deleted,
        }
    }
}
