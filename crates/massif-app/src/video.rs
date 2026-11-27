//! Writing the Render side's animations: as an MP4 (H.264), through the
//! system's own encoder where Massif has it (AVFoundation on macOS), or as a
//! sequence of PNG images everywhere.

use std::path::{Path, PathBuf};

/// Where the frames of an animation go.
pub enum Sink {
    #[cfg(target_os = "macos")]
    Mp4(mac::Writer),
    /// `{stem}_00001.png` and on, in `dir`.
    Png { dir: PathBuf, stem: String },
}

/// Whether an MP4 can be written here.
pub fn mp4_supported() -> bool {
    cfg!(target_os = "macos")
}

/// The file of frame `index` (from 0) of a PNG sequence.
pub fn png_path(dir: &Path, stem: &str, index: u32) -> PathBuf {
    dir.join(format!("{stem}_{:05}.png", index + 1))
}

impl Sink {
    /// An MP4 at `path`, `width` × `height` at `fps` frames a second.
    #[cfg(target_os = "macos")]
    pub fn mp4(path: &Path, width: u32, height: u32, fps: u32) -> Result<Sink, String> {
        mac::Writer::new(path, width, height, fps).map(Sink::Mp4)
    }

    #[cfg(not(target_os = "macos"))]
    pub fn mp4(_path: &Path, _width: u32, _height: u32, _fps: u32) -> Result<Sink, String> {
        Err("MP4 is written on macOS only; choose a PNG sequence".into())
    }

    /// Frame `index`, RGBA, of the size the sink was made for.
    pub fn push(&mut self, index: u32, rgba: &[u8], width: u32, height: u32) -> Result<(), String> {
        match self {
            #[cfg(target_os = "macos")]
            Sink::Mp4(w) => w.push(index, rgba),
            Sink::Png { dir, stem } => {
                let path = png_path(dir, stem, index);
                let mut png = Vec::new();
                image::write_buffer_with_format(&mut std::io::Cursor::new(&mut png), rgba, width, height, image::ColorType::Rgba8, image::ImageFormat::Png).map_err(|e| e.to_string())?;
                massif_core::atomic::write(&path, &png, true).map_err(|e| massif_core::atomic::describe(&e, &path))
            }
        }
    }

    /// Done: the MP4 closed and written whole.
    pub fn finish(self) -> Result<(), String> {
        match self {
            #[cfg(target_os = "macos")]
            Sink::Mp4(w) => w.finish(),
            Sink::Png { .. } => Ok(()),
        }
    }
}

#[cfg(target_os = "macos")]
mod mac {
    //! AVFoundation's AVAssetWriter, H.264 in an MP4, fed CoreVideo pixel
    //! buffers: the few calls it takes, through the Objective-C runtime.

