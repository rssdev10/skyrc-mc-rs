# SkyRC MC5000 BLE Protocol Documentation

Reverse-engineered from official Android app BLE traffic capture.

## Overview

The MC5000 is a 4-slot intelligent battery charger supporting multiple battery chemistries and operation modes via Bluetooth Low Energy (BLE). This document describes the complete binary protocol for:

**Supported Battery Chemistries:**
- Li-Ion (4.2V) - Standard lithium-ion
- Li-Ion HV (4.35V) - High-voltage lithium
- LiFePO4 (3.65V) - Lithium iron phosphate  
- NiMH (1.65V) - Nickel metal hydride with delta-peak detection
- NiCd (1.65V) - Nickel cadmium with delta-peak detection
- eneloop (1.65V) - eneloop NiMH optimized program
- NiZn (1.9V) - Nickel zinc
- RAM (1.65V) - Rechargeable Alkaline Manganese
- LTO (2.85V) - Lithium titanate oxide
- Na-Ion (4.0V) - Sodium-ion

**Operation Modes:**
- **Charge** (0x00) - Normal charging to target voltage
- **Storage** (0x01) - Discharge/charge to storage voltage (Li-Ion long-term storage)
- **Discharge** (0x02) - Discharge only to cutoff voltage
- **Cycle** (0x03) - Charge/discharge cycles for capacity testing; supports sub-modes: C→D, C→D→C, D→C
- **Refresh** (0x04) - Deep cycle restoration for NiMH/NiCd batteries
- **Break-In** (0x05) - Conditioning cycles for new NiMH/NiCd batteries (gentle C→D→C)

**Configurable Parameters:**
- Charge/discharge current (mA)
- Target/storage voltage (mV)
- Cutoff voltage (mV)
- Charge/discharge termination current (mA)
- Trickle charge current (mA, NiMH/NiCd)
- Keep/float voltage (mV)
- Delta-peak voltage (mV, NiMH/NiCd termination)
- Cutoff timer (minutes, safety timeout)
- Max operation time (minutes)

## Connection Details

- **Service UUID**: `0000ffe0-0000-1000-8000-00805f9b34fb`
- **Characteristic UUID**: `0000ffe1-0000-1000-8000-00805f9b34fb`
- **Characteristic Properties**: READ | WRITE_WITHOUT_RESPONSE | NOTIFY
- **Handle**: 0x0011 (data), 0x0012 (CCCD for notifications)

## Packet Structure

All packets follow this format:

```
| Start | Length | Command | Data... | Checksum |
| 0x0F  | 1 byte | 1 byte  | N bytes | 1 byte   |
```

- **Start**: Always `0x0F`
- **Length**: Number of bytes after start (includes command, data, and checksum)
- **Command**: Command identifier
- **Data**: Variable length payload (Length - 2 bytes)
- **Checksum**: **SUM (mod 256)** of all bytes from Command to end of Data (excluding Start, Length, and Checksum bytes)

**Checksum Calculation (VALIDATED):**
```rust
fn calculate_checksum(data: &[u8]) -> u8 {
    // data = [0x0F, length, cmd, ...payload]
    // Sum bytes from index 2 (cmd) to end of payload (exclude checksum slot)
    data[2..].iter().fold(0u8, |acc, &b| acc.wrapping_add(b))
}
```

## Commands

### New findings (Protocol capture 2)

Additional frames recovered from the second HCI capture:

- **0x02 - Keep-alive?** Observed as `0f 02 02 02` (very small frame; likely a ping/ack).
- **0x25 - Large blob (app handshake/session lock?)** Observed once as `0f 2b 25 d8 49 d5 9f ee f3 ad 61 b1 c8 93 58 8c 3d 92 d6 81 b5 01 ff 5e 3f 7c 87 b6 7a 13 b2 d9 73 95 8d 3e 69 da 00 00 00 08 00 00 00` (length byte 0x2b → 43 bytes payload: 41 data + checksum). Contents look like opaque entropy (possibly session/auth seed). **Note**: This may be required to "unlock" the device for control commands. Without it, the device may ACK configs but not apply them.
- **0xEA - Large config/telemetry blob (unknown)** Observed once as `0f 22 ea d0 f9 3c 63 3d c5 00 a2 73 94 12 00 00 00 20 00 00 00 20 00 00 00 01 00 00 00 00 00 e3 24 28 49 c8` (length byte 0x22 → 34-byte payload). Purpose unknown; keep for future decoding.
- **0x94 ACK**: After sending a full 0x94 config the device replies `0f 04 94 <channel> 01 <checksum>` (e.g. `0f 04 94 01 01 96`) confirming the configuration for that channel.
- **0x93 action 0x02**: In addition to start-all (0x03) and stop-all (0x00), a frame `0f 03 93 02 95` was captured, indicating a per-bitmask start/stop action (seen while stopping a single slot).

### 0x06 - Device Greeting (Notification only)

Sent by device immediately upon connection.

**Request**: None (unsolicited notification)

**Response**: `0f 04 06 01 01 08`
- `06` = Command
- `01 01` = Unknown (possibly protocol version)
- Checksum: `08`

---

### 0x57 - Device Handshake / Info Request

