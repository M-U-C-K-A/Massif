//! Renders saved as OpenEXR: the light as traced, linear and before the
//! view transform (Colour Management's view, look, exposure and gamma are
//! left to the tool that opens it), half floats in `R`, `G`, `B` and `A`;
//! with the guides the denoiser is given as layers of the same file, as
//! Blender writes them: `Depth.Z` (blocks, 32-bit), `Normal.X/Y/Z` (in the
//! world, half) and `Albedo.R/G/B` (half).

use crate::gpu::Samples;
use exr::prelude::*;
use std::path::Path;

/// The file's bytes of `s`, its light `color` (three floats a pixel, as
/// [`Samples::color`], denoised or not), with the guides as layers when
/// `layers`.
pub fn encode(s: &Samples, color: &[f32], layers: bool) -> std::result::Result<Vec<u8>, String> {
    let (w, h) = (s.width as usize, s.height as usize);
    let n = w * h;
    if color.len() != n * 3 || (layers && (s.albedo.len() != n * 3 || s.normal.len() != n * 3 || s.depth.len() != n)) {
        return Err("images of the wrong size".into());
    }
    let half = |v: &[f32], c: usize, k: usize| FlatSamples::F16(v.iter().skip(c).step_by(k).map(|&x| f16::from_f32(x)).collect());
    let mut channels = vec![
        AnyChannel::new("R", half(color, 0, 3)),
        AnyChannel::new("G", half(color, 1, 3)),
        AnyChannel::new("B", half(color, 2, 3)),
        AnyChannel::new("A", FlatSamples::F16(vec![f16::ONE; n])),
    ];
    if layers {
        channels.push(AnyChannel::new("Depth.Z", FlatSamples::F32(s.depth.clone())));
        for (c, name) in ["Normal.X", "Normal.Y", "Normal.Z"].into_iter().enumerate() {
            channels.push(AnyChannel::new(name, half(&s.normal, c, 3)));
        }
        for (c, name) in ["Albedo.R", "Albedo.G", "Albedo.B"].into_iter().enumerate() {
            channels.push(AnyChannel::new(name, half(&s.albedo, c, 3)));
        }
    }
    let layer = Layer::new((w, h), LayerAttributes::default(), Encoding::SMALL_LOSSLESS, AnyChannels::sort(SmallVec::from_vec(channels)));
    let mut bytes = std::io::Cursor::new(Vec::new());
    Image::from_layer(layer).write().to_buffered(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes.into_inner())
}

/// [`encode`] written at `path`, whole or not at all.
pub fn write(path: &Path, s: &Samples, color: &[f32], layers: bool) -> std::result::Result<(), String> {
    let bytes = encode(s, color, layers)?;
    massif_core::atomic::write(path, &bytes, true).map_err(|e| massif_core::atomic::describe(&e, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The light goes out as half floats, above 1 where it is, the guides
    /// as layers; read back by the crate with their names and values.
    #[test]
    fn a_render_goes_out_with_its_layers() {
        let (w, h) = (5u32, 3u32);
        let n = (w * h) as usize;
        let color: Vec<f32> = (0..n * 3).map(|k| k as f32 * 0.25).collect();
        let s = Samples {
            width: w,
            height: h,
            samples: 16,
            tone: Default::default(),
            color: color.clone(),
            albedo: (0..n * 3).map(|k| (k % 7) as f32 / 8.0).collect(),
            normal: (0..n * 3).map(|k| if k % 3 == 1 { 1.0 } else { 0.0 }).collect(),
            depth: (0..n).map(|k| 10.0 + k as f32 * 1.5).collect(),
        };
        let bytes = encode(&s, &color, true).unwrap();
        let image = read().no_deep_data().largest_resolution_level().all_channels().first_valid_layer().all_attributes().from_buffered(std::io::Cursor::new(bytes)).unwrap();
        let layer = image.layer_data;
        assert_eq!((layer.size.x(), layer.size.y()), (w as usize, h as usize));
        let channel = |name: &str| layer.channel_data.list.iter().find(|c| c.name.to_string() == name).unwrap_or_else(|| panic!("{name}")).sample_data.values_as_f32().collect::<Vec<f32>>();
        let names: Vec<String> = layer.channel_data.list.iter().map(|c| c.name.to_string()).collect();
        assert_eq!(names, ["A", "Albedo.B", "Albedo.G", "Albedo.R", "B", "Depth.Z", "G", "Normal.X", "Normal.Y", "Normal.Z", "R"]);
        let (r, b) = (channel("R"), channel("B"));
        assert_eq!(r[4], 3.0, "linear, above 1");
        assert_eq!(b[n - 1], (n * 3 - 1) as f32 * 0.25);
        assert!(channel("A").iter().all(|&a| a == 1.0));
        assert_eq!(channel("Depth.Z"), s.depth);
        assert!(channel("Normal.Y").iter().all(|&y| y == 1.0));
        assert_eq!(channel("Albedo.G")[2], s.albedo[7]);
        // Without the layers, the light alone.
        let bytes = encode(&s, &color, false).unwrap();
        let image = read().no_deep_data().largest_resolution_level().all_channels().first_valid_layer().all_attributes().from_buffered(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(image.layer_data.channel_data.list.len(), 4);
    }
}
