use std::{cmp::max, collections::VecDeque, fmt::Display, path::PathBuf, sync::Arc, thread};

use color_eyre::eyre::{Error, Result};
use cursive::{
    align::{Align, HAlign},
    event::Event,
    theme::{BorderStyle, Palette, Theme},
    view::{Nameable, Resizable, ScrollStrategy},
    views::{LinearLayout, Panel, ScrollView, TextView, ThemedView},
    CursiveRunnable, Vec2, View, With, XY,
};
use cursive_aligned_view::{Alignable, AlignedView};
use flume::{unbounded, Receiver, Sender};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};
use veilid_core::{RoutingContext, Sequencing, VeilidStateAttachment, VeilidUpdate};

use distrans_peer::{other_err, veilid_config, wait_for_network, Fetcher, Seeder};

use crate::{cli::Commands, initialize_stderr_logging, initialize_ui_logging, Cli};

pub struct App {
    cli: Cli,
}

#[derive(Debug)]
pub enum State {
    Starting,
    WaitingForNetwork {
        attachment: Box<VeilidStateAttachment>,
    },
    Connected {
        attachment: Box<VeilidStateAttachment>,
    },
    IndexingFile {
        file: String,
    },
    SeedingFile {
        file: String,
        dht_key: String,
        sha256: String,
    },
    ResolvingFetch {
        file: String,
        dht_key: String,
    },
    FetchingFile {
        file: String,
        dht_key: String,
        sha256: String,
    },
    FetchComplete,
    Stopping,
    Stopped,
}

impl Display for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            State::Starting => f.write_str("starting"),
            State::WaitingForNetwork { attachment } => f.write_fmt(format_args!(
                "waiting for network: {}",
                format!("{:?}", attachment)
            )),
            State::Connected { attachment: _ } => f.write_str("connected"),
            State::IndexingFile { file } => f.write_fmt(format_args!("indexing file {}", file)),
            State::SeedingFile {
                file,
                dht_key,
                sha256,
            } => f.write_fmt(format_args!(
                "seeding file {} sha256 {} at DHT key {}",
                file, sha256, dht_key
            )),
            State::ResolvingFetch { file: _, dht_key } => {
                f.write_fmt(format_args!("resolving DHT key {} for fetch", dht_key))
            }
            State::FetchingFile {
                file,
                dht_key,
                sha256,
            } => f.write_fmt(format_args!(
                "fetching DHT key {} into {} sha256 {}",
                dht_key, file, sha256
            )),
            State::FetchComplete => f.write_str("fetch complete"),
            State::Stopping => f.write_str("stopping"),
            State::Stopped => f.write_str("stopped"),
        }
    }
}

impl App {
    pub fn new(cli: Cli) -> Result<App> {
        Ok(App { cli })
    }