**Request**: `0f 13 57 00 ff 34 37 38 c1 a4 00 00 00 00 00 00 00 00 00 00 5e`
- `57` = Command
- `00` = Subcommand (query)
- `ff 34 37 38` = BT address suffix reversed (FF34 → ff34, then "78" from MAC)
- `c1 a4` = Unknown (app identifier?)
- `00...00` = Padding
- `5e` = Checksum

**Response**: `0f 13 57 00 31 30 30 32 31 33 34 04 00 00 00 00 01 52 01 00 80 57`
- `57` = Command
- `00` = Status OK
- `31 30 30 32 31 33 34` = ASCII serial number "1002134"
- `04` = Unknown
- `00 00 00 00` = Padding
- `01 52` = **Firmware Version**: Major=1, Minor=82 → **FW 1.82**
- `01 00` = **Hardware Version**: Major=1, Minor=0 → **HW 1.00**
- `80` = Unknown (device capabilities?)
- `57` = Checksum

---

### 0x74 - Version / Info Request

**Request**: `0f 07 74 00 00 00 00 00 74`
- `74` = Command
- `00 00 00 00 00` = Parameters (all zeros)
- `74` = Checksum

**Response**: `0f 04 74 01 01 76`
- `74` = Command
- `01 01` = Version/info data (possibly internal protocol version: major=1, minor=1)
- `76` = Checksum

---

### 0x91 - Channel Status Request

**Request**: `0f 03 91 <channel> <checksum>`
- `91` = Command
- `channel` = Bitmask: 0x01=Slot1, 0x02=Slot2, 0x04=Slot3, 0x08=Slot4
- Checksum = SUM of cmd + channel bytes

Examples (VALIDATED):
- Slot 1: `0f 03 91 01 92` (0x91 + 0x01 = 0x92)
- Slot 2: `0f 03 91 02 93` (0x91 + 0x02 = 0x93)
- Slot 3: `0f 03 91 04 95` (0x91 + 0x04 = 0x95)
- Slot 4: `0f 03 91 08 99` (0x91 + 0x08 = 0x99)

**Response (idle)**: `0f 15 91 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 03 95`
- `91` = Command
- `01` = Channel
- `00` = Status (0 = idle/empty)
- `00 00 00 00...` = All zeros when no battery or idle

**Response (active/charging) - 23 bytes total (VALIDATED):**

Example from real device:
- Slot 1 (NiMH charging): `0f 15 91 01 00 c6 05 9d 00 00 00 5c 00 00 07 61 02 b0 02 00 00 03 75`
- Slot 4 (Li-Ion CC charging): `0f 15 91 08 01 2e 0f bf 00 00 00 91 00 00 06 d3 00 19 03 02 00 00 1e`

| Offset | Bytes | Description | Validated |
|--------|-------|-------------|-----------|
| 0 | 0f | Start | ✓ |
| 1 | 15 | Length (21 = 0x15) | ✓ |
| 2 | 91 | Command | ✓ |
| 3 | XX | Channel (0x01/0x02/0x04/0x08 = slot 1/2/3/4) | ✓ |
| 4-5 | XX XX | **Current (mA, big-endian u16)**: e.g. 0x012A = 298mA, 0x02BD = 701mA. Zero when idle/completed. Previously misinterpreted as separate "status byte" + "current multiplier" — both bytes together form the current. | ✓ |
| 6-7 | XX XX | **Voltage in mV** (big-endian): e.g., 0x059D = 1437mV | ✓ |
| 8 | 00 | Unknown (always 0x00 observed) | ? |
| 9 | 00 | Unknown (always 0x00 observed) | ? |
| 10-11 | XX XX | **Capacity** (big-endian, mAh): Accumulated charge | ✓ |
| 12 | 00 | Unknown (always 0x00 observed) | ? |
| 13 | 00 | Unknown (always 0x00 observed) | ? |
| 14-15 | XX XX | **Elapsed Time** (big-endian, seconds) | ✓ |
| 16-17 | XX XX | **Resistance** (big-endian, mΩ): Battery internal resistance | ✓ |
| 18 | XX | **Delta-V** (mV): Voltage drop for NiMH termination | ✓ |
| 19 | XX | **Chemistry type?**: 0x00=LiIon, 0x02=NiMH (observed, unconfirmed) | ? |
| 20 | 00 | Unknown (always 0x00 observed) | ? |
| 21 | XX | **Slot index?**: 0x00/0x03 observed - meaning unclear | ? |
| 22 | XX | Checksum (SUM mod 256 of bytes 2-21) | ✓ |

**Status Byte Values (Byte 4) - VALIDATED:**
| Value | State | Notes |
|-------|-------|-------|
| 0x00 | Idle/Charging (NiMH) | For NiMH: if byte5 > 0, actively charging. Check voltage/byte5 to distinguish idle vs active |
| 0x01 | Charging - CC mode | Constant Current phase (Li-Ion) - VALIDATED on slot 4 |
| 0x02 | Charging - CV mode | Constant Voltage phase (current tapering) |
| 0x03 | Charging (generic) | Observed in older captures |
| 0x04 | Completed | Charging cycle finished successfully |
| 0x05 | Charging CV/Trickle | Observed for Li-Ion at target voltage (4.2V), trickle/maintenance |
| 0x06 | Charging (alt) | Alternative charging indicator |
| 0x07 | Discharging | Active discharge, uses byte5 × 10 ≈ mA for current |
| 0x09 | Paused | Charging paused by user |

