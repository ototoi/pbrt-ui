//! Recently opened projects and PBRT scenes.
//!
//! Desktop-only display help: the core and the wire never see this list.
//! Loading, merging and saving are pure so they can be tested without a window.
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const MAX_ENTRIES: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecentKind {
    Project,
    Pbrt,
}

impl RecentKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Pbrt => "pbrt",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentEntry {
    pub kind: RecentKind,
    pub path: PathBuf,
    pub title: String,
    /// RFC 3339 in UTC (`2026-10-25T10:12:00Z`).
    pub opened_at: String,
}

/// The File > Open Recent label: the file or folder name first, then its
/// parent shortened (home as `~`, middle folders as `…`), so entries in
/// look-alike folders stay distinguishable where the menu cuts a long row.
pub fn menu_label(path: &Path, home: &Path) -> String {
    let name = match path.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => return path.display().to_string(),
    };
    let Some(parent) = path.parent() else {
        return name;
    };
    let separator = std::path::MAIN_SEPARATOR_STR;
    let parent = match parent.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Ok(rest) => format!("~{separator}{}", rest.display()),
        Err(_) => parent.display().to_string(),
    };
    let parts = parent.split(separator).collect::<Vec<_>>();
    let parent = if parts.len() > 5 {
        [&parts[..2], &["…"], &parts[parts.len() - 2..]]
            .concat()
            .join(separator)
    } else {
        parent
    };
    format!("{name} — {parent}")
}

/// Parse `recent.json`. A missing, unreadable or malformed file is empty, and
/// malformed entries are skipped.
pub fn parse(bytes: &[u8]) -> Vec<RecentEntry> {
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        return Vec::new();
    };
    if value["version"] != 1 {
        return Vec::new();
    }
    value["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let kind = match entry["kind"].as_str()? {
                "project" => RecentKind::Project,
                "pbrt" => RecentKind::Pbrt,
                _ => return None,
            };
            let path = PathBuf::from(entry["path"].as_str()?);
            if !path.is_absolute() {
                return None;
            }
            Some(RecentEntry {
                kind,
                path,
                title: entry["title"].as_str().unwrap_or_default().to_owned(),
                opened_at: entry["opened_at"].as_str().unwrap_or_default().to_owned(),
            })
        })
        .take(MAX_ENTRIES)
        .collect()
}

pub fn to_json(entries: &[RecentEntry]) -> Value {
    json!({
        "version": 1,
        "entries": entries.iter().map(|entry| json!({
            "kind": entry.kind.as_str(),
            "path": entry.path.to_string_lossy(),
            "title": entry.title,
            "opened_at": entry.opened_at,
        })).collect::<Vec<_>>(),
    })
}

/// Put a successful open first. The same normalized path appears once and the
/// list keeps at most [`MAX_ENTRIES`].
pub fn merge(entries: &mut Vec<RecentEntry>, entry: RecentEntry) {
    let path = normalize(&entry.path);
    entries.retain(|existing| normalize(&existing.path) != path);
    entries.insert(0, RecentEntry { path, ..entry });
    entries.truncate(MAX_ENTRIES);
}

/// Forget one entry (the Start screen's Remove).
pub fn remove(entries: &mut Vec<RecentEntry>, path: &Path) {
    let path = normalize(path);
    entries.retain(|existing| normalize(&existing.path) != path);
}

/// Absolute path without `.` / `..` segments. Symlinks are kept as written so
/// a missing path still normalizes.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

pub fn load(path: &Path) -> Vec<RecentEntry> {
    std::fs::read(path)
        .map(|bytes| parse(&bytes))
        .unwrap_or_default()
}

/// Write through a temporary file and rename, like `layout.json`.
pub fn save(path: &Path, entries: &[RecentEntry]) -> std::io::Result<()> {
    let parent = path.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
    std::fs::create_dir_all(parent)?;
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let bytes = serde_json::to_vec_pretty(&to_json(entries)).map_err(std::io::Error::other)?;
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(temporary, path)
}

/// Format a time as RFC 3339 UTC with second precision.
pub fn timestamp(time: SystemTime) -> String {
    let seconds = time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let (days, rest) = (seconds / 86_400, seconds % 86_400);
    let (year, month, day) = civil_from_days(days as i64);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest / 60 % 60,
        rest % 60
    )
}

/// Parse the shape [`timestamp`] writes. Anything else is unknown.
pub fn parse_timestamp(text: &str) -> Option<SystemTime> {
    let bytes = text.as_bytes();
    if bytes.len() != 20 || bytes[4] != b'-' || bytes[10] != b'T' || bytes[19] != b'Z' {
        return None;
    }
    let number = |range: std::ops::Range<usize>| text.get(range)?.parse::<i64>().ok();
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    let days = days_from_civil(year, month, day);
    let seconds = days * 86_400 + hour * 3600 + minute * 60 + second;
    (seconds >= 0).then(|| UNIX_EPOCH + Duration::from_secs(seconds as u64))
}

