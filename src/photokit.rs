//! The only code that talks to PhotoKit. It never runs in tests: they use a fake `Library`.
//! Every function here blocks, and PhotoKit shows its dialogs on the main thread, so call
//! these only from worker threads.
use std::{
    panic::AssertUnwindSafe,
    path::Path,
    ptr,
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

use block2::RcBlock;
use objc2::{
    AnyThread as _, exception,
    rc::{Retained, autoreleasepool},
    runtime::{AnyObject, ProtocolObject},
};
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage, NSImageCompressionFactor};
use objc2_foundation::{
    NSArray, NSDictionary, NSError, NSNumber, NSSize, NSSortDescriptor, NSString,
};
use objc2_photos::{
    PHAccessLevel, PHAsset, PHAssetChangeRequest, PHAssetMediaType, PHAssetSourceType,
    PHAuthorizationStatus, PHFetchOptions, PHFetchResult, PHImageCancelledKey, PHImageContentMode,
    PHImageErrorKey, PHImageManager, PHImageRequestOptions, PHImageRequestOptionsDeliveryMode,
    PHImageRequestOptionsResizeMode, PHImageResultIsDegradedKey, PHImageResultIsInCloudKey,
    PHPhotoLibrary,
};

use crate::library::{Asset, DeleteOutcome, Library, Status};

/// A slow iCloud download gives up after this long, so it can't hold a worker forever.
const RENDER_TIMEOUT: Duration = Duration::from_secs(90);
// PHPhotosError values. NSUserCancelledError shares the cancelled value.
const CANCELLED: isize = 3072;
const ACCESS_RESTRICTED: isize = 3310;
const ACCESS_DENIED: isize = 3311;
const JPEG_QUALITY: f64 = 0.8;

fn message(code: &str) -> String {
    match code {
        "missing" => "Bildet finnes ikke lenger i Bilder.",
        "denied" => "Bildesveip har ikke lenger tilgang til Bilder.",
        "icloud" => "Bildet kunne ikke hentes fra iCloud.",
        "timeout" => "Det tok for lang tid å hente bildet fra iCloud.",
        _ => "Bildet kunne ikke vises.",
    }
    .into()
}

/// Runs `work`, turning an Objective-C exception into `None` so it can't unwind into Rust.
fn guarded<T>(work: impl FnOnce() -> T) -> Option<T> {
    autoreleasepool(|_| exception::catch(AssertUnwindSafe(work)).ok())
}

fn current_status() -> PHAuthorizationStatus {
    // SAFETY: a class method taking a valid enum value.
    unsafe { PHPhotoLibrary::authorizationStatusForAccessLevel(PHAccessLevel::ReadWrite) }
}

fn to_status(status: PHAuthorizationStatus) -> Status {
    match status {
        PHAuthorizationStatus::NotDetermined => Status::NotDetermined,
        PHAuthorizationStatus::Restricted => Status::Restricted,
        PHAuthorizationStatus::Limited => Status::Limited,
        PHAuthorizationStatus::Authorized => Status::Authorized,
        _ => Status::Denied,
    }
}

fn has_access() -> bool {
    matches!(
        current_status(),
        PHAuthorizationStatus::Authorized | PHAuthorizationStatus::Limited
    )
}

fn is_eligible(asset: &PHAsset) -> bool {
    // A burst is one row standing in for many photos, so deleting it can't be an informed
    // choice about what the user saw.
    // SAFETY: plain property reads on a live asset.
    unsafe {
        asset.mediaType() == PHAssetMediaType::Image
            && !asset.isHidden()
            && !asset.representsBurst()
            && asset
                .sourceType()
                .contains(PHAssetSourceType::TypeUserLibrary)
    }
}

fn millis(date: Option<Retained<objc2_foundation::NSDate>>) -> u64 {
    date.map_or(0, |d| (d.timeIntervalSince1970() * 1000.0).max(0.0) as u64)
}

