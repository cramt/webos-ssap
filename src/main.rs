use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use webos_ssap::{
    Button, CertFingerprint, CertPolicy, ClientKey, Input, MacAddr, PowerState, TurnOff, Tv, Volume,
};

#[derive(Parser)]
#[command(version, about = "Control an LG webOS TV over the network")]
struct Cli {
    #[arg(long, env = "TV_HOST")]
    host: String,
    #[command(subcommand)]
    command: Command,
}

// What every command except `pair` needs to talk to an already paired TV.
#[derive(clap::Args)]
struct Paired {
    /// File holding the client key from `tv pair` (e.g. an opnix secret).
    #[arg(long, env = "TV_KEY_FILE")]
    key_file: PathBuf,
    #[arg(long, env = "TV_CERT_FINGERPRINT")]
    cert_fingerprint: CertFingerprint,
}

#[derive(Subcommand)]
enum Command {
    /// Pop the pairing prompt on the TV and print the key once accepted.
    Pair,
    /// Power state, current input, volume and the input list.
    Status(Paired),
    /// Wake-on-LAN, then wait for the TV to report it's on.
    On {
        #[command(flatten)]
        paired: Paired,
        #[arg(long, env = "TV_MAC")]
        mac: MacAddr,
        /// Switch to this input once it's awake.
        #[arg(long)]
        input: Option<String>,
    },
    /// Turn off, but only if it is actually on.
    Off(Paired),
    /// Switch input, e.g. HDMI_1.
    Input {
        #[command(flatten)]
        paired: Paired,
        id: String,
    },
    /// Set volume, 0 to 100.
    Volume {
        #[command(flatten)]
        paired: Paired,
        level: Volume,
    },
    Mute {
        #[command(flatten)]
        paired: Paired,
        state: Toggle,
    },
    /// Pop a notification on the TV.
    Toast {
        #[command(flatten)]
        paired: Paired,
        message: String,
    },
    /// Picture off with sound still playing, or back on.
    Screen {
        #[command(flatten)]
        paired: Paired,
        state: Toggle,
    },
    /// Press remote buttons in order.
    Press {
        #[command(flatten)]
        paired: Paired,
        #[arg(required = true, value_enum)]
        buttons: Vec<Button>,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum Toggle {
    On,
    Off,
}

impl Paired {
    async fn connect(&self, host: &str) -> anyhow::Result<Tv> {
        let key: ClientKey = std::fs::read_to_string(&self.key_file)
            .with_context(|| format!("reading {}", self.key_file.display()))?
            .parse()?;
        Tv::connect(host, &key, self.cert_fingerprint)
            .await
            .with_context(|| format!("connecting to {host}"))
    }
}

/// Ids typed by a human only become an [`Input`] by matching one the TV has.
async fn find_input(tv: &mut Tv, id: &str) -> anyhow::Result<Input> {
    let inputs = tv.inputs().await?;
    let known = inputs
        .iter()
        .map(|i| i.id().as_str())
        .collect::<Vec<_>>()
        .join(", ");
    inputs
        .into_iter()
        .find(|i| i.id().as_str().eq_ignore_ascii_case(id))
        .with_context(|| format!("no input {id} on this TV, it has {known}"))
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let host = cli.host.as_str();
    match cli.command {
        Command::Pair => {
            eprintln!("accept the prompt on the TV...");
            let (tv, key) = Tv::pair(host, CertPolicy::AcceptAny).await?;
            println!("client key:       {}", key.expose());
            println!("cert fingerprint: {}", tv.fingerprint());
        }
        Command::Status(paired) => {
            let mut tv = paired.connect(host).await?;
            let power = tv.power_state().await?;
            println!("power:  {power:?}");
            let app = tv.foreground_app().await?;
            let inputs = tv.inputs().await?;
            match inputs.iter().find(|i| i.app_id() == &app) {
                Some(input) => println!("input:  {} ({})", input.id().as_str(), input.label()),
                None => println!("app:    {}", app.as_str()),
            }
            let vol = tv.volume().await?;
            let muted = if vol.mute_status { ", muted" } else { "" };
            println!("volume: {}{muted} on {}", vol.volume, vol.sound_output);
            for input in inputs {
                let signal = if input.has_signal() { "signal" } else { "no signal" };
                println!("  {:<7} {:<16} {signal}", input.id().as_str(), input.label());
            }
        }
        Command::On { paired, mac, input } => {
            const WAKE_TIMEOUT: Duration = Duration::from_secs(30);
            let deadline = Instant::now() + WAKE_TIMEOUT;
            let mut tv = loop {
                // Re-sent every round: a magic packet is fire and forget, and
                // a lost one would otherwise cost the whole wait.
                webos_ssap::wake(mac).await.context("sending magic packet")?;
                // Mid-boot any failure just means "not ready yet".
                if let Ok(mut tv) = paired.connect(host).await
                    && let Ok(PowerState::Active) = tv.power_state().await
                {
                    break tv;
                }
                if Instant::now() > deadline {
                    bail!("TV not up after {WAKE_TIMEOUT:?}; is \"Turn on via Wi-Fi\" enabled?");
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            };
            if let Some(id) = input {
                let input = find_input(&mut tv, &id).await?;
                tv.switch_input(&input).await?;
            }
        }
        Command::Off(paired) => {
            if let TurnOff::AlreadyOff(state) = paired.connect(host).await?.turn_off().await? {
                eprintln!("already off ({state:?})");
            }
        }
        Command::Input { paired, id } => {
            let mut tv = paired.connect(host).await?;
            let input = find_input(&mut tv, &id).await?;
            tv.switch_input(&input).await?;
        }
        Command::Volume { paired, level } => paired.connect(host).await?.set_volume(level).await?,
        Command::Mute { paired, state } => {
            let mute = matches!(state, Toggle::On);
            paired.connect(host).await?.set_mute(mute).await?
        }
        Command::Toast { paired, message } => paired.connect(host).await?.toast(&message).await?,
        Command::Screen { paired, state } => {
            let mut tv = paired.connect(host).await?;
            match state {
                Toggle::On => tv.screen_on().await?,
                Toggle::Off => tv.screen_off().await?,
            }
        }
        Command::Press { paired, buttons } => paired.connect(host).await?.press(&buttons).await?,
    }
    Ok(())
}
