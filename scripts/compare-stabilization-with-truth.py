"""Compares the camera path measured by
`cargo run --release -p kadr-engine --example stabilize_video` on the benchmark
video of `scripts/generate-stabilization-benchmark.ps1` with the true motion of
its camera, and checks what the corrections leave of the shake.

The camera rolls the 2560 x 1440 picture clockwise about its centre, then looks
through a 1920 x 1080 window whose corner FFmpeg's crop filter rounds to the
nearest pixel (`lrint`, halves to the even one, like Python's `round`):
    roll = 0.5 sin(2 pi 1.3 t) + 0.25 sin(2 pi 3.7 t + 1) degrees
    left = round(320 + 200 sin(2 pi t / 20)
                 + 10 sin(2 pi 2.3 t) + 6 sin(2 pi 5.1 t + 1) + 3 sin(2 pi 9.7 t + 2))
    top  = round(180 + 8 sin(2 pi 1.7 t + 0.5) + 5 sin(2 pi 4.3 t) + 2 sin(2 pi 8.9 t + 1))
with t in seconds from the start of each shot (shot B starts at frame 1200).

Positions are complex numbers (x + iy, y down) relative to the frame's centre.
A pose is (m, T): a point z of the shot's first frame is seen at m z + T.

Usage: python scripts/compare-stabilization-with-truth.py path.csv
"""

import cmath
import csv
import math
import sys

FRAMES_PER_SECOND = 30
FIRST_FRAME_OF_SHOT_B = 1200
STILL_CENTER = complex(1280, 720)
FRAME_CENTER = complex(960, 540)
# Where the motion is compared: the frame's centre and its four corners.
CHECK_POINTS = [0j, complex(-960, -540), complex(960, -540), complex(-960, 540), complex(960, 540)]
HIGH_PASS_SIGMA_IN_SECONDS = 0.25


def camera(time_in_seconds):
    """The roll (radians, clockwise) and the window's corner at a moment of a shot."""
    t, two_pi = time_in_seconds, 2 * math.pi
    roll = math.radians(0.5 * math.sin(two_pi * 1.3 * t) + 0.25 * math.sin(two_pi * 3.7 * t + 1))
    left = round(
        320 + 200 * math.sin(two_pi * t / 20) + 10 * math.sin(two_pi * 2.3 * t)
        + 6 * math.sin(two_pi * 5.1 * t + 1) + 3 * math.sin(two_pi * 9.7 * t + 2)
    )
    top = round(
        180 + 8 * math.sin(two_pi * 1.7 * t + 0.5) + 5 * math.sin(two_pi * 4.3 * t)
        + 2 * math.sin(two_pi * 8.9 * t + 1)
    )
    return roll, complex(left, top)


def true_pose(frame_index):
    """Where the content of the shot's first frame is seen at `frame_index`."""
    first = FIRST_FRAME_OF_SHOT_B if frame_index >= FIRST_FRAME_OF_SHOT_B else 0
    first_roll, first_corner = camera(0.0)
    roll, corner = camera((frame_index - first) / FRAMES_PER_SECOND)
    turn = cmath.rect(1.0, roll - first_roll)
    shift = STILL_CENTER - corner - FRAME_CENTER + turn * (FRAME_CENTER - STILL_CENTER + first_corner)
    return turn, shift


def measured_pose(row):
    turn = cmath.rect(float(row["path_scale"]), math.radians(float(row["path_rotation"])))
    return turn, complex(float(row["path_x"]), float(row["path_y"]))


def correction_of(row):
    turn = cmath.rect(float(row["correction_scale"]), math.radians(float(row["correction_rotation"])))
    return turn, complex(float(row["correction_x"]), float(row["correction_y"]))


def step_between(previous, current):
    """The motion from one pose to the next, as (m, T)."""
    turn = current[0] / previous[0]
    return turn, current[1] - turn * previous[1]


def percentile(values, fraction):
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int(fraction * len(ordered)))]


def high_passed(positions):
    """`positions` minus their own Gaussian average over time.

    Past the ends, the positions are continued by point reflection about the end
    one, so that a steady pan is not counted as shake there.
    """
    sigma = HIGH_PASS_SIGMA_IN_SECONDS * FRAMES_PER_SECOND
    reach = int(3 * sigma)
    last = len(positions) - 1

    def position_at(index):
        if index < 0:
            return 2 * positions[0] - positions[min(last, -index)]
        if index > last:
            return 2 * positions[last] - positions[max(0, 2 * last - index)]
        return positions[index]

    result = []
    for index, position in enumerate(positions):
        total, weight_sum = 0j, 0.0
        for offset in range(-reach, reach + 1):
            weight = math.exp(-(offset ** 2) / (2 * sigma * sigma))
            total += weight * position_at(index + offset)
            weight_sum += weight
        result.append(position - total / weight_sum)
    return result


