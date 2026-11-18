//! Everything that touches the disk or PhotoKit runs on one thread, in order. That makes a
//! quick undo land after the choice it reverses, and an open or a move wait for every
//! choice before it. It also keeps the main thread free, which PhotoKit needs for its dialogs.
use std::{
    path::{Path, PathBuf},
    process::Command as Process,
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
    thread::JoinHandle,
};

use eframe::egui::Context;

use crate::{
    decisions::{self, Decision, Fingerprint},
    library::{self, Library, LibrarySource, Status},
    photokit::PhotoKit,
    scan::{self, FolderState},
    trash::{self, TrashResult},
};

pub const BUNDLE_ID: &str = "io.github.sti90.bildesveip";

pub struct Paths {
    pub data: PathBuf,
    pub cache: PathBuf,
}

impl Paths {
    pub fn standard() -> Option<Self> {
        let home = PathBuf::from(std::env::var_os("HOME")?);
        let library = home.join("Library");
        Some(Self {
            data: library.join("Application Support").join(BUNDLE_ID),
            cache: library.join("Caches").join(BUNDLE_ID).join("thumbnails"),
        })
    }
}

/// Decisions and files. Takes the folder from the caller only as a saved-state key: the
/// safety checks for moving a file live in `trash::perform` and `LibrarySource::trash`.
pub struct Store {
    data: PathBuf,
    pub library: Arc<LibrarySource>,
}

impl Store {
    pub fn new(data: PathBuf, cache: PathBuf, photos: Arc<dyn Library>) -> Self {
        Self {
            library: Arc::new(LibrarySource::new(photos, data.clone(), cache)),
            data,
        }
    }

    pub fn open(&self, path: &str) -> Result<FolderState, String> {
        if library::is_library(path) {
            return self.library.open();
        }
        let folder = scan::folder(path)?;
        let mut result = scan::scan(&folder)?;
        let location = decisions::location(&self.data, &folder);
        let (mut saved, state_reset) = decisions::load_checked(&location)?;
        result.state_reset = state_reset;
        if scan::apply_saved(&mut result.photos, &mut saved, true) {
            decisions::save(&location, &saved)?;
        }
        decisions::atomic_write(
            &self.data.join("last-folder.json"),
            &serde_json::to_vec(&result.folder)
                .map_err(|e| format!("Kan ikke lagre mappe: {e}"))?,
        )?;
        Ok(result)
    }

    /// Never prompts: the library is only reopened if access was already granted.
    pub fn last_folder(&self) -> Option<String> {
        let bytes = std::fs::read(self.data.join("last-folder.json")).ok()?;
        let saved: String = serde_json::from_slice(&bytes).ok()?;
        let usable = if library::is_library(&saved) {
            self.library.may_reopen()
        } else {
            Path::new(&saved).is_dir()
        };
        usable.then_some(saved)
    }

    pub fn set_decisions(
        &self,
        folder: &str,
        names: Vec<String>,
        versions: Vec<Fingerprint>,
        decision: Decision,
    ) -> Result<(), String> {
        if library::is_library(folder) {
            return self.library.set_decisions(names, versions, decision);
        }
        let folder = scan::folder(folder)?;
        let path = decisions::location(&self.data, &folder);
        let mut values = decisions::load(&path)?;
        decisions::record(&folder, &mut values, names, versions, decision)?;
        decisions::save(&path, &values)
    }

    pub fn trash(
        &self,
        folder: &str,
        names: &[String],
        versions: &[Fingerprint],
    ) -> Result<TrashResult, String> {
        if library::is_library(folder) {
            return self.library.trash(names, versions);
        }
        let opened = folder;
        let folder = scan::folder(opened)?;
        // `opened` is the resolved path from when the folder was opened. If it now leads
        // elsewhere, the confirmation was about another folder.
        if folder.as_os_str() != opened {
            return Err("Mappen er flyttet eller endret. Åpne den på nytt.".into());
        }
        trash::perform(
            &folder,
            names,
            versions,
            &decisions::location(&self.data, &folder),
            trash::system_delete,
        )
    }
}

pub enum Command {
    Open {
        request: u64,
        path: String,
    },
    Write {
        token: u64,
        folder: String,
        names: Vec<String>,
        versions: Vec<Fingerprint>,
        decision: Decision,
    },
    Trash {
        folder: String,
        names: Vec<String>,
        versions: Vec<Fingerprint>,
    },
}

pub enum Reply {
    Opened {
        request: u64,
        result: Result<FolderState, String>,
        access: Option<Status>,
    },
    Written {
        token: u64,
        result: Result<(), String>,
    },
    Trashed {
        result: Result<TrashResult, String>,
    },
}

impl Command {
    /// The reply for a command that crashed, so the UI rolls back instead of waiting forever.
    fn failed(&self) -> Reply {
        let error = "En intern feil avbrøt handlingen. Åpne mappen på nytt.".to_owned();
        match self {
            Command::Open { request, .. } => Reply::Opened {
                request: *request,
                result: Err(error),
                access: None,
            },
            Command::Write { token, .. } => Reply::Written {
                token: *token,
                result: Err(error),
            },
            Command::Trash { .. } => Reply::Trashed { result: Err(error) },
        }
    }
}

/// A panic in one command must not take the worker down: every later choice would then
/// look saved without being written.
fn handle_guarded(store: &Store, command: Command) -> Reply {
    let failed = command.failed();
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle(store, command)))
        .unwrap_or(failed)
}

