//! Decodes photos on background threads and keeps the textures on screen. Read-only: the
//! only files it writes are its own temporary and cached JPEGs.
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread::JoinHandle,
};

use eframe::egui::{ColorImage, Context, TextureHandle, TextureOptions};
use image::{DynamicImage, ImageDecoder as _, codecs::jpeg::JpegDecoder};

use crate::{library::LibrarySource, library::is_asset, thumbs};

const PREVIEW_EDGE: &str = "2400";
const THUMBNAIL_EDGE: u32 = 512;
const PREVIEW_WORKERS: usize = 3;
const THUMBNAIL_WORKERS: usize = 4;
const MAX_THUMBNAIL_TEXTURES: usize = 600;

type Job = (usize, String);
type Decoded = (usize, Result<ColorImage, String>);
type Render = Arc<dyn Fn(&str) -> Result<ColorImage, String> + Send + Sync>;

struct Shared {
    wanted: Mutex<VecDeque<Job>>,
    wake: Condvar,
    stop: AtomicBool,
}

/// Background decoders with a priority queue that follows what's on screen.
struct Pool {
    shared: Arc<Shared>,
    done: Receiver<Decoded>,
    workers: Vec<JoinHandle<()>>,
}

fn merge(queue: &mut VecDeque<Job>, order: &[usize], jobs: Vec<Job>) {
    queue.retain(|(photo, _)| order.contains(photo));
    queue.extend(jobs);
    queue
        .make_contiguous()
        .sort_by_key(|(photo, _)| order.iter().position(|p| p == photo));
}

fn next_job(shared: &Shared) -> Option<Job> {
    let mut wanted = shared.wanted.lock().ok()?;
    loop {
        if shared.stop.load(Ordering::SeqCst) {
            return None;
        }
        if let Some(job) = wanted.pop_front() {
            return Some(job);
        }
        wanted = shared.wake.wait(wanted).ok()?;
    }
}

fn work(shared: &Shared, send: &Sender<Decoded>, ctx: &Context, render: &Render) {
    while let Some((photo, path)) = next_job(shared) {
        if send.send((photo, render(&path))).is_err() {
            return;
        }
        ctx.request_repaint();
    }
}

impl Pool {
    fn new(ctx: &Context, count: usize, render: Render) -> Self {
        let shared = Arc::new(Shared {
            wanted: Mutex::default(),
            wake: Condvar::new(),
            stop: AtomicBool::new(false),
        });
        let (send, done) = mpsc::channel();
        let workers = (0..count)
            .map(|_| {
                let (shared, send, ctx, render) =
                    (shared.clone(), send.clone(), ctx.clone(), render.clone());
                std::thread::spawn(move || work(&shared, &send, &ctx, &render))
            })
            .collect();
        Self {
            shared,
            done,
            workers,
        }
    }

