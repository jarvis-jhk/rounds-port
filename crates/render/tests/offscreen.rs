//! Offscreen rendering of known scenes, checked pixel by pixel.
//!
//! Needs a GPU adapter (any Vulkan/GL device, or lavapipe). Without one the
//! tests print a note and pass, so CI without GPUs is not blocked.

use glam::{Affine2, Vec2};
use rounds_render::*;

const W: u32 = 160;
const H: u32 = 90;

fn setup() -> Option<(Gpu, Renderer, OffscreenTarget)> {
    let gpu = match Gpu::headless() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("skipping: {e}");
            return None;
        }
    };
    let renderer = Renderer::new(&gpu, OFFSCREEN_FORMAT);
    let target = OffscreenTarget::new(&gpu, (W, H), OFFSCREEN_FORMAT);
    Some((gpu, renderer, target))
}

fn close(a: [u8; 4], b: [u8; 4], tol: i32) -> bool {
    a.iter()
        .zip(b)
        .all(|(&x, y)| (x as i32 - y as i32).abs() <= tol)
}

#[track_caller]
fn assert_px(img: &Image, x: u32, y: u32, want: [u8; 4]) {
    let got = img.pixel(x, y);
    assert!(
        close(got, want, 2),
        "pixel ({x},{y}) = {got:?}, want {want:?}"
    );
}

fn save(img: &Image, name: &str) {
    // Keep the output inside the build directory for inspection.
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    img.save_png(dir.join(name)).unwrap();
}

/// 10 px per world unit, origin at the centre of the 160x90 target.
fn frame() -> FrameSettings {
    FrameSettings {
        camera: Camera2D::new(Vec2::ZERO, 4.5),
        clear: Color::hex(0x203040),
        ..Default::default()
    }
}

#[test]
fn shapes_land_where_expected() {
    let Some((gpu, mut r, target)) = setup() else {
        return;
    };
    let mut dl = DrawList::new();
    // Red 2x2 square centred at (-4, 0): pixels x 30..50, y 35..55.
    dl.rect(
        Layer::World,
        0,
        Vec2::new(-4.0, 0.0),
        Vec2::splat(2.0),
        0.0,
        Color::RED,
    );
    // Green circle radius 1.5 at (4, 1): centre pixel (120, 35).
    dl.circle(Layer::World, 0, Vec2::new(4.0, 1.0), 1.5, Color::GREEN);
    // Blue triangle polygon (mesh path) around (0, -3).
    dl.polygon(
        Layer::World,
        0,
        &[
            Vec2::new(-1.0, -4.0),
            Vec2::new(1.0, -4.0),
            Vec2::new(0.0, -2.0),
        ],
        Color::BLUE,
    );
    // Sort keys: a white square drawn first but with a higher key ends on top.
    dl.rect(
        Layer::World,
        5,
        Vec2::new(-4.0, 0.0),
        Vec2::splat(0.6),
        0.0,
        Color::WHITE,
    );
    dl.rect(
        Layer::World,
        1,
        Vec2::new(-4.0, 0.0),
        Vec2::splat(1.0),
        0.0,
        Color::BLACK,
    );
    // Screen layer: yellow 10x10 at pixel (5, 5), pivot top-left.
    dl.push(
        Layer::Screen,
        0,
        Primitive::Rect {
            transform: Affine2::from_translation(Vec2::new(5.0, 5.0)),
            size: Vec2::splat(10.0),
            pivot: Vec2::ZERO,
            corner_radius: 0.0,
            color: Color::rgb(1.0, 1.0, 0.0),
        },
    );
    // Half-transparent white over the clear colour blends in linear light.
    dl.rect(
        Layer::World,
        0,
        Vec2::new(6.5, -3.5),
        Vec2::splat(1.0),
        0.0,
        Color::WHITE.with_alpha(0.5),
    );
    r.render(&gpu, target.target(), &frame(), &dl);
    let img = target.capture(&gpu).unwrap();
    save(&img, "shapes.png");

    let bg = [0x20, 0x30, 0x40, 255];
    assert_px(&img, 1, 88, bg);
    assert_px(&img, 32, 37, [255, 0, 0, 255]);
    assert_px(&img, 48, 53, [255, 0, 0, 255]);
    assert_px(&img, 28, 45, bg);
    assert_px(&img, 36, 45, [0, 0, 0, 255]); // black ring between white and red
    assert_px(&img, 40, 45, [255, 255, 255, 255]); // white on top (higher sort)
    assert_px(&img, 120, 35, [0, 255, 0, 255]);
    assert_px(&img, 120 + 13, 35, [0, 255, 0, 255]);
    assert_px(&img, 120 + 17, 35, bg);
    assert_px(&img, 80, 77, [0, 0, 255, 255]);
    assert_px(&img, 10, 10, [255, 255, 0, 255]);
    assert_px(&img, 16, 10, bg);
    // Linear-light blend: 0.5 * 1.0 + 0.5 * lin(bg), encoded back to sRGB.
    let mix = |c: u8| {
        (linear_to_srgb(0.5 + 0.5 * srgb_to_linear(c as f32 / 255.0)) * 255.0).round() as u8
    };
    assert_px(&img, 145, 80, [mix(0x20), mix(0x30), mix(0x40), 255]);

    let stats = r.stats();
    assert_eq!(stats.mesh_triangles, 1);
    assert!(stats.quads >= 6);
}

