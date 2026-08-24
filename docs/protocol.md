# FoldScan device/app protocol draft

Status: initial contract for backlog refinement. No implementation or interoperability claim exists.

## Principles

- Local-first, transport-neutral, versioned, bounded, and recoverable.
- USB/removable-media import is the mandatory baseline.
- Wi-Fi is optional, off by default, explicitly paired, authenticated, and LAN-only.
- Original captures are immutable through ordinary app operations.
- Device input is untrusted; metadata is data, never commands.
- Unknown major versions fail safely without deletion.

## Identity and versioning

Every manifest or response includes:

```json
{
  "protocol": { "major": 0, "minor": 1 },
  "device_id": "locally-generated-nonsecret-id",
  "firmware_version": "development",
  "capabilities": []
}
```

`device_id` is not an authentication secret and must be resettable. Capabilities are explicit; absence means unsupported. The app may accept newer minor versions only when required fields and semantics remain compatible.

## Storage layout

Proposed removable layout:

```text
/FOLDSCAN/
  device.json
  sessions/
    <session-id>/
      session.json
      captures/
        <capture-id>.jpg
      complete/
        <capture-id>.json
```

Firmware writes image and metadata to temporary names, flushes them, computes a checksum, then atomically renames/finalizes where the filesystem supports it. Recovery ignores or quarantines incomplete temporary entries; it does not guess that they are valid.

## Session manifest

Provisional fields:

```json
{
  "schema": "foldscan.session/0.1",
  "session_id": "uuid-or-equivalent",
  "created_at": "RFC3339-or-null",
  "clock_state": "synced|unsynced|unknown",
  "captures": [
    {
      "capture_id": "uuid-or-equivalent",
      "relative_path": "captures/<id>.jpg",
      "media_type": "image/jpeg",
      "bytes": 0,
      "sha256": "lowercase-hex",
      "width_px": 0,
      "height_px": 0,
      "orientation": 0,
      "captured_at": "RFC3339-or-null",
      "camera": {},
      "illumination": {}
    }
  ]
}
```

Paths are relative, UTF-8, slash-separated, and may not be absolute, contain `..`, use drive prefixes, or escape the session root after canonicalization. The app caps manifest size, capture count, image dimensions, and total imported bytes before allocation.

## Device commands

A framed request/response transport may expose these idempotent operations:

- `get_info`
- `get_status`
- `begin_session`
- `capture`
- `set_illumination`
- `list_sessions`
- `read_manifest`
- `read_capture_range`
- `finalize_session`
- `safe_eject`

Deletion, formatting, provisioning reset, and firmware update are separate privileged workflows with explicit confirmation and recovery documentation. The baseline app never deletes a device capture merely because import succeeded.

Each request carries a request ID, protocol version, operation, bounded payload length, and timeout. Each response echoes the request ID and returns a stable status code plus optional human-readable diagnostic. Retries must not create duplicate captures without a new capture ID.

## Pairing and security assumptions

- Physical USB/removable access implies local possession but not trustworthy file contents.
- Optional Wi-Fi starts disabled. Pairing requires physical device confirmation and a short-lived code displayed by the app/device status flow.
- Pairing creates a revocable local credential stored using OS/device secure facilities available to the selected platform; exact mechanism follows threat review.
- LAN service binds only as required, rejects internet relay/cloud registration, rate-limits requests, caps transfers, and exposes no shell or arbitrary filesystem path.
- Logs exclude image content, OCR text, credentials, and full local paths by default.
- Firmware updates require version/integrity/authenticity verification before a network path can be enabled.

## Error model

Stable categories include: `unsupported_version`, `invalid_request`, `busy`, `storage_full`, `storage_unavailable`, `camera_error`, `capture_timeout`, `checksum_mismatch`, `not_paired`, `unauthorized`, `rate_limited`, and `internal_error`.

Failures preserve existing captures, return the device to a bounded safe state, and turn illumination off when the capture state machine exits unexpectedly.

## Compatibility and test evidence

Protocol fixtures will include known-good manifests and malformed cases: traversal paths, duplicate IDs, integer overflow, excessive dimensions/counts, truncated JSON/images, invalid UTF-8 where transport permits, checksum mismatch, unknown fields, newer minor versions, and unknown major versions. Passing fixture tests is static/software evidence, not physical scanner validation.
