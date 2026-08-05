use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};

use shared::email::Attachment;

#[derive(Clone)]
struct CachedAttachment {
    attachment: Attachment,
    expires_at: SystemTime,
}

pub struct AttachmentStore {
    cache: Arc<RwLock<HashMap<String, CachedAttachment>>>,
    ttl: Duration,
}

impl AttachmentStore {
    pub fn new(ttl_seconds: u64) -> Self {
        AttachmentStore {
            cache: Arc::new(RwLock::new(HashMap::new())),
            ttl: Duration::from_secs(ttl_seconds),
        }
    }

    pub fn store(&self, attachment: Attachment) {
        let expires_at = SystemTime::now() + self.ttl;
        let cached = CachedAttachment {
            attachment: attachment.clone(),
            expires_at,
        };

        if let Ok(mut cache) = self.cache.write() {
            cache.insert(attachment.attachment_id.clone(), cached);
        }
    }

    pub fn get(&self, attachment_id: &str) -> Option<Attachment> {
        if let Ok(mut cache) = self.cache.write() {
            // Clean up expired entries while we have the write lock
            self.cleanup_expired(&mut cache);

            cache.get(attachment_id).and_then(|cached| {
                if cached.expires_at > SystemTime::now() {
                    Some(cached.attachment.clone())
                } else {
                    None
                }
            })
        } else {
            None
        }
    }

    fn cleanup_expired(&self, cache: &mut HashMap<String, CachedAttachment>) {
        let now = SystemTime::now();
        cache.retain(|_, cached| cached.expires_at > now);
    }

    pub fn cleanup(&self) {
        if let Ok(mut cache) = self.cache.write() {
            self.cleanup_expired(&mut cache);
        }
    }

    pub fn size(&self) -> usize {
        if let Ok(cache) = self.cache.read() {
            cache.len()
        } else {
            0
        }
    }
}
