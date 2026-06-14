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

        match std::fs::remove_file(dir.join(relative)) {
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
    let files: Vec<&str> = output_files.iter().map(String::as_str).collect();
    let content = serde_json::to_string_pretty(&files).map_err(Error::other)?;
    write_generated_file(&dir.join(manifest_name), content)
}

pub(crate) fn ensure_generated_file_writable(path: &Path, force: bool) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(symlink_write_error(
            path,
            "refusing to write generated file through symlink",
        )),
        Ok(_) if !force => Err(Error::message(format!(
            "refusing to overwrite `{}` without `--force`",
            path.display()
        ))),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Error::other(error)),
    }
}

pub(crate) fn write_generated_file(path: &Path, contents: impl AsRef<[u8]>) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(symlink_write_error(
                path,
                "refusing to write generated file through symlink",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(Error::other(error)),
    }

    std::fs::write(path, contents).map_err(Error::other)
}

pub(crate) fn generated_file_exists_without_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|metadata| !metadata.file_type().is_symlink())
        .unwrap_or(false)
}

pub(crate) fn safe_manifest_path_with_extension(
    file: &str,
    extension: &str,
    allow_subdirectories: bool,
) -> Option<PathBuf> {
    if file.is_empty() || file.contains('\\') || file.chars().any(char::is_control) {
        return None;
    }

    let path = Path::new(file);
    if path.is_absolute() {
        return None;
    }
    if path.extension().and_then(|ext| ext.to_str()) != Some(extension) {
        return None;
    }

    let mut normal_components = 0usize;
    for component in path.components() {
        match component {
            Component::Normal(_) => normal_components += 1,
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => return None,
        }
    }

    if normal_components == 0 || (!allow_subdirectories && normal_components != 1) {
        return None;
    }

    Some(path.to_path_buf())
}

fn read_manifest(
    dir: &Path,
    manifest_name: &str,
    logging_target: &'static str,
) -> BTreeSet<String> {
    let path = dir.join(manifest_name);
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
    if path.is_absolute() {
        return None;
    }

    for component in path.components() {
        match component {
            Component::Normal(_) => {}
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => return None,
        }
    }

    Some(path)
}

fn symlink_write_error(path: &Path, message: &str) -> Error {
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
        let path = dir.path().join("generated.txt");
        fs::write(&path, "old").unwrap();

        let error = ensure_generated_file_writable(&path, false).unwrap_err();

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

        let error = write_generated_file(&link, "new").unwrap_err();

        assert!(error.to_string().contains("symlink"));
        assert_eq!(fs::read_to_string(&outside).unwrap(), "outside");
    }
}
