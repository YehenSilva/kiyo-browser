#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Mutex;
use tauri::{
    menu::{ContextMenu, Menu, MenuItem, PredefinedMenuItem, Submenu},
    webview::{NewWindowResponse, WebviewBuilder}, window::WindowBuilder, AppHandle, Emitter, LogicalPosition,
    LogicalSize, Manager, PhysicalPosition, PhysicalSize, Url, WebviewUrl, WebviewWindowBuilder,
    WindowEvent,
};

const INJECT_SCRIPT: &str = include_str!("autofill_inject.js");
const CHROME_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

#[derive(Clone, Debug)]
struct LayoutState {
    sidebar: f64,
    top_bar: f64,
    primary_tab: Option<String>,
    secondary_tab: Option<String>,
}

struct Layout(Mutex<LayoutState>);

#[derive(Clone, serde::Serialize, serde::Deserialize, Debug)]
struct AutofillContext {
    tab_id: String,
    host: String,
}

struct AutofillState(Mutex<Option<AutofillContext>>);

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
struct FieldBounds {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
struct AutofillItem {
    action: String,
    index: Option<usize>,
    text: String,
    sub: String,
}

/// Logical gap around the page pane (0.0 for seamless edge-to-edge window).
const PAD: f64 = 0.0;

/// Layout: the UI webview fills the window (sidebar + empty-state stage drawn in HTML).
/// Tab webviews float on top, flush to the right of the sidebar and below the title bar.
/// Supports single tab view and side-by-side split view.
fn relayout(app: &AppHandle) {
    let Some(win) = app.get_window("main") else { return };
    let Ok(size) = win.inner_size() else { return };
    let sf = win.scale_factor().unwrap_or(1.0);
    let layout = {
        let state = app.state::<Layout>();
        let val = state.0.lock().unwrap().clone();
        val
    };

    let sb = (layout.sidebar * sf).round() as i32;
    let top_offset = if layout.top_bar > 0.0 {
        (layout.top_bar * sf).round() as i32
    } else {
        (40.0 * sf).round() as i32
    };
    let (w, h) = (size.width as i32, size.height as i32);

    let is_split = layout.primary_tab.is_some()
        && layout.secondary_tab.is_some()
        && layout.primary_tab != layout.secondary_tab;

    // Split view header is unified directly inside the single-row top bar
    let content_y = top_offset;
    let content_h = (h - content_y).max(1) as u32;
    let avail_w = (w - sb).max(1);

    for (label, wv) in app.webviews() {
        if label == "ui" {
            let _ = wv.set_position(PhysicalPosition::new(0, 0));
            let _ = wv.set_size(PhysicalSize::new(w.max(1) as u32, h.max(1) as u32));
        } else if label == "glance" {
            let gw = ((w as f64) * 0.76).round() as u32;
            let gh = ((h as f64) * 0.80).round() as u32;
            let gx = ((w as u32).saturating_sub(gw) / 2) as i32;
            let gy = ((h as u32).saturating_sub(gh) / 2) as i32;
            let header_h = (44.0 * sf).round() as i32;
            let _ = wv.set_position(PhysicalPosition::new(gx, gy + header_h));
            let _ = wv.set_size(PhysicalSize::new(gw, gh.saturating_sub(header_h as u32)));
            let _ = wv.show();
        } else if let Some(tab_id) = label.strip_prefix("tab-") {
            let is_primary = layout.primary_tab.as_deref() == Some(tab_id);
            let is_secondary = layout.secondary_tab.as_deref() == Some(tab_id);

            if is_split && is_primary {
                // Split view: Left pane (with 4px divider gap)
                let split_gap = (4.0 * sf).round() as i32;
                let w1 = ((avail_w - split_gap) / 2).max(1) as u32;
                let _ = wv.set_position(PhysicalPosition::new(sb, content_y));
                let _ = wv.set_size(PhysicalSize::new(w1, content_h));
                let _ = wv.show();
            } else if is_split && is_secondary {
                // Split view: Right pane
                let split_gap = (4.0 * sf).round() as i32;
                let w1_i32 = ((avail_w - split_gap) / 2).max(1);
                let w2 = (avail_w - w1_i32 - split_gap).max(1) as u32;
                let _ = wv.set_position(PhysicalPosition::new(sb + w1_i32 + split_gap, content_y));
                let _ = wv.set_size(PhysicalSize::new(w2, content_h));
                let _ = wv.show();
            } else if is_primary {
                // Single tab: full seamless stage flush to all borders
                let _ = wv.set_position(PhysicalPosition::new(sb, content_y));
                let _ = wv.set_size(PhysicalSize::new(avail_w as u32, content_h));
                let _ = wv.show();
            } else {
                let _ = wv.hide();
            }
        }
    }
}

fn safe(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '@' || c == '.' { c } else { '_' })
        .collect()
}

