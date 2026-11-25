//! Optional Git provenance, kept separate from the source-content BUILD_ID.
use std::{path::Path, process::Command};

fn git(root: &Path, arguments: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .current_dir(root)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(arguments)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn revision(root: &Path) -> Option<(String, bool)> {
    // An unpacked source archive may sit inside an unrelated parent repository.
    let repository = git(root, &["rev-parse", "--show-toplevel"])
        .and_then(|path| std::fs::canonicalize(path).ok());
    if repository.is_none() || repository != std::fs::canonicalize(root).ok() {
        return None;
    }
    let commit = git(root, &["rev-parse", "--short=7", "HEAD"])?;
    // Include untracked source files, excluding ignored build output.
    let status = git(
        root,
        &["status", "--porcelain=v1", "--untracked-files=normal"],
    )?;
    Some((commit, !status.is_empty()))
}

pub fn generate(root: &Path) {
    let information = revision(root);
    if information.is_some() {
        // Handle ordinary repositories, linked worktrees and packed refs.
        let mut paths = vec![
            "HEAD".to_owned(),
            "index".to_owned(),
            "packed-refs".to_owned(),
        ];
        if let Some(reference) = git(root, &["symbolic-ref", "-q", "HEAD"]) {
            paths.push(reference);
        }
        for name in paths {
            if let Some(path) = git(root, &["rev-parse", "--git-path", &name]) {
                let path = root.join(path);
                if path.exists() {
                    println!("cargo:rerun-if-changed={}", path.display());
                }
            }
        }
        // Track edits outside firmware input directories as well (e.g. README).
        if let Some(files) = git(
            root,
            &[
                "ls-files",
                "-z",
                "--cached",
                "--others",
                "--exclude-standard",
            ],
        ) {
            for file in files.split('\0').filter(|file| !file.is_empty()) {
                println!("cargo:rerun-if-changed={}", root.join(file).display());
            }
        }
    }
    let (commit, modified) = information.unwrap_or_else(|| ("unknown".to_owned(), false));
    println!("cargo:rustc-env=EKD01_GIT_COMMIT={commit}");
    println!("cargo:rustc-env=EKD01_GIT_MODIFIED={modified}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_dirty_state_and_archive_fallback() {
        let root = std::env::temp_dir().join(format!("ekd01-revision-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        assert_eq!(revision(&root), None);
        git(&root, &["init", "-q"]).unwrap();
        std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
        std::fs::write(root.join("source.rs"), "original\n").unwrap();
        git(&root, &["add", "."]).unwrap();
        git(
            &root,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-qm",
                "Fixture",
            ],
        )
        .unwrap();
        let (commit, dirty) = revision(&root).unwrap();
        assert!(commit.len() >= 7 && commit.bytes().all(|c| c.is_ascii_hexdigit()));
        assert!(!dirty);
        std::fs::create_dir(root.join("target")).unwrap();
        std::fs::write(root.join("target/output"), "ignored").unwrap();
        assert_eq!(revision(&root), Some((commit.clone(), false)));
        std::fs::write(root.join("source.rs"), "changed\n").unwrap();
        assert_eq!(revision(&root), Some((commit.clone(), true)));
        git(&root, &["checkout", "--", "source.rs"]).unwrap();
        std::fs::write(root.join("new.rs"), "untracked\n").unwrap();
        assert_eq!(revision(&root), Some((commit.clone(), true)));
        std::fs::remove_file(root.join("new.rs")).unwrap();
        git(&root, &["checkout", "--detach", "-q"]).unwrap();
        assert_eq!(revision(&root), Some((commit, false)));
        // A source archive nested in a different repository must not inherit its ID.
        std::fs::create_dir(root.join("archive")).unwrap();
        assert_eq!(revision(&root.join("archive")), None);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
