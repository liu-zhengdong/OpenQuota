//! Headless design sheet for the macOS menu bar styles. It renders the real compact, bars and text
//! icons onto mock light and dark menu bars without launching the app or touching the screen:
//!
//! `cargo test --lib menu_bar::preview -- --ignored`
//!
//! The sheet lands at `OPENQUOTA_MENU_BAR_PREVIEW` or `target/menu-bar-preview.png`. Labels use
//! the macOS system Hiragino Sans GB font, so this only runs on a Mac.

use std::{fs::File, io::BufWriter, path::PathBuf};

use fontdue::{Font, FontSettings};
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Rect, Transform};

use super::{
    render_bar_rgba, render_compact_strip, render_text_strip, CompactGroup, RenderedStrip, Rgb,
    TextGroup, COMPACT_CAUTION_COLOR, COMPACT_CRITICAL_COLOR, COMPACT_HEALTHY_COLOR,
    COMPACT_UNBOUNDED_COLOR, ICON_SIZE, MAX_BARS, TEXT_HEIGHT,
};
use crate::quota_tier::quota_tier;

const LABEL_FONT: &str = "/System/Library/Fonts/Hiragino Sans GB.ttc";

// Everything is in @2x pixels, like the strips themselves. A non-notched menu bar is 24pt tall.
const BAR_HEIGHT: u32 = 48;
const BAR_PADDING: f32 = 24.0;
const STRIP_CLOCK_GAP: f32 = 28.0;
const CLOCK_TEXT: &str = "周一 14:51";
const CLOCK_SIZE: f32 = 26.0;
const MARGIN: f32 = 48.0;
const TITLE_COLUMN: f32 = 420.0;
const ROW_NAME_COLUMN: f32 = 250.0;
const COLUMN_GAP: f32 = 32.0;
const ROW_GAP: f32 = 16.0;
const SCENARIO_GAP: f32 = 40.0;

const PAGE: Rgb = (255, 255, 255);
const INK: Rgb = (29, 29, 31);
const SECONDARY_INK: Rgb = (110, 110, 115);
const DARK_BAR: Rgb = (40, 40, 42);
const LIGHT_BAR: Rgb = (236, 236, 238);

struct Theme {
    name: &'static str,
    bar: Rgb,
    // What the system tints a template image (the text style) to on this bar.
    template_tint: Rgb,
    template_alpha: f32,
}

const THEMES: [Theme; 2] = [
    Theme {
        name: "深色菜单栏",
        bar: DARK_BAR,
        template_tint: (255, 255, 255),
        template_alpha: 1.0,
    },
    Theme {
        name: "浅色菜单栏",
        bar: LIGHT_BAR,
        template_tint: (0, 0, 0),
        template_alpha: 0.85,
    },
];

/// One provider as the cloud reports it: the remaining share of its tightest pinned bounded
/// quota, or only a money/status value when nothing bounded is pinned.
enum Reading {
    Left(f64),
    Unbounded(&'static str),
}

struct Scenario {
    title: &'static str,
    providers: &'static [(&'static str, Reading)],
}

const SCENARIOS: [Scenario; 5] = [
    Scenario {
        title: "1 家",
        providers: &[("codex", Reading::Left(0.45))],
    },
    Scenario {
        title: "2 家",
        providers: &[
            ("claude", Reading::Left(0.82)),
            ("codex", Reading::Left(0.45)),
        ],
    },
    Scenario {
        title: "3 家：绿 / 橙 / 红",
        providers: &[
            ("claude", Reading::Left(0.82)),
            ("codex", Reading::Left(0.45)),
            ("cursor", Reading::Left(0.12)),
        ],
    },
    Scenario {
        title: "4 家，含一家灰（只钉了金额）",
        providers: &[
            ("claude", Reading::Left(0.82)),
            ("codex", Reading::Left(0.45)),
            ("cursor", Reading::Left(0.12)),
            ("openrouter", Reading::Unbounded("$4.20")),
        ],
    },
    Scenario {
        title: "6 家（紧凑只画最紧的 4 家）",
        providers: &[
            ("claude", Reading::Left(0.82)),
            ("codex", Reading::Left(0.45)),
            ("cursor", Reading::Left(0.12)),
            ("copilot", Reading::Left(0.7)),
            ("zai", Reading::Left(0.3)),
            ("openrouter", Reading::Unbounded("$4.20")),
        ],
    },
];

struct Styles {
    compact: RenderedStrip,
    bars: RenderedStrip,
    text: RenderedStrip,
}

