use crate::decisions::hash;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::UNIX_EPOCH,
};
pub fn convert(cache: &Path, path: &Path, edge: u32) -> Result<PathBuf, String> {
    let meta = fs::metadata(path).map_err(|e| format!("Kan ikke lese bildet: {e}"))?;
    let modified = meta
        .modified()
        .map_err(|e| format!("Kan ikke lese endringstid: {e}"))?
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("Ugyldig endringstid: {e}"))?
        .as_nanos();
    let mut key = path.as_os_str().as_encoded_bytes().to_vec();
    key.extend_from_slice(format!(":{}:{modified}:{edge}:v1", meta.len()).as_bytes());
    let out = cache.join(format!("{}.jpg", hash(&key)));
    if out.is_file() {
        return Ok(out);
    }
    fs::create_dir_all(cache).map_err(|e| format!("Kan ikke opprette bildebuffer: {e}"))?;
    let tmp = tempfile::Builder::new()
        .suffix(".jpg")
        .tempfile_in(cache)
        .map_err(|e| format!("Kan ikke opprette miniatyr: {e}"))?;
    // sips keeps the EXIF orientation with unrotated pixels; WebKit applies it.
    let output = Command::new("/usr/bin/sips")
        .args([
            "-s",
            "format",
            "jpeg",
            "-s",
            "formatOptions",
            "80",
            "-Z",
            &edge.to_string(),
        ])
        .arg(path)
        .arg("--out")
        .arg(tmp.path())
        .output()
        .map_err(|e| format!("Kan ikke starte bildekonvertering: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "Kan ikke konvertere bildet: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    // sips replaces the output inode, so reopen the path rather than using the temp handle.
    let converted =
        fs::File::open(tmp.path()).map_err(|e| format!("Kan ikke lese miniatyr: {e}"))?;
    if converted
        .metadata()
        .map_err(|e| format!("Kan ikke lese miniatyr: {e}"))?
        .len()
        == 0
    {
        return Err("Bildekonverteringen ga en tom fil.".into());
    }
    converted
        .sync_all()
        .map_err(|e| format!("Kan ikke lagre miniatyr: {e}"))?;
    tmp.persist(&out)
        .map_err(|e| format!("Kan ikke lagre miniatyr: {e}"))?;
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn orientation_cache_and_concurrency() {
        let d = tempfile::tempdir().unwrap();
        let source = d.path().join("portrait.jpg");
        fs::write(
            &source,
            include_bytes!("../tests/fixtures/orientation-6.jpg"),
        )
        .unwrap();
        let cache = d.path().join(".cache");
        let (a, b) = std::thread::scope(|s| {
            let a = s.spawn(|| convert(&cache, &source, 20).unwrap());
            let b = s.spawn(|| convert(&cache, &source, 20).unwrap());
            (a.join().unwrap(), b.join().unwrap())
        });
        assert_eq!(a, b);
        let mut reader = std::io::BufReader::new(fs::File::open(&a).unwrap());
        let metadata = exif::Reader::new()
            .read_from_container(&mut reader)
            .unwrap();
        assert_eq!(
            metadata
                .get_field(exif::Tag::Orientation, exif::In::PRIMARY)
                .unwrap()
                .value
                .get_uint(0),
            Some(6)
        );
        assert!(fs::metadata(a).unwrap().len() > 0);
        let mut changed = include_bytes!("../tests/fixtures/orientation-6.jpg").to_vec();
        changed.push(0);
        fs::write(&source, changed).unwrap();
        assert_ne!(b, convert(&cache, &source, 20).unwrap());
    }
}
