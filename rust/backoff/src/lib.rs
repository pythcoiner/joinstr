use std::{
    thread::{self, yield_now},
    time::Duration,
};

use rand::{rng, Rng};

pub struct Backoff {
    current_step: u32,
    max_sleep: Duration,
}

impl Backoff {
    pub fn new_us(max_sleep: u64) -> Self {
        let max_sleep = Duration::from_micros(max_sleep);
        Self {
            current_step: 0,
            max_sleep,
        }
    }

    pub fn new_ms(max_sleep: u64) -> Self {
        let max_sleep = Duration::from_millis(max_sleep);
        Self {
            current_step: 0,
            max_sleep,
        }
    }

    /// Resets the backoff when work is done
    pub fn reset(&mut self) {
        self.current_step = 0;
    }

    /// Applies the next backoff strategy (yield or sleep)
    pub fn snooze(&mut self) {
        if self.current_step < 10 {
            yield_now();
        } else {
            // exponential: base sleep = 2^(n - 10) * 10μs
            let base = 1u64 << (self.current_step - 10).min(10);
            let sleep_micros = base * 10;
            let jitter: u64 = rng().random_range(0..sleep_micros / 2);
            let total_sleep = Duration::from_micros(
                (sleep_micros + jitter).min(self.max_sleep.as_micros() as u64),
            );

            thread::sleep(total_sleep);
        }

        self.current_step = self.current_step.saturating_add(1);
    }
}