    use objc2::encode::{Encode, Encoding, RefEncode};
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Bool};
    use objc2::msg_send;
    use objc2_foundation::{NSDictionary, NSNumber, NSString, NSURL};
    use std::ffi::c_void;
    use std::path::Path;

    #[link(name = "AVFoundation", kind = "framework")]
    unsafe extern "C" {}

    #[repr(C)]
    pub struct CVBuffer {
        _private: [u8; 0],
    }

    // SAFETY: an opaque CoreFoundation type, passed by pointer.
    unsafe impl RefEncode for CVBuffer {
        const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("__CVBuffer", &[]));
    }

    #[link(name = "CoreVideo", kind = "framework")]
    unsafe extern "C" {
        fn CVPixelBufferCreate(allocator: *const c_void, width: usize, height: usize, format: u32, attributes: *const c_void, out: *mut *mut CVBuffer) -> i32;
        fn CVPixelBufferLockBaseAddress(buffer: *mut CVBuffer, flags: u64) -> i32;
        fn CVPixelBufferUnlockBaseAddress(buffer: *mut CVBuffer, flags: u64) -> i32;
        fn CVPixelBufferGetBaseAddress(buffer: *mut CVBuffer) -> *mut c_void;
        fn CVPixelBufferGetBytesPerRow(buffer: *mut CVBuffer) -> usize;
        fn CVPixelBufferRelease(buffer: *mut CVBuffer);
    }

    /// CoreMedia's time: `value / timescale` seconds.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CMTime {
        value: i64,
        timescale: i32,
        flags: u32,
        epoch: i64,
    }

    // SAFETY: the layout of CMTime, `{?=qiIq}`.
    unsafe impl Encode for CMTime {
        const ENCODING: Encoding = Encoding::Struct("?", &[Encoding::LongLong, Encoding::Int, Encoding::UInt, Encoding::LongLong]);
    }

    /// kCVPixelFormatType_32BGRA.
    const BGRA: u32 = u32::from_be_bytes(*b"BGRA");

    pub struct Writer {
        writer: Retained<AnyObject>,
        input: Retained<AnyObject>,
        adaptor: Retained<AnyObject>,
        width: u32,
        height: u32,
        fps: u32,
    }

    // SAFETY: the writer is used from one thread at a time (the one the
    // animation is written on).
    unsafe impl Send for Writer {}

    fn class(name: &std::ffi::CStr) -> Result<&'static AnyClass, String> {
        AnyClass::get(name).ok_or_else(|| format!("no {}", name.to_string_lossy()))
    }

    impl Writer {
        pub fn new(path: &Path, width: u32, height: u32, fps: u32) -> Result<Writer, String> {
            // H.264 takes even sizes.
            if !width.is_multiple_of(2) || !height.is_multiple_of(2) {
                return Err("the size must be even".into());
            }
            let _ = std::fs::remove_file(path);
            // SAFETY: AVFoundation's documented methods, with the types of
            // their arguments; nothing is used after it is released.
            unsafe {
                let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
                let mut error: *mut AnyObject = std::ptr::null_mut();
                let writer: Option<Retained<AnyObject>> = msg_send![class(c"AVAssetWriter")?, assetWriterWithURL: &*url, fileType: &*NSString::from_str("public.mpeg-4"), error: &mut error];
                let writer = writer.ok_or("the movie could not be made there")?;
                // Some 0.2 bits a pixel a frame: clean at 1080p.
                let bitrate = ((width as f64 * height as f64 * fps as f64 * 0.2) as i64).clamp(2_000_000, 200_000_000);
                let compression = NSDictionary::<NSString, AnyObject>::from_slices(&[&*NSString::from_str("AverageBitRate")], &[&*NSNumber::new_i64(bitrate)]);
                let keys = [NSString::from_str("AVVideoCodecKey"), NSString::from_str("AVVideoWidthKey"), NSString::from_str("AVVideoHeightKey"), NSString::from_str("AVVideoCompressionPropertiesKey")];
                let codec = NSString::from_str("avc1");
                let (w, h) = (NSNumber::new_u32(width), NSNumber::new_u32(height));
                let values: [&AnyObject; 4] = [&codec, &w, &h, &compression];
                let settings = NSDictionary::<NSString, AnyObject>::from_slices(&keys.iter().map(|k| &**k).collect::<Vec<_>>(), &values);
                let input: Option<Retained<AnyObject>> = msg_send![class(c"AVAssetWriterInput")?, assetWriterInputWithMediaType: &*NSString::from_str("vide"), outputSettings: &*settings];
                let input = input.ok_or("no video input")?;
                let _: () = msg_send![&*input, setExpectsMediaDataInRealTime: Bool::NO];
                let adaptor: Option<Retained<AnyObject>> = msg_send![class(c"AVAssetWriterInputPixelBufferAdaptor")?, assetWriterInputPixelBufferAdaptorWithAssetWriterInput: &*input, sourcePixelBufferAttributes: std::ptr::null::<AnyObject>()];
                let adaptor = adaptor.ok_or("no pixel buffer adaptor")?;
                let can: Bool = msg_send![&*writer, canAddInput: &*input];
                if !can.as_bool() {
                    return Err("the movie does not take this video".into());
                }
                let _: () = msg_send![&*writer, addInput: &*input];
                let started: Bool = msg_send![&*writer, startWriting];
                if !started.as_bool() {
                    return Err(Self::error_of(&writer));
                }
                let zero = CMTime { value: 0, timescale: fps as i32, flags: 1, epoch: 0 };
                let _: () = msg_send![&*writer, startSessionAtSourceTime: zero];
                Ok(Writer { writer, input, adaptor, width, height, fps })
            }
        }

        fn error_of(writer: &AnyObject) -> String {
            // SAFETY: `error` is an NSError or nil.
            unsafe {
                let error: Option<Retained<AnyObject>> = msg_send![writer, error];
                match error {
                    Some(e) => {
                        let text: Retained<NSString> = msg_send![&*e, localizedDescription];
                        text.to_string()
                    }
                    None => "the movie could not be written".into(),
                }
            }
        }

        /// Frame `index`, RGBA.
        pub fn push(&mut self, index: u32, rgba: &[u8]) -> Result<(), String> {
            let (w, h) = (self.width as usize, self.height as usize);
            if rgba.len() != w * h * 4 {
                return Err("a frame of the wrong size".into());
            }
            // SAFETY: the pixel buffer is made, filled within its rows, and
            // released here; the adaptor retains what it keeps.
            unsafe {
                // The encoder takes frames as fast as it can: wait for it.
                let started = std::time::Instant::now();
                loop {
                    let ready: Bool = msg_send![&*self.input, isReadyForMoreMediaData];
                    if ready.as_bool() {
                        break;
                    }
                    if started.elapsed().as_secs() > 60 {
                        return Err("the encoder stopped taking frames".into());
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                let mut buffer: *mut CVBuffer = std::ptr::null_mut();
                if CVPixelBufferCreate(std::ptr::null(), w, h, BGRA, std::ptr::null(), &mut buffer) != 0 || buffer.is_null() {
                    return Err("no pixel buffer".into());
                }
                CVPixelBufferLockBaseAddress(buffer, 0);
                let base = CVPixelBufferGetBaseAddress(buffer) as *mut u8;
                let row = CVPixelBufferGetBytesPerRow(buffer);
                for y in 0..h {
                    let dst = std::slice::from_raw_parts_mut(base.add(y * row), w * 4);
                    for (d, s) in dst.as_chunks_mut::<4>().0.iter_mut().zip(rgba[y * w * 4..(y + 1) * w * 4].as_chunks::<4>().0) {
                        d.copy_from_slice(&[s[2], s[1], s[0], 255]);
                    }
                }
                CVPixelBufferUnlockBaseAddress(buffer, 0);
                let at = CMTime { value: index as i64, timescale: self.fps as i32, flags: 1, epoch: 0 };
                let ok: Bool = msg_send![&*self.adaptor, appendPixelBuffer: buffer, withPresentationTime: at];
                CVPixelBufferRelease(buffer);
                if !ok.as_bool() {
                    return Err(Self::error_of(&self.writer));
                }
            }
            Ok(())
        }

        /// Close the movie, written whole.
        pub fn finish(self) -> Result<(), String> {
            // SAFETY: as in `new`; `finishWriting` waits for the file.
            unsafe {
                let _: () = msg_send![&*self.input, markAsFinished];
                let done: Bool = msg_send![&*self.writer, finishWriting];
                if done.as_bool() { Ok(()) } else { Err(Self::error_of(&self.writer)) }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A second of a moving gradient written as an MP4 (on macOS) and as
    /// PNG images: the MP4 has its file type box and some size, and the
    /// images are numbered from 1.
    #[test]
    fn an_animation_is_written() {
        let dir = std::env::temp_dir().join(format!("massif-video-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (w, h, fps) = (320u32, 180u32, 24u32);
        let frame = |k: u32| -> Vec<u8> { (0..w * h).flat_map(|i| [((i % w + k * 8) % 256) as u8, ((i / w) % 256) as u8, (k * 10 % 256) as u8, 255]).collect() };
        if mp4_supported() {
            let path = dir.join("test.mp4");
            let mut sink = Sink::mp4(&path, w, h, fps).unwrap();
            for k in 0..fps {
                sink.push(k, &frame(k), w, h).unwrap();
            }
            sink.finish().unwrap();
            let bytes = std::fs::read(&path).unwrap();
            assert!(bytes.len() > 2000, "{} bytes", bytes.len());
            assert_eq!(&bytes[4..8], b"ftyp", "an MP4's file type box");
        }
        let mut sink = Sink::Png { dir: dir.clone(), stem: "frame".into() };
        for k in 0..3 {
            sink.push(k, &frame(k), w, h).unwrap();
        }
        sink.finish().unwrap();
        assert!(png_path(&dir, "frame", 0).ends_with("frame_00001.png"));
        assert!(png_path(&dir, "frame", 2).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
