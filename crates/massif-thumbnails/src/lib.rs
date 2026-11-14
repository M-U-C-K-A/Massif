//! Massif's thumbnail handler for Windows Explorer: the map of a `.massif`
//! file as its icon, as the Quick Look extension shows it in the Finder
//! (`tools/macos/thumbnails`).
//!
//! Massif keeps a JPEG picture of the map in a section of kind `THMB` of the
//! file (see `massif-format`): the header says where the directory of
//! sections is, the directory where the picture is; nothing else is read.
//! Explorer hands the file over as a stream (`IInitializeWithStream`) and
//! asks for a bitmap of a size (`IThumbnailProvider`).
//!
//! Massif registers this library for the current user when it starts (see
//! `massif-app`'s `finder` module), from the folder it runs in.

/// The class of the handler, as it is registered.
pub const CLSID: &str = "{B9D54669-8236-4946-B373-07956D35E29B}";
/// The file name the library goes by next to `Massif.exe`.
pub const FILE_NAME: &str = "MassifThumbnails.dll";

/// Where the picture of the map is in a `.massif` file, given a way to read
/// `len` bytes at an offset; `None` for a file without one.
pub fn find_picture(read: &mut dyn FnMut(u64, usize) -> Option<Vec<u8>>) -> Option<Vec<u8>> {
    let header = read(0, 64)?;
    if header.len() < 64 || &header[..8] != b"MASSIF\x1a\n" {
        return None;
    }
    let little = |b: &[u8], at: usize, n: usize| b[at..at + n].iter().rev().fold(0u64, |v, &x| v << 8 | x as u64);
    let (offset, len) = (little(&header, 0x10, 8), little(&header, 0x18, 4) as usize);
    if len % 32 != 0 || len > 1 << 24 {
        return None;
    }
    let directory = read(offset, len)?;
    for entry in directory.as_chunks::<32>().0 {
        if &entry[..4] == b"THMB" {
            let (at, len) = (little(entry, 8, 8), little(entry, 16, 8));
            if len > 1 << 24 {
                return None;
            }
            return read(at, len as usize);
        }
    }
    None
}

