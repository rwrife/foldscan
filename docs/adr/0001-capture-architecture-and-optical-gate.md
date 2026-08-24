# ADR 0001: USB-first capture architecture and optical evidence gate

- **Status:** Accepted for the disposable spike; custom carrier freeze is blocked
- **Date:** 2026-08-24
- **Issue:** [#1](https://github.com/rwrife/foldscan/issues/1)
- **Evidence level:** Manufacturer-document review and calculation only; no FoldScan bench or field test

## Context

FoldScan needs a low-cost, offline document-capture path, but sensor pixel count alone does not establish full-page readability. Camera module shipments, lens field of view, focus across a page, stand stiffness, illumination, capture latency, storage behavior, and power all affect feasibility.

The repository contains no connected XIAO, video device, serial device, temporary stand, camera sample, or instrument log as of this decision. Consequently, this ADR chooses an architecture for a disposable spike and defines a fail-closed gate. It does **not** approve a camera or custom PCB.

## Decision

1. Use **Seeed Studio SKU 113991115, XIAO ESP32-S3 Sense**, as the first off-the-shelf spike candidate only.
2. Require receipt inspection to record the main-board revision, expansion-board revision, camera sensor marking, lens marking, and firmware commit. SKU 113991115 is not sufficient to identify the sensor because Seeed documents a transition in which OV2640 or OV3660 units may ship.
3. Use the included microSD slot for the disposable capture queue and removable-media import. USB is the baseline for power, firmware/recovery, diagnostics, and host transfer. No Wi-Fi is required for capture, recovery, or import.
4. Keep Wi-Fi disabled by default. Any later LAN transport must require explicit physical/local provisioning, authentication, LAN-only binding, bounded requests/files, revocation, and an independent off switch. It must remain unnecessary for baseline operation.
5. Preserve raw image bytes and sidecar metadata on removable storage. Do perspective correction, illumination correction, OCR, review, and export on the local desktop companion.
6. Use only a certified, current-limited USB 5 V SELV source. The spike contains no mains wiring, battery, charger connection, UV source, or laser. The module's onboard battery circuitry is present but the battery pads remain unused.
7. Hold the custom carrier and final camera choice until the bench gate in `hardware/optical-spike/README.md` passes with a committed, checksum-verified non-sensitive corpus.

## Candidate identity and manufacturer evidence

| Item | Documentation result | Design consequence |
|---|---|---|
| Bundle | Seeed Studio XIAO ESP32-S3 Sense, SKU/MPN **113991115** | Record this exact SKU in spike manifests and preliminary BOM. |
| Controller | ESP32-S3R8, dual-core LX7 up to 240 MHz, 8 MB PSRAM, 8 MB flash | Sufficient architecture for compressed capture experiments; not evidence of optical quality. |
| Current camera | Seeed's current product datasheet names OV3660 at 2048×1536. | Nominal pixel-grid calculation may use OV3660 only after receipt inspection confirms it. |
| Shipment ambiguity | Product datasheet p.3 says OV2640 or OV3660 devices may ship during transition; the current Getting Started page says OV2640 is discontinued and subsequent units use OV3660. | The physical sensor marking is mandatory evidence. Do not infer sensor from SKU or seller text. |
| Lens/FOV | No focal length, field of view, distortion, focus range, or lens MPN was found in the reviewed Seeed product datasheet, current wiki, or v1.5 schematic. | Camera height and full-page framing cannot be verified from documentation. Measure them. |
| Camera bus | Seeed camera guide lists 14 occupied GPIOs: GPIO10–18, 38–40, 47, and 48. | The carrier may use exposed D0–D10, but final allocation must account for shared boot/UART/SPI and expansion functions. |
| Exposed I/O | Seeed pin-multiplexing guide lists D0–D10 and expansion D11/D12; D11/D12 correspond to GPIO42/41. | Reserve controls only after the exact module/expansion board is inspected. |
| microSD | Seeed documents up to 32 GB, FAT32; the slot uses four GPIOs. | Spike storage shall use a documented 32 GB-or-smaller FAT32 card and record card MPN/capacity. |
| USB | ESP32-S3 provides full-speed USB OTG and USB Serial/JTAG; the board exposes USB-C. | USB/removable media remains baseline; sustained transfer behavior still needs measurement. |
| RF | The board uses an external U.FL antenna; product package includes an antenna. | Keep antenna and cable outside copper/metal/fastener enclosures. Wi-Fi stays optional/off by default. |
| Board source revision | Current manufacturer schematic file is named v1.5 but its title blocks show design Rev V1.3 dated 2026-02-10; the controller-only schematic is V1.3 dated 2026-01-15. | Record markings/photos from the received boards rather than treating a download filename as physical revision proof. |

### Manufacturer references

1. Seeed Studio, *Industrial Product Datasheet — Seeed Studio XIAO ESP32-S3 Sense*, SKU 113991115, updated 2026-08-21: pp.2–3 (OV3660 and mixed-shipment note), pp.6–7 (interfaces, dimensions, power), p.10 (package contents). <https://files.seeedstudio.com/Bazaar/product_pdf/113991115.pdf>
2. Seeed Studio, *Getting Started with Seeed Studio XIAO ESP32-S3 Series*: specification, sensor-transition note, power table, and camera resolutions. <https://wiki.seeedstudio.com/xiao_esp32s3_getting_started/>
3. Seeed Studio, *Camera Usage in Seeed Studio XIAO ESP32S3 Sense*: camera GPIO table and 32 GB/FAT32 microSD guidance. <https://wiki.seeedstudio.com/xiao_esp32s3_camera_usage/>
4. Seeed Studio, *MicroSD card for Sense Version*: 32 GB/FAT32 support and slot GPIO table. <https://wiki.seeedstudio.com/xiao_esp32s3_sense_filesystem/>
5. Seeed Studio, *Pin Multiplexing with Seeed Studio XIAO ESP32-S3 (Sense)*: exposed D0–D12 mapping and U.FL connection. <https://wiki.seeedstudio.com/xiao_esp32s3_pin_multiplexing/>
6. Seeed Studio, *XIAO ESP32-S3-Sense schematic*, downloaded 2026-08-24: title-block Rev V1.3, 2026-02-10. <https://files.seeedstudio.com/wiki/SeeedStudio-XIAO-ESP32S3/new-res/202003753_XIAO+ESP32S3+Sense_v1.5_SCH_260226.pdf.pdf>
7. Espressif Systems, *ESP32-S3 Series Datasheet v2.2*, §§4.2.1.7–4.2.1.9 (USB and SD/MMC), Tables 5-1/5-2 (3.3 V limits), Table 5-7 (radio-current conditions). <https://documentation.espressif.com/esp32-s3_datasheet_en.pdf>

## Pixel-grid calculation (not optical verification)

The page dimension that receives fewer pixels bounds the ideal, uncropped pixel density. These calculations ignore the required 8 mm margins, lens distortion, aspect-ratio cropping, focus, demosaic/JPEG loss, and stand misalignment, so real values will be lower.

| Confirmed sensor | Maximum frame | A4 ideal upper bound | US-Letter ideal upper bound |
|---|---:|---:|---:|
| OV3660 | 2048×1536 | 6.90 px/mm | 7.11 px/mm |
| OV2640 | 1600×1200 | 5.39 px/mm | 5.56 px/mm |

Calculation: `min(long-axis pixels / page length, short-axis pixels / page width)`, using A4 297×210 mm and Letter 279.4×215.9 mm in landscape sensor orientation. This table must never be reported as a measured result.

## Initial documented power budget

Seeed's product datasheet reports these Type-C figures for the Sense assembly:

| State | Documented current | Calculated power at 5 V |
|---|---:|---:|
| Circuit operating state | 38.3 mA | 0.192 W |
| Webcam example average | approximately 140 mA | 0.700 W |
| Image-capture peak | approximately 347 mA | 1.735 W |

The project source envelope is 5 V, 3 A (15 W). Subtracting the documented 347 mA module peak leaves a mathematical 2.653 A before illumination, carrier losses, USB cable derating, inrush, SD-card variation, and design margin. That remainder is **not** an illumination allowance. Illumination current, combined transients, connector/cable ratings, and thermal behavior remain unknown until selected and measured.

The module price in Seeed's 2026-08-21 product datasheet is USD 13.90 excluding VAT. Arithmetic alone leaves USD 36.10–61.10 inside the USD 50–75 planning envelope for the stand, lighting, carrier, cable, supply, and hardware. This is not a total estimate or an availability claim; all lines require dated live quotes before ordering.

## Alternatives considered

| Alternative | Benefit | Reason not selected/frozen now |
|---|---|---|
| XIAO Sense with received OV3660 | Lowest integration effort; current manufacturer documentation; 3 MP nominal frame | Lens/FOV and full-page readability are undocumented; mixed sensor shipments require inspection. |
| XIAO Sense with OV5640 replacement | 5 MP class sensor and manufacturer-listed compatibility may improve density | Exact module/lens/FOV, autofocus behavior, firmware support, current, mechanics, and availability need separate verification. |
| ESP32-S3 plus a documented external camera module | Allows a lens/FOV and sensor to be selected deliberately | More wiring, mechanical integration, power validation, and cost; defer until the baseline spike fails or is ambiguous. |
| USB UVC camera plus local host | Better commodity camera choices and simpler host processing | Breaks the intended standalone offline capture queue/controller shape and may increase power/cost; retain as a change path, not MVP baseline. |
| Smartphone capture | Excellent optics and no custom camera electronics | Reintroduces the repeated positioning and device-dependency problem FoldScan is intended to address. |

## Consequences and gate state

- **Architecture:** GO for a disposable USB-powered, microSD-backed, local-processing spike.
- **Camera:** **STOP/HOLD** for custom-PCB or mechanical freeze until a physically identified sensor/lens passes the corpus and stand measurements.
- **Safety:** GO only under the 5 V SELV, no-battery, attended-spike constraints.
- **Wi-Fi:** deferred and unnecessary for baseline.
- **Evidence gap:** no raw captures, optical measurements, current trace, thermal readings, stand dimensions, repeatability, or deflection measurements exist yet.

A future result may be **GO** (retain received camera), **CHANGE** (move to a documented higher-resolution/lens option), or **STOP** (camera/ESP32 architecture cannot meet the documented workflow). The result must cite the committed corpus and validated manifest rather than this ADR alone.
