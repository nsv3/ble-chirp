use std::time::Duration;
use tokio::time::{Instant, sleep};

pub struct RateLimiter {
    capacity: f64,
    tokens: f64,
    rate: f64,
    last: Instant,
}

impl RateLimiter {
    pub fn new(rate: f64) -> Self {
        let cap = rate.max(1.0);
        Self {
            capacity: cap,
            tokens: cap,
            rate,
            last: Instant::now(),
        }
    }

    pub async fn acquire(&mut self) {
        loop {
            self.refill();
            if self.tokens >= 1.0 {
                self.tokens -= 1.0;
                return;
            }
            let needed = (1.0 - self.tokens) / self.rate;
            sleep(Duration::from_secs_f64(needed)).await;
        }
    }

    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.rate).min(self.capacity);
        self.last = now;
    }
}
