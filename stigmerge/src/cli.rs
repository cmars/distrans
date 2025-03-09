use std::io::IsTerminal;

use anyhow::{Error, Result};
use clap::{arg, Parser, Subcommand};
use sha2::{Digest, Sha256};
use tracing::debug;

#[derive(Parser, Debug)]
#[command(name = "stigmerge")]
#[command(bin_name = "stigmerge")]
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
        return self.no_ui || !std::io::stdout().is_terminal();
    }

    pub fn state_dir(&self) -> Result<String> {
        if let Some(s) = &self.state_dir {
            return Ok(s.to_owned());
        }
        match self.commands {
            Commands::Fetch {
                ref dht_key,
                ref root,
            } => self.state_dir_for(format!("get:{}:{}", dht_key, root)),
            Commands::Seed { ref file } => self.state_dir_for(format!("seed:{}", file.to_owned())),
            _ => Err(Error::msg("invalid command")),
        }
    }

    pub fn state_dir_for(&self, key: String) -> Result<String> {
        let mut key_digest = Sha256::new();
        key_digest.update(&key.as_bytes());
        let key_digest_bytes: [u8; 32] = key_digest.finalize().into();
        let dir_name = hex::encode(key_digest_bytes);
        let data_dir = dirs::state_dir()
            .or(dirs::data_local_dir())
            .ok_or(Error::msg("cannot resolve state dir"))?;
        let state_dir = data_dir
            .join("stigmerge")
            .join(dir_name)
            .into_os_string()
            .into_string()
            .map_err(|os| Error::msg(format!("{:?}", os)))?;
        debug!(state_dir);
        Ok(state_dir)
    }

    pub fn version(&self) -> bool {
        if let Commands::Version = self.commands {
            return true;
        }
        return false;
    }
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    Fetch {
        dht_key: String,
        #[arg(default_value = ".")]
        root: String,
    },
    Seed {
        file: String,
    },
    Version,
}
