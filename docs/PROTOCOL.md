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

**Operation Modes** (the mode byte is numbered per chemistry family; see the 0x91 section):
- **Charge** - Normal charging to target voltage
- **Storage** - Discharge/charge to storage voltage (lithium family)
- **Discharge** - Discharge only to cutoff voltage
- **Cycle** - Charge/discharge cycles for capacity testing; sub-modes C→D, D→C, C→D→C, D→C→D
- **Refresh** - Deep cycle restoration (NiMH, NiCd, eneloop)
- **Break-In** - Conditioning cycles for new NiMH/NiCd/eneloop cells

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

- **0x02, 0x25, 0xEA are not MC5000 commands.** Re-parsing every capture at the GATT level
  (connection, handle, direction) shows that the bytes once read as `0x25` and `0xEA` frames sit
  inside 182-byte packets exchanged with a *different* BLE device on another connection (at
  offsets 144 and 114), and that `0f 02 02 02` never occurs in the charger's traffic. The
  official app has no such commands either. Do not send them.
- **0x94 ACK**: After sending a full 0x94 config the device replies `0f 04 94 <channel> 01 <checksum>` (e.g. `0f 04 94 01 01 96`) confirming the configuration for that channel.
- **0x93** reads a slot's stored program in the official app; it does not start or stop anything
  (see the 0x93 section).

### Sources for field meanings

Field meanings marked **app** come from the code of the official SkyRC Android app (version
2.94), which builds and parses every MC5000 frame. Each was checked against all captures of that
app talking to a charger; where the captures cannot confirm a field (for example it is always 0),
the text says so.

### 0x06 - Device Greeting (Notification only)

Sent by device immediately upon connection.

**Request**: None (unsolicited notification)

**Response**: `0f 04 06 01 01 08`
- `06` = Command
- `01 01` = Meaning unknown; the official app ignores this frame.
- Checksum: `08`

A similar unsolicited `0f 04 07 00 00 07` appears after the charger restarts (for example after
a firmware update). The app ignores it too.

---

### 0x57 - Device Handshake / Info Request

**Request**: `0f 13 57 00 ff 34 37 38 c1 a4 00 00 00 00 00 00 00 00 00 00 5e`
- `57` = Command
- `00` = Always 0
- `ff 34 37 38 c1 a4` = the charger's BLE MAC address, least significant byte first
  (here `A4:C1:38:37:34:FF`) (app)
- `00...00` = Padding to 17 payload bytes
- `5e` = Checksum

**Response**: `0f 13 57 00 31 30 30 32 31 34 04 00 00 00 00 01 52 01 00 80 57`

The 16 bytes after `57 00` are what the app calls the device code; it sends them as hex to
SkyRC's update server to look up firmware (app).

| Code byte | Example | Meaning |
|-----------|---------|---------|
| 0–5 | `31 30 30 32 31 34` | Model ID, ASCII "100214" = MC5000 (app: `machineSn`; the same for every MC5000, not a serial number) |
| 6–7 | `04 00` | Part of the code; meaning unknown |
| 8–9 | `00 00` | OEM code (`0000` = SkyRC) (app) |
| 10 | `00` | Unused |
| 11–12 | `01 52` | **Firmware version** major, minor as decimal → 1.82. The official 1.83 image carries `01 53`. |
| 13 | `01` | **Hardware version**: the app reads the low nibble → HW 1 (the charger menu shows 1.0) |
| 14 | `00` | Reserve code (app) |
| 15 | `80` | Check byte: makes the 16 code bytes sum to 0 mod 256 (app recomputes it the same way) |

The frame checksum (`57` here) follows as usual.

---

### 0x74 - Password Verification

**Request**: `0f 07 74 00 00 00 00 00 74`
- `74` = Command
- `00` = Always 0
- `00 00 00 00` = The four password digits, one digit (0–9) per byte. The app uses the stored
  password, default `0000` (app).
- `74` = Checksum