/// A one-shot hand-off from a PhotoKit callback to the waiting thread. A callback that
/// arrives after the waiter gave up only touches this shared cell.
struct Slot<T> {
    value: Mutex<Option<T>>,
    ready: Condvar,
}

impl<T> Slot<T> {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            value: Mutex::new(None),
            ready: Condvar::new(),
        })
    }
    fn put(&self, value: T) {
        if let Ok(mut slot) = self.value.lock() {
            slot.get_or_insert(value);
            self.ready.notify_all();
        }
    }
    fn wait(&self, timeout: Duration) -> Option<T> {
        let deadline = Instant::now() + timeout;
        let mut slot = self.value.lock().ok()?;
        loop {
            if let Some(value) = slot.take() {
                return Some(value);
            }
            let left = deadline.checked_duration_since(Instant::now())?;
            slot = self.ready.wait_timeout(slot, left).ok()?.0;
        }
    }
}

/// An image handed from the callback to the one thread that waits for it.
struct Handed(Retained<NSImage>);
// SAFETY: the callback gives up its reference when it stores this, so exactly one thread
// uses the image afterwards, and it only reads from it.
unsafe impl Send for Handed {}

type Fetched = Result<Handed, &'static str>;

fn flag(info: &NSDictionary<NSString, AnyObject>, key: &NSString) -> bool {
    info.objectForKey(key)
        .and_then(|value| value.downcast::<NSNumber>().ok())
        .is_some_and(|number| number.boolValue())
}

fn fetch_image(
    asset: &PHAsset,
    edge: f64,
    network: bool,
    timeout: Duration,
) -> Result<Handed, &'static str> {
    // SAFETY: setters on an options object owned here, taking valid values.
    let options = unsafe {
        let options = PHImageRequestOptions::new();
        options.setNetworkAccessAllowed(network);
        // A synchronous request can't be cancelled, because its id arrives only once it has
        // finished. Requests that may reach the network run asynchronously, with a deadline.
        options.setSynchronous(!network);
        options.setDeliveryMode(if network {
            PHImageRequestOptionsDeliveryMode::HighQualityFormat
        } else {
            PHImageRequestOptionsDeliveryMode::Opportunistic
        });
        options.setResizeMode(if network {
            PHImageRequestOptionsResizeMode::Exact
        } else {
            PHImageRequestOptionsResizeMode::Fast
        });
        options
    };
    let slot: Arc<Slot<Fetched>> = Slot::new();
    let target = slot.clone();
    let handler = RcBlock::new(move |image: *mut NSImage, info: *mut NSDictionary| {
        // SAFETY: PhotoKit passes a valid dictionary of string keys, or NULL, that lives for
        // this call, and `image` is a valid image or NULL.
        let (info, image) = unsafe {
            let info = info.cast::<NSDictionary<NSString, AnyObject>>();
            (info.as_ref(), Retained::retain(image))
        };
        let Some(info) = info else {
            target.put(image.map(Handed).ok_or("empty"));
            return;
        };
        // SAFETY: these keys are immutable statics exported by Photos.
        let (cancelled, error, degraded, in_cloud) = unsafe {
            (
                flag(info, PHImageCancelledKey),
                info.objectForKey(PHImageErrorKey),
                flag(info, PHImageResultIsDegradedKey),
                flag(info, PHImageResultIsInCloudKey),
            )
        };
        if cancelled {
            target.put(Err("cancelled"));
        } else if let Some(error) = error {
            let code = error.downcast::<NSError>().map_or(0, |e| e.code());
            let denied = code == ACCESS_DENIED || code == ACCESS_RESTRICTED;
            target.put(Err(if denied { "denied" } else { "icloud" }));
        } else if let Some(image) = image {
            // With the network allowed a degraded image is followed by the final one.
            if !(degraded && network) {
                target.put(Ok(Handed(image)));
            }
        } else {
            target.put(Err(if in_cloud { "icloud" } else { "empty" }));
        }
    });
    // SAFETY: the asset and options are live, and the block lives as long as PhotoKit holds it.
    let (manager, request) = unsafe {
        let manager = PHImageManager::defaultManager();
        let request = manager.requestImageForAsset_targetSize_contentMode_options_resultHandler(
            asset,
            NSSize::new(edge, edge),
            PHImageContentMode::AspectFit,
            Some(&options),
            &handler,
        );
        (manager, request)
    };
    match slot.wait(timeout) {
        Some(result) => result,
        None => {
            // SAFETY: `request` came from this manager.
            unsafe { manager.cancelImageRequest(request) };
            Err("timeout")
        }
    }
}

