mod commands;
mod decisions;
mod library;
mod menu;
mod photokit;
mod scan;
mod state;
mod thumbs;
mod trash;
use tauri::{Emitter, Manager};
/// The page may only ever be the app itself, so it can't be sent to a site with photos.
fn navigation_allowed(url: &tauri::Url) -> bool {
    match url.scheme() {
        "tauri" => url.host_str() == Some("localhost"),
        "http" => {
            cfg!(debug_assertions)
                && url.host_str() == Some("localhost")
                && url.port() == Some(1420)
        }
        _ => false,
    }
}
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri::plugin::Builder::<tauri::Wry>::new("navigation-guard")
                .on_navigation(|_, url| navigation_allowed(url))
                .build(),
        )
        .setup(|app| {
            let data = app.path().app_data_dir()?;
            let cache = app.path().app_cache_dir()?.join("thumbnails");
            std::fs::create_dir_all(&data)?;
            std::fs::create_dir_all(&cache)?;
            app.asset_protocol_scope().allow_directory(&cache, false)?;
            let backend = state::Backend::new(data, cache);
            for dir in [
                backend.library.previews_dir(),
                backend.library.thumbnails_dir(),
            ] {
                std::fs::create_dir_all(dir)?;
                app.asset_protocol_scope().allow_directory(dir, false)?;
            }
            app.manage(backend);
            let menu = menu::build(app.handle())?;
            app.set_menu(menu)?;
            Ok(())
        })
        .on_menu_event(|app, event| {
            let id = event.id().as_ref();
            if menu::is_action(app, id)
                && let Err(e) = app.emit("menu-action", id)
            {
                eprintln!("Kan ikke sende menyhandling: {e}");
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::open_folder,
            commands::last_folder,
            commands::set_decisions,
            commands::thumbnail,
            commands::preview,
            commands::trash_photos,
            commands::reveal_in_finder,
            commands::library_status,
            commands::open_photos_privacy_settings,
            menu::set_menu_context
        ])
        .run(tauri::generate_context!())
        .expect("Kunne ikke starte Bildesveip");
}

#[cfg(test)]
mod tests {
    use super::navigation_allowed;
    #[test]
    fn navigation_stays_inside_the_app() {
        let allowed = |u: &str| navigation_allowed(&tauri::Url::parse(u).unwrap());
        assert!(allowed("tauri://localhost/index.html"));
        assert!(!allowed("https://example.com/"));
        assert!(!allowed("tauri://example.com/"));
        assert!(!allowed("http://localhost:9999/"));
        assert!(!allowed("file:///etc/hosts"));
    }
}