#[test]
fn sprite_orientation_and_batching() {
    let Some((gpu, mut r, target)) = setup() else {
        return;
    };
    // 2x2 texture: top-left red, top-right green, bottom-left blue, bottom-right white.
    let mut img = Image::new(2, 2);
    img.set_pixel(0, 0, [255, 0, 0, 255]);
    img.set_pixel(1, 0, [0, 255, 0, 255]);
    img.set_pixel(0, 1, [0, 0, 255, 255]);
    img.set_pixel(1, 1, [255, 255, 255, 255]);
    let tex = r.create_texture(
        &gpu,
        &img,
        TextureOptions {
            filter: Filter::Nearest,
            ..Default::default()
        },
    );
    let mut dl = DrawList::new();
    // World sprite 4x4 units centred at (-4, 0): pixels x 20..60, y 25..65.
    for i in 0..100 {
        // Many sprites with the same texture batch into one draw.
        dl.sprite(
            Layer::World,
            0,
            Sprite {
                texture: tex,
                transform: Affine2::from_translation(Vec2::new(-4.0, 0.0)),
                size: Vec2::splat(4.0 - i as f32 * 0.001),
                ..Default::default()
            },
        );
    }
    // Screen sprite 40x40 px at (100, 25), pivot top-left.
    dl.sprite(
        Layer::Screen,
        0,
        Sprite {
            texture: tex,
            transform: Affine2::from_translation(Vec2::new(100.0, 25.0)),
            size: Vec2::splat(40.0),
            pivot: Vec2::ZERO,
            ..Default::default()
        },
    );
    r.render(&gpu, target.target(), &frame(), &dl);
    let out = target.capture(&gpu).unwrap();
    save(&out, "sprites.png");
    for (ox, oy) in [(20, 25), (100, 25)] {
        assert_px(&out, ox + 10, oy + 10, [255, 0, 0, 255]);
        assert_px(&out, ox + 30, oy + 10, [0, 255, 0, 255]);
        assert_px(&out, ox + 10, oy + 30, [0, 0, 255, 255]);
        assert_px(&out, ox + 30, oy + 30, [255, 255, 255, 255]);
    }
    // 2 sprite batches + 3 fixed passes (composite, post, world).
    assert_eq!(r.stats().draw_calls, 2 + 3);
}

#[test]
fn text_renders_inside_its_box() {
    let Some((gpu, mut r, target)) = setup() else {
        return;
    };
    let mut dl = DrawList::new();
    dl.text(
        Layer::Screen,
        0,
        "ROUNDS",
        Vec2::new(80.0, 45.0),
        24.0,
        Vec2::splat(0.5),
        Color::WHITE,
    );
    let frame = FrameSettings {
        clear: Color::BLACK,
        ..frame()
    };
    r.render(&gpu, target.target(), &frame, &dl);
    let img = target.capture(&gpu).unwrap();
    save(&img, "text.png");
    let size = r.measure_text(FontId::DEFAULT, "ROUNDS", 24.0);
    let (x0, y0) = ((80.0 - size.x / 2.0) as u32, (45.0 - size.y / 2.0) as u32);
    let (x1, y1) = (
        (80.0 + size.x / 2.0) as u32 + 1,
        (45.0 + size.y / 2.0) as u32 + 1,
    );
    let mut inside = 0;
    for y in 0..H {
        for x in 0..W {
            let lit = img.pixel(x, y)[0] > 128;
            let in_box = x >= x0 && x <= x1 && y >= y0 && y <= y1;
            assert!(!lit || in_box, "text pixel outside its box at ({x},{y})");
            inside += lit as u32;
        }
    }
    assert!(inside > 150, "too few text pixels: {inside}");
}

