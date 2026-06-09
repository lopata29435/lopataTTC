# lopataTTC — TrustTunnel GUI

Кроссплатформенный десктопный клиент (GUI) для VPN-протокола
[TrustTunnel](https://github.com/TrustTunnel/TrustTunnelClient): системный трей,
профили серверов, импорт по `tt://`-ссылкам и `.toml`-конфигам, логи, killswitch
и автообновления.

Технологии: [Tauri 2](https://tauri.app) (Rust backend + WebView frontend, без Node-зависимостей).

## Установка

Скачайте установщик для своей платформы со страницы
[Releases](https://github.com/lopata29435/lopataTTC/releases/latest):

| Платформа | Файл |
|---|---|
| Windows x64 / x86 / ARM64 | `*-setup.exe` (NSIS) или `*.msi` |
| macOS (Intel + Apple Silicon) | `*.dmg` |
| Linux | `*.AppImage`, `*.deb`, `*.rpm` |

## Как это работает

### Бинарник VPN-клиента

Сам `trusttunnel_client` **не зашит в установщик**. При первом запуске GUI
скачивает свежий релиз из [TrustTunnel/TrustTunnelClient](https://github.com/TrustTunnel/TrustTunnelClient/releases)
в папку данных (`%APPDATA%\TrustTunnel\clients\` / `~/.config/TrustTunnel/clients/`)
и дальше автоматически подкачивает новые версии в фоне при каждом запуске.

### Автообновление самого GUI

При старте приложение проверяет последний релиз этого репозитория. Если вышла
новая версия — в Настройках появляется кнопка «Установить»: обновление
скачивается, проверяется Ed25519-подписью (tauri-plugin-updater) и ставится без
ручного скачивания установщика. На Linux self-update работает для AppImage;
для `.deb`/`.rpm` — кнопка «Открыть страницу релиза».

### Права администратора

VPN-клиенту нужны права root/admin (TUN-интерфейс, маршруты, firewall):

* **Windows** — приложение само запускается с повышением прав
  (`requireAdministrator`), UAC-запрос при старте.
* **Linux** — GUI работает **от обычного пользователя** (WebKitGTK под root не
  работает). При нажатии «Подключиться» система показывает диалог PolicyKit
  (`pkexec`) и с правами root запускается только сам клиент. Остановка —
  через stop-файл, который отслеживает обёртка-скрипт.
* **macOS** — аналогично, через системный диалог авторизации (`osascript`).

## Сборка из исходников

Требования: Rust (stable), на Linux — `libwebkit2gtk-4.1-dev libgtk-3-dev
libayatana-appindicator3-dev librsvg2-dev` и т.д. (см. [ci.yml](.github/workflows/ci.yml)).

```bash
cargo install tauri-cli --locked
cargo tauri dev      # запуск в dev-режиме
cargo tauri build    # сборка установщиков
```

Для оффлайн-разработки можно заранее положить клиент рядом:
`scripts/fetch-client.sh` / `scripts/fetch-client.ps1`.

## Выпуск релиза

1. Поднимите версию в **двух** файлах: `src-tauri/Cargo.toml` и `package.json`
   (они должны совпадать; `tauri.conf.json` версию не содержит — это проверяет CI).
2. Закоммитьте и создайте тег:

   ```bash
   git tag v0.2.0 && git push origin master --tags
   ```

3. GitHub Actions ([release.yml](.github/workflows/release.yml)):
   * `verify-versions` — сверяет тег с версиями в файлах;
   * `build` — собирает 6 платформ, подписывает апдейтер-артефакты
     (`TAURI_SIGNING_PRIVATE_KEY` в Secrets) и заливает в draft-релиз
     вместе с `latest.json`;
   * `publish` — проверяет, что `latest.json` и `.sig` на месте, и публикует
     релиз. С этого момента все установленные приложения увидят обновление.

### Ключи подписи обновлений

Публичный ключ зашит в `src-tauri/tauri.conf.json` (`plugins.updater.pubkey`),
приватный — в GitHub Secrets (`TAURI_SIGNING_PRIVATE_KEY` +
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`). Если потеряете приватный ключ,
сгенерируйте новую пару (`cargo tauri signer generate`) и обновите **оба**
места — но уже установленные приложения перестанут принимать автообновления
(пользователям придётся один раз поставить новую версию вручную).

## Структура

```
src/                  — фронтенд (vanilla JS + CSS, без сборщика)
src-tauri/src/
  lib.rs              — bootstrap, трей, фоновая проверка обновлений
  vpn.rs              — запуск/остановка клиента, elevation на Unix, логи
  updater.rs          — автодокачка trusttunnel_client из upstream-релизов
  app_updater.rs      — проверка новых версий самого GUI
  profiles.rs         — хранилище профилей (JSON в папке данных)
  deeplink.rs         — разбор tt://-ссылок
  elevate.rs          — повышение прав (UAC / pkexec / osascript)
  service.rs          — установка клиента как службы Windows (автостарт)
```

## Лицензия

GUI — личный проект [@lopata29435](https://github.com/lopata29435).
Upstream-клиент TrustTunnel — Apache 2.0, © AdGuard Software Ltd.
