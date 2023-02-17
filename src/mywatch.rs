use futures::task::AtomicWaker;
use std::future::Future;
use std::pin::Pin;
use std::sync::{atomic::*, Arc, Mutex, RwLock, RwLockReadGuard};
use std::task::{Context, Poll};

pub fn channel<T>(value: T) -> (Sender<T>, Receiver<T>) {
    let shared = Arc::pin(Shared {
        value: RwLock::new(value),
        version: AtomicU64::new(0),
        subscribers: Mutex::new(Subscriber::new()),
    });
    let sender = Sender {
        shared: shared.clone(),
    };
    let receiver = shared.subscribe(0);
    (sender, receiver)
}

struct Subscriber {
    prev: *mut Subscriber,
    next: *mut Subscriber,
    waker: AtomicWaker,
}

// safety: Subscriber is always associated with some list, or
// has null pointers.
unsafe impl Send for Subscriber {}

// safety: we only ever manipulate Subscriber's pointers while locked
unsafe impl Sync for Subscriber {}

impl Subscriber {
    fn new() -> Self {
        Subscriber {
            prev: std::ptr::null_mut(),
            next: std::ptr::null_mut(),
            waker: AtomicWaker::new(),
        }
    }
    fn insert(self: Pin<&mut Self>, list: Pin<&mut Subscriber>) {
        assert!(self.prev.is_null());
        assert!(self.next.is_null());
		let this = self.get_mut();
		let list = list.get_mut();
        this.prev = list as *mut _;
        this.next = list.next;
        if !list.next.is_null() {
			// safety: checked for null ptr, list is locked
            unsafe {
                (*list.next).prev = this as *mut _;
            }
        }
        list.next = this as *mut _;
    }
    fn remove(&mut self, _list: Pin<&mut Subscriber>) {
        let prev = self.prev;
        let next = self.next;
        assert!(!prev.is_null());
		// safety: list is locked, prev can not be null
        unsafe {
            (*prev).next = next;
            if !next.is_null() {
                (*next).prev = prev;
            }
        }
        self.prev = std::ptr::null_mut();
        self.next = std::ptr::null_mut();
    }
}

struct Shared<T> {
    value: RwLock<T>,
    version: AtomicU64,
    subscribers: Mutex<Subscriber>,
}

impl<T> Shared<T> {
    fn subscribe(self: &Pin<Arc<Self>>, version: u64) -> Receiver<T> {
        Receiver {
            shared: self.clone(),
            version,
            subscription: None,
        }
    }
}

pub struct Sender<T> {
    shared: Pin<Arc<Shared<T>>>,
}

impl<T> Sender<T> {
    fn notify_subs(&self) {
        let mut subscriber = self.shared.subscribers.lock().unwrap().next;
        while !subscriber.is_null() {
			// safety: list is locked
            unsafe {
                (*subscriber).waker.wake();
                subscriber = (*subscriber).next;
            }
        }
    }

    pub fn send(&self, value: T) {
        *self.shared.value.write().unwrap() = value;
        self.shared.version.fetch_add(2, Ordering::Release);
        self.notify_subs();
    }

    pub fn subscribe(&self) -> Receiver<T> {
        let version = self.shared.version.load(Ordering::Relaxed);
        self.shared.subscribe(version)
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        self.shared.version.fetch_or(1, Ordering::Release);
        self.notify_subs();
    }
}

pub struct Receiver<T> {
    shared: Pin<Arc<Shared<T>>>,
    subscription: Option<Subscriber>,
    version: u64,
}

impl<T> Receiver<T> {
    pub fn changed(self: Pin<&mut Self>) -> Changed<'_, T> {
        Changed { receiver: self }
    }

    pub fn borrow(&self) -> RwLockReadGuard<'_, T> {
        self.shared.value.read().unwrap()
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        if let Some(subscription) = self.subscription.as_mut() {
            subscription.remove(Pin::new(&mut self.shared.subscribers.lock().unwrap()));
        }
    }
}

impl<T> Clone for Receiver<T> {
    fn clone(&self) -> Self {
        self.shared.subscribe(self.version)
    }
}

pub struct Changed<'a, T> {
    receiver: Pin<&'a mut Receiver<T>>,
}

impl<'a, T> Future for Changed<'a, T> {
    type Output = Result<(), ()>;

    fn poll(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Self::Output> {
        let receiver = self.get_mut().receiver.as_mut().get_mut();
        let version = receiver.shared.version.load(Ordering::Relaxed);
        if (version & 1) != 0 {
            return Poll::Ready(Err(()));
        }
        if version > receiver.version {
            receiver.version = version;
            return Poll::Ready(Ok(()));
        }

        let subscription = if let Some(sub) = receiver.subscription.as_mut() {
            sub
        } else {
            let mut sub = Pin::new(receiver.subscription.insert(Subscriber::new()));
            sub.as_mut().insert(Pin::new(&mut receiver.shared.subscribers.lock().unwrap()));
            sub.get_mut()
        };
        subscription.waker.register(ctx.waker());

        let version = receiver.shared.version.load(Ordering::Acquire);
        if (version & 1) != 0 {
            subscription.waker.take();
            return Poll::Ready(Err(()));
        }
        if version > receiver.version {
            subscription.waker.take();
            receiver.version = version;
            return Poll::Ready(Ok(()));
        }
        Poll::Pending
    }
}