**Response**: `0f 04 74 01 01 76`
- `74` = Command
- byte 3 `01` = not read by the app
- byte 4 `01` = Result: `01` password accepted, anything else rejected (app)
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
| 8-9 | XX XX | **Battery temperature** (big-endian, °C) according to the app. 0 in every capture so far. | app only |
| 10-11 | XX XX | **Capacity** (big-endian, mAh): Accumulated charge | ✓ |
| 12-15 | XX XX XX XX | **Elapsed time** (big-endian u32, seconds). Bytes 12-13 are 0 below 18.2 h, which is why bytes 14-15 alone looked right. (app) | ✓ / app |
| 16-17 | XX XX | **Resistance** (big-endian, mΩ): Battery internal resistance | ✓ |
| 18 | XX | **Phase** of the slot (see below). Previously misread as delta-V. | ✓ live / app |
| 19 | XX | **Configured mode**, numbered per chemistry family as in the 0x94 config (see below). Kept after a stop. | ✓ live / app |
| 20 | XX | **Error code**, 0 = none (see below) (app; 9, 10 and 13 seen in captures) | ✓ |
| 21 | XX | **Configured chemistry**, same codes as the 0x94 config. Kept after a stop. | ✓ live / app |
| 22 | XX | Checksum (SUM mod 256 of bytes 2-21) | ✓ |

**Phase, mode, error and chemistry (bytes 18–21)** — from the app's status parser, checked
against a live charger (Li-Ion and NiMH cells, every mode) and all captures:

| Byte 18 | Phase (app text) | Evidence |
|---------|------------------|----------|
| 0x00 | Idle | live, captures |
| 0x01 | "Wait…" (about to start) | app only |
| 0x02 | Charging | live, captures; also the charge stage of Cycle, Refresh, Break-in |
| 0x03 | Discharging | live, captures; also Li-Ion Storage pulling a full cell down |
| 0x04 | Resting (between cycle stages) | app only |
| 0x05 | Done | app only |
| 0x06 | Done | live (NiMH Discharge ended at the cut-off voltage) |

The app treats a slot with phase 0 or 1 and a voltage below 5 mV as empty.

Byte 19 (and the 0x94 mode byte) is numbered per chemistry **family** (app; captures):

| Byte 19 | Lithium family: Li-Ion, Li-Ion HV, LiFePO4, LTO, Na-Ion | Nickel family: NiMH, NiCd, eneloop | NiZn, RAM |
|---------|------|------|------|
| 0x00 | Charge | Charge | Charge |
| 0x01 | Storage | Refresh | Discharge |
| 0x02 | Discharge | Break-in | Cycle |
| 0x03 | Cycle | Discharge | — |
| 0x04 | — | Cycle | — |

NiZn and RAM form their own family. Captures of the app confirm it: NiZn and RAM configs with
mode 1 ran as a discharge (phase 3) and mode 2 as a cycle (phases 2 and 3). Verified live on
firmware 1.83: a RAM config with mode 1 was reported back as RAM Discharge and discharged at
the set 100 mA.

| Byte 20 | Error (app text) | Seen in captures |
|---------|------------------|------------------|
| 0 | none | ✓ |
| 1 | Input Volt Too Low! | |
| 2 | Input Volt Too High! | |
| 3, 9 | Connection Break! | 9 ✓ |
| 4 | Capacity Limit! | |
| 5 | Time Limit! | |
| 6 | Int. Temp. Too High! | |
| 7 | Calibration Failed! | |
| 8 | High Internal Resistance! | |
| 10 | Battery Type Error! | ✓ (a Li-Ion program sent to slots holding NiMH cells) |
| 11 | Overload Protection! | |
| 12 | Reversed Polarity! | |
| 13 | Fully Charged! | ✓ |

Bytes 19–21 describe what was configured and stay set after a stop; byte 18 says whether it is
running. Bytes 4–5 are always the current magnitude (no sign): use byte 18 for direction.

