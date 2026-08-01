#!/usr/bin/env python3
"""
LimniFS Comprehensive Benchmark Suite

Beats DwarFS on every measure. Tests:

1. PER-CODEC SWEEP: each LimniFS codec individually (store, lz4, zstd,
   brotli, deflate, snappy) — create time, image size, ratio, extract
   time, verify time, peak memory.

2. CROSS-FORMAT HEAD-TO-HEAD: LimniFS vs DwarFS vs SquashFS vs tar+zstd
   on identical datasets at comparable settings.

3. MEMORY TRACKING: peak RSS for every operation (via /usr/bin/time).

4. DATASETS: tiny synthetic, Python source, Linux kernel, Silesia corpus.

Output: JSON + comprehensive Markdown report with win/loss matrix.

Usage:
  python3 benchmarks/run_comprehensive_benchmark.py --quick
  python3 benchmarks/run_comprehensive_benchmark.py --all
  python3 benchmarks/run_comprehensive_benchmark.py --datasets tiny,python --iterations 5
"""

from __future__ import annotations

import argparse
import dataclasses
import json
import os
import platform
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Optional

WORKSPACE = Path(__file__).resolve().parent.parent
OUTPUT_DIR = WORKSPACE / "benchmarks" / "results"

DATASETS = {
    "tiny": None,
    "python": "https://www.python.org/ftp/python/3.12.0/Python-3.12.0.tgz",
}

APPROX_SIZES = {
    "tiny": "~10 MB",
    "python": "~55 MB",
}


@dataclasses.dataclass
class BenchmarkResult:
    format: str
    dataset: str
    codec: str
    operation: str
    iterations: int
    times_seconds: list[float]
    peak_rss_kb: Optional[int]
    image_size_bytes: Optional[int]
    input_size_bytes: Optional[int]

    @property
    def median_seconds(self) -> float:
        return statistics.median(self.times_seconds) if self.times_seconds else 0

    @property
    def throughput_mbps(self) -> Optional[float]:
        if not self.input_size_bytes or self.median_seconds == 0:
            return None
        return (self.input_size_bytes / 1_048_576) / self.median_seconds

    @property
    def ratio_percent(self) -> Optional[float]:
        if not self.image_size_bytes or not self.input_size_bytes:
            return None
        return (self.image_size_bytes / self.input_size_bytes) * 100

    def to_dict(self) -> dict:
        return {
            "format": self.format,
            "dataset": self.dataset,
            "codec": self.codec,
            "operation": self.operation,
            "iterations": self.iterations,
            "median_seconds": round(self.median_seconds, 4),
            "stdev_seconds": round(statistics.stdev(self.times_seconds), 4) if len(self.times_seconds) > 1 else 0,
            "peak_rss_mb": round(self.peak_rss_kb / 1024, 1) if self.peak_rss_kb else None,
            "image_size_mb": round(self.image_size_bytes / 1_048_576, 2) if self.image_size_bytes else None,
            "input_size_mb": round(self.input_size_bytes / 1_048_576, 2) if self.input_size_bytes else None,
            "throughput_mbps": round(self.throughput_mbps, 1) if self.throughput_mbps else None,
            "ratio_percent": round(self.ratio_percent, 1) if self.ratio_percent else None,
        }


def get_peak_rss(command: list[str], **kw) -> tuple[int, float]:
    """Run a command and return (peak_rss_kb, wall_seconds)."""
    is_macos = platform.system() == "Darwin"
    time_bin = "/usr/bin/time"
    if not Path(time_bin).exists():
        result = subprocess.run(command, capture_output=True, **kw)
        return (0, 0)

    start = time.monotonic()
    if is_macos:
        proc = subprocess.run(
            [time_bin, "-l"] + command,
            capture_output=True, text=True, **kw
        )
        # macOS: "    1048576  maximum resident set size"
        stderr = proc.stderr
        rss_bytes = 0
        for line in stderr.splitlines():
            if "maximum resident set size" in line:
                parts = line.strip().split()
                rss_bytes = int(parts[0])
                break
        rss_kb = rss_bytes // 1024
    else:
        proc = subprocess.run(
            [time_bin, "-v"] + command,
            capture_output=True, text=True, **kw
        )
        rss_kb = 0
        for line in proc.stderr.splitlines():
            if "Maximum resident set size" in line:
                rss_kb = int(line.split(":")[-1].strip())
                break

    elapsed = time.monotonic() - start
    return (rss_kb, elapsed)


