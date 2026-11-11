# rbxview на Android

Просмотрщик мест Roblox (`rbxview`) на телефоне. Открывает `.rbxl`, `.rbxm`,
`.rbxlx` и `.rbxmx` — то есть обычные места и модели, сохранённые из Roblox
Studio, — и рисует их так же, как на компьютере: части, материалы, освещение,
гипертекстовые интерфейсы, небо, невидимые сервисы не нужны.

Это тот же рендерер, что и в браузерной версии: `crates/rbx_viewer/web`
собирается в WebAssembly с WebGPU, а приложение на Kotlin даёт ему две вещи,
которых у страницы нет, — доступ к файлам телефона и загрузку ассетов Roblox.

Цель — Pixel 7 (Android 14+, Tensor G2, Mali-G710) и вообще любой Android 12+
с обновлённым Chrome или WebView. Собрано и подписано это в CI
(`.github/workflows/android.yml`); если что-то не заводится — пришлите
logcat, в нём приложение пишет версию WebView и каждый отказ ассетов.

## Что умеет

- **Открытие мест.** Кнопка «Open place or model…» в панели открывает системный
  выбор файлов (Storage Access Framework); файл можно и просто «поделиться» в
  rbxview из файлового менеджера, Telegram или Drive.
- **Осмотр сцены касаниями.** Слева джойстик — полёт вперёд/назад/в стороны;
  свайп по экрану — поворот камеры; щипок — приближение/удаление; кнопки
  справа — вверх, вниз и возврат к исходному виду. Короткое касание выбирает
  часть, как левый клик на компьютере.
- **Панель и Explorer.** Всё, что есть в браузерной версии: переключатели
  отображения, качество, дерево объектов, предупреждения.
- **Ассеты.** Текстуры, звуки, шрифты и модели Roblox подтягиваются из
  `assetdelivery.roblox.com` и пакетов контента Studio (`setup.rbxcdn.com`) и
  кэшируются на телефоне, так что уже показанное место открывается и офлайн.
  Первый показ места без интернета будет неполным: без загруженных текстур
  части рисуются своим цветом.

Чего нет (и не будет): полноценного редактора `rbxstudio`. Он построен на
GPUI Kit для настольных окон, и переносить его на телефон — отдельная работа.

## Первый запуск на Pixel 7

1. Установите APK (ниже — как).
2. Откройте **rbxview**. Появится тёмный экран с кнопкой
   **Open place or model…**.
3. Нажмите её и выберите файл `.rbxl` (или `.rbxm`). Файл может лежать в
   «Загрузках», на Google Drive, в Telegram — подойдёт любой источник, который
   умеет отдавать файлы системе.
4. Место откроется. Слева джойстик, справа кнопки, свайп — камера.

Если вместо картинки написано **This WebView has no WebGPU** — нажмите
**Open in Chrome**, откроется то же место в Chrome, где WebGPU на Pixel 7
работает. После этого можно вернуться в rbxview: он продолжает работать, пока
Chrome открыт.

## Как установить

### Вариант 1: скачать APK из GitHub Actions (без Android Studio)

