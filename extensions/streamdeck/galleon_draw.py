"""Pictures for the Galleon 100 SD's deck: keys (160x160), the top screen
(720x384), the boot animation, the profile-switch glitch, and a theme made from
the wallpaper. Needs Pillow; everything returns PIL images, and `jpeg()` turns
one into what the deck takes.

Ported from galleon-deck (https://github.com/NLMP-DDHS/galleon-deck):

    MIT License

    Copyright (c) 2026 The Galleon-Deck contributors

    Permission is hereby granted, free of charge, to any person obtaining a copy
    of this software and associated documentation files (the "Software"), to deal
    in the Software without restriction, including without limitation the rights
    to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
    copies of the Software, and to permit persons to whom the Software is
    furnished to do so, subject to the following conditions:

    The above copyright notice and this permission notice shall be included in all
    copies or substantial portions of the Software.

    THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
    IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
    FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
    AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
    LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
    OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
    SOFTWARE.
"""

import colorsys
import io
import json
import os
import random
import re
import shlex
import subprocess
import time

from PIL import Image, ImageDraw, ImageFont

from galleon_config import KEYS, THEME_DEFAULTS, colour

KEY_PX = 160
LCD_W, LCD_H = 720, 384
BOOT_FPS = 30
BOOT_TITLE = "GALLEON DECK"
BOOT_LINES = ["DECK LINK 1B1C:2B18|OK", "PROFILES|LOADED", "KEYS|ENCRYPTED", "DIALS|READY"]


def jpeg(img):
    buf = io.BytesIO()
    img.save(buf, "JPEG", quality=92)
    return buf.getvalue()


_fonts = {}


def font(family, size):
    key = (family, size)
    if key not in _fonts:
        try:
            path = subprocess.run(["fc-match", "-f", "%{file}", family], capture_output=True, text=True, timeout=5).stdout
            _fonts[key] = ImageFont.truetype(path, size)
        except (OSError, subprocess.TimeoutExpired):
            _fonts[key] = ImageFont.load_default(size)
    return _fonts[key]


def fit(draw, text, family, size, max_w):
    """The largest font size <= size that fits text in max_w."""
    while size > 10:
        f = font(family, size)
        if draw.textlength(text, font=f) <= max_w:
            return f
        size -= 2
    return font(family, 10)


