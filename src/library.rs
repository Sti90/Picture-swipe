//! The Photos library as a source. Everything here is written against `Library`, so
//! tests run with a fake and never reach PhotoKit.
use crate::{
    decisions::{self, Decision, Decisions, Entry, Fingerprint},
    scan::{self, FolderState, Photo},
    trash::{Failure, TrashResult},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime},
};

pub const LIBRARY_PATH: &str = "bilder:library";
pub const ASSET_PREFIX: &str = "bilder:asset:";
const PREVIEW_EDGE: u32 = 2400;
const THUMBNAIL_EDGE: u32 = 512;
const MAX_PREVIEWS: usize = 64;
const MAX_THUMBNAILS: usize = 3000;
/// Longer than any render can take, so a temp file this old was left by a crash.
const STALE_TEMP: Duration = Duration::from_secs(3600);
/// A localIdentifier is never reused for another photo, so it identifies a photo on its own.
const NO_FINGERPRINT: Fingerprint = Fingerprint {
    len: 0,
    modified_ms: 0,
    inode: 0,
};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    NotDetermined,
    Restricted,
    Denied,
    Limited,
    Authorized,
}
impl Status {
    fn allowed(self) -> bool {
        matches!(self, Status::Limited | Status::Authorized)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    pub id: String,
    pub created_ms: u64,
    /// Only for telling when a cached image is stale. Photos changes it on its own too,
    /// so it must never be used as the photo's identity.
    pub modified_ms: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeleteOutcome {
    Deleted { missing: Vec<String> },
    Cancelled,
    Failed(String),
}
pub trait Library: Send + Sync {
    /// Must never show a prompt.
    fn status(&self) -> Status;
    fn request_access(&self) -> Status;
    fn list(&self) -> Result<Vec<Asset>, String>;
    fn delete(&self, ids: &[String]) -> DeleteOutcome;
    fn render(&self, id: &str, edge: u32, network: bool, out: &Path) -> Result<(), String>;
}

pub fn is_library(path: &str) -> bool {
    path == LIBRARY_PATH
}
pub fn is_asset(path: &str) -> bool {
    path.starts_with(ASSET_PREFIX)
}

fn hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| b.is_ascii_hexdigit())
}
fn digits(s: &str) -> bool {
    (1..=4).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit())
}
/// Exactly the `UUID/L0/001` shape of a PHAsset localIdentifier.
fn valid_id(id: &str) -> bool {
    let parts: Vec<&str> = id.split('/').collect();
    let [uuid, level, index] = parts.as_slice() else {
        return false;
    };
    let groups: Vec<&str> = uuid.split('-').collect();
    let [a, b, c, d, e] = groups.as_slice() else {
        return false;
    };
    hex(a, 8)
        && hex(b, 4)
        && hex(c, 4)
        && hex(d, 4)
        && hex(e, 12)
        && level.strip_prefix('L').is_some_and(digits)
        && digits(index)
}
pub fn encode(id: &str) -> Result<String, String> {
    if !valid_id(id) {
        return Err("Ugyldig bilde-ID.".into());
    }
    Ok(id.replace('/', ":"))
}
pub fn decode(name: &str) -> Result<String, String> {
    if name.contains('/') {
        return Err("Ugyldig bilde-ID.".into());
    }
    let id = name.replace(':', "/");
    if !valid_id(&id) {
        return Err("Ugyldig bilde-ID.".into());
    }
    Ok(id)
}

fn cache_name(id: &str, edge: u32, modified_ms: u64) -> String {
    format!(
        "{}.jpg",
        decisions::hash(format!("{id}:{edge}:{modified_ms}:v2").as_bytes())
    )
}

/// Removes temp files left behind by a crash. Touches only regular `.tmp*.jpg` files
/// directly inside `dir`.
fn remove_stale_temp(dir: &Path, older_than: Duration) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with(".tmp") || !name.ends_with(".jpg") {
            continue;
        }
        let path = entry.path();
        let stale = path.symlink_metadata().is_ok_and(|meta| {
            meta.is_file()
                && meta
                    .modified()
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .is_some_and(|age| age > older_than)
        });
        if stale && let Err(e) = fs::remove_file(&path) {
            eprintln!("Kan ikke rydde midlertidig fil: {e}");
        }
    }
}