#[test]
#[ignore = "writes a design sheet PNG; run explicitly with --ignored"]
fn render_menu_bar_design_sheet() {
    let font_bytes = std::fs::read(LABEL_FONT).expect("macOS ships Hiragino Sans GB");
    let font = Font::from_bytes(font_bytes, FontSettings::default()).expect("label font parses");

    let rendered = SCENARIOS.iter().map(render_styles).collect::<Vec<_>>();
    let widest_strip = rendered
        .iter()
        .flat_map(|styles| [styles.compact.width, styles.bars.width, styles.text.width])
        .max()
        .unwrap_or(0) as f32;
    let clock_width = measure(&font, CLOCK_TEXT, CLOCK_SIZE);
    let bar_width = (BAR_PADDING * 2.0 + widest_strip + STRIP_CLOCK_GAP + clock_width).ceil();

    let header_height = 150.0;
    let scenario_height = BAR_HEIGHT as f32 * 3.0 + ROW_GAP * 2.0;
    let width = (MARGIN * 2.0 + TITLE_COLUMN + ROW_NAME_COLUMN + (bar_width + COLUMN_GAP) * 2.0
        - COLUMN_GAP)
        .ceil();
    let height = (MARGIN * 2.0
        + header_height
        + scenario_height * SCENARIOS.len() as f32
        + SCENARIO_GAP * (SCENARIOS.len() - 1) as f32)
        .ceil();
    let mut sheet = Pixmap::new(width as u32, height as u32).expect("sheet dimensions are valid");
    fill_rect(&mut sheet, 0.0, 0.0, width, height, PAGE);

    draw_label(
        &mut sheet,
        &font,
        "菜单栏图标样式对比 · 无头渲染稿（@2x 实际像素，行名后为图标宽度）",
        30.0,
        MARGIN,
        MARGIN + 30.0,
        INK,
    );
    draw_legend(&mut sheet, &font, MARGIN, MARGIN + 80.0);
    let column_x = |index: usize| {
        MARGIN + TITLE_COLUMN + ROW_NAME_COLUMN + index as f32 * (bar_width + COLUMN_GAP)
    };
    for (index, theme) in THEMES.iter().enumerate() {
        draw_label(
            &mut sheet,
            &font,
            theme.name,
            24.0,
            column_x(index),
            MARGIN + 136.0,
            SECONDARY_INK,
        );
    }

    let mut y = MARGIN + header_height;
    for (scenario, styles) in SCENARIOS.iter().zip(&rendered) {
        draw_label(
            &mut sheet,
            &font,
            scenario.title,
            26.0,
            MARGIN,
            y + 32.0,
            INK,
        );
        let rows = [
            ("紧凑（默认）", &styles.compact, false),
            ("条形", &styles.bars, true),
            ("文字", &styles.text, true),
        ];
        for (row, (name, strip, is_template)) in rows.iter().enumerate() {
            let row_y = y + row as f32 * (BAR_HEIGHT as f32 + ROW_GAP);
            draw_label(
                &mut sheet,
                &font,
                &format!("{name} · {}px", strip.width),
                22.0,
                MARGIN + TITLE_COLUMN,
                row_y + 32.0,
                SECONDARY_INK,
            );
            for (index, theme) in THEMES.iter().enumerate() {
                draw_bar(
                    &mut sheet,
                    &font,
                    theme,
                    strip,
                    *is_template,
                    column_x(index),
                    row_y,
                    bar_width,
                    clock_width,
                );
            }
        }
        y += scenario_height + SCENARIO_GAP;
    }

    let path = std::env::var_os("OPENQUOTA_MENU_BAR_PREVIEW")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/menu-bar-preview.png")
        });
    write_png(&sheet, &path);
    println!("menu bar design sheet: {}", path.display());
}

fn render_styles(scenario: &Scenario) -> Styles {
    let compact = scenario
        .providers
        .iter()
        .map(|(provider_id, reading)| CompactGroup {
            provider_id: (*provider_id).into(),
            tier: match reading {
                Reading::Left(remaining) => Some(quota_tier(*remaining)),
                Reading::Unbounded(_) => None,
            },
        })
        .collect::<Vec<_>>();
    let text = scenario
        .providers
        .iter()
        .map(|(provider_id, reading)| TextGroup {
            provider_id: (*provider_id).into(),
            values: vec![match reading {
                Reading::Left(remaining) => format!("{:.0}%", remaining * 100.0),
                Reading::Unbounded(value) => (*value).into(),
            }],
        })
        .collect::<Vec<_>>();
    let fractions = scenario
        .providers
        .iter()
        .filter_map(|(_, reading)| match reading {
            Reading::Left(remaining) => Some(*remaining),
            Reading::Unbounded(_) => None,
        })
        .take(MAX_BARS)
        .collect::<Vec<_>>();
    Styles {
        compact: render_compact_strip(&compact).expect("scenarios have providers"),
        bars: RenderedStrip {
            rgba: render_bar_rgba(&fractions),
            width: ICON_SIZE,
        },
        text: render_text_strip(&text).expect("scenarios have values"),
    }
}

