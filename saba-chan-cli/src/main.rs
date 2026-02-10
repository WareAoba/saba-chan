mod cli_config;
mod client;
mod gui_config;
mod i18n;
mod module_registry;
mod process;
mod tui;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = client::DaemonClient::new(Some("http://127.0.0.1:57474"));
    tui::run(client).await
}