/// Open a tab as a child webview. Each profile gets its own data directory.
/// When a new tab is opened, any existing autofill overlay is destroyed so that
/// subsequent autofill overlays are recreated above the tab in z-order.
#[tauri::command]
async fn tab_open(app: AppHandle, webview: tauri::Webview, id: String, profile: String, url: String) -> Result<(), String> {
    if webview.label() != "ui" {
        return Err("Forbidden".into());
    }
    // Destroy autofill overlay so recreating it later places it on top of the new tab
    if let Some(af) = app.get_webview("autofill") {
        let _ = af.close();
    }
    *app.state::<AutofillState>().0.lock().unwrap() = None;

    let label = format!("tab-{id}");
    if app.get_webview(&label).is_some() {
        return Ok(());
    }
    let win = app.get_window("main").ok_or("no window")?;
    let parent_folder = if profile.starts_with("temp_") {
        "temp_profiles"
    } else {
        "profiles"
    };
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join(parent_folder)
        .join(safe(&profile));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let parsed: Url = url.parse().map_err(|e| format!("Invalid address: {e}"))?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err("Only http(s) addresses are allowed".into());
    }

    let sf = win.scale_factor().unwrap_or(1.0);
    let size = win.inner_size().unwrap_or(PhysicalSize::new(1280, 800));
    let (sb, content_y, avail_w, content_h) = {
        let state = app.state::<Layout>();
        let mut l = state.0.lock().unwrap();
        l.primary_tab = Some(id.clone());
        let sb = (l.sidebar * sf).round() as i32;
        let top_offset = if l.top_bar > 0.0 {
            (l.top_bar * sf).round() as i32
        } else {
            (40.0 * sf).round() as i32
        };
        let (w, h) = (size.width as i32, size.height as i32);
        (sb, top_offset, (w - sb).max(1) as u32, (h - top_offset).max(1) as u32)
    };

    let (app2, id2) = (app.clone(), id.clone());
    let init_script = format!(
        "window.__KIYO_TAB_ID = {:?};\nwindow.__KIYO_PROFILE = {:?};\n{}",
        id, profile, INJECT_SCRIPT
    );
    let builder = WebviewBuilder::new(label, WebviewUrl::External(parsed))
        .data_directory(dir)
        .user_agent(CHROME_UA)
        .initialization_script_for_all_frames(&init_script)
        .on_new_window(|_, _| NewWindowResponse::Allow)
        .on_navigation(move |u| {
            let _ = app2.emit("tab-url", (id2.clone(), u.to_string()));
            true
        });
    win.add_child(builder, PhysicalPosition::new(sb, content_y), PhysicalSize::new(avail_w, content_h))
        .map_err(|e| e.to_string())?;
    relayout(&app);
    Ok(())
}

/// Show active tab(s) (None hides all and reveals the start page).
/// Supports primary tab and optional secondary split tab.
#[tauri::command]
#[allow(non_snake_case)]
async fn tab_show(
    app: AppHandle,
    webview: tauri::Webview,
    id: Option<String>,
    split_id: Option<String>,
    splitId: Option<String>,
) {
    if webview.label() != "ui" {
        return;
    }
    let sec = split_id.or(splitId);
    // Hide autofill overlay when switching tabs
    if let Some(af) = app.get_webview("autofill") {
        let _ = af.hide();
    }
    {
        let state = app.state::<Layout>();
        let mut l = state.0.lock().unwrap();
        l.primary_tab = id;
        l.secondary_tab = sec;
    }
    relayout(&app);
}

