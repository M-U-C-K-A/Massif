//! Colour management as Blender's: a view transform takes the light the
//! path tracer adds up to the display's values (sRGB), with a look, an
//! exposure and a gamma. The shader does it as each sample comes (`tone` in
//! `params.wgsl`); [`Tone::map`] does the same on the processor, for the
//! images Open Image Denoise gives, the stills and the video frames.
//!
//! Written from the transforms' public descriptions: sRGB's encoding,
//! Khronos PBR Neutral's specification, AgX (stops of light along an S
//! curve, its colours drawn in toward white and back out) and Filmic (the
//! same curve over 16.5 stops).

/// The view transforms, in Blender's order.
pub const VIEWS: [&str; 7] = ["Standard", "Khronos PBR Neutral", "AgX", "Filmic", "Filmic Log", "False Color", "Raw"];
/// The default: AgX, as Blender's since 4.0.
pub const AGX: u32 = 2;
pub const FILMIC: u32 = 3;

/// The looks of AgX, in Blender's order.
pub const AGX_LOOKS: [&str; 10] = ["None", "Punchy", "Greyscale", "Very High Contrast", "High Contrast", "Medium High Contrast", "Base Contrast", "Medium Low Contrast", "Low Contrast", "Very Low Contrast"];
/// The looks of Filmic.
pub const FILMIC_LOOKS: [&str; 8] = ["None", "Very High Contrast", "High Contrast", "Medium High Contrast", "Medium Contrast", "Medium Low Contrast", "Low Contrast", "Very Low Contrast"];

/// The looks view `view` has (None alone for the others).
pub fn looks(view: u32) -> &'static [&'static str] {
    match view {
        AGX => &AGX_LOOKS,
        FILMIC => &FILMIC_LOOKS,
        _ => &["None"],
    }
}

/// The tone mapping's curves in WGSL (`agx`, `s_curve`, `srgb_encode`…),
/// for other shaders to end the same way: the 3D View's Shaders.
pub const TONE_WGSL: &str = include_str!("tone.wgsl");

/// How an image's light becomes the display's values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tone {
    pub exposure: f32,
    pub view: u32,
    pub look: u32,
    pub gamma: f32,
}

impl Default for Tone {
    fn default() -> Self {
        Self { exposure: 1.0, view: AGX, look: 0, gamma: 1.0 }
    }
}

/// A contrast by its name's place: the S curve's steepness.
fn contrast(name: &str) -> f32 {
    match name {
        "Very High Contrast" => 3.0,
        "High Contrast" => 2.7,
        "Medium High Contrast" => 2.45,
        "Medium Low Contrast" => 2.0,
        "Low Contrast" => 1.8,
        "Very Low Contrast" => 1.55,
        _ => 2.2,
    }
}

impl Tone {
    /// What the shader reads (`P.tone`): the gamma, the look's contrast,
    /// its saturation, greyscale.
    pub fn params(&self) -> [f32; 4] {
        let name = looks(self.view).get(self.look as usize).copied().unwrap_or("None");
        let (c, sat, grey) = match name {
            "Punchy" => (2.5, 1.35, 0.0),
            "Greyscale" => (2.2, 1.0, 1.0),
            n => (contrast(n), 1.0, 0.0),
        };
        [self.gamma.max(0.1), c, sat, grey]
    }

    /// Linear light (three floats a pixel) to sRGB bytes, opaque, as the
    /// shader's `tone`.
    pub fn map(&self, linear: &[f32]) -> Vec<u8> {
        let [gamma, contrast, saturation, grey] = self.params();
        let mut out = Vec::with_capacity(linear.len() / 3 * 4);
        for p in linear.as_chunks::<3>().0 {
            let x = p.map(|c| (c * self.exposure).max(0.0));
            let mut v = match self.view {
                0 => x.map(|c| srgb_encode(c.min(1.0))),
                1 => pbr_neutral(x).map(srgb_encode),
                3 => s_curve(x, -10.0, contrast, 1.08 * saturation),
                4 => x.map(|c| ((c.max(1e-10).log2() + 10.0) / 16.5).clamp(0.0, 1.0)),
                5 => false_color(luminance(x)),
                6 => x.map(|c| c.min(1.0)),
                _ => agx(x, contrast, saturation),
            };
            if grey > 0.5 {
                let l = luminance(v);
                v = [l; 3];
            }
            for c in v {
                out.push((c.clamp(0.0, 1.0).powf(1.0 / gamma) * 255.0 + 0.5) as u8);
            }
            out.push(255);
        }
        out
    }
}

