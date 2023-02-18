use futures::task::AtomicWaker;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{atomic::*, Arc, Mutex, RwLock, RwLockReadGuard};
use std::task::{Context, Poll};

pub fn channel<T>(value: T) -> (Sender<T>, Receiver<T>) {
    let shared = Arc::new(Shared {
        value: RwLock::new(value),
        version: AtomicU64::new(0),
        subscribers: Default::default(),
        next_id: AtomicU64::new(1),
    });
    let sender = Sender {
        shared: shared.clone(),
    };
    let receiver = shared.subscribe(0);
    (sender, receiver)
}

struct Shared<T> {
    value: RwLock<T>,
    version: AtomicU64,
    subscribers: Mutex<HashMap<u64, Arc<AtomicWaker>>>,
    next_id: AtomicU64,
}

impl<T> Shared<T> {
    fn subscribe(self: &Arc<Self>, version: u64) -> Receiver<T> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let sub = Arc::new(AtomicWaker::new());
        self.subscribers.lock().unwrap().insert(id, sub.clone());
        Receiver {
            shared: self.clone(),
            sub,
            version,
            id,
        }
    }
}

pub struct Sender<T> {
    shared: Arc<Shared<T>>,
}

impl<T> Sender<T> {
    pub fn send(&self, value: T) {
        *self.shared.value.write().unwrap() = value;
        self.shared.version.fetch_add(2, Ordering::Release);
        for sub in self.shared.subscribers.lock().unwrap().values() {
            sub.wake();
        }
    }

    pub fn subscribe(&self) -> Receiver<T> {
        let version = self.shared.version.load(Ordering::Relaxed);
        self.shared.subscribe(version)
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        self.shared.version.fetch_or(1, Ordering::Release);
        for sub in self.shared.subscribers.lock().unwrap().values() {
            sub.wake();
        }
    }
}

pub struct Receiver<T> {
    shared: Arc<Shared<T>>,
    sub: Arc<AtomicWaker>,
    version: u64,
    id: u64,
}

impl<T> Receiver<T> {
    pub fn changed(&mut self) -> Changed<'_, T> {
        Changed { receiver: self }
    }

    pub fn borrow(&self) -> RwLockReadGuard<'_, T> {
        self.shared.value.read().unwrap()
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        self.shared.subscribers.lock().unwrap().remove(&self.id);
    }
}

impl<T> Clone for Receiver<T> {
    fn clone(&self) -> Self {
        self.shared.subscribe(self.version)
    }
}

pub struct Changed<'a, T> {
    receiver: &'a mut Receiver<T>,
}

impl<'a, T> Future for Changed<'a, T> {
    type Output = Result<(), ()>;

    fn poll(mut self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Self::Output> {
        let version = self.receiver.shared.version.load(Ordering::Relaxed);
        if (version & 1) != 0 {
            return Poll::Ready(Err(()));
        }
        if version > self.receiver.version {
            self.receiver.version = version;
            return Poll::Ready(Ok(()));
        }
        self.receiver.sub.register(ctx.waker());
        let version = self.receiver.shared.version.load(Ordering::Acquire);
        if (version & 1) != 0 {
            return Poll::Ready(Err(()));
        }
        if version > self.receiver.version {
            self.receiver.version = version;
            return Poll::Ready(Ok(()));
        }
        Poll::Pending
    }
}
