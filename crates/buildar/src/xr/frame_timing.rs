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
            .add_systems(Update, systems_once)
            .add_systems(Last, report);
        if let Some(render) = app.get_sub_app_mut(bevy::render::RenderApp) {
            render
                .add_systems(bevy::render::Render, render_from.before(bevy_mod_xr::session::XrRenderSystems::HandleEvents))
                .add_systems(
                    bevy::render::Render,
                    render_pre.after(bevy_mod_xr::session::XrRenderSystems::PreRender).before(bevy::render::RenderSystems::Render),
                )
                .add_systems(
                    bevy::render::Render,
                    render_graph.after(bevy::render::RenderSystems::Render).before(bevy_mod_xr::session::XrRenderSystems::PostRender),
                )
                .add_systems(
                    bevy::render::Render,
                    render_to.after(bevy_mod_xr::session::XrRenderSystems::PostRender).before(bevy::render::RenderSystems::Cleanup),
                );
        }
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
    /// Each frame's update (xrWaitFrame's return to `Last`, main thread) and render (the render schedule's start to
    /// `xrEndFrame`, render thread): sum and most, and how many took over [`SLOW_MS`].
    update_ms: (f64, f64),
    update_slow: u32,
    render_from: Option<Instant>,
    render_ms: (f64, f64),
    render_slow: u32,
    renders: u32,
    /// The render's parts, summed and most: up to after the runtime's pre-render (begin frame, the swapchain image
    /// acquired and waited for), then to after the render graph (and the rest of the render systems), then to after
    /// end frame (the panel layer, release, end frame).
    render_marks: [Option<Instant>; 2],
    render_parts: [(f64, f64); 3],
}

fn render_pre() {
    let now = Instant::now();
    with(|s| s.render_marks[0] = Some(now));
}

fn render_graph() {
    let now = Instant::now();
    with(|s| s.render_marks[1] = Some(now));
}

/// A frame's update or render this long leaves no room in a 72 Hz period for the other's hand-off.
const SLOW_MS: f64 = 11.0;

fn render_from() {
    with(|s| s.render_from = Some(Instant::now()));
}

fn render_to() {
    let now = Instant::now();
    with(|s| {
        if let Some(from) = s.render_from.take() {
            let ms = (now - from).as_secs_f64() * 1000.0;
            add(&mut s.render_ms, ms);
            s.renders += 1;
            if let [Some(a), Some(b)] = s.render_marks {
                for (i, (x, y)) in [(from, a), (a, b), (b, now)].into_iter().enumerate() {
                    add(&mut s.render_parts[i], y.saturating_duration_since(x).as_secs_f64() * 1000.0);
                }
            }
            s.render_marks = [None; 2];
            if ms > SLOW_MS {
                s.render_slow += 1;
            }
        }
    });
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
        if let Some(from) = s.last_start {
            let ms = (now - from).as_secs_f64() * 1000.0;
            add(&mut s.update_ms, ms);
            if ms > SLOW_MS {
                s.update_slow += 1;
            }
        }
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
            info!(
                "FRAMES update avg {:.2} max {:.2} ({} over {SLOW_MS} ms); render avg {:.2} max {:.2} ({} over)",
                s.update_ms.0 / n,
                s.update_ms.1,
                s.update_slow,
                s.render_ms.0 / s.renders.max(1) as f64,
                s.render_ms.1,
                s.render_slow,
            );
            let r = s.renders.max(1) as f64;
            let p = s.render_parts.map(|(sum, max)| format!("{:.2}/{:.2}", sum / r, max));
            info!("FRAMES render parts avg/max: pre-render {} graph {} post-render {}", p[0], p[1], p[2]);
            *s = Stats { last_start: s.last_start, last_predicted: s.last_predicted, period_ms: s.period_ms, second: Some(now), ..Stats::default() };
        }
    });
}

/// Once, a few frames in: how many systems each of the update world's schedules holds (the executor's per-frame
/// cost grows with them, run conditions and all).
fn systems_once(world: &mut World, mut frames: Local<u32>) {
    *frames += 1;
    if *frames != 30 {
        return;
    }
    let Some(schedules) = world.get_resource::<bevy::ecs::schedule::Schedules>() else { return };
    let mut counts: Vec<(String, usize)> =
        schedules.iter().map(|(label, s)| (format!("{label:?}"), s.systems_len())).filter(|(_, n)| *n > 0).collect();
    counts.sort_by(|a, b| b.1.cmp(&a.1));
    let total: usize = counts.iter().map(|c| c.1).sum();
    info!("FRAMES systems: {total} in the update world: {counts:?}");
}
