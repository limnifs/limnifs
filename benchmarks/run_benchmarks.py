#!/usr/bin/env python3
"""
LimniFS Benchmark Suite

Performance testing for LimniFS, modelled on the DwarFS benchmark
suite at https://github.com/tamatebako/dwarfs-t/tree/main/benchmarks.

Compares LimniFS against other read-only filesystem / archive tools
when they are available on the system:

  - tar + zstd   (baseline archive)
  - mksquashfs   (Linux read-only FS)
  - mkdwarfs     (DwarFS, if installed)
  - limni limn   (LimniFS, always tested)

Datasets:
  - tiny:    10 MB synthetic mixed text/binary
  - python:  Python tarball (real source tree, ~50 MB)
  - linux:   Linux kernel source tarball (real, ~1.2 GB)

Operations per format:
  - create:  pack the source tree into the format
  - verify:  integrity check (limni verify, dwarfsck, etc.)
  - extract: full filesystem extract
  - cat:     sequential read of N random files

Output:
  - JSON  (machine-readable, for trend tracking)
  - Markdown report (human-readable)

Usage:
  python3 benchmarks/run_benchmarks.py --datasets tiny,python --iterations 3
  python3 benchmarks/run_benchmarks.py --quick     # tiny only, 1 iter
  python3 benchmarks/run_benchmarks.py --all       # all datasets, 5 iter
"""

from __future__ import annotations

import argparse
import dataclasses
import hashlib
import json
import os
import platform
import shutil
import statistics
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.request
from datetime import datetime, timezone
from pathlib import Path
from typing import Optional

CARGO_WORKSPACE = Path(__file__).resolve().parent.parent
DEFAULT_OUTPUT_DIR = CARGO_WORKSPACE / "benchmarks" / "results"

DATASET_SOURCES = {
    # Tiny dataset: synthetic, no download.
    "tiny": None,
    # Real datasets: tarball URLs.
    "python": "https://www.python.org/ftp/python/3.12.0/Python-3.12.0.tgz",
    "linux": "https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-6.6.tar.xz",
}

DATASET_APPROX_SIZES = {
    "tiny": "10 MB",
    "python": "55 MB",
    "linux": "130 MB packed / 1.2 GB unpacked",
}


@dataclasses.dataclass
class OperationMetrics:
    operation: str
    iterations: int
    times_seconds: list[float]
    peak_rss_kb: Optional[int]
    output_size_bytes: Optional[int]

    @property
    def median_seconds(self) -> float:
        return statistics.median(self.times_seconds) if self.times_seconds else 0.0

    @property
    def stdev_seconds(self) -> float:
        return (
            statistics.stdev(self.times_seconds)
            if len(self.times_seconds) > 1
            else 0.0
        )


@dataclasses.dataclass
class BenchmarkResult:
    format: str
    dataset: str
    create: Optional[OperationMetrics] = None
    verify: Optional[OperationMetrics] = None
    extract: Optional[OperationMetrics] = None
    cat: Optional[OperationMetrics] = None
    notes: str = ""

    def to_dict(self) -> dict:
        return {
            "format": self.format,
            "dataset": self.dataset,
            "create": dataclasses.asdict(self.create) if self.create else None,
            "verify": dataclasses.asdict(self.verify) if self.verify else None,
            "extract": dataclasses.asdict(self.extract) if self.extract else None,
            "cat": dataclasses.asdict(self.cat) if self.cat else None,
            "notes": self.notes,
        }


def cargo_build_release() -> Path:
    """Build the limni release binary and return its path."""
    print("[setup] cargo build --release --bin limni")
    subprocess.run(
        ["cargo", "build", "--release", "--bin", "limni"],
        cwd=CARGO_WORKSPACE,
        check=True,
    )
    return CARGO_WORKSPACE / "target" / "release" / "limni"


def ensure_dataset(name: str, workspace: Path) -> Path:
    """Return a path to the unpacked dataset directory."""
    target_dir = workspace / f"datasets/{name}"
    if target_dir.exists() and any(target_dir.iterdir()):
        return target_dir
    target_dir.mkdir(parents=True, exist_ok=True)

    if name == "tiny":
        print(f"[dataset] generating synthetic tiny dataset (~{DATASET_APPROX_SIZES[name]})")
        generate_tiny_dataset(target_dir)
        return target_dir

    url = DATASET_SOURCES[name]
    if not url:
        raise ValueError(f"unknown dataset: {name}")
    print(f"[dataset] downloading {name} from {url}")
    archive_path = workspace / f"datasets/{name}.tar"
    with urllib.request.urlopen(url) as resp, open(archive_path, "wb") as f:
        shutil.copyfileobj(resp, f)
    print(f"[dataset] unpacking {name}")
    with tarfile.open(archive_path) as tar:
        tar.extractall(target_dir)
    archive_path.unlink(missing_ok=True)

    # If the unpack produced a single top-level dir, hoist its contents.
    children = list(target_dir.iterdir())
    if len(children) == 1 and children[0].is_dir():
        single = children[0]
        for entry in single.iterdir():
            shutil.move(str(entry), str(target_dir / entry.name))
        single.rmdir()

    return target_dir


