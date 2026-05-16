pub use bgci_core::ratings::EvalArgs;
use tracing_subscriber::EnvFilter;

pub async fn run(args: EvalArgs) -> Result<(), String> {
    init_tracing();
    bgci_core::ratings::run_eval(args).await
}

fn init_tracing() {
    let filter = std::env::var("RUST_LOG")
        .ok()
        .and_then(|raw| EnvFilter::try_new(raw).ok())
        .unwrap_or_else(|| EnvFilter::new("warn,bgci_core::ratings=info"));

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_ansi(true)
        .compact()
        .try_init();
}
