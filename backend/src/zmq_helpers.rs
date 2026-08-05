use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use bytes::Bytes;
use http_body::Body;
use hyper::body::Frame;
use lazy_static::lazy_static;
use log::{debug, error, info, warn};
use shared::email::Email;
use std::cell::Cell;
use std::sync::{Arc, Mutex};
use std::task::{Context as TaskContext, Waker};
use std::thread;
use std::time::Instant;
use std::{pin::Pin, task::Poll, thread::sleep, time::Duration};
use zmq::{Context, Socket, SocketType, POLLIN};

use crate::attachments::AttachmentStore;

// NOTE(meawoppl) - this -should- work with inproc sockets too and be a bit faster/cleaner
// That said, when I used them I don't see message transport across the threaded parts of
// the code. My suspicion is that the different threads are not sharing the same context
// which would make inproc fail as I am seeing, but to the best of my knowledge, I can't
// seem to figure out how to debug that
// Seems to be related to this: https://stackoverflow.com/a/67758135/25641012

// Generate unique ZMQ socket path per process to avoid conflicts during deployments
fn get_local_publisher_addr() -> String {
    format!("ipc://local_publisher_{}", std::process::id())
}

lazy_static! {
    static ref ZMQ_CONTEXT: Context = Context::new();
}

pub fn get_context_instance() -> &'static Context {
    let ctx = &ZMQ_CONTEXT;
    debug!("ZMQ Context Ptr: {:p}", ctx);
    ctx
}

pub fn bind_local_publisher() -> Result<Socket, zmq::Error> {
    let ctx = get_context_instance();
    let publisher = ctx.socket(SocketType::PUB)?;
    let addr = get_local_publisher_addr();

    // Try 3 times with some delay in between to avoid self-stomping
    let max_attempts = 3;
    for i in 0..max_attempts {
        match publisher.bind(&addr) {
            Ok(_) => {
                info!("ZMQ publisher bound to {}", addr);
                break;
            }
            Err(e) => {
                // The address is relative, so the socket lands in the working
                // directory -- log both, since a permission failure here is
                // almost always an unwritable cwd rather than a ZMQ problem.
                warn!(
                    "Error binding to local publisher at {} (cwd {:?}): {:?} Attempt ({}/{})",
                    addr,
                    std::env::current_dir(),
                    e,
                    i + 1,
                    max_attempts
                );

                // EACCES will not improve on retry; fail immediately rather
                // than burning the backoff on a permanent condition.
                if e == zmq::Error::EACCES || i == max_attempts - 1 {
                    return Err(e);
                }

                sleep(Duration::from_millis(250 * (i + 1) * (i + 1)));
            }
        }
    }
    Ok(publisher)
}

pub fn connect_to_publisher(topic: &str) -> Result<Socket, zmq::Error> {
    let ctx = get_context_instance();
    let subscriber = ctx.socket(SocketType::SUB)?;
    let addr = get_local_publisher_addr();

    subscriber.connect(&addr)?;
    subscriber.set_subscribe(topic.as_bytes())?;
    subscriber.set_linger(0)?;
    sleep(Duration::from_millis(10));

    debug!("ZMQ subscriber connected to {} with topic: {}", addr, topic);
    Ok(subscriber)
}

pub fn send_multipart_string(socket: &Socket, parts: Vec<String>) -> Result<(), zmq::Error> {
    for part in parts.iter().take(parts.len() - 1) {
        socket.send(part.as_bytes(), zmq::SNDMORE)?;
    }
    socket.send(parts.last().unwrap(), 0)?;
    Ok(())
}

pub fn recv_multipart_string(socket: &Socket) -> Result<Vec<String>, zmq::Error> {
    match socket.recv_multipart(0) {
        Ok(parts) => Ok(parts
            .into_iter()
            .map(|part| String::from_utf8(part).unwrap())
            .collect()),
        Err(e) => Err(e),
    }
}

