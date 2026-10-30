//! Extraction tests. Archives are built in code, so hostile ones (zip-slip, symlink tricks, bombs) can be
//! tested. Every test runs inside a sandbox whose folder names have a space and non-ASCII characters
//! (NFR-07), and checks that nothing was written outside the destination.

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod archive_builders;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use archive_builders::{
    ALL_FORMATS, Entry, TAR_FORMATS, compressible, gzip, incompressible, tar, tar_gz, tar_xz, xz,
    zip,
};
use luna_core::ErrorCode;
use luna_core::extract::{
    ArchiveFormat, ExtractOptions, ExtractProgress, ExtractSummary, extract, extract_sync,
};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

/// `<tmp>/lüna home/archives` holds the archives, `<tmp>/lüna home/sandbox/stagïng dir` is the
/// destination. Anything that appears next to the destination (or elsewhere) is an escape.
struct Sandbox {
    _tmp: TempDir,
    root: PathBuf,
    sandbox: PathBuf,
    dest: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("lüna home");
        let sandbox = root.join("sandbox");
        fs::create_dir_all(root.join("archives")).unwrap();
        fs::create_dir_all(&sandbox).unwrap();
        let dest = sandbox.join("stagïng dir");
        Self {
            _tmp: tmp,
            root,
            sandbox,
            dest,
        }
    }

    fn write_archive(&self, file_name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.root.join("archives").join(file_name);
        fs::write(&path, bytes).unwrap();
        path
    }

    fn extract_with(
        &self,
        file_name: &str,
        bytes: &[u8],
        options: &ExtractOptions,
    ) -> luna_core::Result<ExtractSummary> {
        let path = self.write_archive(file_name, bytes);
        let format = ArchiveFormat::from_file_name(file_name).unwrap();
        extract_sync(
            &path,
            &self.dest,
            format,
            options,
            &CancellationToken::new(),
            &mut |_| {},
        )
    }

    fn extract(&self, file_name: &str, bytes: &[u8]) -> luna_core::Result<ExtractSummary> {
        self.extract_with(file_name, bytes, &ExtractOptions::default())
    }

    /// Nothing but the destination may exist beside it, and nothing but `archives` and `sandbox` in the root.
    fn assert_nothing_escaped(&self) {
        assert_eq!(
            names(&self.sandbox),
            vec!["stagïng dir"],
            "something was written next to the destination"
        );
        assert_eq!(
            names(&self.root),
            vec!["archives", "sandbox"],
            "something was written above the sandbox"
        );
    }

    fn read(&self, relative: &str) -> Vec<u8> {
        fs::read(self.dest.join(relative)).unwrap()
    }
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn code(result: luna_core::Result<ExtractSummary>) -> ErrorCode {
    result.expect_err("extraction should have failed").code
}

