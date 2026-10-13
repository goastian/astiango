// Stract is an open source web search engine.
// Copyright (C) 2024 Stract ApS
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as
// published by the Free Software Foundation, either version 3 of the
// License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

use crate::Result;
use futures::Stream;
use std::{collections::VecDeque, future::Future};

pub trait StreamingResponse: Unpin + Sized {
    type Item: Unpin;
    fn next_batch(&mut self) -> impl Future<Output = Result<Vec<Self::Item>>>;
    fn stream(self) -> impl Stream<Item = Result<Self::Item>> + Unpin {
        Box::pin(futures::stream::unfold(
            (self, VecDeque::new(), false),
            |(mut inner, mut batch, done)| async move {
                if done {
                    return None;
                }
                if batch.is_empty() {
                    match inner.next_batch().await {
                        Ok(items) if items.is_empty() => return None,
                        Ok(items) => batch.extend(items),
                        Err(error) => return Some((Err(error), (inner, batch, true))),
                    }
                }
                batch
                    .pop_front()
                    .map(|item| (Ok(item), (inner, batch, false)))
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use futures::StreamExt;

    use super::*;

    struct TestStreamingResponse {
        items: Vec<String>,
        index: usize,
    }

    impl TestStreamingResponse {
        fn new(items: Vec<String>) -> Self {
            Self { items, index: 0 }
        }
    }

    impl StreamingResponse for TestStreamingResponse {
        type Item = String;

        async fn next_batch(&mut self) -> Result<Vec<Self::Item>> {
            if self.index >= self.items.len() {
                return Ok(Vec::new());
            }

            let mut res = Vec::new();

            res.push(self.items[self.index].clone());

            self.index += 1;

            Ok(res)
        }
    }

    #[tokio::test]
    async fn test_streaming_response_stream() {
        let mut stream =
            TestStreamingResponse::new(vec!["a".to_string(), "b".to_string(), "c".to_string()])
                .stream();

        assert_eq!(
            stream.next().await.map(|r| r.unwrap()),
            Some("a".to_string())
        );
        assert_eq!(
            stream.next().await.map(|r| r.unwrap()),
            Some("b".to_string())
        );
        assert_eq!(
            stream.next().await.map(|r| r.unwrap()),
            Some("c".to_string())
        );
        assert!(stream.next().await.is_none());
    }

    #[tokio::test]
    async fn test_empty_stream() {
        let mut stream = TestStreamingResponse::new(Vec::new()).stream();

        assert!(stream.next().await.is_none());
    }
}

#[cfg(test)]
mod pending_tests {
    use super::*;
    use futures::StreamExt;
    struct Delayed {
        offset: usize,
    }
    impl StreamingResponse for Delayed {
        type Item = usize;
        async fn next_batch(&mut self) -> Result<Vec<usize>> {
            tokio::task::yield_now().await;
            if self.offset == 250 {
                anyhow::bail!("controlled stream error");
            }
            let end = (self.offset + 100).min(250);
            let result = (self.offset..end).collect();
            self.offset = end;
            Ok(result)
        }
    }
    #[tokio::test]
    async fn pending_preserves_future_cursor_order_and_error() {
        let mut stream = Delayed { offset: 0 }.stream();
        for expected in 0..250 {
            assert_eq!(stream.next().await.unwrap().unwrap(), expected);
        }
        assert!(stream
            .next()
            .await
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("controlled"));
        assert!(stream.next().await.is_none());
    }
}
