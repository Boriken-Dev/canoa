//! Canoa's mark as the picture the ribbon button shows.
//!
//! The PNG is the one the browser extension carries, rendered from the app's icon; it is read at build time, so the DLL
//! holds the bytes and loads nothing at run time. A ribbon image is an `IPictureDisp` over a 32-bit premultiplied bitmap,
//! which keeps the mark's transparency.

#![allow(unsafe_code)]

use core::ptr;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateDIBSection, DIB_RGB_COLORS, HBITMAP,
};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICImagingFactory,
    WICBitmapDitherTypeNone, WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, IDispatch};
use windows::Win32::System::Ole::{
    IPictureDisp, OleCreatePictureIndirect, PICTDESC, PICTDESC_0, PICTDESC_0_0, PICTYPE_BITMAP,
};
use windows::core::{Interface, Result};

/// The 32 by 32 mark; Office scales it for a large button at other densities.
pub const MARK_PNG: &[u8] = include_bytes!("../../../clients/browser/chromium/icons/canoa-32.png");

/// A bitmap picture of `png`, for a ribbon `getImage` callback.
///
/// COM must be initialised on the calling thread (Office's main thread is).
pub fn picture_from_png(png: &[u8]) -> Result<IDispatch> {
    // SAFETY: WIC calls on interfaces this function creates and owns; the buffers outlive each call.
    unsafe {
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let stream = factory.CreateStream()?;
        stream.InitializeFromMemory(png)?;
        let decoder = factory.CreateDecoderFromStream(
            &stream,
            ptr::null(),
            WICDecodeMetadataCacheOnDemand,
        )?;
        let frame = decoder.GetFrame(0)?;
        let converter = factory.CreateFormatConverter()?;
        converter.Initialize(
            &frame,
            &GUID_WICPixelFormat32bppPBGRA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )?;
        let (mut width, mut height) = (0_u32, 0_u32);
        converter.GetSize(&mut width, &mut height)?;

        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                // Negative: rows run top-down, as WIC writes them.
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = ptr::null_mut();
        let bitmap: HBITMAP = CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0)?;
        let stride = width * 4;
        // SAFETY of the slice: the section holds `stride * height` bytes at `bits`, alive as long as `bitmap`.
        let pixels = core::slice::from_raw_parts_mut(bits as *mut u8, (stride * height) as usize);
        converter.CopyPixels(ptr::null(), stride, pixels)?;

        let description = PICTDESC {
            cbSizeofstruct: size_of::<PICTDESC>() as u32,
            picType: PICTYPE_BITMAP.0 as u32,
            Anonymous: PICTDESC_0 {
                bmp: PICTDESC_0_0 {
                    hbitmap: bitmap,
                    ..Default::default()
                },
            },
        };
        // `fown` true: the picture deletes the bitmap when it is released.
        let picture: IPictureDisp = OleCreatePictureIndirect(&description, true)?;
        picture.cast()
    }
}

/// The mark's picture.
pub fn mark_picture() -> Result<IDispatch> {
    picture_from_png(MARK_PNG)
}