---

### 0x93 - Read a Slot's Program (not start/stop)

**Request**: `0f 03 93 <index> <checksum>`, where `index` is the **0-based** slot number
(0 = slot 1 … 3 = slot 4), not a bitmask.

The official app sends it when it opens a slot's program screen, to read back the program
stored for that slot, and then sends the 0x94 config when the user saves (app). Every capture
agrees: each `93 <n>` is followed by a 0x94 for slot n+1 (`93 02` → slot 3, `93 03` → slot 4).
**The MC5000 never answered 0x93 in any capture** (86 requests, no reply).

Earlier versions of this document called 0x93 "start/stop with a bitmask". That was a
misreading: in the same captures a slot starts as soon as its 0x94 is acknowledged, before or
without any later 0x93, and stops on 0xFE. The live findings that were attributed to 0x93 are
explained by 0x94 and 0xFE:

- `FE 02` + `93 00` stopped slot 2 while slot 4 kept running: `FE 02` stops slot 2.
- `FE 0A` + `93 00` stopped only slot 2: an FE mask with several bits stops only the lowest slot.
- A start mask such as `93 0A` left a running slot untouched: 0x93 changes nothing.

**Verified live (firmware 1.83):** a lone 0x94 (NiMH Charge, 200 mA, no FE before and no 0x93
after) was acknowledged with `94 02 01` and the slot was charging at 200 mA two seconds later;
`93 01` then got no reply and changed nothing; a lone `FE 02` was answered `fe 02 01` and the
slot was idle at 0 mA within two seconds.

This client still sends `FE <slot>` before each config and a `0x93` after, and `93 00` after
each FE stop; the extra 0x93 frames are harmless.

---

### 0x94 - Charging Configuration

Configures a slot's program **and starts it**: in every capture the slot leaves idle as soon as
the charger acknowledges the 0x94 (app: saving the program sends only 0x94, then closes the
screen).

**Request**: `0f 2a 94 <channel> <mode> <params...> <checksum>`

**Mode (byte 4 = data[1])** is numbered per chemistry family, exactly as status byte 19 (see the
0x91 section): lithium family 0 Charge, 1 Storage, 2 Discharge, 3 Cycle; nickel family (NiMH,
NiCd, eneloop) 0 Charge, 1 Refresh, 2 Break-in, 3 Discharge, 4 Cycle; NiZn and RAM 0 Charge,
1 Discharge, 2 Cycle (app; every captured config agrees).

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

**Packet Structure (44 bytes total, VALIDATED from captures and the app):**

> **Endianness note**: Most multi-byte fields are big-endian. Single-byte fields at data[22] (delta-peak) and data[23] (trickle) replaced previously documented 2-byte LE fields (corrected from capture 4 analysis).

