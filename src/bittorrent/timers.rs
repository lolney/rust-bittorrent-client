use std::marker::PhantomData;
use std::ops::SubAssign;
use std::thread;
use std::time::Duration;

/// A simple timing wheel
pub struct Timers<'a, S: 'a> {
    timers: Vec<Timer<S>>,
    base: Duration,
    state: &'a mut S,
}

impl<'a, S> Timers<'a, S> {
    pub fn new(base: Duration, state: &mut S) -> Timers<S> {
        Timers {
            timers: Vec::new(),
            base: base,
            state: state,
        }
    }

    pub fn add(
        &mut self,
        duration: Duration,
        task: fn(&mut S) -> bool,
    ) -> Result<(), &'static str> {
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

struct Timer<S> {
    base: Duration,
    remaining: Duration,
    task: fn(&mut S) -> bool,
    phantom: PhantomData<S>,
}

impl<S> Timer<S> {
    fn new(duration: Duration, task: fn(&mut S) -> bool) -> Timer<S> {
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

    #[test]
    fn test_loop() {
        // intervals of 1,2,3
        let max: u64 = 5;
        let func = {
            fn anon(a: &mut u64) -> bool {
                *a += 1;
                *a == 5
            };            anon
        };
        for interval in 1..4 {
            let now = Instant::now();
            let mut a: u64 = 0;
            {
                let mut wheel = Timers::new(Duration::new(interval, 0), &mut a);
                assert!(wheel.add(Duration::new(interval, 0), func).is_ok());
                wheel.run_loop();
            }
            assert!(a == max);
            println!("{}, {}, {:?}", interval, max * interval, now.elapsed());
            assert!(now.elapsed() > Duration::new(max * interval, 0));
            assert!(now.elapsed() < Duration::new(max * interval + 1, 0));
        }
    }

}
