//! Adaptive parallel job runner.

use super::TrackResult;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

pub const MAX_WORKERS: usize = 6;
pub const MAX_RETRIES: u8 = 2;

const GROW_AFTER_SUCCESSES: usize = 3;
const THROTTLE_COOLDOWN: Duration = Duration::from_secs(10);
#[cfg(not(test))]
const BACKOFF_STEP: Duration = Duration::from_secs(3);
#[cfg(test)]
const BACKOFF_STEP: Duration = Duration::from_millis(10);
const IDLE_POLL: Duration = Duration::from_millis(250);

pub fn auto_worker_count() -> usize {
    thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        .clamp(2, MAX_WORKERS)
}

#[derive(Debug, Clone)]
pub struct Job {
    pub id: u64,
    pub url: String,
    pub label: String,
    pub attempts: u8,
    pub fallback_attempt: u8,
    pub spotify: Option<crate::backend::spotify::SpotifyMeta>,
    pub deezer: Option<crate::backend::deezer::DeezerMeta>,
    pub folder: Option<String>,
}

impl Job {
    pub fn new(id: u64, url: String, label: String) -> Self {
        Self {
            id,
            url,
            label,
            attempts: 0,
            fallback_attempt: 0,
            spotify: None,
            deezer: None,
            folder: None,
        }
    }
    pub fn can_retry(&self) -> bool {
        self.attempts < MAX_RETRIES
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Finished(TrackResult),
    RateLimited,
    RetryWithFallback,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Tally {
    pub succeeded: usize,
    pub failed: usize,
    pub unavailable: usize,
    pub drm: usize,
    pub skipped: usize,
    pub stopped: bool,
}

impl Tally {
    fn record(&mut self, result: TrackResult) {
        match result {
            TrackResult::Ok => self.succeeded += 1,
            TrackResult::Drm => self.drm += 1,
            TrackResult::Skipped => self.skipped += 1,
            TrackResult::Unavailable => self.unavailable += 1,
            TrackResult::Error => self.failed += 1,
            TrackResult::Stopped => self.stopped = true,
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

struct AdaptiveLimit {
    max: usize,
    current: AtomicUsize,
    streak: AtomicUsize,
    last_cut: Mutex<Option<Instant>>,
}

impl AdaptiveLimit {
    fn new(max: usize) -> Self {
        Self {
            max,
            current: AtomicUsize::new(max),
            streak: AtomicUsize::new(0),
            last_cut: Mutex::new(None),
        }
    }
    fn current(&self) -> usize {
        self.current.load(Ordering::Relaxed)
    }

    fn on_success(&self) {
        let streak = self.streak.fetch_add(1, Ordering::Relaxed) + 1;
        if streak < GROW_AFTER_SUCCESSES {
            return;
        }
        let consumed = self
            .streak
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                if count >= GROW_AFTER_SUCCESSES {
                    Some(count - GROW_AFTER_SUCCESSES)
                } else {
                    None
                }
            })
            .is_ok();
        if consumed {
            let _ = self.current.try_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                (current < self.max).then_some(current + 1)
            });
        }
    }

    fn on_rate_limited(&self) -> Option<usize> {
        self.streak.store(0, Ordering::Relaxed);
        let mut last = lock(&self.last_cut);
        if last.is_some_and(|t| t.elapsed() < THROTTLE_COOLDOWN) {
            return None;
        }
        *last = Some(Instant::now());
        let cur = self.current();
        let next = (cur / 2).max(1);
        self.current.store(next, Ordering::Relaxed);
        (next != cur).then_some(next)
    }
}

struct State {
    queue: VecDeque<Job>,
    active: usize,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    limit: AdaptiveLimit,
    tally: Mutex<Tally>,
}

pub fn run_all<F, N>(
    jobs: Vec<Job>,
    workers: usize,
    stop: &AtomicBool,
    notify: N,
    run: F,
) -> Tally
where
    F: Fn(&Job) -> Outcome + Sync,
    N: Fn(String) + Sync,
{
    let threads = workers.clamp(1, MAX_WORKERS).min(jobs.len().max(1));
    let shared = Shared {
        state: Mutex::new(State {
            queue: jobs.into(),
            active: 0,
        }),
        wake: Condvar::new(),
        limit: AdaptiveLimit::new(threads),
        tally: Mutex::new(Tally::default()),
    };
    thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| worker(&shared, stop, &notify, &run));
        }
    });
    let mut tally = *lock(&shared.tally);
    if stop.load(Ordering::Relaxed) {
        tally.stopped = true;
    }
    tally
}

fn worker<F, N>(shared: &Shared, stop: &AtomicBool, notify: &N, run: &F)
where
    F: Fn(&Job) -> Outcome,
    N: Fn(String),
{
    while let Some(mut job) = next_job(shared, stop) {
        let mut retry = false;
        match run(&job) {
            Outcome::Finished(result) => {
                lock(&shared.tally).record(result);
                if result == TrackResult::Ok {
                    shared.limit.on_success();
                }
            }
            Outcome::RateLimited if job.can_retry() => {
                if let Some(limit) = shared.limit.on_rate_limited() {
                    notify(format!(
                        "Server is rate-limiting us; reducing to {limit} parallel download(s)."
                    ));
                }
                job.attempts += 1;
                retry = true;
                sleep_unless_stopped(BACKOFF_STEP * u32::from(job.attempts), stop);
            }
            Outcome::RateLimited => lock(&shared.tally).record(TrackResult::Error),
            Outcome::RetryWithFallback if job.can_retry() => {
                job.attempts += 1;
                job.fallback_attempt += 1;
                retry = true;
                notify("Retrying a YouTube download with an alternate player client.".to_string());
                sleep_unless_stopped(BACKOFF_STEP * u32::from(job.attempts), stop);
            }
            Outcome::RetryWithFallback => lock(&shared.tally).record(TrackResult::Error),
        }
        let mut state = lock(&shared.state);
        state.active -= 1;
        if retry && !stop.load(Ordering::Relaxed) {
            state.queue.push_front(job);
        }
        drop(state);
        shared.wake.notify_all();
    }
}

fn next_job(shared: &Shared, stop: &AtomicBool) -> Option<Job> {
    let mut state = lock(&shared.state);
    loop {
        if stop.load(Ordering::Relaxed) {
            state.queue.clear();
        }
        if state.active < shared.limit.current() {
            if let Some(job) = state.queue.pop_front() {
                state.active += 1;
                return Some(job);
            }
        }
        if state.queue.is_empty() && state.active == 0 {
            return None;
        }
        state = shared
            .wake
            .wait_timeout(state, IDLE_POLL)
            .unwrap_or_else(|e| e.into_inner())
            .0;
    }
}

fn sleep_unless_stopped(total: Duration, stop: &AtomicBool) {
    let end = Instant::now() + total;
    while Instant::now() < end && !stop.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(100));
    }
}