fn luminance(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn srgb_encode(c: f32) -> f32 {
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

fn s_curve(x: [f32; 3], low: f32, contrast: f32, saturation: f32) -> [f32; 3] {
    let l = luminance(x);
    let x0 = (0.18f32.log2() - low) / 16.5;
    let k = x0.powf(contrast) * 0.54 / (0.46 * (1.0 - x0).powf(contrast));
    x.map(|c| {
        let d = (l + (c - l) * saturation).max(0.0);
        let s = ((d.max(1e-10).log2() - low) / 16.5).clamp(0.0, 1.0);
        let (a, b) = (s.powf(contrast), (1.0 - s).powf(contrast));
        a / (a + k * b).max(1e-10)
    })
}

fn agx(x: [f32; 3], contrast: f32, saturation: f32) -> [f32; 3] {
    let y = s_curve(x, -12.473_93, contrast, 0.82);
    let lin = y.map(|c| c.powf(2.2));
    let l = luminance(lin);
    lin.map(|c| (l + (c - l) * 1.18 * saturation).clamp(0.0, 1.0).powf(1.0 / 2.2))
}

fn pbr_neutral(c: [f32; 3]) -> [f32; 3] {
    const START: f32 = 0.8 - 0.04;
    const DESATURATION: f32 = 0.15;
    let x = c[0].min(c[1]).min(c[2]);
    let offset = if x < 0.08 { x - 6.25 * x * x } else { 0.04 };
    let mut c = c.map(|v| v - offset);
    let peak = c[0].max(c[1]).max(c[2]);
    if peak < START {
        return c;
    }
    let d = 1.0 - START;
    let new_peak = 1.0 - d * d / (peak + d - START);
    c = c.map(|v| v * new_peak / peak);
    let g = 1.0 - 1.0 / (DESATURATION * (peak - new_peak) + 1.0);
    c.map(|v| v + (new_peak - v) * g)
}

fn false_color(l: f32) -> [f32; 3] {
    let ev = (l.max(1e-10) / 0.18).log2();
    match ev {
        e if e < -6.5 => [0.0; 3],
        e if e < -4.5 => [0.4, 0.0, 0.6],
        e if e < -2.5 => [0.1, 0.2, 0.9],
        e if e < -1.5 => [0.0, 0.7, 0.8],
        e if e < -0.5 => [0.2, 0.75, 0.25],
        e if e < 0.5 => [0.5; 3],
        e if e < 1.5 => [0.75; 3],
        e if e < 2.5 => [0.95, 0.9, 0.2],
        e if e < 4.5 => [1.0, 0.55, 0.0],
        e if e < 6.5 => [0.95, 0.1, 0.1],
        _ => [1.0; 3],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mid grey comes out as each view puts it, black stays black, and the
    /// brightest light runs to white (not to a hue) in AgX and Filmic.
    #[test]
    fn the_views_place_grey_and_white() {
        let grey = |view: u32| Tone { view, ..Tone::default() }.map(&[0.18; 3])[0];
        assert_eq!(grey(0), 118, "Standard: sRGB's 0.18");
        assert!((115..=120).contains(&grey(AGX)), "AgX: mid grey at 0.46 ({})", grey(AGX));
        assert!((115..=120).contains(&grey(FILMIC)), "Filmic: {}", grey(FILMIC));
        assert_eq!(grey(6), 46, "Raw: the light itself");
        for view in 0..7 {
            assert_eq!(Tone { view, ..Tone::default() }.map(&[0.0; 3])[..3], [0, 0, 0], "{} keeps black", VIEWS[view as usize]);
        }
        let hot = Tone::default().map(&[400.0, 40.0, 4.0]);
        assert!(hot[1] > 230 && hot[2] > 200, "a bright orange whitens in AgX: {hot:?}");
        let punchy = Tone { look: 1, ..Tone::default() }.map(&[0.3, 0.1, 0.05]);
        let none = Tone::default().map(&[0.3, 0.1, 0.05]);
        assert!(punchy[0] as i32 - punchy[2] as i32 > none[0] as i32 - none[2] as i32, "Punchy is more colourful");
        let g = Tone { look: 2, ..Tone::default() }.map(&[0.3, 0.1, 0.05]);
        assert!(g[0] == g[1] && g[1] == g[2], "Greyscale");
        let darker = Tone { gamma: 0.5, ..Tone::default() }.map(&[0.18; 3])[0];
        assert!(darker < grey(AGX), "a lower gamma darkens");
    }
}
