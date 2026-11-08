use crate::{library::LibrarySource, photokit::PhotoKit};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
pub struct Backend {
    pub data: PathBuf,
    pub cache: PathBuf,
    pub opened: Mutex<HashSet<PathBuf>>,
    locks: Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>,
    pub sips: Arc<tokio::sync::Semaphore>,
    pub library: Arc<LibrarySource>,
}
impl Backend {
    pub fn new(data: PathBuf, cache: PathBuf) -> Self {
        let cores = std::process::Command::new("/usr/sbin/sysctl")
            .args(["-n", "hw.perflevel0.physicalcpu"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse::<usize>().ok())
            .unwrap_or(1)
            .clamp(1, 6);
        Self {
            library: Arc::new(LibrarySource::new(
                Arc::new(PhotoKit),
                data.clone(),
                cache.clone(),
            )),
            data,
            cache,
            opened: Mutex::default(),
            locks: Mutex::default(),
            sips: Arc::new(tokio::sync::Semaphore::new(cores)),
        }
    }
    pub fn lock(&self, folder: &Path) -> Result<Arc<Mutex<()>>, String> {
        Ok(self
            .locks
            .lock()
            .map_err(|_| "Lagringen er utilgjengelig.")?
            .entry(folder.to_path_buf())
            .or_default()
            .clone())
    }
    pub fn opened_folder(&self, path: &str) -> Result<PathBuf, String> {
        let p = crate::scan::folder(path)?;
        if !self
            .opened
            .lock()
            .map_err(|_| "Mappene er utilgjengelige.")?
            .contains(&p)
        {
            return Err("Åpne mappen først.".into());
        }
        Ok(p)
    }
    pub fn authorized_file(&self, path: &str) -> Result<PathBuf, String> {
        let p = Path::new(path);
        if !p.is_absolute() {
            return Err("Filstien må være absolutt.".into());
        }
        let parent = p.parent().ok_or("Ugyldig filsti.")?;
        let folder = self.opened_folder(parent.to_str().ok_or("Ugyldig filsti.")?)?;
        crate::scan::file(
            &folder,
            p.file_name()
                .and_then(|n| n.to_str())
                .ok_or("Ugyldig filnavn.")?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_paths() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("photos");
        std::fs::create_dir(&folder).unwrap();
        let folder = std::fs::canonicalize(folder).unwrap();
        let photo = folder.join("a.jpg");
        std::fs::write(&photo, b"x").unwrap();
        let state = Backend::new(root.path().join("data"), root.path().join("cache"));
        assert!(state.authorized_file(photo.to_str().unwrap()).is_err());
        state.opened.lock().unwrap().insert(folder.clone());
        assert!(state.authorized_file(photo.to_str().unwrap()).is_ok());
        assert!(state.authorized_file("a.jpg").is_err());
        assert!(
            state
                .authorized_file(folder.join("../outside.jpg").to_str().unwrap())
                .is_err()
        );
        let nested = folder.join("nested");
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(nested.join("a.jpg"), b"x").unwrap();
        assert!(
            state
                .authorized_file(nested.join("a.jpg").to_str().unwrap())
                .is_err()
        );
        std::os::unix::fs::symlink(&photo, folder.join("link.jpg")).unwrap();
        assert!(
            state
                .authorized_file(folder.join("link.jpg").to_str().unwrap())
                .is_err()
        );
    }
}
