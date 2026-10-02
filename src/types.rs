use serde::{Deserialize, Serialize};

/// What the TV hands out once a pairing prompt is accepted. Whoever holds it
/// gets every permission in the manifest, so it's a secret.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct ClientKey(String);

#[derive(Debug, thiserror::Error)]
#[error("client key is empty")]
pub struct EmptyClientKey;

impl std::str::FromStr for ClientKey {
    type Err = EmptyClientKey;

    /// Trims, since keys usually come from a secret file with a newline.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "" => Err(EmptyClientKey),
            key => Ok(Self(key.to_owned())),
        }
    }
}

impl ClientKey {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for ClientKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ClientKey(..)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum PowerState {
    #[serde(rename = "Active")]
    Active,
    /// Picture off, everything else running.
    #[serde(rename = "Screen Off")]
    ScreenOff,
    #[serde(rename = "Screen Saver")]
    ScreenSaver,
    /// Off as far as a human can tell, but the API still answers.
    #[serde(rename = "Active Standby")]
    ActiveStandby,
    #[serde(rename = "Suspend")]
    Suspend,
    #[serde(rename = "Request Active Standby")]
    RequestActiveStandby,
    #[serde(rename = "Request Power Off")]
    RequestPowerOff,
    #[serde(rename = "Request Screen Saver")]
    RequestScreenSaver,
    #[serde(rename = "Request Suspend")]
    RequestSuspend,
}

impl PowerState {
    /// `system/turnOff` toggles on some firmware, so it must only ever be
    /// sent to a TV that is actually running.
    pub fn is_on(self) -> bool {
        matches!(self, Self::Active | Self::ScreenOff | Self::ScreenSaver)
    }
}

// Neither id can be built by hand: they only ever come from the TV, so a
// request can never name an app or input the TV doesn't have.

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AppId(String);

impl AppId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InputId(String);

impl InputId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Input {
    id: InputId,
    label: String,
    /// Switching to an input launches this app, so it's also how the
    /// foreground app maps back to an input.
    app_id: AppId,
    #[serde(default)]
    hdmi_signal_exist: bool,
}

impl Input {
    pub fn id(&self) -> &InputId {
        &self.id
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn app_id(&self) -> &AppId {
        &self.app_id
    }

    pub fn has_signal(&self) -> bool {
        self.hdmi_signal_exist
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct Volume(u8);

#[derive(Debug, thiserror::Error)]
#[error("volume must be 0..=100, got {0}")]
pub struct InvalidVolume(u8);

impl TryFrom<u8> for Volume {
    type Error = InvalidVolume;

    fn try_from(v: u8) -> Result<Self, Self::Error> {
        if v <= 100 {
            Ok(Self(v))
        } else {
            Err(InvalidVolume(v))
        }
    }
}

impl From<Volume> for u8 {
    fn from(v: Volume) -> u8 {
        v.0
    }
}

impl std::str::FromStr for Volume {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let v: u8 = s.parse().map_err(|e| format!("{e}"))?;
        v.try_into().map_err(|e: InvalidVolume| e.to_string())
    }
}

impl std::fmt::Display for Volume {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VolumeStatus {
    pub volume: Volume,
    pub mute_status: bool,
    pub sound_output: String,
}

/// Remote buttons, sent over the pointer socket rather than SSAP proper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
pub enum Button {
    Home,
    Back,
    Exit,
    Menu,
    Info,
    Up,
    Down,
    Left,
    Right,
    Enter,
    VolumeUp,
    VolumeDown,
    Mute,
    Play,
    Pause,
    Stop,
    Rewind,
    FastForward,
}

impl Button {
    pub(crate) fn wire_name(self) -> &'static str {
        match self {
            Self::Home => "HOME",
            Self::Back => "BACK",
            Self::Exit => "EXIT",
            Self::Menu => "MENU",
            Self::Info => "INFO",
            Self::Up => "UP",
            Self::Down => "DOWN",
            Self::Left => "LEFT",
            Self::Right => "RIGHT",
            Self::Enter => "ENTER",
            Self::VolumeUp => "VOLUMEUP",
            Self::VolumeDown => "VOLUMEDOWN",
            Self::Mute => "MUTE",
            Self::Play => "PLAY",
            Self::Pause => "PAUSE",
            Self::Stop => "STOP",
            Self::Rewind => "REWIND",
            Self::FastForward => "FASTFORWARD",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn power_state_matches_tv_strings() {
        let s: PowerState = serde_json::from_str(r#""Active Standby""#).unwrap();
        assert_eq!(s, PowerState::ActiveStandby);
        assert!(!s.is_on());
    }

    #[test]
    fn volume_rejects_over_100() {
        assert!(serde_json::from_str::<Volume>("101").is_err());
        assert!("100".parse::<Volume>().is_ok());
    }

    #[test]
    fn input_parses_real_payload() {
        // Trimmed from an OLED65B46LA, webOS 9.2.4.
        let input: Input = serde_json::from_str(
            r#"{"id":"HDMI_1","label":"HDMI 1","port":1,"connected":true,
                "appId":"com.webos.app.hdmi1","hdmiPlugIn":true,"hdmiSignalExist":false}"#,
        )
        .unwrap();
        assert_eq!(input.id().as_str(), "HDMI_1");
        assert_eq!(input.app_id().as_str(), "com.webos.app.hdmi1");
        assert!(!input.has_signal());
    }
}
