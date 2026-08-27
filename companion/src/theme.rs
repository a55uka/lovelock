use egui::{Color32, Context, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Vec2, Visuals};

/// Named font family for cutesy, rounded display headings (Baloo 2), layered
/// over the accessible body copy font (Atkinson Hyperlegible).
pub fn heading_family() -> FontFamily {
    FontFamily::Name("heading".into())
}

/// A heading-styled [`egui::RichText`] using the rounded display font.
pub fn heading_text(text: impl Into<String>, size: f32) -> egui::RichText {
    egui::RichText::new(text).font(FontId::new(size, heading_family()))
}

/// Lovelock's signature accent: a soft, pastel bubblegum pink rather than a
/// harsh neon magenta.
pub const ACCENT: Color32 = Color32::from_rgb(255, 138, 189);
pub const ACCENT_BRIGHT: Color32 = Color32::from_rgb(255, 195, 224);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(107, 60, 92);
pub const SUCCESS: Color32 = Color32::from_rgb(150, 224, 183);
pub const WARNING: Color32 = Color32::from_rgb(245, 199, 137);
pub const DANGER: Color32 = Color32::from_rgb(240, 140, 158);
pub const NEUTRAL: Color32 = Color32::from_rgb(190, 176, 190);

/// A soft dusty-plum base rather than a stark near-black, to keep the pastel
/// accents feeling gentle instead of neon.
pub const BASE: Color32 = Color32::from_rgb(30, 22, 33);
pub const PANEL: Color32 = Color32::from_rgb(38, 28, 42);
pub const CARD: Color32 = Color32::from_rgb(46, 33, 49);
pub const CARD_RAISED: Color32 = Color32::from_rgb(56, 40, 59);
pub const STROKE: Color32 = Color32::from_rgb(78, 55, 74);
pub const TEXT: Color32 = Color32::from_rgb(248, 240, 245);
pub const TEXT_DIM: Color32 = Color32::from_rgb(196, 180, 194);
/// Dot color for the subtle polka-dot background texture.
pub const DOT: Color32 = Color32::from_rgb(70, 50, 66);

pub fn apply(ctx: &Context) {
    let mut visuals = Visuals::dark();
    visuals.override_text_color = Some(TEXT);
    visuals.panel_fill = BASE;
    visuals.window_fill = PANEL;
    visuals.extreme_bg_color = Color32::from_rgb(8, 5, 9);
    visuals.faint_bg_color = CARD;
    visuals.code_bg_color = CARD;
    visuals.hyperlink_color = ACCENT_BRIGHT;
    visuals.selection.bg_fill = ACCENT_DIM;
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    visuals.window_stroke = Stroke::new(1.0, STROKE);

    visuals.widgets.noninteractive.bg_fill = PANEL;
    visuals.widgets.noninteractive.weak_bg_fill = PANEL;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, STROKE);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);

    visuals.widgets.inactive.bg_fill = CARD_RAISED;
    visuals.widgets.inactive.weak_bg_fill = CARD_RAISED;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, STROKE);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);

    visuals.widgets.hovered.bg_fill = ACCENT_DIM;
    visuals.widgets.hovered.weak_bg_fill = ACCENT_DIM;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT);

    visuals.widgets.active.bg_fill = ACCENT;
    visuals.widgets.active.weak_bg_fill = ACCENT;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT_BRIGHT);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);

    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = CornerRadius::same(6);
    }

    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.set_visuals_of(egui::Theme::Dark, visuals);

    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.text_styles.insert(
            TextStyle::Heading,
            FontId::new(24.0, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Body,
            FontId::new(16.0, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Button,
            FontId::new(16.0, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Small,
            FontId::new(13.5, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Monospace,
            FontId::new(14.5, egui::FontFamily::Monospace),
        );
        style.spacing.item_spacing = Vec2::new(8.0, 8.0);
        style.spacing.button_padding = Vec2::new(12.0, 8.0);
    });
}

/// Registers Atkinson Hyperlegible (an accessibility-focused typeface designed
/// by the Braille Institute for maximum legibility) as the body UI font, Baloo
/// 2 (soft, rounded, cutesy) as the display heading font, and the Phosphor
/// icon font so `egui_phosphor::regular::*` glyphs render.
pub fn install_fonts(ctx: &Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "atkinson-hyperlegible".into(),
        egui::FontData::from_static(include_bytes!(
            "../assets/fonts/AtkinsonHyperlegible-Regular.ttf"
        ))
        .into(),
    );
    fonts.font_data.insert(
        "baloo2".into(),
        egui::FontData::from_static(include_bytes!("../assets/fonts/Baloo2-Variable.ttf")).into(),
    );
    if let Some(proportional) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        proportional.insert(0, "atkinson-hyperlegible".into());
    }
    fonts
        .families
        .insert(heading_family(), vec!["baloo2".into()]);
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    ctx.set_fonts(fonts);
}

