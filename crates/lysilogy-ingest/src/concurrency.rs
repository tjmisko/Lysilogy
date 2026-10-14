//! Small concurrency helpers shared by the model passes.

use std::future::Future;
use std::pin::Pin;
use std::task::Poll;

/// A boxed future that may borrow from its caller.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Drive futures concurrently and return their outputs in input order.
pub async fn join_all<'a, T>(futures: Vec<BoxFuture<'a, T>>) -> Vec<T> {
    let mut slots: Vec<(Option<BoxFuture<'a, T>>, Option<T>)> = futures
        .into_iter()
        .map(|future| (Some(future), None))
        .collect();
    std::future::poll_fn(|context| {
        let mut waiting = false;
        for (future, output) in &mut slots {
            let Some(running) = future.as_mut() else {
                continue;
            };
            match running.as_mut().poll(context) {
                Poll::Ready(value) => {
                    *output = Some(value);
                    *future = None;
                }
                Poll::Pending => waiting = true,
            }
        }
        if waiting {
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    })
    .await;
    slots.into_iter().filter_map(|(_, output)| output).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn should_return_outputs_in_input_order_when_futures_finish_out_of_order() {
        let delays = [30_u64, 1, 15];
        let futures = delays
            .iter()
            .map(|delay| {
                let delay = *delay;
                let future: Pin<Box<dyn Future<Output = u64> + Send>> = Box::pin(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    delay
                });
                future
            })
            .collect();
        assert_eq!(join_all(futures).await, vec![30, 1, 15]);
        assert!(join_all::<u8>(Vec::new()).await.is_empty());
    }
}
