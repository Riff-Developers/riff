use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::future::Future;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::sync::Notify;
use tokio::task::JoinHandle;

const NAP: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Done,
    Later,
}

#[derive(Clone, Copy, Debug)]
pub struct Every {
    pub period: Duration,
    pub retry: Duration,
}

#[derive(Clone)]
pub struct RunLog {
    inner: Arc<LogInner>,
}

struct LogInner {
    file: Option<PathBuf>,
    runs: Mutex<HashMap<String, u64>>,
}

impl RunLog {
    pub fn open(file: PathBuf) -> Self {
        let runs = fs::read_to_string(&file)
            .map(|text| parse(&text))
            .unwrap_or_default();
        Self::with(Some(file), runs)
    }

    #[cfg(test)]
    pub fn in_memory() -> Self {
        Self::with(None, HashMap::new())
    }

    fn with(file: Option<PathBuf>, runs: HashMap<String, u64>) -> Self {
        Self {
            inner: Arc::new(LogInner {
                file,
                runs: Mutex::new(runs),
            }),
        }
    }

    pub fn last_run(&self, task: &str) -> Option<SystemTime> {
        let runs = self.inner.runs.lock().unwrap();
        runs.get(task)
            .map(|&secs| UNIX_EPOCH + Duration::from_secs(secs))
    }

    pub fn record(&self, task: &str, at: SystemTime) {
        let secs = at
            .duration_since(UNIX_EPOCH)
            .map(|at| at.as_secs())
            .unwrap_or_default();
        let mut runs = self.inner.runs.lock().unwrap();
        runs.insert(task.to_string(), secs);
        self.save(&runs);
    }

    pub fn forget(&self, task: &str) {
        let mut runs = self.inner.runs.lock().unwrap();
        if runs.remove(task).is_some() {
            self.save(&runs);
        }
    }

    fn save(&self, runs: &HashMap<String, u64>) {
        let Some(file) = &self.inner.file else {
            return;
        };
        if let Err(e) = write(file, runs) {
            warn!(
                "Couldn't save when tasks last ran to {}: {e}",
                file.display()
            );
        }
    }
}

impl fmt::Debug for RunLog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunLog")
            .field("file", &self.inner.file)
            .finish_non_exhaustive()
    }
}

fn parse(text: &str) -> HashMap<String, u64> {
    text.lines()
        .filter_map(|line| {
            let (name, secs) = line.split_once(' ')?;
            Some((name.to_string(), secs.trim().parse().ok()?))
        })
        .collect()
}

fn write(file: &PathBuf, runs: &HashMap<String, u64>) -> io::Result<()> {
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut names: Vec<_> = runs.keys().collect();
    names.sort();
    let text: String = names
        .into_iter()
        .map(|name| format!("{name} {}\n", runs[name]))
        .collect();
    let temp = file.with_extension("new");
    fs::write(&temp, text)?;
    fs::rename(temp, file)
}

type Clock = Arc<dyn Fn() -> SystemTime + Send + Sync>;

pub struct Scheduler {
    log: RunLog,
    clock: Clock,
    tasks: Vec<Task>,
}

struct Task {
    name: &'static str,
    handle: JoinHandle<()>,
    now: Arc<Notify>,
}

impl Scheduler {
    pub fn new(log: RunLog) -> Self {
        Self::with_clock(log, Arc::new(SystemTime::now))
    }

    fn with_clock(log: RunLog, clock: Clock) -> Self {
        Self {
            log,
            clock,
            tasks: Vec::new(),
        }
    }

    pub fn add<F, Fut>(&mut self, name: &'static str, every: Every, task: F)
    where
        F: FnMut() -> Fut + Send + 'static,
        Fut: Future<Output = Outcome> + Send + 'static,
    {
        debug_assert!(!name.is_empty() && !name.contains(char::is_whitespace));
        let (log, clock) = (self.log.clone(), Arc::clone(&self.clock));
        let now = Arc::new(Notify::new());
        let handle = tokio::spawn(run(name, every, log, clock, Arc::clone(&now), task));
        self.tasks.push(Task { name, handle, now });
    }

    pub fn run_now(&self, name: &str) -> bool {
        let Some(task) = self.tasks.iter().find(|task| task.name == name) else {
            return false;
        };
        task.now.notify_one();
        true
    }
}

impl Drop for Scheduler {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.handle.abort();
        }
    }
}

async fn run<F, Fut>(
    name: &'static str,
    every: Every,
    log: RunLog,
    clock: Clock,
    now: Arc<Notify>,
    mut task: F,
) where
    F: FnMut() -> Fut,
    Fut: Future<Output = Outcome>,
{
    let started = clock();
    let mut due = match log.last_run(name) {
        Some(last) => last.min(started) + every.period,
        None => started,
    };
    loop {
        tokio::select! {
            _ = sleep_until(&clock, due) => {}
            _ = now.notified() => debug!("Running {name} now, as asked"),
        }
        debug!("Running {name}");
        let outcome = task().await;
        let ended = clock();
        due = match outcome {
            Outcome::Done => {
                log.record(name, ended);
                ended + every.period
            }
            Outcome::Later => {
                debug!("{name} runs again in {:?}", every.retry);
                ended + every.retry
            }
        };
    }
}

