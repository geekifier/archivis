//! Filesystem type detection for watch mode advisory hints.
//!
//! Detects the filesystem type for a given path and returns a best-effort hint
//! about whether native filesystem events (inotify/FSEvents/ReadDirectoryChanges)
//! are expected to work. This hint is shown in the UI when adding a watched
//! directory — it is *never* used to make automatic decisions.
//!
//! # Container awareness caveat
//!
//! In Docker/Kubernetes, the detected filesystem type may not reflect the actual
//! backing storage. A K8s SMB CSI volume may show as CIFS (detectable) or as
//! FUSE (ambiguous) depending on the CSI driver. A Docker bind mount of an
//! NFS-backed host directory shows as NFS (detectable), but a bind mount of
//! local storage on a SAN appears local (undetectable). The detection result
//! cannot be trusted as ground truth — it is advisory only.

use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Whether native OS filesystem events are likely to work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeSupport {
    /// Local filesystem — native events should work (ext4, btrfs, xfs, APFS, NTFS, etc.)
    Likely,
    /// Network filesystem — native events will NOT detect remote changes (NFS, SMB/CIFS, 9p)
    Unlikely,
    /// Cannot determine — FUSE-based mount, container overlay, or detection unavailable
    Unknown,
}

impl fmt::Display for NativeSupport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Likely => write!(f, "likely"),
            Self::Unlikely => write!(f, "unlikely"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

/// Best-effort detection result for a path's filesystem type.
/// Used as a UI hint to help users choose between native and poll watch modes.
///
/// **IMPORTANT:** This is advisory only. Container environments (Docker, K8s)
/// often obscure the real backing storage. The user must confirm the choice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FsDetectionResult {
    /// Human-readable filesystem type name (e.g., "ext4", "NFS", "CIFS", "FUSE", "unknown").
    pub fs_type: String,
    /// Whether native events are likely to work on this filesystem type.
    pub native_likely_works: NativeSupport,
    /// Explanation shown to the user in the UI.
    pub explanation: String,
}

// ── Linux implementation ─────────────────────────────────────────────

#[cfg(target_os = "linux")]
mod platform {
    use super::{FsDetectionResult, NativeSupport};
    use std::path::Path;

    // Filesystem magic numbers from `statfs(2)` / `<linux/magic.h>`.
    const NFS_MAGIC: i64 = 0x6969;
    const SMB_MAGIC: i64 = 0x517B;
    const CIFS_MAGIC: i64 = 0xFF53_4D42;
    const P9_MAGIC: i64 = 0x0102_1997;
    const FUSE_MAGIC: i64 = 0x6573_5546;
    const OVERLAY_MAGIC: i64 = 0x794C_7630;

    // Common local filesystem magic numbers.
    const EXT4_MAGIC: i64 = 0xEF53;
    const BTRFS_MAGIC: i64 = 0x9123_683E;
    const XFS_MAGIC: i64 = 0x5846_5342;
    const TMPFS_MAGIC: i64 = 0x0102_1994;
    const ZFS_MAGIC: i64 = 0x2FC1_2FC1;

    /// Map a Linux `statfs` filesystem magic number to a detection result.
    ///
    /// This is extracted as a standalone function so it can be unit-tested
    /// with hardcoded magic values without requiring actual mounts.
    pub fn classify_magic(magic: i64) -> FsDetectionResult {
        match magic {
            NFS_MAGIC => FsDetectionResult {
                fs_type: "NFS".to_owned(),
                native_likely_works: NativeSupport::Unlikely,
                explanation: "This path appears to be on an NFS mount. Native events won't \
                    detect changes made by other clients. Polling is recommended."
                    .to_owned(),
            },
            SMB_MAGIC | CIFS_MAGIC => FsDetectionResult {
                fs_type: "CIFS/SMB".to_owned(),
                native_likely_works: NativeSupport::Unlikely,
                explanation: "This path appears to be on an SMB/CIFS mount. Native events \
                    won't detect changes made by other clients. Polling is recommended."
                    .to_owned(),
            },
            P9_MAGIC => FsDetectionResult {
                fs_type: "9p/virtio-fs".to_owned(),
                native_likely_works: NativeSupport::Unlikely,
                explanation: "This path appears to be on a 9p or virtio-fs mount (common \
                    in VMs and some container runtimes). Native events may not work. \
                    Polling is recommended."
                    .to_owned(),
            },
            FUSE_MAGIC => FsDetectionResult {
                fs_type: "FUSE".to_owned(),
                native_likely_works: NativeSupport::Unknown,
                explanation: "This path is on a FUSE filesystem. Some FUSE mounts (like \
                    NTFS-3G or EncFS) support native events, others don't. If unsure, \
                    use polling."
                    .to_owned(),
            },
            OVERLAY_MAGIC => FsDetectionResult {
                fs_type: "OverlayFS".to_owned(),
                native_likely_works: NativeSupport::Unknown,
                explanation: "This path is on an OverlayFS mount (common in containers). \
                    Event support depends on the underlying filesystem. If unsure, \
                    use polling."
                    .to_owned(),
            },
            EXT4_MAGIC => local_result("ext4"),
            BTRFS_MAGIC => local_result("btrfs"),
            XFS_MAGIC => local_result("xfs"),
            TMPFS_MAGIC => local_result("tmpfs"),
            ZFS_MAGIC => local_result("zfs"),
            _ => FsDetectionResult {
                fs_type: "unknown".to_owned(),
                native_likely_works: NativeSupport::Unknown,
                explanation: "Could not determine the filesystem type. If this is a \
                    network or container-mounted volume, use polling."
                    .to_owned(),
            },
        }
    }

