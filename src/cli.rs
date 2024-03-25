use std::io::IsTerminal;

use clap::{arg, Parser, Subcommand};

use color_eyre::eyre::Result;
use distrans_peer::{other_err, Error};
use sha2::{Digest, Sha256};
use tracing::debug;

#[derive(Parser, Debug)]
#[command(name = "distrans")]
#[command(bin_name = "distrans")]
pub struct Cli {
    #[arg(long, env)]
    pub no_ui: bool,

    #[arg(long, env)]
    pub state_dir: Option<String>,

    #[command(subcommand)]
    pub commands: Commands,
}

impl Cli {
    pub fn no_ui(&self) -> bool {
        return self.no_ui || !std::io::stdout().is_terminal()
    }

    pub fn state_dir(&self) -> Result<String> {
        if let Some(s) = &self.state_dir {
            return Ok(s.to_owned());
        }
        match self.commands {
            Commands::Get {
                ref dht_key,
                ref root,
            } => self.state_dir_for(format!("get:{}:{}", dht_key, root)),
            Commands::Post { ref file } => self.state_dir_for(format!("post:{}", file.to_owned())),
        }
    }

    pub fn state_dir_for(&self, key: String) -> Result<String> {
        let mut key_digest = Sha256::new();
        key_digest.update(&key.as_bytes());
        let key_digest_bytes: [u8; 32] = key_digest.finalize().into();
        let dir_name = hex::encode(key_digest_bytes);
        let data_dir =
            dirs::state_dir().ok_or(Error::Other("cannot resolve state dir".to_string()))?;
        let state_dir = data_dir
            .join("distrans")
            .join(dir_name)
            .into_os_string()
            .into_string()
            .map_err(|os| other_err(format!("{:?}", os)))?;
        debug!(state_dir);
        Ok(state_dir)
    }
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    Get {
        dht_key: String,
        #[arg(default_value = ".")]
        root: String,
    },
    Post {
        file: String,
    },
}
