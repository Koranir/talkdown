//! Stable-identity async channels that wake iced subscriptions on each event.

use iced::Subscription;

use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_STREAM_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub struct EventSender<T>(async_channel::Sender<T>);

pub struct EventStream<T> {
    id: u64,
    receiver: async_channel::Receiver<T>,
}

struct SubscriptionData<T> {
    id: u64,
    receiver: async_channel::Receiver<T>,
}

impl<T> Hash for SubscriptionData<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

pub fn unbounded<T>() -> (EventSender<T>, EventStream<T>) {
    let (sender, receiver) = async_channel::unbounded();
    (
        EventSender(sender),
        EventStream {
            id: NEXT_STREAM_ID.fetch_add(1, Ordering::Relaxed),
            receiver,
        },
    )
}

pub fn bounded<T>(capacity: usize) -> (EventSender<T>, EventStream<T>) {
    let (sender, receiver) = async_channel::bounded(capacity);
    (
        EventSender(sender),
        EventStream {
            id: NEXT_STREAM_ID.fetch_add(1, Ordering::Relaxed),
            receiver,
        },
    )
}

impl<T> EventSender<T> {
    pub fn send(&self, event: T) -> Result<(), async_channel::SendError<T>> {
        self.0.send_blocking(event)
    }

    pub fn try_send(&self, event: T) -> Result<(), async_channel::TrySendError<T>> {
        self.0.try_send(event)
    }
}

impl<T> EventStream<T>
where
    T: Send + 'static,
{
    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn subscription(&self) -> Subscription<T> {
        Subscription::run_with(
            SubscriptionData {
                id: self.id,
                receiver: self.receiver.clone(),
            },
            receiver_stream::<T>,
        )
    }

    pub fn tagged_subscription(&self) -> Subscription<(u64, T)> {
        self.subscription().with(self.id)
    }

    #[cfg(test)]
    pub fn try_iter(&self) -> impl Iterator<Item = T> + '_ {
        std::iter::from_fn(|| self.receiver.try_recv().ok())
    }
}

fn receiver_stream<T>(data: &SubscriptionData<T>) -> async_channel::Receiver<T> {
    data.receiver.clone()
}
