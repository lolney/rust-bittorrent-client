use std::marker::PhantomData;
use std::ops::Deref;
use std::ops::SubAssign;
use std::thread;
use std::time::Duration;

/// A simple timing wheel
struct Timers<'a, S: 'a, F>
where
    F: Fn(&mut S) -> bool,
{
    timers: Vec<Timer<S, F>>,
    base: Duration,
    state: &'a mut S,
}

// TODO: macro to replace run_loop
// Input: (function and frequency) pairs
// Generate code
/*
loop {
    thread::sleep(self.base);
    if timer1.tick() {
        self.x()
    }
}
*/
impl<'a, S, F> Timers<'a, S, F>
where
    F: Fn(&mut S) -> bool,
{
    fn new(base: Duration, state: &mut S) -> Timers<S, F> {
        Timers {
            timers: Vec::new(),
            base: base,
            state: state,
        }
    }

    pub fn add(&mut self, duration: Duration, task: F) -> Result<(), &'static str> {
        if duration.as_secs() % self.base.as_secs() != 0 {
            Err("Duration must be a multiple of base")
        } else {
            self.timers.push(Timer::new(duration, task));
            Ok(())
        }
    }

    pub fn run_loop(&mut self) {
        loop {
            thread::sleep(self.base);
            if self.tick() {
                break;
            }
        }
    }

    fn tick(&mut self) -> bool {
        for timer in self.timers.iter_mut() {
            if timer.tick(self.base, self.state) {
                return true;
            }
        }
        return false;
    }
}

struct Timer<S, F>
where
    F: Fn(&mut S) -> bool,
{
    base: Duration,
    remaining: Duration,
    task: F,
    phantom: PhantomData<S>,
}

impl<S, F> Timer<S, F>
where
    for<'a> F: Fn(&'a mut S) -> bool,
{
    fn new(duration: Duration, task: F) -> Timer<S, F> {
        Timer {
            base: duration,
            remaining: duration,
            task: task,
            phantom: PhantomData,
        }
    }

    pub fn tick(&mut self, delta: Duration, state: &mut S) -> bool {
        self.remaining.sub_assign(delta);

        if self.remaining <= Duration::from_secs(0) {
            self.remaining = self.base;
            return (self.task)(state);
        } else {
            return false;
        }
    }
}

#[cfg(test)]
mod tests {

    use bittorrent::timers::*;
    use std::time::{Duration, Instant};

    fn test_loop() {
        // intervals of 1,2,3
        let max: u64 = 10;
        for interval in 1..4 {
            let now = Instant::now();
            let mut a: u64 = 0;
            {
                let mut wheel = Timers::new(Duration::new(interval, 0), &mut a);
                assert!(
                    wheel
                        .add(Duration::new(interval, 0), |a| {
                            *a += interval;
                            if *a == max {
                                false
                            } else {
                                true
                            }
                        })
                        .is_ok()
                );
                wheel.run_loop();
            }
            assert!(a >= max);
            assert!(now.elapsed() > Duration::new(max, 0));
            assert!(now.elapsed() < Duration::new(max + 1, 0));
        }
    }

}
