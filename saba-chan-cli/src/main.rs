#[tokio::main]
async fn main() -> anyhow::Result<()> {
    saba_chan_cli::run().await
}
