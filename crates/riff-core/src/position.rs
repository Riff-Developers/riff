use std::time::Instant;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Position {
    ms: u32,
    since: Option<Instant>,
}

impl Position {
    pub fn running_since(ms: u32, since: Instant) -> Self {
        Self {
            ms,
            since: Some(since),
        }
    }

    pub fn now(&self) -> u32 {
        let elapsed = self
            .since
            .map_or(0, |since| since.elapsed().as_millis() as u32);
        self.ms.saturating_add(elapsed)
    }

    pub fn is_running(&self) -> bool {
        self.since.is_some()
    }

    pub fn set(&mut self, ms: u32, running: bool) {
        self.ms = ms;
        self.since = running.then(Instant::now);
    }

    pub fn run(&mut self, running: bool) {
        self.set(self.now(), running);
    }
}