**Current Calculation (VALIDATED):**
- For status 0x00 (NiMH): `current_mA = byte5 × 4`
  - Example: byte5=0xC6 (198) → 198 × 4 = 792 mA ✓
- For status 0x01 (Li-Ion CC): `current_mA = byte5 × 4`
  - Example: byte5=0x2E (46) → 46 × 4 = 184 mA ✓
- For status 0x07 (Discharging): `current_mA = byte5 × 10`
  - Uses a different multiplier than charging modes ✓

**State Disambiguation (Status 0x00):**
- If voltage_mv = 0: **Empty** (no battery in slot)
- If voltage_mv > 0 and byte5 > 0: **Charging** (NiMH/NiCd style, byte5 = current)
- If voltage_mv > 0 and byte5 = 0 and elapsed_seconds > 0 and cap > 0: **Done** (completed charge)
- If voltage_mv > 0 and byte5 = 0: **Idle** (battery present, not active)

---

### 0x93 - Start/Stop Charging

**Request**: `0f 03 93 <action> <checksum>`
- `93` = Command
- `action` =
    - `0x00` Stop all channels
    - `0x03` Start all configured channels (begins operation on all slots that have received a 0x94 config)
    - `0x01/0x02/0x04/0x08` Channel bitmask — starts operation on a specific slot (must have received 0x94 config first)

**Note**: In capture 3, the sequence to start a single slot after stop-all was:
1. Send 0x94 config for the slot
2. Send 0x93 with the slot's bitmask (e.g., 0x01 for slot 1)

No per-slot stop command was observed; only `StopAll` (0x00) was used to stop operations.

Examples:
- Start all: `0f 03 93 03 96`
- Stop all: `0f 03 93 00 93`
- Start slot 1 only: `0f 03 93 01 94`
- Start slot 2 only (observed in capture 2): `0f 03 93 02 95`

---

### 0x94 - Charging Configuration

Configures charging parameters for a slot. Sent before 0x93 Start command.

**Request**: `0f 2a 94 <channel> <mode> <params...> <checksum>`

**Operation Modes (Byte 4 = data[1]):**
| Value | Mode | Description |
|-------|------|-------------|
| 0x00 | Normal Charge | Standard charging (Li-Ion, NiMH, NiCd, LiFePO4, Li-Ion HV) |
| 0x01 | Storage | Discharge/charge to storage voltage (Li-Ion, LiFePO4, LTO, Na-Ion) |
| 0x02 | Discharge | Discharge only to cutoff voltage. **Note**: In capture 3, the app sent mode 0x02 for the "Break-In" (NiMH) operation. Either Break-In is implemented as plain Discharge internally, or mode 0x05 is used in different contexts. |
| 0x03 | Cycle | Charge/discharge cycles (capacity test); cycle direction encoded in data[21] |
| 0x04 | Refresh | Deep cycle restoration mode (NiMH/NiCd only) |
| 0x05 | Break-In | Conditioning cycles for new batteries (NiMH/NiCd only); uses C→D→C pattern. Not directly observed in capture 3 (app sent 0x02 instead). |

**Battery Chemistry Types (data[32]) — confirmed series 5 capture (all 10 chemistries tested):**
| Value | Chemistry | Target Voltage | Cutoff Voltage | Notes |
|-------|-----------|---------------|----------------|-------|
| 0x00 | Li-Ion | 4200 mV (4.2V) | 3200 mV | Standard lithium-ion (confirmed captures 3-5) |
| 0x01 | Li-Ion HV | 4350 mV (4.35V) | 3400 mV | High-voltage lithium (confirmed captures 4-5) |
| 0x02 | LiFePO4 | 3650 mV (3.65V) | 2900 mV | Lithium iron phosphate (confirmed captures 4-5) |
| 0x03 | NiMH | 1650 mV (1.65V) | 900 mV | Nickel metal hydride; uses delta-peak detection (confirmed captures 3-5) |
| 0x04 | NiCd | 1650 mV (1.65V) | 900 mV | Nickel cadmium; uses delta-peak detection (confirmed series 5) |
| 0x05 | eneloop | 1650 mV (1.65V) | 900 mV | eneloop NiMH optimized program; uses delta-peak (confirmed series 5) |
| 0x06 | NiZn | 1900 mV (1.9V) | 1100 mV | Nickel zinc; CV termination (confirmed series 5) |
| 0x07 | RAM | 1650 mV (1.65V) | 900 mV | Rechargeable Alkaline Manganese (confirmed captures 4-5) |
| 0x08 | LTO | 2850 mV (2.85V) | 1800 mV | Lithium titanate oxide; supports storage mode (confirmed series 5) |
| 0x09 | Na-Ion | 4000 mV (4.0V) | 2000 mV | Sodium-ion; supports storage mode (confirmed series 5) |

**Packet Structure (44 bytes total, VALIDATED from capture 3 btsnoop):**

> **Endianness note**: Most multi-byte fields are big-endian. Single-byte fields at data[22] (delta-peak) and data[23] (trickle) replaced previously documented 2-byte LE fields (corrected from capture 4 analysis).

