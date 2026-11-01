//! Frame-2 treatments of two-frame strips: the selected state of faction and agency logos, the disabled state of
//! building icons.
//!
//! Frame 1 is the fitted icon. Frame 2 is made from it, so the two always agree on shape and place:
//!
//! * **Selected** (faction and agency logos): a light stroke around the icon and a wide dark-red glow outside
//!   it, the icon itself unchanged on top. The constants were fitted to the vanilla logos (1.19.4.1): over 31
//!   faction strips the alpha outside frame 1's silhouette falls from 0.97 at 1 px to 0.53 at 5 px, 0.26 at 8 px,
//!   0.13 at 10 px and 0.055 at 12 px, the first two pixels are cream (about 208, 198, 165) and everything beyond
//!   the fourth pixel is (37, 5, 5). The model is `stroke = clamp(2.91 + 0.5 - d)`, `glow = 0.97 * Gaussian
//!   blur(sigma 4.0) of the silhouette dilated by 5.5 px`, glow under stroke under icon. Measured against the
//!   vanilla strips by a test that skips when the game is missing.
//! * **Disabled** (buildings): the icon with its colour mixed 45 percent toward its own luminance, alpha
//!   untouched.
//!
//! Both are pure functions of the pixels. The strips are built with [`crate::recipes::strip::build_strip`].

use crate::error::{Error, Result};
use crate::pixels::Rgba;
use crate::recipes::pillow::{alpha_composite, gaussian_blur_plane};
use crate::recipes::strip::build_strip;

/// Version of the frame recipes (bump when an output could change).
pub const RECIPE_VERSION: u32 = 1;

/// Alpha at which a pixel counts as part of the icon's silhouette.
pub const SOLID_ALPHA: u8 = 128;

/// Tunables of the selected state.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectedStyle {
    /// Stroke width in pixels: stroke alpha is `clamp(stroke + 0.5 - distance)`.
    pub stroke: f64,
    /// Stroke colour.
    pub stroke_rgb: [u8; 3],
    /// The silhouette is dilated by this many pixels before the glow blur.
    pub glow_dilate: f64,
    /// Gaussian sigma of the glow.
    pub glow_sigma: f64,
    /// Glow opacity from 0 to 1.
    pub glow_opacity: f64,
    /// Glow colour.
    pub glow_rgb: [u8; 3],
}

impl Default for SelectedStyle {
    fn default() -> Self {
        SelectedStyle { stroke: 2.91, stroke_rgb: [208, 198, 165], glow_dilate: 5.5, glow_sigma: 4.0, glow_opacity: 0.97, glow_rgb: [37, 5, 5] }
    }
}

/// How much of the colour the disabled state gives up: the share mixed toward luminance grey.
pub const DISABLED_DESATURATION: f64 = 0.45;

/// Distance from every pixel centre to the nearest solid pixel centre (0 on solid pixels), brute force within
/// `limit` pixels (further pixels report `limit + 1`). Icon frames are small, so this stays cheap.
pub fn distance_from_solid(img: &Rgba, solid_alpha: u8, limit: u32) -> Vec<f32> {
    let (w, h) = (img.width as i64, img.height as i64);
    let lim = i64::from(limit);
    let solid: Vec<bool> = img.data.chunks_exact(4).map(|p| p[3] >= solid_alpha).collect();
    let mut out = vec![(lim + 1) as f32; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            let mut best = i64::MAX;
            for dy in -lim..=lim {
                let yy = y + dy;
                if yy < 0 || yy >= h {
                    continue;
                }
                for dx in -lim..=lim {
                    let xx = x + dx;
                    if xx < 0 || xx >= w || !solid[(yy * w + xx) as usize] {
                        continue;
                    }
                    best = best.min(dx * dx + dy * dy);
                }
            }
            if best != i64::MAX {
                let d = (best as f64).sqrt();
                if d <= lim as f64 {
                    out[(y * w + x) as usize] = d as f32;
                }
            }
        }
    }
    out
}

fn check_frames(frames: u32) -> Result<()> {
    if frames != 2 {
        return Err(Error::Invalid(format!("frame treatments make two-frame strips; {frames} frames were asked for")));
    }
    Ok(())
}