/// A rounded, tinted card frame used to group related controls.
pub fn card(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::group(ui.style())
        .fill(CARD)
        .stroke(Stroke::new(1.0, STROKE))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(14.0)
}

/// A slightly brighter card with a soft pink glow, used for the currently
/// selected trigger row.
pub fn card_selected(ui: &egui::Ui) -> egui::Frame {
    card(ui)
        .fill(ACCENT_DIM.gamma_multiply(0.9))
        .stroke(Stroke::new(1.5, ACCENT))
        .shadow(egui::Shadow {
            offset: [0, 0],
            blur: 18,
            spread: 1,
            color: ACCENT.gamma_multiply(0.35),
        })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BadgeTone {
    Neutral,
    Success,
    Warning,
    Danger,
}
impl BadgeTone {
    fn color(self) -> Color32 {
        match self {
            Self::Neutral => NEUTRAL,
            Self::Success => SUCCESS,
            Self::Warning => WARNING,
            Self::Danger => DANGER,
        }
    }
}

/// A small colored status pill, replacing plain colored status text.
pub fn badge(ui: &mut egui::Ui, text: &str, tone: BadgeTone) {
    let color = tone.color();
    egui::Frame::NONE
        .fill(color.gamma_multiply(0.18))
        .stroke(Stroke::new(1.0, color))
        .corner_radius(CornerRadius::same(255))
        .inner_margin(egui::Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(7.0), egui::Sense::hover());
                ui.painter().circle_filled(rect.center(), 3.5, color);
                ui.colored_label(color, text);
            });
        });
}

/// Paints `text` in the normal body label style, shifted down by `offset`
/// pixels. The space reserved in the layout is the plain, un-shifted label
/// size, so sibling widgets in the same horizontal row lay out and
/// vertically center exactly as if this were a normal `ui.label` — only the
/// painted glyphs move, which keeps the nudge predictable regardless of
/// what else is in the row.
pub fn label_nudged_down(ui: &mut egui::Ui, text: &str, offset: f32) -> egui::Response {
    colored_text_nudged_down(ui, text, ui.visuals().text_color(), offset)
}

/// Same as [`label_nudged_down`], but with an explicit color — used to nudge
/// icon glyphs (which are painted via `colored_label`) so they can be
/// vertically matched to an adjacent heading without disturbing layout.
pub fn colored_text_nudged_down(
    ui: &mut egui::Ui,
    text: &str,
    color: Color32,
    offset: f32,
) -> egui::Response {
    let font_id = TextStyle::Body.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font_id, color);
    let (rect, response) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
    ui.painter()
        .galley(rect.left_top() + Vec2::new(0.0, offset), galley, color);
    response
}

/// A round icon badge (colored ring + centered glyph), used for trigger rows.
pub fn icon_badge(ui: &mut egui::Ui, glyph: &str, active: bool) -> egui::Response {
    icon_badge_sized(ui, glyph, active, 36.0)
}

