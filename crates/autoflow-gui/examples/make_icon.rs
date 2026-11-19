//! Generates the AutoFlowMate icon: `cargo run -p autoflow-gui --example make_icon`.
//! Writes assets/autoflowmate.png (256 px) and assets/autoflowmate.ico (16-256 px).
// tiny-skia paints are configured field by field.
#![allow(clippy::field_reassign_with_default)]

use tiny_skia::*;

fn draw(size: u32) -> Pixmap {
    let mut pm = Pixmap::new(size, size).unwrap();
    let s = size as f32 / 256.0;
    let tf = Transform::from_scale(s, s);

    // Rounded-square background with a blue -> violet gradient.
    let bg = rounded_rect(8.0, 8.0, 240.0, 240.0, 56.0);
    let mut paint = Paint::default();
    paint.anti_alias = true;
    paint.shader = LinearGradient::new(
        Point::from_xy(0.0, 0.0),
        Point::from_xy(256.0, 256.0),
        vec![GradientStop::new(0.0, Color::from_rgba8(37, 99, 235, 255)), GradientStop::new(1.0, Color::from_rgba8(124, 58, 237, 255))],
        SpreadMode::Pad,
        Transform::identity(),
    )
    .unwrap();
    pm.fill_path(&bg, &paint, FillRule::Winding, tf, None);

    let mut white = Paint::default();
    white.anti_alias = true;
    white.set_color_rgba8(255, 255, 255, 255);

    // Two flow nodes.
    pm.fill_path(&rounded_rect(40.0, 44.0, 96.0, 50.0, 14.0), &white, FillRule::Winding, tf, None);
    pm.fill_path(&rounded_rect(120.0, 162.0, 96.0, 50.0, 14.0), &white, FillRule::Winding, tf, None);

    // Elbow connector from node 1 down and right into node 2, with an arrowhead.
    let mut pb = PathBuilder::new();
    pb.move_to(88.0, 94.0);
    pb.line_to(88.0, 187.0);
    pb.line_to(104.0, 187.0);
    let line = pb.finish().unwrap();
    let stroke = Stroke { width: 14.0, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Stroke::default() };
    pm.stroke_path(&line, &white, &stroke, tf, None);
    let mut pb = PathBuilder::new();
    pb.move_to(100.0, 168.0);
    pb.line_to(124.0, 187.0);
    pb.line_to(100.0, 206.0);
    pb.close();
    pm.fill_path(&pb.finish().unwrap(), &white, FillRule::Winding, tf, None);

    // Amber mouse pointer (automation) with a dark outline.
    let mut pb = PathBuilder::new();
    pb.move_to(158.0, 40.0);
    pb.line_to(158.0, 128.0);
    pb.line_to(178.0, 110.0);
    pb.line_to(192.0, 140.0);
    pb.line_to(206.0, 133.0);
    pb.line_to(192.0, 104.0);
    pb.line_to(218.0, 101.0);
    pb.close();
    let pointer = pb.finish().unwrap();
    let mut amber = Paint::default();
    amber.anti_alias = true;
    amber.set_color_rgba8(251, 191, 36, 255);
    pm.fill_path(&pointer, &amber, FillRule::Winding, tf, None);
    let mut outline = Paint::default();
    outline.anti_alias = true;
    outline.set_color_rgba8(30, 27, 75, 255);
    pm.stroke_path(&pointer, &outline, &Stroke { width: 6.0, line_join: LineJoin::Round, ..Stroke::default() }, tf, None);
    pm
}

fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Path {
    let mut pb = PathBuilder::new();
    let k = 0.5523 * r;
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish().unwrap()
}

/// tiny-skia stores premultiplied RGBA; ICO/PNG consumers want straight alpha.
fn unpremultiply(pm: &Pixmap) -> Vec<u8> {
    pm.pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect()
}

fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    std::fs::create_dir_all(&root).unwrap();
    draw(256).save_png(root.join("autoflowmate.png")).unwrap();

    let mut dir = ico::IconDir::new(ico::ResourceType::Icon);
    for size in [16u32, 20, 24, 32, 40, 48, 64, 128, 256] {
        let pm = draw(size);
        let img = ico::IconImage::from_rgba_data(size, size, unpremultiply(&pm));
        dir.add_entry(ico::IconDirEntry::encode(&img).unwrap());
    }
    dir.write(std::fs::File::create(root.join("autoflowmate.ico")).unwrap()).unwrap();
    println!("wrote {}", root.canonicalize().unwrap().display());
}