/// Destroy the webview (frees its memory).
#[tauri::command]
async fn tab_close(app: AppHandle, webview: tauri::Webview, id: String, profile: Option<String>) {
    if webview.label() != "ui" {
        return;
    }
    if let Some(af) = app.get_webview("autofill") {
        let _ = af.hide();
    }
    {
        let state = app.state::<Layout>();
        let mut l = state.0.lock().unwrap();
        if l.primary_tab.as_deref() == Some(&id) {
            l.primary_tab = None;
        }
        if l.secondary_tab.as_deref() == Some(&id) {
            l.secondary_tab = None;
        }
    }
    if let Some(wv) = app.get_webview(&format!("tab-{id}")) {
        let _ = wv.close();
    }
    if let Some(p) = profile {
        if p.starts_with("temp_") {
            if let Ok(app_dir) = app.path().app_data_dir() {
                let temp_dir = app_dir.join("temp_profiles").join(safe(&p));
                let _ = std::fs::remove_dir_all(temp_dir);
            }
        }
    }
    relayout(&app);
}

#[tauri::command]
async fn tab_nav(app: AppHandle, webview: tauri::Webview, id: String, action: String, url: Option<String>) {
    if webview.label() != "ui" {
        return;
    }
    let Some(wv) = app.get_webview(&format!("tab-{id}")) else { return };
    match action.as_str() {
        "back" => { let _ = wv.eval("history.back()"); }
        "forward" => { let _ = wv.eval("history.forward()"); }
        "reload" => { let _ = wv.eval("location.reload()"); }
        "go" => {
            if let Some(u) = url.and_then(|u| u.parse::<Url>().ok()) {
                let _ = wv.navigate(u);
            }
        }
        _ => {}
    }
}

#[tauri::command]
async fn set_sidebar(app: AppHandle, webview: tauri::Webview, width: f64) {
    if webview.label() != "ui" {
        return;
    }
    {
        let state = app.state::<Layout>();
        state.0.lock().unwrap().sidebar = width;
    }
    relayout(&app);
}

#[tauri::command]
async fn set_top_bar(app: AppHandle, webview: tauri::Webview, height: f64) {
    if webview.label() != "ui" {
        return;
    }
    {
        let state = app.state::<Layout>();
        state.0.lock().unwrap().top_bar = height;
    }
    relayout(&app);
}

/// Opens a floating Glance (Quick Look) preview webview centered on screen.
#[tauri::command]
async fn glance_open(app: AppHandle, webview: tauri::Webview, url: String) -> Result<(), String> {
    if webview.label() != "ui" {
        return Err("Forbidden".into());
    }
    let parsed: Url = url.parse().map_err(|e| format!("Invalid address: {e}"))?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err("Only http(s) addresses are allowed".into());
    }
    let win = app.get_window("main").ok_or("no main window")?;
    if let Some(gw) = app.get_webview("glance") {
        let _ = gw.navigate(parsed);
        let _ = gw.show();
        relayout(&app);
        return Ok(());
    }
    let builder = WebviewBuilder::new("glance", WebviewUrl::External(parsed))
        .user_agent(CHROME_UA);
    win.add_child(builder, LogicalPosition::new(100.0, 100.0), LogicalSize::new(400.0, 400.0))
        .map_err(|e| e.to_string())?;
    relayout(&app);
    Ok(())
}

/// Closes the floating Glance preview webview.
#[tauri::command]
async fn glance_close(app: AppHandle, webview: tauri::Webview) {
    if webview.label() != "ui" {
        return;
    }
    if let Some(gw) = app.get_webview("glance") {
        let _ = gw.close();
    }
}

/// Set the exact position and size of the Glance floating webview based on the UI container rect.
#[tauri::command]
fn set_glance_bounds(app: AppHandle, x: f64, y: f64, width: f64, height: f64) {
    if let Some(win) = app.get_window("main") {
        let sf = win.scale_factor().unwrap_or(1.0);
        if let Some(gw) = app.get_webview("glance") {
            let px = (x * sf).round() as i32;
            let py = (y * sf).round() as i32;
            let pw = (width * sf).round().max(1.0) as u32;
            let ph = (height * sf).round().max(1.0) as u32;
            let _ = gw.set_position(PhysicalPosition::new(px, py));
            let _ = gw.set_size(PhysicalSize::new(pw, ph));
            let _ = gw.show();
        }
    }
}

/// Request opening a URL in the Glance Quick Look sheet (triggered from child webviews or injected scripts).
#[tauri::command]
async fn request_glance(app: AppHandle, url: String) {
    if let Some(ui) = app.get_webview("ui") {
        let _ = ui.emit("trigger-glance", url);
    }
}

