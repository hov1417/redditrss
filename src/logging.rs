use color_eyre::config::{EyreHook, HookBuilder, PanicHook, Theme};
use tracing::error;
use tracing_error::ErrorLayer;
use tracing_subscriber::fmt;
use tracing_subscriber::fmt::format::FmtSpan;

fn build_error_hooks() -> (PanicHook, EyreHook) {
    HookBuilder::new()
        .theme(Theme::default())
        .display_env_section(false)
        .add_default_filters()
        .into_hooks()
}

fn init_panic_hook() -> eyre::Result<()> {
    let (panic_hook, eyre_hook) = build_error_hooks();

    eyre_hook.install()?;
    std::panic::set_hook(Box::new(move |pi| {
        error!("Panic caught: {}", panic_hook.panic_report(pi));
    }));
    Ok(())
}

fn tracing() {
    use tracing_subscriber::prelude::*;

    tracing_subscriber::registry()
        .with(ErrorLayer::default())
        .with(
            fmt::layer()
                .with_span_events(FmtSpan::ENTER)
                .with_target(false)
                .with_ansi(true)
                .json(),
        )
        .with(
            // let user override RUST_LOG in local run if they want to
            tracing_subscriber::filter::EnvFilter::try_from_default_env()
                .or_else(|_| tracing_subscriber::filter::EnvFilter::try_new("info,shuttle=trace"))
                .unwrap(),
        )
        .init();
}

pub fn init_logging() {
    tracing();
    init_panic_hook().unwrap();
}