| Offset | Data idx | Bytes | Description | Endian | Validated |
|--------|----------|-------|-------------|--------|-----------|
| 0 | - | 0f | Start | - | ✓ |
| 1 | - | 2a | Length (42) | - | ✓ |
| 2 | - | 94 | Command | - | ✓ |
| 3 | 0 | XX | Channel bitmask (01/02/04/08); 0x00 = all slots | - | ✓✓ |
| 4 | 1 | XX | **Mode**, numbered per chemistry family (see above) | - | ✓ |
| 5-6 | 2-3 | XX XX | Charge current (mA) | BE | ✓ |
| 7-8 | 4-5 | XX XX | Discharge current (mA) | BE | ✓ |
| 9-10 | 6-7 | XX XX | Capacity (mAh) | BE | ✓ |
| 11-12 | 8-9 | XX XX | Target/CV voltage (mV); for Storage mode in capture 4 = charge limit (4200mV), NOT storage voltage | BE | ✓ |
| 13-14 | 10-11 | XX XX | Cutoff/discharge voltage (mV) | BE | ✓ |
| 15-16 | 12-13 | XX XX | Charge cutoff current (mA): CV termination threshold (typ. 100mA) | BE | ✓ |
| 17-18 | 14-15 | XX XX | Discharge cutoff current (mA): Stop discharge below this (typ. 100mA). When it is 0 the app sends the discharge current here instead. | BE | ✓ |
| 19-20 | 16-17 | XX XX | **Charge resting time** (minutes): Pause between charge→discharge phases in cycle/refresh modes (confirmed series 5: 10-15 min typical, 0 when not cycling) | BE | ✓✓ |
| 21-22 | 18-19 | XX XX | **Discharge resting time** (minutes): Pause between discharge→charge phases in cycle/refresh modes (confirmed series 5: 10-11 min typical, 0 when not cycling) | BE | ✓✓ |
| 23 | 20 | XX | **Cycle count**: Number of charge/discharge cycles (default 1; observed 2 in NiZn series 5) | - | ✓✓ |
| 24 | 21 | XX | **Cycle direction flag**: 0x00=C→D, 0x01=D→C, 0x02=C→D→C, 0x03=D→C→D (all confirmed series 5) | - | ✓✓ |
| 25 | 22 | XX | **Delta-peak voltage** (mV, single byte): NiMH/NiCd termination (typ. 6mV). 0 for lithium. | - | ✓✓ |
| 26 | 23 | XX | **Trickle charge** (×10 mA, single byte): 0x05=50mA for NiMH. 0 when unused. | - | ✓✓ |
| 27-28 | 24-25 | XX XX | **Keep/float voltage** (mV): NiMH float voltage e.g. 1300mV=1.3V. 0 when unused. | BE | ✓✓ |
| 29 | 26 | XX | **Battery temperature cut-off** (°C) (app: `tempProtect`, from the program's "battery temperature cut-off"; the app sends 71 when it is unset). 60 in every captured config. | - | ✓ / app |
| 30-31 | 27-28 | XX XX | **Cutoff timer** (minutes): Safety timeout (e.g. 60, 90, 120 min). 0 = no timer. | BE | ✓✓ |
| 32-33 | 29-30 | XX XX | Fixed 300 (app field `inMaxbostCapacity`, never user-set for the MC5000; meaning unconfirmed). 300 in every captured config. | BE | ✓ |
| 34 | 31 | 00 | Turbo mode (app); 0 in every captured config | - | ✓ / app |
| 35 | 32 | XX | **Chemistry byte**: 0x00=Li-Ion, 0x03=NiMH (see table above) | - | ✓✓ |
| 36-37 | 33-34 | XX XX | **Storage voltage** (mV) (app): 3800 Li-Ion, 3900 Li-Ion HV, 3300 LiFePO4, 2400 LTO, 3500 Na-Ion. Nickel, NiZn and RAM programs leave it at 3300 (it is not 110 % of capacity: an eneloop config with 2000 mAh also carries 3300). | BE | ✓✓ |
| 38 | 35 | 00 | eneloop sub-type (app); 0 in every captured config | - | ✓ / app |
| 39-42 | 36-39 | 00... | Unused, always 0 | - | ✓ |
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
Channel: 0x08 (Slot 4), **Mode: 0x02** (Break-in in the nickel numbering), Charge: 300mA, Discharge: 600mA, Capacity: 3000mAh, Target: 1650mV, Cutoff: 900mV, CycleDir: 0x00, Chemistry: 0x03 (NiMH), DeltaPeak: 6mV, Storage-voltage field: 3300 (unused)

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
Channel: 0x01 (Slot 1), Mode: 0x00 (Charge), Charge: 300mA, Discharge: 1000mA, Capacity: 3000mAh, Target: 1650mV, Cutoff: 900mV, **DeltaPeak: 6mV** (data[22]=0x06), **Trickle: 50mA** (data[23]=0x05, ×10), **KeepV: 1300mV** (data[24-25]=0x0514 BE), Chemistry: 0x03 (NiMH), Timer: 90min (data[27-28]=0x005a BE), Storage-voltage field: 3300 (unused)

**Example 8: Li-Ion Storage with timer (Slot 4, 700/800mA, 5000mAh) — from capture 4:**
```
0f 2a 94 08 01 02 bc 03 20 13 88 10 68 0c 80 00 64 00 5a 00 0a 00 0a 01 00 06 00 00 00 3c 00 78 01 2c 00 00 0e d8 00 00 00 00 00 bd
```
Channel: 0x08 (Slot 4), Mode: 0x01 (Storage), Charge: 700mA, Discharge: 800mA, Capacity: 5000mAh, **Target: 4200mV** (charge limit, NOT storage voltage), Cutoff: 3200mV, DischCutoff: 90mA, Timer: 120min (data[27-28]=0x0078 BE), Chemistry: 0x00 (Li-Ion), **Storage voltage: 3800mV**

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
Channel: 0x08 (Slot 4), Mode: 0x00 (Charge), Chemistry: **0x07 (RAM)**, Target: 1650mV, Cutoff: 900mV, Storage-voltage field: 3300 (unused)

---

### 0x65 - Firmware-Update Flag

**Request**: `0f 04 65 00 <flag> <checksum>`
- `00` = Always 0
- `flag` = `01` if SkyRC's update server offers newer firmware for this charger, else `00` (app:
  `checkNewVer`). Captures show `00`; the app sent `01` while 1.83 was available for a 1.82
  charger.