/// Notify UI that a tab was clicked or focused (for active split pane synchronization).
#[tauri::command]
async fn tab_focused(app: AppHandle, id: String) {
    if let Some(ui) = app.get_webview("ui") {
        let _ = ui.emit("tab-focused", id);
    }
}

/// Toggle DevTools for a specific tab webview or the active tab.
#[tauri::command]
async fn tab_devtools(app: AppHandle, id: Option<String>) -> Result<(), String> {
    let label = if let Some(tab_id) = id {
        if tab_id == "ui" {
            "ui".to_string()
        } else {
            format!("tab-{tab_id}")
        }
    } else {
        let layout = app.state::<Layout>().0.lock().unwrap().clone();
        if let Some(p) = layout.primary_tab {
            format!("tab-{p}")
        } else {
            "ui".to_string()
        }
    };

    if let Some(wv) = app.get_webview(&label) {
        if wv.is_devtools_open() {
            wv.close_devtools();
        } else {
            wv.open_devtools();
        }
    }
    Ok(())
}

/// Evaluates JavaScript within a specific tab webview. Called only by the UI webview.
#[tauri::command]
async fn tab_eval(app: AppHandle, webview: tauri::Webview, id: String, js: String) -> Result<(), String> {
    if webview.label() != "ui" {
        return Err("Forbidden".into());
    }
    let wv = app.get_webview(&format!("tab-{id}")).ok_or("tab not found")?;
    wv.eval(&js).map_err(|e| e.to_string())?;
    Ok(())
}

/// Invoked from an injected script inside a tab when a password field is hovered or focused.
/// It verifies origin (https or localhost) and forwards coordinates to the UI webview.
#[tauri::command]
async fn autofill_field(app: AppHandle, webview: tauri::Webview, bounds: FieldBounds) -> Result<(), String> {
    let label = webview.label().to_string();
    if !label.starts_with("tab-") {
        return Err("Forbidden".into());
    }
    let tab_id = label.strip_prefix("tab-").unwrap().to_string();
    let url = webview.url().map_err(|e| e.to_string())?;
    let scheme = url.scheme();
    let host = url.host_str().unwrap_or("").to_string();
    let is_local = host == "localhost" || host == "127.0.0.1";
    if scheme != "https" && !is_local {
        return Ok(());
    }

    if let Some(ui) = app.get_webview("ui") {
        let _ = ui.emit("autofill-request", serde_json::json!({
            "tab_id": tab_id,
            "url": url.to_string(),
            "host": host,
            "bounds": bounds,
        }));
    }
    Ok(())
}

