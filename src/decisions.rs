use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Decision {
    #[default]
    Undecided,
    Keep,
    Discard,
}
/// Identifies the exact file a decision was made about, so that a different file
/// later saved under the same name never inherits it.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Fingerprint {
    pub len: u64,
    pub modified_ms: u64,
    pub inode: u64,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Entry {
    pub decision: Decision,
    pub fingerprint: Option<Fingerprint>,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Stored {
    Entry(Entry),
    Legacy(Decision),
}
pub type Decisions = BTreeMap<String, Entry>;
pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn location(data: &Path, folder: &Path) -> PathBuf {
    data.join(format!(
        "{}.json",
        hash(folder.as_os_str().as_encoded_bytes())
    ))
}
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("Ugyldig lagringssti.")?;
    fs::create_dir_all(parent).map_err(|e| format!("Kan ikke opprette lagringsmappe: {e}"))?;
    let mut tmp =
        tempfile::NamedTempFile::new_in(parent).map_err(|e| format!("Kan ikke lagre: {e}"))?;
    tmp.write_all(bytes)
        .and_then(|()| tmp.as_file().sync_all())
        .map_err(|e| format!("Kan ikke lagre: {e}"))?;
    tmp.persist(path)
        .map_err(|e| format!("Kan ikke fullføre lagringen: {e}"))?;
    fs::File::open(parent)
        .and_then(|f| f.sync_all())
        .map_err(|e| format!("Kan ikke sikre lagringen: {e}"))
}
pub fn load(path: &Path) -> Result<Decisions, String> {
    load_checked(path).map(|(decisions, _)| decisions)
}
/// The flag is true when the saved file was damaged and has been set aside.
pub fn load_checked(path: &Path) -> Result<(Decisions, bool), String> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((Decisions::new(), false)),
        Err(e) => return Err(format!("Kan ikke lese lagrede valg: {e}")),
    };
    match serde_json::from_slice::<BTreeMap<String, Stored>>(&bytes) {
        Ok(stored) => {
            let mut d: Decisions = stored
                .into_iter()
                .map(|(name, value)| {
                    let entry = match value {
                        Stored::Entry(entry) => entry,
                        Stored::Legacy(decision) => Entry {
                            decision,
                            fingerprint: None,
                        },
                    };
                    (name, entry)
                })
                .collect();
            d.retain(|n, v| {
                crate::scan::validate_name(n).is_ok() && v.decision != Decision::Undecided
            });
            Ok((d, false))
        }
        Err(e) => {
            // Keep each damaged version, including when a previous backup already exists.
            let mut backup = path.with_extension("json.corrupt");
            let mut i = 0;
            while backup.exists() {
                i += 1;
                backup = path.with_extension(format!("json.{i}.corrupt"));
            }
            fs::rename(path, &backup)
                .map_err(|e| format!("Skadede valg kunne ikke sikkerhetskopieres: {e}"))?;
            eprintln!("Skadede valg bevart i {}: {e}", backup.display());
            Ok((Decisions::new(), true))
        }
    }
}
/// Records a decision about the exact file version the user was shown. Nothing is
/// changed if any file differs from that version.
pub fn record(
    folder: &Path,
    values: &mut Decisions,
    names: Vec<String>,
    versions: Vec<Fingerprint>,
    decision: Decision,
) -> Result<(), String> {
    if names.len() != versions.len() {
        return Err("Ugyldig forespørsel.".into());
    }
    let mut checked = Vec::with_capacity(names.len());
    for (name, shown) in names.into_iter().zip(versions) {
        let current = crate::scan::fingerprint(&crate::scan::file(folder, &name)?)?;
        if decision != Decision::Undecided && current != shown {
            return Err("Filen er endret siden den ble vist. Åpne mappen på nytt.".into());
        }
        checked.push((name, current));
    }
    for (name, current) in checked {
        if decision == Decision::Undecided {
            values.remove(&name);
        } else {
            let fingerprint = Some(current);
            values.insert(
                name,
                Entry {
                    decision,
                    fingerprint,
                },
            );
        }
    }
    Ok(())
}
pub fn save(path: &Path, decisions: &Decisions) -> Result<(), String> {
    let filtered: Decisions = decisions
        .iter()
        .filter(|(_, v)| v.decision != Decision::Undecided)
        .map(|(n, v)| (n.clone(), *v))
        .collect();
    atomic_write(
        path,
        &serde_json::to_vec(&filtered).map_err(|e| format!("Kan ikke lagre valg: {e}"))?,
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn persistence() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("state.json");
        let values = Decisions::from([
            ("a.jpg".into(), unstamped(Decision::Keep)),
            ("b.jpg".into(), unstamped(Decision::Undecided)),
        ]);
        save(&p, &values).unwrap();
        let saved = load(&p).unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved["a.jpg"].decision, Decision::Keep);
        fs::write(&p, b"broken").unwrap();
        let (recovered, reset) = load_checked(&p).unwrap();
        assert!(recovered.is_empty() && reset);
        assert_eq!(
            fs::read(p.with_extension("json.corrupt")).unwrap(),
            b"broken"
        );
        save(&p, &values).unwrap();
        assert_eq!(load(&p).unwrap(), saved);
        assert!(!load_checked(&p).unwrap().1);
    }
    fn unstamped(decision: Decision) -> Entry {
        Entry {
            decision,
            fingerprint: None,
        }
    }
    #[test]
    fn refuses_a_decision_about_a_version_never_shown() {
        let d = tempfile::tempdir().unwrap();
        let file = d.path().join("a.jpg");
        fs::write(&file, b"shown").unwrap();
        let shown = crate::scan::fingerprint(&file).unwrap();
        fs::remove_file(&file).unwrap();
        fs::write(&file, b"replacement").unwrap();
        let mut values = Decisions::new();
        let names = || vec!["a.jpg".to_string()];
        assert!(
            record(
                d.path(),
                &mut values,
                names(),
                vec![shown],
                Decision::Discard
            )
            .is_err()
        );
        assert!(values.is_empty());
        let current = crate::scan::fingerprint(&file).unwrap();
        record(
            d.path(),
            &mut values,
            names(),
            vec![current],
            Decision::Discard,
        )
        .unwrap();
        assert_eq!(values["a.jpg"].fingerprint, Some(current));
    }
    #[test]
    fn reads_the_earlier_format() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("state.json");
        fs::write(&p, br#"{"a.jpg":"discard"}"#).unwrap();
        assert_eq!(load(&p).unwrap()["a.jpg"], unstamped(Decision::Discard));
    }
}
