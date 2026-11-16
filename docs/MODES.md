# MC5000 default settings per chemistry and mode

These are the default settings the MC5000 shows in its own menu for every chemistry and mode.
They were collected by going through the menu on a real charger, one chemistry and mode at a
time. The firmware version was not recorded.

The GUI's Configure dialog uses the same defaults (`charger-controller/src/config_dialog.rs`).

**Reading the tables**

- Currents are shown as magnitudes; the charger's menu displays discharge currents as negative
  values (for example −2.0 A).
- *C cut-off* / *D cut-off*: the current at which charging / discharging ends.
- *Cycle order*: C>D (charge, then discharge), C>D>C, or D>C.
- *Rest*: pause after the charge stage and after the discharge stage of a multi-stage mode.
- *Timer*: overall cut-off timer in minutes; "off" means no time limit.
- *−ΔV*: negative delta-V termination for nickel cells.

## Li-Ion

| Mode | Capacity | Charge | Discharge | Voltage | Cut-off current | Other |
|------|----------|--------|-----------|---------|-----------------|-------|
| Charge | 5000 mAh | 3.0 A | — | target 4.20 V | C 100 mA | keep voltage off (4.15 V noted as optimal); timer off |
| Storage | 5000 mAh | 3.0 A | 2.0 A | storage 3.80 V | C 100 mA, D 100 mA | timer off |
| Discharge | 5000 mAh | — | 2.0 A | cut-off 3.20 V | D 100 mA | timer off |
| Cycle | 5000 mAh | 3.0 A | 2.0 A | 4.20 V / 3.20 V | C 100 mA, D 100 mA | order C>D, C>D>C or D>C; 1 cycle; rest 10 + 10 min; timer off |

