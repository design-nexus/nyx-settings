# NVIDIA

NVIDIA graphics in Settings, through `nvidia-smi`: temperature, use, memory, clocks
and power, the programs using the GPU and their memory, the power limit and
persistence mode. Fans are shown only: on Wayland the driver offers no way to set
them.

## Trying it without changing a real GPU

`NVIDIA_MOCK=rtx-5080-laptop` runs the page against a pretend GeForce RTX 5080 Laptop
GPU (driver 580.95, 61 °C, 38% in use, 9.2 of 16 GB, 84 W of a 150 W limit, with
Hyprland, nyx-ai-core and Firefox using it). It answers the `nvidia-smi` commands the
page runs instead of the real one; a power limit or persistence mode set from the page
is kept in `$SETTINGS_EXTENSION_STATE/nvidia-mock`, and nothing is sent to the GPU.
Any non-empty value turns it on.

```sh
SETTINGS_EXTENSION_STATE=/tmp/nvidia-state NVIDIA_MOCK=rtx-5080-laptop settings-nvidia describe gpu0
```
