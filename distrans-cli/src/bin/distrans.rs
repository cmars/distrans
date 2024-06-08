use clap::Parser;
use color_eyre::eyre::Result;

use distrans_cli::{App, Cli};

#[tokio::main]
async fn main() -> Result<()> {
    if let Err(e) = tokio_main().await {
        eprintln!("{} error: Something went wrong", env!("CARGO_PKG_NAME"));
        Err(e)
    } else {
        Ok(())
    }
}

async fn tokio_main() -> Result<()> {
    let cli = Cli::parse();
    let mut app = App::new(cli)?;
    app.run().await?;
    Ok(())
}