| Offset | Data idx | Bytes | Description | Endian | Validated |
|--------|----------|-------|-------------|--------|-----------|
| 0 | - | 0f | Start | - | ✓ |
| 1 | - | 2a | Length (42) | - | ✓ |
| 2 | - | 94 | Command | - | ✓ |
| 3 | 0 | XX | Channel bitmask (01/02/04/08); 0x00 = all slots | - | ✓✓ |
| 4 | 1 | XX | **Mode**: 0x00=Charge, 0x01=Storage, 0x02=Discharge, 0x03=Cycle, 0x04=Refresh, 0x05=Break-In (see note) | - | ✓ |
| 5-6 | 2-3 | XX XX | Charge current (mA) | BE | ✓ |
| 7-8 | 4-5 | XX XX | Discharge current (mA) | BE | ✓ |
| 9-10 | 6-7 | XX XX | Capacity (mAh) | BE | ✓ |
| 11-12 | 8-9 | XX XX | Target/CV voltage (mV); for Storage mode in capture 4 = charge limit (4200mV), NOT storage voltage | BE | ✓ |
| 13-14 | 10-11 | XX XX | Cutoff/discharge voltage (mV) | BE | ✓ |
| 15-16 | 12-13 | XX XX | Charge cutoff current (mA): CV termination threshold (typ. 100mA) | BE | ✓ |
| 17-18 | 14-15 | XX XX | Discharge cutoff current (mA): Stop discharge below this (typ. 100mA) | BE | ✓ |
| 19-20 | 16-17 | XX XX | **Charge resting time** (minutes): Pause between charge→discharge phases in cycle/refresh modes (confirmed series 5: 10-15 min typical, 0 when not cycling) | BE | ✓✓ |
| 21-22 | 18-19 | XX XX | **Discharge resting time** (minutes): Pause between discharge→charge phases in cycle/refresh modes (confirmed series 5: 10-11 min typical, 0 when not cycling) | BE | ✓✓ |
| 23 | 20 | XX | **Cycle count**: Number of charge/discharge cycles (default 1; observed 2 in NiZn series 5) | - | ✓✓ |
| 24 | 21 | XX | **Cycle direction flag**: 0x00=C→D, 0x01=D→C, 0x02=C→D→C, 0x03=D→C→D (all confirmed series 5) | - | ✓✓ |
| 25 | 22 | XX | **Delta-peak voltage** (mV, single byte): NiMH/NiCd termination (typ. 6mV). 0 for lithium. | - | ✓✓ |
| 26 | 23 | XX | **Trickle charge** (×10 mA, single byte): 0x05=50mA for NiMH. 0 when unused. | - | ✓✓ |
| 27-28 | 24-25 | XX XX | **Keep/float voltage** (mV): NiMH float voltage e.g. 1300mV=1.3V. 0 when unused. | BE | ✓✓ |
| 29 | 26 | 3C | Always 0x3C (= 60 decimal, purpose unknown; present in ALL capture 4 configs) | - | ✓ |
| 30-31 | 27-28 | XX XX | **Cutoff timer** (minutes): Safety timeout (e.g. 60, 90, 120 min). 0 = no timer. | BE | ✓✓ |
| 32-33 | 29-30 | XX XX | **Max time** (minutes): Maximum operation time (typ. 300 min = 5h) | BE | ✓ |
| 34 | 31 | 00 | Always 0x00 (chemistry prefix) | - | ✓ |
| 35 | 32 | XX | **Chemistry byte**: 0x00=Li-Ion, 0x03=NiMH (see table above) | - | ✓✓ |
| 36-37 | 33-34 | XX XX | **Secondary value**: Lithium-class = storage voltage (mV): 3800=Li-Ion, 3900=LiHV, 3300=LiFePO4, 2400=LTO, 3500=Na-Ion. Nickel/Alk = 110% capacity (mAh): 3000→3300. | BE | ✓✓ |
| 38-42 | 35-39 | 00... | Padding (always zeros in captures, 5 bytes) | - | ✓ |
| 43 | - | XX | Checksum | - | ✓ |

**IMPORTANT field layout correction (capture 4):** Previous documentation (captures 1-3) described data[22-23] as 2-byte LE delta-peak and data[26-27] as 2-byte LE timer. Capture 4 Config #3 (NiMH charge, slot 1) with trickle=50mA, keep=1300mV, timer=90min conclusively proves the corrected single-byte layout: byte 0x06 at data[22] is delta-peak (6mV), byte 0x05 at data[23] is trickle (50mA ÷ 10), bytes 0x05 0x14 at data[24-25] are keep voltage (1300mV BE), and bytes 0x00 0x5A at data[27-28] are timer (90min BE). Under the old 2-byte LE interpretation, delta-peak would be 0x0506=1286mV which is absurd for NiMH.

**Response/ACK:** Device replies with `0f 04 94 <channel> 01 <checksum>` to confirm it accepted the configuration. When channel=0x00 (all slots), the ACK uses channel=0x10 (observed in captures 3-4: `0f 04 94 10 01 a5`). This 0x10 value may represent "all slots acknowledged" as a distinct response marker.

**Example 1: Li-Ion Charge (Slot 3, 3A charge, 5000mAh, 4.2V) — from capture 2:**
```
0f 2a 94 04 00 0b b8 07 d0 13 88 10 68 0c 80 00 64 00 64 00 0a 00 0a 01 00 06 00 00 00 3c 00 00 01 2c 00 00 0e d8 00 00 00 00 00 03
```
Channel: 0x04 (Slot 3), Mode: 0x00 (Charge), Charge: 3000mA, Discharge: 2000mA, Capacity: 5000mAh, Target: 4200mV, Cutoff: 3200mV, Chemistry: 0x00 (Li-Ion)

