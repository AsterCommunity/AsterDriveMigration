use color_eyre::eyre::{Result, bail};

#[derive(Debug, Clone, PartialEq, Eq)]
enum LocalPathRoot {
    Relative,
    Posix,
    Drive(char),
    Unc(String, String),
}

#[derive(Debug, Clone)]
struct LocalPath {
    root: LocalPathRoot,
    segments: Vec<String>,
}

pub fn normalize_local_storage_path(root: &str, storage_path: &str) -> Result<String> {
    let storage_path = parse_local_path(storage_path)?;
    if storage_path.root == LocalPathRoot::Relative {
        return non_empty_relative_path(storage_path.segments);
    }

    let root = parse_local_path(root)?;
    if !same_local_root(&root.root, &storage_path.root) {
        bail!("absolute storage path uses a different root than the configured local root");
    }
    if root.segments.len() > storage_path.segments.len()
        || !root
            .segments
            .iter()
            .zip(&storage_path.segments)
            .all(|(root, path)| local_segment_eq(&storage_path.root, root, path))
    {
        bail!("absolute storage path is outside the configured local root");
    }
    non_empty_relative_path(storage_path.segments[root.segments.len()..].to_vec())
}

fn parse_local_path(path: &str) -> Result<LocalPath> {
    let normalized = path.replace('\\', "/");
    let (root, remainder) = if let Some(remainder) = normalized.strip_prefix("//") {
        let mut parts = remainder.split('/').filter(|part| !part.is_empty());
        let server = parts
            .next()
            .ok_or_else(|| color_eyre::eyre::eyre!("UNC path is missing a server"))?;
        let share = parts
            .next()
            .ok_or_else(|| color_eyre::eyre::eyre!("UNC path is missing a share"))?;
        (
            LocalPathRoot::Unc(server.to_string(), share.to_string()),
            parts.collect::<Vec<_>>().join("/"),
        )
    } else if let Some(remainder) = normalized.strip_prefix('/') {
        (LocalPathRoot::Posix, remainder.to_string())
    } else if normalized.as_bytes().get(1) == Some(&b':') {
        let drive = normalized
            .chars()
            .next()
            .filter(char::is_ascii_alphabetic)
            .ok_or_else(|| color_eyre::eyre::eyre!("invalid Windows drive prefix"))?;
        let remainder = normalized
            .get(2..)
            .and_then(|value| value.strip_prefix('/'))
            .ok_or_else(|| color_eyre::eyre::eyre!("Windows drive path must be absolute"))?;
        (
            LocalPathRoot::Drive(drive.to_ascii_uppercase()),
            remainder.to_string(),
        )
    } else {
        (LocalPathRoot::Relative, normalized)
    };

    let mut segments = Vec::new();
    for segment in remainder.split('/') {
        match segment {
            "" | "." => {}
            ".." => bail!("storage path must not contain parent-directory segments"),
            value => segments.push(value.to_string()),
        }
    }
    Ok(LocalPath { root, segments })
}

fn same_local_root(left: &LocalPathRoot, right: &LocalPathRoot) -> bool {
    match (left, right) {
        (LocalPathRoot::Posix, LocalPathRoot::Posix) => true,
        (LocalPathRoot::Drive(left), LocalPathRoot::Drive(right)) => left == right,
        (
            LocalPathRoot::Unc(left_server, left_share),
            LocalPathRoot::Unc(right_server, right_share),
        ) => {
            left_server.eq_ignore_ascii_case(right_server)
                && left_share.eq_ignore_ascii_case(right_share)
        }
        _ => false,
    }
}

fn local_segment_eq(root: &LocalPathRoot, left: &str, right: &str) -> bool {
    match root {
        LocalPathRoot::Drive(_) | LocalPathRoot::Unc(_, _) => left.eq_ignore_ascii_case(right),
        LocalPathRoot::Relative | LocalPathRoot::Posix => left == right,
    }
}

fn non_empty_relative_path(segments: Vec<String>) -> Result<String> {
    if segments.is_empty() {
        bail!("storage path resolves to the local storage root");
    }
    Ok(segments.join("/"))
}

#[cfg(test)]
mod tests {
    use super::normalize_local_storage_path;

    #[test]
    fn normalizes_relative_paths_without_using_the_root() {
        for (root, source, expected) in [
            ("/srv/cloudreve", "uploads/object.bin", "uploads/object.bin"),
            (
                "/srv/cloudreve",
                "./uploads//object.bin",
                "uploads/object.bin",
            ),
            (
                "C:/Cloudreve",
                "uploads\\nested\\object.bin",
                "uploads/nested/object.bin",
            ),
            ("ignored", "资料/演示 文稿.pptx", "资料/演示 文稿.pptx"),
        ] {
            assert_eq!(
                normalize_local_storage_path(root, source).expect("relative path"),
                expected
            );
        }
    }

    #[test]
    fn strips_posix_absolute_roots_by_complete_path_segment() {
        assert_eq!(
            normalize_local_storage_path("/srv/./cloudreve/", "/srv/cloudreve/uploads/object.bin")
                .expect("POSIX path"),
            "uploads/object.bin"
        );
        for source in [
            "/srv/cloudreve-archive/object.bin",
            "/srv/other/object.bin",
            "/SRV/cloudreve/object.bin",
        ] {
            assert!(normalize_local_storage_path("/srv/cloudreve", source).is_err());
        }
    }

    #[test]
    fn handles_windows_drive_paths_case_insensitively() {
        assert_eq!(
            normalize_local_storage_path(
                "C:\\Cloudreve\\Data",
                "c:/cloudreve/data/Uploads/Object.bin"
            )
            .expect("Windows drive path"),
            "Uploads/Object.bin"
        );
        assert!(normalize_local_storage_path("C:/cloudreve", "D:/cloudreve/object.bin").is_err());
        assert!(normalize_local_storage_path("C:/cloudreve", "C:object.bin").is_err());
    }

    #[test]
    fn handles_unc_paths_and_rejects_different_shares() {
        assert_eq!(
            normalize_local_storage_path(
                "\\\\server\\share\\Cloudreve",
                "//SERVER/SHARE/cloudreve/uploads/object.bin"
            )
            .expect("UNC path"),
            "uploads/object.bin"
        );
        assert!(
            normalize_local_storage_path(
                "//server/share/cloudreve",
                "//server/other/cloudreve/object.bin"
            )
            .is_err()
        );
        assert!(normalize_local_storage_path("//server", "//server/share/object.bin").is_err());
        assert!(normalize_local_storage_path("//", "//server/share/object.bin").is_err());
    }

    #[test]
    fn rejects_traversal_in_relative_absolute_and_root_paths() {
        for (root, source) in [
            ("/srv/cloudreve", "uploads/../secret.bin"),
            ("/srv/cloudreve", "/srv/cloudreve/uploads/../secret.bin"),
            ("/srv/../cloudreve", "/srv/cloudreve/object.bin"),
        ] {
            assert!(normalize_local_storage_path(root, source).is_err());
        }
    }

    #[test]
    fn rejects_paths_that_resolve_to_no_object_key() {
        for (root, source) in [
            ("/srv/cloudreve", "/srv/cloudreve"),
            ("/", "/"),
            ("ignored", "."),
            ("ignored", ""),
        ] {
            assert!(normalize_local_storage_path(root, source).is_err());
        }
    }

    #[test]
    fn rejects_absolute_paths_when_the_configured_root_is_relative() {
        assert!(
            normalize_local_storage_path("cloudreve-data", "/srv/cloudreve/object.bin").is_err()
        );
    }
}
