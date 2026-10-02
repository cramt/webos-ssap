# webos-ssap

Rust client for SSAP, the websocket API on LG webOS TVs, plus a `tv` CLI.

LG never documented SSAP. Payloads are checked against a real OLED65B46LA
(webOS 9.2.4); [aiowebostv](https://github.com/home-assistant-libs/aiowebostv)
is the reference for everything else, and the registration manifest mirrors it
exactly so keys paired through Home Assistant work here too.

## Usage

```sh
nix run github:cramt/webos-ssap -- --host 192.168.178.36 pair
```

Accept the prompt on the TV. `pair` prints the client key and the TV's
certificate fingerprint. Every other command needs both:

```sh
export TV_HOST=192.168.178.36
export TV_KEY_FILE=/run/secrets/tv-key        # file holding the client key
export TV_CERT_FINGERPRINT=11:C5:B1:...       # from `tv pair`
export TV_MAC=58:96:0a:9a:36:a4               # only for `tv on`

tv status
tv on --input HDMI_1      # Wake-on-LAN, wait until Active, switch input
tv off                    # no-op if it's already off
tv volume 20
tv mute on
tv screen off             # picture off, sound keeps playing
tv toast "deploy done"
tv press home down enter
```

## Design notes

- **TLS only, with a pinned cert.** Each TV serves its own self-signed cert, so
  there's no CA to check against. After pairing, every connection, including
  the separate pointer socket for button presses, has to present the same cert.
  Without the pin, anyone on the LAN could pose as the TV and collect the key.
- **Inputs and apps come from the TV.** `InputId` and `AppId` can't be built by
  hand. `switch_input` takes an `Input` from `Tv::inputs`, so it can't name an
  input the TV doesn't have.
- **`turn_off` checks first.** On some firmware `system/turnOff` wakes a TV in
  standby, so it's only sent when the TV reports itself on.
- **Wake-on-LAN** needs "Turn on via Wi-Fi" enabled on the TV. Despite the
  name, it covers the ethernet port too.