Firmware 1.83 lowered the lowest Li-ion discharge cut-off the charger accepts from 2.90 V to
2.50 V (SkyRC's release note for 1.83). The 3.20 V default is unchanged. The app does not limit
the cut-off itself; the charger enforces its own range.

## Li-Ion HV

| Mode | Capacity | Charge | Discharge | Voltage | Cut-off current | Other |
|------|----------|--------|-----------|---------|-----------------|-------|
| Charge | 5000 mAh | 3.0 A | — | target 4.35 V | C 100 mA | keep voltage off; timer off |
| Storage | 5000 mAh | 3.0 A | 2.0 A | storage 3.90 V | C 100 mA, D 100 mA | timer off |
| Discharge | 5000 mAh | — | 2.0 A | cut-off 3.40 V | D 100 mA | timer off |
| Cycle | 5000 mAh | 3.0 A | 2.0 A | 4.35 V / 3.40 V | C 100 mA, D 100 mA | order C>D, C>D>C or D>C; 1 cycle; rest 10 + 10 min; timer off |

## LiFePO4

| Mode | Capacity | Charge | Discharge | Voltage | Cut-off current | Other |
|------|----------|--------|-----------|---------|-----------------|-------|
| Charge | 5000 mAh | 3.0 A | — | target 3.65 V | C 100 mA | keep voltage off; timer off |
| Storage | 5000 mAh | 3.0 A | 2.0 A | storage 3.30 V | C 100 mA, D 100 mA | timer off |
| Discharge | 5000 mAh | — | 2.0 A | cut-off 2.90 V | D 100 mA | timer off |
| Cycle | 5000 mAh | 3.0 A | 2.0 A | 3.65 V / 2.90 V | C 100 mA, D 100 mA | order C>D (also C>D>C, D>C); 1 cycle; rest 10 + 10 min; timer off |

## NiMH

| Mode | Capacity | Charge | Discharge | Voltage | Cut-off current | Other |
|------|----------|--------|-----------|---------|-----------------|-------|
| Charge | 3000 mAh | 0.4 A | — | target 1.65 V | — | −ΔV 6 mV; trickle off (50 mA when on); keep voltage off; timer off |
| Refresh | 3000 mAh | 0.4 A | 0.25 A | 1.65 V / 0.90 V | D 90 mA | −ΔV 6 mV; trickle off (50 mA when on); rest 10 + 10 min; timer off |
| Break-in | nominal 3000 mAh | — | — | cut-off 0.90 V | D 90 mA | order C>D>C; rest 10 + 10 min |
| Discharge | 3000 mAh | — | 0.6 A | cut-off 0.90 V | D 90 mA | timer off |
| Cycle | 3000 mAh | 0.3 A | 0.6 A | 1.65 V / 0.90 V | D 90 mA | −ΔV 6 mV; order C>D>C; 1 cycle; trickle off; rest 10 + 10 min; timer off |

## NiCd

| Mode | Capacity | Charge | Discharge | Voltage | Cut-off current | Other |
|------|----------|--------|-----------|---------|-----------------|-------|
| Charge | 3000 mAh | 0.3 A | — | target 1.65 V | — | −ΔV 6 mV; trickle 50 mA; keep voltage off; timer 90 min |
| Refresh | 3000 mAh | 0.3 A | 0.6 A | 1.65 V / 0.90 V | D 100 mA | −ΔV 6 mV; trickle 50 mA; rest 10 + 10 min; timer 90 min |
| Break-in | nominal 3000 mAh | — | — | cut-off 0.90 V | D 100 mA | order C>D>C; rest 10 + 10 min |
| Discharge | 3000 mAh | — | 1.0 A | cut-off 0.90 V | D 100 mA | timer 90 min |
| Cycle | 3000 mAh | 0.3 A | 1.0 A | 1.65 V / 0.90 V | D 100 mA | −ΔV 6 mV; order C>D; 1 cycle; trickle 50 mA; keep voltage off; rest 10 + 10 min; timer 90 min |

## eneloop

Listed in the charger's chemistry menu; its settings were not recorded.

## NiZn

| Mode | Capacity | Charge | Discharge | Voltage | Cut-off current | Other |
|------|----------|--------|-----------|---------|-----------------|-------|
| Charge | 3000 mAh | 1.0 A | — | target 1.90 V | — | keep voltage off; timer off |
| Discharge | 3000 mAh | — | 1.0 A | cut-off 1.10 V | D 100 mA | timer off |
| Cycle | 3000 mAh | 1.0 A | 1.0 A | 1.90 V / 1.10 V | C 100 mA, D 100 mA | order C>D (also C>D>C, D>C); 1 cycle; rest 10 + 10 min; timer off |

## RAM (recorded as "LTO")

The original notes list this section under "LTO", but its voltages (1.65 V target, 0.90 V
cut-off) are those of RAM (rechargeable alkaline) in the charger's chemistry table, and LTO has
its own section below. It most likely belongs to RAM.

| Mode | Capacity | Charge | Discharge | Voltage | Cut-off current | Other |
|------|----------|--------|-----------|---------|-----------------|-------|
| Charge | 3000 mAh | 1.0 A | — | target 1.65 V | — | keep voltage off; timer off |
| Discharge | 3000 mAh | — | 1.0 A | cut-off 0.90 V | D 100 mA | timer off |
| Cycle | 3000 mAh | 1.0 A | 1.0 A | 1.65 V / 0.90 V | D 100 mA | order C>D (also C>D>C, D>C); 1 cycle; rest 10 + 10 min; timer off |

## LTO

| Mode | Capacity | Charge | Discharge | Voltage | Cut-off current | Other |
|------|----------|--------|-----------|---------|-----------------|-------|
| Charge | 3000 mAh | 1.0 A | — | target 2.85 V | C 100 mA | keep voltage off; timer off |
| Storage | 3000 mAh | 1.0 A | 1.0 A | storage 2.40 V | C 100 mA, D 100 mA | timer off |
| Discharge | 3000 mAh | — | 1.0 A | cut-off 1.80 V | D 100 mA | timer off |
| Cycle | 3000 mAh | 1.0 A | 1.0 A | 2.85 V / 1.80 V | C 100 mA, D 100 mA | order C>D (also C>D>C, D>C); 1 cycle; rest 10 + 10 min; timer off |

## Na-Ion

| Mode | Capacity | Charge | Discharge | Voltage | Cut-off current | Other |
|------|----------|--------|-----------|---------|-----------------|-------|
| Charge | 5000 mAh | 2.0 A | — | target 4.00 V | C 100 mA | keep voltage off; timer off |
| Storage | 5000 mAh | 2.0 A | 2.0 A | storage 3.50 V | C 100 mA, D 100 mA | timer off |
| Discharge | 5000 mAh | — | 2.0 A | cut-off 2.00 V | D 100 mA | timer off |
| Cycle | 5000 mAh | 2.0 A | 2.0 A | 4.00 V / 2.00 V | C 100 mA, D 100 mA | order C>D (also C>D>C, D>C); 1 cycle; rest 10 + 10 min; timer off |
