#!/usr/bin/env python3
"""Package an already-built cpyref binary without third-party Python modules."""

import argparse
import datetime
import gzip
import os
from pathlib import Path, PurePosixPath
import re
import stat
import subprocess
import tarfile
import tempfile
import zipfile


ROOT = Path(__file__).resolve().parents[1]
TARGETS = (
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-musl",
    "x86_64-pc-windows-msvc",
    "aarch64-pc-windows-msvc",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
)
VERSION_RE = re.compile(
    r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
    r"(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?"
    r"(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?"
)


def validate_version(value):
    match = VERSION_RE.fullmatch(value)
    if match is None or any(
        part.isdigit() and len(part) > 1 and part.startswith("0")
        for part in (match.group(4) or "").split(".")
    ):
        raise ValueError("version must be SemVer without the leading v (for example 0.1.0)")
    return value


def validate_target(value):
    if value not in TARGETS:
        raise ValueError(f"unsupported target: {value}")
    return value


def regular_file(path):
    if path.is_symlink() or not path.is_file():
        raise ValueError(f"expected a regular file, not a symlink: {path}")
    return path


def release_entries(root, binary, name, windows):
    """Return explicit safe paths; never follow links into other directories."""
    regular_file(binary)
    if binary.stat().st_size == 0:
        raise ValueError(f"binary is empty: {binary}")
    files = [
        (binary, "cpyref.exe" if windows else "cpyref", 0o755),
        (regular_file(root / "LICENSE"), "LICENSE", 0o644),
        (regular_file(root / "README.md"), "README.md", 0o644),
    ]
    docs = root / "docs"
    if docs.is_symlink() or not docs.is_dir():
        raise ValueError(f"expected a documentation directory, not a symlink: {docs}")
    entries = [(None, PurePosixPath(name), 0o755)]
    entries.extend((source, PurePosixPath(name, relative), mode) for source, relative, mode in files)
    entries.append((None, PurePosixPath(name, "docs"), 0o755))
    for source in sorted(docs.rglob("*")):
        if source.is_symlink():
            raise ValueError(f"documentation must not contain symlinks: {source}")
        relative = PurePosixPath(name, source.relative_to(root).as_posix())
        if source.is_dir():
            entries.append((None, relative, 0o755))
        else:
            entries.append((regular_file(source), relative, 0o644))
    return entries


def source_date_epoch():
    try:
        epoch = int(os.environ.get("SOURCE_DATE_EPOCH", "0"))
    except ValueError as error:
        raise ValueError("SOURCE_DATE_EPOCH must be a non-negative integer") from error
    if epoch < 0 or epoch > 0xFFFFFFFF:
        raise ValueError("SOURCE_DATE_EPOCH must fit in an unsigned 32-bit timestamp")
    return epoch


def write_tar(path, entries, epoch):
    with path.open("xb") as output:
        with gzip.GzipFile(filename="", fileobj=output, mode="wb", mtime=epoch) as compressed:
            with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as archive:
                for source, relative, mode in entries:
                    info = tarfile.TarInfo(relative.as_posix())
                    info.mode = mode
                    info.mtime = epoch
                    if source is None:
                        info.type = tarfile.DIRTYPE
                        archive.addfile(info)
                    else:
                        info.size = source.stat().st_size
                        with source.open("rb") as content:
                            archive.addfile(info, content)


def write_zip(path, entries, epoch):
    # ZIP cannot represent dates before 1980. Every entry uses the same UTC time.
    timestamp = datetime.datetime.fromtimestamp(max(epoch, 315532800), datetime.timezone.utc)
    date_time = (timestamp.year, timestamp.month, timestamp.day, timestamp.hour, timestamp.minute, timestamp.second)
    with zipfile.ZipFile(path, "x", compression=zipfile.ZIP_DEFLATED) as archive:
        for source, relative, mode in entries:
            name = relative.as_posix() + ("/" if source is None else "")
            info = zipfile.ZipInfo(name, date_time=date_time)
            info.create_system = 3
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = ((stat.S_IFDIR if source is None else stat.S_IFREG) | mode) << 16
            if source is None:
                info.external_attr |= 0x10
                archive.writestr(info, b"")
            else:
                with source.open("rb") as content, archive.open(info, "w") as output:
                    while chunk := content.read(1024 * 1024):
                        output.write(chunk)