fn draw_legend(sheet: &mut Pixmap, font: &Font, x: f32, baseline: f32) {
    let entries = [
        (COMPACT_HEALTHY_COLOR, "剩余 ≥60%"),
        (COMPACT_CAUTION_COLOR, "20%–60%"),
        (COMPACT_CRITICAL_COLOR, "≤20%"),
        (COMPACT_UNBOUNDED_COLOR, "钉住的指标没有上限额度"),
    ];
    let mut pen_x = x;
    for (color, label) in entries {
        fill_circle(sheet, pen_x + 10.0, baseline - 8.0, 10.0, color);
        pen_x += 30.0;
        draw_label(sheet, font, label, 22.0, pen_x, baseline, SECONDARY_INK);
        pen_x += measure(font, label, 22.0) + 36.0;
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_bar(
    sheet: &mut Pixmap,
    font: &Font,
    theme: &Theme,
    strip: &RenderedStrip,
    is_template: bool,
    x: f32,
    y: f32,
    width: f32,
    clock_width: f32,
) {
    fill_rect(sheet, x, y, width, BAR_HEIGHT as f32, theme.bar);
    // Status items sit immediately left of the clock, so the strip is right-aligned against it.
    let clock_x = x + width - BAR_PADDING - clock_width;
    let strip_x = clock_x - STRIP_CLOCK_GAP - strip.width as f32;
    let strip_y = y + (BAR_HEIGHT - TEXT_HEIGHT) as f32 / 2.0;
    let tint = is_template.then_some((theme.template_tint, theme.template_alpha));
    blit_strip(sheet, strip, strip_x as i32, strip_y as i32, tint);
    let clock_color = theme.template_tint;
    draw_label(
        sheet,
        font,
        CLOCK_TEXT,
        CLOCK_SIZE,
        clock_x,
        y + 33.0,
        clock_color,
    );
}

/// Source-over a demultiplied strip onto the opaque sheet. Template strips only carry coverage,
/// so they are recolored the way the system tints template images on each bar.
fn blit_strip(sheet: &mut Pixmap, strip: &RenderedStrip, x: i32, y: i32, tint: Option<(Rgb, f32)>) {
    for row in 0..TEXT_HEIGHT {
        for column in 0..strip.width {
            let source = ((row * strip.width + column) * 4) as usize;
            let pixel = &strip.rgba[source..source + 4];
            let (color, alpha) = match tint {
                Some((color, opacity)) => (color, pixel[3] as f32 / 255.0 * opacity),
                None => ((pixel[0], pixel[1], pixel[2]), pixel[3] as f32 / 255.0),
            };
            blend(sheet, x + column as i32, y + row as i32, color, alpha);
        }
    }
}

fn draw_label(
    sheet: &mut Pixmap,
    font: &Font,
    text: &str,
    size: f32,
    x: f32,
    baseline: f32,
    color: Rgb,
) {
    let mut pen_x = x;
    for character in text.chars() {
        let (metrics, bitmap) = font.rasterize(character, size);
        let glyph_x = (pen_x + metrics.xmin as f32).round() as i32;
        let glyph_y = (baseline - metrics.height as f32 - metrics.ymin as f32).round() as i32;
        for row in 0..metrics.height {
            for column in 0..metrics.width {
                let coverage = bitmap[row * metrics.width + column] as f32 / 255.0;
                blend(
                    sheet,
                    glyph_x + column as i32,
                    glyph_y + row as i32,
                    color,
                    coverage,
                );
            }
        }
        pen_x += metrics.advance_width;
    }
}

fn measure(font: &Font, text: &str, size: f32) -> f32 {
    text.chars()
        .map(|character| font.metrics(character, size).advance_width)
        .sum()
}

fn blend(sheet: &mut Pixmap, x: i32, y: i32, color: Rgb, alpha: f32) {
    if alpha <= 0.0 || x < 0 || y < 0 || x >= sheet.width() as i32 || y >= sheet.height() as i32 {
        return;
    }
    let offset = ((y as u32 * sheet.width() + x as u32) * 4) as usize;
    let data = sheet.data_mut();
    for (channel, source) in [color.0, color.1, color.2].into_iter().enumerate() {
        let target = data[offset + channel] as f32;
        data[offset + channel] = (source as f32 * alpha + target * (1.0 - alpha)).round() as u8;
    }
    data[offset + 3] = 255;
}

fn fill_rect(sheet: &mut Pixmap, x: f32, y: f32, width: f32, height: f32, color: Rgb) {
    let rect = Rect::from_xywh(x, y, width, height).expect("rect is valid");
    sheet.fill_rect(rect, &solid(color), Transform::identity(), None);
}

fn fill_circle(sheet: &mut Pixmap, cx: f32, cy: f32, radius: f32, color: Rgb) {
    let path = PathBuilder::from_circle(cx, cy, radius).expect("circle is valid");
    sheet.fill_path(
        &path,
        &solid(color),
        FillRule::Winding,
        Transform::identity(),
        None,
    );
}

fn solid(color: Rgb) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color_rgba8(color.0, color.1, color.2, 255);
    paint.anti_alias = true;
    paint
}

fn write_png(sheet: &Pixmap, path: &PathBuf) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("preview directory is writable");
    }
    let file = File::create(path).expect("preview file is writable");
    let mut encoder = png::Encoder::new(BufWriter::new(file), sheet.width(), sheet.height());
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut writer| writer.write_image_data(sheet.data()))
        .expect("preview PNG encodes");
}