fn write_jpeg(image: &NSImage, out: &Path) -> Result<(), &'static str> {
    // SAFETY: a null proposed rect and no context are documented as valid, and the bitmap
    // and its properties are owned here for the duration of the calls.
    let data = unsafe {
        let cg = image
            .CGImageForProposedRect_context_hints(ptr::null_mut(), None, None)
            .ok_or("empty")?;
        let bitmap = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &cg);
        let quality = NSNumber::new_f64(JPEG_QUALITY);
        let quality: &AnyObject = &quality;
        let properties = NSDictionary::from_slices(&[NSImageCompressionFactor], &[quality]);
        bitmap
            .representationUsingType_properties(NSBitmapImageFileType::JPEG, &properties)
            .ok_or("write")?
    };
    let bytes = data.to_vec();
    if bytes.is_empty() {
        return Err("write");
    }
    // Writes into the file the caller created, keeping its private permissions.
    std::fs::write(out, bytes).map_err(|_| "write")
}

fn render(id: &str, edge: u32, network: bool, out: &Path) -> Result<(), &'static str> {
    if !has_access() {
        return Err("denied");
    }
    let identifiers = NSArray::from_retained_slice(&[NSString::from_str(id)]);
    // SAFETY: a valid array of strings, and no options is documented as valid.
    let fetched = unsafe { PHAsset::fetchAssetsWithLocalIdentifiers_options(&identifiers, None) };
    let asset = first(&fetched)
        .filter(|asset| is_eligible(asset))
        .ok_or("missing")?;
    let edge = f64::from(edge);
    let image = match fetch_image(&asset, edge, network, RENDER_TIMEOUT) {
        // Without the network a smaller local rendition may still exist.
        Err("icloud") if !network => fetch_image(&asset, edge / 2.0, false, RENDER_TIMEOUT)?,
        other => other?,
    };
    write_jpeg(&image.0, out)
}

fn first(fetched: &PHFetchResult<PHAsset>) -> Option<Retained<PHAsset>> {
    // SAFETY: a plain read of a live fetch result.
    unsafe { fetched.firstObject() }
}

fn delete(ids: &[String]) -> DeleteOutcome {
    if !has_access() {
        // Without access a fetch comes back empty, which would look like missing photos.
        return DeleteOutcome::Failed(message("denied"));
    }
    let identifiers: Retained<NSArray<NSString>> = NSArray::from_retained_slice(
        &ids.iter()
            .map(|id| NSString::from_str(id))
            .collect::<Vec<_>>(),
    );
    // SAFETY: a valid array of strings, and no options is documented as valid.
    let fetched = unsafe { PHAsset::fetchAssetsWithLocalIdentifiers_options(&identifiers, None) };
    let mut assets = Vec::new();
    let mut found = std::collections::HashSet::new();
    // SAFETY: indexes stay below `count`.
    unsafe {
        for index in 0..fetched.count() {
            let asset = fetched.objectAtIndex(index);
            if is_eligible(&asset) {
                found.insert(asset.localIdentifier().to_string());
                assets.push(asset);
            }
        }
    }
    let missing: Vec<String> = ids
        .iter()
        .filter(|id| !found.contains(*id))
        .cloned()
        .collect();
    if assets.is_empty() {
        return DeleteOutcome::Deleted { missing };
    }
    let assets = NSArray::from_retained_slice(&assets);
    let change = RcBlock::new({
        let assets = assets.clone();
        move || {
            // SAFETY: called by Photos inside its change block, with an array of assets.
            unsafe { PHAssetChangeRequest::deleteAssets(ProtocolObject::from_ref(&*assets)) };
        }
    });
    // SAFETY: the block outlives this call, and `performChangesAndWait` doesn't keep it.
    // Photos itself asks the user to confirm, and moves the assets to Recently Deleted.
    let result = unsafe {
        PHPhotoLibrary::sharedPhotoLibrary()
            .performChangesAndWait_error(RcBlock::as_ptr(&change).cast())
    };
    match result {
        Ok(()) => DeleteOutcome::Deleted { missing },
        Err(error) => match error.code() {
            CANCELLED => DeleteOutcome::Cancelled,
            ACCESS_DENIED | ACCESS_RESTRICTED => DeleteOutcome::Failed(message("denied")),
            code => {
                DeleteOutcome::Failed(format!("Bilder kunne ikke slette bildene (kode {code})."))
            }
        },
    }
}

