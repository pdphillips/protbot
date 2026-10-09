mod app;
mod bridge;
mod cache;
mod calendar;
mod cli;
mod mail;
mod mcp;
mod policy;
mod redact;
mod settings;
mod tools;
mod watcher;

pub use app::{App, Config};
pub use bridge::{check_loopback, RecordingMailer};
pub use cli::{CallRecord, CliError, CliOutput, MapRunner};
pub use policy::AllowList;
pub use settings::Settings;
pub use tools::call_tool;

pub fn run() -> Result<(), String> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|err| err.to_string())?
        .block_on(run_async())
}

async fn run_async() -> Result<(), String> {
    let app = App::from_env()?;
    if !app.cfg().skip_bridge_probe {
        app.probe_bridge().await?;
    }
    let notices = if app.cfg().disable_watcher {
        None
    } else {
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        let watch = app.clone();
        tokio::spawn(async move { tools::watch_loop(watch, tx).await });
        Some(rx)
    };
    eprintln!("protbot ready");
    mcp::serve(app, notices).await
}
