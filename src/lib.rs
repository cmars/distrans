pub mod app;
pub mod cli;

pub use app::App;
pub use cli::Cli;
use flume::Sender;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

pub fn initialize_stderr_logging() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(
            EnvFilter::builder()
                .with_default_directive("distrans=debug".parse().unwrap())
                .from_env_lossy(),
        )
        .init();
}

pub fn initialize_ui_logging(sender: Sender<Vec<u8>>) {
    let writer = ChannelWriter::new(sender);
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(move || writer.clone()))
        .with(
            EnvFilter::builder()
                .with_default_directive("distrans=debug".parse().unwrap())
                .from_env_lossy(),
        )
        .init();
}

#[derive(Clone)]
struct ChannelWriter {
    sender: Sender<Vec<u8>>,
}

impl ChannelWriter {
    fn new(sender: Sender<Vec<u8>>) -> Self {
        ChannelWriter { sender }
    }
}

impl std::io::Write for ChannelWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.sender.send(buf.to_vec()).map_err(|e| std::io::Error::other(e))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
