#!/usr/bin/env python3
import argparse
import os
import subprocess
import sys
import time

KB = 1024
MB = 1024 * KB
GB = 1024 * MB
TB = 1024 * GB


def format_size(num):
    size = float(num)
    for unit in ["B", "KB", "MB", "GB", "TB", "PB"]:
        if size < 1024.0:
            if unit == "B":
                return f"{int(size)} {unit}"
            return f"{size:.1f} {unit}"
        size /= 1024.0
    return f"{size:.1f} EB"


def format_age(ts):
    if ts is None:
        return "unknown"
    now = time.time()
    if ts > now:
        return "0d"
    age = int(now - ts)
    days = age // 86400
    if days > 0:
        return f"{days}d"
    hours = age // 3600
    if hours > 0:
        return f"{hours}h"
    minutes = age // 60
    return f"{minutes}m"


def safe_stat(path):
    try:
        st = os.stat(path)
        return st
    except OSError:
        return None


def dir_size(path, max_entries=None):
    total = 0
    latest_mtime = None
    scanned = 0
    errors = 0
    for root, dirs, files in os.walk(path, topdown=True, followlinks=False):
        for name in files:
            full = os.path.join(root, name)
            st = safe_stat(full)
            if st is None:
                errors += 1
                continue
            total += st.st_size
            if latest_mtime is None or st.st_mtime > latest_mtime:
                latest_mtime = st.st_mtime
            scanned += 1
            if max_entries and scanned >= max_entries:
                return total, latest_mtime, scanned, errors
    return total, latest_mtime, scanned, errors


def report_path(path, max_entries=None):
    if not os.path.exists(path):
        return None
    size, latest, scanned, errors = dir_size(path, max_entries=max_entries)
    return {
        "path": path,
        "size": size,
        "latest": latest,
        "scanned": scanned,
        "errors": errors,
    }


def run_move(root, min_size, no_claude, include_system):
    cmd = ["cargo", "run", "-p", "move", "--", "suggest", "--root", root, "--min-size", min_size]
    if no_claude:
        cmd.append("--no-claude")
    if include_system:
        cmd.append("--include-system")
    return subprocess.call(cmd)


def parse_size(text):
    value = "".join(ch for ch in text if (ch.isdigit() or ch == ".")).strip()
    unit = "".join(ch for ch in text if ch.isalpha()).strip().lower()
    if not value:
        raise ValueError(f"Invalid size: {text}")
    size = float(value)
    if unit in ("", "b"):
        mult = 1
    elif unit in ("k", "kb", "kib"):
        mult = KB
    elif unit in ("m", "mb", "mib"):
        mult = MB
    elif unit in ("g", "gb", "gib"):
        mult = GB
    elif unit in ("t", "tb", "tib"):
        mult = TB
    else:
        raise ValueError(f"Unknown size unit: {unit}")
    return int(size * mult)


def scan_move_candidates(min_size, min_age_days):
    home = os.path.expanduser("~")
    roots = [
        os.path.join(home, "Desktop"),
        os.path.join(home, "Documents"),
        os.path.join(home, "Downloads"),
        os.path.join(home, "Movies"),
        os.path.join(home, "Pictures"),
        os.path.join(home, "Music"),
    ]
    cutoff = time.time() - (min_age_days * 86400)
    candidates = []
    for root in roots:
        if not os.path.exists(root):
            continue
        for dirpath, dirnames, filenames in os.walk(root, topdown=True, followlinks=False):
            if "/.Trash" in dirpath or "/.git" in dirpath:
                continue
            for name in filenames:
                full = os.path.join(dirpath, name)
                st = safe_stat(full)
                if st is None:
                    continue
                if st.st_size < min_size:
                    continue
                if st.st_mtime > cutoff:
                    continue
                candidates.append(
                    {
                        "path": full,
                        "size": st.st_size,
                        "mtime": st.st_mtime,
                    }
                )
    candidates.sort(key=lambda x: x["size"], reverse=True)
    return candidates


