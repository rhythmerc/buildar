//! The faithful port's log subscriber (Bevy's `LogPlugin` is disabled in its app): events to stderr,
//! which the Quest entry point forwards to logcat. `RUST_LOG` overrides the filter.
use bevy::log::{
    tracing,
    tracing_subscriber::{self as subscriber, Layer, layer::SubscriberExt},
};

pub(crate) fn init() -> Result<(), String> {
    let filter = subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| subscriber::EnvFilter::new("info,wgpu=error,naga=warn"));
    let logs = subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(subscriber::filter::FilterFn::new(|m| m.is_event()))
        .with_filter(filter);
    tracing::subscriber::set_global_default(subscriber::registry().with(logs))
        .map_err(|_| "Cannot install the log subscriber")?;
    let _ = tracing_log::LogTracer::init();
    Ok(())
}
