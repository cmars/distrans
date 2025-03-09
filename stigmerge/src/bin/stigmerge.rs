#![recursion_limit = "256"]

use anyhow::Result;
use clap::Parser;

use stigmerge::{App, Cli};

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
