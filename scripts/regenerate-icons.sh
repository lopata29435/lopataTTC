#!/usr/bin/env bash
# Регенерация иконок приложения из app-icon.png в корне проекта.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOURCE="${ROOT}/app-icon.png"

if [[ ! -f "$SOURCE" ]]; then
  echo "Не найден $SOURCE. Положи туда квадратный PNG минимум 1024x1024." >&2
  exit 1
fi

cd "$ROOT"
echo "Источник: $SOURCE"
cargo tauri icon "$SOURCE"
cp -f src-tauri/icons/32x32.png src-tauri/icons/tray.png
echo "Готово. Пересобери проект: cargo tauri dev / cargo tauri build"
