// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use lazy_static::lazy_static;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use tauri::{AppHandle, Manager, SystemTray, SystemTrayEvent};
use tauri::{CustomMenuItem, SystemTrayMenu, SystemTrayMenuItem, SystemTraySubmenu};

use aw_server::endpoints::build_rocket;

mod manager;

static HANDLE: OnceLock<Mutex<AppHandle>> = OnceLock::new();
lazy_static! {
    static ref HANDLE_CONDVAR: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());
}

fn init_app_handle(handle: AppHandle) {
    HANDLE.get_or_init(|| Mutex::new(handle));
    let (lock, cvar) = &*HANDLE_CONDVAR;
    let mut started = lock.lock().unwrap();
    *started = true;
    cvar.notify_all();
}

pub(crate) fn get_app_handle() -> &'static Mutex<AppHandle> {
    HANDLE.get().unwrap()
}

// Learn more about Tauri commands at https://tauri.app/v1/guides/features/command
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

fn main() {
    let testing = true;

    let mut config = aw_server::config::create_config(testing);
    config.port = 5699;
    let db_path = aw_server::dirs::db_path(testing)
        .expect("Failed to get db path")
        .to_str()
        .unwrap()
        .to_string();

    let device_id = aw_server::device_id::get_device_id();
    let manager_state = manager::start_manager();
    let tray = create_tray(&manager_state);
    tauri::Builder::default()
        .setup(|app| {
            init_app_handle(app.handle().clone());
            let webui_var = std::env::var("AW_WEBUI_DIR");
            let asset_path_opt = if let Ok(path_str) = &webui_var {
                let asset_path = PathBuf::from(&path_str);
                if asset_path.exists() {
                    println!("Using webui path: {}", path_str);
                    Some(asset_path)
                } else {
                    panic!("Path set via env var AW_WEBUI_DIR does not exist");
                }
            } else {
                println!("Using bundled assets");
                None
            };

            let legacy_import = false;
            let server_state = aw_server::endpoints::ServerState {
                // Even if legacy_import is set to true it is disabled on Android so
                // it will not happen there
                datastore: Mutex::new(aw_datastore::Datastore::new(db_path, legacy_import)),
                asset_resolver: aw_server::endpoints::AssetResolver::new(asset_path_opt),
                device_id,
            };

            tauri::async_runtime::spawn(build_rocket(server_state, config).launch());
            Ok(())
        })
        .system_tray(tray)
        .on_system_tray_event(move |app, event| on_tray_event(app, event, &manager_state))
        .on_window_event(|event| match event.event() {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                event.window().hide().unwrap();
                api.prevent_close();
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![greet])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|_app_handle, event| match event {
            tauri::RunEvent::ExitRequested { api, .. } => {
                api.prevent_exit();
            }
            _ => {}
        });
}

fn create_tray_menu(manager_state: &Arc<Mutex<manager::ManagerState>>) -> SystemTrayMenu {
    // here `"quit".to_string()` defines the menu item id, and the second parameter is the menu item label.
    let open = CustomMenuItem::new("open".to_string(), "Open");
    let quit = CustomMenuItem::new("quit".to_string(), "Quit");

    // modules
    let mut module_menu = SystemTrayMenu::new();

    let state = manager_state.lock().unwrap();
    println!("state: {:?}", state);
    for (module, running) in state.modules_running.iter() {
        let label = format!(
            "{} ({})",
            module,
            if *running { "Running" } else { "Stopped" }
        );
        module_menu = module_menu.add_item(CustomMenuItem::new(module.clone(), &label));
    }

    let module_submenu = SystemTraySubmenu::new("Modules", module_menu);

    SystemTrayMenu::new()
        .add_item(open)
        .add_native_item(SystemTrayMenuItem::Separator)
        .add_submenu(module_submenu)
        .add_native_item(SystemTrayMenuItem::Separator)
        .add_item(quit)
}

fn create_tray(manager_state: &Arc<Mutex<manager::ManagerState>>) -> SystemTray {
    let tray_menu = create_tray_menu(manager_state);
    SystemTray::new().with_menu(tray_menu)
}

fn on_tray_event(
    app: &AppHandle,
    event: SystemTrayEvent,
    manager_state: &Arc<Mutex<manager::ManagerState>>,
) {
    match event {
        SystemTrayEvent::DoubleClick {
            position: _,
            size: _,
            ..
        } => {
            println!("system tray received a double click");
            let window = app.get_window("main").unwrap();
            window.show().unwrap();
        }
        SystemTrayEvent::MenuItemClick { id, .. } => match id.as_str() {
            "quit" => {
                println!("system tray received a quit click");
                let state = manager_state.lock().unwrap();
                state.stop_modules();
                app.exit(0);
            }
            "open" => {
                println!("system tray received a open click");
                let window = app.get_window("main").unwrap();
                window.show().unwrap();
            }
            _ => {
                println!("system tray received a module click at {}", id.as_str());
                let mut state = manager_state.lock().unwrap();
                state.handle_system_click(id.as_str());
            }
        },
        _ => {}
    }
}
