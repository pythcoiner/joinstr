use rand::{rng, Rng};
use std::{
    thread,
    time::{Duration, SystemTime},
};

/// return the current timestamp
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("unix timestamp must not fail")
        .as_secs()
}

/// wait for a random delay (200ms-5sec.)
pub fn rand_delay() {
    let mut rng = rng();
    let millis: u64 = rng.random_range(200..5000);
    let delay = Duration::from_millis(millis);
    thread::sleep(delay);
}