**Response**: `0f 04 65 01 01 67`. The app does not read it.

---

### 0xFE - Stop

**Request**: `0f 03 fe <mask> <checksum>`
- `mask` = slot bitmask (0x01, 0x02, 0x04, 0x08), or `0x00` for **all slots** (app: `stopCharge`;
  the app's Stop dialog sends the slot's bit, or 0 when "all" is ticked).

**Response**: `0f 04 fe <mask> <result> <checksum>`; `mask` is `0x10` for "all". `result` `01` =
stopped; anything else makes the app show "stop failed".

Examples (captures):
- All slots: TX `0f 03 fe 00 fe` → RX `0f 04 fe 10 01 0f`, then every running slot went idle
- Slot 4: TX `0f 03 fe 08 06` → RX `0f 04 fe 08 01 07`, then slot 4 went idle

Earlier versions of this document called 0xFE a "slot query". It is the app's only stop command,
which is why `FE 00` on connect stopped every running program. A mask with several bits stops
only the lowest slot (live), so stop several slots with one FE each.

### 0x96 - Read System Settings / 0x95 - Write System Settings

**Request**: `0f 03 96 04 9a` (the app always sends `04`).

**Response**: `0f 09 96 <x> <beep> <notify> <end> <lcd> <warning> <sleep> <checksum>` (app):

| Byte | Meaning |
|------|---------|
| 3 | Not read by the app (`01` and `08` seen) |
| 4 | System beep |
| 5 | Notification beep |
| 6 | End-of-program beep |
| 7 | LCD setting |
| 8 | Warning setting |
| 9 | Sleep time |

Captured replies: `0f 09 96 01 00 00 00 01 01 00 99` and `0f 09 96 08 00 01 00 01 01 00 a1`.

0x95 writes the same settings: payload `00 <beep> <notify> <end> <lcd> <warning> <sleep> <mode>`
padded to 17 bytes; reply byte 4 `01` = accepted (app). This client never sends it.

### Commands this client must not send

From the app; listed so nobody experiments with them by accident:

| Cmd | App use |
|-----|---------|
| 0x60, 0x61, 0x62, 0x66 | Firmware update (prepare, data packets, finish with image checksum, boot-loader version). Writes flash. |
| 0x97 | Reset (`97 04`) |
| 0x92 | "Turbo" setting |
| 0x98 | Charge history (curve) download |

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

## Framing Received Data