/// Removes the oldest previews beyond `keep`. Touches only finished preview files directly
/// inside `dir`: no temp files, subfolders or symlinks.
fn prune_previews(dir: &Path, keep: usize) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(SystemTime, PathBuf)> = entries
        .flatten()
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            name.ends_with(".jpg") && !name.starts_with('.')
        })
        .filter_map(|e| {
            let meta = e.path().symlink_metadata().ok()?;
            meta.is_file().then_some(())?;
            Some((meta.modified().ok()?, e.path()))
        })
        .collect();
    if files.len() <= keep {
        return;
    }
    files.sort();
    let excess = files.len() - keep;
    for (_, path) in files.into_iter().take(excess) {
        if let Err(e) = fs::remove_file(&path) {
            eprintln!("Kan ikke rydde forhåndsvisning: {e}");
        }
    }
}

pub struct LibrarySource {
    library: Arc<dyn Library>,
    data: PathBuf,
    thumbnails: PathBuf,
    previews: PathBuf,
    versions: Mutex<HashMap<String, u64>>,
    opened: AtomicBool,
    writes: Mutex<()>,
}

impl LibrarySource {
    pub fn new(library: Arc<dyn Library>, data: PathBuf, cache: PathBuf) -> Self {
        Self {
            library,
            data,
            previews: cache.join("library-previews"),
            thumbnails: cache.join("library-thumbnails"),
            versions: Mutex::default(),
            opened: AtomicBool::new(false),
            writes: Mutex::new(()),
        }
    }
    #[cfg(test)]
    pub fn previews_dir(&self) -> &Path {
        &self.previews
    }
    fn version(&self, id: &str) -> u64 {
        self.versions
            .lock()
            .ok()
            .and_then(|v| v.get(id).copied())
            .unwrap_or(0)
    }
    fn saved(&self) -> PathBuf {
        decisions::location(&self.data, Path::new(LIBRARY_PATH))
    }
    fn ensure_opened(&self) -> Result<(), String> {
        if self.opened.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err("Åpne Bilder-biblioteket først.".into())
        }
    }
    pub fn status(&self) -> Status {
        self.library.status()
    }
    /// Reopening at launch must never prompt, so this only reads the current status.
    pub fn may_reopen(&self) -> bool {
        self.library.status().allowed()
    }
    pub fn open(&self) -> Result<FolderState, String> {
        let status = match self.library.status() {
            Status::NotDetermined => self.library.request_access(),
            status => status,
        };
        if !status.allowed() {
            return Err(
                "Bildesveip har ikke tilgang til Bilder. Slå det på i Systeminnstillinger, under Personvern og sikkerhet > Bilder."
                    .into(),
            );
        }
        let assets = self.library.list()?;
        if let Ok(mut versions) = self.versions.lock() {
            *versions = assets
                .iter()
                .map(|a| (a.id.clone(), a.modified_ms))
                .collect();
        }
        remove_stale_temp(&self.thumbnails, STALE_TEMP);
        remove_stale_temp(&self.previews, STALE_TEMP);
        prune_previews(&self.thumbnails, MAX_THUMBNAILS);
        let mut photos = Vec::new();
        for asset in assets {
            let Ok(name) = encode(&asset.id) else {
                eprintln!("Hopper over bilde med ukjent ID-format.");
                continue;
            };
            photos.push(Photo {
                path: format!("{ASSET_PREFIX}{name}"),
                name,
                size: 0,
                modified: asset.created_ms,
                decision: Decision::Undecided,
                web_safe: false,
                fingerprint: NO_FINGERPRINT,
            });
        }
        let _guard = self
            .writes
            .lock()
            .map_err(|_| "Lagringen er utilgjengelig.")?;
        let location = self.saved();
        let (mut saved, state_reset) = decisions::load_checked(&location)?;
        // An empty listing proves nothing, and must not consume every saved decision.
        if !photos.is_empty() && scan::apply_saved(&mut photos, &mut saved) {
            decisions::save(&location, &saved)?;
        }
        decisions::atomic_write(
            &self.data.join("last-folder.json"),
            &serde_json::to_vec(LIBRARY_PATH).map_err(|e| format!("Kan ikke lagre mappe: {e}"))?,
        )?;
        self.opened.store(true, Ordering::SeqCst);
        Ok(FolderState {
            folder: LIBRARY_PATH.into(),
            folder_name: "Bilder".into(),
            photos,
            state_reset,
            source: "library",
        })
    }
    pub fn set_decisions(
        &self,
        names: Vec<String>,
        versions: Vec<Fingerprint>,
        decision: Decision,
    ) -> Result<(), String> {
        self.ensure_opened()?;
        if names.len() != versions.len() || versions.iter().any(|v| *v != NO_FINGERPRINT) {
            return Err("Ugyldig forespørsel.".into());
        }
        for name in &names {
            decode(name)?;
        }
        let _guard = self
            .writes
            .lock()
            .map_err(|_| "Lagringen er utilgjengelig.")?;
        let location = self.saved();
        let mut values = decisions::load(&location)?;
        for name in names {
            if decision == Decision::Undecided {
                values.remove(&name);
            } else {
                let fingerprint = Some(NO_FINGERPRINT);
                values.insert(
                    name,
                    Entry {
                        decision,
                        fingerprint,
                    },
                );
            }
        }
        decisions::save(&location, &values)
    }
    pub fn trash(&self, names: &[String]) -> Result<TrashResult, String> {
        self.ensure_opened()?;
        for name in names {
            decode(name)?;
        }
        let _guard = self
            .writes
            .lock()
            .map_err(|_| "Lagringen er utilgjengelig.")?;
        let location = self.saved();
        let mut saved = decisions::load(&location)?;
        let mut result = TrashResult {
            trashed: vec![],
            failed: vec![],
            bytes_freed: 0,
            state_error: None,
        };
        let (allowed, refused) = marked(names, &saved);
        for name in refused {
            result.failed.push(Failure {
                name,
                error: "Bildet er ikke merket for sletting.".into(),
            });
        }
        if allowed.is_empty() {
            return Ok(result);
        }
        let ids = allowed
            .iter()
            .map(|n| decode(n))
            .collect::<Result<Vec<_>, _>>()?;
        match self.library.delete(&ids) {
            DeleteOutcome::Cancelled => result.failed.clear(),
            DeleteOutcome::Failed(error) => {
                result
                    .failed
                    .extend(allowed.into_iter().map(|name| Failure {
                        name,
                        error: error.clone(),
                    }));
            }
            DeleteOutcome::Deleted { missing } => {
                let missing: HashSet<String> = missing.into_iter().collect();
                for (name, id) in allowed.into_iter().zip(&ids) {
                    if missing.contains(id) {
                        result.failed.push(Failure {
                            name,
                            error: "Bildet finnes ikke lenger i Bilder.".into(),
                        });
                    } else {
                        saved.remove(&name);
                        self.forget(id);
                        result.trashed.push(name);
                    }
                }
                if !result.trashed.is_empty() {
                    result.state_error = decisions::save(&location, &saved).err();
                }
            }
        }
        Ok(result)
    }
    /// Private photos shouldn't linger in the cache after being deleted.
    fn forget(&self, id: &str) {
        let version = self.version(id);
        let _ = fs::remove_file(
            self.thumbnails
                .join(cache_name(id, THUMBNAIL_EDGE, version)),
        );
        let _ = fs::remove_file(self.previews.join(cache_name(id, PREVIEW_EDGE, version)));
    }
    /// `path` is `bilder:asset:<name>`. Returns the absolute path of a cached JPEG.
    pub fn image(&self, path: &str, preview: bool) -> Result<String, String> {
        self.ensure_opened()?;
        let name = path.strip_prefix(ASSET_PREFIX).ok_or("Ugyldig bilde-ID.")?;
        let id = decode(name)?;
        let (dir, edge) = if preview {
            (&self.previews, PREVIEW_EDGE)
        } else {
            (&self.thumbnails, THUMBNAIL_EDGE)
        };
        let version = self.version(&id);
        let out = render_cached(self.library.as_ref(), dir, &id, edge, version, preview)?;
        if preview {
            prune_previews(dir, MAX_PREVIEWS);
        }
        Ok(out.to_string_lossy().into_owned())
    }
}

