# Lopata

🇷🇺 [Читать по-русски](README.ru.md)

Desktop client for the [TrustTunnel](https://github.com/TrustTunnel/TrustTunnelClient)
VPN protocol for Windows, macOS and Linux. System tray, server profiles,
`tt://` link import, connection logs, kill-switch, automatic updates.

## Download

Get the installer from the [latest release](https://github.com/lopata29435/lopataTTC/releases/latest):

| Your system | File |
|---|---|
| **Windows 10/11** | `Lopata_*_x64-setup.exe` |
| Windows on ARM / 32-bit | `*_arm64-setup.exe` / `*_x86-setup.exe` |
| **macOS** (Intel & Apple Silicon) | `Lopata_*_universal.dmg` |
| **Ubuntu / Debian / Mint** | `*_amd64.deb` |
| Fedora / openSUSE | `*.x86_64.rpm` |
| Any Linux (portable, no install) | `*_amd64.AppImage` |

The VPN core is downloaded automatically on first launch. After that both the
app and the core keep themselves up to date — no manual updates needed.

## How to use

1. Install and launch Lopata.
2. Add a server in any convenient way:
   * click a `tt://` link in your browser;
   * copy a `tt://` link and press **Import from clipboard**;
   * import a `.toml` config file.
3. Press **Connect**.

Administrator rights are required to create the VPN network interface:
Windows asks once at startup (UAC prompt), Linux and macOS ask when you
press Connect.

## Profile options

| Option | Values |
|---|---|
| Protocol | HTTP/2 (TCP) or HTTP/3 (QUIC) |
| VPN mode | `general` — all traffic through VPN, or `selective` — only listed domains |
| Anti-DPI | masks VPN traffic from deep packet inspection |
| Kill-switch | blocks traffic if the VPN connection drops |
| Post-quantum TLS | on/off |
| Custom SNI, DNS upstreams, route exclusions | free-form |

## License

Lopata is free software under the [MIT License](LICENSE).

Lopata is an independent third-party client and is **not affiliated with
AdGuard**. The TrustTunnel protocol and the VPN core are © AdGuard Software
Ltd, licensed under Apache License 2.0. Lopata does not bundle or modify the
core — it downloads unmodified official builds from the
[upstream releases](https://github.com/TrustTunnel/TrustTunnelClient/releases),
which is permitted by the Apache 2.0 license.
