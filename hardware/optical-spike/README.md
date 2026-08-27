# Disposable optical and stand spike

This directory defines the reproducible bench gate for FoldScan issue #1. It does not contain a measured FoldScan corpus yet. The committed example manifest is metadata-only and is accepted by the validator only with `--allow-template`.

## Safety and evidence boundary

- Use a certified, current-limited USB 5 V SELV supply only.
- Do not connect a battery to the XIAO battery pads. Do not add mains wiring, UV/laser light, or an exposed high-power lamp.
- Keep the spike attended. Stop for odor, swelling, unstable connectors, repeated brownout, unexpected current rise, or any user-touchable surface approaching 45 °C.
- Use only non-sensitive printed targets and project-owned test text. Do not publish personal documents.
- Label outputs as **bench test**. The test is not custom-hardware validation, field testing, archival qualification, or a legal/medical/accessibility guarantee.

## Required off-the-shelf equipment

Record exact manufacturer/MPN or identifying markings in the manifest:

1. Seeed Studio XIAO ESP32-S3 Sense, SKU 113991115, with included expansion board, camera, antenna, and heatsink.
2. Known-good USB-C data cable and certified/current-limited 5 V supply; use a USB power meter or bench supply with current logging where available.
3. A 32 GB-or-smaller microSD card formatted FAT32.
4. Temporary rigid base and vertical arm with positive stops; clamps are preferred to improvised ballast.
5. Camera mount that permits measured height and focus adjustment without loading the FPC.
6. Two matched visible-white low-voltage lamps with diffusers. Power them independently during the first optical captures; do not drive them from a GPIO or the module 3.3 V rail.
7. Steel rule or calipers, square, plumb line or digital level, 0.5 mm-grid witness scale, timer/current logger, and contact thermometer or thermocouple.
8. Printed `test-target.svg` at 100% scale on matte paper; verify its 100 mm ruler before using it.

## Receipt and revision inspection

Before powering the module:

1. Photograph both sides of the main board, expansion board, camera flex, sensor/lens markings, antenna, and package label.
2. Record bundle SKU, main-board revision, expansion-board revision, camera sensor marking, lens marking, and any date/lot code.
3. Do not infer OV3660 from the SKU. Seeed's product datasheet says OV2640 or OV3660 units may ship during transition.
4. Verify the antenna is attached before enabling radio. Keep radio disabled for the baseline capture test.
5. Inspect the B2B and FPC connectors under magnification. Stop if misaligned or damaged.

## Temporary fixture

- Support both A4 (210×297 mm) and US-Letter (215.9×279.4 mm) with at least 8 mm visible alignment margin per side.
- Put the page plane on a matte, flat, contrasting base. Use removable corner guides outside the final crop.
- Locate the optical axis at page center with the camera plane parallel to the page. Record camera-to-page height, arm span, base footprint, camera-head mass, material, joint type, and clamp/ballast.
- Route USB and light cables away from the hinge and antenna. Add strain relief so cable motion cannot move the camera head.
- Mark a fixed witness point on the head and a 0.5 mm scale at the page plane for repeatability/deflection readings.

No single prescribed height is claimed because the shipped lens field of view is not documented. Raise the head until the full page plus margins fits, then record the measured height.

## Capture matrix

Use the sensor's maximum reliable JPEG frame and preserve bytes exactly as written. Do not upscale. Lock or record exposure, gain, white balance, quality, and focus procedure.

Capture at least the following for **both A4 and US-Letter**:

| Condition | Minimum captures | Purpose |
|---|---:|---|
| Ambient only | 3 | Baseline focus/exposure and shadow sensitivity |
| Symmetric lights, matte target | 10 | Primary repeatability, focus, distortion, latency, and size statistics |
| Left light only / right light only | 3 each | Shadow and asymmetry diagnosis |
| Symmetric lights, glossy blank/sample | 3 | Glare hotspot characterization; no private document |
| Repeated unfold/refold | 1 after each of 10 cycles | Optical-axis repeatability |
| Ten minutes deployed | 1 before and 1 after | Vertical deflection/focus drift |

Retain failed and outlier captures. Mark them in metadata; do not silently discard them.

## Measurements

### Framing and density

1. Verify all four page edges and 8 mm margins are visible.
2. Record the usable crop width/height in pixels after excluding margin.
3. Compute horizontal and vertical px/mm from known target distances at center and all four corners.
4. Report minimum, median, and maximum. Never substitute sensor resolution divided by page size for measured density.

### Focus and readable print

1. Use the target's center and four corner groups.
2. For every location, record the smallest text size read correctly by two independent readings across three captures.
3. Record the highest line-pair group whose lines remain distinct in both orientations.
4. Keep subjective readable-print observations separate from OCR metrics.

### Distortion

- Fit straight lines to the target border/grid or use a documented image-analysis script.
- Report maximum edge displacement in pixels and millimetres relative to a straight chord before correction.
- Preserve the uncorrected original and the exact processing recipe.

