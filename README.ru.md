# Lopata

🇬🇧 [Read in English](README.md)

Кроссплатформенный десктопный клиент VPN-протокола
[TrustTunnel](https://github.com/TrustTunnel/TrustTunnelClient). Системный трей,
профили серверов, импорт `tt://`-ссылок, логи, kill-switch, автообновления.
Сделан на [Tauri 2](https://tauri.app) (Rust + WebView).

## Скачать

Установщик — в [последнем релизе](https://github.com/lopata29435/lopataTTC/releases/latest):

| Система | Файл |
|---|---|
| **Windows 10/11** | `Lopata_*_x64-setup.exe` |
| Windows на ARM / 32-bit | `*_arm64-setup.exe` / `*_x86-setup.exe` |
| **macOS** (Intel и Apple Silicon) | `Lopata_*_universal.dmg` |
| **Ubuntu / Debian / Mint** | `*_amd64.deb` |
| Fedora / openSUSE | `*.x86_64.rpm` |
| Любой Linux (портативно) | `*_amd64.AppImage` |

Ядро `trusttunnel_client` скачивается автоматически из официальных релизов
при первом запуске. Дальше и приложение, и ядро обновляются сами.

## Использование

1. Установи и запусти.
2. Добавь сервер: кликни `tt://`-ссылку в браузере, вставь её из буфера
   или импортируй `.toml`-конфиг клиента.
3. Нажми **Подключиться**.

Для создания TUN-интерфейса нужны права администратора: Windows спрашивает
один раз при запуске (UAC), Linux/macOS — при подключении (pkexec / системный
диалог).

### Параметры профиля

| Параметр | Значения |
|---|---|
| Протокол | HTTP/2 (TCP) или HTTP/3 (QUIC) |
| Режим VPN | `general` (весь трафик) или `selective` (только списки доменов) |
| Anti-DPI, kill-switch, постквантовый TLS | вкл/выкл |
| Свой SNI, DNS-апстримы, исключения маршрутов | произвольно |

## Сборка из исходников

```bash
cargo install tauri-cli --locked
cargo tauri dev     # запуск
cargo tauri build   # сборка установщиков
```

Зависимости для сборки под Linux — в [ci.yml](.github/workflows/ci.yml).

## Релиз

Подними версию в `src-tauri/Cargo.toml` **и** `package.json`, затем:

```bash
git tag vX.Y.Z && git push origin master --tags
```

CI соберёт все платформы, подпишет артефакты автообновления и опубликует
релиз автоматически.

## Лицензия

Lopata распространяется под [лицензией MIT](LICENSE).

Lopata — независимый сторонний клиент, **не аффилированный с AdGuard**.
Протокол TrustTunnel и ядро `trusttunnel_client` — © AdGuard Software Ltd,
лицензия Apache 2.0. Lopata не включает ядро в дистрибутив и не модифицирует
его — при первом запуске скачиваются немодифицированные официальные сборки из
[релизов upstream](https://github.com/TrustTunnel/TrustTunnelClient/releases),
что разрешено лицензией Apache 2.0.