def write_move_candidates(candidates, output_path):
    os.makedirs(os.path.dirname(output_path), exist_ok=True)
    with open(output_path, "w", encoding="utf-8") as f:
        f.write("path,size_bytes,age\n")
        for item in candidates:
            f.write(f"{item['path']},{item['size']},{format_age(item['mtime'])}\n")


def write_delete_candidates(candidates, output_path):
    os.makedirs(os.path.dirname(output_path), exist_ok=True)
    with open(output_path, "w", encoding="utf-8") as f:
        f.write("path,size_bytes,age\n")
        for item in candidates:
            f.write(f"{item['path']},{item['size']},{format_age(item['latest'])}\n")


def main():
    parser = argparse.ArgumentParser(description="Cleanup helper (Xcode + large files).")
    parser.add_argument("--root", default=os.path.expanduser("~"), help="Root to scan (default: ~)")
    parser.add_argument("--min-size", default="200MB", help="Minimum size for move CLI (default: 200MB)")
    parser.add_argument(
        "--min-move-size",
        default="500MB",
        help="Minimum size for move candidates (default: 500MB)",
    )
    parser.add_argument(
        "--min-age-days",
        type=int,
        default=30,
        help="Minimum age in days for move candidates (default: 30)",
    )
    parser.add_argument(
        "--move-output",
        default="scripts/move_candidates.csv",
        help="Where to write move candidates CSV",
    )
    parser.add_argument(
        "--delete-output",
        default="scripts/delete_candidates.csv",
        help="Where to write delete candidates CSV",
    )
    parser.add_argument("--no-claude", action="store_true", help="Skip Claude call in move CLI")
    parser.add_argument("--include-system", action="store_true", help="Include system paths if root is /")
    parser.add_argument("--xcode-only", action="store_true", help="Only print Xcode-related cleanup info")
    parser.add_argument("--run-move", action="store_true", help="Invoke move CLI after Xcode report")
    parser.add_argument(
        "--scan-move",
        action="store_true",
        help="Scan for safe-to-move user files and write CSV",
    )

    args = parser.parse_args()

    home = os.path.expanduser("~")
    xcode_paths = [
        os.path.join(home, "Library/Developer/Xcode/DerivedData"),
        os.path.join(home, "Library/Developer/Xcode/Archives"),
        os.path.join(home, "Library/Developer/CoreSimulator/Devices"),
        os.path.join(home, "Library/Developer/Xcode/iOS DeviceSupport"),
    ]

    delete_candidates = []
    print("Xcode cleanup targets:")
    any_found = False
    for path in xcode_paths:
        info = report_path(path)
        if info is None:
            continue
        any_found = True
        delete_candidates.append(info)
        print(
            f"- {format_size(info['size']):>10}  {format_age(info['latest']):>6}  {info['path']}"
        )
    if not any_found:
        print("- (none found)")

    print("\nSuggested safe cleanup commands (manual):")
    print("- rm -rf ~/Library/Developer/Xcode/DerivedData")
    print("- rm -rf ~/Library/Developer/Xcode/Archives")
    print("- rm -rf ~/Library/Developer/CoreSimulator/Devices")
    print("- rm -rf ~/Library/Developer/Xcode/iOS\\ DeviceSupport")

    if delete_candidates:
        write_delete_candidates(delete_candidates, args.delete_output)
        print(f"\nDelete candidates written to: {args.delete_output}")

    if args.xcode_only:
        return 0

    if args.scan_move:
        move_candidates = scan_move_candidates(
            min_size=parse_size(args.min_move_size),
            min_age_days=args.min_age_days,
        )
        write_move_candidates(move_candidates, args.move_output)
        print(f"\nMove candidates written to: {args.move_output}")

    if args.run_move:
        print("\nRunning move CLI scan...")
        return run_move(args.root, args.min_size, args.no_claude, args.include_system)

    print("\nTip: run the move CLI for a full disk scan:")
    print(f"  cargo run -p move -- suggest --root {args.root} --min-size {args.min_size}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