/// Shows and positions the autofill dropdown overlay below the targeted password field.
#[tauri::command]
async fn autofill_show(
    app: AppHandle,
    webview: tauri::Webview,
    tab_id: String,
    host: String,
    bounds: FieldBounds,
    items: Vec<AutofillItem>,
) -> Result<(), String> {
    if webview.label() != "ui" {
        return Err("Forbidden".into());
    }
    if items.is_empty() {
        if let Some(af) = app.get_webview("autofill") {
            let _ = af.hide();
        }
        return Ok(());
    }

    *app.state::<AutofillState>().0.lock().unwrap() = Some(AutofillContext {
        tab_id: tab_id.clone(),
        host,
    });

    let win = app.get_window("main").ok_or("no main window")?;
    let sf = win.scale_factor().unwrap_or(1.0);
    let layout = {
        let state = app.state::<Layout>();
        let val = state.0.lock().unwrap().clone();
        val
    };
    let sb = (layout.sidebar * sf).round() as i32;
    let top_bar = (layout.top_bar * sf).round() as i32;
    let pad = (PAD * sf).round() as i32;
    let win_size = win.inner_size().map_err(|e| e.to_string())?;

    let popup_x_logical = bounds.x.max(0.0);
    let popup_y_logical = bounds.y + 2.0;
    let popup_w_logical = bounds.width.clamp(240.0, 340.0);
    let row_height = 38.0;
    let manage_height = 32.0;
    let content_h = (items.len().saturating_sub(1) as f64 * row_height + manage_height + 2.0).clamp(42.0, 260.0);

    let mut phys_x = sb + (popup_x_logical * sf).round() as i32;
    let mut phys_y = pad + top_bar + (popup_y_logical * sf).round() as i32;
    let phys_w = (popup_w_logical * sf).round() as u32;
    let phys_h = (content_h * sf).round() as u32;

    if phys_x + phys_w as i32 > win_size.width as i32 - pad {
        phys_x = (win_size.width as i32 - pad - phys_w as i32).max(sb);
    }
    if phys_y + phys_h as i32 > win_size.height as i32 - pad {
        let above_y = pad + ((bounds.y - bounds.height - content_h - 2.0) * sf).round() as i32;
        if above_y >= pad {
            phys_y = above_y;
        }
    }

    let items_json = serde_json::to_string(&items).unwrap_or_default();

    if let Some(af) = app.get_webview("autofill") {
        let _ = af.set_position(PhysicalPosition::new(phys_x, phys_y));
        let _ = af.set_size(PhysicalSize::new(phys_w, phys_h));
        let _ = af.show();
        let _ = af.eval(format!("window.populate && window.populate({items_json});"));
        let _ = af.emit("autofill-populate", &items);
    } else {
        let builder = WebviewBuilder::new("autofill", WebviewUrl::App("autofill.html".into()));
        win.add_child(builder, LogicalPosition::new(0.0, 0.0), LogicalSize::new(10.0, 10.0))
            .map_err(|e| e.to_string())?;

        let a = app.clone();
        let items_copy = items.clone();
        std::thread::spawn(move || {
            for ms in [40u64, 120, 260] {
                std::thread::sleep(std::time::Duration::from_millis(ms));
                if let Some(af) = a.get_webview("autofill") {
                    let _ = af.set_position(PhysicalPosition::new(phys_x, phys_y));
                    let _ = af.set_size(PhysicalSize::new(phys_w, phys_h));
                    let _ = af.show();
                    let _ = af.eval(format!("window.populate && window.populate({items_json});"));
                    let _ = af.emit("autofill-populate", &items_copy);
                }
            }
        });
    }

    Ok(())
}

/// Invoked when a user selects an entry or action from the autofill overlay.
#[tauri::command]
async fn autofill_select(
    app: AppHandle,
    webview: tauri::Webview,
    action: String,
    index: Option<usize>,
) -> Result<(), String> {
    let label = webview.label().to_string();
    if !label.starts_with("tab-") && label != "autofill" && label != "ui" {
        return Err("Forbidden".into());
    }
    if let Some(af) = app.get_webview("autofill") {
        let _ = af.hide();
    }
    let ctx = app.state::<AutofillState>().0.lock().unwrap().clone();
    let (tab_id, host) = if label.starts_with("tab-") {
        let tid = label.strip_prefix("tab-").unwrap().to_string();
        let h = webview.url().ok().and_then(|u| u.host_str().map(|s| s.to_string())).unwrap_or_default();
        (tid, h)
    } else if let Some(ctx) = ctx {
        (ctx.tab_id, ctx.host)
    } else {
        (String::new(), String::new())
    };

    if let Some(ui) = app.get_webview("ui") {
        let _ = ui.emit("autofill-select", serde_json::json!({
            "action": action,
            "tab_id": tab_id,
            "host": host,
            "index": index,
        }));
    }

    // Give keyboard focus back to the tab so Enter submits the form
    if action == "fill" && !tab_id.is_empty() {
        if let Some(tab_wv) = app.get_webview(&format!("tab-{}", tab_id)) {
            let _ = tab_wv.set_focus();
        }
    }

    Ok(())
}

/// Dismisses and hides the autofill overlay.
#[tauri::command]
async fn autofill_dismiss(app: AppHandle) -> Result<(), String> {
    if let Some(af) = app.get_webview("autofill") {
        let _ = af.hide();
    }
    *app.state::<AutofillState>().0.lock().unwrap() = None;
    Ok(())
}

/// Window management commands (macOS traffic light buttons)
#[tauri::command]
fn win_drag(app: AppHandle) {
    if let Some(win) = app.get_window("main") {
        let _ = win.start_dragging();
    }
}

#[tauri::command]
async fn win_close(app: AppHandle) {
    if let Some(win) = app.get_window("main") {
        let _ = win.close();
    }
}

#[tauri::command]
async fn win_minimize(app: AppHandle) {
    if let Some(win) = app.get_window("main") {
        let _ = win.minimize();
    }
}

