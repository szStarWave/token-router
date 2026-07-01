use std::pin::Pin;

use async_stream::stream;
use bytes::Bytes;
use futures::StreamExt;

pub type ByteStream = Pin<Box<dyn futures::Stream<Item = Result<Bytes, std::io::Error>> + Send>>;

pub trait SseTransform: Send {
    fn transform_line(&mut self, line: &[u8]) -> Vec<Bytes>;
    fn finish(&mut self) -> Vec<Bytes> {
        Vec::new()
    }
}

/// Buffer incoming SSE bytes into lines and apply `transform` per line.
pub fn wrap_sse_transform<S, T>(inner: S, transform: T) -> ByteStream
where
    S: futures::Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static,
    T: SseTransform + Send + 'static,
{
    Box::pin(stream! {
        let mut transform = transform;
        let mut buffer = Vec::new();
        futures::pin_mut!(inner);
        while let Some(item) = inner.next().await {
            let chunk = match item {
                Ok(b) => b,
                Err(e) => {
                    yield Err(e);
                    continue;
                }
            };
            buffer.extend_from_slice(&chunk);
            while let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = buffer.drain(..=pos).collect();
                for out in transform.transform_line(&line) {
                    yield Ok(out);
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