pub struct ZMQStreamBody {
    socket: Arc<Mutex<Socket>>,
    last_ping: Arc<Mutex<Cell<Instant>>>,
    attachment_store: Arc<AttachmentStore>,
}

impl ZMQStreamBody {
    pub fn new(socket: Socket, attachment_store: Arc<AttachmentStore>) -> Self {
        ZMQStreamBody {
            socket: Arc::new(Mutex::new(socket)),
            last_ping: Arc::new(Mutex::new(Cell::new(Instant::now()))),
            attachment_store,
        }
    }

    fn update_last_ping(&self) {
        self.last_ping.lock().unwrap().set(Instant::now());
    }

    fn setup_poll_waiter(&self, waker: Waker) {
        let skt = self.socket.clone();
        thread::spawn(move || {
            let poll_lock_result = skt.try_lock();

            match poll_lock_result {
                Ok(mut_socket) => match mut_socket.poll(POLLIN, DUR.as_millis() as i64) {
                    Ok(_) => {}
                    Err(err) => {
                        error!("ZMQ poll error: {:?}", err);
                        thread::sleep(DUR);
                    }
                },
                Err(err) => {
                    error!("Failed to acquire lock for ZMQ poll loop, error: {:?}", err);
                    thread::sleep(DUR);
                }
            }
            waker.wake();
        });
    }
}

const DUR: Duration = Duration::from_millis(100);