def generate_tiny_dataset(target: Path) -> None:
    """Synthetic 10 MB mixed-content dataset."""
    # 5 MB of text (repeating lorem-ipsum-like content).
    text_chunk = (
        b"Lorem ipsum dolor sit amet, consectetur adipiscing elit. "
        b"Integer nec odio. Praesent libero. Sed cursus ante dapibus diam. "
        b"Sed nisi. Nulla quis sem at nibh elementum imperdiet. "
    )
    (target / "doc.txt").write_bytes(text_chunk * 20_000)  # ~5 MB

    # 4 MB of pseudo-random binary (incompressible).
    rng_state = 0x1234_5678_9ABC_DEF0
    random_bytes = bytearray()
    for _ in range(4 * 1024 * 1024 // 8):
        rng_state = (rng_state * 6_364_136_223_846_793_005 + 1_442_695_040_888_963_407) & 0xFFFF_FFFF_FFFF_FFFF
        random_bytes.extend(rng_state.to_bytes(8, "big"))
    (target / "binary.bin").write_bytes(random_bytes)

    # Small tree of small files (1000 files × 1 KB).
    many_dir = target / "many"
    many_dir.mkdir()
    for i in range(1000):
        (many_dir / f"file_{i:04d}").write_bytes(
            hashlib.sha256(f"file-{i}".encode()).digest()
        )


def time_operation(
    cmd: list[str],
    cwd: Optional[Path] = None,
    capture_output: bool = True,
    iterations: int = 1,
    output_files_to_clean: Optional[list[Path]] = None,
) -> OperationMetrics:
    """Run cmd N times, return timing + memory stats.

    For create-style benchmarks where the output path is overwritten
    each iteration, pass `output_files_to_clean` so each iteration
    starts from a clean state (some tools, like mksquashfs, refuse to
    overwrite by default).
    """
    times: list[float] = []
    peak_rss_kb: Optional[int] = None

    for _ in range(iterations):
        if output_files_to_clean:
            for path in output_files_to_clean:
                path.unlink(missing_ok=True)
        start = time.perf_counter()
        result = subprocess.run(
            cmd,
            cwd=cwd,
            capture_output=capture_output,
            text=True,
        )
        elapsed = time.perf_counter() - start
        times.append(elapsed)
        if result.returncode != 0:
            raise RuntimeError(
                f"command failed: {' '.join(cmd)}\nstderr: {result.stderr}"
            )

    # On Linux we can use /usr/bin/time -v to get peak RSS.
    time_bin = shutil.which("time")
    if time_bin:
        # Run one more iteration with /usr/bin/time to sample memory.
        mem_cmd = [time_bin, "-v"] + cmd
        mem_result = subprocess.run(
            mem_cmd,
            cwd=cwd,
            capture_output=True,
            text=True,
        )
        if mem_result.returncode == 0:
            for line in mem_result.stderr.splitlines():
                if "Maximum resident set size" in line:
                    parts = line.split()
                    if parts:
                        peak_rss_kb = int(parts[-1])
                    break

    return OperationMetrics(
        operation=cmd[0],
        iterations=iterations,
        times_seconds=times,
        peak_rss_kb=peak_rss_kb,
        output_size_bytes=None,
    )


def benchmark_limnifs(
    limni: Path,
    source: Path,
    workspace: Path,
    iterations: int,
) -> BenchmarkResult:
    """Run limni create/verify/extract/cat against the source tree."""
    result = BenchmarkResult(format="limnifs", dataset=source.name)

    image_path = workspace / "limnifs.lim"
    print(f"  [limnifs] create ({iterations} iter)")
    create_metrics = time_operation(
        [str(limni), "limn", str(source), str(image_path)],
        iterations=iterations,
    )
    create_metrics.output_size_bytes = image_path.stat().st_size
    result.create = create_metrics

    print(f"  [limnifs] verify ({iterations} iter)")
    result.verify = time_operation(
        [str(limni), "verify", str(image_path)],
        iterations=iterations,
    )

    extract_dir = workspace / "limnifs_extract"
    if extract_dir.exists():
        shutil.rmtree(extract_dir)
    print(f"  [limnifs] extract ({iterations} iter)")
    extract_metrics = time_operation(
        [str(limni), "extract", str(image_path), str(extract_dir)],
        iterations=iterations,
    )
    result.extract = extract_metrics
    shutil.rmtree(extract_dir, ignore_errors=True)

    # Sequential read: cat every file in the source tree.
    print(f"  [limnifs] cat (sequential read, 1 iter)")
    files = sorted(p for p in source.rglob("*") if p.is_file())
    cat_metric = OperationMetrics(
        operation="limni cat (sequential)",
        iterations=1,
        times_seconds=[],
        peak_rss_kb=None,
        output_size_bytes=None,
    )
    if files:
        start = time.perf_counter()
        for f in files:
            rel = f.relative_to(source)
            subprocess.run(
                [str(limni), "cat", str(image_path), f"/{rel}"],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                check=True,
            )
        cat_metric.times_seconds = [time.perf_counter() - start]
    result.cat = cat_metric

    image_path.unlink(missing_ok=True)
    return result


def benchmark_tar_zstd(
    source: Path,
    workspace: Path,
    iterations: int,
) -> Optional[BenchmarkResult]:
    """tar + zstd baseline. Returns None if zstd not installed."""
    zstd = shutil.which("zstd")
    tar = shutil.which("tar")
    if not zstd or not tar:
        return None
    result = BenchmarkResult(format="tar+zstd", dataset=source.name, notes="baseline archive")

    archive = workspace / "tar_zstd.tar.zst"
    print(f"  [tar+zstd] create ({iterations} iter)")
    create_metrics = time_operation(
        [tar, "--use-compress-program", zstd, "-cf", str(archive), "-C", str(source.parent), source.name],
        iterations=iterations,
    )
    create_metrics.output_size_bytes = archive.stat().st_size
    result.create = create_metrics

    extract_dir = workspace / "tar_zstd_extract"
    if extract_dir.exists():
        shutil.rmtree(extract_dir)
    extract_dir.mkdir()
    print(f"  [tar+zstd] extract ({iterations} iter)")
    result.extract = time_operation(
        [tar, "--use-compress-program", zstd, "-xf", str(archive), "-C", str(extract_dir)],
        iterations=iterations,
    )
    shutil.rmtree(extract_dir, ignore_errors=True)
    archive.unlink(missing_ok=True)
    return result


def benchmark_squashfs(
    source: Path,
    workspace: Path,
    iterations: int,
) -> Optional[BenchmarkResult]:
    if not shutil.which("mksquashfs"):
        return None
    result = BenchmarkResult(format="squashfs", dataset=source.name)
    image = workspace / "squashfs.sqfs"
    print(f"  [squashfs] create ({iterations} iter)")
    create_metrics = time_operation(
        ["mksquashfs", str(source), str(image), "-noappend", "-no-progress"],
        iterations=iterations,
        output_files_to_clean=[image],
    )
    create_metrics.output_size_bytes = image.stat().st_size
    result.create = create_metrics
    image.unlink(missing_ok=True)
    # Extract/verify: would need mounting or unsquashfs; skip if not available.
    if shutil.which("unsquashfs"):
        extract_dir = workspace / "squashfs_extract"
        if extract_dir.exists():
            shutil.rmtree(extract_dir)
        print(f"  [squashfs] extract ({iterations} iter)")
        result.extract = time_operation(
            ["unsquashfs", "-d", str(extract_dir), str(image)],
            iterations=iterations,
        )
        shutil.rmtree(extract_dir, ignore_errors=True)
    return result


def benchmark_dwarfs(
    source: Path,
    workspace: Path,
    iterations: int,
) -> Optional[BenchmarkResult]:
    if not shutil.which("mkdwarfs"):
        return None
    result = BenchmarkResult(format="dwarfs", dataset=source.name)
    image = workspace / "dwarfs.dwarfs"
    print(f"  [dwarfs] create ({iterations} iter)")
    create_metrics = time_operation(
        ["mkdwarfs", "-i", str(source), "-o", str(image), "-f"],
        iterations=iterations,
    )
    create_metrics.output_size_bytes = image.stat().st_size
    result.create = create_metrics

    if shutil.which("dwarfsextract"):
        extract_dir = workspace / "dwarfs_extract"
        if extract_dir.exists():
            shutil.rmtree(extract_dir)
        print(f"  [dwarfs] extract ({iterations} iter)")
        result.extract = time_operation(
            ["dwarfsextract", "-i", str(image), "-o", str(extract_dir)],
            iterations=iterations,
        )
        shutil.rmtree(extract_dir, ignore_errors=True)
    image.unlink(missing_ok=True)
    return result


def render_markdown_report(
    results: list[BenchmarkResult],
    metadata: dict,
) -> str:
    lines = [
        "# LimniFS Benchmark Report",
        "",
        f"- **Date**: {metadata['date']}",
        f"- **Platform**: {metadata['platform']} {metadata['machine']}",
        f"- **Iterations**: {metadata['iterations']}",
        f"- **LimniFS version**: {metadata['limnifs_version']}",
        "",
        "## Results",
        "",
    ]
    by_dataset: dict[str, list[BenchmarkResult]] = {}
    for r in results:
        by_dataset.setdefault(r.dataset, []).append(r)

    for dataset, group in by_dataset.items():
        lines.append(f"### Dataset: `{dataset}` (~{DATASET_APPROX_SIZES.get(dataset, '?')})")
        lines.append("")
        lines.append("| Format | Create (s) | Verify (s) | Extract (s) | Cat (s) | Size (MB) |")
        lines.append("|---|---|---|---|---|---|")
        for r in group:
            create = f"{r.create.median_seconds:.2f}" if r.create else "—"
            verify = f"{r.verify.median_seconds:.2f}" if r.verify else "—"
            extract = f"{r.extract.median_seconds:.2f}" if r.extract else "—"
            cat = f"{r.cat.median_seconds:.2f}" if r.cat and r.cat.times_seconds else "—"
            size = (
                f"{r.create.output_size_bytes / 1024 / 1024:.1f}"
                if r.create and r.create.output_size_bytes
                else "—"
            )
            lines.append(f"| {r.format} | {create} | {verify} | {extract} | {cat} | {size} |")
        lines.append("")

    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--datasets",
        default="tiny",
        help=f"comma-separated dataset names; one of: {','.join(DATASET_SOURCES)}",
    )
    parser.add_argument("--iterations", type=int, default=3)
    parser.add_argument("--quick", action="store_true", help="tiny only, 1 iteration")
    parser.add_argument("--all", action="store_true", help="all datasets, 5 iterations")
    parser.add_argument("--output-dir", type=Path, default=DEFAULT_OUTPUT_DIR)
    parser.add_argument(
        "--workspace",
        type=Path,
        default=None,
        help="scratch dir for datasets and benchmark artifacts",
    )
    args = parser.parse_args()

    if args.quick:
        args.datasets = "tiny"
        args.iterations = 1
    if args.all:
        args.datasets = ",".join(DATASET_SOURCES)
        args.iterations = 5

    workspace = args.workspace or Path(tempfile.mkdtemp(prefix="limnifs-bench-"))
    workspace.mkdir(parents=True, exist_ok=True)
    args.output_dir.mkdir(parents=True, exist_ok=True)

    limni = cargo_build_release()

    datasets = [d.strip() for d in args.datasets.split(",") if d.strip()]
    results: list[BenchmarkResult] = []
    for dataset in datasets:
        print(f"\n{'=' * 70}\nDataset: {dataset}\n{'=' * 70}")
        source = ensure_dataset(dataset, workspace)
        run_workspace = workspace / f"run_{dataset}"
        run_workspace.mkdir(exist_ok=True)

        for runner in (benchmark_limnifs, benchmark_tar_zstd, benchmark_squashfs, benchmark_dwarfs):
            try:
                if runner is benchmark_limnifs:
                    r = runner(limni, source, run_workspace, args.iterations)
                else:
                    r = runner(source, run_workspace, args.iterations)
                if r is not None:
                    results.append(r)
            except Exception as exc:
                print(f"  [skip] {runner.__name__}: {exc}", file=sys.stderr)

    metadata = {
        "date": datetime.now(timezone.utc).isoformat(),
        "platform": platform.system(),
        "machine": platform.machine(),
        "iterations": args.iterations,
        "limnifs_version": subprocess.check_output(
            [str(limni), "--version"], text=True
        ).strip(),
    }

    timestamp = datetime.now().strftime("%Y%m%d_%H%M%S")
    json_path = args.output_dir / f"bench_{timestamp}.json"
    md_path = args.output_dir / f"bench_{timestamp}.md"

    json_path.write_text(
        json.dumps({"metadata": metadata, "results": [r.to_dict() for r in results]}, indent=2)
    )
    md_path.write_text(render_markdown_report(results, metadata))

    print(f"\nJSON report:    {json_path}")
    print(f"Markdown report: {md_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
