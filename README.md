[![](https://gamebanana.com/mods/embeddables/700758?type=large)](https://gamebanana.com/mods/700758)

# Lovelock Companion

Lovelock Companion is a desktop app that syncs local-player deaths, kills, assists, ability uses, and cooldown readiness in Deadlock to a Lovense toy over the local Standard API.
Made because my friend complained about OCR missfiring for them.

## !!! Required Deadlock mod !!!

Lovelock Companion does not work by itself. Install and enable the [DeadlockShock mod from GameBanana](https://gamebanana.com/mods/700758) in Deadlock before starting the companion. The mod detects gameplay events and writes them to the log that the companion listens to.

## Contents

- `companion/` is Lovelock Companion, the Lovense-only desktop app.
- `lovense/` is the crate Lovelock Companion uses to talk to Lovense toys over
  the local Standard API ("Game Mode"), via the Lovense Connect/Remote app
  running on the same LAN. In **Setup**, enable Game Mode in the Lovense
  Remote app on the same PC, then Test connection. Connection settings, toy
  selection, and per-trigger vibration settings are all persisted.
- `mod/` has the Panorama gameplay-state hook for the companion DeadlockShock
  mod, which is what actually feeds Lovelock Companion its events.

## Preview

[![Preview](./media/showcase.png)](./media/showcase.png)

## Building from source

You will need [Rust](https://rust-lang.org/), [PowerShell](https://learn.microsoft.com/en-us/powershell/), and [Reduced CSDK 12](https://deadlockmodding.pages.dev/modding-tools/csdk-12).

From the repo root:

```powershell
.\build.bat
cargo build --manifest-path companion/Cargo.toml --release
```

That puts the addon at `dist/deadlock_death_hook.vpk`. Install and enable it in Deadlock, then start Lovelock Companion:

```sh
cargo run --manifest-path companion/Cargo.toml --release
```

On Windows, `build_and_run.bat` builds Lovelock Companion in debug mode and launches it in one step, which is handy while iterating.

In **Setup**, enter the Lovense Connect/Remote domain and HTTP port, test the connection, and optionally pick a specific toy (leave unselected to vibrate every connected toy).

In **Effects**, configure Death, Kill, Assist, Ability use, and Cooldown ready independently. Each trigger has its own vibration profile (fixed strength/duration, or a random interval). Ability-use and cooldown-ready also have independent positional-slot filters that apply across heroes; ability names appear when the addon reports them, with numbered slots as the fallback. Use the explicit Copy control to copy only the active vibration profile between triggers without changing enablement or ability selection. Local-player death is enabled by default, while Kill, Assist, and both ability triggers are opt-in. Cooldown ready covers both a normal cooldown finishing and a charged ability restoring a charge. Kill and Assist are read from the local player's own scoreboard KDA counters, so they fire once per credited kill or assist regardless of who lands the killing blow on a shared-credit takedown.

In **Game connection**, Lovelock Companion automatically resumes a saved `console.log` path or auto-detects Deadlock and starts the listener at launch. Use **Auto-detect** and **Start/Restart listener** for diagnostics, retry, or a manual path override. Deadlock must run with `-condebug` so the log is written.
On Windows releases Lovelock Companion uses the GUI application subsystem, so launching it from Explorer does not open a command window. On Windows and Linux, open **Menu → Show logs** for selectable startup and live diagnostics. Logs are retained only in memory for the current run and are not written to a persistent log file.

Lovelock Companion remembers your setup, including the Lovense connection, all five vibration profiles, and ability filters, in your OS user config directory. Ability names are runtime diagnostics and are not saved.

## Provider/action architecture

`src/provider.rs` owns the Lovense connection snapshot, connected blocking client, toy targets, test action, execution, and disconnect. `src/action.rs` owns vibration settings, validation, immutable resolution, and safe summaries; `src/action_ui.rs` contains the explicit egui editor. `src/theme.rs` holds Lovelock Companion's visual identity: a pastel bubblegum-pink accent on a dusty-plum dark theme, paired with the Baloo 2 display font and Atkinson Hyperlegible body font. Event acceptance resolves an action before a bounded worker queue, so later UI edits cannot change queued work and provider calls never run on the egui thread.

Saved state is strict schema 7 JSON; anything else (including old multi-provider saves) resets to defaults and the old file is preserved alongside it as a backup rather than migrated, since Lovelock Companion is a from-scratch Lovense-only companion.

## Publishing a release

Lovelock Companion uses one lockstep Semantic Version for the companion, the DeadlockShock Panorama mod, Git tag, and GameBanana listing. Before publishing:

1. Choose the release version and update `companion/Cargo.toml` plus `MOD_VERSION` in `mod/panorama/scripts/death_http_bridge.js` together.
2. Run `bun test tests/death_http_bridge.test.js` and the affected companion tests; the bridge test verifies the cross-component version invariant.
3. Build and smoke-test the VPK separately on Windows, then publish the mod on [GameBanana](https://gamebanana.com/mods/700758) with the same version.
4. Push the matching tag only after the mod artifact/version is available:

```sh
git tag v<version>
git push origin v<version>
```

Drone verifies `DRONE_TAG == v<companion Cargo version>` and the emitted mod metadata before building companion artifacts. The current pipeline does not build or upload the VPK; do not claim a tagged release contains the addon unless it was built and verified separately.