    fn update(&self, order: &[usize], jobs: Vec<Job>) {
        let added = !jobs.is_empty();
        if let Ok(mut wanted) = self.shared.wanted.lock() {
            merge(&mut wanted, order, jobs);
        }
        if added {
            self.shared.wake.notify_all();
        }
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        // Holding the lock means no worker can be between checking `stop` and waiting.
        let guard = self.shared.wanted.lock();
        self.shared.stop.store(true, Ordering::SeqCst);
        drop(guard);
        self.shared.wake.notify_all();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

fn decode_jpeg(bytes: Vec<u8>) -> Result<ColorImage, String> {
    let mut decoder = JpegDecoder::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let orientation = decoder.orientation().map_err(|e| e.to_string())?;
    let mut image = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    image.apply_orientation(orientation);
    let rgba = image.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Ok(ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()))
}

/// `sips` handles HEIC and camera RAW. It keeps EXIF orientation as a tag on unrotated
/// pixels, so the rotation is applied by `decode_jpeg`.
pub fn decode_with_sips(path: &Path, target: &Path) -> Result<ColorImage, String> {
    let output = Command::new("/usr/bin/sips")
        .args([
            "-s",
            "format",
            "jpeg",
            "-s",
            "formatOptions",
            "85",
            "-Z",
            PREVIEW_EDGE,
        ])
        .arg(path)
        .arg("--out")
        .arg(target)
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    decode_jpeg(fs::read(target).map_err(|e| e.to_string())?)
}

fn decode_file(path: &str) -> Result<ColorImage, String> {
    decode_jpeg(fs::read(path).map_err(|e| e.to_string())?)
}

#[derive(Default)]
struct Slot {
    requested: HashSet<usize>,
    failed: HashSet<usize>,
}

pub struct Images {
    previews: HashMap<usize, TextureHandle>,
    thumbnails: HashMap<usize, (TextureHandle, u64)>,
    preview_state: Slot,
    thumbnail_state: Slot,
    frame: u64,
    pools: Option<(Pool, Pool)>,
    _temp: Option<tempfile::TempDir>,
}

impl Images {
    /// Declared so `_temp` outlives the workers using it: `pools` is dropped first.
    pub fn start(
        ctx: &Context,
        library: Arc<LibrarySource>,
        cache: PathBuf,
    ) -> Result<Self, String> {
        let temp =
            tempfile::tempdir().map_err(|e| format!("Kan ikke lage midlertidig mappe: {e}"))?;
        let out = temp.path().to_path_buf();
        let counter = Arc::new(AtomicU64::new(0));
        let preview_library = library.clone();
        let previews: Render = Arc::new(move |path| {
            if is_asset(path) {
                return decode_file(&preview_library.image(path, true)?);
            }
            let target = out.join(format!("{}.jpg", counter.fetch_add(1, Ordering::Relaxed)));
            let image = decode_with_sips(Path::new(path), &target);
            let _ = fs::remove_file(&target);
            image
        });
        let thumbnails: Render = Arc::new(move |path| {
            if is_asset(path) {
                return decode_file(&library.image(path, false)?);
            }
            let file = thumbs::convert(&cache, Path::new(path), THUMBNAIL_EDGE)?;
            decode_file(&file.to_string_lossy())
        });
        Ok(Self {
            pools: Some((
                Pool::new(ctx, PREVIEW_WORKERS, previews),
                Pool::new(ctx, THUMBNAIL_WORKERS, thumbnails),
            )),
            _temp: Some(temp),
            ..Self::idle()
        })
    }

    pub fn idle() -> Self {
        Self {
            previews: HashMap::new(),
            thumbnails: HashMap::new(),
            preview_state: Slot::default(),
            thumbnail_state: Slot::default(),
            frame: 0,
            pools: None,
            _temp: None,
        }
    }

    /// Takes in finished images, then asks for what's on screen. `path` looks a photo up.
    pub fn sync(
        &mut self,
        ctx: &Context,
        previews: &[usize],
        thumbnails: &[usize],
        path: impl Fn(usize) -> String,
    ) {
        self.frame += 1;
        let frame = self.frame;
        let Some((preview_pool, thumbnail_pool)) = &self.pools else {
            return;
        };
        let keep: HashSet<usize> = previews.iter().copied().collect();
        while let Ok((photo, image)) = preview_pool.done.try_recv() {
            match image {
                Ok(image) if keep.contains(&photo) => {
                    let texture =
                        ctx.load_texture(format!("p{photo}"), image, TextureOptions::LINEAR);
                    self.previews.insert(photo, texture);
                }
                Ok(_) => {
                    self.preview_state.requested.remove(&photo);
                }
                Err(_) => {
                    self.preview_state.failed.insert(photo);
                }
            }
        }
        while let Ok((photo, image)) = thumbnail_pool.done.try_recv() {
            match image {
                Ok(image) => {
                    let texture =
                        ctx.load_texture(format!("t{photo}"), image, TextureOptions::LINEAR);
                    self.thumbnails.insert(photo, (texture, frame));
                }
                Err(_) => {
                    self.thumbnail_state.failed.insert(photo);
                }
            }
        }
        self.previews.retain(|photo, _| keep.contains(photo));
        let state = &mut self.preview_state;
        state
            .requested
            .retain(|photo| keep.contains(photo) || state.failed.contains(photo));
        let jobs = previews
            .iter()
            .filter(|photo| self.preview_state.requested.insert(**photo))
            .map(|photo| (*photo, path(*photo)))
            .collect();
        preview_pool.update(previews, jobs);

        let visible: HashSet<usize> = thumbnails.iter().copied().collect();
        for photo in thumbnails {
            if let Some((_, used)) = self.thumbnails.get_mut(photo) {
                *used = frame;
            }
        }
        if self.thumbnails.len() > MAX_THUMBNAIL_TEXTURES {
            let mut ages: Vec<(u64, usize)> = self
                .thumbnails
                .iter()
                .map(|(photo, (_, used))| (*used, *photo))
                .collect();
            ages.sort_unstable();
            for (_, photo) in ages
                .iter()
                .take(self.thumbnails.len() - MAX_THUMBNAIL_TEXTURES)
            {
                if !visible.contains(photo) {
                    self.thumbnails.remove(photo);
                    self.thumbnail_state.requested.remove(photo);
                }
            }
        }
        // A photo scrolled away before it started has to be asked for again.
        let state = &mut self.thumbnail_state;
        let finished = &self.thumbnails;
        state.requested.retain(|photo| {
            visible.contains(photo) || finished.contains_key(photo) || state.failed.contains(photo)
        });
        let jobs = thumbnails
            .iter()
            .filter(|photo| self.thumbnail_state.requested.insert(**photo))
            .map(|photo| (*photo, path(*photo)))
            .collect();
        thumbnail_pool.update(thumbnails, jobs);
    }

