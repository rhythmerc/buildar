//! Frame pacing to the log, once a second (`FRAMES ...`): how long `xrWaitFrame` held the frame (the headroom), the
//! time from one frame's start to the next, and how many frames the runtime's predicted display time jumped by more
//! than a display period (a refresh this app's frame missed); and at startup the task pools' sizes. (With rendering
//! pipelined, a frame's update and its render are on two threads a frame apart: there's no one span to time.)
use bevy::prelude::*;
use bevy_mod_openxr::render::OxrWaitFrameSystem;
use bevy_mod_openxr::resources::OxrFrameState;
use bevy_mod_xr::session::XrFirst;
use std::sync::Mutex;
use std::time::Instant;

pub(crate) struct FrameTimingPlugin;

impl Plugin for FrameTimingPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(XrFirst, (before_wait.before(OxrWaitFrameSystem), after_wait.after(OxrWaitFrameSystem)))
            .add_systems(First, predicted)
            .add_systems(Startup, pools)
            .add_systems(Last, report);
    }
}

#[derive(Default)]
struct Stats {
    second: Option<Instant>,
    wait_from: Option<Instant>,
    last_start: Option<Instant>,
    last_predicted: Option<i64>,
    frames: u32,
    wait_ms: (f64, f64),
    interval_max_ms: f64,
    long_intervals: u32,
    missed: u32,
    period_ms: f64,
}

/// The task pools' sizes, and the cores the system offers (Bevy sizes the pools from it).
fn pools() {
    use bevy::tasks::{AsyncComputeTaskPool, ComputeTaskPool, IoTaskPool};
    info!(
        "FRAMES pools: cores {:?}; compute {} async {} io {}",
        std::thread::available_parallelism(),
        ComputeTaskPool::get().thread_num(),
        AsyncComputeTaskPool::get().thread_num(),
        IoTaskPool::get().thread_num(),
    );
}


static STATS: Mutex<Option<Stats>> = Mutex::new(None);

fn with(f: impl FnOnce(&mut Stats)) {
    if let Ok(mut s) = STATS.lock() {
        f(s.get_or_insert_with(Stats::default));
    }
}

fn add((sum, max): &mut (f64, f64), ms: f64) {
    *sum += ms;
    *max = max.max(ms);
}

fn before_wait() {
    with(|s| s.wait_from = Some(Instant::now()));
}

fn after_wait() {
    let now = Instant::now();
    with(|s| {
        if let Some(from) = s.wait_from.take() {
            add(&mut s.wait_ms, (now - from).as_secs_f64() * 1000.0);
        }
        if let Some(last) = s.last_start.replace(now) {
            let ms = (now - last).as_secs_f64() * 1000.0;
            s.interval_max_ms = s.interval_max_ms.max(ms);
            if s.period_ms > 0.0 && ms > s.period_ms * 1.5 {
                s.long_intervals += 1;
            }
        }
    });
}

fn predicted(state: Option<Res<OxrFrameState>>) {
    let Some(state) = state else { return };
    let time = state.predicted_display_time.as_nanos();
    let period = state.predicted_display_period.as_nanos();
    with(|s| {
        s.period_ms = period as f64 / 1e6;
        if let Some(last) = s.last_predicted.replace(time)
            && time != last
            && period > 0
            && (time - last) as f64 > period as f64 * 1.5
        {
            s.missed += ((time - last) as f64 / period as f64).round() as u32 - 1;
        }
    });
}

fn report() {
    let now = Instant::now();
    with(|s| {
        s.frames += 1;
        let second = *s.second.get_or_insert(now);
        if (now - second).as_secs_f64() >= 1.0 && s.frames > 0 {
            let n = s.frames as f64;
            info!(
                "FRAMES {} in {:.2}s (period {:.2} ms): wait avg {:.2} max {:.2}; interval max {:.2}, {} over 1.5 periods; predicted time skipped {} refreshes",
                s.frames,
                (now - second).as_secs_f64(),
                s.period_ms,
                s.wait_ms.0 / n,
                s.wait_ms.1,
                s.interval_max_ms,
                s.long_intervals,
                s.missed,
            );
            *s = Stats { last_start: s.last_start, last_predicted: s.last_predicted, period_ms: s.period_ms, second: Some(now), ..Stats::default() };
        }
    });
}
