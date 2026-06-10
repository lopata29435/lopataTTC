# Lopata

🇷🇺 [Читать по-русски](README.ru.md)

Cross-platform desktop client for the [TrustTunnel](https://github.com/TrustTunnel/TrustTunnelClient)
VPN protocol. System tray, server profiles, `tt://` link import, logs, kill-switch,
automatic updates. Built with [Tauri 2](https://tauri.app) (Rust + WebView).

## Download

Grab the installer from the [latest release](https://github.com/lopata29435/lopataTTC/releases/latest):

| Your system | File |
|---|---|
| **Windows 10/11** | `Lopata_*_x64-setup.exe` |
| Windows on ARM / 32-bit | `*_arm64-setup.exe` / `*_x86-setup.exe` |
| **macOS** (Intel & Apple Silicon) | `Lopata_*_universal.dmg` |
| **Ubuntu / Debian / Mint** | `*_amd64.deb` |
| Fedora / openSUSE | `*.x86_64.rpm` |
| Any Linux (portable) | `*_amd64.AppImage` |

The `trusttunnel_client` core is downloaded automatically from the official
upstream releases on first launch. Both the app and the core then keep
themselves up to date.

## Usage

1. Install and launch.
2. Add a server: click a `tt://` link in the browser, paste one from the
   clipboard, or import a client `.toml` config.
3. Press **Connect**.

Administrator rights are required to create the TUN interface: Windows asks
once at startup (UAC), Linux/macOS ask at connect time (pkexec / system dialog).

### Profile options

| Option | Values |
|---|---|
| Protocol | HTTP/2 (TCP) or HTTP/3 (QUIC) |
| VPN mode | `general` (everything through VPN) or `selective` (listed domains only) |
| Anti-DPI, kill-switch, post-quantum TLS | on/off |
| Custom SNI, DNS upstreams, route exclusions | free-form |

## Build from source

```bash
cargo install tauri-cli --locked
cargo tauri dev     # run
cargo tauri build   # produce installers
```

Linux build deps: see [ci.yml](.github/workflows/ci.yml).

## Release

Bump the version in `src-tauri/Cargo.toml` **and** `package.json`, then:

```bash
git tag vX.Y.Z && git push origin master --tags
```

CI builds all platforms, signs the updater artifacts and publishes the release
automatically.

## License

Lopata is licensed under the [MIT License](LICENSE).

Lopata is an independent third-party client and is **not affiliated with
AdGuard**. The TrustTunnel protocol and the `trusttunnel_client` core are
© AdGuard Software Ltd, licensed under Apache License 2.0. Lopata does not
bundle or modify the core — it downloads unmodified official builds from the
[upstream releases](https://github.com/TrustTunnel/TrustTunnelClient/releases)
at first launch, which is permitted by the Apache 2.0 license.