**Example 2: NiMH Charge (Slot 1, 1A charge, 3000mAh, 1.65V) — from capture 2:**
```
0f 2a 94 01 00 03 e8 03 e8 0b b8 06 72 03 84 00 64 00 64 00 0a 00 0a 01 00 06 00 00 00 3c 00 00 01 2c 00 03 0c e4 00 00 00 00 00 6c
```
Channel: 0x01 (Slot 1), Mode: 0x00 (Charge), Charge: 1000mA, Discharge: 1000mA, Capacity: 3000mAh, Target: 1650mV, Cutoff: 900mV, Chemistry: 0x03 (NiMH), DeltaPeak: 6mV

**Example 3: Li-Ion Storage Mode (Slot 4, 3A charge/2A discharge, 5000mAh, storage 3.8V) — from capture 2:**
```
0f 2a 94 08 01 0b b8 07 d0 13 88 0e d8 0c 80 00 64 00 5a 00 32 00 0a 01 00 00 00 00 00 00 78 00 00 01 2c 00 00 13 88 00 00 00 00 XX
```
Channel: 0x08 (Slot 4), Mode: 0x01 (Storage), Charge: 3000mA, Discharge: 2000mA, Capacity: 5000mAh, Storage: 3800mV, Charge cutoff: 100mA, Discharge cutoff: 90mA, Timer: 120min

**Example 4: NiMH "Break-In" (Slot 4, 300mA charge, 3000mAh, 1.65V) — from capture 3:**
```
0f 2a 94 08 02 01 2c 02 58 0b b8 06 72 03 84 00 64 00 64 00 0a 00 0a 01 00 06 00 00 00 3c 00 00 01 2c 00 03 0c e4 00 00 00 00 00 XX
```
Channel: 0x08 (Slot 4), **Mode: 0x02** (Discharge — see note on Break-In), Charge: 300mA, Discharge: 600mA, Capacity: 3000mAh, Target: 1650mV, Cutoff: 900mV, CycleDir: 0x00, Chemistry: 0x03 (NiMH), DeltaPeak: 6mV, Secondary: 3300mAh (110% of 3000)

**Example 5: Li-Ion Cycle D→C (Slot 1, 2A, 5000mAh, 4.2V) — from capture 3:**
```
0f 2a 94 01 03 07 d0 07 d0 13 88 10 68 0c 80 00 64 00 64 00 0a 00 0a 01 01 06 00 00 00 3c 00 00 01 2c 00 00 0e d8 00 00 00 00 00 XX
```
Channel: 0x01 (Slot 1), Mode: 0x03 (Cycle), Charge: 2000mA, Discharge: 2000mA (same as charge), Capacity: 5000mAh, Target: 4200mV, Cutoff: 3200mV, **CycleDir: 0x01** (D→C = discharge first), Chemistry: 0x00 (Li-Ion), DeltaPeak: 6mV

**Example 6: Li-Ion Charge (Slot 2, 1A, 5000mAh, 4.2V) — from capture 3:**
```
0f 2a 94 02 00 03 e8 07 d0 13 88 10 68 0c 80 00 64 00 64 00 0a 00 0a 01 00 06 00 00 00 3c 00 00 01 2c 00 00 0e d8 00 00 00 00 00 XX
```
Channel: 0x02 (Slot 2), Mode: 0x00 (Charge), Charge: 1000mA, Capacity: 5000mAh, Target: 4200mV, Cutoff: 3200mV, CycleDir: 0x00, Chemistry: 0x00 (Li-Ion)

**Example 7: NiMH Charge with trickle/keep (Slot 1, 300mA, 3000mAh) — from capture 4 (CORRECTED FIELDS):**
```
0f 2a 94 01 00 01 2c 03 e8 0b b8 06 72 03 84 00 64 00 64 00 0a 00 0a 01 00 06 05 05 14 3c 00 5a 01 2c 00 03 0c e4 00 00 00 00 00 26
```
Channel: 0x01 (Slot 1), Mode: 0x00 (Charge), Charge: 300mA, Discharge: 1000mA, Capacity: 3000mAh, Target: 1650mV, Cutoff: 900mV, **DeltaPeak: 6mV** (data[22]=0x06), **Trickle: 50mA** (data[23]=0x05, ×10), **KeepV: 1300mV** (data[24-25]=0x0514 BE), Chemistry: 0x03 (NiMH), Timer: 90min (data[27-28]=0x005a BE), Cap2: 3300mAh (110% of 3000)

**Example 8: Li-Ion Storage with timer (Slot 4, 700/800mA, 5000mAh) — from capture 4:**
```
0f 2a 94 08 01 02 bc 03 20 13 88 10 68 0c 80 00 64 00 5a 00 0a 00 0a 01 00 06 00 00 00 3c 00 78 01 2c 00 00 0e d8 00 00 00 00 00 bd
```
Channel: 0x08 (Slot 4), Mode: 0x01 (Storage), Charge: 700mA, Discharge: 800mA, Capacity: 5000mAh, **Target: 4200mV** (charge limit, NOT storage voltage), Cutoff: 3200mV, DischCutoff: 90mA, Timer: 120min (data[27-28]=0x0078 BE), Chemistry: 0x00 (Li-Ion), **Cap2: 3800mV** (storage voltage for Li-Ion)

