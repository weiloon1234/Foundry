use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use crate::foundation::{Error, Result};

pub(crate) fn clean_manifest_files(
    dir: &Path,
    manifest_name: &str,
    planned_files: &BTreeSet<String>,
    logging_target: &'static str,
    safe_relative_path: impl Fn(&str) -> Option<PathBuf>,
) -> Result<()> {
    let mut files = read_manifest(dir, manifest_name, logging_target);
    files.extend(planned_files.iter().cloned());

    for file in files {
        let Some(relative) = safe_relative_path(&file).and_then(safe_manifest_relative_path) else {
            tracing::warn!(
                target: "foundry.generated_manifest",
                area = logging_target,
                file = %file,
                "skipping unsafe generated manifest path"
            );
            continue;
        };

        let path = checked_generated_path(dir, &relative, FinalSymlinkPolicy::Allow)?;
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(Error::other(error)),
        }
    }

    Ok(())
}

pub(crate) fn write_manifest(
    dir: &Path,
    manifest_name: &str,
    output_files: &BTreeSet<String>,
) -> Result<()> {
    let content = serde_json::to_string_pretty(output_files).map_err(Error::other)?;
    write_generated_file(dir, Path::new(manifest_name), content)
}

pub(crate) fn create_generated_dir_all(root: &Path, relative: &Path) -> Result<()> {
    let path = checked_generated_path(root, relative, FinalSymlinkPolicy::Reject)?;
    std::fs::create_dir_all(path).map_err(Error::other)
}

pub(crate) fn prepare_generated_file_path(root: &Path, relative: &Path) -> Result<PathBuf> {
    validate_generated_relative_path(relative)?;
    if let Some(parent) = relative
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        create_generated_dir_all(root, parent)?;
    }
    checked_generated_path(root, relative, FinalSymlinkPolicy::Reject)
}

pub(crate) fn ensure_generated_file_writable(
    root: &Path,
    relative: &Path,
    force: bool,
) -> Result<()> {
    let path = checked_generated_path(root, relative, FinalSymlinkPolicy::Reject)?;
    match std::fs::symlink_metadata(&path) {
        Ok(_) if !force => Err(Error::message(format!(
            "refusing to overwrite `{}` without `--force`",
            path.display()
        ))),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Error::other(error)),
    }
}

pub(crate) fn write_generated_file(
    root: &Path,
    relative: &Path,
    contents: impl AsRef<[u8]>,
) -> Result<()> {
    let path = prepare_generated_file_path(root, relative)?;
    std::fs::write(path, contents).map_err(Error::other)
}

pub(crate) fn generated_file_exists_without_symlink(root: &Path, relative: &Path) -> bool {
    let Ok(path) = checked_generated_path(root, relative, FinalSymlinkPolicy::Reject) else {
        return false;
    };
    std::fs::symlink_metadata(path)
        .map(|metadata| !metadata.file_type().is_symlink())
        .unwrap_or(false)
}

pub(crate) fn safe_manifest_path_with_extension(
    file: &str,
    extension: &str,
    allow_subdirectories: bool,
) -> Option<PathBuf> {
    if file.contains('\\') || file.chars().any(char::is_control) {
        return None;
    }

    let path = Path::new(file);
    validate_generated_relative_path(path).ok()?;
    if path.extension().and_then(|ext| ext.to_str()) != Some(extension) {
        return None;
    }

    if !allow_subdirectories && path.components().count() != 1 {
        return None;
    }

    Some(path.to_path_buf())
}

fn read_manifest(
    dir: &Path,
    manifest_name: &str,
    logging_target: &'static str,
) -> BTreeSet<String> {
    let path =
        match checked_generated_path(dir, Path::new(manifest_name), FinalSymlinkPolicy::Reject) {
            Ok(path) => path,
            Err(error) => {
                tracing::warn!(
                    target: "foundry.generated_manifest",
                    area = logging_target,
                    manifest = manifest_name,
                    error = %error,
                    "ignoring unsafe generated manifest path"
                );
                return BTreeSet::new();
            }
        };
    let Ok(content) = std::fs::read_to_string(&path) else {
        return BTreeSet::new();
    };

    match serde_json::from_str::<Vec<String>>(&content) {
        Ok(files) => files.into_iter().collect(),
        Err(error) => {
            tracing::warn!(
                target: "foundry.generated_manifest",
                area = logging_target,
                path = %path.display(),
                error = %error,
                "ignoring invalid generated manifest"
            );
            BTreeSet::new()
        }
    }
}

