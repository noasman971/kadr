"""Compares a track made by `cargo run -p kadr-engine --example track_video` on
the benchmark video of `scripts/generate-tracking-benchmark.ps1` with the true
trajectory of its moving box.

The box is 200 x 240 pixels and FFmpeg's overlay filter places it at
    left = floor(860 + 500 sin(2 pi t / 10))
    top  = floor(420 + 200 sin(2 pi t / 7))

Usage: python scripts/compare-tracking-with-truth.py track.csv
"""

import csv
import math
import sys

BOX_WIDTH, BOX_HEIGHT = 200, 240


def true_center(time_in_seconds):
    left = math.floor(860 + 500 * math.sin(2 * math.pi * time_in_seconds / 10))
    top = math.floor(420 + 200 * math.sin(2 * math.pi * time_in_seconds / 7))
    return left + BOX_WIDTH / 2, top + BOX_HEIGHT / 2


def main(track_path):
    with open(track_path, encoding="utf-8") as track_file:
        rows = list(csv.DictReader(track_file))
    center_errors, size_errors, state_counts = [], [], {}
    worst_error, worst_time = 0.0, 0.0
    for row in rows:
        time = float(row["time_in_seconds"])
        left, top = float(row["left"]), float(row["top"])
        width, height = float(row["width"]), float(row["height"])
        truth_x, truth_y = true_center(time)
        error = math.hypot(left + width / 2 - truth_x, top + height / 2 - truth_y)
        center_errors.append(error)
        size_errors.append(abs(width / BOX_WIDTH - 1))
        state_counts[row["confidence"]] = state_counts.get(row["confidence"], 0) + 1
        if error > worst_error:
            worst_error, worst_time = error, time
    ordered = sorted(center_errors)
    print(f"{len(rows)} frames, states {state_counts}")
    print(
        f"centre error: mean {sum(center_errors) / len(center_errors):.2f} px, "
        f"median {ordered[len(ordered) // 2]:.2f} px, "
        f"95 % {ordered[max(0, int(0.95 * len(ordered)) - 1)]:.2f} px, "
        f"max {worst_error:.2f} px at {worst_time:.2f} s"
    )
    print(
        f"size error: mean {100 * sum(size_errors) / len(size_errors):.1f} %, "
        f"max {100 * max(size_errors):.1f} %"
    )
    last_second = int(float(rows[-1]["time_in_seconds"]))
    for second in range(0, last_second + 1, 5):
        index = min(range(len(rows)), key=lambda i: abs(float(rows[i]["time_in_seconds"]) - second))
        print(
            f"  t={second:2d} s  error {center_errors[index]:7.2f} px  "
            f"width {float(rows[index]['width']):6.1f}  {rows[index]['confidence']}"
        )


if __name__ == "__main__":
    main(sys.argv[1])
