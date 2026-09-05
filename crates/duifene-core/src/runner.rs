use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration as StdDuration, Instant};

use chrono::{Local, NaiveDateTime};

use crate::engine::{Engine, Event};

pub struct Runner {
    engine: Engine,
    interval: StdDuration,
}

impl Runner {
    pub fn new(engine: Engine, interval: StdDuration) -> Self {
        Runner { engine, interval }
    }

    pub fn run(&mut self, emit: &mut dyn FnMut(Event)) {
        self.run_until(emit, None);
    }

    pub fn run_until(&mut self, emit: &mut dyn FnMut(Event), deadline: Option<Instant>) {
        loop {
            if !self.engine.running() {
                return;
            }
            if let Some(deadline) = deadline
                && Instant::now() >= deadline
            {
                return;
            }
            let now = Local::now().naive_local();
            for event in self.engine.tick(now) {
                emit(event);
            }
            thread::sleep(self.next_sleep(Local::now().naive_local()));
        }
    }

    pub fn run_with_stop(&mut self, emit: &mut dyn FnMut(Event), stop: &AtomicBool) {
        loop {
            if stop.load(Ordering::Relaxed) || !self.engine.running() {
                return;
            }
            let now = Local::now().naive_local();
            for event in self.engine.tick(now) {
                emit(event);
            }
            thread::sleep(self.next_sleep(Local::now().naive_local()));
        }
    }

    fn next_sleep(&self, now: NaiveDateTime) -> StdDuration {
        match self.engine.earliest_due(now) {
            Some(remaining) => remaining.min(self.interval),
            None => self.interval,
        }
    }
}
