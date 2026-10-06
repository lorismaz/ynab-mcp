#[tokio::main]
async fn main() {
    if let Err(error) = ynab_mcp::run().await {
        eprintln!("ynab-mcp failed: {error}");
        std::process::exit(1);
    }
}