fn handle(store: &Store, command: Command) -> Reply {
    match command {
        Command::Open { request, path } => {
            let result = store.open(&path);
            let access =
                (result.is_err() && library::is_library(&path)).then(|| store.library.status());
            Reply::Opened {
                request,
                result,
                access,
            }
        }
        Command::Write {
            token,
            folder,
            names,
            versions,
            decision,
        } => Reply::Written {
            token,
            result: store.set_decisions(&folder, names, versions, decision),
        },
        Command::Trash {
            folder,
            names,
            versions,
        } => Reply::Trashed {
            result: store.trash(&folder, &names, &versions),
        },
    }
}

/// The app's connection to the worker thread.
pub struct Backend {
    commands: Option<Sender<Command>>,
    pub replies: Receiver<Reply>,
    worker: Option<JoinHandle<()>>,
    pub store: Arc<Store>,
}

impl Backend {
    pub fn start(ctx: Context, paths: Paths) -> Self {
        let store = Arc::new(Store::new(paths.data, paths.cache, Arc::new(PhotoKit)));
        let (send, commands) = mpsc::channel::<Command>();
        let (reply, replies) = mpsc::channel();
        let worker_store = store.clone();
        let worker = std::thread::spawn(move || {
            for command in commands {
                if reply.send(handle_guarded(&worker_store, command)).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
        });
        Self {
            commands: Some(send),
            replies,
            worker: Some(worker),
            store,
        }
    }

    pub fn send(&self, command: Command) {
        if let Some(commands) = &self.commands {
            let _ = commands.send(command);
        }
    }

    /// Finishes every queued write before returning, so quitting can't lose a choice.
    pub fn shutdown(&mut self) {
        self.commands = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub fn reveal_in_finder(path: &str) {
    let _ = Process::new("/usr/bin/open").arg("-R").arg(path).status();
}

pub fn open_photos_privacy_settings() {
    let _ = Process::new("/usr/bin/open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Photos")
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{Asset, DeleteOutcome};

    struct NoPhotos;
    impl Library for NoPhotos {
        fn status(&self) -> Status {
            Status::Denied
        }
        fn request_access(&self) -> Status {
            Status::Denied
        }
        fn list(&self) -> Result<Vec<Asset>, String> {
            Ok(Vec::new())
        }
        fn delete(&self, _: &[(String, u64)]) -> DeleteOutcome {
            panic!("must not delete")
        }
        fn render(&self, _: &str, _: u32, _: bool, _: &Path) -> Result<(), String> {
            Err("none".into())
        }
    }

    fn store(root: &Path) -> Store {
        Store::new(root.join("data"), root.join("cache"), Arc::new(NoPhotos))
    }

    struct Crashing;
    impl Library for Crashing {
        fn status(&self) -> Status {
            Status::Authorized
        }
        fn request_access(&self) -> Status {
            Status::Authorized
        }
        fn list(&self) -> Result<Vec<Asset>, String> {
            panic!("simulated crash")
        }
        fn delete(&self, _: &[(String, u64)]) -> DeleteOutcome {
            panic!("must not delete")
        }
        fn render(&self, _: &str, _: u32, _: bool, _: &Path) -> Result<(), String> {
            Err("none".into())
        }
    }

    #[test]
    fn a_crashing_command_fails_alone_and_the_next_one_runs() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("photos");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("a.jpg"), b"x").unwrap();
        let store = Store::new(
            root.path().join("data"),
            root.path().join("cache"),
            Arc::new(Crashing),
        );
        let open = |path: &str| Command::Open {
            request: 7,
            path: path.into(),
        };
        let Reply::Opened {
            request, result, ..
        } = handle_guarded(&store, open(library::LIBRARY_PATH))
        else {
            panic!("wrong reply")
        };
        assert_eq!(request, 7);
        assert!(result.is_err());
        let Reply::Opened { result, .. } = handle_guarded(&store, open(folder.to_str().unwrap()))
        else {
            panic!("wrong reply")
        };
        assert_eq!(result.unwrap().photos.len(), 1);
    }

    #[test]
    fn a_folder_path_that_now_leads_elsewhere_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let (a, b) = (root.path().join("a"), root.path().join("b"));
        for dir in [&a, &b] {
            std::fs::create_dir(dir).unwrap();
            std::fs::write(dir.join("x.jpg"), b"x").unwrap();
        }
        let store = store(root.path());
        let opened = store.open(a.to_str().unwrap()).unwrap();
        let x = &opened.photos[0];
        std::fs::rename(&a, root.path().join("old")).unwrap();
        std::os::unix::fs::symlink(&b, &a).unwrap();
        let result = store.trash(
            &opened.folder,
            std::slice::from_ref(&x.name),
            &[x.fingerprint],
        );
        assert!(result.unwrap_err().contains("Åpne den på nytt"));
    }

    #[test]
    fn decisions_survive_reopening_a_folder() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("photos");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("a.jpg"), b"x").unwrap();
        let store = store(root.path());
        let opened = store.open(folder.to_str().unwrap()).unwrap();
        let photo = &opened.photos[0];
        store
            .set_decisions(
                &opened.folder,
                vec![photo.name.clone()],
                vec![photo.fingerprint],
                Decision::Discard,
            )
            .unwrap();
        assert_eq!(
            store.open(&opened.folder).unwrap().photos[0].decision,
            Decision::Discard
        );
        assert_eq!(store.last_folder().as_deref(), Some(opened.folder.as_str()));
    }

    #[test]
    fn the_library_is_not_reopened_without_access() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        std::fs::create_dir_all(root.path().join("data")).unwrap();
        std::fs::write(
            root.path().join("data/last-folder.json"),
            br#""bilder:library""#,
        )
        .unwrap();
        assert_eq!(store.last_folder(), None);
    }

    #[test]
    fn opening_a_library_package_as_a_folder_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("Photos.photoslibrary");
        std::fs::create_dir(&package).unwrap();
        assert!(store(root.path()).open(package.to_str().unwrap()).is_err());
    }
}
