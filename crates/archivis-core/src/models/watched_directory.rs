use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A directory monitored by the filesystem watcher for new/changed ebook files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchedDirectory {
    pub id: Uuid,
    pub path: String,
    pub watch_mode: WatchMode,
    pub poll_interval_secs: Option<i64>,
    pub enabled: bool,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// The watch backend to use for a directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatchMode {
    /// Use native OS filesystem events (inotify on Linux, `FSEvents` on macOS,
    /// `ReadDirectoryChanges` on Windows). Lower latency but does NOT work on
    /// network filesystems (NFS, SMB/CIFS, 9p) or many container volume mounts.
    Native,
    /// Use periodic polling. Works everywhere, including network mounts and
    /// container volumes. Higher latency (events detected on next poll cycle).
    Poll,
}

impl fmt::Display for WatchMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Native => write!(f, "Native"),
            Self::Poll => write!(f, "Poll"),
        }
    }
}

impl FromStr for WatchMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "native" => Ok(Self::Native),
            "poll" => Ok(Self::Poll),
            _ => Err(format!("unknown watch mode: {s}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── WatchMode ─────────────────────────────────────────────────

    #[test]
    fn watch_mode_display() {
        assert_eq!(WatchMode::Native.to_string(), "Native");
        assert_eq!(WatchMode::Poll.to_string(), "Poll");
    }

    #[test]
    fn watch_mode_from_str() {
        assert_eq!("native".parse::<WatchMode>().unwrap(), WatchMode::Native);
        assert_eq!("poll".parse::<WatchMode>().unwrap(), WatchMode::Poll);
        assert_eq!("Native".parse::<WatchMode>().unwrap(), WatchMode::Native);
        assert_eq!("POLL".parse::<WatchMode>().unwrap(), WatchMode::Poll);
        assert!("auto".parse::<WatchMode>().is_err());
        assert!("".parse::<WatchMode>().is_err());
    }

    #[test]
    fn watch_mode_serde_roundtrip() {
        let mode = WatchMode::Native;
        let json = serde_json::to_string(&mode).unwrap();
        assert_eq!(json, r#""native""#);
        let deserialized: WatchMode = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, mode);

        let mode = WatchMode::Poll;
        let json = serde_json::to_string(&mode).unwrap();
        assert_eq!(json, r#""poll""#);
        let deserialized: WatchMode = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, mode);
    }
}
