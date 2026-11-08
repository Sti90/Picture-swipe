use crate::decisions::{Decision, Decisions, Fingerprint};
use serde::Serialize;
use std::{
    collections::HashSet,
    fs,
    path::{Component, Path, PathBuf},
    time::UNIX_EPOCH,
};

pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.contains(['/', '\\', '\0'])
        || name == "."
        || name == ".."
        || Path::new(name)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err("Ugyldig filnavn.".into());
    }
    Ok(())
}
pub fn web_safe(path: &Path) -> Option<bool> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "heif" | "avif" | "bmp" | "tif"
        | "tiff" => Some(true),
        "cr2" | "cr3" | "nef" | "arw" | "dng" | "raf" | "orf" | "rw2" => Some(false),
        _ => None,
    }
}
pub fn folder(path: &str) -> Result<PathBuf, String> {
    let p = fs::canonicalize(path).map_err(|e| format!("Kan ikke åpne mappen: {e}"))?;
    if !p.is_dir() {
        return Err("Stien er ikke en mappe.".into());
    }
    // Moving files inside a Photos library behind its back would corrupt it.
    let in_library = p.components().any(|c| {
        c.as_os_str()
            .to_string_lossy()
            .to_lowercase()
            .ends_with(".photoslibrary")
    });
    if in_library {
        return Err(
            "Bilder-biblioteket kan ikke åpnes som mappe. Bruk Åpne Bilder-biblioteket.".into(),
        );
    }
    Ok(p)
}
pub fn file(folder: &Path, name: &str) -> Result<PathBuf, String> {
    validate_name(name)?;
    let p = folder.join(name);
    let meta = fs::symlink_metadata(&p).map_err(|e| format!("Kan ikke lese filen: {e}"))?;
    if !meta.is_file() || meta.file_type().is_symlink() || web_safe(&p).is_none() {
        return Err("Filen er ikke en vanlig bildefil.".into());
    }
    Ok(p)
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Photo {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub modified: u64,
    pub decision: Decision,
    pub web_safe: bool,
    pub fingerprint: Fingerprint,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderState {
    pub folder: String,
    pub folder_name: String,
    pub photos: Vec<Photo>,
    pub state_reset: bool,
    pub source: &'static str,
}
pub fn fingerprint(path: &Path) -> Result<Fingerprint, String> {
    use std::os::unix::fs::MetadataExt;
    let meta = fs::symlink_metadata(path).map_err(|e| format!("Kan ikke lese filen: {e}"))?;
    Ok(Fingerprint {
        len: meta.len(),
        modified_ms: modified_ms(&meta),
        inode: meta.ino(),
    })
}
fn modified_ms(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
/// Copies saved decisions onto the photos. A decision only survives if it was made
/// about the file now under that name: entries for missing files are consumed, so a
/// photo later restored from the Trash starts over. A discard saved before fingerprints
/// existed can't be tied to a file, so it must be made again. Returns true if `saved` changed.
pub fn apply_saved(photos: &mut [Photo], saved: &mut Decisions) -> bool {
    let present: HashSet<&str> = photos.iter().map(|p| p.name.as_str()).collect();
    let before = saved.len();
    saved.retain(|name, _| present.contains(name.as_str()));
    let mut changed = saved.len() != before;
    for photo in photos {
        let Some(entry) = saved.get_mut(&photo.name) else {
            continue;
        };
        match entry.fingerprint {
            Some(known) if known == photo.fingerprint => photo.decision = entry.decision,
            None if entry.decision == Decision::Keep => {
                entry.fingerprint = Some(photo.fingerprint);
                photo.decision = Decision::Keep;
                changed = true;
            }
            _ => {
                saved.remove(&photo.name);
                changed = true;
            }
        }
    }
    changed
}
pub fn scan(folder: &Path) -> Result<FolderState, String> {
    let mut photos = Vec::new();
    for entry in fs::read_dir(folder).map_err(|e| format!("Kan ikke lese mappen: {e}"))? {
        let entry = entry.map_err(|e| format!("Kan ikke lese fil: {e}"))?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        let Ok(path) = file(folder, &name) else {
            continue;
        };
        let meta = fs::metadata(&path).map_err(|e| format!("Kan ikke lese fil: {e}"))?;
        photos.push(Photo {
            name,
            path: path.to_string_lossy().into_owned(),
            size: meta.len(),
            modified: modified_ms(&meta),
            decision: Decision::Undecided,
            web_safe: web_safe(&path).unwrap_or(false),
            fingerprint: fingerprint(&path)?,
        });
    }
    photos.sort_by(|a, b| a.modified.cmp(&b.modified).then(a.name.cmp(&b.name)));
    Ok(FolderState {
        folder: folder.to_string_lossy().into_owned(),
        folder_name: folder
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        photos,
        state_reset: false,
        source: "folder",
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names() {
        for n in ["", ".", "..", "../a", "a/b", "/a", "a\\b", "a\0"] {
            assert!(validate_name(n).is_err(), "{n:?}");
        }
        assert!(validate_name("bilde æ.jpg").is_ok());
    }
    #[test]
    fn refuses_folders_inside_a_photos_library() {
        let d = tempfile::tempdir().unwrap();
        let inside = d.path().join("Bilder.PhotosLibrary/originals/0");
        fs::create_dir_all(&inside).unwrap();
        assert!(folder(inside.to_str().unwrap()).is_err());
        assert!(folder(inside.parent().unwrap().parent().unwrap().to_str().unwrap()).is_err());
        assert!(folder(d.path().to_str().unwrap()).is_ok());
    }
    #[test]
    fn scanning() {
        let d = tempfile::tempdir().unwrap();
        for n in ["b.jpg", "a.jpg", ".hidden.jpg", "notes.txt", "raw.NEF"] {
            fs::write(d.path().join(n), b"x").unwrap();
        }
        let time = fs::File::open(d.path().join("b.jpg"))
            .unwrap()
            .metadata()
            .unwrap()
            .modified()
            .unwrap();
        for n in ["a.jpg", "raw.NEF"] {
            fs::File::options()
                .write(true)
                .open(d.path().join(n))
                .unwrap()
                .set_modified(time)
                .unwrap();
        }
        std::os::unix::fs::symlink("a.jpg", d.path().join("link.jpg")).unwrap();
        let s = scan(d.path()).unwrap();
        assert_eq!(
            s.photos.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            vec!["a.jpg", "b.jpg", "raw.NEF"]
        );
        assert!(!s.photos[2].web_safe);
    }
    #[test]
    fn decision_does_not_follow_a_replaced_file() {
        use crate::decisions::Entry;
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("same.jpg"), b"one").unwrap();
        fs::write(d.path().join("swapped.jpg"), b"one").unwrap();
        let mut saved = Decisions::new();
        for name in ["same.jpg", "swapped.jpg"] {
            let entry = Entry {
                decision: Decision::Discard,
                fingerprint: Some(fingerprint(&d.path().join(name)).unwrap()),
            };
            saved.insert(name.into(), entry);
        }
        fs::remove_file(d.path().join("swapped.jpg")).unwrap();
        fs::write(d.path().join("swapped.jpg"), b"a different image").unwrap();
        let mut photos = scan(d.path()).unwrap().photos;
        assert!(apply_saved(&mut photos, &mut saved));
        let decision = |n: &str| photos.iter().find(|p| p.name == n).unwrap().decision;
        assert_eq!(decision("same.jpg"), Decision::Discard);
        assert_eq!(decision("swapped.jpg"), Decision::Undecided);
        assert!(!saved.contains_key("swapped.jpg"));
    }
    #[test]
    fn old_and_orphaned_entries_do_not_keep_permission() {
        use crate::decisions::Entry;
        let d = tempfile::tempdir().unwrap();
        for n in ["discard.jpg", "keep.jpg"] {
            fs::write(d.path().join(n), b"x").unwrap();
        }
        let old = |decision| Entry {
            decision,
            fingerprint: None,
        };
        let mut saved = Decisions::from([
            ("discard.jpg".to_string(), old(Decision::Discard)),
            ("keep.jpg".to_string(), old(Decision::Keep)),
            ("gone.jpg".to_string(), old(Decision::Discard)),
        ]);
        let mut photos = scan(d.path()).unwrap().photos;
        assert!(apply_saved(&mut photos, &mut saved));
        assert_eq!(saved.keys().collect::<Vec<_>>(), vec!["keep.jpg"]);
        assert!(saved["keep.jpg"].fingerprint.is_some());
        let decision = |n: &str| photos.iter().find(|p| p.name == n).unwrap().decision;
        assert_eq!(decision("discard.jpg"), Decision::Undecided);
        assert_eq!(decision("keep.jpg"), Decision::Keep);
    }
}
