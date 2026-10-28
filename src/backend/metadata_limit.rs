//! Global limit for concurrent yt-dlp "metadata" processes
//! (previews, playlist expansion, Spotify matching).
//!
//! Downloads have their own scheduler limit. This is separate: it caps
//! the number of short-lived yt-dlp search/preview processes running at
//! once, so pasting many links does not launch dozens of them.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, LazyLock, Mutex};
use std::time::Duration;

const MAX_CONCURRENT: usize = 4;
const WAIT_SLICE: Duration = Duration::from_millis(100);

struct Semaphore {
    active: Mutex<usize>,
    wake: Condvar,
}

static SEM: LazyLock<Semaphore> = LazyLock::new(|| Semaphore {
    active: Mutex::new(0),
    wake: Condvar::new(),
});

pub struct Permit {
    _private: (),
}

impl Drop for Permit {
    fn drop(&mut self) {
        let mut n = SEM.active.lock().unwrap_or_else(|e| e.into_inner());
        *n = n.saturating_sub(1);
        SEM.wake.notify_one();
    }
}

pub fn acquire(cancel: &AtomicBool) -> Option<Permit> {
    let mut n = SEM.active.lock().unwrap_or_else(|e| e.into_inner());
    loop {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        if *n < MAX_CONCURRENT {
            *n += 1;
            return Some(Permit { _private: () });
        }
        let (guard, _) = SEM
            .wake
            .wait_timeout(n, WAIT_SLICE)
            .unwrap_or_else(|e| e.into_inner());
        n = guard;
    }
}
