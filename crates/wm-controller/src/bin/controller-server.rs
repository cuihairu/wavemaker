//! Controller server binary (WM-0104): binds HTTP :7600 + gRPC control service.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).map(std::path::PathBuf::from);
    let config = wm_controller::ControllerConfig::load(path.as_deref())?;
    wm_controller::run(config).await
}