/// The selected-state version of a fitted icon.
pub fn selected_variant(icon: &Rgba, style: &SelectedStyle) -> Result<Rgba> {
    if icon.width == 0 || icon.height == 0 {
        return Err(Error::Invalid("the icon is empty".into()));
    }
    if !icon.data.chunks_exact(4).any(|p| p[3] >= SOLID_ALPHA) {
        return Err(Error::Image("the icon has no visible subject to outline".into()));
    }
    let limit = (style.glow_dilate + 1.0).ceil() as u32 + 1;
    let dist = distance_from_solid(icon, SOLID_ALPHA, limit.max(style.stroke.ceil() as u32 + 2));
    let n = (icon.width * icon.height) as usize;
    // glow: dilated silhouette, blurred
    let dilated: Vec<u8> = dist.iter().map(|d| if f64::from(*d) <= style.glow_dilate { 255 } else { 0 }).collect();
    let blurred = gaussian_blur_plane(&dilated, icon.width as usize, icon.height as usize, style.glow_sigma);
    let mut glow = Rgba::new(icon.width, icon.height);
    let mut stroke = Rgba::new(icon.width, icon.height);
    for i in 0..n {
        let ga = (f64::from(blurred[i]) * style.glow_opacity).round().clamp(0.0, 255.0) as u8;
        glow.data[i * 4..i * 4 + 4].copy_from_slice(&[style.glow_rgb[0], style.glow_rgb[1], style.glow_rgb[2], ga]);
        let sa = ((style.stroke + 0.5 - f64::from(dist[i])).clamp(0.0, 1.0) * 255.0).round() as u8;
        stroke.data[i * 4..i * 4 + 4].copy_from_slice(&[style.stroke_rgb[0], style.stroke_rgb[1], style.stroke_rgb[2], sa]);
    }
    // glow, then the icon, then the stroke over everything that is not solid icon: vanilla's stroke also covers the
    // icon's soft edge pixels, which is what keeps the rim light instead of dark-fringed
    for (i, d) in dist.iter().enumerate() {
        if *d == 0.0 {
            stroke.data[i * 4 + 3] = 0;
        }
    }
    let mut out = glow;
    alpha_composite(&mut out, icon, 0, 0);
    alpha_composite(&mut out, &stroke, 0, 0);
    Ok(out)
}

/// The disabled-state version of a fitted icon: colour mixed toward luminance, alpha untouched.
pub fn disabled_variant(icon: &Rgba, mix: f64) -> Result<Rgba> {
    if !(0.0..=1.0).contains(&mix) {
        return Err(Error::Invalid("the desaturation mix must be between 0 and 1".into()));
    }
    let mut out = icon.clone();
    for p in out.data.chunks_exact_mut(4) {
        let l = f64::from(crate::recipes::pillow::luma(p[0], p[1], p[2]));
        for c in p.iter_mut().take(3) {
            *c = (l + (f64::from(*c) - l) * (1.0 - mix)).round().clamp(0.0, 255.0) as u8;
        }
    }
    Ok(out)
}

/// Two-frame strip: the icon, then its selected state.
pub fn selected_strip(icon: &Rgba, frames: u32) -> Result<Rgba> {
    check_frames(frames)?;
    let second = selected_variant(icon, &SelectedStyle::default())?;
    build_strip(&[icon.clone(), second], (icon.width, icon.height))
}

