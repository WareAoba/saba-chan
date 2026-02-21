mod cli_config;
mod client;
mod gui_config;
mod i18n;
mod module_registry;
mod process;
mod tui;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // settings.json에서 IPC 포트를 읽어 사용 (GUI와 포트 설정 일치)
    let base_url = gui_config::get_ipc_base_url();
    let client = client::DaemonClient::new(Some(&base_url));
    tui::run(client).await
}