**Example 9: All-slots Li-Ion Charge (Channel 0x00, ACK uses 0x10) — from capture 4:**
```
TX: 0f 2a 94 00 00 01 2c 07 d0 13 88 10 68 0c 80 00 64 00 64 00 0a 00 0a 01 00 06 00 00 00 3c 00 00 01 2c 00 00 0e d8 00 00 00 00 00 69
RX: 0f 04 94 10 01 a5
```
Channel: 0x00 (all slots), Mode: 0x00 (Charge), Charge: 300mA, Capacity: 5000mAh, Target: 4200mV, Chemistry: 0x00 (Li-Ion). **Note**: ACK response uses channel 0x10, not 0x00.

**Example 10: RAM Charge (Slot 4, 1000mA, 3000mAh) — from capture 4:**
```
0f 2a 94 08 00 03 e8 03 e8 0b b8 06 72 03 84 00 64 00 64 00 0a 00 0a 01 00 06 00 00 00 3c 00 00 01 2c 00 07 0c e4 00 00 00 00 00 77
```
Channel: 0x08 (Slot 4), Mode: 0x00 (Charge), Chemistry: **0x07 (RAM)**, Target: 1650mV, Cutoff: 900mV, Cap2: 3300mAh

---

### 0x65 - Settings Query

**Request**: `0f 04 65 00 00 65`
- `65` = Command
- `00 00` = Query parameters (all zeros)

**Response**: `0f 04 65 01 01 67`
- `65` = Command
- `01 01` = Settings data (possibly device mode or global settings flags)
- `67` = Checksum

---

### 0xFE - Slot Query / Status Check (captures 3+4)

**Request**: `0f 03 fe <channel> <checksum>`
- `fe` = Command
- `channel` = Slot bitmask: 0x01=Slot1, 0x02=Slot2, 0x04=Slot3, 0x08=Slot4, 0x00=General query

**Response**: `0f 04 fe <channel_resp> 01 <checksum>`
- `channel_resp` = Slot bitmask echo (or 0x10 for general query)
- `01` = Status/acknowledgment (always 0x01 observed)

**Examples from capture 4:**
- General query: TX `0f 03 fe 00 fe` → RX `0f 04 fe 10 01 0f`
- Slot 1: TX `0f 03 fe 01 ff` → RX `0f 04 fe 01 01 00`
- Slot 2: TX `0f 03 fe 02 00` → RX `0f 04 fe 02 01 01`
- Slot 3: TX `0f 03 fe 04 02` → RX `0f 04 fe 04 01 03`
- Slot 4: TX `0f 03 fe 08 06` → RX `0f 04 fe 08 01 07`

**Purpose**: Appears to be a per-slot status/presence query. Sent before and after start/stop operations. The app sends this frequently during state transitions (stop/start), suggesting it's used to verify slot readiness.

### 0x96 - Slot Info Query (NEW - capture 4)

**Request**: `0f 03 96 <channel> <checksum>`
- `96` = Command
- `channel` = Slot bitmask (e.g. 0x04 = Slot 3)

**Response**: `0f 09 96 <slot> <data...> <checksum>`
- `96` = Command
- Observed: `0f 09 96 01 00 00 00 01 01 00 99` (7 data bytes)

Purpose: May query detailed slot configuration or capabilities. Observed once in capture 4 for slot 3.

---

## Channel Bitmask

The MC5000 has 4 slots (not 8 like MC3000), using a bitmask:

| Slot | Bitmask | Request |
|------|---------|---------|
| 1 | 0x01 | `0f 03 91 01 92` |
| 2 | 0x02 | `0f 03 91 02 93` |
| 3 | 0x04 | `0f 03 91 04 95` |
| 4 | 0x08 | `0f 03 91 08 99` |

## Checksum Calculation (VALIDATED)

```rust
fn calculate_checksum(data: &[u8]) -> u8 {
    // Sum of bytes from command (index 2) to end of data (excluding checksum), mod 256
    // data = [0x0F, length, cmd, ...payload, checksum_slot]
    // For building: sum bytes[2..len-1], for validating: sum bytes[2..len-1] should equal bytes[len-1]
    data[2..data.len()-1].iter().fold(0u8, |acc, &b| acc.wrapping_add(b))
}
```

The checksum is the **SUM (mod 256)** of all bytes from the command byte to the end of data payload, **NOT including**:
- Start marker (0x0F)
- Length byte
- Checksum byte itself

**Verified examples:**
- `0f 15 91 01 00 c6 ... 03 75`: sum(0x91..0x03) = 0x75 ✓
- `0f 04 94 01 01 96`: sum(0x94 + 0x01 + 0x01) = 0x96 ✓

## App Communication Flow

1. Connect to device via BLE
2. Subscribe to notifications on 0xFFE1 characteristic (write 0x0100 to CCCD handle 0x0012)
3. Receive greeting notification (0x06)
4. Send handshake (0x57) → get device info, FW/HW version, serial number
5. **Send 0x25 session/auth blob?** (observed once in captures 1-2, NOT in capture 3; may be optional)
6. Send 0x74 version query → get protocol version info
7. Send 0x65 settings query → get device settings
8. Send 0xFE query → unknown purpose (observed in capture 3)
9. Send 0x94 config for each slot to be used
10. Send 0x93 start (bitmask for slots, e.g. 0x0B = slots 1+2+4)
11. **Poll channel status (0x91) continuously** for all slots at ~2s intervals
12. Send 0x93 stop (0x00=all) when done

