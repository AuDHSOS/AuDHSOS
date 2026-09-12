// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::fs`.

use crate::fs::{create_sparse, extension, file_name, walk_files};
use std::path::Path;

#[test]
fn walk_finds_this_file_and_skips_target() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let files = walk_files(root).unwrap();
    assert!(files.iter().any(|f| f.ends_with("src/fs.rs")));
    assert!(
        files
            .iter()
            .all(|f| !f.components().any(|c| c.as_os_str() == "target"))
    );
    let mut sorted = files.clone();
    sorted.sort();
    assert_eq!(files, sorted);
}

#[test]
fn extension_and_file_name_handle_missing_parts() {
    assert_eq!(extension(Path::new("a/b.rs")), "rs");
    assert_eq!(extension(Path::new("LICENSE")), "");
    assert_eq!(file_name(Path::new("a/Cargo.lock")), "Cargo.lock");
    assert_eq!(file_name(Path::new("/")), "");
}

#[test]
fn walking_a_missing_directory_is_an_error() {
    assert!(walk_files(Path::new("/definitely/missing/dir")).is_err());
}

#[test]
fn the_walk_skips_the_directories_the_policy_excludes() {
    use crate::policy::EXCLUDED_DIRECTORIES;
    let root = std::env::temp_dir().join("audhsos-xtask-excluded");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("crates")).expect("a directory");
    std::fs::write(root.join("crates/kept.rs"), b"").expect("a file");
    for excluded in EXCLUDED_DIRECTORIES {
        std::fs::create_dir_all(root.join(excluded)).expect("a directory");
        std::fs::write(root.join(excluded).join("skipped.rs"), b"").expect("a file");
    }
    let files = walk_files(&root).expect("the walk");
    assert_eq!(files.len(), 1, "{files:?}");
    assert!(
        files.first().is_some_and(|path| path.ends_with("kept.rs")),
        "{files:?}"
    );
    assert!(
        EXCLUDED_DIRECTORIES.contains(&"research"),
        "reference material of other projects stays out of the checks"
    );
    assert!(
        EXCLUDED_DIRECTORIES.contains(&".claude"),
        "a worktree under `.claude/worktrees/` is a checkout of its own"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_scratch_disk_is_created_once_and_kept_afterwards() {
    let dir = std::env::temp_dir().join(format!("audhsos-xtask-sparse-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("qemu").join("run.scratch.img");
    assert!(create_sparse(&path, 4096).expect("the disk"));
    assert_eq!(std::fs::metadata(&path).expect("the disk").len(), 4096);
    std::fs::write(&path, b"what a run wrote").expect("a write");
    assert!(!create_sparse(&path, 4096).expect("the disk"));
    assert_eq!(
        std::fs::read(&path).expect("the disk"),
        b"what a run wrote",
        "a second run took the disk as it stood"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_scratch_disk_whose_directory_cannot_be_made_is_an_error() {
    // A path under the root would be an error for an ordinary user and a
    // directory tree at `/` for a privileged one, so the thing in the way
    // is a file: no user may make a directory below one.
    let dir = std::env::temp_dir().join(format!("audhsos-xtask-blocked-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a directory");
    let blocker = dir.join("in-the-way");
    std::fs::write(&blocker, b"not a directory").expect("a file");
    assert!(create_sparse(&blocker.join("disk.img"), 4096).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
