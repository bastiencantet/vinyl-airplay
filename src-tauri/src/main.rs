#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};
use tauri::{
    Emitter, Manager,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use vinyl_airplay::control::Controller;

struct Desktop {
    controller: Controller,
    quitting: AtomicBool,
}

#[tauri::command]
async fn control_request(
    state: tauri::State<'_, Desktop>,
    operation: String,
    payload: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    state
        .controller
        .request(&operation, payload)
        .await
        .map_err(|e| format!("{e:#}"))
}
#[tauri::command]
async fn track_artwork(state: tauri::State<'_, Desktop>) -> Result<Vec<u8>, String> {
    state
        .controller
        .artwork()
        .await
        .map_err(|e| format!("{e:#}"))
}
#[tauri::command]
fn desktop_info() -> serde_json::Value {
    serde_json::json!({"platform":std::env::consts::OS,"version":env!("CARGO_PKG_VERSION"),"background":true})
}
fn show(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        let _ = window.emit("desktop-visible", true);
    }
}
fn sender_path() -> anyhow::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let sibling = exe
        .parent()
        .unwrap()
        .join(format!("vinyl-airplay{}", std::env::consts::EXE_SUFFIX));
    if sibling.is_file() {
        return Ok(sibling);
    }
    #[cfg(debug_assertions)]
    {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
            "binaries/vinyl-airplay-{}{}",
            env!("VINYL_TARGET"),
            std::env::consts::EXE_SUFFIX
        ));
        if path.is_file() {
            return Ok(path);
        }
    }
    anyhow::bail!("Le moteur audio embarqué est absent. Réinstalle l’application.")
}
fn main() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| show(app)))
        .invoke_handler(tauri::generate_handler![
            control_request,
            track_artwork,
            desktop_info
        ])
        .setup(|app| {
            let root = app.path().app_data_dir()?;
            std::fs::create_dir_all(&root)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))?;
            }
            let token = format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            );
            let controller = tauri::async_runtime::block_on(Controller::open(
                root,
                sender_path()?,
                token,
                None,
                None,
            ))?;
            app.manage(Desktop {
                controller,
                quitting: AtomicBool::new(false),
            });
            let open = MenuItem::with_id(app, "show", "Ouvrir Vinyl AirPlay", true, None::<&str>)?;
            let stop = MenuItem::with_id(app, "stop", "Arrêter la diffusion", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quitter", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &stop, &quit])?;
            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("Vinyl AirPlay")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show(app),
                    "stop" => {
                        let app = app.clone();
                        tauri::async_runtime::spawn(async move {
                            let _ = app.state::<Desktop>().controller.disable().await;
                        });
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show(tray.app_handle());
                    }
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.emit("desktop-visible", false);
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("Démarrer Vinyl AirPlay");
    app.run(|app, event| {
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Reopen {
            has_visible_windows: false,
            ..
        } = event
        {
            show(app);
        }
        if let tauri::RunEvent::ExitRequested { api, .. } = event {
            let state = app.state::<Desktop>();
            if !state.quitting.swap(true, Ordering::SeqCst) {
                api.prevent_exit();
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = app.state::<Desktop>().controller.stop().await;
                    app.exit(0);
                });
            }
        }
    });
}