/// Same as [`icon_badge`], with an explicit diameter (used for the larger
/// effect-editor header icon).
pub fn icon_badge_sized(ui: &mut egui::Ui, glyph: &str, active: bool, size: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    let painter = ui.painter();
    let (fill, ring, glyph_color) = if active {
        (ACCENT_DIM, ACCENT, ACCENT_BRIGHT)
    } else {
        (CARD_RAISED, STROKE, TEXT_DIM)
    };
    painter.circle_filled(rect.center(), size / 2.0, fill);
    painter.circle_stroke(rect.center(), size / 2.0 - 0.5, Stroke::new(1.5, ring));
    painter.text(
        glyph_center(rect.center(), size * 0.5),
        egui::Align2::CENTER_CENTER,
        glyph,
        FontId::proportional(size * 0.5),
        glyph_color,
    );
    if active {
        let accent_pos = rect.center() + Vec2::splat(size * 0.32);
        let accent_size = (size * 0.22).max(7.0);
        painter.circle_filled(accent_pos, accent_size, PANEL);
        painter.text(
            glyph_center(accent_pos, accent_size),
            egui::Align2::CENTER_CENTER,
            egui_phosphor::regular::HEART_STRAIGHT,
            FontId::proportional(accent_size),
            ACCENT_BRIGHT,
        );
    }
    response
}

/// Nudges a glyph's anchor point down slightly to compensate for
/// [`egui::Painter::text`]'s `CENTER_CENTER` anchor using the font's full
/// ascent+descent row height rather than the glyph's visual ink bounds:
/// icon fonts otherwise render a few pixels above where they visually look
/// centered against a circle or line drawn around the same point.
fn glyph_center(point: egui::Pos2, font_size: f32) -> egui::Pos2 {
    point + Vec2::new(0.0, font_size * 0.07)
}

/// Small decorative heart glyph, used as a section-header bullet.
pub fn heart_bullet(ui: &mut egui::Ui) {
    ui.colored_label(ACCENT, egui_phosphor::regular::HEART);
}

/// Paints a soft, evenly-spaced polka-dot texture across `ui`'s current
/// rect, purely decorative and drawn behind whatever is added afterward.
pub fn paint_dotted_background(ui: &egui::Ui) {
    let rect = ui.max_rect();
    let spacing = 26.0;
    let radius = 1.6;
    let painter = ui.painter();
    let start_x = (rect.left() / spacing).floor() * spacing;
    let start_y = (rect.top() / spacing).floor() * spacing;
    let mut y = start_y;
    let mut row = 0_i32;
    while y < rect.bottom() {
        let offset = if row % 2 == 0 { 0.0 } else { spacing / 2.0 };
        let mut x = start_x + offset;
        while x < rect.right() {
            painter.circle_filled(egui::pos2(x, y), radius, DOT);
            x += spacing;
        }
        y += spacing;
        row += 1;
    }
}

/// A cute divider: a soft pink line fading from both edges into a small heart
/// at its center. Used under section headings for a bit of flourish.
pub fn flourish(ui: &mut egui::Ui) {
    let height = 14.0;
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), height),
        egui::Sense::hover(),
    );
    let painter = ui.painter();
    let mid = rect.center();
    let gap = 10.0;
    let segments = 24;
    for side in [-1.0_f32, 1.0] {
        for i in 0..segments {
            let t0 = i as f32 / segments as f32;
            let t1 = (i + 1) as f32 / segments as f32;
            let fade = 1.0 - t1;
            let x0 = mid.x + side * (gap + t0 * (rect.width() / 2.0 - gap));
            let x1 = mid.x + side * (gap + t1 * (rect.width() / 2.0 - gap));
            painter.line_segment(
                [egui::pos2(x0, mid.y), egui::pos2(x1, mid.y)],
                Stroke::new(1.0, ACCENT.gamma_multiply(fade.max(0.0) * 0.7)),
            );
        }
    }
    painter.text(
        glyph_center(mid, 11.0),
        egui::Align2::CENTER_CENTER,
        egui_phosphor::regular::HEART_STRAIGHT,
        FontId::proportional(11.0),
        ACCENT_BRIGHT,
    );
}

/// A faint sparkle accent glyph, for a touch of whimsy near headings.
pub fn sparkle(ui: &mut egui::Ui, size: f32) {
    ui.add(
        egui::Label::new(
            egui::RichText::new(egui_phosphor::regular::SPARKLE)
                .size(size)
                .color(ACCENT_BRIGHT.gamma_multiply(0.8)),
        )
        .selectable(false),
    );
}
