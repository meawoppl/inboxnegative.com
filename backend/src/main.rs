mod addresses;
mod attachments;
mod db;
mod deleted;
mod http;
mod oauth;
mod salt;
mod smtp;
mod zmq_helpers;

use deleted::EmailStats;
use log::{debug, error, info, warn};
use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::flag;
use std::env;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};
use tokio::runtime;

fn attach_signal_watcher() -> Arc<AtomicBool> {
    // Setup sigterm/sigint handling. stich with an atomic bool that can be shared between threads
    let term_flag = Arc::new(AtomicBool::new(false));

    // Spawn a background task to listen for SIGTERM and SIGINT signals
    flag::register(SIGTERM, Arc::clone(&term_flag)).expect("Failed to register SIGTERM handler");
    flag::register(SIGINT, Arc::clone(&term_flag)).expect("Failed to register SIGINT handler");

    term_flag
}

async fn spawn_testing_message_sender() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let skt = zmq_helpers::bind_local_publisher().unwrap();
    loop {
        tokio::time::sleep(tokio::time::Duration::from_millis(2000)).await;
        debug!("Sending a test message...");
        // println!("Still alive... {}", a);
        zmq_helpers::send_multipart_string(
            &skt,
            vec!["".to_string(), "EmailMessageContentFromHTTP".to_string()],
        )
        .unwrap();
    }
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    pretty_env_logger::init();

    // Set up panic hook to capture stack traces
    std::panic::set_hook(Box::new(|panic_info| {
        let backtrace = std::backtrace::Backtrace::capture();

        if let Some(location) = panic_info.location() {
            error!(
                "PANIC occurred at {}:{}:{}",
                location.file(),
                location.line(),
                location.column()
            );
        }

        if let Some(message) = panic_info.payload().downcast_ref::<&str>() {
            error!("Panic message: {}", message);
        } else if let Some(message) = panic_info.payload().downcast_ref::<String>() {
            error!("Panic message: {}", message);
        } else {
            error!("Panic occurred with unknown payload type");
        }

        error!("Stack backtrace:\n{}", backtrace);
    }));

    // Load the hash salts before anything can hash. A missing salt is a hard
    // startup failure by design: booting without it would silently produce
    // different addresses and stats keys, orphaning every existing record.
    if let Err(e) = salt::init() {
        error!("❌ Refusing to start: {}", e);
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, e));
    }

    let term_flag = attach_signal_watcher();

    // Parse/validate the various
    info!(
        "Started from: {}",
        env::current_dir()
            .unwrap()
            .into_os_string()
            .into_string()
            .unwrap()
    );

    let smtp_host = std::env::var("SMTP_HOST").unwrap_or_else(|_| String::from("0.0.0.0"));
    let smtp_port = std::env::var("SMTP_PORT").unwrap_or_else(|_| String::from("2525"));

    let http_host = std::env::var("HTTP_HOST").unwrap_or_else(|_| String::from("0.0.0.0"));
    let http_port = std::env::var("HTTP_PORT").unwrap_or_else(|_| String::from("8080"));

    // Check if in testing mode (bypasses OAuth authentication)
    let testing_mode = std::env::var("TESTING_MODE")
        .unwrap_or_else(|_| String::from("false"))
        .to_lowercase()
        == "true";
    if testing_mode {
        warn!("🧪 TESTING MODE ENABLED: Authentication bypassed with testing@gmail.com");
    }

    // NB: HTTP_ROOT is not used upstream in the codebase I want to config this here, but
    // it has been hard to thread the string through the http service_fn method??

    let http_addr: SocketAddr = format!("{}:{}", http_host, http_port).parse().unwrap();
    let smtp_addr: SocketAddr = format!("{}:{}", smtp_host, smtp_port).parse().unwrap();

    let runtime = runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();

    let stats_path_str =
        std::env::var("STATS_PATH").unwrap_or_else(|_| String::from("email_stats.json"));
    let stats_path = Path::new(&stats_path_str);

    // Initialize the database connection pool
    let db_pool = match db::create_pool() {
        Ok(pool) => {
            info!("✅ Connected to database successfully");
            Some(pool)
        }
        Err(e) => {
            warn!("⚠️ Failed to connect to database: {}", e);
            warn!("⚠️ Falling back to file-based storage");
            None
        }
    };

    // Load the stats from file or create new ones
    let mut stats = EmailStats::load(stats_path).unwrap_or(EmailStats::new());

    // Attach the database pool if available
    if let Some(pool) = db_pool.as_ref() {
        stats.set_db_pool(Arc::clone(pool));

        // Migrate existing stats to the database if needed
        if let Err(e) = stats.migrate_to_db() {
            warn!("⚠️ Failed to migrate stats to database: {}", e);
        }
    }

    let email_stats = Arc::new(RwLock::new(stats));

    // Create attachment store with 5 minute TTL
    let attachment_store = Arc::new(attachments::AttachmentStore::new(300));

    let http_email_stats = Arc::clone(&email_stats);
    let http_attachment_store = Arc::clone(&attachment_store);
    let http_term = term_flag.clone();
    let http_handle = runtime.spawn(async move {
        http::run_http_server(
            http_addr,
            http_term,
            http_email_stats,
            http_attachment_store,
            testing_mode,
        )
        .await
        .unwrap()
    });

    let mut smtp_email_stats: Arc<RwLock<EmailStats>> = Arc::clone(&email_stats);
    let smtp_term = term_flag.clone();
    let smtp_handle = runtime.spawn(async move {
        smtp::run_smtp_server(smtp_addr, smtp_term, &mut smtp_email_stats)
            .await
            .unwrap_or_else(|e| panic!("SMTP server failed on {}: {:?}", smtp_addr, e))
    });

    // Spawn background task to cleanup expired attachments every minute
    let cleanup_attachment_store = Arc::clone(&attachment_store);
    let cleanup_term = term_flag.clone();
    runtime.spawn(async move {
        info!("Starting attachment cleanup task (runs every 60 seconds)");
        while !cleanup_term.load(std::sync::atomic::Ordering::Relaxed) {
            tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
            if !cleanup_term.load(std::sync::atomic::Ordering::Relaxed) {
                let size_before = cleanup_attachment_store.size();
                cleanup_attachment_store.cleanup();
                let size_after = cleanup_attachment_store.size();
                if size_before != size_after {
                    info!(
                        "Cleaned up {} expired attachments (was {}, now {})",
                        size_before - size_after,
                        size_before,
                        size_after
                    );
                } else {
                    debug!("No expired attachments to clean up (total: {})", size_after);
                }
            }
        }
        info!("Attachment cleanup task shutting down");
    });

    let spawn_local_sender = false;

    if spawn_local_sender {
        runtime.spawn(async move { spawn_testing_message_sender().await.unwrap() });
    }

    // Wait for a signal to shutdown
    while !term_flag.load(std::sync::atomic::Ordering::Relaxed) {
        if http_handle.is_finished() {
            error!("HTTP server has finished unexpectedly, shutting down.");
            match http_handle.await {
                Ok(_) => {}
                Err(e) => {
                    error!("HTTP server task panicked: {:?}", e);
                }
            }
            break;
        }

        if smtp_handle.is_finished() {
            error!("SMTP server has finished unexpectedly, shutting down.");
            match smtp_handle.await {
                Ok(_) => {}
                Err(e) => {
                    error!("SMTP server task panicked: {:?}", e);
                }
            }
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    term_flag.store(true, std::sync::atomic::Ordering::Relaxed);

    runtime.shutdown_background();

    Ok(())
}
