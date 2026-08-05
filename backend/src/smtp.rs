use std::io::{BufRead, BufReader, BufWriter, Error, ErrorKind, Result, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

use base64::Engine;
use log::{debug, error, info, warn};
use shared::email::Email;
use socket2::{Domain, Socket as Socket2, Type};

use crate::addresses::is_valid_address;
use crate::deleted::EmailStats;
use crate::zmq_helpers::{bind_local_publisher, send_multipart_string};
use zmq::Socket;

pub async fn run_smtp_server(
    address: SocketAddr,
    term_flag: Arc<AtomicBool>,
    email_stats: &mut Arc<RwLock<EmailStats>>,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Startup the smtp server
    let publisher = bind_local_publisher()?;
    info!("Starting SMTP server on {}", address);

    // Create socket with SO_REUSEADDR to allow binding even if connections are in TIME_WAIT
    let socket = Socket2::new(Domain::IPV4, Type::STREAM, None)?;
    socket.set_reuse_address(true)?;
    socket.bind(&address.into())?;
    socket.listen(128)?;
    socket.set_nonblocking(true)?;

    let listener: TcpListener = socket.into();
    info!(
        "SMTP server listening on {} (SO_REUSEADDR enabled)",
        address
    );

    // Start the smtp server
    loop {
        // Check if the server should keep running
        if term_flag.load(Ordering::Relaxed) {
            info!("Shut down triggered by signal semaphore...");
            break;
        }

        match listener.accept() {
            Ok((stream, addr)) => {
                debug!("Connection from {}", addr);
                if let Err(e) = handle_client(stream, addr, Some(&publisher), email_stats) {
                    // Broken pipe errors are common from health checks and scanners
                    if e.kind() == ErrorKind::BrokenPipe || e.kind() == ErrorKind::ConnectionAborted
                    {
                        debug!("Client {} disconnected early: {}", addr, e);
                    } else if e.kind() == ErrorKind::UnexpectedEof {
                        debug!("Client {} closed connection unexpectedly: {}", addr, e);
                    } else {
                        error!("Error handling client {}: {}", addr, e);
                    }
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                // No incoming connections, let's do something else
                thread::sleep(Duration::from_millis(1));
            }
            Err(e) => error!("Connection failed: {}", e),
        }
    }

    // Cleanup the sockets
    if let Ok(Ok(last_skt)) = publisher.get_last_endpoint() {
        debug!("Unbinding from {}", last_skt);
        publisher.unbind(&last_skt)?;
    }

    Ok(())
}

pub fn handle_client(
    stream: TcpStream,
    address: SocketAddr,
    publisher: Option<&Socket>,
    email_stats: &mut Arc<RwLock<EmailStats>>,
) -> Result<()> {
    // 10 seconds is plenty... right?
    stream.set_read_timeout(Some(Duration::new(10, 0)))?;
    let mut reader = BufReader::new(&stream);
    let mut writer = BufWriter::new(&stream);
    smtp_session(&mut reader, &mut writer, address, publisher, email_stats)
}

fn write_response<W: Write>(writer: &mut W, response: &str) -> Result<()> {
    writer.write_all(response.as_bytes())?;
    writer.flush()?;
    Ok(())
}

/// Reads a line from the given BufReader with a timeout, one character at a time.
/// Returns Ok with the line or Err on timeout or other I/O errors.
fn read_line_with_timeout<R: BufRead>(reader: &mut R, line: &mut String) -> Result<()> {
    // Buffer to read one character at a time
    let mut buffer = [0; 1];

    line.clear();

    loop {
        match reader.read(&mut buffer) {
            Ok(0) => {
                // End of stream reached
                return Ok(());
            }
            Ok(_) => {
                // Successfully read one character
                let c = buffer[0] as char;

                // Append the character to the line
                line.push(c);

                // Break if a newline is encountered, similar to `read_line`
                if c == '\n' {
                    break;
                }
            }
            Err(e) if e.kind() == ErrorKind::TimedOut => {
                // Handle timeout
                return Err(Error::new(ErrorKind::TimedOut, "Read operation timed out"));
            }
            Err(e) => {
                // Handle other errors
                return Err(e);
            }
        }
    }

    Ok(())
}

fn extract_path(line: &str) -> Option<String> {
    let start_char = line.find("<");
    let end_char = line.find(">");
    if let (Some(start), Some(end)) = (start_char, end_char) {
        if start < end {
            return Some(line[start + 1..end].to_string());
        }
    }
    None
}

fn smtp_session<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    address: SocketAddr,
    publisher: Option<&Socket>,
    email_stats: &mut Arc<RwLock<EmailStats>>,
) -> Result<()> {
    // Send greeting
    match writer.write_all(b"220 inboxnegative.com SMTP Server DTF\r\n") {
        Ok(()) => {}
        Err(e) if e.kind() == ErrorKind::BrokenPipe => {
            // Client disconnected immediately after connecting (common for health checks)
            return Err(e);
        }
        Err(e) => return Err(e),
    }

    match writer.flush() {
        Ok(()) => {}
        Err(e) if e.kind() == ErrorKind::BrokenPipe => {
            // Client disconnected immediately after connecting
            return Err(e);
        }
        Err(e) => return Err(e),
    }

    let mut line = String::new();
    let mut from = String::new();
    let mut recipients: Vec<String> = Vec::new();
    let mut data = String::new();

    loop {
        match read_line_with_timeout(reader, &mut line) {
            Ok(_) => {}
            Err(e) => {
                if e.kind() == ErrorKind::TimedOut {
                    write_response(writer, "421 Timeout\r\n")?;
                }
                return Err(e);
            }
        }

        let trimmed_line = line.trim();
        let command = trimmed_line.to_uppercase();

        if !command.is_empty() {
            debug!("Received: {:?}", command);
        }

        match command.as_str() {
            s if s.starts_with("HELO") => {
                debug!("Received HELO");
                let resp = format!("250 Hello {}\r\n", address.ip());
                write_response(writer, resp.as_str())?;
            }
            s if s.starts_with("EHLO") => {
                debug!("Received EHLO");
                // Multi-line response advertising AUTH capabilities
                let resp = format!("250-Hello {}\r\n250 AUTH PLAIN LOGIN\r\n", address.ip());
                write_response(writer, resp.as_str())?;
            }
            s if s.starts_with("MAIL FROM:") => match extract_path(&line) {
                Some(path) => {
                    from = path;
                    write_response(writer, "250 OK\r\n")?;
                    continue;
                }
                None => {
                    write_response(writer, "501 Invalid sender\r\n")?;
                    continue;
                }
            },
            s if s.starts_with("RCPT TO:") => {
                if recipients.len() >= 100 {
                    write_response(writer, "452 Too many recipients\r\n")?;
                    continue;
                }

                // find the recipient email address
                match extract_path(&line) {
                    Some(path) => {
                        if let Err(e) = is_valid_address(&path) {
                            warn!("Invalid recipient {}, probably probing: {:?}", path, e);
                            write_response(writer, "501 Invalid recipient\r\n")?;
                            continue;
                        }

                        recipients.push(path);
                        write_response(writer, "250 OK\r\n")?;
                        continue;
                    }
                    None => {
                        write_response(writer, "501 Invalid recipient\r\n")?;
                        continue;
                    }
                }
            }
            s if s.starts_with("AUTH PLAIN") => {
                // AUTH PLAIN base64(username\0username\0password)
                // Use original trimmed_line to preserve case-sensitive base64
                let auth_data = trimmed_line
                    .strip_prefix("AUTH PLAIN ")
                    .or_else(|| trimmed_line.strip_prefix("auth plain "))
                    .unwrap_or("");

                if let Ok(decoded) = base64::prelude::BASE64_STANDARD.decode(auth_data.trim()) {
                    if let Ok(decoded_str) = String::from_utf8(decoded) {
                        let parts: Vec<&str> = decoded_str.split('\0').collect();
                        if parts.len() >= 3 {
                            info!(
                                "AUTH PLAIN attempt from {}: username='{}', password='{}'",
                                address, parts[1], parts[2]
                            );
                        } else {
                            info!("AUTH PLAIN attempt from {} with malformed data", address);
                        }
                    }
                } else {
                    info!("AUTH PLAIN attempt from {} with invalid base64", address);
                }

                write_response(writer, "235 Authentication successful\r\n")?;
                continue;
            }
            s if s.starts_with("AUTH LOGIN") => {
                // AUTH LOGIN is interactive - prompt for username, then password
                write_response(writer, "334 VXNlcm5hbWU6\r\n")?; // base64("Username:")

                line.clear();
                reader.read_line(&mut line)?;
                let username =
                    if let Ok(decoded) = base64::prelude::BASE64_STANDARD.decode(line.trim()) {
                        String::from_utf8(decoded).unwrap_or_else(|_| String::from("<invalid>"))
                    } else {
                        String::from("<invalid>")
                    };

                write_response(writer, "334 UGFzc3dvcmQ6\r\n")?; // base64("Password:")

                line.clear();
                reader.read_line(&mut line)?;
                let password =
                    if let Ok(decoded) = base64::prelude::BASE64_STANDARD.decode(line.trim()) {
                        String::from_utf8(decoded).unwrap_or_else(|_| String::from("<invalid>"))
                    } else {
                        String::from("<invalid>")
                    };

                info!(
                    "AUTH LOGIN attempt from {}: username='{}', password='{}'",
                    address, username, password
                );

                write_response(writer, "235 Authentication successful\r\n")?;
                continue;
            }
            s if s.starts_with("AUTH") => {
                // Catch-all for other AUTH methods
                warn!("Unsupported AUTH method from {}: {}", address, trimmed_line);
                write_response(writer, "235 Authentication successful\r\n")?;
                continue;
            }
            s if s.starts_with("RSET") => {
                from.clear();
                recipients.clear();
                data.clear();
                write_response(writer, "250 OK\r\n")?;
            }
            "DATA" => {
                write_response(writer, "354 Start mail input; end with <CRLF>.<CRLF>\r\n")?;
                loop {
                    line.clear();
                    reader.read_line(&mut line)?;
                    if line.trim() == "." {
                        break;
                    }
                    // Dot-Stuffing "convention" for SMTP
                    // https://github.com/mikel/mail/issues/669
                    if line.starts_with("..") {
                        line = line[1..].to_string();
                    }
                    data.push_str(&line);
                }

                for recipient in &recipients {
                    info!("Sending email to: {}", recipient);
                    let email = Email::new(from.clone(), recipient.clone(), data.clone());
                    match publisher {
                        Some(publisher) => {
                            let serialized_email = serde_json::to_string(&email)?;
                            let parts = vec![recipient.to_string(), serialized_email];

                            debug!("Email data sent: {:?}", &parts);
                            if let Err(e) = send_multipart_string(publisher, parts) {
                                error!("Error sending email: {:?}", e);
                            }
                        }
                        None => {
                            debug!("Email data deleted: {} bytes", data.len());
                        }
                    }
                }

                // We do this after to just grab the write lock once
                match email_stats.write() {
                    Ok(mut stats) => {
                        for recipient in &recipients {
                            if let Err(e) = stats.add_for_email_address(recipient) {
                                error!("Error updating stats for recipient {}: {}", recipient, e);
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to acquire write lock for email stats: {}", e);
                    }
                }

                write_response(writer, "250 OK\r\n")?;
            }
            "QUIT" => {
                write_response(writer, "221 Bye\r\n")?;
                debug!("Connection closed");
                break;
            }
            s if s.starts_with("VRFY") => {
                write_response(writer, "252 Verify\r\n")?;
            }
            s if s.starts_with("NOOP") => {
                write_response(writer, "250 OK\r\n")?;
            }
            "" => {
                // We don't print this b/c this is what aws uses for health checks
                // also what many internet scanners use, so its spammy AF
                debug!(
                    "Empty command from {} (likely health check or scanner)",
                    address
                );
                write_response(writer, "500 Command not recognized\r\n")?;
            }
            _ => {
                warn!("Unrecognized command: '{:?}'", trimmed_line);
                write_response(writer, "500 Command not recognized\r\n")?;
            }
        }
    }
    Ok(())
}

// Test code is inside a #[cfg(test)] module
#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::{
        io::{Cursor, Read},
        str::FromStr,
    };

    use crate::zmq_helpers::{bind_local_publisher, connect_to_publisher, recv_multipart_string};

    struct TimeoutReader {}

    impl TimeoutReader {
        fn new() -> Self {
            TimeoutReader {}
        }
    }

    impl Read for TimeoutReader {
        fn read(&mut self, _buf: &mut [u8]) -> Result<usize> {
            Err(Error::new(ErrorKind::TimedOut, "Read operation timed out"))
        }
    }

    #[test]
    fn test_read_line_with_timeout() {
        // Test reading a normal line
        let data = "This is a test line.\nMore data here.";
        let mut reader = Cursor::new(data);
        let mut line = String::new();

        // Read the first line
        let result = read_line_with_timeout(&mut reader, &mut line);
        assert!(result.is_ok(), "Expected Ok result, got Err: {:?}", result);
        assert_eq!(line, "This is a test line.\n", "Unexpected line content");

        // Read the second line
        line.clear();
        let result = read_line_with_timeout(&mut reader, &mut line);
        assert!(result.is_ok(), "Expected Ok result, got Err: {:?}", result);
        assert_eq!(line, "More data here.", "Unexpected line content");

        // Test handling EOF
        let mut eof_reader = Cursor::new("");
        let mut eof_line = String::new();
        let eof_result = read_line_with_timeout(&mut eof_reader, &mut eof_line);
        assert!(eof_result.is_ok(), "Expected Ok result on EOF");
        assert!(eof_line.is_empty(), "Expected empty string on EOF");

        // Test simulated timeout error
        let mut timeout_reader = BufReader::new(TimeoutReader::new());
        let mut timeout_line = String::new();
        let timeout_result = read_line_with_timeout(&mut timeout_reader, &mut timeout_line);
        match timeout_result {
            Err(e) if e.kind() == ErrorKind::TimedOut => {
                println!("Simulated timeout error correctly handled.");
            }
            _ => panic!("Expected a timeout error"),
        }
    }

    const EMAIL_ADDRESS: &str = "deadbeefdeadbeef@inboxnegative.com";

    const VALID_INPUT: &str = "HELO localhost\r\n\
        MAIL FROM: <foo@bar.baz>\r\n\
        RCPT TO: <deadbeefdeadbeef@inboxnegative.com>\r\n\
        DATA\r\n\
        Subject: Test email\r\n\
        This is a test email.\r\n\
        This is a test email.\r\n\
        This is a test email.\r\n\
        This is a test email.\r\n\
        .\r\n\
        QUIT\r\n";

    const EXPECTED_OUTPUT: &str = "220 inboxnegative.com SMTP Server DTF\r\n\
        250 Hello 0.0.0.0\r\n\
        250 OK\r\n\
        250 OK\r\n\
        354 Start mail input; end with <CRLF>.<CRLF>\r\n\
        250 OK\r\n\
        221 Bye\r\n";

    #[test]
    fn test_smtp_session() {
        let mut input = VALID_INPUT.as_bytes();
        let mut output = Vec::new();

        // Create EmailStats with migrating=true for tests to work without DB
        let mut stats = EmailStats::new();
        stats.set_migrating(true);
        let mut email_stats: Arc<RwLock<EmailStats>> = Arc::new(RwLock::new(stats));

        {
            let mut reader = BufReader::new(&mut input);
            let mut writer = BufWriter::new(&mut output);
            let addr = SocketAddr::from_str("0.0.0.0:1234").unwrap();
            smtp_session(&mut reader, &mut writer, addr, None, &mut email_stats).unwrap();
        }

        let actual_output = String::from_utf8_lossy(&output);

        assert_eq!(actual_output, EXPECTED_OUTPUT);
        let unwrapped_stats = email_stats.read().unwrap();

        let email_str = EMAIL_ADDRESS.to_string();
        let n_sent = unwrapped_stats
            .get_deleted_count_for_address(&email_str)
            .unwrap_or(0);
        assert_eq!(n_sent, 1);

        let n_total = unwrapped_stats.get_total_deleted().unwrap_or(0);
        assert_eq!(n_total, 1);
    }

    #[test]
    fn test_smtp_session_rejects_probes() {
        let binding = VALID_INPUT.replace("beef", "kale");
        let mut input = binding.as_bytes();
        let mut output = Vec::new();

        // Create EmailStats with migrating=true for tests to work without DB
        let mut stats = EmailStats::new();
        stats.set_migrating(true);
        let mut email_stats: Arc<RwLock<EmailStats>> = Arc::new(RwLock::new(stats));

        {
            let mut reader = BufReader::new(&mut input);
            let mut writer = BufWriter::new(&mut output);
            let addr = SocketAddr::from_str("0.0.0.0:1234").unwrap();
            smtp_session(&mut reader, &mut writer, addr, None, &mut email_stats).unwrap();
        }

        let actual_output = String::from_utf8_lossy(&output);

        assert!(actual_output.contains("501 Invalid recipient"));
        let unwrapped_stats = email_stats.read().unwrap();
        let n_total = unwrapped_stats.get_total_deleted().unwrap_or(0);
        assert_eq!(n_total, 0);
    }

    #[test]
    #[serial]
    fn test_smtp_session_publish() {
        let mut input = VALID_INPUT.as_bytes();
        let mut output = Vec::new();

        // Create EmailStats with migrating=true for tests to work without DB
        let mut stats = EmailStats::new();
        stats.set_migrating(true);
        let mut email_stats: Arc<RwLock<EmailStats>> = Arc::new(RwLock::new(stats));

        let publisher = bind_local_publisher().unwrap();
        let subscriber = connect_to_publisher("").unwrap();

        // Wait for subscriber to be ready to avoid slow joiner problem
        std::thread::sleep(std::time::Duration::from_millis(100));

        {
            let mut reader = BufReader::new(&mut input);
            let mut writer = BufWriter::new(&mut output);
            let addr = SocketAddr::from_str("0.0.0.0:1234").unwrap();

            smtp_session(
                &mut reader,
                &mut writer,
                addr,
                Some(&publisher),
                &mut email_stats,
            )
            .unwrap();
        }

        let actual_output = String::from_utf8_lossy(&output);
        assert_eq!(actual_output, EXPECTED_OUTPUT);

        // Check that it posted a message
        subscriber.set_rcvtimeo(2000).unwrap(); // Increased timeout to 2 seconds

        // Retry up to 3 times if we get temporary unavailable errors
        let mut retry_count = 0;
        let max_retries = 3;
        let messages = loop {
            match recv_multipart_string(&subscriber) {
                Ok(msgs) => break msgs,
                Err(e) => {
                    retry_count += 1;
                    if retry_count >= max_retries {
                        panic!("Failed to receive message after {max_retries} attempts: {e}");
                    }
                    // Sleep briefly before retrying
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
        };

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0], "deadbeefdeadbeef@inboxnegative.com");
        let email: Email = serde_json::from_str(&messages[1]).unwrap();
        assert_eq!(email.sender, "foo@bar.baz");
        assert_eq!(email.destination, "deadbeefdeadbeef@inboxnegative.com");
    }

    #[test]
    fn test_extract_path() {
        let line = "RCPT TO: <foo@bar.com>";

        let path = extract_path(line);
        assert_eq!(path.unwrap(), "foo@bar.com");
    }

    #[test]
    fn test_extract_path_crazy_1() {
        let line = "RCPT TO: >foo@bar.com<";

        let path = extract_path(line);
        assert!(path.is_none());
    }

    #[test]
    fn test_extract_path_crazy_2() {
        let line = "<foo@bar.com";

        let path = extract_path(line);
        assert!(path.is_none());
    }

    #[test]
    fn test_extract_path_crazy_3() {
        let line = "foo@bar.com>";

        let path = extract_path(line);
        assert!(path.is_none());
    }
}
