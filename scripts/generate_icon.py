"""从原创矢量形状生成多尺寸 Windows 图标；构建时直接使用仓库里的成品。"""
from pathlib import Path
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[1] / "assets"
ROOT.mkdir(exist_ok=True)
SCALE = 4
canvas = Image.new("RGBA", (256 * SCALE, 256 * SCALE))
draw = ImageDraw.Draw(canvas)
svg = ['<svg xmlns="http://www.w3.org/2000/svg" width="256" height="256" viewBox="0 0 256 256">']


def polygon(points, color):
    draw.polygon([(x * SCALE, y * SCALE) for x, y in points], fill=color)
    svg.append(f'<polygon points="{" ".join(f"{x},{y}" for x,y in points)}" fill="{color}"/>')


def line(points, color, width):
    draw.line([(x * SCALE, y * SCALE) for x, y in points], fill=color, width=width * SCALE)
    for x, y in (points[0], points[-1]):
        draw.ellipse(((x-width/2)*SCALE, (y-width/2)*SCALE, (x+width/2)*SCALE, (y+width/2)*SCALE), fill=color)
    svg.append(f'<polyline points="{" ".join(f"{x},{y}" for x,y in points)}" fill="none" stroke="{color}" stroke-width="{width}" stroke-linecap="round" stroke-linejoin="round"/>')


# 深色圆角底板保证草地方块和白色镜框在深浅主题、16 像素下都清楚。
draw.rounded_rectangle((8*SCALE, 8*SCALE, 248*SCALE, 248*SCALE), radius=48*SCALE, fill="#142b32")
svg.append('<rect x="8" y="8" width="240" height="240" rx="48" fill="#142b32"/>')
polygon([(43,94),(118,52),(193,94),(118,136)], "#65dda3")
polygon([(43,94),(118,136),(118,207),(43,165)], "#976345")
polygon([(118,136),(193,94),(193,165),(118,207)], "#674934")
polygon([(43,94),(118,136),(118,157),(94,143),(94,155),(73,143),(73,132),(43,115)], "#389b70")
polygon([(118,136),(193,94),(193,118),(169,131),(169,119),(148,131),(148,148),(118,165)], "#267f60")
polygon([(66,155),(82,164),(82,178),(66,169)], "#b5825c")
polygon([(91,177),(107,186),(107,198),(91,189)], "#805339")
polygon([(154,159),(169,150),(169,164),(154,173)], "#805b3b")
# 白色放大镜围住方块右侧；镜片留空，避免小尺寸出现模糊半透明细节。
line([(190,183),(216,213)], "#142b32", 27)
line([(190,183),(216,213)], "#effcf7", 15)
draw.ellipse((137*SCALE,130*SCALE,205*SCALE,198*SCALE), outline="#142b32", width=18*SCALE)
draw.ellipse((137*SCALE,130*SCALE,205*SCALE,198*SCALE), outline="#effcf7", width=9*SCALE)
svg.append('<circle cx="171" cy="164" r="25" fill="none" stroke="#142b32" stroke-width="18"/>')
svg.append('<circle cx="171" cy="164" r="29.5" fill="none" stroke="#effcf7" stroke-width="9"/>')
svg.append('</svg>')
(ROOT / "icon.svg").write_text("\n".join(svg) + "\n", encoding="utf-8")
image = canvas.resize((256, 256), Image.Resampling.LANCZOS)
image.save(ROOT / "icon-256.png")
image.save(ROOT / "icon.ico", sizes=[(n,n) for n in [16,24,32,48,64,128,256]])
print("已生成 SVG、256px PNG 和七种尺寸的 ICO")