Frames are delimited by their length byte, not by BLE notifications. With a 20-byte notification
payload (the BLE default on some platforms), a 23-byte 0x91 status frame arrives in two
notifications; on macOS it arrives whole. The receiver (`FrameDecoder` in
`mc5000-protocol/src/session.rs`) reassembles frames, accepts one only when its checksum matches,
and resynchronizes on the next `0x0F` otherwise. It does not require a particular frame length,
so a firmware that sends longer status frames still decodes.

## Request Pacing (measured on a live charger)

- The charger never sends 0x91 status on its own: it must be polled.
- A 0x91 request with mask `0x0F` returns a single frame, so each slot needs its own request.
- Requests sent too close together are silently dropped. Share of 0x91 requests answered, by
  gap between requests: 0 ms ≈ 30 %, 10–50 ms ≈ 65–85 %, 100–150 ms ≈ 95–98 %.
- The 0x57 handshake was not answered by the tested unit (its 0x74, 0x65 and 0xFE replies were:
  `0f 04 74 01 01 76`, `0f 04 65 01 01 67`, `0f 04 fe 10 01 0f`).

`Session` therefore keeps writes at least 120 ms apart, polls one slot at a time, and re-requests
slots that did not answer.

## App Communication Flow

What the official app does (app; matches every capture):

1. Connect via BLE, enable notifications on FFE1 (CCCD handle 0x0012), request a larger MTU.
2. The charger may send the 0x06 greeting.
3. 0x57 device info → model ID, firmware and hardware version.
4. 0x74 password verification (default `0000`).
5. 0x65 firmware-update flag (`01` when newer firmware is available).
6. Poll 0x91 status, one slot per request.
7. To start a slot: optionally 0x93 to read its stored program, then **0x94** with the program.
   The slot starts when the 0x94 is acknowledged.
8. To stop: **0xFE** with the slot's bit, or `FE 00` for all slots.

**`FE 00` stops every running program (verified live):** a NiMH charge on slot 2 kept running
after `0x74`, `0x65` and `FE 08` (which stops slot 4 only), but stopped at once after `FE 00`.
This client never sends `FE 00` on connect; it sends only `0x74` and `0x65`, so connecting and
reconnecting never interrupt a charge (verified through a connect and four automatic
reconnects with a charge running).

Start and stop do not need 0x74 or 0x65: with only the 0x57 handshake, `FE <slot>` + 0x94 +
`93 <mask>` started a slot and `FE <slot>` + `93 00` stopped it. Per the app and the captures,
the 0x94 and the FE do the work in those sequences.

**Polling and disconnects:** the charger never pushes status. A program keeps running after the
BLE link drops (a slot 4 discharge ran on for ~90 s until stopped explicitly), so always stop
programs explicitly.

## Voltage Examples (VALIDATED)

| Hex | Decimal | Voltage | Chemistry |
|-----|---------|---------|-----------|
| 05 3d | 1341 | 1.341V | NiMH (idle) |
| 05 9d | 1437 | 1.437V | NiMH (charging) |
| 0f bf | 4031 | 4.031V | Li-Ion (CC charging) |
| 10 62 | 4194 | 4.194V | Li-Ion (near full) |

---

## Capture Validation

All Android HCI captures of the official app were parsed down to GATT (connection, handle,
direction) and the charger's frames reassembled from its notify/write characteristic (handle
0x0011). 23 153 frames, every one with a valid checksum:

| Cmd | App → charger | Charger → app | Meaning |
|-----|---------------|---------------|---------|
| 0x06 | — | 15 | Greeting after connect (ignored by the app) |
| 0x07 | — | 1 | Sent after a restart (ignored by the app) |
| 0x57 | 20 | 10 | Device info |
| 0x65 | 10 | 10 | Firmware-update flag |
| 0x74 | 10 | 10 | Password verification |
| 0x91 | 11 360 | 11 331 | Slot status |
| 0x93 | 86 | **0** | Read a slot's program (never answered) |
| 0x94 | 72 | 72 | Program a slot and start it |
| 0x96 | 2 | 2 | Read system settings |
| 0xFE | 71 | 71 | Stop |