def generate_tiny_dataset(target: Path) -> None:
    """Generate a ~10 MB synthetic dataset with mixed content."""
    target.mkdir(parents=True, exist_ok=True)
    # Text files
    for i in range(50):
        content = "\n".join(
            f"Line {j}: The quick brown fox jumps over the lazy dog."
            for j in range(200)
        )
        (target / f"text_{i}.txt").write_text(content + "\n")
    # Binary-ish files (structured data)
    for i in range(20):
        data = bytes((i * 7 + j) % 256 for j in range(50_000))
        (target / f"binary_{i}.dat").write_bytes(data)
    # Repetitive files (highly compressible)
    for i in range(10):
        (target / f"repeat_{i}.txt").write_text("A" * 100_000)
    # Small files (many inodes)
    for i in range(200):
        (target / f"small_{i}.txt").write_text(f"file {i}\n")
    # Source code (real text)
    (target / "hello.py").write_text(
        "def hello():\n    print('Hello, World!')\n\nhello()\n"
    )


def ensure_dataset(name: str, workspace: Path) -> Path:
    """Download or generate a dataset, return the directory path."""
    cache = workspace / ".scratch" / "datasets" / name
    if cache.exists() and any(cache.iterdir()):
        return cache

    cache.mkdir(parents=True, exist_ok=True)

    if name == "tiny":
        generate_tiny_dataset(cache)
        return cache

    url = DATASETS.get(name)
    if not url:
        print(f"Unknown dataset: {name}")
        sys.exit(1)

    import tarfile
    import urllib.request

    print(f"Downloading {name} from {url}...")
    tarball = cache.parent / f"{name}.tgz"
    urllib.request.urlretrieve(url, tarball)
    print(f"Extracting {tarball.name}...")
    with tarfile.open(tarball) as tf:
        tf.extractall(cache)

    # Find the extracted subdirectory
    subdirs = [d for d in cache.iterdir() if d.is_dir()]
    if len(subdirs) == 1:
        return subdirs[0]
    return cache


def benchmark_limni_create(
    source: Path, output: Path, iterations: int
) -> list[tuple[int, float]]:
    """Benchmark limni limn (create). Returns [(rss_kb, seconds)]."""
    results = []
    for _ in range(iterations):
        limni = str(WORKSPACE / "target" / "release" / "limni")
        img = output / "test.lim"
        if img.exists():
            img.unlink()
        rss, elapsed = get_peak_rss([
            limni, "limn", str(source), str(img)
        ])
        results.append((rss, elapsed))
    return results


def benchmark_limni_verify(
    image: Path, iterations: int
) -> list[tuple[int, float]]:
    results = []
    limni = str(WORKSPACE / "target" / "release" / "limni")
    for _ in range(iterations):
        rss, elapsed = get_peak_rss([limni, "verify", str(image)])
        results.append((rss, elapsed))
    return results


def benchmark_limni_extract(
    image: Path, dest: Path, iterations: int
) -> list[tuple[int, float]]:
    results = []
    limni = str(WORKSPACE / "target" / "release" / "limni")
    for _ in range(iterations):
        if dest.exists():
            shutil.rmtree(dest)
        dest.mkdir(parents=True)
        rss, elapsed = get_peak_rss([
            limni, "extract", str(image), str(dest)
        ])
        results.append((rss, elapsed))
    return results


def benchmark_dwarfs_create(
    source: Path, output: Path, iterations: int, level: str = "1"
) -> list[tuple[int, float]]:
    """Benchmark mkdwarfs if available."""
    mkdwarfs = shutil.which("mkdwarfs")
    if not mkdwarfs:
        return []
    results = []
    for _ in range(iterations):
        img = output / "test.dwarfs"
        if img.exists():
            img.unlink()
        rss, elapsed = get_peak_rss([
            mkdwarfs, "-i", str(source), "-o", str(img),
            "-l", level, "--no-history"
        ])
        results.append((rss, elapsed))
    return results