def create_archive(root, binary, target, version, out_dir):
    validate_target(target)
    validate_version(version)
    windows = "windows" in target
    name = f"cpyref-{version}-{target}"
    entries = release_entries(root, binary, name, windows)
    epoch = source_date_epoch()
    destination = out_dir / (name + (".zip" if windows else ".tar.gz"))
    out_dir.mkdir(parents=True, exist_ok=True)
    # A hard link publishes the completed archive atomically and refuses to
    # overwrite even if another process creates the destination concurrently.
    with tempfile.TemporaryDirectory(prefix=".cpyref-package-", dir=out_dir) as temporary:
        staged = Path(temporary) / destination.name
        (write_zip if windows else write_tar)(staged, entries, epoch)
        os.link(staged, destination)
    return destination


def create_linux_packages(root, binary, target, version, out_dir, nfpm):
    validate_target(target)
    validate_version(version)
    if not target.endswith("-unknown-linux-musl"):
        raise ValueError("--linux-packages requires a Linux musl target")
    # Validate the same source tree used by the portable archive before nFPM's
    # tree traversal. The workflow verifies that the musl binary is static.
    release_entries(root, binary, "cpyref", False)
    environment = os.environ.copy()
    environment.update(
        CPYREF_VERSION=version,
        CPYREF_ARCH="amd64" if target.startswith("x86_64-") else "arm64",
        CPYREF_BINARY=str(binary.resolve()),
        CPYREF_LICENSE=str((root / "LICENSE").resolve()),
        CPYREF_README=str((root / "README.md").resolve()),
        CPYREF_DOCS=str((root / "docs").resolve()),
    )
    formats = (("deb", ".deb"), ("rpm", ".rpm"), ("archlinux", ".pkg.tar.zst"))
    destinations = [out_dir / f"cpyref-{version}-{target}{suffix}" for _, suffix in formats]
    for destination in destinations:
        if destination.exists() or destination.is_symlink():
            raise FileExistsError(f"refusing to overwrite existing package: {destination}")
    out_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".cpyref-nfpm-", dir=out_dir) as temporary:
        staged = []
        for (packager, _), destination in zip(formats, destinations):
            path = Path(temporary) / destination.name
            subprocess.run(
                [nfpm, "package", "--config", str(root / "packaging" / "nfpm.yaml"),
                 "--packager", packager, "--target", str(path)],
                check=True, env=environment,
            )
            regular_file(path)
            staged.append(path)
        for path, destination in zip(staged, destinations):
            os.link(path, destination)
    return destinations


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True, choices=TARGETS)
    parser.add_argument("--version", required=True)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--out-dir", required=True, type=Path)
    parser.add_argument("--linux-packages", action="store_true", help="also create deb/rpm/Arch Linux packages")
    parser.add_argument("--nfpm", default="nfpm", help="path to the nFPM executable")
    args = parser.parse_args()
    try:
        validate_version(args.version)
        if args.linux_packages and not args.target.endswith("-unknown-linux-musl"):
            raise ValueError("--linux-packages requires a Linux musl target")
        outputs = [create_archive(ROOT, args.binary, args.target, args.version, args.out_dir)]
        if args.linux_packages:
            outputs.extend(create_linux_packages(ROOT, args.binary, args.target, args.version, args.out_dir, args.nfpm))
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"packaging failed: {error}\n")
    for output in outputs:
        print(output)


if __name__ == "__main__":
    main()
