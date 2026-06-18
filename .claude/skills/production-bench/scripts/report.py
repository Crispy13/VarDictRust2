#!/usr/bin/env python3
"""report.py — render the production-bench REPORT.md.

Reads /usr/bin/time -v logs ($root/time/<cell>.<tool>.run<i>.time) and the captured TSVs
($root/out/<cell>.<tool>.tsv), computes median resource usage across the timed runs, checks
vdj-vs-vdr output parity (byte-identical; sorted-identical fallback), and writes a per-cell
table with Java/Rust/ratio plus a parity verdict. Parity FAIL marks the cell INVALID — a
perf number is meaningless if the two tools disagree on output.
"""
import argparse, hashlib, os, re, subprocess, sys
from statistics import median

def parse_time_v(path):
    wall = user = sysc = rss = None
    with open(path) as fh:
        for line in fh:
            line = line.strip()
            if line.startswith("Elapsed (wall clock) time"):
                v = line.split(": ", 1)[1]
                parts = [float(x) for x in v.split(":")]
                wall = parts[0]*3600 + parts[1]*60 + parts[2] if len(parts) == 3 else parts[0]*60 + parts[1]
            elif line.startswith("User time (seconds):"):
                user = float(line.split(": ", 1)[1])
            elif line.startswith("System time (seconds):"):
                sysc = float(line.split(": ", 1)[1])
            elif line.startswith("Maximum resident set size (kbytes):"):
                rss = float(line.split(": ", 1)[1]) / 1024.0  # -> MB
    return wall, user, sysc, rss

def medians(root, cell, tool, runs):
    vals = {"wall": [], "user": [], "sys": [], "rss": []}
    for i in range(1, runs + 1):  # run0 is warmup, not timed
        p = os.path.join(root, "time", f"{cell}.{tool}.run{i}.time")
        if not os.path.isfile(p):
            continue
        w, u, s, r = parse_time_v(p)
        for k, v in zip(("wall", "user", "sys", "rss"), (w, u, s, r)):
            if v is not None:
                vals[k].append(v)
    return {k: (median(v) if v else None) for k, v in vals.items()}

def md5(path):
    h = hashlib.md5()
    with open(path, "rb") as fh:
        for b in iter(lambda: fh.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest()

def parity(root, cell):
    vdj = os.path.join(root, "out", f"{cell}.vdj.tsv")
    vdr = os.path.join(root, "out", f"{cell}.vdr.tsv")
    if os.path.exists(os.path.join(root, "out", f"{cell}.vdr.UNSUPPORTED")):
        return "N/A (vdr unsupported)", False
    if not (os.path.isfile(vdj) and os.path.isfile(vdr)):
        return "MISSING output", True
    if md5(vdj) == md5(vdr):
        return "IDENTICAL", False
    # sorted fallback (thread-order tolerant)
    def sorted_md5(p):
        out = subprocess.run(["sort", p], capture_output=True)
        return hashlib.md5(out.stdout).hexdigest()
    if sorted_md5(vdj) == sorted_md5(vdr):
        return "identical (sorted)", False
    d = subprocess.run(["bash", "-c", f"diff <(sort {vdj!r}) <(sort {vdr!r}) | grep -c '^[<>]'"],
                       capture_output=True, text=True)
    return f"DIFFER ({d.stdout.strip()} lines)", True

def fmt(x, unit=""):
    return "—" if x is None else (f"{x:.0f}{unit}" if unit == "MB" else f"{x:.1f}{unit}")

def ratio(r, j):
    return "—" if (r is None or j in (None, 0)) else f"{r/j:.2f}x"

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", required=True)
    ap.add_argument("--runs", type=int, required=True)
    ap.add_argument("--workloads", required=True)
    ap.add_argument("--presets", required=True)
    a = ap.parse_args()

    rows, any_invalid = [], False
    for wl in a.workloads.split():
        for ps in a.presets.split():
            cell = f"{wl}.{ps}"
            if not (os.path.isfile(os.path.join(a.root, "out", f"{cell}.vdj.tsv"))
                    or os.path.exists(os.path.join(a.root, "out", f"{cell}.vdr.UNSUPPORTED"))):
                continue
            j = medians(a.root, cell, "vdj", a.runs)
            r = medians(a.root, cell, "vdr", a.runs)
            par, bad = parity(a.root, cell)
            if bad:
                any_invalid = True
            rows.append((wl, ps, j, r, par))

    lines = ["# Production bench — VarDictJava (vdj) vs VarDict-rs (vdr)", ""]
    up = os.path.join(a.root, "uptime.txt")
    if os.path.isfile(up):
        load = open(up).read().strip()
        lines += [f"Machine load at start: `{load}`"]
        if re.search(r"load average:\s*([\d.]+)", load) and \
           float(re.search(r"load average:\s*([\d.]+)", load).group(1)) > 2.0:
            lines += ["", "> ⚠️ loadavg > 2 at start — **idle re-measure owed** before trusting timing."]
    lines += ["", f"Runs timed per cell: {a.runs} (median; 1 warmup discarded). Thread count: -th 8.", ""]
    lines += ["| Workload | Preset | Metric | Java (vdj) | Rust (vdr) | Rust/Java | Parity |",
              "|---|---|---|---:|---:|---:|---|"]
    for wl, ps, j, r, par in rows:
        for k, label, unit in (("wall", "Wall s", "s"), ("user", "User s", "s"),
                               ("sys", "Sys s", "s"), ("rss", "Peak RSS", "MB")):
            pcell = par if k == "wall" else ""
            lines.append(f"| {wl if k=='wall' else ''} | {ps if k=='wall' else ''} | {label} "
                         f"| {fmt(j[k], unit)} | {fmt(r[k], unit)} | {ratio(r[k], j[k])} | {pcell} |")
    lines += ["", "**Verdict:** " + ("⚠️ one or more cells INVALID (parity mismatch / missing) — "
              "do not trust their perf numbers." if any_invalid else
              "✅ all measured cells byte-identical; resource numbers are valid.")]
    lines += ["", "Ratio < 1.00x means Rust is faster / leaner than Java.", ""]

    out = os.path.join(a.root, "REPORT.md")
    open(out, "w").write("\n".join(lines) + "\n")
    print("\n".join(lines))
    print(f"\nwrote {out}")

if __name__ == "__main__":
    main()
