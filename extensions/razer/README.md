# Razer

Razer mice, keyboards and headsets in Settings through OpenRazer's daemon (D-Bus,
`org.razer` on the session bus): sensitivity, polling rate, battery, sleep time,
lighting effects and brightness.

## Pretend device

`RAZER_MOCK=basilisk-v3` answers the daemon's calls with a made-up Basilisk V3
(serial `PM1234`, 4,000 of 26,000 dpi, 1000 Hz, 86% and charging), for checking pages
without one. `RAZER_MOCK=no-daemon` has the Basilisk plugged in but the daemon not
answering and the account not in `plugdev`, which shows the setup page. Changes are
kept in `SETTINGS_EXTENSION_STATE`; nothing calls busctl, pkexec or systemctl while
it's on.
