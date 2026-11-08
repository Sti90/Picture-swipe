//! The only code that talks to PhotoKit. It never runs in tests: they use a fake `Library`.
use crate::library::{Asset, DeleteOutcome, Library, Status};
use serde::Deserialize;
use std::{
    ffi::{CStr, CString, c_char, c_double, c_int},
    path::Path,
};

unsafe extern "C" {
    fn bk_status() -> *mut c_char;
    fn bk_request_access() -> *mut c_char;
    fn bk_list() -> *mut c_char;
    fn bk_delete(ids_json: *const c_char) -> *mut c_char;
    fn bk_render(
        id: *const c_char,
        edge: c_int,
        allow_network: c_int,
        out: *const c_char,
        timeout: c_double,
    ) -> *mut c_char;
    fn bk_free(value: *mut c_char);
}

/// A slow iCloud download gives up after this long, so it can't hold a permit forever.
const RENDER_TIMEOUT_SECONDS: c_double = 90.0;

/// Copies and frees a string returned by the bridge. `None` if the bridge returned NULL.
fn take(raw: *mut c_char) -> Option<String> {
    if raw.is_null() {
        return None;
    }
    // SAFETY: a non-null bridge result is a NUL-terminated buffer from malloc that nothing
    // else references. It is read here, then released exactly once by `bk_free`.
    unsafe {
        let copied = CStr::from_ptr(raw).to_string_lossy().into_owned();
        bk_free(raw);
        Some(copied)
    }
}

#[derive(Deserialize)]
struct StatusReply {
    status: Status,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum ListReply {
    Assets(Vec<(String, i64, i64)>),
    Error { error: String },
}
#[derive(Deserialize)]
#[serde(rename_all = "lowercase", tag = "outcome")]
enum DeleteReply {
    Deleted { missing: Vec<String> },
    Cancelled,
    Denied,
    Error { code: i64 },
}
#[derive(Deserialize)]
struct RenderReply {
    error: Option<String>,
}

fn parse_status(raw: *mut c_char) -> Status {
    // An unreadable reply is treated as no access, the safe direction.
    take(raw)
        .and_then(|s| serde_json::from_str::<StatusReply>(&s).ok())
        .map_or(Status::Denied, |r| r.status)
}

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

pub struct PhotoKit;

impl Library for PhotoKit {
    fn status(&self) -> Status {
        // SAFETY: takes no arguments, and the result is owned by `parse_status`.
        parse_status(unsafe { bk_status() })
    }
    fn request_access(&self) -> Status {
        // SAFETY: as above. Only called from the blocking pool, as the bridge requires.
        parse_status(unsafe { bk_request_access() })
    }
    fn list(&self) -> Result<Vec<Asset>, String> {
        // SAFETY: takes no arguments, and the result is owned by `take`.
        let text = take(unsafe { bk_list() }).ok_or("Kan ikke lese Bilder-biblioteket.")?;
        match serde_json::from_str(&text).map_err(|_| "Kan ikke lese Bilder-biblioteket.")? {
            ListReply::Assets(rows) => Ok(rows
                .into_iter()
                .map(|(id, created, modified)| Asset {
                    id,
                    created_ms: created.max(0) as u64,
                    modified_ms: modified.max(0) as u64,
                })
                .collect()),
            ListReply::Error { error } => Err(message(&error)),
        }
    }
    fn delete(&self, ids: &[String]) -> DeleteOutcome {
        let Some(input) = serde_json::to_vec(ids)
            .ok()
            .and_then(|v| CString::new(v).ok())
        else {
            return DeleteOutcome::Failed("Ugyldig forespørsel.".into());
        };
        // SAFETY: `input` is a valid NUL-terminated string that outlives the call, and the
        // bridge doesn't keep it. The result is owned by `take`.
        let reply = take(unsafe { bk_delete(input.as_ptr()) });
        match reply.and_then(|s| serde_json::from_str(&s).ok()) {
            Some(DeleteReply::Deleted { missing }) => DeleteOutcome::Deleted { missing },
            Some(DeleteReply::Cancelled) => DeleteOutcome::Cancelled,
            Some(DeleteReply::Denied) => DeleteOutcome::Failed(message("denied")),
            Some(DeleteReply::Error { code }) => {
                DeleteOutcome::Failed(format!("Bilder kunne ikke slette bildene (kode {code})."))
            }
            None => DeleteOutcome::Failed("Bilder ga et ugyldig svar.".into()),
        }
    }
    fn render(&self, id: &str, edge: u32, network: bool, out: &Path) -> Result<(), String> {
        let invalid = || "Ugyldig forespørsel.".to_string();
        let id = CString::new(id).map_err(|_| invalid())?;
        let out = CString::new(out.to_str().ok_or_else(invalid)?).map_err(|_| invalid())?;
        let edge = c_int::try_from(edge).map_err(|_| invalid())?;
        // SAFETY: `id` and `out` are valid NUL-terminated strings that outlive the call, and
        // the bridge doesn't keep them. The result is owned by `take`.
        let reply = take(unsafe {
            bk_render(
                id.as_ptr(),
                edge,
                c_int::from(network),
                out.as_ptr(),
                RENDER_TIMEOUT_SECONDS,
            )
        })
        .and_then(|s| serde_json::from_str::<RenderReply>(&s).ok())
        .ok_or_else(|| message("invalid"))?;
        match reply.error {
            Some(code) => Err(message(&code)),
            None => Ok(()),
        }
    }
}
