//! Builds archives in memory for extraction tests, including hostile ones that normal libraries refuse
//! to write (`../evil` names, links pointing outside, special files).

// Shared by several test crates, and each one uses only part of it.
#![allow(dead_code)]

use std::io::{Cursor, Write};

pub enum Kind {
    File(Vec<u8>),
    Dir,
    Symlink(String),
    Hardlink(String),
    Fifo,
}

pub struct Entry {
    pub path: String,
    pub kind: Kind,
    pub mode: u32,
}

impl Entry {
    pub fn file(path: &str, content: impl Into<Vec<u8>>) -> Self {
        Self {
            path: path.into(),
            kind: Kind::File(content.into()),
            mode: 0o644,
        }
    }
    pub fn dir(path: &str) -> Self {
        Self {
            path: path.into(),
            kind: Kind::Dir,
            mode: 0o755,
        }
    }
    pub fn symlink(path: &str, target: &str) -> Self {
        Self {
            path: path.into(),
            kind: Kind::Symlink(target.into()),
            mode: 0o777,
        }
    }
    pub fn hardlink(path: &str, target: &str) -> Self {
        Self {
            path: path.into(),
            kind: Kind::Hardlink(target.into()),
            mode: 0o644,
        }
    }
    pub fn fifo(path: &str) -> Self {
        Self {
            path: path.into(),
            kind: Kind::Fifo,
            mode: 0o644,
        }
    }
    pub fn mode(mut self, mode: u32) -> Self {
        self.mode = mode;
        self
    }
}

/// A plain tar. The header bytes are filled in directly, so no path validation gets in the way.
pub fn tar(entries: &[Entry]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for entry in entries {
        let mut header = tar::Header::new_gnu();
        let (entry_type, content): (tar::EntryType, &[u8]) = match &entry.kind {
            Kind::File(content) => (tar::EntryType::Regular, content),
            Kind::Dir => (tar::EntryType::Directory, &[]),
            Kind::Symlink(_) => (tar::EntryType::Symlink, &[]),
            Kind::Hardlink(_) => (tar::EntryType::Link, &[]),
            Kind::Fifo => (tar::EntryType::Fifo, &[]),
        };
        header.set_entry_type(entry_type);
        header.set_size(content.len() as u64);
        header.set_mode(entry.mode);
        header.set_mtime(1_700_000_000);

        let old = header.as_old_mut();
        let name = entry.path.as_bytes();
        assert!(
            name.len() < 100,
            "test entry names must fit the 100 byte tar field"
        );
        old.name[..name.len()].copy_from_slice(name);
        if let Kind::Symlink(target) | Kind::Hardlink(target) = &entry.kind {
            let target = target.as_bytes();
            assert!(target.len() < 100);
            old.linkname[..target.len()].copy_from_slice(target);
        }
        header.set_cksum();
        builder.append(&header, content).unwrap();
    }
    builder.into_inner().unwrap()
}

pub fn gzip(raw: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(raw).unwrap();
    encoder.finish().unwrap()
}

pub fn xz(raw: &[u8]) -> Vec<u8> {
    let mut writer =
        lzma_rust2::XzWriter::new(Vec::new(), lzma_rust2::XzOptions::with_preset(1)).unwrap();
    writer.write_all(raw).unwrap();
    writer.finish().unwrap()
}

pub fn tar_gz(entries: &[Entry]) -> Vec<u8> {
    gzip(&tar(entries))
}

pub fn tar_xz(entries: &[Entry]) -> Vec<u8> {
    xz(&tar(entries))
}

pub fn zip(entries: &[Entry]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for entry in entries {
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(entry.mode);
        match &entry.kind {
            Kind::File(content) => {
                writer.start_file(entry.path.as_str(), options).unwrap();
                writer.write_all(content).unwrap();
            }
            Kind::Dir => writer.add_directory(entry.path.as_str(), options).unwrap(),
            Kind::Symlink(target) => {
                writer
                    .add_symlink(entry.path.as_str(), target.as_str(), options)
                    .unwrap();
            }
            Kind::Hardlink(_) | Kind::Fifo => panic!("zip has no hard links or special files"),
        }
    }
    writer.finish().unwrap().into_inner()
}

/// The three formats under test, with a name for failure messages.
pub type Builder = fn(&[Entry]) -> Vec<u8>;

pub const TAR_FORMATS: [(&str, Builder); 2] = [("tar.gz", tar_gz), ("tar.xz", tar_xz)];
pub const ALL_FORMATS: [(&str, Builder); 3] =
    [("zip", zip), ("tar.gz", tar_gz), ("tar.xz", tar_xz)];

/// `bytes` of repeating text: compresses extremely well, which is what a decompression bomb relies on.
pub fn compressible(bytes: usize) -> Vec<u8> {
    vec![b'A'; bytes]
}

/// Bytes that do not compress, so truncating the archive really cuts into the data.
pub fn incompressible(bytes: usize) -> Vec<u8> {
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    (0..bytes)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 32) as u8
        })
        .collect()
}