**CRITICAL - Init Sequence Required for Control Commands:**
The init sequence (steps 6-8: 0x74 → 0x65 → 0xFE) MUST be sent after the handshake for the device to accept start/stop commands. Without this sequence, the device acknowledges config packets (0x94) but silently ignores start/stop (0x93). This was confirmed by live testing: stop commands failed until the init sequence was added to the connect flow.

**CRITICAL - Multi-Slot Start Command:**
The start command (0x93) sets the COMPLETE active slot set. Sending start(0x01) for slot 1 followed by start(0x02) for slot 2 in separate sessions does NOT result in both slots running — each start replaces the previous one. To start multiple slots:
1. Send configs for ALL slots first (0x94 for each)
2. Send a SINGLE start command with all slot bits combined (e.g. 0x0B = slots 1+2+4)

**CRITICAL - Continuous Polling Required:**
The device appears to require ongoing status polling (0x91) to keep operations active. Operations stop within seconds of BLE disconnection if no polling has been maintained. The official app polls all 4 slots every ~2 seconds throughout the session.

**Note on Init Sequence Side Effects:**
When a NEW BLE session connects and sends the init sequence (0x74/0x65/0xFE), it may restart previously configured operations with reset timers and capacity counters. The device retains slot configurations across BLE sessions but the elapsed-time and capacity-accumulated counters reset on re-init.

## Voltage Examples (VALIDATED)

| Hex | Decimal | Voltage | Chemistry |
|-----|---------|---------|-----------|
| 05 3d | 1341 | 1.341V | NiMH (idle) |
| 05 9d | 1437 | 1.437V | NiMH (charging) |
| 0f bf | 4031 | 4.031V | Li-Ion (CC charging) |
| 10 62 | 4194 | 4.194V | Li-Ion (near full) |

---

## Binary Log Correlation (Validation)

Extracted and validated protocol frames from Android BTSnoop HCI logs:

**Source logs:**
- `logs/bugreport/FS/data/log/bt/btsnoop_hci.log` (271KB) → **197 valid MC5000 frames**
- `logs/bugreport/FS/data/log/bt/btsnoop_hci.log.last` (428KB) → **319 valid MC5000 frames**
- `logs/series_3/btsnooz_hci.log` (btsnoop v1, 4398 records) → **340 valid MC5000 GATT frames**

**Extraction method:** Standard btsnoop v1 parsing → ACL → L2CAP → ATT → identify MC5000 frames by 0x0F start marker + valid command byte + checksum verification.

**Validated commands from binary logs:**
| Command | Captures 1-2 | Capture 3 | Purpose | Confirmed |
|---------|-------------|-----------|---------|-----------|
| 0x02 | 1 | ~20 | Keep-alive | ✓ |
| 0x06 | 4 | 1 | Greeting | ✓ |
| 0x25 | 1 | 0 | Auth/Session (43 bytes) | ✓ |
| 0x57 | 4 | 2 | Device Info | ✓ |
| 0x65 | 4 | 2 | Settings | ✓ |
| 0x74 | 4 | 2 | Version Info | ✓ |
| 0x91 | 490+ | ~300 | Slot Status Polling | ✓ |
| 0x93 | 5 | 3 | Start/Stop | ✓ |
| 0x94 | 8 | 7 | Config (req + ACK) | ✓ |
| 0xEA | 1 | 0 | Telemetry (unknown) | ✓ |
| 0xFE | 0 | 2 | Unknown query (NEW) | ✓ |

**Example correlation with `protocol 2.md` textual log:**

*Action:* "Slot 3: Started charge, capacity 5000 mAh, target 4.2 V, charge 3 A"

*Binary frame (offset 0x02B81F):*
```
0F 2A 94 04 00 0B B8 07 D0 13 88 10 68 ...
         │  │  ├──┤  ├──┤  ├──┤  ├──┤
         │  │  3000mA 2000mA 5000  4200mV
         │  Li-Ion    (disch) (mAh) (target)
         Slot 3
```
✓ All values match the textual description.

**Checksum validation:** 516 total frames extracted, 0 checksum failures - confirms SUM (mod 256) algorithm.

---

## Protocol Capture 3 - Additional Modes (VALIDATED from btsnoop)

**Source**: `logs/series_3/btsnooz_hci.log` (standard btsnoop v1 format)  
**Extraction**: 4398 HCI records → 1238 ACL packets → 387 ATT operations → **340 MC5000 GATT frames**  
**Session log**: `logs/series_3/protocol 3.md`

This session tests break-in, cycle, and stop-all commands with a mixed-chemistry setup:
- **Slots 1, 2**: Li-Ion batteries (5000mAh)
- **Slot 4**: NiMH battery (3000mAh)

### GATT Service Structure (from bluetooth_manager.log cache)

| Service | Handle Range | Description |
|---------|-------------|-------------|
| `0000ffe0-0000-1000-8000-00805f9b34fb` | 0x000F - 0x0013 | MC5000 data service |
| `00010203-0405-0607-0809-0a0b0c0d1912` | 0x0014 - 0x0018 | Secondary service (unknown purpose) |

