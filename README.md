# Kiyo

Zen-style vertical browser on Tauri v2 (system WebView, no bundled Chromium).

- **Gmail profiles**: each Gmail = its own isolated session (separate data dir). Click the colored circle to switch.
- **Low RAM**: tabs idle 5 min are destroyed and restored on click; tabs restore asleep on launch.
- **Compact mode**: ☰ collapses the sidebar to an icon rail.

## Run
```
cargo install tauri-cli --version "^2"
cd kiyo
cargo tauri icon path/to/any-square.png   # one-time, creates src-tauri/icons
cargo tauri dev
```
Linux needs `webkit2gtk-4.1`, `libgtk-3-dev`, `libsoup-3.0-dev`. Windows needs WebView2 (preinstalled on Win 10/11).

## Known limits
- Per-profile isolation uses `data_directory` (Windows/Linux). macOS needs a data-store identifier instead.
- Google may refuse sign-in in embedded WebViews ("browser not secure"); it's a Google policy, not the app.
- No extensions (WebView limitation).