impl Body for ZMQStreamBody {
    type Data = Bytes;
    type Error = std::io::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        {
            if self.last_ping.lock().unwrap().get().elapsed() > Duration::from_secs(1) {
                let encoded_ping = Bytes::from("event: ping\ndata: PING\n\n");
                self.update_last_ping();
                return Poll::Ready(Some(Ok(Frame::data(encoded_ping))));
            }

            let lock_result = self.socket.try_lock();
            match lock_result {
                Ok(mut_socket) => {
                    let poll_result = mut_socket.poll(POLLIN, 0);
                    match poll_result {
                        Ok(code) if code > 0 => {
                            debug!("ZMQ messages received");
                            let messages = recv_multipart_string(&mut_socket).unwrap();
                            // The first message is the subscription topic, we drop it here

                            // Parse the email and store attachments
                            if let Ok(email) = serde_json::from_str::<Email>(&messages[1]) {
                                for attachment in &email.attachments {
                                    self.attachment_store.store(attachment.clone());
                                    debug!("Stored attachment: {}", attachment.attachment_id);
                                }
                            }

                            let b64_encoded: String = BASE64.encode(messages[1].as_bytes());
                            let framed = format!("event: mail\ndata: {}\n\n", b64_encoded);
                            let encoded = Bytes::from(framed);
                            debug!("Received ZMQ frame: {} bytes", encoded.len());
                            let frame = Frame::data(encoded);
                            Poll::Ready(Some(Ok(frame)))
                        }
                        Ok(0) => {
                            // This means no zmq messages waiting
                            self.setup_poll_waiter(cx.waker().clone());
                            Poll::Pending
                        }
                        Ok(code) => {
                            warn!("Unexpected ZMQ poll return code: {}", code);
                            Poll::Ready(None)
                        }
                        Err(err) => {
                            error!("ZMQ poll error: {:?}", err);
                            Poll::Ready(None)
                        }
                    }
                }
                Err(_) => {
                    self.setup_poll_waiter(cx.waker().clone());
                    Poll::Pending
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::{thread::sleep, time::Duration};

    #[test]
    #[serial]
    fn test_bind_local_publisher() {
        let publisher = bind_local_publisher();
        assert!(publisher.is_ok());
        let addr = get_local_publisher_addr();
        publisher.unwrap().disconnect(&addr).unwrap();
    }

    #[test]
    #[serial]
    fn test_connect_to_publisher() {
        let subscriber = connect_to_publisher("my_topic");
        assert!(subscriber.is_ok());
        let addr = get_local_publisher_addr();
        subscriber.unwrap().disconnect(&addr).unwrap();
    }

    #[test]
    #[serial]
    fn test_pub_sub_transmission() {
        let publisher = bind_local_publisher().unwrap();
        let topic = "hello";
        let subscriber = connect_to_publisher(topic).unwrap();
        sleep(Duration::from_millis(500));

        subscriber.set_rcvtimeo(100).unwrap();
        // Message matched topic
        let message = "hello world";
        publisher.send(message.as_bytes(), 0).unwrap();
        let msg = subscriber.recv_string(0).unwrap().unwrap();
        assert_eq!(msg, message);

        // Message did not match topic
        let message = "goodbye world";
        publisher.send(message.as_bytes(), 0).unwrap();
        if subscriber.recv_string(0).is_ok() {
            panic!("Should not have received message")
        }

        let addr = get_local_publisher_addr();
        publisher.disconnect(&addr).unwrap();
        subscriber.disconnect(&addr).unwrap();
    }

    #[test]
    fn test_pub_sub_subscription_screens() {
        let addr = "inproc://test_pub_sub_xmit";
        let ctx = zmq::Context::new();
        let publisher = ctx.socket(zmq::PUB).unwrap();
        publisher.bind(addr).unwrap();

        let subscriber = ctx.socket(zmq::SUB).unwrap();
        subscriber.connect(addr).unwrap();

        let topic = "hello".to_string();
        subscriber.set_subscribe(topic.as_bytes()).unwrap();
        sleep(Duration::from_millis(50));

        send_multipart_string(&publisher, vec![topic, "Yes".to_string()]).unwrap();

        let msg = recv_multipart_string(&subscriber).unwrap();
        assert_eq!(msg, vec!["hello", "Yes"]);

        send_multipart_string(&publisher, vec!["offtopic".to_string(), "No!!".to_string()])
            .unwrap();

        // Shouldn't poll as able to be read
        let code = subscriber.poll(zmq::POLLIN, 200).unwrap();
        assert_eq!(code, 0);
    }

    #[test]
    fn test_send_recv_multipart_string() {
        let skt = "inproc://test_mp_str";
        let ctx = zmq::Context::new();
        let push_socket = ctx.socket(zmq::PUSH).unwrap();
        push_socket.bind(skt).unwrap();

        let pull_socket = ctx.socket(zmq::PULL).unwrap();
        pull_socket.connect(skt).unwrap();

        let strings = ["this", "is", "a", "test"]
            .iter()
            .map(|s| s.to_string())
            .collect();

        send_multipart_string(&push_socket, strings).unwrap();

        let received = recv_multipart_string(&pull_socket).unwrap();
        assert_eq!(received, vec!["this", "is", "a", "test"]);
    }

    #[test]
    fn test_send_recv_multipart_string_timeout() {
        let skt = "inproc://test_mp_str_to";
        let ctx = zmq::Context::new();

        let pull_socket = ctx.socket(zmq::PULL).unwrap();
        pull_socket.connect(skt).unwrap();
        pull_socket.set_rcvtimeo(10).unwrap();

        let err = recv_multipart_string(&pull_socket).unwrap_err();
        assert_eq!(err, zmq::Error::EAGAIN);
    }

    #[test]
    fn test_zmq_stream_body_poll_success() {
        let skt = "inproc://test_mp_str_to";
        let ctx = zmq::Context::new();

        let pull_socket = ctx.socket(zmq::PULL).unwrap();
        pull_socket.connect(skt).unwrap();
        pull_socket.set_rcvtimeo(10).unwrap();

        let store = Arc::new(AttachmentStore::new(300));
        let body_stream = ZMQStreamBody::new(pull_socket, store);
        let _n = http_body_util::BodyStream::new(body_stream);
        // TODO(meawoppl) - figure out how to test this?
    }
}
