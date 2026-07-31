# limni — LimniFS CLI

One format, one CLI. `limni` is the command-line interface for
LimniFS — Layered, Immutable, Merkle-rooted, Network Image filesystem.

## Installation

```sh
cargo build --release
# Binary at target/release/limni
```

For FUSE mount support (requires macFUSE on macOS or libfuse on Linux):

```sh
cargo build --release --features fuse
```

## Commands

### Create

| Command | Description |
|---|---|
| `limni limn <dir> <image.lim>` | Build a `.lim` image from a directory tree |
| `limni compact <source.lim> <output.lim>` | Eliminate slab garbage by re-packing |

### Inspect

| Command | Description |
|---|---|
| `limni verify <image.lim>` | Validate manifest, compute `ManifestRoot` |
| `limni verify --json <image.lim>` | Machine-readable JSON output |
| `limni inspect <image.lim>` | Comprehensive overview (metadata, slabs, ratios) |
| `limni tree <image.lim> [path]` | Recursive directory tree listing |
| `limni ls <image.lim> [path]` | List directory contents |
| `limni stat <image.lim> <path>` | Print inode metadata (mode, sizes, content handle) |
| `limni cat <image.lim> <path>` | Write file contents to stdout |
| `limni history <image.lim>` | Print the history section (build/delta/flatten ops) |

### Analyse

| Command | Description |
|---|---|
| `limni diff <parent.lim> <child.lim>` | Compute tree operations (Add/Remove/Replace) |
| `limni gc <image.lim>` | Find unreferenced drops (slab garbage analysis) |
| `limni dedup <image.lim>` | Analyse drop dedup across files |
| `limni slab <slab.bin>` | Inspect a slab file (drop records, codecs, ratios) |
| `limni check <image.lim>` | Deep integrity check (BLAKE3 hash verification) |
| `limni benchmark` | Quick write/read/extract benchmark on synthetic tree |

### Mount

| Command | Description |
|---|---|
| `limni mount <image.lim> <mountpoint>` | Mount as read-only FUSE filesystem (requires `fuse` feature) |

## Examples

### Build and verify

```sh
$ limni limn ./mydir output.lim
output.lim: wrote 417 bytes, b3:kouzvnjlelxh4rp4zsgvma6za7tkd5cpaffiozzv6mvxnmw23o4a
  inodes: 4  files: 2  dirs: 2  drops: 0

$ limni verify output.lim
output.lim: valid LimniFS manifest
  ...
  merkle root:         b3:kouzvnjlelxh4rp4zsgvma6za7tkd5cpaffiozzv6mvxnmw23o4a
```

### Explore

```sh
$ limni tree output.lim
├── a.txt
└── sub dir
    └── b.txt

$ limni cat output.lim /a.txt
hello world

$ limni inspect output.lim
image: output.lim
  format versions: drop_store=1 metadata=1 manifest=1
  metadata: inlined
  metadata blob: 4 inodes, 2 dir nodes, root inode = 1
    files: 2, directories: 2
  slab index: 0 entries
```

### Diff two images

```sh
$ limni diff parent.lim child.lim
ops: 1
A b.txt inode=3
```

### GC analysis

```sh
$ limni gc image.lim
gc analysis: image.lim
  total drops in slab(s): 2
  referenced by manifest: 2
  garbage (unreferenced):  0 drops, 0 bytes
  status: clean (no garbage)
```

## Exit codes

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | read error (I/O, format) |
| 2 | usage error |
