"""Draws the app icon (a navy planner page with a gold band and binder rings).

Writes:
  assets/icon.ico                 Windows icon, 16-256 px
  assets/icon.png                 256 px, used as the page icon and on Linux
  packaging/macos/icon-1024.png   turned into icon.icns on a Mac when packaging
  assets/tray-template.rgba       36x36 black-on-clear menu-bar picture for macOS

Needs Pillow:  pip install pillow
"""
import os
from PIL import Image, ImageDraw

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
NAVY, GOLD, WHITE = (30, 48, 84, 255), (184, 134, 11, 255), (255, 255, 255, 255)


def app_icon(size):
    s = size * 4
    im = Image.new('RGBA', (s, s), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    pad, r = s * 0.06, s * 0.17
    box = (pad, pad * 1.4, s - pad, s - pad)
    d.rounded_rectangle(box, radius=r, fill=NAVY)
    band = Image.new('RGBA', (s, s), (0, 0, 0, 0))
    ImageDraw.Draw(band).rectangle((0, 0, s, pad * 1.4 + s * 0.27), fill=GOLD)
    mask = Image.new('L', (s, s), 0)
    ImageDraw.Draw(mask).rounded_rectangle(box, radius=r, fill=255)
    im.paste(band, (0, 0), Image.composite(band, Image.new('RGBA', (s, s)), mask).split()[3])
    w = max(s * 0.075, 4)
    for x in (0.32, 0.68):
        cx = s * x
        d.rounded_rectangle((cx - w * 0.7, pad * 0.4, cx + w * 0.7, pad * 1.4 + s * 0.17), radius=w * 0.7, fill=WHITE)
    for y, length in ((0.56, 0.38), (0.73, 0.52)):
        x0, yy = s * 0.24, s * y
        d.rounded_rectangle((x0, yy - w / 2, x0 + s * length, yy + w / 2), radius=w / 2, fill=WHITE)
    return im.resize((size, size), Image.LANCZOS)


def tray_template(size=36):
    s = size * 8
    im = Image.new('RGBA', (s, s), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    black = (0, 0, 0, 255)
    lw = s * 0.09
    box = (s * 0.1, s * 0.16, s * 0.9, s * 0.92)
    d.rounded_rectangle(box, radius=s * 0.14, outline=black, width=int(lw))
    d.rectangle((s * 0.1 + lw / 2, s * 0.16 + lw / 2, s * 0.9 - lw / 2, s * 0.36), fill=black)
    d.rounded_rectangle((s * 0.1, s * 0.16, s * 0.9, s * 0.4), radius=s * 0.14, fill=black)
    for x in (0.33, 0.67):  # binder rings poking out of the top, with a clear gap around them
        d.rounded_rectangle((s * x - lw * 1.3, s * 0.0, s * x + lw * 1.3, s * 0.34), radius=lw * 1.3, fill=(0, 0, 0, 0))
        d.rounded_rectangle((s * x - lw * 0.6, s * 0.02, s * x + lw * 0.6, s * 0.3), radius=lw * 0.6, fill=black)
    for y, length in ((0.56, 0.36), (0.74, 0.5)):
        d.rounded_rectangle((s * 0.25, s * y - lw / 2, s * (0.25 + length), s * y + lw / 2), radius=lw / 2, fill=black)
    return im.resize((size, size), Image.LANCZOS)


big = app_icon(1024)
os.makedirs(os.path.join(ROOT, 'packaging', 'macos'), exist_ok=True)
big.save(os.path.join(ROOT, 'packaging', 'macos', 'icon-1024.png'), optimize=True)
app_icon(256).save(os.path.join(ROOT, 'assets', 'icon.png'), optimize=True)
sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256]
app_icon(256).save(os.path.join(ROOT, 'assets', 'icon.ico'), sizes=[(n, n) for n in sizes],
                   append_images=[app_icon(n) for n in sizes])
with open(os.path.join(ROOT, 'assets', 'tray-template.rgba'), 'wb') as f:
    f.write(tray_template().tobytes())
print('icons written')
