# Laptop

Battery health and charge limits, power profile, keyboard light, and microphone and
camera switches in Settings, for laptops of any brand, through the kernel's standard
controls. On an ASUS with asusctl the ASUS extension does all this, so this page stays
hidden (`NYX_LAPTOP_SHOW=1` shows it anyway).

## Trying it without the laptop

`LAPTOP_MOCK=framework-13` runs the page against a pretend Framework Laptop 13 (78%
and charging, 92% health, 214 cycles, charging between 75% and 80%, a three-step
keyboard light, the three power profiles, a microphone and a camera). Its sysfs files
are a scratch tree in `$SETTINGS_EXTENSION_STATE/laptop-mock`, made the first time;
changes from the page (limits, profile, keyboard light, microphone, camera) are
written there and nothing real is read or changed. Any non-empty value turns it on;
delete the folder to start over.

```sh
SETTINGS_EXTENSION_STATE=/tmp/laptop-state LAPTOP_MOCK=framework-13 settings-laptop describe laptop
```
