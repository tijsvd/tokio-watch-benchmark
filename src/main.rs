use rand::prelude::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use tokio::sync::{watch, Notify};
// use tokio::time::sleep;

mod mywatch;

const NTASK: usize = 1000;
const NREP: i32 = 1000;

#[derive(Default)]
struct WorkGroup {
    count: AtomicU64,
    done_n: Notify,
}

impl WorkGroup {
    fn add(&self, n: u64) {
        self.count.fetch_add(n, Ordering::Relaxed);
    }
    fn done(&self) {
        if self.count.fetch_sub(1, Ordering::Release) == 1 {
            self.done_n.notify_one();
        }
    }
    async fn wait(&self) {
        while self.count.load(Ordering::Acquire) > 0 {
            self.done_n.notified().await; // notify_one buffers
        }
    }
}

fn do_work(rng: &mut impl RngCore) -> u32 {
    // let start = Instant::now();
    let values: HashMap<i32, f64> = (1..=10).map(|i| (i, rng.gen())).collect();
    let message = serde_json::to_vec(&values).unwrap();
    message
        .into_iter()
        .map(|c| c as u32)
        .fold(0, u32::wrapping_add)
}

macro_rules! def_test {
    ($modname:ident, $defname:ident) => {
        mod $defname {
            use super::*;

            async fn one_rep(i: i32, snd: &$modname::Sender<i32>, wg: &WorkGroup) -> u64 {
                wg.add(NTASK as u64);
                let start = Instant::now();
                let _ = snd.send(i);
                wg.wait().await;
                start.elapsed().as_nanos() as u64
            }

            pub async fn run_test() {
                let (snd, _) = $modname::channel(0i32);
                let wg = Arc::new(WorkGroup::default());
                for n in 0..NTASK {
                    let mut rcv = snd.subscribe();
                    let wg = wg.clone();
                    let mut rng = rand::rngs::StdRng::seed_from_u64(n as u64);
                    tokio::spawn(async move {
                        loop {
                            if rcv.changed().await.is_err() {
                                break;
                            }
                            let _ = *rcv.borrow();
                            let r = do_work(&mut rng);
                            if r == 1337 {
                                println!("coincidence...");
                            }
                            wg.done();
                        }
                    });
                }
                // warmup
                let warmup_start = Instant::now();
                while warmup_start.elapsed() < Duration::from_millis(25) {
                    one_rep(42, &snd, &wg).await;
                }
                let mut results = Vec::with_capacity(NREP as usize);
                println!("start");
                for i in 0..NREP {
                    results.push(one_rep(i, &snd, &wg).await);
                }
                assert_eq!(results.len(), NREP as usize);
                results.sort_unstable();
                let avg = results.iter().copied().sum::<u64>() / NREP as u64;
                let l = NREP as usize;
                println!(
                    "avg={:?}  <{:?} [ {:?} {:?} {:?} ] {:?}>",
                    Duration::from_nanos(avg),
                    Duration::from_nanos(results[l / 20]),
                    Duration::from_nanos(results[l / 4]),
                    Duration::from_nanos(results[l / 2]),
                    Duration::from_nanos(results[l * 3 / 4]),
                    Duration::from_nanos(results[l * 19 / 20]),
                );
            }
        }
    };
}

def_test!(watch, wtest);
def_test!(mywatch, mytest);

fn main() {
    println!("TOKIO WATCH");
    /*
    println!("running single thread");
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(wtest::run_test());
    */
    println!("running single thread with spawned sender");
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { tokio::spawn(wtest::run_test()).await.unwrap() });
    /*
    println!("running multi thread with main thread sender");
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(wtest::run_test());
    */
    println!("running multi thread with spawned sender");
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { tokio::spawn(wtest::run_test()).await.unwrap() });
    println!("CUSTOM WATCH");
    /*
    println!("running single thread");
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(mytest::run_test());
    println!("running multi thread with main thread sender");
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(mytest::run_test());
    */
    println!("running multi thread with spawned sender");
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { tokio::spawn(mytest::run_test()).await.unwrap() });
}