/// Splits names into those saved as `discard` and the rest, ignoring repeats.
fn marked(names: &[String], saved: &Decisions) -> (Vec<String>, Vec<String>) {
    let mut seen = HashSet::new();
    let mut allowed = Vec::new();
    let mut refused = Vec::new();
    for name in names {
        if !seen.insert(name) {
            continue;
        }
        let entry = saved.get(name);
        if entry.is_some_and(|e| {
            e.decision == Decision::Discard && e.fingerprint == Some(NO_FINGERPRINT)
        }) {
            allowed.push(name.clone());
        } else {
            refused.push(name.clone());
        }
    }
    (allowed, refused)
}

fn render_cached(
    library: &dyn Library,
    dir: &Path,
    id: &str,
    edge: u32,
    version: u64,
    network: bool,
) -> Result<PathBuf, String> {
    let failed = |e: std::io::Error| format!("Kan ikke lagre bildet: {e}");
    let out = dir.join(cache_name(id, edge, version));
    if out.is_file() {
        // Keeps recently viewed previews from being the first ones pruned.
        if let Ok(file) = fs::File::options().write(true).open(&out) {
            let _ = file.set_modified(SystemTime::now());
        }
        return Ok(out);
    }
    fs::create_dir_all(dir).map_err(failed)?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).map_err(failed)?;
    let tmp = tempfile::Builder::new()
        .prefix(".tmp")
        .suffix(".jpg")
        .tempfile_in(dir)
        .map_err(failed)?;
    library.render(id, edge, network, tmp.path())?;
    let meta = fs::metadata(tmp.path()).map_err(failed)?;
    if meta.len() == 0 {
        return Err("Bildet kunne ikke vises.".into());
    }
    fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o600)).map_err(failed)?;
    tmp.as_file().sync_all().map_err(failed)?;
    tmp.persist(&out)
        .map_err(|e| format!("Kan ikke lagre bildet: {e}"))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    const A: &str = "ED7AC36B-A150-4C38-BB8C-B6D696F4F2ED/L0/001";
    const B: &str = "11111111-2222-3333-4444-555555555555/L0/001";
    type Hook = Box<dyn FnOnce() + Send>;

    struct Fake {
        status: Mutex<Status>,
        after_request: Status,
        requests: AtomicUsize,
        assets: Mutex<Vec<Asset>>,
        outcome: Mutex<DeleteOutcome>,
        deleted: Mutex<Vec<Vec<String>>>,
        renders: AtomicUsize,
        on_delete: Mutex<Option<Hook>>,
    }
    impl Fake {
        fn new(status: Status) -> Arc<Self> {
            Arc::new(Self {
                status: Mutex::new(status),
                after_request: Status::Authorized,
                requests: AtomicUsize::new(0),
                assets: Mutex::new(vec![
                    Asset {
                        id: A.into(),
                        created_ms: 1,
                        modified_ms: 1,
                    },
                    Asset {
                        id: B.into(),
                        created_ms: 2,
                        modified_ms: 2,
                    },
                ]),
                outcome: Mutex::new(DeleteOutcome::Deleted { missing: vec![] }),
                deleted: Mutex::new(vec![]),
                renders: AtomicUsize::new(0),
                on_delete: Mutex::new(None),
            })
        }
    }
    impl Library for Fake {
        fn status(&self) -> Status {
            *self.status.lock().unwrap()
        }
        fn request_access(&self) -> Status {
            self.requests.fetch_add(1, Ordering::SeqCst);
            *self.status.lock().unwrap() = self.after_request;
            self.after_request
        }
        fn list(&self) -> Result<Vec<Asset>, String> {
            Ok(self.assets.lock().unwrap().clone())
        }
        fn delete(&self, ids: &[String]) -> DeleteOutcome {
            self.deleted.lock().unwrap().push(ids.to_vec());
            if let Some(hook) = self.on_delete.lock().unwrap().take() {
                hook();
            }
            self.outcome.lock().unwrap().clone()
        }
        fn render(&self, _: &str, _: u32, _: bool, out: &Path) -> Result<(), String> {
            self.renders.fetch_add(1, Ordering::SeqCst);
            fs::write(out, b"jpeg").map_err(|e| e.to_string())
        }
    }

    fn name(id: &str) -> String {
        encode(id).unwrap()
    }
    fn setup(status: Status) -> (tempfile::TempDir, Arc<Fake>, LibrarySource) {
        let dir = tempfile::tempdir().unwrap();
        let fake = Fake::new(status);
        let source = LibrarySource::new(
            fake.clone(),
            dir.path().join("data"),
            dir.path().join("cache"),
        );
        fs::create_dir_all(dir.path().join("data")).unwrap();
        (dir, fake, source)
    }
    fn opened() -> (tempfile::TempDir, Arc<Fake>, LibrarySource) {
        let all = setup(Status::Authorized);
        all.2.open().unwrap();
        all
    }
    fn mark(source: &LibrarySource, ids: &[&str], decision: Decision) {
        let names: Vec<String> = ids.iter().map(|id| name(id)).collect();
        let versions = vec![NO_FINGERPRINT; names.len()];
        source.set_decisions(names, versions, decision).unwrap();
    }
    fn saved(source: &LibrarySource) -> Decisions {
        decisions::load(&source.saved()).unwrap()
    }

    #[test]
    fn ids_are_strictly_validated() {
        assert_eq!(decode(&name(A)).unwrap(), A);
        for bad in [
            "",
            "../../etc/passwd",
            "..:..:etc",
            "ED7AC36B-A150-4C38-BB8C-B6D696F4F2ED",
            "ED7AC36B-A150-4C38-BB8C-B6D696F4F2ED:L0",
            "ED7AC36B-A150-4C38-BB8C-B6D696F4F2ED:L0:001:x",
            "ED7AC36B-A150-4C38-BB8C-B6D696F4F2EZ:L0:001",
            "ED7AC36B-A150-4C38-BB8C-B6D696F4F2ED:X0:001",
            "ED7AC36B-A150-4C38-BB8C-B6D696F4F2ED:L0:..",
            "ED7AC36B-A150-4C38-BB8C-B6D696F4F2ED/L0/001",
            "ED7AC36B-A150-4C38-BB8C-B6D696F4F2ED:L0:001\0",
        ] {
            assert!(decode(bad).is_err(), "{bad:?}");
        }
        assert!(encode("../evil").is_err());
        assert!(validate_saved_name(&name(A)));
    }
    fn validate_saved_name(n: &str) -> bool {
        scan::validate_name(n).is_ok()
    }

    #[test]
    fn opening_prompts_only_when_undetermined() {
        let (_d, fake, source) = setup(Status::NotDetermined);
        let state = source.open().unwrap();
        assert_eq!(fake.requests.load(Ordering::SeqCst), 1);
        assert_eq!(state.folder, LIBRARY_PATH);
        assert_eq!(state.source, "library");
        assert_eq!(state.photos[0].path, format!("{ASSET_PREFIX}{}", name(A)));
        assert_eq!(state.photos[0].fingerprint, NO_FINGERPRINT);

        for status in [Status::Denied, Status::Restricted] {
            let (_d, fake, source) = setup(status);
            assert!(source.open().is_err());
            assert_eq!(fake.requests.load(Ordering::SeqCst), 0);
            assert!(
                source
                    .image(&format!("{ASSET_PREFIX}{}", name(A)), false)
                    .is_err()
            );
        }
    }

    #[test]
    fn reopening_at_launch_never_prompts() {
        for (status, allowed) in [
            (Status::NotDetermined, false),
            (Status::Denied, false),
            (Status::Restricted, false),
            (Status::Limited, true),
            (Status::Authorized, true),
        ] {
            let (_d, fake, source) = setup(status);
            assert_eq!(source.may_reopen(), allowed);
            assert_eq!(fake.requests.load(Ordering::SeqCst), 0);
        }
    }

    #[test]
    fn refused_until_the_library_is_opened() {
        let (_d, fake, source) = setup(Status::Authorized);
        let names = vec![name(A)];
        assert!(
            source
                .set_decisions(names.clone(), vec![NO_FINGERPRINT], Decision::Discard)
                .is_err()
        );
        assert!(source.trash(&names).is_err());
        assert!(
            source
                .image(&format!("{ASSET_PREFIX}{}", name(A)), true)
                .is_err()
        );
        assert!(fake.deleted.lock().unwrap().is_empty());
        assert_eq!(fake.renders.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn decisions_reject_bad_input() {
        let (_d, _fake, source) = opened();
        let version = Fingerprint {
            len: 1,
            modified_ms: 0,
            inode: 0,
        };
        assert!(
            source
                .set_decisions(vec![name(A)], vec![version], Decision::Discard)
                .is_err()
        );
        assert!(
            source
                .set_decisions(vec![name(A)], vec![], Decision::Discard)
                .is_err()
        );
        assert!(
            source
                .set_decisions(vec!["../x".into()], vec![NO_FINGERPRINT], Decision::Discard)
                .is_err()
        );
        assert!(saved(&source).is_empty());
    }

    #[test]
    fn refuses_names_not_marked_discard() {
        let (_d, fake, source) = opened();
        mark(&source, &[A], Decision::Keep);
        let result = source.trash(&[name(A), name(B)]).unwrap();
        assert!(result.trashed.is_empty());
        assert_eq!(result.failed.len(), 2);
        assert!(fake.deleted.lock().unwrap().is_empty());
        assert!(source.trash(&["../x".into()]).is_err());
    }

    #[test]
    fn deletes_only_marked_photos_and_forgets_them() {
        let (_d, fake, source) = opened();
        mark(&source, &[A], Decision::Discard);
        mark(&source, &[B], Decision::Keep);
        let result = source.trash(&[name(A), name(A), name(B)]).unwrap();
        assert_eq!(result.trashed, vec![name(A)]);
        assert_eq!(result.failed.len(), 1);
        assert_eq!(result.bytes_freed, 0);
        assert_eq!(*fake.deleted.lock().unwrap(), vec![vec![A.to_string()]]);
        let left = saved(&source);
        assert!(!left.contains_key(&name(A)) && left.contains_key(&name(B)));
    }

    #[test]
    fn cancelling_the_system_dialog_changes_nothing() {
        let (_d, fake, source) = opened();
        mark(&source, &[A], Decision::Discard);
        *fake.outcome.lock().unwrap() = DeleteOutcome::Cancelled;
        let result = source.trash(&[name(A), name(B)]).unwrap();
        assert!(result.trashed.is_empty() && result.failed.is_empty());
        assert!(result.state_error.is_none());
        assert!(saved(&source).contains_key(&name(A)));
    }

    #[test]
    fn a_photokit_error_fails_every_name_and_keeps_decisions() {
        let (_d, fake, source) = opened();
        mark(&source, &[A, B], Decision::Discard);
        *fake.outcome.lock().unwrap() = DeleteOutcome::Failed("Nei.".into());
        let result = source.trash(&[name(A), name(B)]).unwrap();
        assert!(result.trashed.is_empty());
        assert_eq!(result.failed.len(), 2);
        assert!(result.failed.iter().all(|f| f.error == "Nei."));
        assert_eq!(saved(&source).len(), 2);
    }

    #[test]
    fn missing_assets_fail_while_the_rest_proceed() {
        let (_d, fake, source) = opened();
        mark(&source, &[A, B], Decision::Discard);
        *fake.outcome.lock().unwrap() = DeleteOutcome::Deleted {
            missing: vec![B.into()],
        };
        let result = source.trash(&[name(A), name(B)]).unwrap();
        assert_eq!(result.trashed, vec![name(A)]);
        assert_eq!(result.failed[0].name, name(B));
        assert!(!saved(&source).contains_key(&name(A)));
    }

    #[test]
    fn a_save_failure_is_reported_after_deleting() {
        let (_d, fake, source) = opened();
        mark(&source, &[A], Decision::Discard);
        let location = source.saved();
        *fake.on_delete.lock().unwrap() = Some(Box::new(move || {
            fs::remove_file(&location).unwrap();
            fs::create_dir(&location).unwrap();
        }));
        let result = source.trash(&[name(A)]).unwrap();
        assert_eq!(result.trashed, vec![name(A)]);
        assert!(result.state_error.is_some());
    }

    #[test]
    fn decisions_survive_reopening_and_an_empty_listing() {
        let (_d, fake, source) = opened();
        mark(&source, &[A], Decision::Discard);
        assert_eq!(source.open().unwrap().photos[0].decision, Decision::Discard);
        let everything = std::mem::take(&mut *fake.assets.lock().unwrap());
        source.open().unwrap();
        assert!(saved(&source).contains_key(&name(A)));
        *fake.assets.lock().unwrap() = everything[1..].to_vec();
        source.open().unwrap();
        assert!(!saved(&source).contains_key(&name(A)));
    }

    #[test]
    fn rendered_images_are_cached_privately_under_hashed_names() {
        let (_d, fake, source) = opened();
        let path = format!("{ASSET_PREFIX}{}", name(A));
        let first = source.image(&path, true).unwrap();
        assert_eq!(source.image(&path, true).unwrap(), first);
        assert_eq!(fake.renders.load(Ordering::SeqCst), 1);
        assert!(!first.contains("ED7AC36B"));
        assert!(first.starts_with(source.previews_dir().to_str().unwrap()));
        let mode = fs::metadata(&first).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert!(source.image("bilder:asset:../../x", true).is_err());
        assert!(source.image("/etc/hosts", true).is_err());
    }

    #[test]
    fn an_edited_photo_is_rendered_again() {
        let (_d, fake, source) = opened();
        let path = format!("{ASSET_PREFIX}{}", name(A));
        source.image(&path, true).unwrap();
        source.image(&path, true).unwrap();
        assert_eq!(fake.renders.load(Ordering::SeqCst), 1);
        fake.assets.lock().unwrap()[0].modified_ms = 99;
        source.open().unwrap();
        source.image(&path, true).unwrap();
        assert_eq!(fake.renders.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn stale_cleanup_removes_only_old_temp_files() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("cache");
        fs::create_dir(&dir).unwrap();
        let old = SystemTime::now() - Duration::from_secs(7200);
        for file in [".tmpold.jpg", "photo.jpg", ".tmpold.txt"] {
            fs::write(dir.join(file), b"x").unwrap();
            let handle = fs::File::options()
                .write(true)
                .open(dir.join(file))
                .unwrap();
            handle.set_modified(old).unwrap();
        }
        fs::write(dir.join(".tmpbusy.jpg"), b"x").unwrap();
        remove_stale_temp(&dir, STALE_TEMP);
        assert!(!dir.join(".tmpold.jpg").exists());
        for file in ["photo.jpg", ".tmpold.txt", ".tmpbusy.jpg"] {
            assert!(dir.join(file).exists(), "{file}");
        }
    }

    #[test]
    fn pruning_removes_only_oldest_previews_in_its_folder() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("previews");
        fs::create_dir_all(dir.join("nested")).unwrap();
        fs::write(root.path().join("outside.jpg"), b"x").unwrap();
        fs::write(dir.join("nested/inner.jpg"), b"x").unwrap();
        fs::write(dir.join("notes.txt"), b"x").unwrap();
        fs::write(dir.join(".tmpbusy.jpg"), b"x").unwrap();
        std::os::unix::fs::symlink(root.path().join("outside.jpg"), dir.join("link.jpg")).unwrap();
        let start = SystemTime::now() - std::time::Duration::from_secs(100);
        for i in 0..5u64 {
            let path = dir.join(format!("{i}.jpg"));
            fs::write(&path, b"x").unwrap();
            fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(start + std::time::Duration::from_secs(i))
                .unwrap();
        }
        prune_previews(&dir, 3);
        for (name, kept) in [
            ("0.jpg", false),
            ("1.jpg", false),
            ("2.jpg", true),
            ("4.jpg", true),
        ] {
            assert_eq!(dir.join(name).exists(), kept, "{name}");
        }
        assert!(root.path().join("outside.jpg").exists());
        assert!(dir.join("nested/inner.jpg").exists());
        assert!(dir.join("notes.txt").exists());
        assert!(dir.join(".tmpbusy.jpg").exists());
        assert!(dir.join("link.jpg").symlink_metadata().is_ok());
    }
}