No 0x02, 0x25 or 0xEA frame exists on the charger's connection.

**Example: a capture correlated with the user action recorded at the time:**

*Action:* "Slot 3: Started charge, capacity 5000 mAh, target 4.2 V, charge 3 A"

*Frame:*
```
0F 2A 94 04 00 0B B8 07 D0 13 88 10 68 ...
         │  │  ├──┤  ├──┤  ├──┤  ├──┤
         │  │  3000mA 2000mA 5000  4200mV
         │  Li-Ion    (disch) (mAh) (target)
         Slot 3
```
✓ All values match the textual description.


---

## Protocol Capture 3 - Additional Modes (VALIDATED from a capture)

This session tests break-in, cycle, and stop-all commands with a mixed-chemistry setup:
- **Slots 1, 2**: Li-Ion batteries (5000mAh)
- **Slot 4**: NiMH battery (3000mAh)

### GATT Service Structure (from the phone's GATT cache)

| Service | Handle Range | Description |
|---------|-------------|-------------|
| `0000ffe0-0000-1000-8000-00805f9b34fb` | 0x000F - 0x0013 | MC5000 data service |
| `00010203-0405-0607-0809-0a0b0c0d1912` | 0x0014 - 0x0018 | Telink OTA service (the standard UUID of Telink BLE chips, used to update the BLE module itself). The app updates the charger firmware over FFE1 instead. |

| Characteristic | Handle | Properties | Description |
|----------------|--------|------------|-------------|
| `0000ffe1-...` | 0x0011 | 0x16 (READ\|WRITE_WITHOUT_RESPONSE\|NOTIFY) | Main data channel |
| CCCD (FFE1) | 0x0012 | - | Client Characteristic Configuration Descriptor |
| `00010203-...-1912` char | 0x0016 | 0x06 (READ\|WRITE_WITHOUT_RESPONSE) | Telink OTA data |

### Session Timeline (from the capture)

1. **Connect & Greeting**: Device sends `0f 04 06 01 01 08`
2. **Device Info**: TX 0x57 with the MAC → RX model "100214", FW 1.82, HW 1
3. **Password**: TX `0f 07 74 00 00 00 00 00 74` → RX `0f 04 74 01 01 76`
4. **Firmware-update flag**: TX `0f 04 65 00 00 65` → RX `0f 04 65 01 01 67`
5. **Slot 4** (NiMH Break-in): `93 03` (read slot 4), then 0x94 mode 0x02, chemistry 0x03 → slot 4 starts charging
6. **Slot 1** (Li-Ion Cycle D→C): `93 00`, then 0x94 mode 0x03, cycle direction 0x01 → slot 1 starts discharging
7. **Slot 2** (Li-Ion Charge): `93 01`, then 0x94 mode 0x00 → slot 2 starts charging
8. **Polling**: 0x91 for each slot
9. **Stop all**: TX `0f 03 fe 00 fe` → RX `0f 04 fe 10 01 0f`; all three slots go idle

### Key Findings

**Cycle D→C Confirmed**: Cycle (0x03, lithium family) with data[21]=0x01 starts with a
discharge, then charges.

**Chemistry Byte Confirmed**: data[32] encodes the battery chemistry (0x00=Li-Ion, 0x03=NiMH).

**Break-in is mode 0x02 for nickel cells.** In the nickel-family numbering 0x02 *is* Break-in
(see the mode table); an earlier reading of it as "Discharge" used the lithium numbering. Mode
0x05 does not exist: sent live it made the charger end after ~42 s with 0 mAh.

**BLE Notification Truncation**: Full 0x91 responses are 23 bytes, but with a 20-byte
notification payload they arrive in two parts; the frame decoder reassembles them.

### Mode Availability per Chemistry

From the charger's menu (`docs/MODES.md`):

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