#[tauri::command]
async fn win_maximize(app: AppHandle) {
    if let Some(win) = app.get_window("main") {
        if let Ok(is_max) = win.is_maximized() {
            if is_max {
                let _ = win.unmaximize();
            } else {
                let _ = win.maximize();
            }
        }
    }
}

/// Opens a new window of the browser for a profile.
#[tauri::command]
async fn new_window(_app: AppHandle, profile: Option<String>) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut cmd = std::process::Command::new(exe);
    if let Some(p) = profile {
        cmd.arg("--profile").arg(p);
    }
    cmd.spawn().map_err(|e| e.to_string())?;
    Ok(())
}

fn dirs_fallback() -> Option<std::path::PathBuf> {
    std::env::var_os("USERPROFILE").map(|u| std::path::PathBuf::from(u).join("Downloads"))
}

/// Open a downloaded file with default OS application
#[tauri::command]
fn open_download_file(path: String) -> Result<(), String> {
    let p = std::path::Path::new(&path);
    let resolved = if p.exists() {
        path
    } else if let Some(dl) = dirs_fallback() {
        let candidate = dl.join(&path);
        if candidate.exists() {
            candidate.to_string_lossy().to_string()
        } else {
            return Err("File not found on disk".into());
        }
    } else {
        return Err("File not found on disk".into());
    };

    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", &resolved])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = std::process::Command::new("open").arg(&resolved).spawn().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Reveal a downloaded file inside Windows File Explorer
#[tauri::command]
fn show_download_folder(path: String) -> Result<(), String> {
    let p = std::path::Path::new(&path);
    let resolved = if p.exists() {
        path
    } else if let Some(dl) = dirs_fallback() {
        let candidate = dl.join(&path);
        if candidate.exists() {
            candidate.to_string_lossy().to_string()
        } else {
            dl.to_string_lossy().to_string()
        }
    } else {
        path
    };

    #[cfg(target_os = "windows")]
    {
        let norm = resolved.replace('/', "\\");
        let _ = std::process::Command::new("explorer")
            .args(["/select,", &norm])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = std::process::Command::new("open").arg(&resolved).spawn().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Clears cookies, cache, and session data for a specific profile only
#[tauri::command]
async fn clear_profile_data(app: AppHandle, webview: tauri::Webview, profile: String) -> Result<(), String> {
    if webview.label() != "ui" {
        return Err("Forbidden".into());
    }
    let p_safe = safe(&profile);
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("profiles")
        .join(p_safe);
    if dir.exists() {
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
    }
    Ok(())
}

/// Notify UI when page zoom changes via Ctrl + Mouse Wheel in any tab webview
#[tauri::command]
async fn tab_zoom_changed(app: AppHandle, id: String, zoom: f64) {
    if let Some(ui) = app.get_webview("ui") {
        let _ = ui.emit("tab-zoom", (id, zoom));
    }
}

/// Opens a dedicated OAuth or popup window sharing the active profile data directory
#[tauri::command]
async fn open_popup_window(
    app: AppHandle,
    url: String,
    profile: Option<String>,
    features: Option<String>,
) -> Result<(), String> {
    let parsed: Url = url.parse().map_err(|e| format!("Invalid address: {e}"))?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err("Only http(s) allowed".into());
    }

    let mut w = 540.0;
    let mut h = 680.0;
    if let Some(ref feat) = features {
        for part in feat.split(',') {
            let p = part.trim().to_lowercase();
            if let Some(val) = p.strip_prefix("width=") {
                if let Ok(v) = val.parse::<f64>() {
                    w = v.clamp(380.0, 1200.0);
                }
            } else if let Some(val) = p.strip_prefix("height=") {
                if let Ok(v) = val.parse::<f64>() {
                    h = v.clamp(400.0, 950.0);
                }
            }
        }
    }

    let p_name = profile.unwrap_or_else(|| "default".into());
    let parent_folder = if p_name.starts_with("temp_") {
        "temp_profiles"
    } else {
        "profiles"
    };
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join(parent_folder)
        .join(safe(&p_name));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let popup_label = format!("oauth-popup-{timestamp}");

    // Close any previous popup window
    for (label, win) in app.windows() {
        if label.starts_with("oauth-popup-") {
            let _ = win.close();
        }
    }

    let app2 = app.clone();
    let init_script = format!("window.__KIYO_IS_POPUP = true;\n{}", INJECT_SCRIPT);

    let popup_win = WebviewWindowBuilder::new(&app, &popup_label, WebviewUrl::External(parsed))
        .title("Sign In")
        .inner_size(w, h)
        .center()
        .always_on_top(true)
        .data_directory(dir)
        .user_agent(CHROME_UA)
        .initialization_script_for_all_frames(&init_script)
        .on_navigation(move |u| {
            let u_str = u.to_string();
            // If the popup navigates directly to app dashboard or chats, OAuth succeeded
            if u_str.contains("/chats") || u_str.contains("/app") || u_str.contains("/home") || u_str.contains("claude.ai/new") {
                let app_c = app2.clone();
                let dest = u_str.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(600));
                    for (label, win) in app_c.windows() {
                        if label.starts_with("oauth-popup-") {
                            let _ = win.close();
                        }
                    }
                    let state = app_c.state::<Layout>();
                    let primary = state.0.lock().unwrap().primary_tab.clone();
                    if let Some(tab_id) = primary {
                        if let Some(tab_wv) = app_c.get_webview(&format!("tab-{tab_id}")) {
                            let _ = tab_wv.eval(format!("window.location.href = {:?};", dest));
                        }
                    }
                    let _ = app_c.emit("oauth-completed", ());
                });
            }
            true
        })
        .build()
        .map_err(|e| e.to_string())?;

    let _ = popup_win.set_focus();

    Ok(())
}