async fn sleep_until(clock: &Clock, due: SystemTime) {
    while let Ok(left) = due.duration_since(clock()) {
        if left.is_zero() {
            return;
        }
        tokio::time::sleep(left.min(NAP)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    const HOUR: Duration = Duration::from_secs(60 * 60);
    const DAILY: Every = Every {
        period: Duration::from_secs(24 * 60 * 60),
        retry: HOUR,
    };

    fn base() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_700_000_000)
    }

    fn clock() -> Clock {
        let start = tokio::time::Instant::now();
        Arc::new(move || base() + start.elapsed())
    }

    fn counting(scheduler: &mut Scheduler, every: Every, outcomes: Vec<Outcome>) -> Arc<AtomicU32> {
        let runs = Arc::new(AtomicU32::new(0));
        let counted = Arc::clone(&runs);
        scheduler.add("task", every, move || {
            let run = counted.fetch_add(1, Ordering::SeqCst) as usize;
            let outcome = outcomes.get(run).copied().unwrap_or(Outcome::Done);
            async move { outcome }
        });
        runs
    }

    async fn after(wait: Duration) {
        tokio::time::sleep(wait).await;
    }

    #[tokio::test(start_paused = true)]
    async fn runs_at_once_when_it_never_ran_then_on_its_period() {
        let mut scheduler = Scheduler::with_clock(RunLog::in_memory(), clock());
        let runs = counting(&mut scheduler, DAILY, vec![]);
        after(Duration::from_secs(1)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        after(DAILY.period - HOUR).await;
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        after(HOUR).await;
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn waits_out_what_is_left_of_its_period() {
        let log = RunLog::in_memory();
        log.record("task", base() - 20 * HOUR);
        let mut scheduler = Scheduler::with_clock(log.clone(), clock());
        let runs = counting(&mut scheduler, DAILY, vec![]);
        after(4 * HOUR - Duration::from_secs(1)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 0);
        after(Duration::from_secs(2)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        assert_eq!(log.last_run("task"), Some(base() + 4 * HOUR));
    }

    #[tokio::test(start_paused = true)]
    async fn a_run_due_long_ago_happens_at_once() {
        let log = RunLog::in_memory();
        log.record("task", base() - 30 * DAILY.period);
        let mut scheduler = Scheduler::with_clock(log, clock());
        let runs = counting(&mut scheduler, DAILY, vec![]);
        after(Duration::from_secs(1)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_run_after_now_counts_from_now() {
        let log = RunLog::in_memory();
        log.record("task", base() + 30 * DAILY.period);
        let mut scheduler = Scheduler::with_clock(log, clock());
        let runs = counting(&mut scheduler, DAILY, vec![]);
        after(DAILY.period + Duration::from_secs(1)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn later_tries_again_after_the_retry_delay_and_isnt_logged() {
        let log = RunLog::in_memory();
        let mut scheduler = Scheduler::with_clock(log.clone(), clock());
        let runs = counting(&mut scheduler, DAILY, vec![Outcome::Later]);
        after(Duration::from_secs(1)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        assert_eq!(log.last_run("task"), None);
        after(HOUR).await;
        assert_eq!(runs.load(Ordering::SeqCst), 2);
        assert_eq!(log.last_run("task"), Some(base() + HOUR));
    }

    #[tokio::test(start_paused = true)]
    async fn run_now_runs_it_at_once_and_times_the_next_from_then() {
        let mut scheduler = Scheduler::with_clock(RunLog::in_memory(), clock());
        let runs = counting(&mut scheduler, DAILY, vec![]);
        after(HOUR).await;
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        assert!(scheduler.run_now("task"));
        assert!(!scheduler.run_now("another"));
        after(Duration::from_secs(1)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 2);
        after(DAILY.period - HOUR).await;
        assert_eq!(runs.load(Ordering::SeqCst), 2);
        after(HOUR).await;
        assert_eq!(runs.load(Ordering::SeqCst), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn dropping_the_scheduler_stops_its_tasks() {
        let mut scheduler = Scheduler::with_clock(RunLog::in_memory(), clock());
        let runs = counting(&mut scheduler, DAILY, vec![]);
        after(Duration::from_secs(1)).await;
        drop(scheduler);
        after(2 * DAILY.period).await;
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn the_log_carries_over_in_its_file() {
        let dir = std::env::temp_dir().join(format!("riff-run-log-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let file = dir.join("schedule");
        let log = RunLog::open(file.clone());
        assert_eq!(log.last_run("a"), None);
        log.record("a", base());
        log.record("b", base() + HOUR);
        log.forget("b");
        let again = RunLog::open(file);
        assert_eq!(again.last_run("a"), Some(base()));
        assert_eq!(again.last_run("b"), None);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unreadable_lines_are_skipped() {
        let runs = parse("a 10\nnonsense\nb x\nc 30\n");
        assert_eq!(runs.len(), 2);
        assert_eq!(runs["a"], 10);
        assert_eq!(runs["c"], 30);
    }
}