/// Everything below `dir` as sorted relative paths, for comparing whole trees.
fn tree(dir: &Path) -> Vec<String> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let relative = path
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let is_dir = fs::symlink_metadata(&path).unwrap().is_dir();
            out.push(if is_dir {
                format!("{relative}/")
            } else {
                relative
            });
            if is_dir {
                walk(base, &path, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

// ---------------------------------------------------------------------------------------------
// Real archives from reference implementations (Python zipfile, tarfile, liblzma)
// ---------------------------------------------------------------------------------------------

fn fixture(name: &str) -> Vec<u8> {
    fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/archives")
            .join(name),
    )
    .unwrap()
}

#[test]
fn archives_made_by_reference_tools_extract_to_the_same_tree() {
    let data_bin: Vec<u8> = (0..3000u32).map(|i| ((i * 7 + 3) % 256) as u8).collect();
    // The tar fixtures without a symlink: those extract on every OS (see the Unix-only test below).
    for name in [
        "official.zip",
        "official-nolink.tar.gz",
        "official-nolink.tar.xz",
    ] {
        let sandbox = Sandbox::new();
        let options = ExtractOptions {
            strip_components: 1,
            ..ExtractOptions::default()
        };
        let summary = sandbox
            .extract_with(name, &fixture(name), &options)
            .unwrap();

        assert_eq!(sandbox.read("README.txt"), b"hello\n", "{name}");
        assert_eq!(sandbox.read("lib/data.bin"), data_bin, "{name}");
        assert_eq!(
            sandbox.read("ünï/файл.txt"),
            "unicode name\n".as_bytes(),
            "{name}"
        );
        assert_eq!(
            sandbox.read("bin/tool"),
            b"#!/bin/sh\necho tool\n",
            "{name}"
        );
        assert!(
            sandbox.dest.join("empty").is_dir(),
            "{name}: empty directory is kept"
        );
        assert_eq!(summary.files, 4, "{name}");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(sandbox.dest.join("bin/tool"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(
                mode & 0o777,
                0o755,
                "{name}: the executable bit must survive"
            );
            let readme = fs::metadata(sandbox.dest.join("README.txt"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(
                readme & 0o111,
                0,
                "{name}: ordinary files are not executable"
            );
        }
        sandbox.assert_nothing_escaped();
    }
}

#[cfg(unix)]
#[test]
fn symlinks_from_reference_tar_archives_are_recreated() {
    for name in ["official.tar.gz", "official.tar.xz"] {
        let sandbox = Sandbox::new();
        let options = ExtractOptions {
            strip_components: 1,
            ..ExtractOptions::default()
        };
        let summary = sandbox
            .extract_with(name, &fixture(name), &options)
            .unwrap();
        assert_eq!(summary.symlinks, 1, "{name}");
        assert_eq!(
            fs::read_link(sandbox.dest.join("bin/tool-link")).unwrap(),
            Path::new("tool")
        );
        assert_eq!(sandbox.read("bin/tool-link"), b"#!/bin/sh\necho tool\n");
    }
}

#[test]
fn all_formats_produce_identical_trees_from_identical_entries() {
    let entries = || {
        vec![
            Entry::dir("app/"),
            Entry::file("app/bin/php", b"binary".to_vec()).mode(0o755),
            Entry::file("app/lib/a.txt", b"a"),
            Entry::file("app/ünï/файл.txt", b"unicode"),
            Entry::dir("app/empty/"),
        ]
    };
    let mut trees = Vec::new();
    for (ext, build) in ALL_FORMATS {
        let sandbox = Sandbox::new();
        let options = ExtractOptions {
            strip_components: 1,
            ..ExtractOptions::default()
        };
        sandbox
            .extract_with(&format!("x.{ext}"), &build(&entries()), &options)
            .unwrap();
        assert_eq!(sandbox.read("bin/php"), b"binary", "{ext}");
        trees.push((ext, tree(&sandbox.dest)));
    }
    assert_eq!(trees[0].1, trees[1].1, "{} vs {}", trees[0].0, trees[1].0);
    assert_eq!(trees[1].1, trees[2].1, "{} vs {}", trees[1].0, trees[2].0);
}

#[test]
fn strip_components_drops_the_wrapper_and_skips_entries_without_a_remainder() {
    let entries = vec![
        Entry::dir("wrapper/"),
        Entry::file("wrapper/a.txt", b"a"),
        Entry::file("wrapper/sub/b.txt", b"b"),
        Entry::file("loose.txt", b"loose"),
    ];
    for (ext, build) in ALL_FORMATS {
        let sandbox = Sandbox::new();
        let options = ExtractOptions {
            strip_components: 1,
            ..ExtractOptions::default()
        };
        sandbox
            .extract_with(&format!("x.{ext}"), &build(&entries), &options)
            .unwrap();
        assert_eq!(
            tree(&sandbox.dest),
            vec!["a.txt", "sub/", "sub/b.txt"],
            "{ext}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Zip-slip and path attacks
// ---------------------------------------------------------------------------------------------

const HOSTILE_NAMES: [&str; 7] = [
    "../evil.txt",
    "a/../../evil.txt",
    "../../../../../../evil.txt",
    "/abs-evil.txt",
    "..\\evil.txt",
    "C:\\evil.txt",
    "\\\\server\\share\\evil.txt",
];

#[test]
fn hostile_entry_names_are_rejected_in_every_format_and_nothing_escapes() {
    for (ext, build) in ALL_FORMATS {
        for hostile in HOSTILE_NAMES {
            let sandbox = Sandbox::new();
            let archive = build(&[
                Entry::file("ok.txt", b"fine"),
                Entry::file(hostile, b"pwned"),
            ]);
            let error = sandbox
                .extract(&format!("x.{ext}"), &archive)
                .expect_err(&format!("{ext} {hostile:?}"));
            assert_eq!(
                error.code,
                ErrorCode::ArchiveUnsafe,
                "{ext} {hostile:?}: {}",
                error.message
            );
            sandbox.assert_nothing_escaped();
        }
    }
}

#[test]
fn a_hostile_name_is_rejected_even_when_stripping_would_hide_it() {
    for (ext, build) in ALL_FORMATS {
        let sandbox = Sandbox::new();
        let archive = build(&[Entry::file("../../evil.txt", b"pwned")]);
        let options = ExtractOptions {
            strip_components: 2,
            ..ExtractOptions::default()
        };
        let error = sandbox
            .extract_with(&format!("x.{ext}"), &archive, &options)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::ArchiveUnsafe, "{ext}");
        sandbox.assert_nothing_escaped();
    }
}

#[cfg(unix)]
#[test]
fn symlinks_pointing_outside_are_rejected() {
    let escapes = [
        ("link", "/etc"),
        ("link", "../.."),
        ("link", ".."),
        ("a/link", "../../x"),
        ("link", "/"),
    ];
    for (ext, build) in ALL_FORMATS {
        for (link, target) in escapes {
            let sandbox = Sandbox::new();
            let archive = build(&[Entry::dir("a/"), Entry::symlink(link, target)]);
            let error = sandbox
                .extract(&format!("x.{ext}"), &archive)
                .expect_err(&format!("{ext} {link}->{target}"));
            assert_eq!(
                error.code,
                ErrorCode::ArchiveUnsafe,
                "{ext} {link}->{target}"
            );
            sandbox.assert_nothing_escaped();
            assert!(
                fs::symlink_metadata(sandbox.dest.join(link)).is_err(),
                "{ext}: the link must not exist"
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn symlinks_staying_inside_are_created() {
    for (ext, build) in ALL_FORMATS {
        let sandbox = Sandbox::new();
        let archive = build(&[
            Entry::file("sub/real.txt", b"data"),
            Entry::symlink("sub/alias", "real.txt"),
            Entry::symlink("up", "sub/real.txt"),
            Entry::dir("bin/"),
            Entry::symlink("bin/tool", "../sub/real.txt"),
        ]);
        let summary = sandbox.extract(&format!("x.{ext}"), &archive).unwrap();
        assert_eq!(summary.symlinks, 3, "{ext}");
        assert_eq!(
            fs::read_link(sandbox.dest.join("bin/tool")).unwrap(),
            Path::new("../sub/real.txt")
        );
        assert_eq!(sandbox.read("up"), b"data", "{ext}");
    }
}

#[cfg(windows)]
#[test]
fn symlinks_are_refused_on_windows() {
    for (ext, build) in ALL_FORMATS {
        let sandbox = Sandbox::new();
        let archive = build(&[
            Entry::file("real.txt", b"x"),
            Entry::symlink("alias", "real.txt"),
        ]);
        assert_eq!(
            code(sandbox.extract(&format!("x.{ext}"), &archive)),
            ErrorCode::ArchiveUnsafe,
            "{ext}"
        );
        sandbox.assert_nothing_escaped();
    }
}

/// The subtle attack: a link that points *inside* the destination, followed by a file written
/// "through" it. Each link target below is allowed on its own, so only the write is the problem.
#[cfg(unix)]
#[test]
fn nothing_is_ever_written_through_a_symlink() {
    let attacks: Vec<(&str, Vec<Entry>)> = vec![
        (
            "link to the root",
            vec![Entry::symlink("d", "."), Entry::file("d/escaped.txt", b"x")],
        ),
        (
            "link to a subfolder",
            vec![
                Entry::dir("sub/"),
                Entry::symlink("d", "sub"),
                Entry::file("d/x.txt", b"x"),
            ],
        ),
        (
            "different letter case",
            vec![
                Entry::dir("sub/"),
                Entry::symlink("Link", "sub"),
                Entry::file("link/x.txt", b"x"),
            ],
        ),
        (
            "directory entry through the link",
            vec![
                Entry::dir("sub/"),
                Entry::symlink("d", "sub"),
                Entry::dir("d/deeper/"),
            ],
        ),
        (
            "second link through the first",
            vec![
                Entry::dir("sub/"),
                Entry::symlink("d", "sub"),
                Entry::symlink("d/up", "../sub"),
            ],
        ),
    ];
    for (ext, build) in TAR_FORMATS
        .into_iter()
        .chain([("zip", archive_builders::zip as archive_builders::Builder)])
    {
        for (label, entries) in &attacks {
            let sandbox = Sandbox::new();
            let error = sandbox
                .extract(&format!("x.{ext}"), &build(entries))
                .expect_err(&format!("{ext}: {label}"));
            assert_eq!(
                error.code,
                ErrorCode::ArchiveUnsafe,
                "{ext}: {label}: {}",
                error.message
            );
            assert!(
                !sandbox.dest.join("sub/x.txt").exists(),
                "{ext}: {label}: wrote through the link"
            );
            assert!(
                !sandbox.dest.join("escaped.txt").exists(),
                "{ext}: {label}: wrote through the link"
            );
            assert!(
                !sandbox.dest.join("sub/deeper").exists(),
                "{ext}: {label}: wrote through the link"
            );
            sandbox.assert_nothing_escaped();
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Hard links, special files, duplicates
// ---------------------------------------------------------------------------------------------

#[test]
fn hard_links_to_earlier_regular_files_work_including_after_stripping() {
    for (ext, build) in TAR_FORMATS {
        let sandbox = Sandbox::new();
        let archive = build(&[
            Entry::dir("w/"),
            Entry::file("w/a.txt", b"shared"),
            Entry::hardlink("w/b.txt", "w/a.txt"),
        ]);
        let options = ExtractOptions {
            strip_components: 1,
            ..ExtractOptions::default()
        };
        let summary = sandbox
            .extract_with(&format!("x.{ext}"), &archive, &options)
            .unwrap();
        assert_eq!(summary.hard_links, 1, "{ext}");
        assert_eq!(sandbox.read("b.txt"), b"shared", "{ext}");
    }
}

#[test]
fn hard_links_that_could_reach_outside_or_are_not_plain_earlier_files_are_rejected() {
    let cases: Vec<(&str, Vec<Entry>)> = vec![
        (
            "outside the destination",
            vec![Entry::hardlink("evil", "../outside")],
        ),
        (
            "absolute target",
            vec![Entry::hardlink("evil", "/etc/passwd")],
        ),
        (
            "target not extracted yet",
            vec![
                Entry::hardlink("early", "later.txt"),
                Entry::file("later.txt", b"x"),
            ],
        ),
        (
            "target missing",
            vec![Entry::hardlink("evil", "nothing.txt")],
        ),
        (
            "target is a directory",
            vec![Entry::dir("d/"), Entry::hardlink("evil", "d")],
        ),
    ];
    for (ext, build) in TAR_FORMATS {
        for (label, entries) in &cases {
            let sandbox = Sandbox::new();
            let error = sandbox
                .extract(&format!("x.{ext}"), &build(entries))
                .expect_err(&format!("{ext}: {label}"));
            assert_eq!(
                error.code,
                ErrorCode::ArchiveUnsafe,
                "{ext}: {label}: {}",
                error.message
            );
            sandbox.assert_nothing_escaped();
        }
    }
}

#[test]
fn special_files_are_rejected() {
    for (ext, build) in TAR_FORMATS {
        let sandbox = Sandbox::new();
        let archive = build(&[Entry::file("ok.txt", b"x"), Entry::fifo("pipe")]);
        assert_eq!(
            code(sandbox.extract(&format!("x.{ext}"), &archive)),
            ErrorCode::ArchiveUnsafe,
            "{ext}"
        );
        assert!(!sandbox.dest.join("pipe").exists());
    }
}

#[test]
fn duplicate_names_and_files_under_files_are_rejected() {
    // The zip library refuses to write duplicate names, so duplicates can only be built as tar.
    for (ext, build) in TAR_FORMATS {
        let sandbox = Sandbox::new();
        let duplicate = build(&[
            Entry::file("a.txt", b"first"),
            Entry::file("a.txt", b"second"),
        ]);
        assert_eq!(
            code(sandbox.extract(&format!("x.{ext}"), &duplicate)),
            ErrorCode::ArchiveUnsafe,
            "{ext} duplicate"
        );
        assert_eq!(
            sandbox.read("a.txt"),
            b"first",
            "{ext}: the first file is never overwritten"
        );
    }
    for (ext, build) in ALL_FORMATS {
        let sandbox = Sandbox::new();
        let under_file = build(&[Entry::file("a", b"file"), Entry::file("a/b.txt", b"nested")]);
        assert_eq!(
            code(sandbox.extract(&format!("x.{ext}"), &under_file)),
            ErrorCode::ArchiveUnsafe,
            "{ext} under file"
        );
        sandbox.assert_nothing_escaped();
    }
}

#[cfg(unix)]
#[test]
fn permission_bits_beyond_rwx_are_dropped() {
    use std::os::unix::fs::PermissionsExt;
    let archive = tar_gz(&[
        Entry::file("setuid", b"x").mode(0o4755),
        Entry::file("setgid", b"x").mode(0o2755),
        Entry::file("unreadable", b"x").mode(0o000),
        Entry::dir("sticky/").mode(0o1777),
    ]);
    let sandbox = Sandbox::new();
    sandbox.extract("x.tar.gz", &archive).unwrap();
    let mode = |name: &str| {
        fs::metadata(sandbox.dest.join(name))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777
    };
    assert_eq!(mode("setuid"), 0o755);
    assert_eq!(mode("setgid"), 0o755);
    assert_eq!(mode("unreadable") & 0o7000, 0);
    assert_eq!(
        mode("unreadable") & 0o600,
        0o600,
        "the owner can always use what we extracted"
    );
    assert_eq!(mode("sticky") & 0o7000, 0);
}

// ---------------------------------------------------------------------------------------------
// Limits, damaged archives, destination rules
// ---------------------------------------------------------------------------------------------

fn dir_size(dir: &Path) -> u64 {
    tree(dir)
        .iter()
        .filter(|p| !p.ends_with('/'))
        .map(|p| fs::metadata(dir.join(p)).unwrap().len())
        .sum()
}

#[test]
fn decompression_bombs_are_stopped_by_the_total_size_limit() {
    let bomb = [Entry::file("zeros.bin", compressible(40 * 1024 * 1024))];
    for (ext, build) in ALL_FORMATS {
        let sandbox = Sandbox::new();
        let archive = build(&bomb);
        assert!(
            archive.len() < 400 * 1024,
            "{ext}: the test archive really is a small bomb ({} bytes)",
            archive.len()
        );
        let options = ExtractOptions {
            max_total_bytes: 4 * 1024 * 1024,
            ..ExtractOptions::default()
        };
        let error = sandbox
            .extract_with(&format!("x.{ext}"), &archive, &options)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::ArchiveTooLarge, "{ext}");
        assert!(
            dir_size(&sandbox.dest) <= 4 * 1024 * 1024 + 128 * 1024,
            "{ext}: it stopped writing soon after the limit"
        );
    }
}

#[test]
fn the_per_file_limit_and_the_entry_count_limit_apply() {
    for (ext, build) in ALL_FORMATS {
        let sandbox = Sandbox::new();
        let big = build(&[Entry::file("big.bin", compressible(3 * 1024 * 1024))]);
        let options = ExtractOptions {
            max_entry_bytes: 1024 * 1024,
            ..ExtractOptions::default()
        };
        assert_eq!(
            code(sandbox.extract_with(&format!("x.{ext}"), &big, &options)),
            ErrorCode::ArchiveTooLarge,
            "{ext} size"
        );

        let sandbox = Sandbox::new();
        let many: Vec<Entry> = (0..6)
            .map(|i| Entry::file(&format!("f{i}.txt"), b"x"))
            .collect();
        let options = ExtractOptions {
            max_entries: 5,
            ..ExtractOptions::default()
        };
        assert_eq!(
            code(sandbox.extract_with(&format!("x.{ext}"), &build(&many), &options)),
            ErrorCode::ArchiveTooLarge,
            "{ext} count"
        );
    }
}

#[test]
fn damaged_truncated_empty_and_mismatched_archives_fail_cleanly() {
    let entries = [
        Entry::file("pkg/big.bin", incompressible(300_000)),
        Entry::file("pkg/b.txt", b"b"),
    ];
    let valid: Vec<(&str, Vec<u8>)> = vec![
        ("zip", zip(&entries)),
        ("tar.gz", tar_gz(&entries)),
        ("tar.xz", tar_xz(&entries)),
    ];

    for (ext, bytes) in &valid {
        let sandbox = Sandbox::new();
        let truncated = &bytes[..bytes.len() / 2];
        assert_eq!(
            code(sandbox.extract(&format!("t.{ext}"), truncated)),
            ErrorCode::ArchiveCorrupt,
            "{ext} truncated"
        );

        let sandbox = Sandbox::new();
        let garbage = incompressible(4096);
        assert_eq!(
            code(sandbox.extract(&format!("g.{ext}"), &garbage)),
            ErrorCode::ArchiveCorrupt,
            "{ext} garbage"
        );

        let sandbox = Sandbox::new();
        assert_eq!(
            code(sandbox.extract(&format!("e.{ext}"), b"")),
            ErrorCode::ArchiveCorrupt,
            "{ext} empty file"
        );

        // Bytes of one format handed to the decoder of another.
        for (other_ext, other_bytes) in &valid {
            if other_ext != ext {
                let sandbox = Sandbox::new();
                let name = format!("m.{ext}");
                assert_eq!(
                    code(sandbox.extract(&name, other_bytes)),
                    ErrorCode::ArchiveCorrupt,
                    "{other_ext} data read as {ext}"
                );
            }
        }
    }

    // A valid, empty archive is not a runtime either.
    let sandbox = Sandbox::new();
    assert_eq!(
        code(sandbox.extract("empty.tar.gz", &gzip(&tar(&[])))),
        ErrorCode::ArchiveCorrupt
    );
    let sandbox = Sandbox::new();
    assert_eq!(
        code(sandbox.extract("empty.tar.xz", &xz(&tar(&[])))),
        ErrorCode::ArchiveCorrupt
    );
}

#[test]
fn the_destination_must_be_empty_or_missing() {
    let archive = tar_gz(&[Entry::file("a.txt", b"a")]);

    let sandbox = Sandbox::new();
    fs::create_dir_all(&sandbox.dest).unwrap();
    sandbox.extract("x.tar.gz", &archive).unwrap();
    assert_eq!(
        sandbox.read("a.txt"),
        b"a",
        "an existing empty folder is fine"
    );

    let sandbox = Sandbox::new();
    fs::create_dir_all(&sandbox.dest).unwrap();
    fs::write(sandbox.dest.join("old.txt"), b"old").unwrap();
    assert_eq!(
        code(sandbox.extract("x.tar.gz", &archive)),
        ErrorCode::InvalidPath
    );
    assert_eq!(
        sandbox.read("old.txt"),
        b"old",
        "existing content is untouched"
    );

    let sandbox = Sandbox::new();
    fs::write(&sandbox.dest, b"i am a file").unwrap();
    assert_eq!(
        code(sandbox.extract("x.tar.gz", &archive)),
        ErrorCode::InvalidPath
    );
}

// ---------------------------------------------------------------------------------------------
// Cancellation, progress, async wrapper
// ---------------------------------------------------------------------------------------------

#[test]
fn cancelling_stops_extraction_without_touching_the_destination_if_done_early() {
    let sandbox = Sandbox::new();
    let path = sandbox.write_archive("x.tar.gz", &tar_gz(&[Entry::file("a.txt", b"a")]));
    let cancel = CancellationToken::new();
    cancel.cancel();
    let error = extract_sync(
        &path,
        &sandbox.dest,
        ArchiveFormat::TarGz,
        &ExtractOptions::default(),
        &cancel,
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::Cancelled);
    assert!(
        !sandbox.dest.exists(),
        "nothing is created for an already cancelled extraction"
    );
}

#[test]
fn cancelling_midway_stops_promptly() {
    let entries: Vec<Entry> = (0..20)
        .map(|i| Entry::file(&format!("f{i}.bin"), incompressible(200_000)))
        .collect();
    for (ext, build) in ALL_FORMATS {
        let sandbox = Sandbox::new();
        let path = sandbox.write_archive(&format!("x.{ext}"), &build(&entries));
        let cancel = CancellationToken::new();
        let trigger = cancel.clone();
        let format = ArchiveFormat::from_file_name(&format!("x.{ext}")).unwrap();
        let error = extract_sync(
            &path,
            &sandbox.dest,
            format,
            &ExtractOptions::default(),
            &cancel,
            &mut |p| {
                if p.bytes_written > 300_000 {
                    trigger.cancel();
                }
            },
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::Cancelled, "{ext}");
        let written = tree(&sandbox.dest).len();
        assert!(written < 20, "{ext}: stopped after {written} of 20 files");
    }
}

#[test]
fn progress_is_monotonic_and_ends_at_the_real_totals() {
    let entries = [
        Entry::file("a.bin", incompressible(150_000)),
        Entry::file("b.bin", incompressible(90_000)),
        Entry::dir("d/"),
    ];
    for (ext, build) in ALL_FORMATS {
        let sandbox = Sandbox::new();
        let path = sandbox.write_archive(&format!("x.{ext}"), &build(&entries));
        let format = ArchiveFormat::from_file_name(&format!("x.{ext}")).unwrap();
        let mut events: Vec<ExtractProgress> = Vec::new();
        let summary = extract_sync(
            &path,
            &sandbox.dest,
            format,
            &ExtractOptions::default(),
            &CancellationToken::new(),
            &mut |p| {
                events.push(p);
            },
        )
        .unwrap();

        assert_eq!(summary.bytes, 240_000, "{ext}");
        assert!(
            events
                .windows(2)
                .all(|w| w[0].bytes_written <= w[1].bytes_written
                    && w[0].entries_done <= w[1].entries_done),
            "{ext}: never goes backwards"
        );
        assert_eq!(events.last().unwrap().bytes_written, 240_000, "{ext}");
        let expected_total = if ext == "zip" { Some(240_000) } else { None };
        assert!(
            events.iter().all(|p| p.bytes_total == expected_total),
            "{ext}: the total is only known for zip"
        );
    }
}

#[tokio::test]
async fn the_async_wrapper_runs_off_the_runtime_and_reports_progress() {
    let sandbox = Sandbox::new();
    let path = sandbox.write_archive("x.tar.xz", &tar_xz(&[Entry::file("pkg/a.txt", b"async")]));
    let seen: Arc<Mutex<Vec<ExtractProgress>>> = Arc::default();
    let sink = Arc::clone(&seen);

    let summary = extract(
        path,
        sandbox.dest.clone(),
        ArchiveFormat::TarXz,
        ExtractOptions {
            strip_components: 1,
            ..ExtractOptions::default()
        },
        CancellationToken::new(),
        move |p| sink.lock().unwrap().push(p),
    )
    .await
    .unwrap();

    assert_eq!(summary.files, 1);
    assert_eq!(sandbox.read("a.txt"), b"async");
    assert!(!seen.lock().unwrap().is_empty());
}