    pub async fn run(&mut self) -> Result<()> {
        if self.cli.version() {
            println!("distrans {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        let (tx, rx) = unbounded();
        if self.cli.no_ui() {
            self.run_no_ui(tx, rx).await
        } else {
            self.run_ui(tx, rx).await
        }
    }

    async fn run_no_ui(&mut self, tx: Sender<State>, rx: Receiver<State>) -> Result<()> {
        initialize_stderr_logging();
        let logger = tokio::spawn(async move {
            loop {
                match rx.recv_async().await {
                    Ok(state) => {
                        if let State::Stopped = state {
                            return Ok(());
                        }
                    }
                    Err(e) => return Err(e),
                }
            }
        });
        let result = self.run_backend(tx, CancellationToken::new()).await;
        if let Err(e) = logger.await {
            warn!(err = format!("{:?}", e), "failed to join logger task");
        }
        result
    }

    async fn run_ui(&mut self, tx: Sender<State>, rx: Receiver<State>) -> Result<()> {
        let cancel = CancellationToken::new();
        let quit_cancel = cancel.clone();
        let poll_cancel = cancel.clone();
        let (log_tx, log_rx) = unbounded();
        initialize_ui_logging(log_tx);
        let ui_handle = thread::spawn(move || {
            let mut siv = cursive::default();
            siv.set_theme(Self::theme());
            siv.set_autorefresh(true);
            siv.add_global_callback('q', move |s| {
                quit_cancel.cancel();
                s.quit();
            });
            Self::add_panel(&mut siv, log_rx);
            siv.add_global_callback(Event::Refresh, move |s| {
                if poll_cancel.is_cancelled() {
                    s.quit();
                }
                while let Ok(state) = rx.try_recv() {
                    match &state {
                        State::FetchingFile {
                            file,
                            dht_key,
                            sha256,
                        } => {
                            s.call_on_name("mode", |view: &mut TextView| {
                                view.set_content("Fetching")
                            });
                            s.call_on_name("dht_key", |view: &mut TextView| {
                                view.set_content(dht_key)
                            });
                            s.call_on_name("file", |view: &mut TextView| view.set_content(file));
                            s.call_on_name("sha256", |view: &mut TextView| {
                                view.set_content(sha256)
                            });
                        }
                        State::ResolvingFetch { file, dht_key } => {
                            s.call_on_name("mode", |view: &mut TextView| {
                                view.set_content("Resolving")
                            });
                            s.call_on_name("dht_key", |view: &mut TextView| {
                                view.set_content(dht_key)
                            });
                            s.call_on_name("file", |view: &mut TextView| view.set_content(file));
                            s.call_on_name("qr_code", |view: &mut TextView| {
                                view.set_content(cursive::utils::markup::ansi::parse(
                                    qr2term::generate_qr_string(dht_key).unwrap(),
                                ))
                            });
                        }
                        State::FetchComplete => {
                            s.call_on_name("mode", |view: &mut TextView| {
                                view.set_content("Completed")
                            });
                        }
                        State::IndexingFile { file } => {
                            s.call_on_name("mode", |view: &mut TextView| {
                                view.set_content("Indexing")
                            });
                            s.call_on_name("file", |view: &mut TextView| view.set_content(file));
                        }
                        State::SeedingFile {
                            file,
                            dht_key,
                            sha256,
                        } => {
                            s.call_on_name("mode", |view: &mut TextView| {
                                view.set_content("Seeding")
                            });
                            s.call_on_name("file", |view: &mut TextView| view.set_content(file));
                            s.call_on_name("sha256", |view: &mut TextView| {
                                view.set_content(sha256)
                            });
                            s.call_on_name("dht_key", |view: &mut TextView| {
                                view.set_content(dht_key)
                            });
                            s.call_on_name("qr_code", |view: &mut TextView| {
                                view.set_content(cursive::utils::markup::ansi::parse(
                                    qr2term::generate_qr_string(dht_key).unwrap(),
                                ))
                            });
                            s.call_on_name("share_text", |view: &mut TextView| {
                                view.set_content(format!(
                                    "Fetch this file with:\ndistrans get {}",
                                    dht_key
                                ));
                            });
                        }
                        _ => {}
                    }
                    s.call_on_name("status", |view: &mut TextView| {
                        view.set_content(format!("{}", state));
                    });
                }
            });

            // Use buffered backend to prevent refresh flickering. crossterm redraws the entire screen by default.
            let backend_init = || -> std::io::Result<Box<dyn cursive::backend::Backend>> {
                let backend = cursive::backends::crossterm::Backend::init()?;
                let buffered_backend = cursive_buffered_backend::BufferedBackend::new(backend);
                Ok(Box::new(buffered_backend))
            };
            siv.try_run_with(backend_init)?;
            Ok::<(), Error>(())
        });
        let result = self.run_backend(tx, cancel.clone()).await;
        cancel.cancel();
        if let Err(e) = ui_handle.join() {
            warn!(err = format!("{:?}", e), "failed to join ui thread");
        }
        result
    }

    pub async fn run_backend(
        &mut self,
        tx: Sender<State>,
        cancel: CancellationToken,
    ) -> Result<()> {
        tx.send_async(State::Starting).await?;
        let (routing_context, updates) = self.new_routing_context().await?;
        wait_for_network(&updates, |update| {
            match update {
                VeilidUpdate::Attachment(attachment) => {
                    if attachment.public_internet_ready {
                        let _ = tx.send(State::Connected {
                            attachment: attachment.clone(),
                        });
                    } else {
                        let _ = tx.send(State::WaitingForNetwork {
                            attachment: attachment.clone(),
                        });
                    }
                    debug!(attachment = format!("{:?}", attachment));
                }
                VeilidUpdate::Shutdown => {
                    cancel.cancel();
                }
                _ => {}
            };
        })
        .await?;

        let ctrl_c_cancel = cancel.clone();
        let complete_cancel = cancel.clone();
        let canceller = tokio::spawn(async move {
            tokio::select! {
                _ = ctrl_c_cancel.cancelled() => {
                }
                _ = tokio::signal::ctrl_c() => {
                    ctrl_c_cancel.cancel();
                }
            }
        });

        let result = match self.cli.commands {
            Commands::Get {
                ref dht_key,
                ref root,
            } => {
                tx.send_async(State::ResolvingFetch {
                    file: root.to_owned(),
                    dht_key: dht_key.to_owned(),
                })
                .await?;
                let fetcher =
                    Fetcher::from_dht(routing_context.clone(), dht_key.as_str(), root.as_str())
                        .await?;
                tx.send_async(State::FetchingFile {
                    file: fetcher.file(),
                    dht_key: dht_key.to_owned(),
                    sha256: fetcher.digest(),
                })
                .await?;
                fetcher.fetch(cancel).await?;
                tx.send_async(State::FetchComplete).await?;
                Ok(())
            }
            Commands::Post { ref file } => {
                tx.send_async(State::IndexingFile {
                    file: file.to_owned(),
                })
                .await?;
                let seeder = Seeder::from_file(routing_context.clone(), file.as_str()).await?;
                tx.send_async(State::SeedingFile {
                    file: file.to_owned(),
                    dht_key: seeder.dht_key(),
                    sha256: seeder.digest(),
                })
                .await?;
                seeder.seed(cancel, updates).await
            }
            _ => Err(other_err("invalid command")),
        };
        complete_cancel.cancel();
        if let Err(e) = canceller.await {
            warn!(err = format!("{}", e), "failed to join canceller task");
        }

        if let Err(e) = tx.send_async(State::Stopping).await {
            warn!(err = format!("{}", e), "failed to send state");
        }
        routing_context.api().shutdown().await;
        if let Err(e) = tx.send_async(State::Stopped).await {
            warn!(err = format!("{}", e), "failed to send state");
        }

        Ok(result?)
    }

    async fn new_routing_context(&self) -> Result<(RoutingContext, Receiver<VeilidUpdate>)> {
        let state_path_buf = PathBuf::from(self.cli.state_dir()?);

        // Veilid API state channel
        let (node_sender, updates): (Sender<VeilidUpdate>, Receiver<VeilidUpdate>) = unbounded();

        // Start up Veilid core
        let update_callback = Arc::new(move |change: VeilidUpdate| {
            let _ = node_sender.send(change);
        });
        let config_state_path = Arc::new(state_path_buf);
        let config_callback = Arc::new(move |key| {
            veilid_config::callback(config_state_path.to_str().unwrap().to_string(), key)
        });

        let api: veilid_core::VeilidAPI =
            veilid_core::api_startup(update_callback, config_callback).await?;
        api.attach().await?;

        let routing_context = api
            .routing_context()?
            .with_sequencing(Sequencing::EnsureOrdered)
            .with_default_safety()?;
        Ok((routing_context, updates))
    }

    fn theme() -> Theme {
        Theme {
            shadow: false,
            borders: BorderStyle::Simple,
            palette: Palette::retro().with(|palette| {
                use cursive::theme::BaseColor::*;
                use cursive::theme::PaletteColor::*;

                palette[Background] = Black.dark();
                palette[Shadow] = Black.dark();
                palette[View] = Black.dark();
                palette[Primary] = Cyan.light();
                palette[Secondary] = Cyan.light();
                palette[Tertiary] = Green.light();
                palette[TitlePrimary] = Magenta.light();
                palette[TitleSecondary] = Magenta.light();
                palette[Highlight] = White.light();
                palette[HighlightInactive] = White.dark();
                palette[HighlightText] = Black.dark();
            }),
        }
    }

    fn add_panel(s: &mut CursiveRunnable, log_rx: Receiver<Vec<u8>>) {
        s.add_layer(
            Panel::new(
                LinearLayout::vertical()
                    .child(AlignedView::with_top_center(
                        LinearLayout::horizontal()
                            .child(ThemedView::new(
                                Self::theme().with(|t| {
                                    use cursive::theme::BaseColor::*;
                                    use cursive::theme::PaletteColor::*;
                                    t.palette[Primary] = Cyan.dark();
                                }),
                                LinearLayout::vertical()
                                    .child(
                                        TextView::new("Connecting")
                                            .h_align(HAlign::Left)
                                            .with_name("mode")
                                            .min_width(10),
                                    )
                                    .child(
                                        TextView::new("DHT Key")
                                            .h_align(HAlign::Left)
                                            .min_width(10),
                                    )
                                    .child(
                                        TextView::new("SHA256").h_align(HAlign::Left).min_width(10),
                                    ),
                            ))
                            .child(
                                LinearLayout::vertical()
                                    .child(TextView::new("").with_name("file").min_width(48))
                                    .child(TextView::new("").with_name("dht_key").min_width(48))
                                    .child(TextView::new("").with_name("sha256").min_width(64)),
                            ),
                    ))
                    .child(
                        AlignedView::with_center(
                            LinearLayout::vertical()
                                .child(
                                    TextView::new("")
                                        .align(Align::center())
                                        .with_name("qr_code")
                                        .align_center(),
                                )
                                .child(
                                    TextView::new("")
                                        .align(Align::center())
                                        .with_name("share_text")
                                        .align_center(),
                                ),
                        )
                        .full_screen(),
                    )
                    .child(
                        ScrollView::new(BufferView::new(10, log_rx))
                            .show_scrollbars(false)
                            .scroll_x(true)
                            .scroll_strategy(ScrollStrategy::StickToBottom)
                            .max_height(5),
                    )
                    .child(AlignedView::with_bottom_center(TextView::new(
                        "Shift+Click to select text. Press 'q' to quit",
                    )))
                    .full_screen(),
            )
            .title(format!("distrans {}", env!("CARGO_PKG_VERSION")))
            .title_position(HAlign::Center)
            .full_screen(),
        )
    }
}

struct BufferView {
    buffer: VecDeque<String>,
    rx: Receiver<Vec<u8>>,
}

impl BufferView {
    fn new(size: usize, rx: Receiver<Vec<u8>>) -> Self {
        let mut buffer = VecDeque::new();
        buffer.resize(size, String::new());
        BufferView { buffer, rx }
    }

    fn update(&mut self) -> Vec2 {
        while let Ok(line_bytes) = self.rx.try_recv() {
            let line = String::from_utf8_lossy(&line_bytes);
            self.buffer.push_back(line.into());
            self.buffer.pop_front();
        }

        let mut width = 1usize;
        let mut height = 0usize;
        for line in self.buffer.iter() {
            width = max(width, line.len());
            if !line.is_empty() {
                height += 1;
            }
        }
        XY::new(width, height)
    }
}

impl View for BufferView {
    fn layout(&mut self, _: Vec2) {
        self.update();
    }

    fn required_size(&mut self, _: Vec2) -> Vec2 {
        self.update()
    }

    fn draw(&self, printer: &cursive::Printer) {
        for (i, line) in self.buffer.iter().rev().take(printer.size.y).enumerate() {
            printer.print_styled(
                (0, printer.size.y - 1 - i),
                &cursive::utils::markup::ansi::parse(line),
            );
        }
    }
}