def frame_ranges(frame_indices):
    """Frame numbers as text, runs of consecutive ones joined: "161, 601-612"."""
    ranges = []
    for index in frame_indices:
        if ranges and index == ranges[-1][1] + 1:
            ranges[-1][1] = index
        else:
            ranges.append([index, index])
    return ", ".join(str(first) if first == last else f"{first}-{last}" for first, last in ranges) or "none"


def main(csv_path):
    with open(csv_path, encoding="utf-8") as csv_file:
        rows = list(csv.DictReader(csv_file))
    frame_count = len(rows)
    measured = [measured_pose(row) for row in rows]
    truth = [true_pose(index) for index in range(frame_count)]

    # Checks 1 to 3: the motion from each frame to the next.
    step_errors, roll_errors, unmeasured = [], [], []
    for index in range(1, frame_count):
        if rows[index]["motion_was_measured"] != "1":
            unmeasured.append(index)
            continue
        if index == FIRST_FRAME_OF_SHOT_B:
            continue
        measured_step = step_between(measured[index - 1], measured[index])
        true_step = step_between(truth[index - 1], truth[index])
        for point in CHECK_POINTS:
            step_errors.append(abs(
                (measured_step[0] * point + measured_step[1]) - (true_step[0] * point + true_step[1])
            ))
        roll_errors.append(abs(math.degrees(cmath.phase(measured_step[0] / true_step[0]))))
    others = [index for index in unmeasured if abs(index - FIRST_FRAME_OF_SHOT_B) > 1]
    cut_was_seen = any(abs(index - FIRST_FRAME_OF_SHOT_B) <= 1 for index in unmeasured)

    # Checks 4 and 5: what is left of the shake once the corrections are applied
    # to the true motion, shot by shot.
    shots = [range(0, min(FIRST_FRAME_OF_SHOT_B, frame_count)), range(FIRST_FRAME_OF_SHOT_B, frame_count)]
    shake_before, shake_after, largest_jump = [], [], 0.0
    for shot in shots:
        for point in CHECK_POINTS:
            filmed = [truth[index][0] * point + truth[index][1] for index in shot]
            steadied = []
            for index, position in zip(shot, filmed):
                turn, shift = correction_of(rows[index])
                steadied.append(turn * position + shift)
            shake_before += [abs(value) for value in high_passed(filmed)]
            left_over = high_passed(steadied)
            shake_after += [abs(value) for value in left_over]
            for previous, current in zip(left_over, left_over[1:]):
                largest_jump = max(largest_jump, abs(current - previous))

    def root_mean_square(values):
        return math.sqrt(sum(value * value for value in values) / max(1, len(values)))

    shake_before_rms, shake_after_rms = root_mean_square(shake_before), root_mean_square(shake_after)
    zoom = max(float(row["zoom"]) for row in rows)
    partly_corrected = sum(1 for row in rows if float(row["fraction_applied"]) < 1.0)

    mean_step_error = sum(step_errors) / max(1, len(step_errors))
    checks = [
        ("1. step error, mean (px)", mean_step_error, 0.2),
        ("1. step error, 95th percentile (px)", percentile(step_errors, 0.95), 0.5),
        ("2. roll error, 95th percentile (deg)", percentile(roll_errors, 0.95), 0.02),
        ("3. frames not measured, apart from the cut", len(others), 3),
        ("4. shake left, RMS (px)", shake_after_rms, 0.4),
        ("4. shake left / shake filmed", shake_after_rms / max(1e-9, shake_before_rms), 0.05),
        ("5. largest jump of the steadied picture (px)", largest_jump, 1.5),
        ("6. enlargement", zoom, 1.08),
        ("6. frames only partly corrected", partly_corrected, 0),
    ]
    print(f"{frame_count} frames; shake filmed: {shake_before_rms:.2f} px RMS")
    failed = not cut_was_seen
    print(f"{'ok  ' if cut_was_seen else 'FAIL'} 3. the cut at frame {FIRST_FRAME_OF_SHOT_B} is seen "
          f"(frames not measured: {frame_ranges(unmeasured)})")
    for name, value, limit in checks:
        passed = value <= limit
        failed = failed or not passed
        print(f"{'ok  ' if passed else 'FAIL'} {name}: {value:.4g} (at most {limit})")
    return 1 if failed else 0


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    sys.exit(main(sys.argv[1]))