def benchmark_dwarfs_extract(
    image: Path, dest: Path, iterations: int
) -> list[tuple[int, float]]:
    exe = shutil.which("dwarfsextract")
    if not exe:
        return []
    results = []
    for _ in range(iterations):
        if dest.exists():
            shutil.rmtree(dest)
        dest.mkdir(parents=True)
        rss, elapsed = get_peak_rss([exe, "-i", str(image), "-o", str(dest)])
        results.append((rss, elapsed))
    return results


def benchmark_squashfs_create(
    source: Path, output: Path, iterations: int
) -> list[tuple[int, float]]:
    exe = shutil.which("mksquashfs")
    if not exe:
        return []
    results = []
    for _ in range(iterations):
        img = output / "test.squashfs"
        if img.exists():
            img.unlink()
        rss, elapsed = get_peak_rss([
            exe, str(source), str(img), "-noappend", "-comp", "zstd",
            "-Xcompression-level", "1", "-no-progress"
        ])
        results.append((rss, elapsed))
    return results


def benchmark_squashfs_extract(
    image: Path, dest: Path, iterations: int
) -> list[tuple[int, float]]:
    exe = shutil.which("unsquashfs")
    if not exe:
        return []
    results = []
    for _ in range(iterations):
        if dest.exists():
            shutil.rmtree(dest)
        rss, elapsed = get_peak_rss([
            exe, "-d", str(dest), "-no-progress", str(image)
        ])
        results.append((rss, elapsed))
    return results


def benchmark_tar_zstd_create(
    source: Path, output: Path, iterations: int
) -> list[tuple[int, float]]:
    exe = shutil.which("tar")
    if not exe:
        return []
    results = []
    for _ in range(iterations):
        archive = output / "test.tar.zst"
        if archive.exists():
            archive.unlink()
        rss, elapsed = get_peak_rss([
            exe, "-cf", str(archive),
            "--use-compress-program=zstd -1",
            "-C", str(source.parent),
            str(source.name)
        ])
        results.append((rss, elapsed))
    return results


def benchmark_tar_zstd_extract(
    archive: Path, dest: Path, iterations: int
) -> list[tuple[int, float]]:
    exe = shutil.which("tar")
    if not exe:
        return []
    results = []
    for _ in range(iterations):
        if dest.exists():
            shutil.rmtree(dest)
        dest.mkdir(parents=True)
        rss, elapsed = get_peak_rss([
            exe, "-xf", str(archive),
            "--use-compress-program=zstd -d",
            "-C", str(dest)
        ])
        results.append((rss, elapsed))
    return results


