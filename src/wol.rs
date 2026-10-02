use std::fmt;
use std::str::FromStr;

use tokio::net::UdpSocket;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MacAddr([u8; 6]);

#[derive(Debug, thiserror::Error)]
#[error("expected a MAC address like 58:96:0a:9a:36:a4")]
pub struct InvalidMac;

impl FromStr for MacAddr {
    type Err = InvalidMac;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut bytes = [0u8; 6];
        let mut parts = s.split([':', '-']);
        for byte in &mut bytes {
            let part = parts.next().ok_or(InvalidMac)?;
            if part.len() != 2 {
                return Err(InvalidMac);
            }
            *byte = u8::from_str_radix(part, 16).map_err(|_| InvalidMac)?;
        }
        match parts.next() {
            Some(_) => Err(InvalidMac),
            None => Ok(Self(bytes)),
        }
    }
}

impl fmt::Display for MacAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [a, b, c, d, e, g] = self.0;
        write!(f, "{a:02x}:{b:02x}:{c:02x}:{d:02x}:{e:02x}:{g:02x}")
    }
}

/// Broadcast a magic packet. The TV only listens for it when
/// "Turn on via Wi-Fi" is enabled, which also covers its ethernet port.
pub async fn wake(mac: MacAddr) -> std::io::Result<()> {
    let mut packet = [0xFFu8; 102];
    for chunk in packet[6..].chunks_mut(6) {
        chunk.copy_from_slice(&mac.0);
    }
    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    socket.set_broadcast(true)?;
    socket.send_to(&packet, "255.255.255.255:9").await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_round_trips() {
        let mac: MacAddr = "58:96:0A:9a:36:a4".parse().unwrap();
        assert_eq!(mac.to_string(), "58:96:0a:9a:36:a4");
    }

    #[test]
    fn rejects_wrong_length() {
        assert!("58:96:0a:9a:36".parse::<MacAddr>().is_err());
        assert!("58:96:0a:9a:36:a4:00".parse::<MacAddr>().is_err());
        assert!("589:6:0a:9a:36:a4".parse::<MacAddr>().is_err());
    }
}
