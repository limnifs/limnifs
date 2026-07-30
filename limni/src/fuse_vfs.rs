//! `FUSE` filesystem frontend — bridges [`crate::vfs::Vfs`] to the
//! kernel via the `fuser` crate.
//!
//! Only available when the `fuse` feature is enabled:
//!
//! ```sh
//! cargo build --release --features fuse
//! ```
//!
//! Implements a read-only filesystem. All mutating operations
//! (`create`, `write`, `unlink`, etc.) return `ENOSYS`.

#![cfg(feature = "fuse")]

use std::ffi::OsStr;
use std::time::Duration;

use fuser::{
    Filesystem, ReplyAttr, ReplyData, ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyOpen,
    ReplyWrite, Request, FUSE_ROOT_ID,
};

use crate::vfs::{Vfs, VfsType};

/// Convert a VFS type to a `fuser` file type constant.
const ROOT_MODE: u32 = 0o755;

/// A read-only `FUSE` filesystem backed by a [`Vfs`].
pub struct FuseVfs {
    vfs: Vfs,
}

impl FuseVfs {
    /// Create a new `FUSE` filesystem from an opened [`Vfs`].
    #[must_use]
    pub const fn new(vfs: Vfs) -> Self {
        Self { vfs }
    }

    /// Map a `FUSE` inode number to a `VFS` inode number. The `FUSE`
    /// root (`FUSE_ROOT_ID = 1`) maps to the `VFS` root.
    fn map_inode(&self, ino: u64) -> u64 {
        if ino == FUSE_ROOT_ID {
            self.vfs.root_inode()
        } else {
            ino
        }
    }

    fn vfs_type_to_ftype(kind: VfsType) -> fuser::FileType {
        match kind {
            VfsType::Regular => fuser::FileType::RegularFile,
            VfsType::Directory => fuser::FileType::Directory,
            VfsType::Symlink => fuser::FileType::Symlink,
            VfsType::Other => fuser::FileType::RegularFile,
        }
    }

    fn make_attr(&self, ino: u64, attr: &crate::vfs::VfsAttr) -> fuser::FileAttr {
        let mapped = self.map_inode(ino);
        fuser::FileAttr {
            ino: mapped,
            size: attr.size,
            blocks: attr.size.div_ceil(512),
            atime: epoch_to_time(attr.mtime_ns),
            mtime: epoch_to_time(attr.mtime_ns),
            ctime: epoch_to_time(attr.mtime_ns),
            crtime: epoch_to_time(attr.mtime_ns),
            kind: Self::vfs_type_to_ftype(attr.kind),
            perm: (attr.mode & 0o7777) as u16,
            nlink: attr.nlink,
            uid: 0,
            gid: 0,
            rdev: 0,
            blksize: 4096,
            flags: 0,
        }
    }
}

fn epoch_to_time(ns: u64) -> std::time::SystemTime {
    let secs = ns / 1_000_000_000;
    let nanos = u32::try_from(ns % 1_000_000_000).unwrap_or(0);
    std::time::UNIX_EPOCH + Duration::new(secs, nanos)
}

impl Filesystem for FuseVfs {
    fn lookup(&mut self, req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let parent_vfs = self.map_inode(parent);
        let name_str = match name.to_str() {
            Some(s) => s,
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };
        match self.vfs.lookup(parent_vfs, name_str) {
            Some(child_ino) => {
                if let Some(attr) = self.vfs.getattr(child_ino) {
                    let fattr = self.make_attr(child_ino, &attr);
                    reply.entry(&Duration::from_secs(1), &fattr, 0);
                } else {
                    reply.error(libc::ENOENT);
                }
            }
            None => {
                reply.error(libc::ENOENT);
            }
        }
        let _ = req;
    }

    fn getattr(&mut self, req: &Request, ino: u64, reply: ReplyAttr) {
        let vfs_ino = self.map_inode(ino);
        match self.vfs.getattr(vfs_ino) {
            Some(attr) => {
                let fattr = self.make_attr(ino, &attr);
                reply.attr(&Duration::from_secs(1), &fattr);
            }
            None => {
                reply.error(libc::ENOENT);
            }
        }
        let _ = req;
    }

    fn readdir(
        &mut self,
        req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        let vfs_ino = self.map_inode(ino);
        let entries = self.vfs.readdir(vfs_ino);

        let mut idx = offset;
        if idx == 0 {
            let attr = self.vfs.getattr(vfs_ino).map(|a| self.make_attr(ino, &a));
            if reply.add(ino, 1, fuser::FileType::Directory, ".") {
                return;
            }
            if reply.add(ino, 2, fuser::FileType::Directory, "..") {
                return;
            }
            let _ = attr;
            idx = 2;
        }

        for (i, (child_ino, name, kind)) in entries.iter().enumerate() {
            let entry_idx = i64::try_from(i).unwrap_or(0) + 2;
            if entry_idx < offset {
                continue;
            }
            let ftype = Self::vfs_type_to_ftype(*kind);
            if reply.add(*child_ino, entry_idx + 1, ftype, name) {
                break;
            }
        }
        reply.ok();
        let _ = req;
    }

    fn open(&mut self, req: &Request, ino: u64, flags: i32, reply: ReplyOpen) {
        let _ = (req, ino, flags);
        reply.opened(0, 0);
    }

    fn read(
        &mut self,
        req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        let vfs_ino = self.map_inode(ino);
        match self
            .vfs
            .read(vfs_ino, u64::try_from(offset).unwrap_or(0), size as usize)
        {
            Ok(data) => reply.data(&data),
            Err(_) => reply.error(libc::EIO),
        }
        let _ = req;
    }

    fn release(
        &mut self,
        req: &Request,
        ino: u64,
        fh: u64,
        flags: i32,
        lock_owner: Option<u64>,
        flush: bool,
        reply: ReplyEmpty,
    ) {
        let _ = (req, ino, fh, flags, lock_owner, flush);
        reply.ok();
    }

    fn write(
        &mut self,
        req: &Request,
        ino: u64,
        fh: u64,
        offset: i64,
        data: &[u8],
        write_flags: u32,
        flags: i32,
        lock_owner: Option<u64>,
        reply: ReplyWrite,
    ) {
        let _ = (req, ino, fh, offset, data, write_flags, flags, lock_owner);
        reply.error(libc::ENOSYS);
    }
}

/// Mount the `FUSE` filesystem at `mountpoint`. Blocks until the
/// filesystem is unmounted.
///
/// # Errors
///
/// Returns `std::io::Error` if the mount fails (e.g. the mountpoint
/// does not exist, or `FUSE` kernel support is unavailable).
pub fn mount(vfs: Vfs, mountpoint: &std::path::Path) -> std::io::Result<()> {
    let fs = FuseVfs::new(vfs);
    fuser::mount2(fs, mountpoint, &[])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_to_time_zero_is_unix_epoch() {
        let t = epoch_to_time(0);
        assert_eq!(t, std::time::UNIX_EPOCH);
    }

    #[test]
    fn epoch_to_time_round_trips() {
        let ns: u64 = 1_700_000_000_000_000_000;
        let t = epoch_to_time(ns);
        let dur = t.duration_since(std::time::UNIX_EPOCH).unwrap();
        assert_eq!(dur.as_nanos(), u128::from(ns));
    }
}
