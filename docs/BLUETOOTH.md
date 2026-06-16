# Bridging a phone's GSM call over Bluetooth (Linux)

This is how to make a Linux PC act as a Bluetooth **hands-free unit** for a
phone, so a real GSM voice call's audio becomes PCM you can pipe through `dov` —
no extra hardware, just the phone + a Bluetooth adapter. (Tested on Debian 13,
PipeWire/WirePlumber 0.5.8, a Pixel 7 forced to 2G.)

It's fiddly. The two gotchas that cost the most time:

- **PipeWire's *native* HFP backend cannot be a hands-free unit for a phone** —
  it only ever offers the `audio-gateway` card profile (PC pretends to be the
  phone), never `headset-head-unit`. You must use **oFono** as the backend.
- The PipeWire property is **`bluez5.hfphsp-backend`** — *not* `bluez5.hfp-backend`
  (a silent no-op). HFP roles, if you need them, are `bluez5.headset-roles`.

## Setup

```sh
# 1. oFono (the HFP hands-free backend)
sudo apt install ofono
sudo systemctl enable --now ofono

# 2. Tell PipeWire to use oFono for HFP/HSP
mkdir -p ~/.config/wireplumber/wireplumber.conf.d
cat > ~/.config/wireplumber/wireplumber.conf.d/51-bluez-hfp-ofono.conf <<'EOF'
monitor.bluez.properties = {
  bluez5.hfphsp-backend = "ofono"
}
EOF

# 3. Restart IN THIS ORDER: wireplumber first (frees the HFP UUID), then ofono.
#    If ofono logs "UUID already registered", it lost the race — restart it again.
systemctl --user restart wireplumber
sudo systemctl restart ofono           # `journalctl -u ofono` should be clean

# 4. Pair the phone, and on the phone enable "Phone calls"/HFP for this PC.
bluetoothctl
#   power on / agent on / scan on / pair <MAC> / trust <MAC> / connect <MAC> / quit
```

After `connect`, oFono auto-creates an HFP modem and brings it online (it exposes
`org.ofono.Handsfree` + `VoiceCallManager`):

```sh
busctl --system call org.ofono / org.ofono.Manager GetModems   # Powered/Online = true
```

> First `connect` after a WirePlumber restart often fails with
> `br-connection-profile-unavailable` — that's a profile-registration race. Wait
> a few seconds and retry.

## Driving a call

The call-audio nodes appear **only during an active call** routed to the PC, as
`bluez_output.<MAC>.1` (uplink → into the call) and `bluez_input.<MAC>.0`
(downlink). They show in `wpctl status` / `pw-cli ls Node`, not in `pactl`. Find
them with:

```sh
wpctl status | grep bluez
```

Place a 2G call on the phone, route its audio to the PC ("…" → Bluetooth), then:

```sh
# transmit a message into the call (the far end hears the modem):
dov encode "hello over GSM" /tmp/tx.wav
pw-play --target bluez_output.<MAC>.1 /tmp/tx.wav
# ...or straight from dov via the pw: device prefix:
dov send "hello over GSM" pw:bluez_output.<MAC>.1

# receive + decode the downlink:
dov recv 20 pw:bluez_input.<MAC>.0
```

Mute the PC mic first (`pactl set-source-mute <mic> 1`) so room noise doesn't
leak into the call uplink.

## What this gets you, and the limit

This bridges **one** end of the call (one Bluetooth SCO link per adapter). You
can fully *inject* `dov` into a real GSM call and confirm the far end hears it.
**Decoding** needs the *other* end captured too — a second Linux box bridged to
the second phone (the clean end-to-end test), a carrier echo number (single-box
round-trip), or a cable from the far phone into a line-in. macOS can't bridge a
phone (it's an audio gateway, not a hands-free unit), so it isn't a second
bridge. The USB GSM voice dongle sidesteps all of this by exposing call audio as
USB PCM directly.