1. Откройте вкладку **Actions** репозитория, workflow **android**
   (<https://github.com/SMSLIMEES/rbx-native/actions/workflows/android.yml>),
   и последний зелёный запуск (галочка, не крестик).
2. Внизу страницы запуска, в разделе **Artifacts**, скачайте
   `rbxview-android-<sha>.zip` и распакуйте.
3. Внутри — `app-debug.apk` (подписан отладочным ключом, ставится сразу) и
   `rbxview-ci-release.apk` (подписан одноразовым ключом CI). Ставьте
   **release**: он меньше и без отладочных флагов, а `debug` пригодится, только
   если нужно подключиться отладчиком.
4. Артефакты GitHub отдаёт только вошедшим в аккаунт; если это неудобно,
   соберите APK сами (вариант 2) или возьмите его из релиза
   (см. «Релизы» ниже).
5. Перекиньте APK на телефон (кабель, Drive, Telegram) и откройте его.
   Android спросит разрешение «Устанавливать неизвестные приложения» — для
   файлового менеджера или Telegram, из которого вы открываете, — разрешите.
6. Если Play Protect предупредит про незнакомое приложение, выберите
   «Всё равно установить»: APK не из магазина.

### Релизы

На каждый тег `v*` (`release.yml`) к релизу прикладывается
`rbxview-<версия>-android.apk` — это тот самый release-APK из CI, только
без похода в Actions и без аккаунта GitHub.

### Вариант 2: собрать самому

Нужны Rust (cargo), `wasm32-unknown-unknown`, `wasm-bindgen-cli` той же
версии, что `wasm-bindgen` в `Cargo.lock`, JDK 17 и Android SDK (compileSdk 35,
build-tools 35.0.0).

```sh
# Из корня репозитория: страница и wasm-модуль.
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.128 --locked   # версию см. в Cargo.lock
./scripts/build-web.sh

# Приложение: адрес SDK — в local.properties или ANDROID_HOME.
cd packaging/android
echo "sdk.dir=$ANDROID_HOME" > local.properties
./gradlew assembleDebug
```

APK окажется в `packaging/android/app/build/outputs/apk/debug/app-debug.apk`:

```sh
adb install -r packaging/android/app/build/outputs/apk/debug/app-debug.apk
```

### Вариант 3: запустить на телефоне из Android Studio

Откройте в Android Studio папку `packaging/android` (не корень репозитория),
затем **Run**. Перед этим всё равно нужен `./scripts/build-web.sh` — без
`crates/rbx_viewer/web/pkg` сборка останавливается с подсказкой.

## WebGPU: почему может понадобиться Chrome

Рендерер использует WebGPU. В Chrome на Android он есть с версии 121 (Android
12+, GPU Qualcomm/ARM/Mali), а вот системный **WebView** отдаёт `navigator.gpu`
не на всех прошивках и версиях.

Поэтому приложение не полагается на WebView: если страница не видит WebGPU,
она показывает кнопку **Open in Chrome**. Нажатие открывает в Chrome тот же
самый адрес (`http://127.0.0.1:<порт>/`) — страница, wasm-модуль и прокси
ассетов продолжают работать, потому что сервер живёт в приложении, а не в
WebView. Вернитесь в rbxview — он продолжит работать в фоне, пока Chrome
открыт.

Если и в Chrome не работает:

- проверьте `chrome://gpu` — строка **WebGPU** должна быть `Hardware accelerated`;
- обновите Chrome и «Android System WebView» в Play Store;
- Android должен быть 12 или новее (на Pixel 7 это так).

## Устройство

| Файл | Что делает |
| --- | --- |
| `app/src/main/java/dev/rbxnative/viewer/MainActivity.kt` | WebView, выбор файла (SAF), интенты `.rbxl`/`.rbxm`/`.rbxlx`/`.rbxmx`, запуск в Chrome |
| `LocalServer.kt` | `rbxview --serve` на телефоне: HTTP на `127.0.0.1`, отдаёт страницу из APK, `/place` и `/asset/...` |
| `AssetProxy.kt` | Резолвер ассетов: `assetdelivery.roblox.com` и пакеты `setup.rbxcdn.com`, кэш в `cacheDir` |
| `../../crates/rbx_viewer/web/index.html` | Страница просмотрщика; сенсорное управление и вызов нативного выбора файла — здесь же |

Страница и прокси повторяют `crates/rbx_viewer/src/serve.rs` и
`crates/rbx_viewer/src/load/fetcher/web.rs` один в один по протоколу: те же
пути, те же коды (404 — окончательный отказ, 503 — временный, страница
повторит), то же поведение для `rbxasset://` (подстановки `SpawnLocation.png`
и `GuiImagePlaceholder.png`, пакеты `content-textures2/3`, `content-sky`,
`content-fonts`, `content-sounds`, `content-models`, `content-avatar`).

Подпись: CI подписывает release-сборку одноразовым ключом, поэтому новая
сборка ставится поверх старой только после удаления приложения. Для личного
пользования это не мешает; для нормальной подписи заведите свой keystore и
`signingConfigs` в `app/build.gradle.kts`.
