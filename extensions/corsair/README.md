# Corsair

Lighting for Corsair keyboards, mice, headsets, memory and fan hubs (through OpenRGB),
with device details and access for Corsair Web Hub.

## Pretend devices

`CORSAIR_MOCK=galleon-100-sd` simulates a Galleon 100 SD and its built-in Stream Deck on
USB (add `,denied` for a keyboard this account can't open), and a pretend OpenRGB with
the Galleon's lighting and a pair of Vengeance RGB sticks. While it's set nothing
connects to OpenRGB or the USB devices: lighting changes only update the pretend state,
kept in `corsair-mock.json` in `SETTINGS_EXTENSION_STATE`.
