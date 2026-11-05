//! The time the interface goes by. In tests it stands still until the test moves it on, so
//! debounces, timeouts and animations end exactly when a test says.

#[cfg(test)]
pub use frozen::{Instant, advance};
#[cfg(not(test))]
pub use std::time::Instant;

#[cfg(test)]
mod frozen {
    use std::{
        cell::Cell,
        ops::{Add, AddAssign, Sub, SubAssign},
        time::Duration,
    };

    // Per thread, as each test runs on a runtime of its own thread.
    thread_local! {
        static START: std::time::Instant = std::time::Instant::now();
        static AHEAD: Cell<Duration> = const { Cell::new(Duration::ZERO) };
    }

    /// Moves this thread's clock on.
    pub fn advance(by: Duration) {
        AHEAD.set(AHEAD.get() + by);
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct Instant(std::time::Instant);

    impl Instant {
        pub fn now() -> Self {
            Self(START.with(|start| *start) + AHEAD.get())
        }

        pub fn elapsed(&self) -> Duration {
            Self::now() - *self
        }
    }

    impl Add<Duration> for Instant {
        type Output = Self;
        fn add(self, duration: Duration) -> Self {
            Self(self.0 + duration)
        }
    }

    impl AddAssign<Duration> for Instant {
        fn add_assign(&mut self, duration: Duration) {
            self.0 += duration;
        }
    }

    impl Sub<Duration> for Instant {
        type Output = Self;
        fn sub(self, duration: Duration) -> Self {
            Self(self.0 - duration)
        }
    }

    impl SubAssign<Duration> for Instant {
        fn sub_assign(&mut self, duration: Duration) {
            self.0 -= duration;
        }
    }

    impl Sub for Instant {
        type Output = Duration;
        fn sub(self, earlier: Self) -> Duration {
            self.0.saturating_duration_since(earlier.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn stands_still_until_moved() {
        let start = Instant::now();
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(start.elapsed(), Duration::ZERO);
        advance(Duration::from_millis(250));
        assert_eq!(start.elapsed(), Duration::from_millis(250));
        assert_eq!(Instant::now(), start + Duration::from_millis(250));
    }
}
