use std::pin::Pin;
use std::time::Duration;

use async_stream::stream;
use bytes::Bytes;
use futures::StreamExt;
use tokio::time::{interval, MissedTickBehavior};

pub type ByteStream = Pin<Box<dyn futures::Stream<Item = Result<Bytes, std::io::Error>> + Send>>;

const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(20);
const KEEPALIVE_BYTES: &[u8] = b": keepalive\n\n";

pub trait SseTransform: Send {
    fn transform_line(&mut self, line: &[u8]) -> Vec<Bytes>;
    fn finish(&mut self) -> Vec<Bytes> {
        Vec::new()
    }
}

/// Buffer incoming SSE bytes into lines and apply `transform` per line.
/// Sends SSE comment keepalives when upstream is silent for longer than
/// [`KEEPALIVE_INTERVAL`].
pub fn wrap_sse_transform<S, T>(inner: S, transform: T) -> ByteStream
where
    S: futures::Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static,
    T: SseTransform + Send + 'static,
{
    Box::pin(stream! {
        let mut transform = transform;
        let mut buffer = Vec::new();
        futures::pin_mut!(inner);
        let mut keepalive = interval(KEEPALIVE_INTERVAL);
        keepalive.set_missed_tick_behavior(MissedTickBehavior::Delay);
        keepalive.tick().await;

        loop {
            tokio::select! {
                item = inner.next() => {
                    match item {
                        None => break,
                        Some(Ok(chunk)) => {
                            keepalive.reset();
                            buffer.extend_from_slice(&chunk);
                            while let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
                                let line: Vec<u8> = buffer.drain(..=pos).collect();
                                for out in transform.transform_line(&line) {
                                    yield Ok(out);
                                }
                            }
                        }
                        Some(Err(e)) => {
                            yield Err(e);
                        }
                    }
                }
                _ = keepalive.tick() => {
                    yield Ok(Bytes::from_static(KEEPALIVE_BYTES));
                }
            }
        }
        if !buffer.is_empty() {
            for out in transform.transform_line(&buffer) {
                yield Ok(out);
            }
        }
        for out in transform.finish() {
            yield Ok(out);
        }
    })
}

pub fn anthropic_sse_event(event: &str, json_data: &[u8]) -> Bytes {
    let mut out = Vec::with_capacity(event.len() + json_data.len() + 16);
    out.extend_from_slice(b"event: ");
    out.extend_from_slice(event.as_bytes());
    out.extend_from_slice(b"\ndata: ");
    out.extend_from_slice(json_data);
    out.extend_from_slice(b"\n\n");
    Bytes::from(out)
}

pub fn responses_sse_event(event: &str, json_data: &[u8]) -> Bytes {
    anthropic_sse_event(event, json_data)
}