| Characteristic | Handle | Properties | Description |
|----------------|--------|------------|-------------|
| `0000ffe1-...` | 0x0011 | 0x16 (READ\|WRITE_WITHOUT_RESPONSE\|NOTIFY) | Main data channel |
| CCCD (FFE1) | 0x0012 | - | Client Characteristic Configuration Descriptor |
| `00010203-...-1912` char | 0x0016 | 0x06 (READ\|WRITE_WITHOUT_RESPONSE) | Secondary (unknown) |

### Session Timeline (from btsnoop)

1. **Connect & Greeting**: Device sends `0f 04 06 01 01 08` (greeting)
2. **Device Info**: TX handshake with MAC `FF:34:37:38:C1:A4` → RX serial "100214", FW 1.82, HW 1.0
3. **Version Query**: TX `0f 07 74 00 00 00 00 00 74` → RX `0f 04 74 01 01 76`
4. **Settings Query**: TX `0f 04 65 00 00 65` → RX `0f 04 65 01 01 67`
5. **Unknown 0xFE**: TX `0f 03 fe 00 fe` → RX `0f 04 fe 10 01 0f`
6. **Start All**: TX `0f 03 93 03 96` (start all configured channels)
7. **Config Slot 4** (NiMH "Break-In"): Mode 0x02, 300mA charge, 600mA discharge, chemistry 0x03
8. **Config Slot 1** (Li-Ion Cycle D→C): Mode 0x03, 2000mA, cycle_direction=0x01, chemistry 0x00
9. **Config Slot 2** (Li-Ion Charge): Mode 0x00, 1000mA, chemistry 0x00
10. **Polling**: Continuous 0x91 status requests for all 4 slots
11. **Stop All**: TX `0f 03 93 00 93`
12. **Start Slot 1**: Re-configure slot 1, then TX `0f 03 93 01 94` (single slot start)

### Key Findings

**Cycle D→C Confirmed**: When Cycle mode (0x03) is configured with data[21]=0x01, the charger starts with discharge (status 0x07) then transitions to charge. Slot 1 status timeline: Idle → CC → Charging → **Discharging** → Idle.

**Chemistry Byte Confirmed**: data[32] encodes the battery chemistry (0x00=Li-Ion, 0x03=NiMH). Previous versions inferred chemistry from target voltage.

**Break-In Mode Confirmed as Discharge (0x02)**: The app's "Break-In" operation for NiMH sends mode byte 0x02 (Discharge), NOT 0x05 (Break-In). **Confirmed by live testing**: mode 0x05 causes the device to abort after ~42 seconds with 0mAh and enter a "Done" state. Mode 0x02 with break-in parameters (discharge_current = 2× charge_current, cutoff_timer=60min, max_time=300min) works correctly. "Break-In" is a UI concept implemented as Discharge with specific parameters.

**BLE Notification Truncation**: Full 0x91 responses are 23 bytes, but BLE notifications may be truncated to 20 bytes (BLE default MTU). Parsers should handle responses shorter than 23 bytes gracefully. Bytes 19-22 (chemistry hint, unknown, slot index, checksum) may be missing in truncated notifications.

### Mode Availability per Chemistry

From app UI analysis (`modes.md`):

| Chemistry | Charge | Storage | Discharge | Cycle | Refresh | Break-In |
|-----------|--------|---------|-----------|-------|---------|----------|
| Li-Ion    | ✓      | ✓       | ✓         | ✓     |         |          |
| Li-Ion HV | ✓      | ✓       | ✓         | ✓     |         |          |
| LiFePO4   | ✓      | ✓       | ✓         | ✓     |         |          |
| NiMH      | ✓      |         | ✓         | ✓     | ✓       | ✓        |
| NiCd      | ✓      |         | ✓         | ✓     | ✓       | ✓        |
| eneloop   | ✓      |         | ✓         | ✓     | ✓       | ✓        |
| NiZn      | ✓      |         | ✓         | ✓     |         |          |
| RAM       | ✓      |         | ✓         | ✓     |         |          |
| LTO       | ✓      | ✓       | ✓         | ✓     |         |          |
| Na-Ion    | ✓      | ✓       | ✓         | ✓     |         |          |

### Cycle Mode Sub-Parameters (CONFIRMED from series 5)

The Cycle mode (0x03) supports direction sub-modes:
- **C→D**: Charge first, then discharge — data[21] = 0x00 (default)
- **D→C**: Discharge first, then charge — **data[21] = 0x01 (CONFIRMED from capture 3)**
- **C→D→C**: Charge, discharge, charge — **data[21] = 0x02 (CONFIRMED from series 5)**
- **D→C→D**: Discharge, charge, discharge — **data[21] = 0x03 (CONFIRMED from series 5)**

The cycle direction is encoded at data[21] (packet offset 24).

The Cycle mode also has additional parameters (all CONFIRMED from series 5):
- **Cycle count** (number of C/D cycles): data[20] (packet offset 23). Default 1, observed up to 2 (NiZn).
- **Charge resting** (minutes between charge→discharge phases): data[16-17] BE (packet offset 19-20). Typical 10-15 min.
- **Discharge resting** (minutes between discharge→charge phases): data[18-19] BE (packet offset 21-22). Typical 10-11 min.