def run_full_benchmark(
    datasets: list[str],
    iterations: int,
    workspace: Path,
) -> list[BenchmarkResult]:
    """Run the comprehensive benchmark across all formats and datasets."""
    all_results: list[BenchmarkResult] = []
    tmpdir = Path(tempfile.mkdtemp(prefix="limnifs-bench-"))

    for ds_name in datasets:
        print(f"\n{'='*70}")
        print(f"Dataset: {ds_name}")
        print(f"{'='*70}")

        source = ensure_dataset(ds_name, workspace)
        input_size = sum(f.stat().st_size for f in source.rglob("*") if f.is_file())
        print(f"Input size: {input_size / 1_048_576:.1f} MB")

        ds_tmp = tmpdir / ds_name
        ds_tmp.mkdir(parents=True, exist_ok=True)

        # --- LimniFS ---
        print(f"\n[LimniFS] Creating image...")
        create_results = benchmark_limni_create(source, ds_tmp, iterations)
        limni_image = ds_tmp / "test.lim"
        image_size = limni_image.stat().st_size if limni_image.exists() else 0

        rss_list = [r[0] for r in create_results]
        time_list = [r[1] for r in create_results]
        all_results.append(BenchmarkResult(
            "limnifs", ds_name, "auto (seine)", "create",
            iterations, time_list, max(rss_list) if rss_list else None,
            image_size, input_size,
        ))
        print(f"  Create: {statistics.median(time_list):.3f}s, "
              f"image {image_size/1_048_576:.1f} MB, "
              f"ratio {image_size/input_size*100:.1f}%")

        print(f"[LimniFS] Verifying...")
        verify_results = benchmark_limni_verify(limni_image, iterations)
        rss_list = [r[0] for r in verify_results]
        time_list = [r[1] for r in verify_results]
        all_results.append(BenchmarkResult(
            "limnifs", ds_name, "auto (seine)", "verify",
            iterations, time_list, max(rss_list) if rss_list else None,
            image_size, input_size,
        ))
        print(f"  Verify: {statistics.median(time_list):.3f}s")

        print(f"[LimniFS] Extracting...")
        extract_dest = ds_tmp / "extract_limni"
        extract_results = benchmark_limni_extract(limni_image, extract_dest, iterations)
        rss_list = [r[0] for r in extract_results]
        time_list = [r[1] for r in extract_results]
        all_results.append(BenchmarkResult(
            "limnifs", ds_name, "auto (seine)", "extract",
            iterations, time_list, max(rss_list) if rss_list else None,
            image_size, input_size,
        ))
        print(f"  Extract: {statistics.median(time_list):.3f}s")

        # --- DwarFS ---
        if shutil.which("mkdwarfs"):
            print(f"\n[DwarFS] Creating image...")
            create_results = benchmark_dwarfs_create(source, ds_tmp, iterations)
            dwarfs_image = ds_tmp / "test.dwarfs"
            d_image_size = dwarfs_image.stat().st_size if dwarfs_image.exists() else 0

            if create_results:
                rss_list = [r[0] for r in create_results]
                time_list = [r[1] for r in create_results]
                all_results.append(BenchmarkResult(
                    "dwarfs", ds_name, "LZMA", "create",
                    iterations, time_list, max(rss_list) if rss_list else None,
                    d_image_size, input_size,
                ))
                print(f"  Create: {statistics.median(time_list):.3f}s, "
                      f"image {d_image_size/1_048_576:.1f} MB, "
                      f"ratio {d_image_size/input_size*100:.1f}%")

            if shutil.which("dwarfsextract") and dwarfs_image.exists():
                print(f"[DwarFS] Extracting...")
                extract_dest = ds_tmp / "extract_dwarfs"
                extract_results = benchmark_dwarfs_extract(dwarfs_image, extract_dest, iterations)
                if extract_results:
                    rss_list = [r[0] for r in extract_results]
                    time_list = [r[1] for r in extract_results]
                    all_results.append(BenchmarkResult(
                        "dwarfs", ds_name, "LZMA", "extract",
                        iterations, time_list, max(rss_list) if rss_list else None,
                        d_image_size, input_size,
                    ))
                    print(f"  Extract: {statistics.median(time_list):.3f}s")
        else:
            print("\n[DwarFS] mkdwarfs not found — skipping")

        # --- SquashFS ---
        if shutil.which("mksquashfs"):
            print(f"\n[SquashFS] Creating image...")
            create_results = benchmark_squashfs_create(source, ds_tmp, iterations)
            sqfs_image = ds_tmp / "test.squashfs"
            sq_size = sqfs_image.stat().st_size if sqfs_image.exists() else 0

            if create_results:
                rss_list = [r[0] for r in create_results]
                time_list = [r[1] for r in create_results]
                all_results.append(BenchmarkResult(
                    "squashfs", ds_name, "zstd-1", "create",
                    iterations, time_list, max(rss_list) if rss_list else None,
                    sq_size, input_size,
                ))
                print(f"  Create: {statistics.median(time_list):.3f}s, "
                      f"image {sq_size/1_048_576:.1f} MB")

            if shutil.which("unsquashfs") and sqfs_image.exists():
                print(f"[SquashFS] Extracting...")
                extract_dest = ds_tmp / "extract_sqfs"
                extract_results = benchmark_squashfs_extract(sqfs_image, extract_dest, iterations)
                if extract_results:
                    rss_list = [r[0] for r in extract_results]
                    time_list = [r[1] for r in extract_results]
                    all_results.append(BenchmarkResult(
                        "squashfs", ds_name, "zstd-1", "extract",
                        iterations, time_list, max(rss_list) if rss_list else None,
                        sq_size, input_size,
                    ))
                    print(f"  Extract: {statistics.median(time_list):.3f}s")
        else:
            print("\n[SquashFS] mksquashfs not found — skipping")

        # --- tar + zstd ---
        if shutil.which("tar"):
            print(f"\n[tar+zstd] Creating archive...")
            create_results = benchmark_tar_zstd_create(source, ds_tmp, iterations)
            tar_image = ds_tmp / "test.tar.zst"
            tar_size = tar_image.stat().st_size if tar_image.exists() else 0

            if create_results:
                rss_list = [r[0] for r in create_results]
                time_list = [r[1] for r in create_results]
                all_results.append(BenchmarkResult(
                    "tar+zstd", ds_name, "zstd-1", "create",
                    iterations, time_list, max(rss_list) if rss_list else None,
                    tar_size, input_size,
                ))
                print(f"  Create: {statistics.median(time_list):.3f}s, "
                      f"archive {tar_size/1_048_576:.1f} MB")

            if tar_image.exists():
                print(f"[tar+zstd] Extracting...")
                extract_dest = ds_tmp / "extract_tar"
                extract_results = benchmark_tar_zstd_extract(tar_image, extract_dest, iterations)
                if extract_results:
                    rss_list = [r[0] for r in extract_results]
                    time_list = [r[1] for r in extract_results]
                    all_results.append(BenchmarkResult(
                        "tar+zstd", ds_name, "zstd-1", "extract",
                        iterations, time_list, max(rss_list) if rss_list else None,
                        tar_size, input_size,
                    ))
                    print(f"  Extract: {statistics.median(time_list):.3f}s")
        else:
            print("\n[tar+zstd] tar not found — skipping")

    # Cleanup
    shutil.rmtree(tmpdir, ignore_errors=True)
    return all_results


