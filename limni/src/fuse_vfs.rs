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
    Config, Errno, FileHandle, FileType, Filesystem, FopenFlags, Generation, INodeNo, LockOwner,
    OpenFlags, ReplyAttr, ReplyData, ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyOpen, ReplyWrite,
    Request, WriteFlags,
};

use crate::vfs::{Vfs, VfsType};

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
    /// root (`INodeNo::ROOT = 1`) maps to the `VFS` root.
    fn map_inode(&self, ino: INodeNo) -> u64 {
        if ino == INodeNo::ROOT {
            self.vfs.root_inode()
        } else {
            ino.0
        }
    }

    fn vfs_type_to_ftype(kind: VfsType) -> FileType {
        match kind {
            VfsType::Directory => FileType::Directory,
            VfsType::Symlink => FileType::Symlink,
            VfsType::Regular | VfsType::Other => FileType::RegularFile,
        }
    }

    fn make_attr(ino: u64, attr: &crate::vfs::VfsAttr) -> fuser::FileAttr {
        fuser::FileAttr {
            ino: INodeNo(ino),
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
    fn lookup(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        let parent_vfs = self.map_inode(parent);
        let Some(name_str) = name.to_str() else {
            reply.error(Errno::ENOENT);
            return;
        };
        match self.vfs.lookup(parent_vfs, name_str) {
            Some(child_ino) => {
                if let Some(attr) = self.vfs.getattr(child_ino) {
                    let fattr = Self::make_attr(child_ino, &attr);
                    reply.entry(&Duration::from_secs(1), &fattr, Generation(0));
                } else {
                    reply.error(Errno::ENOENT);
                }
            }
            None => {
                reply.error(Errno::ENOENT);
            }
        }
    }

    fn getattr(&self, _req: &Request, ino: INodeNo, _fh: Option<FileHandle>, reply: ReplyAttr) {
        let vfs_ino = self.map_inode(ino);
        match self.vfs.getattr(vfs_ino) {
            Some(attr) => {
                let fattr = Self::make_attr(vfs_ino, &attr);
                reply.attr(&Duration::from_secs(1), &fattr);
            }
            None => {
                reply.error(Errno::ENOENT);
            }
        }
    }

    fn readdir(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        let vfs_ino = self.map_inode(ino);
        let entries = self.vfs.readdir(vfs_ino);

        if offset == 0 {
            if reply.add(ino, 1, FileType::Directory, ".") {
                return;
            }
            if reply.add(ino, 2, FileType::Directory, "..") {
                return;
            }
        }

        for (i, (child_ino, name, kind)) in entries.iter().enumerate() {
            let entry_idx = u64::try_from(i).unwrap_or(0) + 2;
            if entry_idx < offset {
                continue;
            }
            let ftype = Self::vfs_type_to_ftype(*kind);
            if reply.add(INodeNo(*child_ino), entry_idx + 1, ftype, name) {
                break;
            }
        }
        reply.ok();
    }

    fn open(&self, _req: &Request, _ino: INodeNo, _flags: OpenFlags, reply: ReplyOpen) {
        reply.opened(FileHandle(0), FopenFlags::empty());
    }

    fn read(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        size: u32,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyData,
    ) {
        let vfs_ino = self.map_inode(ino);
        match self
            .vfs
            .read(vfs_ino, offset, usize::try_from(size).unwrap_or(usize::MAX))
        {
            Ok(data) => reply.data(&data),
            Err(_) => reply.error(Errno::EIO),
        }
    }

    fn release(
        &self,
        _req: &Request,
        _ino: INodeNo,
        _fh: FileHandle,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        reply.ok();
    }

    fn write(
        &self,
        _req: &Request,
        _ino: INodeNo,
        _fh: FileHandle,
        _offset: u64,
        _data: &[u8],
        _write_flags: WriteFlags,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyWrite,
    ) {
        reply.error(Errno::ENOSYS);
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
    fuser::mount(fs, mountpoint, &Config::default())
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
