use crate::{
    decisions::{self, Decision, Fingerprint},
    scan,
};
use ::trash::macos::TrashContextExtMacos;
use serde::Serialize;
use std::{collections::HashSet, path::Path};
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashResult {
    pub trashed: Vec<String>,
    pub failed: Vec<Failure>,
    pub bytes_freed: u64,
    pub state_error: Option<String>,
}
#[derive(Debug, Serialize)]
pub struct Failure {
    pub name: String,
    pub error: String,
}
pub fn perform(
    folder: &Path,
    names: &[String],
    versions: &[Fingerprint],
    saved: &Path,
    mut delete: impl FnMut(&Path) -> Result<(), String>,
) -> Result<TrashResult, String> {
    if names.len() != versions.len() {
        return Err("Ugyldig forespørsel.".into());
    }
    for n in names {
        scan::validate_name(n)?;
    }
    let mut decisions = decisions::load(saved)?;
    let mut result = TrashResult {
        trashed: vec![],
        failed: vec![],
        bytes_freed: 0,
        state_error: None,
    };
    let mut seen = HashSet::new();
    for (name, shown) in names.iter().zip(versions) {
        if !seen.insert(name) {
            continue;
        }
        let outcome = (|| -> Result<u64, String> {
            let entry = decisions
                .get(name)
                .filter(|e| e.decision == Decision::Discard)
                .ok_or("Bildet er ikke merket for sletting.")?;
            let path = scan::file(folder, name)?;
            // The file must be the one marked and the one the user confirmed, even if the
            // folder path now leads somewhere else.
            if entry.fingerprint != Some(*shown) || scan::fingerprint(&path)? != *shown {
                return Err("Filen er endret siden den ble merket. Åpne mappen på nytt.".into());
            }
            let size = path
                .metadata()
                .map_err(|e| format!("Kan ikke lese filen: {e}"))?
                .len();
            delete(&path)?;
            Ok(size)
        })();
        match outcome {
            Ok(size) => {
                result.bytes_freed += size;
                result.trashed.push(name.clone());
                decisions.remove(name);
            }
            Err(error) => result.failed.push(Failure {
                name: name.clone(),
                error,
            }),
        }
    }
    // Files have already moved, so report that even if the bookkeeping fails. Leftover
    // entries are harmless: they are ignored for missing files and fingerprinted otherwise.
    result.state_error = decisions::save(saved, &decisions).err();
    Ok(result)
}
pub fn system_delete(path: &Path) -> Result<(), String> {
    let mut context = ::trash::TrashContext::default();
    context.set_delete_method(::trash::macos::DeleteMethod::NsFileManager);
    context
        .delete(path)
        .map_err(|e| format!("Kan ikke flytte til papirkurven: {e}"))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_discarded() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("state.json");
        for n in ["keep.jpg", "discard.jpg", "unknown.jpg"] {
            std::fs::write(d.path().join(n), b"abc").unwrap();
        }
        decisions::save(
            &p,
            &decisions::Decisions::from([
                (
                    "keep.jpg".into(),
                    entry(d.path(), "keep.jpg", Decision::Keep),
                ),
                (
                    "discard.jpg".into(),
                    entry(d.path(), "discard.jpg", Decision::Discard),
                ),
            ]),
        )
        .unwrap();
        let mut called = vec![];
        let r = perform(
            d.path(),
            &[
                "keep.jpg".into(),
                "unknown.jpg".into(),
                "discard.jpg".into(),
            ],
            &now(d.path(), &["keep.jpg", "unknown.jpg", "discard.jpg"]),
            &p,
            |path| {
                called.push(path.file_name().unwrap().to_string_lossy().into_owned());
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(called, vec!["discard.jpg"]);
        assert_eq!(r.failed.len(), 2);
        assert_eq!(r.bytes_freed, 3);
        assert!(!decisions::load(&p).unwrap().contains_key("discard.jpg"));
        assert!(
            perform(d.path(), &["../outside.jpg".into()], &[V0], &p, |_| panic!(
                "must not trash"
            ))
            .is_err()
        );
    }
    const V0: Fingerprint = Fingerprint {
        len: 0,
        modified_ms: 0,
        inode: 0,
    };
    /// What the UI would have shown for each file right now.
    fn now(dir: &Path, names: &[&str]) -> Vec<Fingerprint> {
        names
            .iter()
            .map(|n| scan::fingerprint(&dir.join(n)).unwrap_or(V0))
            .collect()
    }
    #[test]
    fn refuses_a_file_other_than_the_one_confirmed() {
        // The folder path was repointed to another folder with its own marked a.jpg: that
        // file is marked, unchanged since, but isn't the version the user confirmed.
        let shown = tempfile::tempdir().unwrap();
        std::fs::write(shown.path().join("a.jpg"), b"one").unwrap();
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("state.json");
        std::fs::write(d.path().join("a.jpg"), b"two!").unwrap();
        let marked = entry(d.path(), "a.jpg", Decision::Discard);
        decisions::save(&p, &decisions::Decisions::from([("a.jpg".into(), marked)])).unwrap();
        let confirmed = now(shown.path(), &["a.jpg"]);
        let r = perform(d.path(), &["a.jpg".into()], &confirmed, &p, |_| {
            panic!("must not trash")
        })
        .unwrap();
        assert!(r.trashed.is_empty());
        assert_eq!(r.failed.len(), 1);
        assert!(perform(d.path(), &["a.jpg".into()], &[], &p, |_| panic!()).is_err());
    }
    fn entry(dir: &Path, name: &str, decision: Decision) -> decisions::Entry {
        decisions::Entry {
            decision,
            fingerprint: Some(scan::fingerprint(&dir.join(name)).unwrap()),
        }
    }
    #[test]
    fn refuses_a_file_replaced_since_it_was_marked() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("state.json");
        std::fs::write(d.path().join("a.jpg"), b"one").unwrap();
        let marked = entry(d.path(), "a.jpg", Decision::Discard);
        decisions::save(&p, &decisions::Decisions::from([("a.jpg".into(), marked)])).unwrap();
        let confirmed = now(d.path(), &["a.jpg"]);
        std::fs::remove_file(d.path().join("a.jpg")).unwrap();
        std::fs::write(d.path().join("a.jpg"), b"another").unwrap();
        let r = perform(d.path(), &["a.jpg".into()], &confirmed, &p, |_| {
            panic!("must not trash")
        })
        .unwrap();
        assert!(r.trashed.is_empty());
        assert_eq!(r.failed.len(), 1);
    }
    #[test]
    fn reports_moved_files_when_saving_state_fails() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("state.json");
        std::fs::write(d.path().join("a.jpg"), b"one").unwrap();
        let marked = entry(d.path(), "a.jpg", Decision::Discard);
        decisions::save(&p, &decisions::Decisions::from([("a.jpg".into(), marked)])).unwrap();
        let confirmed = now(d.path(), &["a.jpg"]);
        let r = perform(d.path(), &["a.jpg".into()], &confirmed, &p, |_| {
            std::fs::remove_file(&p).unwrap();
            std::fs::create_dir(&p).unwrap();
            Ok(())
        })
        .unwrap();
        assert_eq!(r.trashed, vec!["a.jpg"]);
        assert!(r.state_error.is_some());
    }
}
