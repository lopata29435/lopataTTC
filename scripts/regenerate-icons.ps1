# Регенерация иконок приложения из исходника `app-icon.png` в корне проекта.
# Создаёт все варианты в `src-tauri/icons/` + копирует tray-иконку.
# Запускать после замены `app-icon.png`.

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$source = Join-Path $root 'app-icon.png'

if (-not (Test-Path $source)) {
    Write-Error "Не найден $source. Положи туда квадратный PNG минимум 1024x1024."
}

Push-Location $root
try {
    Write-Host "Источник: $source"
    cargo tauri icon $source
    Copy-Item (Join-Path $root 'src-tauri/icons/32x32.png') `
              (Join-Path $root 'src-tauri/icons/tray.png') -Force
    Write-Host "Готово. Пересобери проект: cargo tauri dev / cargo tauri build"
} finally {
    Pop-Location
}
