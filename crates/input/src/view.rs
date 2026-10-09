use qa_core::sys_events::EventTime;

/// Native pitch drift state. Frame seconds stay f64 until each float assignment.
#[derive(Clone, Copy, Default)]
pub(crate) struct PitchDrift {
    last_stop: EventTime,
    stopped: bool,
    velocity: f32,
    moving_seconds: f32,
}
impl PitchDrift {
    pub(crate) fn start(&mut self, time: EventTime, speed: f32) {
        if self.last_stop == time {
            return;
        }
        if self.stopped || self.velocity == 0.0 {
            self.velocity = speed;
            self.stopped = false;
            self.moving_seconds = 0.0;
        }
    }
    pub(crate) fn stop(&mut self, time: EventTime) {
        self.last_stop = time;
        self.stopped = true;
        self.velocity = 0.0;
    }
    pub(crate) fn advance(
        &mut self,
        time: EventTime,
        seconds: f64,
        pitch: f32,
        input: DriftInput,
    ) -> f32 {
        if input.disabled || !input.grounded {
            self.moving_seconds = 0.0;
            self.velocity = 0.0;
            return pitch;
        }
        if self.stopped {
            self.moving_seconds = if input.forward.abs() < input.threshold {
                0.0
            } else {
                (f64::from(self.moving_seconds) + seconds) as f32
            };
            if self.moving_seconds > input.delay {
                self.start(time, input.speed);
            }
            return pitch;
        }
        let delta = input.ideal_pitch - pitch;
        if delta == 0.0 {
            self.velocity = 0.0;
            return pitch;
        }
        let mut step = (seconds * f64::from(self.velocity)) as f32;
        self.velocity = (f64::from(self.velocity) + seconds * f64::from(input.speed)) as f32;
        if step > delta.abs() {
            self.velocity = 0.0;
            step = delta.abs();
        }
        if delta > 0.0 {
            pitch + step
        } else {
            pitch - step
        }
    }
}
pub(crate) struct DriftInput {
    pub grounded: bool,
    pub disabled: bool,
    pub forward: f32,
    pub threshold: f32,
    pub ideal_pitch: f32,
    pub speed: f32,
    pub delay: f32,
}
