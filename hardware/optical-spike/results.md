# Optical spike evidence ledger

## Run 2026-08-24-documentation

| Field | Value |
|---|---|
| Evidence category | Manufacturer-document review, calculation, and host inventory only |
| Custom hardware | Not built |
| XIAO module | Not connected/present on the executor host |
| Temporary stand | Not present |
| Raw capture corpus | Not produced |
| Bench instruments | Not present/inventoried |
| Field test | Not performed |
| Camera decision | **STOP/HOLD** camera and custom-carrier freeze pending bench evidence |

### Host inventory evidence

Commands run on the Linux executor:

```text
$ lsusb
Bus 001 Device 001: ID 1d6b:0002 Linux Foundation 2.0 root hub
...
Bus 011 Device 002: ID 13d3:3630 IMC Networks Wireless_Device

$ list /dev/video*
(no devices)

$ list /dev/ttyACM* /dev/ttyUSB*
(no devices)
```

The omitted `lsusb` lines were USB root hubs. No Seeed/Espressif serial or camera device was visible. This evidence only establishes why a bench run was not performed in this automation environment; it is not a test of the module.

### Completed evidence

- Exact candidate bundle identified as Seeed Studio SKU/MPN 113991115.
- Manufacturer product datasheet, current wiki pages, board schematics, and Espressif SoC datasheet reviewed and cited in ADR 0001.
- Mixed OV2640/OV3660 shipment risk identified; receipt inspection made mandatory.
- Nominal pixel-density and documented-power calculations recorded and explicitly separated from measurement.
- Reproducible A4 target, run manifest contract, fail-closed validator, and bench procedure added.

### Open physical acceptance evidence

- Received sensor/lens and physical board revisions
- Full A4 and US-Letter framing with margins
- Center/corner px/mm, focus, readable print, line pairs, distortion, glare/shadow
- Capture-to-durable-file latency and representative file sizes
- Camera-axis repeatability and ten-minute head deflection
- Idle/capture/write/transfer/combined illumination current
- Illumination and user-touchable temperatures
- Stable artifact location for raw non-sensitive samples and measurement logs

Issue #1 must remain open until these items are measured and the resulting manifest passes `validate_manifest.py` without `--allow-template`.
