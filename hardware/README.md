# FoldScan hardware

## System block description

```text
certified USB 5 V SELV
  -> input protection / bulk capacitance / test point
  -> XIAO ESP32S3 Sense candidate
       -> camera module
       -> microSD
       -> USB data
       -> optional local Wi-Fi
  -> carrier GPIO
       -> capture button
       -> status indication
       -> illumination PWM/switch stage -> diffused visible LEDs
```

The first custom PCB is a carrier, not an attempt to redesign the ESP32-S3 camera module. It should expose test points, protect external interfaces, enforce known connector orientation, support mounting, and make lighting/button wiring reproducible.

## Controller choice

The current candidate is **Seeed Studio XIAO ESP32S3 Sense** because it combines an ESP32-S3 module, camera expansion, removable storage, USB, and Wi-Fi in a compact maker-accessible form. This is provisional. Before design freeze, the project must confirm the exact revision, manufacturer documentation, camera sensor and lens, available pins, power draw, USB mode, antenna constraints, microSD behavior, and sustained image quality over the full page.

## Interfaces

- Camera sensor/module on the controller expansion.
- microSD for offline capture queue and recovery.
- USB for power, firmware recovery, diagnostics, and baseline host transfer.
- One debounced physical capture button.
- Status indicator that does not rely on color alone.
- PWM-controlled visible-light illumination through a current-limited switch/driver selected from datasheets.
- Optional local Wi-Fi provisioned explicitly; no required cloud endpoint.
- Programming/debug and named test points for 5 V, 3.3 V, ground, reset/boot, light control, button, and relevant buses.

## Power plan

- Certified external USB 5 V SELV source only.
- No mains input, battery, charging, or energy-storage pack.
- Separate measured budgets for controller/camera/microSD and illumination.
- Input protection, inrush/bulk needs, wire/connector current rating, and thermal behavior are selected from manufacturer data.
- Firmware starts illumination in the safe off state and caps brightness until configuration is valid.

## Enclosure and assembly concept

A weighted or clampable base supports a folding arm with positive stops. The camera head is centered over an A4/US-Letter page plane. Two angled diffused light bars reduce hand and arm shadows. Page guides are replaceable and visible in calibration images without intruding into final crop. Cable routing includes strain relief and does not cross hinge pinch zones.

Mechanical source must remain editable. Printed parts should avoid trapped fasteners and allow the camera module, carrier, button, cable, and light bars to be replaced independently.

## Safety limits

Indoor prototype only. USB 5 V SELV only. No UV/laser illumination, no exposed mains, no battery charging, and no unattended operation until measured validation exists. Requirements cover surface temperature, stability, cable strain, and hinge pinch risk. The scanner does not make archival, legal, medical, or accessibility guarantees.

## Expected KiCad deliverables

The hardware milestone must create and maintain:

- `hardware/kicad/foldscan.kicad_pro`
- `hardware/kicad/foldscan.kicad_sch`
- `hardware/kicad/foldscan.kicad_pcb`
- manufacturer/MPN/supplier properties in schematic symbols;
- ERC and DRC reports with every exception resolved or justified;
- schematic PDF and board renders;
- fabrication Gerbers, Excellon drill, position/CPL where applicable, and release ZIP;
- BOM exported from schematic properties to `bom/bom.csv`;
- assembly drawing, test-point map, and bring-up checklist.

Images and PDFs are documentation, not substitutes for editable source.

## Current status

Planning only. There is no schematic, PCB, validated BOM, ERC/DRC output, enclosure, assembled carrier, or bench evidence yet.
