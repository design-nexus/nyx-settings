# Webcams

USB webcams in Settings, through `v4l2-ctl`: one page per camera, with a row for
every control the camera reports, a live preview, the microphone's level and mic
options (never idle, the default mic, unmuted). OBSBOT Tiny 3 cameras can also be
kept awake and put to sleep, through their UVC extension unit (`obsbot.py`).

When Settings can draw a preview pane, the controls sit on the left and the preview,
framing, saved positions and mic level on the right.

## Trying it without a camera

`WEBCAM_MOCK=obsbot-tiny-3-lite` shows a pretend OBSBOT Tiny 3 Lite instead of the
real cameras. It answers like `v4l2-ctl`, the OBSBOT sleep commands, `pw-dump` and
`wpctl`, and keeps its controls, mic options and saved positions in
`$SETTINGS_EXTENSION_STATE` (a scratch folder if that isn't set). It never opens a
device, writes the WirePlumber rule or starts the guard service. Its preview and
level point at a device and a PipeWire source that don't exist, so Settings shows
its own "no picture".

```sh
WEBCAM_MOCK=obsbot-tiny-3-lite SETTINGS_EXTENSION_STATE=/tmp/webcam-mock ./settings-webcam pages
```

Tests: `python3 test_webcam.py`.
