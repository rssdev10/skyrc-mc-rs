# Copilot Instructions

Guidance for GitHub Copilot when working in the MC5000 Charger Controller repository.

## Read README.md first

**[README.md](README.md) is the single source of truth** for this repo: crate layout and
responsibilities, the concurrency model, the documentation map, supported chemistries and modes,
build, test and lint commands, diagnostics, CI and release packaging. Read the relevant sections
before changing anything. This file adds only rules for AI assistants.

## Cross-Tool Instruction Sync

This project keeps three parallel AI-assistant instruction files, one per tool:

- `CLAUDE.md` — Claude Code
- `AGENTS.md` — OpenAI Codex
- `.github/copilot-instructions.md` — GitHub Copilot

They are deliberately thin. **Shared project guidance belongs in `README.md`, not here** — add or
fix it there, and link to it from these files rather than copying it.

Their bodies are intentionally identical. **If any one of these files is updated by any tool or
contributor, the other two MUST be updated in the same change.** Only the title and the tool-name
framing may differ.

## Rules for AI assistants

### Before you finish

Every change must leave both of these passing; run them yourself and report the result:

```bash
cargo test --workspace                                        # unit tests
cargo clippy --workspace --all-targets -- -D warnings         # lint (must be warning-free)
```

- Do not silence lints with `#[allow(...)]`. If one is truly needed, scope it to the smallest
  item and say why in a comment.
- Run `rustfmt` on files you create. Many existing files are not rustfmt-clean: match their
  style and do not reformat code you did not change.

### Tests

- Behaviour changes come with tests in the same crate. Do not delete or weaken a test without an
  equivalent replacement.
- Protocol changes need tests on concrete byte arrays, preferably frames received from a real
  charger, including the checksum.
- Tests must not need Bluetooth hardware. `Session` runs on any `Transport`: test it with an
  in-memory transport, as the tests in `skyrc-mc5000/src/session.rs` do.

### Concurrency

BLE notifications, the session's reader and poller, iced subscriptions and `Task::perform`
futures all run concurrently with the code that sends commands.

- **All charger I/O goes through `skyrc_mc5000::Session`** (README, "Architecture"). It paces
  writes and keeps command sequences from interleaving. Never write to the peripheral directly
  and never add a second polling loop.
- **Never block the GUI thread.** No `block_on` or sleeps in `update()`; return a
  `Task::perform(...)`. `Tick` must not send anything to the charger.
- **Shared state is `Send + Sync`**, behind `Arc` with a `Mutex`, `RwLock` or atomic. Add new
  shared types to the compile-time `Send + Sync` assertion in that crate's tests.
- **Locks:** never hold a `std::sync::Mutex` guard across `.await` (use `tokio::sync::Mutex` for
  async exclusion), never call anything that locks while holding a guard, and never hold two
  locks at once. Check-then-act happens under one lock. Recover from poisoning with
  `.lock().unwrap_or_else(PoisonError::into_inner)`, not `.lock().unwrap()`.
- **One write per line** for output several tasks produce: format the line, then print it once.
- **Concurrency changes need a concurrent test** (`#[tokio::test(flavor = "multi_thread")]` or
  real threads) that fails when the guard it protects is removed. Check that it does.

### Hardware and battery safety

This software pushes amps into lithium and nickel cells. A wrong byte can overcharge or
over-discharge a battery.

- Protocol facts need evidence: a verified capture or a live test. Say what the evidence is in the
  change, and record new facts in `docs/PROTOCOL.md` in the same change. Do not change voltages,
  currents, cut-offs, mode bytes or command encodings without it.
- Do not assume one unit or firmware: names, address suffixes and frame lengths vary between
  chargers.
- Placeholders must fail loudly. Never return fake readings or fake replies from production code.
- Ask the user before changing what the charger does physically: default currents, auto-charge
  current selection, start or stop handling.

### Testing on a real charger

- Only when the user has a charger connected and has agreed. Read-only checks (`charger-cli
  status`, `charger-cli debug`, or `charger-controller -H` with `state`, `wait-until`, `watch`)
  need no extra consent; starting or stopping a slot does.
- Test GUI behaviour through the GUI's own code path: `charger-controller -H` (README, "Headless
  mode").
- A connected app keeps the charger from advertising. If a scan finds no charger, look for a
  running `charger-controller` or `charger-cli` and ask the user; never stop their processes.
- Re-read every slot's voltage right before starting anything: cells get swapped between runs.
  Start programs through the GUI path, which refuses a chemistry the cell's voltage rules out.
- Use low currents and short runs. A disconnect does not stop a program: finish every test with a
  stop and confirm from the charger's status, over a fresh connection, that every slot is idle.

### Change discipline

- Prefer small, focused changes that keep existing APIs and behaviour.
- Update `docs/GUI_DESIGN.md` whenever the GUI changes. Add new user-visible strings to every file
  in `charger-controller/locales/`.
- Keep CLI examples in README and docs in exact agreement with the `charger-controller-cli`
  argument names.
- Do not change version numbers, the license or authorship without explicit instruction.
- Never commit secrets, local settings or device captures.
- Commit messages use a short scoped prefix, as in the history: `protocol: ...`, `gui: ...`,
  `cli: ...`, `docs: ...`, `cicd: ...`. Commit or push only when the user asks.
- If a requirement is unclear, ask one concise question before making the change.