fn safe_manifest_relative_path(path: PathBuf) -> Option<PathBuf> {
    validate_generated_relative_path(&path).ok()?;
    Some(path)
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum FinalSymlinkPolicy {
    Allow,
    Reject,
}

fn checked_generated_path(
    root: &Path,
    relative: &Path,
    final_symlink_policy: FinalSymlinkPolicy,
) -> Result<PathBuf> {
    validate_generated_relative_path(relative)?;

    let components = relative.components().collect::<Vec<_>>();
    let last_component = components.len() - 1;
    let mut path = root.to_path_buf();
    for (index, component) in components.into_iter().enumerate() {
        let Component::Normal(name) = component else {
            unreachable!("relative generated path was already validated");
        };
        path.push(name);

        match std::fs::symlink_metadata(&path) {
            Ok(metadata)
                if metadata.file_type().is_symlink()
                    && (index != last_component
                        || final_symlink_policy == FinalSymlinkPolicy::Reject) =>
            {
                return Err(symlink_output_error(
                    &path,
                    "refusing generated output through symlink component",
                ));
            }
            Ok(metadata) if index != last_component && !metadata.is_dir() => {
                return Err(Error::message(format!(
                    "generated output parent is not a directory: `{}`",
                    path.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(Error::other(error)),
        }
    }

    Ok(path)
}

fn validate_generated_relative_path(relative: &Path) -> Result<()> {
    let mut components = 0usize;
    for component in relative.components() {
        match component {
            Component::Normal(_) => components += 1,
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return Err(Error::message(format!(
                    "generated output path must be relative to its trusted root: `{}`",
                    relative.display()
                )));
            }
        }
    }
    if components == 0 {
        return Err(Error::message("generated output path cannot be empty"));
    }
    Ok(())
}

fn symlink_output_error(path: &Path, message: &str) -> Error {
    Error::message(format!("{message}: `{}`", path.display()))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn ensure_generated_file_writable_refuses_existing_without_force() {
        let dir = tempdir().unwrap();
        let relative = Path::new("generated.txt");
        let path = dir.path().join(relative);
        fs::write(&path, "old").unwrap();

        let error = ensure_generated_file_writable(dir.path(), relative, false).unwrap_err();

        assert!(error.to_string().contains("without `--force`"));
    }

    #[cfg(unix)]
    #[test]
    fn generated_writes_refuse_symlink_targets() {
        use std::os::unix::fs::symlink;

        let dir = tempdir().unwrap();
        let outside = dir.path().join("outside.txt");
        let link = dir.path().join("generated.txt");
        fs::write(&outside, "outside").unwrap();
        symlink(&outside, &link).unwrap();

        let error =
            write_generated_file(dir.path(), Path::new("generated.txt"), "new").unwrap_err();

        assert!(error.to_string().contains("symlink"));
        assert_eq!(fs::read_to_string(&outside).unwrap(), "outside");
    }

    #[cfg(unix)]
    #[test]
    fn generated_operations_refuse_symlinked_descendant_directories() {
        use std::os::unix::fs::symlink;

        let root = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let outside_file = outside.path().join("stale.ts");
        fs::write(&outside_file, "outside").unwrap();
        symlink(outside.path(), root.path().join("routes")).unwrap();

        let write_error =
            write_generated_file(root.path(), Path::new("routes/generated.ts"), "generated")
                .unwrap_err();
        assert!(write_error.to_string().contains("symlink"));
        assert!(!outside.path().join("generated.ts").exists());

        let create_error =
            create_generated_dir_all(root.path(), Path::new("routes/nested")).unwrap_err();
        assert!(create_error.to_string().contains("symlink"));
        assert!(!outside.path().join("nested").exists());

        fs::write(
            root.path().join("manifest.json"),
            serde_json::to_string(&vec!["routes/stale.ts"]).unwrap(),
        )
        .unwrap();
        let clean_error = clean_manifest_files(
            root.path(),
            "manifest.json",
            &BTreeSet::new(),
            "test",
            |file| safe_manifest_path_with_extension(file, "ts", true),
        )
        .unwrap_err();
        assert!(clean_error.to_string().contains("symlink"));
        assert_eq!(fs::read_to_string(outside_file).unwrap(), "outside");
    }

    #[cfg(unix)]
    #[test]
    fn generated_operations_allow_a_symlinked_trusted_root() {
        use std::os::unix::fs::symlink;

        let holder = tempdir().unwrap();
        let target = tempdir().unwrap();
        let root = holder.path().join("selected-output");
        symlink(target.path(), &root).unwrap();

        write_generated_file(&root, Path::new("nested/generated.txt"), "generated").unwrap();

        assert_eq!(
            fs::read_to_string(target.path().join("nested/generated.txt")).unwrap(),
            "generated"
        );
    }
}
