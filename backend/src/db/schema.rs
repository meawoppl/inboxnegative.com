diesel::table! {
    email_stats (email_hash) {
        email_hash -> Text,
        deleted_count -> BigInt,
    }
}