#[cfg(windows)]
mod com {
    use std::cell::RefCell;
    use std::ffi::c_void;
    use std::ptr::null_mut;
    use std::sync::atomic::{AtomicIsize, Ordering};
    use windows::Win32::Foundation::{CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_FAIL, E_POINTER, E_UNEXPECTED, S_FALSE, S_OK};
    use windows::Win32::Graphics::Gdi::{BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateDIBSection, DIB_RGB_COLORS, HBITMAP};
    use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl, IStream, STREAM_SEEK_SET};
    use windows::Win32::UI::Shell::PropertiesSystem::{IInitializeWithStream, IInitializeWithStream_Impl};
    use windows::Win32::UI::Shell::{IThumbnailProvider, IThumbnailProvider_Impl, WTS_ALPHATYPE, WTSAT_RGB};
    use windows_core::{BOOL, Error, GUID, HRESULT, IUnknown, Interface, Ref, Result, implement};

    const CLASS: GUID = GUID::from_u128(0xb9d54669_8236_4946_b373_07956d35e29b);

    /// Objects alive and locks held: the library stays loaded while any.
    static ALIVE: AtomicIsize = AtomicIsize::new(0);

    #[implement(IThumbnailProvider, IInitializeWithStream)]
    struct Provider {
        stream: RefCell<Option<IStream>>,
    }

    impl Provider {
        fn new() -> Self {
            ALIVE.fetch_add(1, Ordering::SeqCst);
            Self { stream: RefCell::new(None) }
        }
    }

    impl Drop for Provider {
        fn drop(&mut self) {
            ALIVE.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// `len` bytes of `stream` at `at`.
    fn read_at(stream: &IStream, at: u64, len: usize) -> Option<Vec<u8>> {
        unsafe { stream.Seek(at as i64, STREAM_SEEK_SET, None) }.ok()?;
        let mut buf = vec![0u8; len];
        let mut done = 0usize;
        while done < len {
            let mut read = 0u32;
            let hr = unsafe { stream.Read(buf[done..].as_mut_ptr() as *mut c_void, (len - done) as u32, Some(&mut read)) };
            if hr.is_err() || read == 0 {
                return None;
            }
            done += read as usize;
        }
        Some(buf)
    }

    impl IInitializeWithStream_Impl for Provider_Impl {
        fn Initialize(&self, stream: Ref<IStream>, _mode: u32) -> Result<()> {
            *self.stream.borrow_mut() = Some(stream.ok()?.clone());
            Ok(())
        }
    }

    impl IThumbnailProvider_Impl for Provider_Impl {
        fn GetThumbnail(&self, cx: u32, bitmap: *mut HBITMAP, alpha: *mut WTS_ALPHATYPE) -> Result<()> {
            if bitmap.is_null() || alpha.is_null() {
                return Err(E_POINTER.into());
            }
            let stream = self.stream.borrow().clone().ok_or(Error::from(E_UNEXPECTED))?;
            let jpeg = super::find_picture(&mut |at, len| read_at(&stream, at, len)).ok_or(Error::from(E_FAIL))?;
            let picture = image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg).map_err(|_| Error::from(E_FAIL))?.to_rgb8();
            // As large as asked, its proportions kept.
            let (w, h) = picture.dimensions();
            let scale = (cx.max(1) as f32 / w.max(h) as f32).min(1.0);
            let (tw, th) = (((w as f32 * scale).round() as u32).max(1), ((h as f32 * scale).round() as u32).max(1));
            let picture = if (tw, th) == (w, h) { picture } else { image::imageops::resize(&picture, tw, th, image::imageops::FilterType::Triangle) };
            // A 32-bit bitmap, its rows top down.
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: tw as i32,
                    biHeight: -(th as i32),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut c_void = null_mut();
            let made = unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0)? };
            if bits.is_null() {
                return Err(E_FAIL.into());
            }
            let out = unsafe { std::slice::from_raw_parts_mut(bits as *mut u8, (tw * th * 4) as usize) };
            for (o, p) in out.chunks_exact_mut(4).zip(picture.pixels()) {
                o.copy_from_slice(&[p[2], p[1], p[0], 255]);
            }
            unsafe {
                *bitmap = made;
                *alpha = WTSAT_RGB;
            }
            Ok(())
        }
    }

    #[implement(IClassFactory)]
    struct Factory;

    impl IClassFactory_Impl for Factory_Impl {
        fn CreateInstance(&self, outer: Ref<IUnknown>, iid: *const GUID, object: *mut *mut c_void) -> Result<()> {
            if !outer.is_null() {
                return Err(CLASS_E_NOAGGREGATION.into());
            }
            if object.is_null() {
                return Err(E_POINTER.into());
            }
            unsafe { *object = null_mut() };
            let provider: IThumbnailProvider = Provider::new().into();
            unsafe { provider.query(iid, object) }.ok()
        }

        fn LockServer(&self, lock: BOOL) -> Result<()> {
            ALIVE.fetch_add(if lock.as_bool() { 1 } else { -1 }, Ordering::SeqCst);
            Ok(())
        }
    }

    #[unsafe(no_mangle)]
    extern "system" fn DllGetClassObject(class: *const GUID, iid: *const GUID, object: *mut *mut c_void) -> HRESULT {
        if object.is_null() || class.is_null() {
            return E_POINTER;
        }
        unsafe { *object = null_mut() };
        if unsafe { *class } != CLASS {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        let factory: IClassFactory = Factory.into();
        unsafe { factory.query(iid, object) }
    }

    #[unsafe(no_mangle)]
    extern "system" fn DllCanUnloadNow() -> HRESULT {
        if ALIVE.load(Ordering::SeqCst) <= 0 { S_OK } else { S_FALSE }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picture_is_found_in_a_massif_file() {
        // A header pointing at a directory of two sections, the second the
        // picture.
        let mut file = b"MASSIF\x1a\n".to_vec();
        file.resize(64, 0);
        let picture = b"\xff\xd8 a JPEG \xff\xd9".to_vec();
        let at = file.len() as u64;
        file.extend_from_slice(&picture);
        let directory_at = file.len() as u64;
        for (kind, offset, len) in [(b"META", 0u64, 0u64), (b"THMB", at, picture.len() as u64)] {
            file.extend_from_slice(kind);
            file.extend_from_slice(&0u32.to_le_bytes());
            file.extend_from_slice(&offset.to_le_bytes());
            file.extend_from_slice(&len.to_le_bytes());
            file.extend_from_slice(&[0u8; 8]);
        }
        file[0x10..0x18].copy_from_slice(&directory_at.to_le_bytes());
        file[0x18..0x1c].copy_from_slice(&64u32.to_le_bytes());
        let mut read = |at: u64, len: usize| file.get(at as usize..at as usize + len).map(<[u8]>::to_vec);
        assert_eq!(find_picture(&mut read), Some(picture));
        // Not a Massif file: nothing.
        let mut other = |_: u64, len: usize| Some(vec![0u8; len]);
        assert_eq!(find_picture(&mut other), None);
    }

    /// The picture a saved map keeps is the one found.
    #[test]
    fn a_saved_map_shows_its_picture() {
        use massif_core::generate::{GenParams, generate};
        let mut world = generate(&GenParams { tiles_x: 2, tiles_z: 2, ..GenParams::default() });
        let path = std::env::temp_dir().join(format!("massif-thumbnails-{}.massif", std::process::id()));
        massif_format::save(&world.snapshot(), &path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let mut read = |at: u64, len: usize| bytes.get(at as usize..at as usize + len).map(<[u8]>::to_vec);
        let found = find_picture(&mut read).expect("a picture");
        assert_eq!(Some(found), massif_format::read_thumbnail(&path).unwrap());
        std::fs::remove_file(&path).unwrap();
    }
}
