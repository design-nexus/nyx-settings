# RGB lighting

Lighting for every device OpenRGB sees, through its SDK server on port 6742.

## Pretend devices

`RGB_MOCK=1` makes `pages`, `describe` and `set` use a pretend OpenRGB instead of the
real server: an ROG STRIX Z790-E motherboard (4 zones), Vengeance RGB × 2, an RTX 4080
SUPER, LL120 fans × 3 and a Basilisk V3. Changes only update the pretend state, kept in
`rgb-mock.json` in `SETTINGS_EXTENSION_STATE`. Nothing connects to OpenRGB while it's set.