    /// Helper: build a `Likely` result for a known local filesystem.
    fn local_result(name: &str) -> FsDetectionResult {
        FsDetectionResult {
            fs_type: name.to_owned(),
            native_likely_works: NativeSupport::Likely,
            explanation: "This path appears to be on a local filesystem. Native events \
                should work."
                .to_owned(),
        }
    }

    /// Detect the filesystem type for the given path using `statfs(2)`.
    ///
    /// Returns a best-effort hint for the UI. The result is advisory only —
    /// container environments often obscure the real backing storage.
    pub fn detect_fs_type(path: &Path) -> FsDetectionResult {
        nix::sys::statfs::statfs(path).map_or_else(
            |_| FsDetectionResult {
                fs_type: "unknown".to_owned(),
                native_likely_works: NativeSupport::Unknown,
                explanation: "Could not determine the filesystem type. If this is a \
                    network or container-mounted volume, use polling."
                    .to_owned(),
            },
            |stat| {
                // nix::sys::statfs::FsType wraps the magic number.
                let magic = stat.filesystem_type().0;
                classify_magic(magic)
            },
        )
    }
}

// ── Non-Linux fallback ───────────────────────────────────────────────

#[cfg(not(target_os = "linux"))]
mod platform {
    use super::{FsDetectionResult, NativeSupport};
    use std::path::Path;

    /// Detect the filesystem type for the given path.
    ///
    /// On macOS and Windows, native OS events (`FSEvents` / `ReadDirectoryChanges`)
    /// generally work for all mounted volumes because the OS provides the
    /// eventing layer — including for SMB mounts. Therefore this always returns
    /// `Likely`.
    pub fn detect_fs_type(path: &Path) -> FsDetectionResult {
        let _ = path; // Used only for API consistency; no OS-level detection needed.
        FsDetectionResult {
            fs_type: "native".to_owned(),
            native_likely_works: NativeSupport::Likely,
            explanation: "macOS/Windows native events generally work for all mounted \
                volumes."
                .to_owned(),
        }
    }
}