    pub fn preview(&self, photo: usize) -> Option<&TextureHandle> {
        self.previews.get(&photo)
    }
    pub fn thumbnail(&self, photo: usize) -> Option<&TextureHandle> {
        self.thumbnails.get(&photo).map(|(texture, _)| texture)
    }
    pub fn preview_failed(&self, photo: usize) -> bool {
        self.preview_state.failed.contains(&photo)
    }
    pub fn thumbnail_failed(&self, photo: usize) -> bool {
        self.thumbnail_state.failed.contains(&photo)
    }

    /// Forgets a failed preview so the next frame asks for it again.
    pub fn retry_preview(&mut self, photo: usize) {
        self.preview_state.failed.remove(&photo);
        self.preview_state.requested.remove(&photo);
    }

    #[cfg(test)]
    pub fn insert_for_test(&mut self, ctx: &Context, photo: usize, image: ColorImage) {
        let preview = ctx.load_texture(format!("p{photo}"), image.clone(), TextureOptions::LINEAR);
        let thumbnail = ctx.load_texture(format!("t{photo}"), image, TextureOptions::LINEAR);
        self.previews.insert(photo, preview);
        self.thumbnails.insert(photo, (thumbnail, 0));
    }

    #[cfg(test)]
    pub fn fail_preview_for_test(&mut self, photo: usize) {
        self.previews.remove(&photo);
        self.preview_state.failed.insert(photo);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(photo: usize) -> Job {
        (photo, photo.to_string())
    }

    #[test]
    fn new_jobs_never_push_out_a_photo_still_waiting() {
        let mut queue = VecDeque::from([job(3)]);
        merge(&mut queue, &[1, 2, 3, 4], vec![job(4)]);
        assert_eq!(queue.iter().map(|(p, _)| *p).collect::<Vec<_>>(), [3, 4]);
    }

    #[test]
    fn photos_no_longer_needed_leave_the_queue_and_the_rest_are_ordered() {
        let mut queue = VecDeque::from([job(1), job(5)]);
        merge(&mut queue, &[6, 5], vec![job(6)]);
        assert_eq!(queue.iter().map(|(p, _)| *p).collect::<Vec<_>>(), [6, 5]);
    }

    #[test]
    fn exif_rotation_is_applied() {
        let dir = tempfile::tempdir().unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/orientation-6.jpg");
        let raw = image::load_from_memory(&fs::read(&source).unwrap()).unwrap();
        assert_eq!(
            raw.width(),
            raw.height() * 2,
            "the file is stored landscape"
        );
        let decoded = decode_with_sips(&source, &dir.path().join("out.jpg")).unwrap();
        assert_eq!(decoded.size[0] * 2, decoded.size[1], "and shown portrait");
    }

    #[test]
    fn dropping_a_pool_ends_its_workers() {
        let render: Render = Arc::new(|_| Err("none".into()));
        let pool = Pool::new(&Context::default(), 2, render);
        assert_eq!(pool.workers.len(), 2);
        drop(pool);
    }
}
