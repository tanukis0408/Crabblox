<p align="center">
  <img src="branding/icons/crabblox-256.png" width="150" alt="Crabblox Logo">
</p>

<h1 align="center">🦀 Crabblox</h1>

<p align="center">
  <b>Быстрый, современный и нативный лаунчер Roblox для Linux на Rust</b><br>
  <i>Запуск оригинального клиента macOS Roblox через <a href="https://www.darlinghq.org">Darling</a> с полной интеграцией в систему.</i>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Language-Rust-orange?logo=rust&logoColor=white" alt="Rust">
  <img src="https://img.shields.io/badge/Platform-Linux%20x86__64-blue?logo=linux&logoColor=white" alt="Linux">
  <img src="https://img.shields.io/badge/GUI-GTK4%20%2B%20Libadwaita-4A90E2?logo=gnome&logoColor=white" alt="GTK4">
  <img src="https://img.shields.io/badge/Author-Monster%20Dev-FF2A85" alt="Monster Dev">
  <img src="https://img.shields.io/badge/License-MIT-green" alt="MIT">
</p>

---

## ⚡ Особенности

* **🦀 Переписан на Rust**: Мгновенный отклик, сверхнизкое потребление ОЗУ и высокая стабильность.
* **🎨 Современный интерфейс**: Адаптивный дизайн на GTK4 и Libadwaita с поддержкой системных темной и светлой тем.
* **🎮 Оригинальный клиент Roblox**: Плавная графика (OpenGL), звук через PipeWire, поддержка захвата курсора, камеры и горячих клавиш.
* **🎧 Discord Rich Presence (RPC)**: Встроенная поддержка активности Discord с официальным логотипом Crabblox, временем в игре и ссылкой.
* **🚩 Fast Flags (FFlags)**: Удобное встроенное управление флагами клиента (разблокировка FPS, оптимизация графики, освещение и др.).
* **🌐 Умный DNS-селектор**: Обход проблем с загрузкой ассетов и подключением к серверам (Quad9, Cloudflare, Google DNS).
* **💻 Мощный CLI-интерфейс**: Возможность запускать игру и управлять настройками прямо из терминала без открытия графического окна.

---

## 🚀 Быстрая установка

Для установки Crabblox на **CachyOS**, **Arch Linux**, **Ubuntu**, **Debian** или **Fedora** выполните одну команду в терминале:

```bash
curl -fsSL https://raw.githubusercontent.com/tanukis0408/Crabblox/main/install.sh | bash
```

> **Что сделает установщик:**
> 1. Автоматически проверит и установит Darling (`darling-bin`) и системные библиотеки.
> 2. Скачает и установит Crabblox в `~/.local/share/Crabblox`.
> 3. Добавит иконки с крабом и ярлык в меню приложений, а также команду `crabblox` в систему.

После установки просто откройте **Crabblox** из меню приложений (или наберите `crabblox` в терминале), нажмите **Install Roblox**, а затем **Play**!

---

## ⌨️ Команды терминала (CLI)

Crabblox можно использовать как обычную консольную утилиту:

```bash
crabblox          # Запустить графический интерфейс (GUI)
crabblox run      # Запустить игру напрямую без окна лаунчера
crabblox status   # Проверить статус установки, версию клиента и пути
crabblox update   # Проверить и скачать обновление клиента Roblox
crabblox login    # Быстрая авторизация в аккаунт
crabblox flags    # Управление параметрами Fast Flags
```

---

## 🛠 Ручная сборка из исходников

Если вы хотите собрать лаунчер самостоятельно:

```bash
# 1. Клонируйте репозиторий
git clone https://github.com/tanukis0408/Crabblox.git
cd Crabblox

# 2. Соберите Rust-лаунчер
cargo build --release --manifest-path launcher-rs/Cargo.toml
cp launcher-rs/target/release/crabblox ./crabblox

# 3. Соберите динамические библиотеки Darling
./build_debug_shim.sh

# 4. Установите ярлыки и иконки
./launcher/install.sh
```

---

## ❓ Часто задаваемые вопросы (FAQ)

<details>
<summary><b>Безопасен ли мой аккаунт?</b></summary>

Да. Авторизация происходит внутри официального клиента Roblox. Лаунчер никогда не имеет доступа к вашему паролю. Сессия сохраняется локально в зашифрованном префиксе Darling на вашем компьютере.
</details>

<details>
<summary><b>Не загружаются плейсы или текстуры?</b></summary>

В некоторых регионах провайдеры могут блокировать отдельные адреса Roblox. В лаунчере перейдите в **Настройки → DNS** и переключитесь на **Quad9** или **Cloudflare**. Это исправит подключение.
</details>

<details>
<summary><b>Как настроить Fast Flags (разблокировку FPS)?</b></summary>

Во вкладке **Fast Flags** вы можете выставить желаемый лимит кадров (например, 144, 240 или 0 для снятия лока), а также включить оптимизации рендеринга Vulkan/OpenGL.
</details>

---

## 📜 Лицензия и благодарности

* Проект распространяется под лицензией [MIT](LICENSE).
* Разработано командой **Monster Dev**.
* Основано на исследованиях и наработках проекта [MacOBlox](https://github.com/narezy/MacOBlox) и слоя совместимости [Darling](https://www.darlinghq.org).
* *Crabblox не связан с Roblox Corporation.*
