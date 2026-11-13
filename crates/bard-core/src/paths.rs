//! Path containment guards. A port of `src/shared/paths.ts`.
//!
//! The bard:// protocol these once guarded is gone -- the audio engine reads
//! files directly -- but "reveal in file manager" still needs [`is_revealable`],
//! and [`is_inside`] is what it is built on.

use std::path::{Component, Path, PathBuf};

/// `path.resolve`: absolute, with `.` and `..` folded lexically.
///
/// Deliberately not `canonicalize`: symlinks are kept as written. Track ids
/// are hashed from this form, so resolving a symlinked music folder would
/// change every id and detach stats and covers from their tracks.
pub fn resolve(p: impl AsRef<Path>) -> PathBuf {
    let p = p.as_ref();
    let joined = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")).join(p)
    };
    // Starts from the path's own root: `/` on Unix, the drive (`C:\`) or
    // share on Windows. `pop` never climbs above it, as `..` at the root of
    // a Unix path stays there.
    let mut out = PathBuf::new();
    for c in joined.components() {
        match c {
            Component::Prefix(_) | Component::RootDir => out.push(c.as_os_str()),
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(s) => out.push(s),
            Component::CurDir => {}
        }
    }
    if out.as_os_str().is_empty() {
        out.push("/");
    }
    out
}

/// True when `target` resolves to something strictly beneath `root`.
///
/// Compared component-wise rather than with a string prefix, so /music-secrets
/// does not read as being inside /music.
pub fn is_inside(root: impl AsRef<Path>, target: impl AsRef<Path>) -> bool {
    let root = resolve(root);
    let target = resolve(target);
    target != root && target.starts_with(&root)
}

/// True when `target` is one of `roots` or sits beneath one.
///
/// The guard on revealing a path, which is a wider hole than it looks: opening
/// a path does not merely highlight a folder, it hands the path to the
/// desktop's default handler -- so a .desktop file under a directory we admit
/// by mistake is executed. Unlike [`is_inside`], the roots themselves pass,
/// because "Open config folder" reveals exactly the data directory.
pub fn is_revealable<P: AsRef<Path>>(roots: &[P], target: impl AsRef<Path>) -> bool {
    let target = target.as_ref();
    if target.as_os_str().is_empty() {
        return false;
    }
    roots.iter().any(|r| resolve(r) == resolve(target) || is_inside(r, target))
}

/// The guard the bard:// handler used: inside some configured root, and
/// nothing at all when no roots are configured.
pub fn is_allowed<P: AsRef<Path>>(roots: &[P], file: impl AsRef<Path>) -> bool {
    let file = file.as_ref();
    !file.as_os_str().is_empty() && roots.iter().any(|r| is_inside(r, file))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "/home/user/Music";

    #[cfg(unix)]
    #[test]
    fn resolve_folds_dots_like_node() {
        assert_eq!(resolve("/a/./b/../c/"), PathBuf::from("/a/c"));
        assert_eq!(resolve("/../x"), PathBuf::from("/x"));
    }

    #[cfg(windows)]
    #[test]
    fn resolve_keeps_the_drive() {
        assert_eq!(resolve(r"D:\Music\.\A\..\B\x.flac"), PathBuf::from(r"D:\Music\B\x.flac"));
        assert_eq!(resolve(r"D:\..\x"), PathBuf::from(r"D:\x"));
        assert_eq!(resolve(r"\\nas\music\a\..\b"), PathBuf::from(r"\\nas\music\b"));
    }

    #[test]
    fn inside_accepts_a_file_beneath() {
        assert!(is_inside(ROOT, "/home/user/Music/Lofi/track.flac"));
    }

    #[test]
    fn inside_accepts_dot_segments() {
        assert!(is_inside(ROOT, "/home/user/Music/./Lofi/track.flac"));
    }

    #[test]
    fn inside_rejects_the_root_itself() {
        assert!(!is_inside(ROOT, "/home/user/Music"));
    }

    #[test]
    fn inside_rejects_a_prefix_sibling() {
        assert!(!is_inside(ROOT, "/home/user/Music-secrets/passwords.txt"));
    }

    #[test]
    fn inside_rejects_traversal() {
        assert!(!is_inside(ROOT, "/home/user/Music/../.ssh/id_rsa"));
        assert!(!is_inside(ROOT, "/home/user/Music/../../etc/passwd"));
    }

    #[test]
    fn inside_rejects_an_unrelated_path() {
        assert!(!is_inside(ROOT, "/etc/passwd"));
    }

    #[test]
    fn allowed_denies_everything_with_no_roots() {
        assert!(!is_allowed::<&str>(&[], "/home/user/Music/a.flac"));
    }

    #[test]
    fn allowed_denies_an_empty_path() {
        assert!(!is_allowed(&[ROOT], ""));
    }

    #[test]
    fn allowed_accepts_a_file_under_any_root() {
        let roots = [ROOT, "/mnt/media"];
        assert!(is_allowed(&roots, "/mnt/media/album/track.flac"));
        assert!(is_allowed(&roots, "/home/user/Music/track.flac"));
        assert!(!is_allowed(&roots, "/mnt/other/track.flac"));
    }

    const ROOTS: [&str; 2] = ["/home/user/.config/bard", "/home/user/Music"];

    #[test]
    fn revealable_accepts_a_root_itself() {
        assert!(is_revealable(&ROOTS, "/home/user/.config/bard"));
    }

    #[test]
    fn revealable_accepts_a_path_beneath_a_root() {
        assert!(is_revealable(&ROOTS, "/home/user/Music/Lofi/track.flac"));
    }

    #[test]
    fn revealable_rejects_a_prefix_sibling() {
        assert!(!is_revealable(&ROOTS, "/home/user/.config/bard-evil/payload.desktop"));
        assert!(!is_revealable(&ROOTS, "/home/user/Music-secrets/passwords.txt"));
    }

    #[test]
    fn revealable_rejects_traversal() {
        assert!(!is_revealable(&ROOTS, "/home/user/Music/../.ssh/id_rsa"));
    }

    #[test]
    fn revealable_rejects_unrelated_and_empty() {
        assert!(!is_revealable(&ROOTS, "/etc/passwd"));
        assert!(!is_revealable(&ROOTS, ""));
    }

    #[test]
    fn revealable_rejects_everything_with_no_roots() {
        assert!(!is_revealable::<&str>(&[], "/home/user/Music/track.flac"));
    }
}
