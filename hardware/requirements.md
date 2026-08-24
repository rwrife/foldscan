# FoldScan hardware and system requirements

Status: initial measurable targets for validation. Values are requirements or planning envelopes, not measured results.

## Electrical

| ID | Requirement |
|---|---|
| ELEC-01 | Accept power only from a certified USB 5 V SELV source; include no mains or battery-charging circuitry. |
| ELEC-02 | Remain within a 5 V, 3 A source envelope including illumination; the selected design must publish calculated and measured peak/steady current with margin. |
| ELEC-03 | Default illumination output off during reset, boot, brownout, firmware crash, and unprovisioned state. |
| ELEC-04 | Use a datasheet-selected current-limiting/driver method; do not drive illumination load directly from an MCU GPIO. |
| ELEC-05 | Expose labeled test points for input 5 V, regulated 3.3 V, ground, boot/reset, capture button, light control, and critical buses. |
| ELEC-06 | Provide input/interface protection appropriate to user-accessible USB and external low-voltage wiring; selection must cite manufacturer datasheets. |
| ELEC-07 | Preserve module antenna keepout and routing guidance; no copper, fastener, cable, or enclosure intrusion into the verified keepout. |
| ELEC-08 | Carrier schematic must pass ERC and PCB must pass DRC with zero unexplained violations under documented rules. |

## Optical and capture

| ID | Requirement |
|---|---|
| OPT-01 | Frame an entire A4 (210 × 297 mm) and US-Letter (216 × 279 mm) page with at least 8 mm alignment margin on every side. |
| OPT-02 | Before PCB freeze, resolve a printed test target across center and all four corners; publish pixels-per-mm, smallest consistently readable text, distortion, and focus variation. No minimum is claimed until the spike measures it. |
| OPT-03 | Capture button to durable-file completion target: median ≤ 3 s and 95th percentile ≤ 5 s after exposure is settled. |
| OPT-04 | Dual or otherwise symmetric diffused visible illumination must be evaluated for shadows, glare, banding, and color shift on matte and glossy samples. |
| OPT-05 | Preserve original image bytes and calibration metadata even when processed derivatives or OCR are generated. |
| OPT-06 | Every durable capture has a unique ID, timestamp source/status, dimensions, byte length, checksum, and session association. |

## Storage and connectivity

| ID | Requirement |
|---|---|
| DATA-01 | Capture at least 200 pages at the selected production quality on a documented supported microSD size without a host connection. |
| DATA-02 | Use atomic manifest/file-finalization behavior and demonstrate recovery after induced power loss at each write stage. |
| DATA-03 | USB/removable-media import is the baseline and must work with Wi-Fi disabled. |
| DATA-04 | If Wi-Fi transfer is implemented, it is disabled until explicit pairing, limited to the local network, authenticated, rate/size bounded, and independently disableable. |
| DATA-05 | Protocol changes are versioned; unknown major versions fail safely and never delete captures. |

## Mechanical

| ID | Requirement |
|---|---|
| MECH-01 | Folded target envelope ≤ 330 × 120 × 90 mm, excluding detachable cable and optional desk clamp. |
| MECH-02 | Deployed stand supports A4/US-Letter page placement without camera head overlap over the page region. |
| MECH-03 | Camera optical axis lateral repeatability after ten unfold/refold cycles: target ≤ 2 mm at the page plane; measure and publish actual result. |
| MECH-04 | Camera-head vertical deflection after 10 minutes deployed: target ≤ 2 mm under its own mass; measure at documented ambient temperature. |
| MECH-05 | A 20 N horizontal load at the base edge must not expose live conductors or cause a sharp fracture; stability behavior and whether a clamp is required must be documented. |
| MECH-06 | Hinge pinch zones, hot surfaces, and cable paths must be guarded or marked; cables require strain relief. |
| MECH-07 | Controller, carrier, button, light bars/diffusers, hinge, cable, and page guides are independently replaceable with common tools. |

## Environmental and thermal

| ID | Requirement |
|---|---|
| ENV-01 | Indoor use target: 10–35 °C and 20–80% RH non-condensing; no outdoor/weatherproof claim. |
| ENV-02 | At 35 °C ambient and maximum allowed continuous illumination for 30 minutes, user-touchable surfaces target < 45 °C and no component exceeds its datasheet rating/derating policy. |
| ENV-03 | Device must not emit UV or laser radiation; illumination is visible-light, low-voltage, and diffused. |
| ENV-04 | Store and transport unpowered; no unattended operation claim until bench and field evidence exists. |

## Companion and accessibility

| ID | Requirement |
|---|---|
| APP-01 | Core import, review, correction, and export work without internet or an account on supported Windows/macOS/Linux builds. |
| APP-02 | Originals are immutable by default; destructive cleanup requires explicit confirmation and a documented recovery boundary. |
| APP-03 | Export session manifest, original/processed images, OCR text, and PDF without proprietary lock-in. |
| APP-04 | Keyboard-only operation, screen-reader labels, scalable text, visible focus, high contrast, and non-color-only state indicators are acceptance requirements. |

## Cost and sourcing

| ID | Requirement |
|---|---|
| COST-01 | Planning target USD 50–75 for one prototype including electronics, ordinary printed/mechanical parts, fasteners, cable, and suitable certified USB supply; excluding computer, printer, tools, shipping, and tax. |
| COST-02 | Reprice every line from live sources before ordering; record quote date, source, stock, and substitutions. The planning target is not a quote. |
| COST-03 | Populate Manufacturer and exact MPN in KiCad symbol properties before schematic approval; validate package, pinout, lifecycle, ratings, and availability from manufacturer/distributor evidence. |
| COST-04 | Track enclosure, diffuser, cable, fasteners, page guides, clamp/weight, and power supply as non-schematic BOM items. |

## Verification vocabulary

- **Static check:** ERC/DRC/analyzers/code inspection only.
- **Simulation:** calculated or modeled result with inputs and tool version.
- **Bench test:** measured physical prototype under recorded conditions.
- **Field test:** representative user/workflow use outside the controlled bench setup.

A requirement is not “verified” until its specified evidence category exists.