/// `2h`, `3d`, … for the Start screen. Unknown or future times show nothing.
pub fn age(opened_at: &str, now: SystemTime) -> String {
    let Some(elapsed) = parse_timestamp(opened_at).and_then(|time| now.duration_since(time).ok())
    else {
        return String::new();
    };
    let seconds = elapsed.as_secs();
    match seconds {
        0..60 => "now".into(),
        60..3600 => format!("{}m", seconds / 60),
        3600..86_400 => format!("{}h", seconds / 3600),
        _ => format!("{}d", seconds / 86_400),
    }
}

// Howard Hinnant's civil-date conversions.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let yoe = year.rem_euclid(400);
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: RecentKind, path: &str) -> RecentEntry {
        RecentEntry {
            kind,
            path: PathBuf::from(path),
            title: Path::new(path)
                .file_stem()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            opened_at: "2026-10-25T10:12:00Z".into(),
        }
    }

    #[test]
    fn recent_merge_puts_the_newest_first_and_removes_duplicates() {
        let mut entries = vec![
            entry(RecentKind::Project, "/work/A"),
            entry(RecentKind::Pbrt, "/scenes/b.pbrt"),
        ];
        merge(
            &mut entries,
            entry(RecentKind::Pbrt, "/scenes/./x/../b.pbrt"),
        );
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, PathBuf::from("/scenes/b.pbrt"));
        assert_eq!(entries[1].path, PathBuf::from("/work/A"));
    }

    #[test]
    fn recent_merge_keeps_at_most_twelve_entries() {
        let mut entries = Vec::new();
        for index in 0..20 {
            merge(
                &mut entries,
                entry(RecentKind::Project, &format!("/work/P{index}")),
            );
        }
        assert_eq!(entries.len(), MAX_ENTRIES);
        assert_eq!(entries[0].path, PathBuf::from("/work/P19"));
        assert_eq!(entries[11].path, PathBuf::from("/work/P8"));
    }

    // Unix-only: the fixtures spell absolute paths the Unix way.
    #[cfg(unix)]
    #[test]
    fn recent_merge_treats_a_broken_file_as_empty() {
        assert!(parse(b"{not json").is_empty());
        assert!(parse(br#"{"version":2,"entries":[]}"#).is_empty());
        let mixed = br#"{"version":1,"entries":[
            {"kind":"project","path":"/work/A","title":"A","opened_at":"2026-10-25T10:12:00Z"},
            {"kind":"folder","path":"/work/B"},
            {"kind":"pbrt","path":"relative.pbrt"},
            {"kind":"pbrt","path":"/s/c.pbrt"}
        ]}"#;
        let entries = parse(mixed);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].kind, RecentKind::Pbrt);
        assert_eq!(entries[1].title, "");
    }

    // Unix-only: the fixtures spell absolute paths the Unix way.
    #[cfg(unix)]
    #[test]
    fn recent_round_trips_through_the_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("pbrt-ui/recent.json");
        let entries = vec![
            entry(RecentKind::Project, "/work/A"),
            entry(RecentKind::Pbrt, "/s/c.pbrt"),
        ];
        save(&path, &entries).unwrap();
        assert_eq!(load(&path), entries);
        remove(&mut entries.clone(), Path::new("/work/A"));
        let mut kept = entries.clone();
        remove(&mut kept, Path::new("/work/A"));
        assert_eq!(kept.len(), 1);
    }

    #[test]
    fn timestamps_round_trip_and_show_their_age() {
        let time = UNIX_EPOCH + Duration::from_secs(1_793_000_000);
        let text = timestamp(time);
        assert_eq!(parse_timestamp(&text), Some(time));
        assert_eq!(timestamp(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        assert_eq!(
            parse_timestamp("2026-10-25T10:12:00Z"),
            Some(UNIX_EPOCH + Duration::from_secs(1_792_923_120))
        );
        let now = time + Duration::from_secs(2 * 3600 + 5);
        assert_eq!(age(&text, now), "2h");
        assert_eq!(age(&text, time + Duration::from_secs(3 * 86_400)), "3d");
        assert_eq!(age("yesterday", now), "");
    }
}

#[cfg(all(test, unix))]
mod menu_label_tests {
    use super::menu_label;
    use std::path::Path;

    #[test]
    fn labels_lead_with_the_name_and_shorten_the_parent() {
        let home = Path::new("/home/user");
        assert_eq!(
            menu_label(Path::new("/home/user/scenes/book"), home),
            "book — ~/scenes"
        );
        assert_eq!(menu_label(Path::new("/home/user/book"), home), "book — ~");
        assert_eq!(
            menu_label(Path::new("/home/user/work/qa/2026/round/hl3/P"), home),
            "P — ~/work/…/round/hl3"
        );
        assert_eq!(
            menu_label(Path::new("/mnt/hdd1/src/other/scenes/kitchen.pbrt"), home),
            "kitchen.pbrt — /mnt/…/other/scenes"
        );
    }
}
