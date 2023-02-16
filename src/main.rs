use rand::prelude::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use tokio::sync::watch;
use tokio::time::sleep;

mod mywatch;

fn do_work(start: Instant, t: &AtomicU64) {
    let values: HashMap<i32, f64> = (1..=10).map(|i| (i, random())).collect();
    let message = serde_json::to_vec(&values).unwrap();
    let sum = message
        .into_iter()
        .map(|c| c as u32)
        .fold(0, u32::wrapping_add);
    if sum == 1337 {
        println!("foo");
    }
    t.store(start.elapsed().as_nanos() as u64, Ordering::Release);
}

macro_rules! impl_test {
    ($name:ident, $module:ident) => {
        async fn $name() {
            let (snd, _) = $module::channel(Instant::now());
            let t = Arc::new(AtomicU64::new(0));
            for _ in 0..1000 {
                let mut rcv = snd.subscribe();
                let t = t.clone();
                tokio::spawn(async move {
                    loop {
                        if rcv.changed().await.is_err() {
                            break;
                        }
                        // read lock
                        let start = *rcv.borrow();
                        do_work(start, &t);
                    }
                });
            }
            sleep(Duration::from_millis(25)).await;
            let mut results = Vec::with_capacity(1000);
            for _ in 0..1000 {
                let _ = snd.send(Instant::now());
                sleep(Duration::from_millis(25)).await;
                results.push(t.load(Ordering::SeqCst));
            }
            results.sort_unstable();
            let avg = results.iter().copied().sum::<u64>() / 1000;
            println!(
                "avg={:?}  <{:?} [ {:?} {:?} {:?} ] {:?}>",
                Duration::from_nanos(avg),
                Duration::from_nanos(results[50]),
                Duration::from_nanos(results[250]),
                Duration::from_nanos(results[500]),
                Duration::from_nanos(results[750]),
                Duration::from_nanos(results[950]),
            );
        }
    };
}

impl_test!(run_test, watch);
impl_test!(run_test_my, mywatch);

fn main() {
    println!("TOKIO WATCH");
    println!("running single thread");
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(run_test());
    println!("running multi thread with main thread sender");
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(run_test());
    println!("running multi thread with spawned sender");
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { tokio::spawn(run_test()).await.unwrap() });
    println!("CUSTOM WATCH");
    println!("running single thread");
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(run_test_my());
    println!("running multi thread with main thread sender");
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(run_test_my());
    println!("running multi thread with spawned sender");
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { tokio::spawn(run_test_my()).await.unwrap() });
}