def draw_frame(d, box, theme, outline, width, tick=None):
    """A key's or the screen's outline in the theme's key_style."""
    if theme.get("key_style") != "hud":
        d.rounded_rectangle(box, radius=theme["radius"], outline=outline, width=width)
        return
    x0, y0, x1, y1 = box
    cut = theme["radius"]
    d.polygon([(x0 + cut, y0), (x1, y0), (x1, y1 - cut), (x1 - cut, y1), (x0, y1), (x0, y0 + cut)],
              outline=outline, width=width)
    tick = colour(tick or theme.get("tick", "accent"), theme)
    arm = max(12, (x1 - x0) // 7)
    d.line([(x1 - arm, y0), (x1, y0), (x1, y0 + arm)], fill=tick, width=width + 1, joint="curve")
    d.line([(x0, y1 - arm), (x0, y1), (x0 + arm, y1)], fill=tick, width=width + 1, joint="curve")


def key(spec, theme, pressed=False, paths=None):
    bg = colour(spec.get("bg", theme["key_bg"]), theme)
    fg = colour(spec.get("fg", theme["fg"]), theme)
    if pressed:
        bg, fg = fg, bg
    img = Image.new("RGB", (KEY_PX, KEY_PX), bg)
    d = ImageDraw.Draw(img)
    if not spec:
        return img
    bw = theme["border_width"]
    draw_frame(d, [bw, bw, KEY_PX - 1 - bw, KEY_PX - 1 - bw], theme,
               colour(spec.get("border", theme["border"]), theme), bw, tick=spec.get("border"))
    label = str(spec.get("label", ""))
    icon = spec.get("icon")
    image = spec.get("image")
    if image:
        try:
            pic = Image.open(paths.resolve(image) if paths else os.path.expanduser(image)).convert("RGBA")
            pic.thumbnail((96, 96) if label else (124, 124))
            y = 16 if label else (KEY_PX - pic.height) // 2
            img.paste(pic, ((KEY_PX - pic.width) // 2, y), pic)
        except OSError:
            pass
    elif icon:
        f = font(theme["icon_font"], 72 if label else 96)
        d.text((KEY_PX // 2, 62 if label else KEY_PX // 2), icon, font=f, fill=fg, anchor="mm")
    if label:
        if icon or image:
            f = fit(d, label, theme["font"], 26, KEY_PX - 16)
            d.text((KEY_PX // 2, 128), label, font=f, fill=fg, anchor="mm")
        else:
            f = fit(d, label, theme["font"], 72 if len(label) <= 2 else 40, KEY_PX - 20)
            d.text((KEY_PX // 2, KEY_PX // 2), label, font=f, fill=fg, anchor="mm")
    return img


def theme_key(name, theme, current):
    """A key that shows a theme in its own colours and font."""
    img = Image.new("RGB", (KEY_PX, KEY_PX), theme["key_bg"])
    d = ImageDraw.Draw(img)
    bw = theme["border_width"] + (2 if current else 0)
    draw_frame(d, [bw, bw, KEY_PX - 1 - bw, KEY_PX - 1 - bw], theme, theme["accent"] if current else theme["border"], bw)
    for i, c in enumerate((theme["fg"], theme["accent"], theme["dim"])):
        d.ellipse([34 + i * 34, 34, 58 + i * 34, 58], fill=c)
    d.text((KEY_PX // 2, 100), name.upper(), font=fit(d, name.upper(), theme["font"], 28, KEY_PX - 20),
           fill=theme["fg"], anchor="mm")
    if current:
        d.text((KEY_PX // 2, 132), "ACTIVE", font=font(theme["font"], 16), fill=theme["accent"], anchor="mm")
    return img


def invert(img):
    return Image.eval(img, lambda v: 255 - v)


def _outline(d, theme):
    if theme.get("key_style") == "hud":
        draw_frame(d, [4, 4, LCD_W - 5, LCD_H - 5], {**theme, "radius": 28}, colour(theme.get("frame", "accent"), theme), 3)
    else:
        d.rectangle([4, 4, LCD_W - 5, LCD_H - 5], outline=colour(theme.get("frame", "accent"), theme), width=3)


def lcd(state, theme):
    """The top screen. state: profile, page, page_index, page_count, clock, date,
    and when they're shown: media, media_icon, volume (percent, muted), error."""
    img = Image.new("RGB", (LCD_W, LCD_H), theme["bg"])
    d = ImageDraw.Draw(img)
    fam, fg, accent, dim = theme["font"], theme["fg"], theme["accent"], theme["dim"]
    _outline(d, theme)

    if state.get("show_profile", True) or state.get("error"):
        head = f"{state['profile']} / {state['page']}".upper()
        d.text((28, 40), head, font=fit(d, head, fam, 32, 480), fill=accent, anchor="lm")
        n, cur = state["page_count"], state["page_index"]
        for i in range(n):
            x = LCD_W - 36 - (n - 1 - i) * 30
            d.ellipse([x - 9, 31, x + 9, 49], fill=accent if i == cur else None, outline=accent if i == cur else dim, width=2)
        d.line([24, 70, LCD_W - 24, 70], fill=dim, width=1)

    if state.get("error"):
        d.text((LCD_W // 2, 130), "CONFIG ERROR", font=font(fam, 44), fill=accent, anchor="mm")
        lines, line = [], ""
        for word in state["error"].split():
            if d.textlength(line + " " + word, font=font(fam, 20)) > LCD_W - 60:
                lines.append(line)
                line = word
            else:
                line = (line + " " + word).strip()
        lines.append(line)
        d.multiline_text((28, 180), "\n".join(lines[:6]), font=font(fam, 20), fill=fg, spacing=6)
        return img

    d.text((LCD_W // 2, 150), state["clock"], font=font(theme["clock_font"], 110), fill=fg, anchor="mm")
    d.text((LCD_W // 2, 232), state["date"], font=fit(d, state["date"], fam, 30, LCD_W - 60), fill=dim, anchor="mm")

    media = state.get("media")
    if media:
        f = fit(d, media, fam, 28, LCD_W - 110)
        d.text((56, 286), state.get("media_icon", ""), font=font(theme["icon_font"], 30), fill=accent, anchor="lm")
        d.text((96, 286), media, font=f, fill=fg, anchor="lm")

    vol, muted = state.get("volume") or (None, False)
    if vol is not None:
        x0, x1, y = 96, LCD_W - 130, 340
        r = 0 if theme.get("key_style") == "hud" else 9
        d.text((56, y), "\U000F075F" if muted else "\U000F057E", font=font(theme["icon_font"], 30),
               fill=dim if muted else accent, anchor="lm")
        d.rounded_rectangle([x0, y - 9, x1, y + 9], radius=r, outline=dim, width=2)
        fill_w = int((x1 - x0 - 6) * min(vol, 100) / 100)
        if fill_w > 0:
            d.rounded_rectangle([x0 + 3, y - 6, x0 + 3 + fill_w, y + 6], radius=r * 2 // 3, fill=dim if muted else fg)
        d.text((LCD_W - 40, y), "MUTE" if muted else f"{vol}%", font=font(fam, 26), fill=dim if muted else fg, anchor="rm")
    return img


def screen_logo(theme, logo, done):
    """The profile-switch screen: the logo fading in over the theme's background."""
    screen = Image.new("RGB", (LCD_W, LCD_H), theme["bg"]).convert("RGBA")
    layer = Image.new("RGBA", (LCD_W, LCD_H), (0, 0, 0, 0))
    layer.paste(logo, ((LCD_W - logo.width) // 2, (LCD_H - logo.height) // 2), logo)
    layer.putalpha(layer.getchannel("A").point(lambda v: int(v * min(1, done * 1.6))))
    screen.alpha_composite(layer)
    screen = screen.convert("RGB")
    _outline(ImageDraw.Draw(screen), theme)
    return screen


# ------------------------------------------------------------------ boot

def _scanlines():
    over = Image.new("RGBA", (LCD_W, LCD_H), (0, 0, 0, 0))
    d = ImageDraw.Draw(over)
    for y in range(0, LCD_H, 3):
        d.line([0, y, LCD_W, y], fill=(0, 0, 0, 70))
    return over


def _blend(a, b, t):
    ca, cb = (tuple(int(c[i:i + 2], 16) for i in (1, 3, 5)) for c in (a, b))
    return tuple(round(x + (y - x) * t) for x, y in zip(ca, cb))


def load_logo(path, theme, fallback, box=(LCD_W - 60, LCD_H - 80)):
    """A logo image trimmed to its content, or `fallback` drawn as a text logo."""
    if path:
        try:
            img = Image.open(path).convert("RGBA")
            img = img.crop(img.getchannel("A").getbbox() or (0, 0, *img.size))
            img.thumbnail(box)
            return img
        except OSError:
            pass
    img = Image.new("RGBA", box, (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    text = str(fallback).strip() or BOOT_TITLE
    d.text((img.width // 2, img.height // 2), text, font=fit(d, text, theme["clock_font"], 90, img.width - 20),
           fill=theme["fg"], anchor="mm")
    return img


def glitch(img, rng, amount):
    """Bands slide sideways, the colour channels separate, and now and then the
    frame flares. amount 0 leaves the image alone."""
    if amount <= 0.02:
        return img
    w, h = img.size
    out = Image.new("RGB", (w, h), (0, 0, 0))
    y = 0
    while y < h:
        band = rng.randint(max(2, h // 20), max(3, h // 5))
        dx = int(rng.uniform(-w * 0.25, w * 0.25) * amount) if rng.random() < 0.55 else 0
        out.paste(img.crop((0, y, w, y + band)), (dx, y))
        y += band
    r, g, b = out.split()
    r = r.transform(r.size, Image.AFFINE, (1, 0, -int(w * 0.05 * amount), 0, 1, 0))
    b = b.transform(b.size, Image.AFFINE, (1, 0, int(w * 0.03 * amount), 0, 1, 0))
    out = Image.merge("RGB", (r, g, b))
    if rng.random() < 0.3 * amount:
        out = Image.eval(out, lambda v: min(255, int(v * 1.5) + 20))
    return out


CIPHER = "0123456789ABCDEF#$%&*+=/<>?!"


class KeyDecrypt:
    """Keys show scrambling ciphertext, then decrypt one by one in random order."""

    BLOCK = 16

    def __init__(self, theme, real_keys, rng, mono, stretch):
        self.theme, self.real, self.rng = theme, real_keys, rng
        self.font = font(mono + ":bold", 26)
        self.order = rng.sample(range(KEYS), KEYS)
        self.gap = max(1, round(4 * stretch))
        self.length = max(2, round(12 * stretch))
        self.cells = (KEY_PX // self.BLOCK) ** 2
        self.reveal = {i: rng.sample(range(self.cells), self.cells) for i in range(KEYS)}
        self.start = None
        self.done = set()

    def cipher(self):
        th = self.theme
        img = Image.new("RGB", (KEY_PX, KEY_PX), th["key_bg"])
        d = ImageDraw.Draw(img)
        bw = th["border_width"]
        draw_frame(d, [bw, bw, KEY_PX - 1 - bw, KEY_PX - 1 - bw], th, th["border"], bw)
        for row in range(4):
            for col in range(4):
                fill = th["fg"] if self.rng.random() < 0.2 else th["dim"]
                d.text((26 + col * 36, 26 + row * 36), self.rng.choice(CIPHER), font=self.font, fill=fill, anchor="mm")
        return img

    def resolving(self, i, frac):
        n = int(self.cells * frac)
        per_row = KEY_PX // self.BLOCK
        mask = Image.new("L", (KEY_PX, KEY_PX), 0)
        hot = Image.new("L", (KEY_PX, KEY_PX), 0)
        dm, dh = ImageDraw.Draw(mask), ImageDraw.Draw(hot)
        fresh = max(1, self.cells // self.length)
        for k, cell in enumerate(self.reveal[i][:n]):
            y, x = divmod(cell, per_row)
            box = [x * self.BLOCK, y * self.BLOCK, (x + 1) * self.BLOCK - 1, (y + 1) * self.BLOCK - 1]
            dm.rectangle(box, fill=255)
            if k >= n - fresh:
                dh.rectangle(box, fill=150)
        img = Image.composite(self.real(i), self.cipher(), mask)
        return Image.composite(Image.new("RGB", img.size, self.theme["accent"]), img, hot)

    def frame(self, f):
        out = {}
        scramble = f % 3 == 0
        for i in range(KEYS):
            if i in self.done:
                continue
            if self.start is None or f < self.start + self.order.index(i) * self.gap:
                if scramble:
                    out[i] = self.cipher()
                continue
            t = f - self.start - self.order.index(i) * self.gap
            if t < self.length:
                out[i] = self.resolving(i, (t + 1) / self.length)
            elif t == self.length:
                out[i] = Image.eval(self.real(i), lambda v: min(255, int(v * 1.7) + 40))
            else:
                out[i] = "real"
                self.done.add(i)
        return out


def boot_frames(theme, boot, footer, real_keys, logo_path=None):
    """Yield (screen, {key: image | "real"}, seconds): a CRT power-on, a typed boot
    log over ciphered keys, the logo glitching in while the keys decrypt."""
    speed = boot.get("speed", 1.0)
    stretch = 1 / max(0.1, float(speed))

    def frames(n):
        return max(1, round(n * stretch))

    bg, fg, accent = theme["bg"], theme["fg"], theme["accent"]
    frame_col = colour(theme.get("frame", "accent"), theme)
    scan = _scanlines()
    mono = boot.get("font", "JetBrains Mono")
    rng = random.Random()
    keys = KeyDecrypt(theme, real_keys, rng, mono, stretch)
    kf = 0
    tick = 1 / BOOT_FPS

    def finish(img):
        img = img.convert("RGBA")
        img.alpha_composite(scan)
        return img.convert("RGB")

    n = frames(12)
    for f in range(n):
        t = (f + 1) / n
        img = Image.new("RGB", (LCD_W, LCD_H), "#000000")
        d = ImageDraw.Draw(img)
        w = LCD_W * min(1, t * 2)
        h = 2 if t <= 0.5 else max(2, LCD_H * (t - 0.5) * 2)
        glow = _blend("#ffffff", bg, max(0, t - 0.5) * 2)
        d.rectangle([(LCD_W - w) / 2, (LCD_H - h) / 2, (LCD_W + w) / 2, (LCD_H + h) / 2], fill=glow)
        yield finish(img), {}, tick

    title = boot.get("title", BOOT_TITLE)
    rows = []
    for line in boot.get("lines", BOOT_LINES)[:6]:
        label, _, status = str(line).partition("|")
        rows.append((f"> {label} ".ljust(28, ".") + " ", status))
    f_title, f_line = font(mono + ":bold", 30), font(mono, 24)
    total = len(title) + sum(len(a) + len(b) for a, b in rows)
    step = total / frames(40)
    shown, f = 0.0, 0
    while shown < total + step * frames(6):
        shown += step
        f += 1
        img = Image.new("RGB", (LCD_W, LCD_H), bg)
        d = ImageDraw.Draw(img)
        d.rectangle([4, 4, LCD_W - 5, LCD_H - 5], outline=frame_col, width=3)
        left = int(shown)
        d.text((32, 40), title[:left], font=f_title, fill=accent, anchor="lm")
        cursor = (32 + d.textlength(title[:left], font=f_title), 40)
        left -= len(title)
        y = 100
        for text, status in rows:
            if left <= 0:
                break
            d.text((32, y), text[:left], font=f_line, fill=fg, anchor="lm")
            x = 32 + d.textlength(text[:left], font=f_line)
            part = status[:max(0, left - len(text))]
            if part:
                d.text((x, y), part, font=f_line, fill=accent, anchor="lm")
                x += d.textlength(part, font=f_line)
            cursor = (x, y)
            left -= len(text) + len(status)
            y += 44
        if (f // frames(1)) % 16 < 10:
            d.rectangle([cursor[0] + 4, cursor[1] - 12, cursor[0] + 18, cursor[1] + 12], fill=fg)
        yield finish(img), keys.frame(kf), tick
        kf += 1

    logo = load_logo(logo_path, theme, title.split("//")[0].strip() or title)
    keys.start = kf
    n = frames(36)
    img = Image.new("RGB", (LCD_W, LCD_H), bg)
    for f in range(n):
        p = (f + 1) / n
        g = (1 - p) ** 2
        img = Image.new("RGB", (LCD_W, LCD_H), bg)
        layer = Image.new("RGBA", (LCD_W, LCD_H), (0, 0, 0, 0))
        layer.paste(logo, ((LCD_W - logo.width) // 2, 16), logo)
        if g > 0.02:
            out = Image.new("RGBA", layer.size, (0, 0, 0, 0))
            y = 0
            while y < LCD_H:
                band = rng.randint(6, 40)
                dx = int(rng.uniform(-60, 60) * g) if rng.random() < 0.6 else 0
                out.paste(layer.crop((0, y, LCD_W, y + band)), (dx, y))
                y += band
            r, gg, b, a = out.split()
            r = r.transform(r.size, Image.AFFINE, (1, 0, -int(14 * g), 0, 1, 0))
            layer = Image.merge("RGBA", (r, gg, b, a))
        alpha = layer.getchannel("A").point(lambda v: int(v * min(1, p * 1.6)))
        layer.putalpha(alpha)
        img = img.convert("RGBA")
        img.alpha_composite(layer)
        img = img.convert("RGB")
        if rng.random() < 0.6 * g:
            img = Image.eval(img, lambda v: min(255, int(v * 1.6)))
        yield finish(img), keys.frame(kf), tick
        kf += 1

    base = img
    f_foot = font(mono + ":bold", 26)
    f = 0
    while f < frames(len(footer) + 18) or len(keys.done) < KEYS:
        img = base.copy()
        d = ImageDraw.Draw(img)
        d.text((LCD_W // 2, LCD_H - 34), footer[:f // frames(1) + 1], font=f_foot, fill=accent, anchor="mm")
        yield finish(img), keys.frame(kf), tick
        kf += 1
        f += 1


def gif_frames(path, theme):
    """Yield (screen, {}, seconds) from a GIF/APNG/WebP, fitted to the top screen."""
    anim = Image.open(path)
    for index in range(getattr(anim, "n_frames", 1)):
        anim.seek(index)
        frame = anim.convert("RGBA")
        frame.thumbnail((LCD_W, LCD_H))
        img = Image.new("RGB", (LCD_W, LCD_H), theme["bg"])
        img.paste(frame, ((LCD_W - frame.width) // 2, (LCD_H - frame.height) // 2), frame)
        yield img, {}, anim.info.get("duration", 1000 / BOOT_FPS) / 1000


# ------------------------------------------------------- wallpaper theme

def _hex(rgb):
    return "#%02x%02x%02x" % tuple(max(0, min(255, round(c))) for c in rgb)


def _from_hsv(h, s, v):
    return _hex(c * 255 for c in colorsys.hsv_to_rgb(h % 1, max(0, min(1, s)), max(0, min(1, v))))


def _luminance(c):
    def lin(x):
        x /= 255
        return x / 12.92 if x <= 0.03928 else ((x + 0.055) / 1.055) ** 2.4
    r, g, b = (int(c[i:i + 2], 16) for i in (1, 3, 5))
    return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)


def _contrast(a, b):
    la, lb = sorted((_luminance(a), _luminance(b)), reverse=True)
    return (la + 0.05) / (lb + 0.05)


def _hue_gap(a, b):
    d = abs(a - b) % 1
    return min(d, 1 - d)


WALLPAPER_COMMANDS = ["noctalia msg wallpaper-get", "swww query", "hyprctl hyprpaper listactive",
                      "gsettings get org.gnome.desktop.background picture-uri-dark",
                      "gsettings get org.gnome.desktop.background picture-uri"]


def wallpaper_path(settings):
    """Omarchy's current background, or the configured command's answer, or the
    first common wallpaper tool that names an existing image."""
    omarchy = os.path.expanduser("~/.config/omarchy/current/background")
    if not settings.get("command") and os.path.exists(omarchy):
        return os.path.realpath(omarchy)
    for command in [settings["command"]] if settings.get("command") else WALLPAPER_COMMANDS:
        try:
            out = subprocess.run(shlex.split(command), capture_output=True, text=True, timeout=1).stdout
        except (OSError, ValueError, subprocess.TimeoutExpired):
            continue
        for token in re.findall(r"(?:file://)?(/[^\s'\",]+\.(?:png|jpe?g|webp|bmp|gif))", out, re.I):
            path = os.path.expanduser(token.replace("file://", ""))
            if os.path.isfile(path):
                return path
    return None


def theme_from_image(path, base=None):
    """Its most vivid colours for text and accents, a dark tint of its overall
    colour for the background, with the text kept readable."""
    im = Image.open(path).convert("RGB")
    im.thumbnail((320, 320))
    raw = im.tobytes()
    px = [tuple(raw[i:i + 3]) for i in range(0, len(raw), 3)]
    hsv = [colorsys.rgb_to_hsv(r / 255, g / 255, b / 255) for r, g, b in px]

    def clusters(pixels, n):
        q = Image.new("RGB", (len(pixels), 1))
        q.putdata(pixels)
        q = q.quantize(colors=n, method=Image.Quantize.MEDIANCUT)
        pal = q.getpalette()
        return [(count, colorsys.rgb_to_hsv(*(c / 255 for c in pal[i * 3:i * 3 + 3]))) for count, i in q.getcolors()]

    vivid = [p for p, (h, sat, val) in zip(px, hsv) if sat > 0.45 and val > 0.35]
    if len(vivid) < 50:
        vivid = [p for _, p in sorted(zip((sat * val for _, sat, val in hsv), px))[-500:]]
    ranked = sorted(clusters(vivid, 8), key=lambda c: c[0] * c[1][1] * c[1][2], reverse=True)
    picks = []
    for _, col in ranked:
        if all(_hue_gap(col[0], p[0]) > 0.07 for p in picks):
            picks.append(col)
    fg_h, fg_s, fg_v = picks[0]
    ac_h, ac_s, ac_v = picks[1] if len(picks) > 1 else (fg_h + 0.08, fg_s * 0.6, 1.0)
    fr_h, fr_s, fr_v = picks[2] if len(picks) > 2 else (ac_h, ac_s, ac_v)
    _, (bg_h, bg_s, _) = max(clusters(px, 8), key=lambda c: c[0])
    if bg_s < 0.15:
        bg_h, bg_s = fg_h, 0.3
    bg_s = min(0.8, max(0.35, bg_s))
    theme = {**(base or THEME_DEFAULTS)}
    theme.update(fg=_from_hsv(fg_h, max(fg_s, 0.75), max(fg_v, 0.92)),
                 accent=_from_hsv(ac_h, max(ac_s, 0.6), max(ac_v, 0.85)),
                 frame=_from_hsv(fr_h, max(fr_s, 0.7), max(fr_v, 0.8)),
                 dim=_from_hsv(bg_h, 0.18, 0.58), border=_from_hsv(fg_h, 0.75, 0.38))
    v = 0.11
    while True:
        theme["bg"], theme["key_bg"] = _from_hsv(bg_h, bg_s, v), _from_hsv(bg_h, bg_s, v + 0.04)
        if _contrast(theme["fg"], theme["key_bg"]) >= 4.5 or v <= 0.03:
            break
        v -= 0.02
    return theme


def write_wallpaper_theme(paths, settings, image=None):
    """themes/wallpaper.toml from the wallpaper (or `image`). Returns its path or None."""
    image = image or wallpaper_path(settings)
    if not image:
        return None
    base = {**THEME_DEFAULTS, **{k: settings[k] for k in ("font", "clock_font", "radius", "border_width") if k in settings}}
    theme = theme_from_image(image, base)
    os.makedirs(paths.themes, exist_ok=True)
    out = os.path.join(paths.themes, "wallpaper.toml")
    lines = ["# Made from your wallpaper by nyx-deck; made again when the wallpaper changes",
             "# (with [wallpaper_theme] auto = true) or from Settings. Edits here are",
             "# overwritten; copy this file under another name to keep a version.",
             f"# {time.strftime('%Y-%m-%d %H:%M')}", "",
             f"source = {json.dumps(image)}", f"source_mtime = {int(os.stat(image).st_mtime)}"]
    for k in ("bg", "key_bg", "fg", "accent", "frame", "dim", "border", "radius", "border_width", "font", "clock_font"):
        lines.append(f"{k} = {json.dumps(theme[k])}")
    with open(out + ".tmp", "w") as f:
        f.write("\n".join(lines) + "\n")
    os.replace(out + ".tmp", out)
    return out