#[tauri::command]
async fn oauth_popup_post_message(
    app: AppHandle,
    message: String,
    origin: Option<String>,
) -> Result<(), String> {
    let orig = origin.unwrap_or_else(|| "*".into());
    let _ = app.emit("oauth-message", serde_json::json!({ "message": &message, "origin": &orig }));

    let msg_str = serde_json::to_string(&message).unwrap_or_default();
    let orig_str = serde_json::to_string(&orig).unwrap_or_default();
    let js = format!(
        r#"(function() {{
            try {{
                let raw = {msg_str};
                let parsed = raw;
                try {{ parsed = JSON.parse(raw); }} catch (_) {{}}
                const ev = new MessageEvent('message', {{
                    data: parsed,
                    origin: {orig_str},
                    source: window
                }});
                window.dispatchEvent(ev);
            }} catch (e) {{}}
        }})();"#
    );

    for (label, wv) in app.webviews() {
        if label.starts_with("tab-") {
            let _ = wv.eval(&js);
        }
    }

    Ok(())
}

#[tauri::command]
async fn oauth_popup_navigate_opener(app: AppHandle, url: String) -> Result<(), String> {
    let state = app.state::<Layout>();
    let primary = state.0.lock().unwrap().primary_tab.clone();
    if let Some(tab_id) = primary {
        if let Some(tab_wv) = app.get_webview(&format!("tab-{tab_id}")) {
            if url == "reload" {
                let _ = tab_wv.eval("window.location.reload();");
            } else if let Ok(parsed) = url.parse::<Url>() {
                if parsed.scheme() == "http" || parsed.scheme() == "https" {
                    let _ = tab_wv.eval(format!("window.location.href = {:?};", parsed.as_str()));
                }
            } else {
                let _ = tab_wv.eval(format!("window.location.href = {:?};", url));
            }
        }
    }
    Ok(())
}

#[tauri::command]
async fn close_popup_window(app: AppHandle) -> Result<(), String> {
    for (label, win) in app.windows() {
        if label.starts_with("oauth-popup-") {
            let _ = win.close();
        }
    }
    let app_c = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(400));
        let _ = app_c.emit("oauth-completed", ());
        let _ = app_c.emit("oauth-closed", ());
    });
    Ok(())
}