### Glare and shadow

- Record saturated-pixel fraction and location for matte and glossy samples.
- Compare left-only, right-only, ambient, and symmetric-light captures.
- Record PWM frequency/duty if used. Watch for rolling bands at multiple exposure settings.

### Latency and file size

- Use at least 30 captures for the primary lighting condition.
- Measure button/request timestamp to durable final-file completion, not merely sensor exposure.
- Report median, p95, maximum, failures, and retry count.
- Report minimum/median/p95/maximum JPEG bytes and the exact sensor/frame/quality settings.

### Mechanical repeatability and deflection

1. Record the optical-axis witness displacement at the page plane after each of ten unfold/refold cycles; report maximum radial displacement.
2. With the stand deployed and cables strain-relieved, record vertical witness height immediately and after ten minutes; report displacement and ambient temperature.
3. Record whether the fixture is clamped, ballasted, or free-standing and any movement during button operation.

### Power and thermal

- Log module current for idle, capture, microSD write, and USB transfer.
- Measure illumination separately, then combined, including startup/capture transients.
- Record the supply/cable, current-meter bandwidth/sample rate, ambient temperature, and test duration.
- Record module/heatsink, light/diffuser, cable connector, and user-touchable surface temperatures after a documented steady load.
- Do not use Seeed's approximate 347 mA capture figure as a measured FoldScan value.

## Corpus layout

```text
hardware/optical-spike/samples/<run-id>/
  manifest.json
  raw/
    <capture-id>.jpg
  measurements/
    current.csv
    temperature.csv
    mechanical.csv
  photos/
    fixture-overview.jpg
    revision-markings.jpg
```

Large raw images may be attached to a GitHub release or durable artifact store instead of normal Git history, but the repository must retain the validated manifest, checksums, stable artifact URL, license/consent statement, and measurement summaries. Do not use expiring or private links as acceptance evidence.

Validate a real run:

```bash
python3 hardware/optical-spike/validate_manifest.py \
  hardware/optical-spike/samples/<run-id>/manifest.json

python3 hardware/optical-spike/summarize_manifest.py \
  hardware/optical-spike/samples/<run-id>/manifest.json \
  --output hardware/optical-spike/samples/<run-id>/summary.json
```

The second command reruns the fail-closed validator before computing per-page latency, file-size, pixel-density, distortion, saturation, mechanical, power, and thermal summaries. It evaluates only the narrow numeric targets directly supported by manifest fields. Unsupported claims remain explicitly `not-evaluable` or `observed-no-threshold`, and `unresolved_evidence` lists the required manual/raw-evidence review. The summary never turns a template into bench evidence.

The validator requires A4 and Letter captures, all primary conditions, 30 latency samples per page size, ten refold-cycle observations, power/thermal observations, existing unique JPEG files, integer byte/pixel counts, SHA-256 matches, and agreement between each manifest dimension claim and the JPEG frame header. Real bench manifests also require canonical UTC/commit provenance, positive measured dimensions/density/current/latency/sample-rate values, a thermal run of at least 30 minutes, bounded humidity and saturation fractions, usable crops no larger than their encoded images, and a combined-current peak no lower than any constituent peak. Template mode checks the complete collection shape but explicitly permits placeholders and zeroes.

For real-run files, the validator fails closed unless the host supports POSIX descriptor-relative opening with `O_NOFOLLOW`; the repository CI runs this path on Ubuntu. Capture paths may not traverse symlinks, and each declared file is capped at 50 MiB before reading. JPEG checks are intentionally limited to the expected 8-bit baseline DCT/SOF0 camera output: selected frame/scan/component/sampling/DRI invariants, non-empty entropy data, complete frame-component scan coverage, and terminal EOI are required.

These checks establish metadata completeness, path safety, selected baseline-JPEG framing/header consistency, and file integrity from one stable bounded byte snapshot. They do not decode image pixels, validate every JPEG table/codec rule, or prove that measurements were taken correctly or that optical, timing, mechanical, electrical, or thermal acceptance thresholds passed; reviewers must inspect the raw evidence and compare measured results against `hardware/requirements.md`.

Validate the committed metadata template only:

```bash
python3 hardware/optical-spike/validate_manifest.py \
  hardware/optical-spike/manifest.example.json --allow-template
```

## Decision rule

- **GO:** both page sizes frame with margins; measured focus/readability is acceptable for the explicitly documented use cases; timing, stability, power, and thermal targets pass; raw evidence is reproducible.
- **CHANGE:** architecture remains viable but the received camera/lens, stand geometry, illumination, or module must change. Record the selected change and rerun the full matrix.
- **STOP:** the architecture cannot meet the basic full-page/readability, safety, offline storage, or budget constraints with a credible change path.

Current decision: **STOP/HOLD camera and custom-carrier freeze**. No connected module or bench corpus was available on 2026-08-24. See `results.md` and ADR 0001.