#[test]
fn light_and_shadow() {
    let Some((gpu, mut r, target)) = setup() else {
        return;
    };
    let mut dl = DrawList::new();
    // A white floor everywhere, lit by a light at (-6, 0), with a box at
    // the centre casting a shadow towards +x.
    dl.rect(
        Layer::World,
        0,
        Vec2::ZERO,
        Vec2::new(16.0, 9.0),
        0.0,
        Color::WHITE,
    );
    let caster = [
        Vec2::new(-0.5, -1.0),
        Vec2::new(0.5, -1.0),
        Vec2::new(0.5, 1.0),
        Vec2::new(-0.5, 1.0),
    ];
    dl.shadow_caster(&caster);
    dl.light(Light {
        position: Vec2::new(-6.0, 0.0),
        range: 14.0,
        intensity: 2.0,
        source_radius: 0.2,
        ..Default::default()
    });
    let frame = FrameSettings {
        lighting: LightingSettings {
            enabled: true,
            ambient: Color::BLACK,
            exposure: 1.0,
            light_map_scale: 1,
            ..Default::default()
        },
        ..frame()
    };
    r.render(&gpu, target.target(), &frame, &dl);
    let img = target.capture(&gpu).unwrap();
    save(&img, "light.png");
    let lum = |x, y| img.pixel(x, y)[0] as i32;
    // In front of the box (x = -3): lit. Behind it (x = +3): umbra.
    let front = lum(50, 45);
    let behind = lum(110, 45);
    // Same distance from the light but outside the shadow (y = +3.5).
    let side = lum(110, 10);
    assert!(front > 150, "front {front}");
    assert!(behind < 10, "behind {behind}");
    assert!(side > 40, "side {side}");
    // Far corner beyond range is dark; edge counts reported.
    assert!(lum(159, 89) < 5);
    assert_eq!(r.stats().lights, 1);
    assert_eq!(r.stats().shadow_edges, 1);
}

#[test]
fn value_masks_clip_to_their_writer() {
    let Some((gpu, mut r, target)) = setup() else {
        return;
    };
    // A 2x1 texture: left texel opaque, right texel transparent.
    let mut img = Image::new(2, 1);
    img.set_pixel(0, 0, [255, 255, 255, 255]);
    img.set_pixel(1, 0, [255, 255, 255, 0]);
    let tex = r.create_texture(
        &gpu,
        &img,
        TextureOptions {
            filter: Filter::Nearest,
            ..Default::default()
        },
    );
    let mut dl = DrawList::new();
    // Writer: 4x2 units at the centre (x 60..100, y 35..55), only its left
    // half is opaque and writes value 7 (drawn red).
    dl.sprite(
        Layer::World,
        0,
        Sprite {
            texture: tex,
            size: Vec2::new(4.0, 2.0),
            tint: Color::RED,
            mask: Mask::WriteValue(7),
            ..Default::default()
        },
    );
    // A big green quad visible only where the stencil is 7, and a blue one
    // only where it is 8 (nowhere).
    for (v, c) in [(7, Color::GREEN), (8, Color::BLUE)] {
        dl.sprite(
            Layer::World,
            1,
            Sprite {
                size: Vec2::new(12.0, 6.0),
                tint: c,
                mask: Mask::InsideValue(v),
                ..Default::default()
            },
        );
    }
    r.render(&gpu, target.target(), &frame(), &dl);
    let out = target.capture(&gpu).unwrap();
    save(&out, "value_masks.png");
    assert_px(&out, 70, 45, [0, 255, 0, 255]); // inside the opaque half
    assert_px(&out, 90, 45, [0x20, 0x30, 0x40, 255]); // transparent half: no mask
    assert_px(&out, 20, 45, [0x20, 0x30, 0x40, 255]); // outside the writer
}