#[tauri::command]
async fn toggle_command_palette(app: AppHandle) -> Result<(), String> {
    if let Some(ui) = app.get_webview("ui") {
        let _ = ui.emit("toggle-palette", ());
    }
    Ok(())
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
struct CtxMenuItem {
    id: String,
    label: Option<String>,
    accelerator: Option<String>,
    separator: Option<bool>,
    enabled: Option<bool>,
    sub: Option<Vec<CtxMenuItem>>,
}

#[tauri::command]
fn show_context_menu(
    app: AppHandle,
    x: f64,
    y: f64,
    items: Vec<CtxMenuItem>,
) -> Result<(), String> {
    let win = app.get_window("main").ok_or("No main window")?;
    let menu = Menu::new(&app).map_err(|e| e.to_string())?;

    for it in &items {
        if it.separator.unwrap_or(false) {
            let sep = PredefinedMenuItem::separator(&app).map_err(|e| e.to_string())?;
            menu.append(&sep).map_err(|e| e.to_string())?;
        } else if let Some(ref sub_items) = it.sub {
            let label = it.label.as_deref().unwrap_or("More");
            let sub = Submenu::new(&app, label, it.enabled.unwrap_or(true)).map_err(|e| e.to_string())?;
            for s_it in sub_items {
                if s_it.separator.unwrap_or(false) {
                    let sep = PredefinedMenuItem::separator(&app).map_err(|e| e.to_string())?;
                    sub.append(&sep).map_err(|e| e.to_string())?;
                } else {
                    let s_label = s_it.label.as_deref().unwrap_or("");
                    let s_mi = MenuItem::with_id(&app, &s_it.id, s_label, s_it.enabled.unwrap_or(true), None::<&str>).map_err(|e| e.to_string())?;
                    sub.append(&s_mi).map_err(|e| e.to_string())?;
                }
            }
            menu.append(&sub).map_err(|e| e.to_string())?;
        } else {
            let label = it.label.as_deref().unwrap_or("");
            let mi = MenuItem::with_id(&app, &it.id, label, it.enabled.unwrap_or(true), None::<&str>).map_err(|e| e.to_string())?;
            menu.append(&mi).map_err(|e| e.to_string())?;
        }
    }

    menu.popup_at(win, LogicalPosition::new(x, y)).map_err(|e| e.to_string())?;
    Ok(())
}

fn main() {
    #[cfg(target_os = "windows")]
    {
        std::env::set_var(
            "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
            "--enable-features=ParallelDownloading,CanvasOopRasterization,VaapiVideoDecoder \
             --enable-gpu-rasterization --enable-zero-copy --ignore-gpu-blocklist \
             --disable-background-timer-throttling --enable-quic --enable-tcp-fastopen",
        );
    }
    tauri::Builder::default()
        .manage(Layout(Mutex::new(LayoutState {
            sidebar: 260.0,
            top_bar: 0.0,
            primary_tab: None,
            secondary_tab: None,
        })))
        .manage(AutofillState(Mutex::new(None)))
        .invoke_handler(tauri::generate_handler![
            tab_open,
            tab_show,
            tab_close,
            tab_nav,
            set_sidebar,
            set_top_bar,
            tab_eval,
            autofill_field,
            autofill_show,
            autofill_select,
            autofill_dismiss,
            new_window,
            glance_open,
            glance_close,
            set_glance_bounds,
            request_glance,
            tab_focused,
            tab_devtools,
            win_close,
            win_minimize,
            win_maximize,
            win_drag,
            open_download_file,
            show_download_folder,
            clear_profile_data,
            tab_zoom_changed,
            open_popup_window,
            close_popup_window,
            oauth_popup_post_message,
            oauth_popup_navigate_opener,
            show_context_menu,
            toggle_command_palette
        ])
        .on_menu_event(|app, event| {
            if let Some(ui) = app.get_webview("ui") {
                let _ = ui.emit("menu-item-clicked", event.id().as_ref());
            }
        })
        .on_window_event(|w, e| {
            if let WindowEvent::Resized(_) = e {
                relayout(w.app_handle());
                if let Ok(is_max) = w.is_maximized() {
                    let _ = w.emit("win-maximized", is_max);
                }
            } else if let WindowEvent::Destroyed = e {
                if w.label().starts_with("oauth-popup-") {
                    let _ = w.emit("oauth-closed", ());
                }
            }
        })
        .setup(|app| {
            if let Ok(app_dir) = app.path().app_data_dir() {
                let temp_dir = app_dir.join("temp_profiles");
                if temp_dir.exists() {
                    let _ = std::fs::remove_dir_all(temp_dir);
                }
            }
            let win = WindowBuilder::new(app, "main")
                .title("Kiyo")
                .inner_size(1280.0, 800.0)
                .min_inner_size(900.0, 600.0)
                .decorations(false)
                .build()?;
            win.add_child(
                WebviewBuilder::new("ui", WebviewUrl::App("index.html".into())),
                LogicalPosition::new(0.0, 0.0),
                LogicalSize::new(1280.0, 800.0),
            )?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error running Kiyo");
}
