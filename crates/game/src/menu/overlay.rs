//! Connection status overlay of online matches: one compact line per
//! remote machine (ping, direct / relayed / via host), rollback statistics,
//! and centred notices while synchronising or resyncing after a drop.

use glam::Vec2;
use rounds_render::{Color, DrawList, Layer};

use crate::play::assets::PlayAssets;
use crate::play::driver::NetStatus;
use crate::ui::{Canvas, Font, Frame, style};

pub fn draw_net_status(list: &mut DrawList, canvas: &Canvas, assets: &PlayAssets, st: &NetStatus) {
    let mut f = Frame::default();
    let s = &st.stats;
    let x = -canvas.width() * 0.5 + 24.0;
    let mut y = 1060.0 - 26.0 * s.machines.len() as f32;
    let y0 = y;
    let dim = Color::rgba(1.0, 1.0, 1.0, 0.55);
    f.text(
        format!(
            "FRAME {}  CONFIRMED {}  AHEAD {}  ROLLBACKS {} (MAX {})  STALLS {}{}",
            s.current_frame,
            s.confirmed_frame,
            s.frames_ahead,
            s.rollbacks,
            s.max_rollback_depth,
            s.prediction_stalls,
            if st.desyncs > 0 {
                format!("  DESYNCS {}", st.desyncs)
            } else {
                String::new()
            }
        ),
        Vec2::new(x, y - 26.0),
        20.0,
        Vec2::new(0.0, 0.5),
        Font::Regular,
        if st.desyncs > 0 { style::NOPE } else { dim },
    );
    for m in &s.machines {
        let ping = m
            .ggrs
            .as_ref()
            .map(|g| g.ping)
            .or(m.link_rtt_ms.map(|r| r as u128));
        let path = match (m.relayed, m.via_host) {
            (_, true) => "VIA HOST",
            (Some(true), _) => "RELAYED",
            (Some(false), _) => "DIRECT",
            (None, _) => "",
        };
        f.text(
            format!(
                "{}  {}  {}",
                m.name.to_uppercase(),
                ping.map_or("-".to_string(), |p| format!("{p} MS")),
                path
            ),
            Vec2::new(x, y),
            20.0,
            Vec2::new(0.0, 0.5),
            Font::Regular,
            dim,
        );
        y += 26.0;
    }
    if !st.gone_slots.is_empty() {
        f.text(
            format!("{} PLAYER(S) LEFT", st.gone_slots.len()),
            Vec2::new(x, y0 - 52.0),
            20.0,
            Vec2::new(0.0, 0.5),
            Font::Regular,
            style::NOPE,
        );
    }
    let notice = if st.frozen {
        Some("PLAYER LEFT - RESYNCING")
    } else if st.synchronising {
        Some("SYNCHRONISING")
    } else {
        None
    };
    if let Some(n) = notice {
        f.rect(
            crate::ui::Rect::new(Vec2::new(0.0, 540.0), Vec2::new(900.0, 110.0)),
            14.0,
            style::PANEL,
        );
        f.text(
            n,
            Vec2::new(0.0, 540.0),
            56.0,
            Vec2::splat(0.5),
            Font::Light,
            style::TEXT,
        );
    }
    f.draw(list, canvas, assets, false, 900_000);
    let _ = Layer::Screen;
}