pub struct PhotoKit;

impl Library for PhotoKit {
    fn status(&self) -> Status {
        // An exception is treated as no access, the safe direction.
        guarded(current_status).map_or(Status::Denied, to_status)
    }

    fn request_access(&self) -> Status {
        guarded(|| {
            let slot: Arc<Slot<PHAuthorizationStatus>> = Slot::new();
            let target = slot.clone();
            let handler = RcBlock::new(move |status| target.put(status));
            // SAFETY: the block lives as long as PhotoKit holds it.
            unsafe {
                PHPhotoLibrary::requestAuthorizationForAccessLevel_handler(
                    PHAccessLevel::ReadWrite,
                    &handler,
                );
            }
            // The user may leave the prompt open for as long as they like.
            slot.wait(Duration::from_secs(60 * 60 * 24))
                .unwrap_or(PHAuthorizationStatus::Denied)
        })
        .map_or(Status::Denied, to_status)
    }

    fn list(&self) -> Result<Vec<Asset>, String> {
        guarded(|| {
            if !has_access() {
                return Err(message("denied"));
            }
            let sort = NSSortDescriptor::sortDescriptorWithKey_ascending(
                Some(&NSString::from_str("creationDate")),
                true,
            );
            // SAFETY: setters on an options object owned here, and indexes stay below `count`.
            unsafe {
                let options = PHFetchOptions::new();
                options.setIncludeHiddenAssets(false);
                options.setIncludeAllBurstAssets(false);
                options.setIncludeAssetSourceTypes(PHAssetSourceType::TypeUserLibrary);
                options.setSortDescriptors(Some(&NSArray::from_retained_slice(&[sort])));
                let assets = PHAsset::fetchAssetsWithMediaType_options(
                    PHAssetMediaType::Image,
                    Some(&options),
                );
                let mut rows = Vec::with_capacity(assets.count());
                for index in 0..assets.count() {
                    autoreleasepool(|_| {
                        let asset = assets.objectAtIndex(index);
                        if is_eligible(&asset) {
                            rows.push(Asset {
                                id: asset.localIdentifier().to_string(),
                                created_ms: millis(asset.creationDate()),
                                modified_ms: millis(asset.modificationDate()),
                            });
                        }
                    });
                }
                Ok(rows)
            }
        })
        .unwrap_or_else(|| Err("Kan ikke lese Bilder-biblioteket.".into()))
    }

    fn delete(&self, ids: &[String]) -> DeleteOutcome {
        guarded(|| delete(ids))
            .unwrap_or_else(|| DeleteOutcome::Failed("Bilder ga et ugyldig svar.".into()))
    }

    fn render(&self, id: &str, edge: u32, network: bool, out: &Path) -> Result<(), String> {
        match guarded(|| render(id, edge, network, out)) {
            Some(Ok(())) => Ok(()),
            Some(Err(code)) => Err(message(code)),
            None => Err(message("exception")),
        }
    }
}