/// Two-frame strip: the icon, then its disabled state.
pub fn disabled_strip(icon: &Rgba, frames: u32) -> Result<Rgba> {
    check_frames(frames)?;
    let second = disabled_variant(icon, DISABLED_DESATURATION)?;
    build_strip(&[icon.clone(), second], (icon.width, icon.height))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disc(size: u32, radius: f64, rgb: [u8; 3]) -> Rgba {
        let mut img = Rgba::new(size, size);
        let c = f64::from(size) / 2.0;
        for y in 0..size {
            for x in 0..size {
                let d = ((f64::from(x) + 0.5 - c).powi(2) + (f64::from(y) + 0.5 - c).powi(2)).sqrt();
                if d < radius {
                    img.set_px(x, y, [rgb[0], rgb[1], rgb[2], 255]);
                }
            }
        }
        img
    }

    #[test]
    fn selected_strip_keeps_frame_one_and_leaves_the_interior_of_frame_two_alone() {
        let icon = disc(100, 30.0, [30, 120, 200]);
        let strip = selected_strip(&icon, 2).unwrap();
        assert_eq!((strip.width, strip.height), (200, 100));
        let frames = strip.split_frames(2).unwrap();
        assert_eq!(frames[0], icon);
        for y in 0..100 {
            for x in 0..100 {
                if icon.alpha(x, y) == 255 {
                    assert_eq!(frames[1].px(x, y), icon.px(x, y), "interior pixel {x},{y} changed");
                }
            }
        }
        // the rim just outside is the cream stroke, further out the dark glow, far away nothing
        let at = |dx: i64| frames[1].px((50 + 30 + dx) as u32, 50);
        assert!(at(1)[3] > 230 && at(1)[0] > 180, "stroke {:?}", at(1));
        let glow = at(7);
        assert!(glow[3] > 40 && glow[0] < 60 && glow[1] < 20, "glow {glow:?}");
        assert_eq!(frames[1].px(2, 2)[3], 0, "the corner far from the icon stays transparent");
    }

    #[test]
    fn selected_is_deterministic_and_refuses_empty_art_and_odd_frame_counts() {
        let icon = disc(60, 20.0, [200, 50, 50]);
        assert_eq!(selected_strip(&icon, 2).unwrap(), selected_strip(&icon, 2).unwrap());
        assert!(selected_strip(&icon, 3).is_err());
        assert!(selected_strip(&Rgba::new(40, 40), 2).is_err());
        assert!(disabled_strip(&icon, 1).is_err());
    }

    #[test]
    fn disabled_mixes_toward_luminance_and_keeps_alpha() {
        let mut icon = Rgba::new(4, 1);
        icon.set_px(0, 0, [200, 40, 40, 255]);
        icon.set_px(1, 0, [90, 90, 90, 255]);
        icon.set_px(2, 0, [10, 200, 90, 77]);
        let strip = disabled_strip(&icon, 2).unwrap();
        let f = strip.split_frames(2).unwrap();
        assert_eq!(f[0], icon);
        let sat = |p: [u8; 4]| f64::from(p[0].max(p[1]).max(p[2]) - p[0].min(p[1]).min(p[2]));
        assert!((sat(f[1].px(0, 0)) / sat(icon.px(0, 0)) - 0.55).abs() < 0.03, "{:?}", f[1].px(0, 0));
        assert_eq!(f[1].px(1, 0), [90, 90, 90, 255], "grey stays grey");
        assert_eq!(f[1].px(2, 0)[3], 77);
        assert_eq!(f[1].px(3, 0), [0, 0, 0, 0]);
        assert!(disabled_variant(&icon, 1.5).is_err());
    }

    // ------------------------------------------------------------ against the installed game

    fn game_dir() -> Option<std::path::PathBuf> {
        let g = std::env::var("HOI4_GAME_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| "C:/Program Files (x86)/Steam/steamapps/common/Hearts of Iron IV".into());
        g.join("gfx/interface").is_dir().then_some(g)
    }

    fn read_dds(p: &std::path::Path) -> Option<Rgba> {
        crate::dds::decode(&std::fs::read(p).ok()?).ok()
    }

    /// Mean alpha and alpha-weighted mean colour of the pixels whose distance from frame 1's silhouette is
    /// within `0.5` of `r`.
    fn ring(frame: &Rgba, dist: &[f32], r: f32) -> Option<(f64, [f64; 3])> {
        let (mut n, mut a_sum, mut rgb) = (0usize, 0f64, [0f64; 3]);
        for (i, p) in frame.data.chunks_exact(4).enumerate() {
            if (dist[i] - r).abs() <= 0.5 && dist[i] > 0.0 {
                n += 1;
                let a = f64::from(p[3]) / 255.0;
                a_sum += a;
                for c in 0..3 {
                    rgb[c] += f64::from(p[c]) * a;
                }
            }
        }
        (n > 0 && a_sum > 0.0).then(|| (a_sum / n as f64, [rgb[0] / a_sum, rgb[1] / a_sum, rgb[2] / a_sum]))
    }

    const RADII: [f32; 9] = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0, 12.0];

    /// Mean alpha at each radius and the mean rim colour (pixels 1 and 2), ours and vanilla's, over `strips`.
    fn selected_measurement(strips: &[Rgba], style: &SelectedStyle) -> ([f64; 9], [f64; 9], [f64; 3], [f64; 3]) {
        let (mut mine, mut theirs) = ([0f64; 9], [0f64; 9]);
        let (mut rim_mine, mut rim_theirs) = ([0f64; 3], [0f64; 3]);
        let mut rims = 0f64;
        for s in strips {
            let f = s.split_frames(2).unwrap();
            let dist = distance_from_solid(&f[0], SOLID_ALPHA, 20);
            let made = selected_variant(&f[0], style).unwrap();
            for (i, r) in RADII.iter().enumerate() {
                mine[i] += ring(&made, &dist, *r).map(|x| x.0).unwrap_or(0.0);
                theirs[i] += ring(&f[1], &dist, *r).map(|x| x.0).unwrap_or(0.0);
            }
            for r in [1.0f32, 2.0] {
                if let (Some(a), Some(b)) = (ring(&made, &dist, r), ring(&f[1], &dist, r)) {
                    for c in 0..3 {
                        rim_mine[c] += a.1[c];
                        rim_theirs[c] += b.1[c];
                    }
                    rims += 1.0;
                }
            }
        }
        let n = strips.len() as f64;
        (mine.map(|v| v / n), theirs.map(|v| v / n), rim_mine.map(|v| v / rims), rim_theirs.map(|v| v / rims))
    }

    fn vanilla_faction_strips() -> Option<Vec<Rgba>> {
        let game = game_dir()?;
        let dir = game.join("gfx/interface/factions/faction_logos");
        let mut files: Vec<_> = std::fs::read_dir(&dir).ok()?.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        files.sort();
        Some(files.iter().filter_map(|p| read_dds(p)).filter(|i| (i.width, i.height) == (200, 100)).collect())
    }

    #[test]
    fn selected_state_is_close_to_vanilla_faction_logos() {
        let Some(all) = vanilla_faction_strips() else {
            eprintln!("SKIPPED selected_frame vs vanilla: Hearts of Iron IV not found (set HOI4_GAME_DIR)");
            return;
        };
        if all.len() < 5 {
            eprintln!("SKIPPED selected_frame vs vanilla: fewer than 5 two-frame faction logos found");
            return;
        }
        for (label, set) in [("first 5", &all[..5]), ("all", &all[..])] {
            let (mine, theirs, rim_mine, rim_theirs) = selected_measurement(set, &SelectedStyle::default());
            let mut worst = 0f64;
            for (i, r) in RADII.iter().enumerate() {
                eprintln!("selected [{label}, {} strips] alpha at {r:>4} px: ours {:.3} vanilla {:.3}", set.len(), mine[i], theirs[i]);
                worst = worst.max((mine[i] - theirs[i]).abs());
            }
            eprintln!("selected [{label}] rim colour (px 1-2): ours {:?} vanilla {:?}", rim_mine.map(|v| v.round()), rim_theirs.map(|v| v.round()));
            assert!(worst < 0.10, "{label}: alpha profile outside the icon differs from vanilla by {worst:.3}");
            for c in 0..3 {
                assert!((rim_mine[c] - rim_theirs[c]).abs() < 30.0, "{label}: rim colour channel {c}: {rim_mine:?} against {rim_theirs:?}");
            }
        }
    }

    #[test]
    fn disabled_state_is_compared_with_the_vanilla_building_strip() {
        let Some(game) = game_dir() else {
            eprintln!("SKIPPED disabled_frame vs vanilla: Hearts of Iron IV not found (set HOI4_GAME_DIR)");
            return;
        };
        let Some(strip) = read_dds(&game.join("gfx/interface/buildings/building_icon_strip.dds")) else {
            eprintln!("SKIPPED disabled_frame vs vanilla: building_icon_strip.dds not found or not uncompressed");
            return;
        };
        let h = strip.height;
        let frames = strip.width / h;
        let sat = |f: &Rgba| {
            let (mut s, mut n) = (0f64, 0usize);
            for p in f.data.chunks_exact(4).filter(|p| p[3] > 200) {
                let (mx, mn) = (f64::from(p[0].max(p[1]).max(p[2])), f64::from(p[0].min(p[1]).min(p[2])));
                if mx > 0.0 {
                    s += (mx - mn) / mx;
                    n += 1;
                }
            }
            if n == 0 {
                None
            } else {
                Some(s / n as f64 * 100.0)
            }
        };
        let (mut even, mut odd, mut made, mut pairs) = (0f64, 0f64, 0f64, 0u32);
        for i in (0..frames.saturating_sub(1)).step_by(2) {
            let a = strip.crop(i * h, 0, h, h).unwrap();
            let b = strip.crop((i + 1) * h, 0, h, h).unwrap();
            if let (Some(sa), Some(sb)) = (sat(&a), sat(&b)) {
                even += sa;
                odd += sb;
                made += sat(&disabled_variant(&a, DISABLED_DESATURATION).unwrap()).unwrap();
                pairs += 1;
            }
        }
        assert!(pairs > 3, "{pairs} frame pairs");
        let n = f64::from(pairs);
        eprintln!(
            "disabled: vanilla building_icon_strip over {pairs} frame pairs: mean saturation frame 1 {:.1}, frame 2 {:.1}; ours maps frame 1 {:.1} to {:.1}",
            even / n,
            odd / n,
            even / n,
            made / n
        );
        // vanilla's two frames of a pair are different art (no per-pixel correspondence), so only the direction can
        // be compared: the disabled look has clearly lower saturation than the frame it came from
        assert!(made < even * 0.8, "disabled saturation {made} against {even}");
    }
}