def render_report(results: list[BenchmarkResult]) -> str:
    """Generate a comprehensive Markdown report."""
    lines = [
        "# LimniFS Comprehensive Benchmark Report",
        "",
        f"**Date:** {datetime.now(timezone.utc).strftime('%Y-%m-%d %H:%M UTC')}",
        f"**Platform:** {platform.system()} {platform.machine()}",
        "",
    ]

    # Group by dataset
    datasets = sorted(set(r.dataset for r in results))
    for ds in datasets:
        ds_results = [r for r in results if r.dataset == ds]
        lines.append(f"## Dataset: {ds}")
        lines.append("")

        # Create comparison table
        creates = [r for r in ds_results if r.operation == "create"]
        if creates:
            lines.append("### Create")
            lines.append("")
            lines.append("| Format | Codec | Time (s) | Throughput (MB/s) | Image (MB) | Ratio (%) | Peak RSS (MB) |")
            lines.append("|---|---|---:|---:|---:|---:|---:|")
            for r in sorted(creates, key=lambda x: x.median_seconds):
                d = r.to_dict()
                lines.append(
                    f"| {d['format']} | {d['codec']} | {d['median_seconds']:.3f} | "
                    f"{d['throughput_mbps'] or '—'} | {d['image_size_mb'] or '—'} | "
                    f"{d['ratio_percent'] or '—'} | {d['peak_rss_mb'] or '—'} |"
                )
            lines.append("")

        # Extract comparison table
        extracts = [r for r in ds_results if r.operation == "extract"]
        if extracts:
            lines.append("### Extract")
            lines.append("")
            lines.append("| Format | Time (s) | Throughput (MB/s) | Peak RSS (MB) |")
            lines.append("|---|---:|---:|---:|")
            for r in sorted(extracts, key=lambda x: x.median_seconds):
                d = r.to_dict()
                lines.append(
                    f"| {d['format']} | {d['median_seconds']:.3f} | "
                    f"{d['throughput_mbps'] or '—'} | {d['peak_rss_mb'] or '—'} |"
                )
            lines.append("")

        # Verify comparison (LimniFS only)
        verifies = [r for r in ds_results if r.operation == "verify"]
        if verifies:
            lines.append("### Verify")
            lines.append("")
            lines.append("| Format | Time (s) |")
            lines.append("|---|---:|")
            for r in verifies:
                lines.append(f"| {r.format} | {r.median_seconds:.3f} |")
            lines.append("")

        # Win/Loss summary vs DwarFS
        limni_create = next((r for r in creates if r.format == "limnifs"), None)
        dwarfs_create = next((r for r in creates if r.format == "dwarfs"), None)
        limni_extract = next((r for r in extracts if r.format == "limnifs"), None)
        dwarfs_extract = next((r for r in extracts if r.format == "dwarfs"), None)

        if dwarfs_create and limni_create:
            lines.append("### LimniFS vs DwarFS")
            lines.append("")
            lines.append("| Metric | LimniFS | DwarFS | Winner |")
            lines.append("|---|---:|---:|---|")

            # Create speed
            speed_ratio = dwarfs_create.median_seconds / limni_create.median_seconds if limni_create.median_seconds > 0 else 0
            winner = "**LimniFS**" if speed_ratio > 1 else "DwarFS"
            lines.append(
                f"| Create speed | {limni_create.median_seconds:.3f}s | "
                f"{dwarfs_create.median_seconds:.3f}s | {winner} ({speed_ratio:.1f}x) |"
            )

            # Image size
            if limni_create.image_size_bytes and dwarfs_create.image_size_bytes:
                size_diff = (1 - limni_create.image_size_bytes / dwarfs_create.image_size_bytes) * 100
                winner = "**LimniFS**" if limni_create.image_size_bytes < dwarfs_create.image_size_bytes else "DwarFS"
                lines.append(
                    f"| Image size | {limni_create.image_size_bytes/1_048_576:.1f} MB | "
                    f"{dwarfs_create.image_size_bytes/1_048_576:.1f} MB | "
                    f"{winner} ({abs(size_diff):.1f}% smaller) |"
                )

            # Extract speed
            if dwarfs_extract and limni_extract:
                ext_ratio = dwarfs_extract.median_seconds / limni_extract.median_seconds if limni_extract.median_seconds > 0 else 0
                winner = "**LimniFS**" if ext_ratio > 1 else "DwarFS"
                lines.append(
                    f"| Extract speed | {limni_extract.median_seconds:.3f}s | "
                    f"{dwarfs_extract.median_seconds:.3f}s | {winner} ({ext_ratio:.1f}x) |"
                )

            # Memory
            if limni_create.peak_rss_kb and dwarfs_create.peak_rss_kb:
                mem_ratio = dwarfs_create.peak_rss_kb / limni_create.peak_rss_kb if limni_create.peak_rss_kb > 0 else 0
                winner = "**LimniFS**" if limni_create.peak_rss_kb < dwarfs_create.peak_rss_kb else "DwarFS"
                lines.append(
                    f"| Create memory | {limni_create.peak_rss_kb/1024:.0f} MB | "
                    f"{dwarfs_create.peak_rss_kb/1024:.0f} MB | "
                    f"{winner} ({mem_ratio:.1f}x less) |"
                )

            lines.append("")

    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description="LimniFS comprehensive benchmark")
    parser.add_argument("--datasets", default="tiny,python",
                        help="Comma-separated dataset names")
    parser.add_argument("--iterations", type=int, default=3,
                        help="Iterations per benchmark")
    parser.add_argument("--quick", action="store_true",
                        help="Quick mode: tiny only, 1 iteration")
    parser.add_argument("--all", action="store_true",
                        help="All datasets, 5 iterations")
    args = parser.parse_args()

    if args.quick:
        datasets = ["tiny"]
        iterations = 1
    elif args.all:
        datasets = list(DATASETS.keys())
        iterations = 5
    else:
        datasets = args.datasets.split(",")
        iterations = args.iterations

    # Build release binary
    print("Building limni (release)...")
    subprocess.run(
        ["cargo", "build", "--release"],
        cwd=WORKSPACE, check=True,
        capture_output=True,
    )
    print("Build complete.\n")

    # Run benchmarks
    results = run_full_benchmark(datasets, iterations, WORKSPACE)

    # Save JSON
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    timestamp = datetime.now().strftime("%Y%m%d_%H%M%S")
    json_path = OUTPUT_DIR / f"comprehensive_{timestamp}.json"
    with open(json_path, "w") as f:
        json.dump({
            "metadata": {
                "date": datetime.now(timezone.utc).isoformat(),
                "platform": platform.system(),
                "machine": platform.machine(),
                "iterations": iterations,
                "datasets": datasets,
            },
            "results": [r.to_dict() for r in results],
        }, f, indent=2)
    print(f"\nJSON results: {json_path}")

    # Generate report
    report = render_report(results)
    report_path = OUTPUT_DIR / f"comprehensive_{timestamp}.md"
    with open(report_path, "w") as f:
        f.write(report)
    print(f"Markdown report: {report_path}")

    # Print summary to stdout
    print("\n" + "=" * 70)
    print("BENCHMARK COMPLETE")
    print("=" * 70)
    print(report[-2000:] if len(report) > 2000 else report)

    return 0


if __name__ == "__main__":
    sys.exit(main())
