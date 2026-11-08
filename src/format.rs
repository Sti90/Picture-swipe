//! Norwegian text for numbers, sizes and dates.
use chrono::{DateTime, Datelike as _, Local, Timelike as _};

use crate::{library, scan::Photo};

const MONTHS: [&str; 12] = [
    "jan.", "feb.", "mars", "apr.", "mai", "juni", "juli", "aug.", "sep.", "okt.", "nov.", "des.",
];

pub fn count(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push('\u{a0}');
        }
        out.push(digit);
    }
    out
}

pub fn images(n: usize) -> String {
    format!("{} {}", count(n), if n == 1 { "bilde" } else { "bilder" })
}

/// Decimal units, the same way Finder counts.
pub fn bytes(n: u64) -> String {
    if n < 1000 {
        return format!("{n} byte");
    }
    let units = ["kB", "MB", "GB", "TB"];
    let mut value = n as f64;
    let mut unit = 0;
    value /= 1000.0;
    while value >= 1000.0 && unit < units.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    let rounded = format!("{value:.1}").replace('.', ",");
    format!("{} {}", rounded.trim_end_matches(",0"), units[unit])
}

pub fn when(ms: u64) -> String {
    let Some(utc) = DateTime::from_timestamp_millis(ms as i64) else {
        return "Ukjent dato".to_owned();
    };
    let date = utc.with_timezone(&Local);
    format!(
        "{}. {} {}, {:02}:{:02}",
        date.day(),
        MONTHS[date.month0() as usize],
        date.year(),
        date.hour(),
        date.minute()
    )
}

pub fn is_library_photo(photo: &Photo) -> bool {
    library::is_asset(&photo.path)
}

/// What to call a photo on screen. A library photo's `name` is an identifier and is never shown.
pub fn photo_title(photo: &Photo) -> String {
    if !is_library_photo(photo) {
        photo.name.clone()
    } else if photo.modified != 0 {
        when(photo.modified)
    } else {
        "Ukjent dato".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_read_the_norwegian_way() {
        assert_eq!(count(1234567), "1\u{a0}234\u{a0}567");
        assert_eq!(images(1), "1 bilde");
        assert_eq!(images(2), "2 bilder");
        assert_eq!(bytes(999), "999 byte");
        assert_eq!(bytes(1500), "1,5 kB");
        assert_eq!(bytes(2_000_000), "2 MB");
    }
}
