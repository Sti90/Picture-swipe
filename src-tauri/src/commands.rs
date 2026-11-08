use crate::{
    decisions::{self, Decision},
    library, scan,
    state::Backend,
    thumbs, trash,
};
use tauri::Manager;
pub async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("Arbeidet kunne ikke fullføres: {e}"))?
}
#[tauri::command]
pub async fn open_folder(app: tauri::AppHandle, path: String) -> Result<scan::FolderState, String> {
    blocking(move || {
        let state = app.state::<Backend>();
        if library::is_library(&path) {
            return state.library.open();
        }
        let folder = scan::folder(&path)?;
        let lock = state.lock(&folder)?;
        let _guard = lock.lock().map_err(|_| "Lagringen er utilgjengelig.")?;
        let mut result = scan::scan(&folder)?;
        let location = decisions::location(&state.data, &folder);
        let (mut saved, state_reset) = decisions::load_checked(&location)?;
        result.state_reset = state_reset;
        if scan::apply_saved(&mut result.photos, &mut saved) {
            decisions::save(&location, &saved)?;
        }
        app.asset_protocol_scope()
            .allow_directory(&folder, false)
            .map_err(|e| format!("Kan ikke gi tilgang til bildene: {e}"))?;
        decisions::atomic_write(
            &state.data.join("last-folder.json"),
            &serde_json::to_vec(&result.folder)
                .map_err(|e| format!("Kan ikke lagre mappe: {e}"))?,
        )?;
        state
            .opened
            .lock()
            .map_err(|_| "Mappene er utilgjengelige.")?
            .insert(folder);
        Ok(result)
    })
    .await
}
#[tauri::command]
pub async fn last_folder(app: tauri::AppHandle) -> Result<Option<String>, String> {
    blocking(move || {
        let state = app.state::<Backend>();
        let path = state.data.join("last-folder.json");
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("Kan ikke lese siste mappe: {e}")),
        };
        let saved: Option<String> = serde_json::from_slice(&bytes).ok();
        Ok(saved.filter(|p| {
            if library::is_library(p) {
                state.library.may_reopen()
            } else {
                std::path::Path::new(p).is_dir()
            }
        }))
    })
    .await
}
#[tauri::command]
pub async fn set_decisions(
    app: tauri::AppHandle,
    folder: String,
    names: Vec<String>,
    versions: Vec<decisions::Fingerprint>,
    decision: Decision,
) -> Result<(), String> {
    blocking(move || {
        let state = app.state::<Backend>();
        if library::is_library(&folder) {
            return state.library.set_decisions(names, versions, decision);
        }
        let folder = state.opened_folder(&folder)?;
        let lock = state.lock(&folder)?;
        let _guard = lock.lock().map_err(|_| "Lagringen er utilgjengelig.")?;
        let path = decisions::location(&state.data, &folder);
        let mut values = decisions::load(&path)?;
        decisions::record(&folder, &mut values, names, versions, decision)?;
        decisions::save(&path, &values)
    })
    .await
}
#[tauri::command]
pub async fn thumbnail(app: tauri::AppHandle, path: String) -> Result<String, String> {
    if library::is_asset(&path) {
        return library_image(app, path, false).await;
    }
    thumbs::image(app, path, 512, false).await
}
#[tauri::command]
pub async fn preview(app: tauri::AppHandle, path: String) -> Result<String, String> {
    if library::is_asset(&path) {
        return library_image(app, path, true).await;
    }
    thumbs::image(app, path, 2400, true).await
}
#[tauri::command]
pub async fn trash_photos(
    app: tauri::AppHandle,
    folder: String,
    names: Vec<String>,
) -> Result<trash::TrashResult, String> {
    blocking(move || {
        let state = app.state::<Backend>();
        if library::is_library(&folder) {
            return state.library.trash(&names);
        }
        let folder = state.opened_folder(&folder)?;
        let lock = state.lock(&folder)?;
        let _guard = lock.lock().map_err(|_| "Lagringen er utilgjengelig.")?;
        trash::perform(
            &folder,
            &names,
            &decisions::location(&state.data, &folder),
            trash::system_delete,
        )
    })
    .await
}
async fn library_image(
    app: tauri::AppHandle,
    path: String,
    preview: bool,
) -> Result<String, String> {
    let state = app.state::<Backend>();
    let source = state.library.clone();
    // Separate limits, so a slow iCloud download can't starve the grid.
    let permits = if preview {
        source.preview_permits.clone()
    } else {
        state.sips.clone()
    };
    let permit = permits
        .acquire_owned()
        .await
        .map_err(|_| "Bildekonverteringen er utilgjengelig.")?;
    blocking(move || {
        let _permit = permit;
        source.image(&path, preview)
    })
    .await
}
#[tauri::command]
pub async fn library_status(app: tauri::AppHandle) -> Result<library::Status, String> {
    blocking(move || Ok(app.state::<Backend>().library.status())).await
}
#[tauri::command]
pub async fn open_photos_privacy_settings() -> Result<(), String> {
    blocking(|| {
        let status = std::process::Command::new("/usr/bin/open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Photos")
            .status()
            .map_err(|e| format!("Kan ikke åpne Systeminnstillinger: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err("Kan ikke åpne Systeminnstillinger.".into())
        }
    })
    .await
}
#[tauri::command]
pub async fn reveal_in_finder(app: tauri::AppHandle, path: String) -> Result<(), String> {
    blocking(move || {
        if library::is_library_path(&path) {
            return Err("Bilder i biblioteket kan ikke vises i Finder.".into());
        }
        let path = app.state::<Backend>().authorized_file(&path)?;
        let status = std::process::Command::new("/usr/bin/open")
            .arg("-R")
            .arg(path)
            .status()
            .map_err(|e| format!("Kan ikke vise i Finder: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err("Kan ikke vise filen i Finder.".into())
        }
    })
    .await
}