/// Detect the filesystem type for the given path.
///
/// Returns a best-effort hint for the UI. On Linux, uses `statfs(2)` to read
/// the filesystem magic number. On other platforms, returns a generic result
/// indicating that native events generally work.
///
/// **IMPORTANT:** This is advisory only. Container environments (Docker, K8s)
/// often obscure the real backing storage. The user must confirm the choice.
pub fn detect_fs_type(path: &Path) -> FsDetectionResult {
    platform::detect_fs_type(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── NativeSupport Display ────────────────────────────────────────

    #[test]
    fn native_support_display_likely() {
        assert_eq!(NativeSupport::Likely.to_string(), "likely");
    }

    #[test]
    fn native_support_display_unlikely() {
        assert_eq!(NativeSupport::Unlikely.to_string(), "unlikely");
    }

    #[test]
    fn native_support_display_unknown() {
        assert_eq!(NativeSupport::Unknown.to_string(), "unknown");
    }

    // ── NativeSupport Debug ──────────────────────────────────────────

    #[test]
    fn native_support_debug() {
        assert_eq!(format!("{:?}", NativeSupport::Likely), "Likely");
        assert_eq!(format!("{:?}", NativeSupport::Unlikely), "Unlikely");
        assert_eq!(format!("{:?}", NativeSupport::Unknown), "Unknown");
    }

    // ── NativeSupport Serde ──────────────────────────────────────────

    #[test]
    fn native_support_serde_roundtrip() {
        for variant in [
            NativeSupport::Likely,
            NativeSupport::Unlikely,
            NativeSupport::Unknown,
        ] {
            let json = serde_json::to_string(&variant).unwrap();
            let deserialized: NativeSupport = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, variant);
        }
    }

    #[test]
    fn native_support_serde_values() {
        assert_eq!(
            serde_json::to_string(&NativeSupport::Likely).unwrap(),
            r#""likely""#
        );
        assert_eq!(
            serde_json::to_string(&NativeSupport::Unlikely).unwrap(),
            r#""unlikely""#
        );
        assert_eq!(
            serde_json::to_string(&NativeSupport::Unknown).unwrap(),
            r#""unknown""#
        );
    }

    // ── FsDetectionResult Serialize ──────────────────────────────────

    #[test]
    fn fs_detection_result_serializes() {
        let result = FsDetectionResult {
            fs_type: "ext4".to_owned(),
            native_likely_works: NativeSupport::Likely,
            explanation: "Local filesystem.".to_owned(),
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains(r#""fs_type":"ext4""#));
        assert!(json.contains(r#""native_likely_works":"likely""#));
        assert!(json.contains(r#""explanation":"Local filesystem.""#));
    }

    // ── detect_fs_type on a temp directory ───────────────────────────

    #[test]
    fn detect_fs_type_on_temp_dir() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let result = detect_fs_type(dir.path());

        // On any platform, a temp dir on local storage should return a valid result.
        assert!(!result.fs_type.is_empty());
        assert!(!result.explanation.is_empty());

        // On macOS/Windows, native events work. On Linux, a local tmpdir
        // should be on a local filesystem (ext4, tmpfs, btrfs, etc.).
        // Either way, it should not be Unlikely.
        assert_ne!(result.native_likely_works, NativeSupport::Unlikely);
    }

    #[test]
    fn detect_fs_type_on_nonexistent_path() {
        let result = detect_fs_type(Path::new("/nonexistent/path/that/does/not/exist"));

        // Should not panic — gracefully returns Unknown or Likely (non-Linux).
        assert!(!result.explanation.is_empty());
    }

    // ── Non-Linux fallback path ──────────────────────────────────────

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn non_linux_returns_likely() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let result = detect_fs_type(dir.path());
        assert_eq!(result.fs_type, "native");
        assert_eq!(result.native_likely_works, NativeSupport::Likely);
        assert!(result.explanation.contains("native events generally work"));
    }

    // ── Linux magic number classification ────────────────────────────
    // These test the mapping function directly with hardcoded values,
    // without requiring actual NFS/SMB/CIFS mounts.

    #[cfg(target_os = "linux")]
    mod linux_tests {
        use super::super::platform::classify_magic;
        use super::*;

        #[test]
        fn nfs_magic() {
            let result = classify_magic(0x6969);
            assert_eq!(result.fs_type, "NFS");
            assert_eq!(result.native_likely_works, NativeSupport::Unlikely);
        }

        #[test]
        fn smb_magic() {
            let result = classify_magic(0x517B);
            assert_eq!(result.fs_type, "CIFS/SMB");
            assert_eq!(result.native_likely_works, NativeSupport::Unlikely);
        }

        #[test]
        fn cifs_magic() {
            let result = classify_magic(0xFF53_4D42);
            assert_eq!(result.fs_type, "CIFS/SMB");
            assert_eq!(result.native_likely_works, NativeSupport::Unlikely);
        }

        #[test]
        fn p9_magic() {
            let result = classify_magic(0x0102_1997);
            assert_eq!(result.fs_type, "9p/virtio-fs");
            assert_eq!(result.native_likely_works, NativeSupport::Unlikely);
        }

        #[test]
        fn fuse_magic() {
            let result = classify_magic(0x6573_5546);
            assert_eq!(result.fs_type, "FUSE");
            assert_eq!(result.native_likely_works, NativeSupport::Unknown);
        }

        #[test]
        fn overlay_magic() {
            let result = classify_magic(0x794C_7630);
            assert_eq!(result.fs_type, "OverlayFS");
            assert_eq!(result.native_likely_works, NativeSupport::Unknown);
        }

        #[test]
        fn ext4_magic() {
            let result = classify_magic(0xEF53);
            assert_eq!(result.fs_type, "ext4");
            assert_eq!(result.native_likely_works, NativeSupport::Likely);
        }

        #[test]
        fn btrfs_magic() {
            let result = classify_magic(0x9123_683E);
            assert_eq!(result.fs_type, "btrfs");
            assert_eq!(result.native_likely_works, NativeSupport::Likely);
        }

        #[test]
        fn xfs_magic() {
            let result = classify_magic(0x5846_5342);
            assert_eq!(result.fs_type, "xfs");
            assert_eq!(result.native_likely_works, NativeSupport::Likely);
        }

        #[test]
        fn tmpfs_magic() {
            let result = classify_magic(0x0102_1994);
            assert_eq!(result.fs_type, "tmpfs");
            assert_eq!(result.native_likely_works, NativeSupport::Likely);
        }

        #[test]
        fn zfs_magic() {
            let result = classify_magic(0x2FC1_2FC1);
            assert_eq!(result.fs_type, "zfs");
            assert_eq!(result.native_likely_works, NativeSupport::Likely);
        }

        #[test]
        fn unknown_magic() {
            let result = classify_magic(0xDEAD_BEEF);
            assert_eq!(result.fs_type, "unknown");
            assert_eq!(result.native_likely_works, NativeSupport::Unknown);
        }
    }
}
