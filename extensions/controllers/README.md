# Game controllers

Xbox, PlayStation, Switch Pro, 8BitDo and other controllers the kernel drives, in
Settings: battery, a live button and stick tester, a rumble test, and the lightbar and
player lights on PlayStation controllers.

## Pretend controller

`CONTROLLERS_MOCK=dualsense` (or `xbox`) makes `pages`, `describe` and `set` use a
made-up controller instead of the real ones, for checking pages without one: a
DualSense on Bluetooth at 60% with ✕ and R1 held, the left stick moved and the right
trigger at 72%, a lightbar and four player lights (`xbox` has no lights). `set` only
changes the pretend lightbar and player lights, kept in `SETTINGS_EXTENSION_STATE`;
nothing touches real devices while it's on.
