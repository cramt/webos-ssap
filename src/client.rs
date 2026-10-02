use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::tls::{self, CertFingerprint, CertPolicy};
use crate::types::{AppId, Button, ClientKey, Input, PowerState, Volume, VolumeStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnOff {
    Sent,
    /// Nothing sent, since on some firmware `turnOff` wakes a TV in standby.
    AlreadyOff(PowerState),
}

/// TLS only. Plain ws on 3000 would put the client key on the LAN in clear.
const PORT: u16 = 3001;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Long enough for someone to find the remote.
const PAIRING_TIMEOUT: Duration = Duration::from_secs(60);

/// Mirrors aiowebostv's manifest exactly. The TV ties a client key to the
/// permission set it was granted under, so asking for a different set means
/// another prompt on the TV, and keys paired via Home Assistant keep working.
const PERMISSIONS: &[&str] = &[
    "APP_TO_APP",
    "CLOSE",
    "CONTROL_AUDIO",
    "CONTROL_DISPLAY",
    "CONTROL_INPUT_JOYSTICK",
    "CONTROL_INPUT_MEDIA_PLAYBACK",
    "CONTROL_INPUT_MEDIA_RECORDING",
    "CONTROL_INPUT_TEXT",
    "CONTROL_INPUT_TV",
    "CONTROL_MOUSE_AND_KEYBOARD",
    "CONTROL_POWER",
    "CONTROL_TV_SCREEN",
    "LAUNCH",
    "LAUNCH_WEBAPP",
    "READ_APP_STATUS",
    "READ_COUNTRY_INFO",
    "READ_CURRENT_CHANNEL",
    "READ_INPUT_DEVICE_LIST",
    "READ_INSTALLED_APPS",
    "READ_LGE_SDX",
    "READ_LGE_TV_INPUT_EVENTS",
    "READ_NETWORK_STATE",
    "READ_NOTIFICATIONS",
    "READ_POWER_STATE",
    "READ_RUNNING_APPS",
    "READ_SETTINGS",
    "READ_TV_CHANNEL_LIST",
    "READ_TV_CURRENT_TIME",
    "READ_UPDATE_INFO",
    "SEARCH",
    "TEST_OPEN",
    "TEST_PROTECTED",
    "TEST_SECURE",
    "UPDATE_FROM_REMOTE_APP",
    "WRITE_NOTIFICATION_ALERT",
    "WRITE_NOTIFICATION_TOAST",
    "WRITE_SETTINGS",
];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("websocket: {0}")]
    Ws(#[from] tokio_tungstenite::tungstenite::Error),
    #[error("TV closed the connection")]
    Closed,
    #[error("no reply from the TV within {0:?}")]
    Timeout(Duration),
    #[error("pairing rejected: {0}")]
    PairingRejected(String),
    #[error("{uri}: {error}")]
    Tv { uri: String, error: String },
    #[error("unexpected payload from {uri}: {source}")]
    Payload { uri: String, source: serde_json::Error },
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Incoming {
    Response {
        id: String,
        #[serde(default)]
        payload: Value,
    },
    Registered {
        id: String,
        payload: Registered,
    },
    Error {
        #[serde(default)]
        id: Option<String>,
        error: String,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct Registered {
    #[serde(rename = "client-key")]
    client_key: ClientKey,
}

pub struct Tv {
    host: String,
    fingerprint: CertFingerprint,
    ws: Socket,
    next_id: u64,
}

impl Tv {
    /// Pops the "allow this device" prompt on the TV and waits for someone to
    /// accept it with the remote.
    pub async fn pair(host: &str, policy: CertPolicy) -> Result<(Self, ClientKey)> {
        let mut tv = Self::open(host, policy).await?;
        let key = tv.register(None).await?;
        Ok((tv, key))
    }

    pub async fn connect(host: &str, key: &ClientKey, pin: CertFingerprint) -> Result<Self> {
        let mut tv = Self::open(host, CertPolicy::Pinned(pin)).await?;
        tv.register(Some(key)).await?;
        Ok(tv)
    }

    /// The cert the TV presented. Pin it after pairing; every later
    /// connection, including the pointer socket, is held to it.
    pub fn fingerprint(&self) -> CertFingerprint {
        self.fingerprint
    }

    async fn open(host: &str, policy: CertPolicy) -> Result<Self> {
        let ws = open_socket(&format!("wss://{host}:{PORT}"), policy).await?;
        let fingerprint = match ws.get_ref() {
            MaybeTlsStream::Rustls(tls) => tls
                .get_ref()
                .1
                .peer_certificates()
                .and_then(|certs| certs.first())
                .map(CertFingerprint::of)
                .expect("rustls handshake completed without a peer cert"),
            _ => unreachable!("wss:// always negotiates TLS"),
        };
        Ok(Self {
            host: host.to_owned(),
            fingerprint,
            ws,
            next_id: 0,
        })
    }

    async fn register(&mut self, key: Option<&ClientKey>) -> Result<ClientKey> {
        let mut payload = json!({
            "forcePairing": false,
            "pairingType": "PROMPT",
            "manifest": {
                "appVersion": "1.1",
                "manifestVersion": 1,
                "permissions": PERMISSIONS,
            },
        });
        if let Some(key) = key {
            payload["client-key"] = json!(key);
        }
        self.send(json!({ "id": "register", "type": "register", "payload": payload }))
            .await?;

        // A known key comes straight back as `registered`; an unknown one
        // first gets a `response` saying the prompt is up.
        let timeout = if key.is_some() {
            REQUEST_TIMEOUT
        } else {
            PAIRING_TIMEOUT
        };
        tokio::time::timeout(timeout, async {
            loop {
                match self.recv().await? {
                    Incoming::Registered { id, payload } if id == "register" => {
                        return Ok(payload.client_key);
                    }
                    Incoming::Error { id, error } if id.as_deref() == Some("register") => {
                        return Err(Error::PairingRejected(error));
                    }
                    _ => {}
                }
            }
        })
        .await
        .map_err(|_| Error::Timeout(timeout))?
    }

    async fn send(&mut self, message: Value) -> Result<()> {
        self.ws.send(Message::text(message.to_string())).await?;
        Ok(())
    }

    async fn recv(&mut self) -> Result<Incoming> {
        loop {
            match self.ws.next().await.ok_or(Error::Closed)?? {
                Message::Text(text) => {
                    // Anything we can't make sense of isn't a reply to us.
                    return Ok(serde_json::from_str(text.as_str()).unwrap_or(Incoming::Other));
                }
                Message::Close(_) => return Err(Error::Closed),
                _ => {}
            }
        }
    }

    fn take_id(&mut self) -> String {
        self.next_id += 1;
        self.next_id.to_string()
    }

    /// One `ssap://` request, waiting for its reply.
    pub async fn request<T: DeserializeOwned>(&mut self, uri: &str, payload: Value) -> Result<T> {
        let id = self.take_id();
        self.send(json!({ "id": id, "type": "request", "uri": format!("ssap://{uri}"), "payload": payload }))
            .await?;
        let payload = tokio::time::timeout(REQUEST_TIMEOUT, async {
            loop {
                match self.recv().await? {
                    Incoming::Response { id: got, payload } if got == id => return Ok(payload),
                    Incoming::Error { id: Some(got), error } if got == id => {
                        return Err(Error::Tv {
                            uri: uri.to_owned(),
                            error,
                        });
                    }
                    _ => {}
                }
            }
        })
        .await
        .map_err(|_| Error::Timeout(REQUEST_TIMEOUT))??;

        // Some services report failure inside a successful response.
        if payload.get("returnValue") == Some(&Value::Bool(false)) {
            let error = payload
                .get("errorText")
                .and_then(Value::as_str)
                .unwrap_or("returnValue false")
                .to_owned();
            return Err(Error::Tv {
                uri: uri.to_owned(),
                error,
            });
        }
        serde_json::from_value(payload).map_err(|source| Error::Payload {
            uri: uri.to_owned(),
            source,
        })
    }

    /// For requests the TV may never answer, like turning itself off.
    async fn fire(&mut self, uri: &str) -> Result<()> {
        let id = self.take_id();
        self.send(json!({ "id": id, "type": "request", "uri": format!("ssap://{uri}"), "payload": {} }))
            .await
    }

    pub async fn power_state(&mut self) -> Result<PowerState> {
        #[derive(Deserialize)]
        struct R {
            state: PowerState,
        }
        let r: R = self
            .request("com.webos.service.tvpower/power/getPowerState", json!({}))
            .await?;
        Ok(r.state)
    }

    pub async fn turn_off(&mut self) -> Result<TurnOff> {
        let state = self.power_state().await?;
        if !state.is_on() {
            return Ok(TurnOff::AlreadyOff(state));
        }
        self.fire("system/turnOff").await?;
        Ok(TurnOff::Sent)
    }

    pub async fn screen_off(&mut self) -> Result<()> {
        self.request::<Value>("com.webos.service.tvpower/power/turnOffScreen", json!({}))
            .await
            .map(drop)
    }

    pub async fn screen_on(&mut self) -> Result<()> {
        self.request::<Value>("com.webos.service.tvpower/power/turnOnScreen", json!({}))
            .await
            .map(drop)
    }

    pub async fn foreground_app(&mut self) -> Result<AppId> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct R {
            app_id: AppId,
        }
        let r: R = self
            .request("com.webos.applicationManager/getForegroundAppInfo", json!({}))
            .await?;
        Ok(r.app_id)
    }

    pub async fn inputs(&mut self) -> Result<Vec<Input>> {
        #[derive(Deserialize)]
        struct R {
            devices: Vec<Input>,
        }
        let r: R = self.request("tv/getExternalInputList", json!({})).await?;
        Ok(r.devices)
    }

    /// Takes an [`Input`] from [`Tv::inputs`] rather than a bare id, so it can
    /// only ever name an input this TV has.
    pub async fn switch_input(&mut self, input: &Input) -> Result<()> {
        self.request::<Value>("tv/switchInput", json!({ "inputId": input.id() }))
            .await
            .map(drop)
    }

    pub async fn volume(&mut self) -> Result<VolumeStatus> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct R {
            volume_status: VolumeStatus,
        }
        let r: R = self.request("audio/getVolume", json!({})).await?;
        Ok(r.volume_status)
    }

    pub async fn set_volume(&mut self, volume: Volume) -> Result<()> {
        self.request::<Value>("audio/setVolume", json!({ "volume": volume }))
            .await
            .map(drop)
    }

    pub async fn set_mute(&mut self, mute: bool) -> Result<()> {
        self.request::<Value>("audio/setMute", json!({ "mute": mute }))
            .await
            .map(drop)
    }

    pub async fn toast(&mut self, message: &str) -> Result<()> {
        self.request::<Value>(
            "system.notifications/createToast",
            json!({ "message": message, "iconData": "", "iconExtension": "" }),
        )
        .await
        .map(drop)
    }

    /// Buttons go over a second socket whose URL the main one hands out.
    pub async fn press(&mut self, buttons: &[Button]) -> Result<()> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct R {
            socket_path: String,
        }
        let r: R = self
            .request("com.webos.service.networkinput/getPointerInputSocket", json!({}))
            .await?;
        let mut pointer = open_socket(&r.socket_path, CertPolicy::Pinned(self.fingerprint)).await?;
        for button in buttons {
            let frame = format!("type:button\nname:{}\n\n", button.wire_name());
            pointer.send(Message::text(frame)).await?;
        }
        pointer.close(None).await?;
        Ok(())
    }

    pub fn host(&self) -> &str {
        &self.host
    }
}

async fn open_socket(url: &str, policy: CertPolicy) -> Result<Socket> {
    let (ws, _) = tokio::time::timeout(
        REQUEST_TIMEOUT,
        tokio_tungstenite::connect_async_tls_with_config(url, None, true, Some(tls::connector(policy))),
    )
    .await
    .map_err(|_| Error::Timeout(REQUEST_TIMEOUT))??;
    Ok(ws)
}
