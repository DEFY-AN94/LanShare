//! The clipboard, read only when the user sends it to a phone (menu bar,
//! global shortcut) and written by 复制 / 复制最近收到的文字. Its content
//! never reaches the log.
//!
//! macOS reads the pasteboard natively: a screenshot is sent as the PNG
//! macOS holds, colour profile and all. Windows uses arboard.

use crate::core::bridge::Clip;

const NOTHING_TO_SEND: &str = "剪贴板里的内容不能发送，只能发送文字和图片";

#[cfg(target_os = "macos")]
mod imp {
    use objc2_app_kit::{
        NSBitmapImageFileType, NSBitmapImageRep, NSPasteboard, NSPasteboardTypeFileURL, NSPasteboardTypePNG, NSPasteboardTypeString, NSPasteboardTypeTIFF,
        NSPasteboardTypeURL,
    };
    use std::path::PathBuf;

    use objc2_foundation::{NSDictionary, NSString, NSURL};

    use super::{Clip, NOTHING_TO_SEND};

    /// The files the pasteboard names, one per item.
    fn files(pb: &NSPasteboard) -> Vec<PathBuf> {
        // SAFETY: as in `read_from`.
        unsafe {
            let Some(items) = pb.pasteboardItems() else { return Vec::new() };
            items
                .iter()
                .filter_map(|item| {
                    let text = item.stringForType(NSPasteboardTypeFileURL)?;
                    let url = NSURL::URLWithString(&text)?;
                    // Finder may give a file reference (file:///.file/id=…).
                    Some(PathBuf::from(url.filePathURL()?.path()?.to_string()))
                })
                .collect()
        }
    }

    /// The picture on the pasteboard as PNG.
    fn picture(pb: &NSPasteboard) -> Option<Vec<u8>> {
        // SAFETY: as in `read_from`.
        unsafe {
            if let Some(png) = pb.dataForType(NSPasteboardTypePNG) {
                return Some(png.to_vec());
            }
            // An image copied from Preview is often TIFF only.
            let tiff = pb.dataForType(NSPasteboardTypeTIFF)?;
            let png = NSBitmapImageRep::imageRepWithData(&tiff)?.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())?;
            Some(png.to_vec())
        }
    }

    pub(super) fn read_from(pb: &NSPasteboard) -> Clip {
        // SAFETY: the pasteboard type constants are valid for the process lifetime.
        unsafe {
            let types = pb.types().map(|t| t.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap_or_default();
            if types.is_empty() {
                return Clip::Empty;
            }
            let has = |t: &NSString| types.iter().any(|x| *x == t.to_string());
            // Finder offers a copied file's name as text and its icon; QQ
            // offers a copied picture as an image too. The file is what was
            // copied; QQ's image stands in when its file cannot be read.
            if has(NSPasteboardTypeFileURL) {
                let icon = types.iter().any(|t| t == "com.apple.icns");
                return Clip::Files { paths: files(pb), image: if icon { None } else { picture(pb) } };
            }
            let text = pb.stringForType(NSPasteboardTypeString).map(|s| s.to_string()).filter(|t| !t.trim().is_empty());
            // "Copy Image" in a browser also offers the image's address as text.
            let browser_picture = has(NSPasteboardTypeURL) && (has(NSPasteboardTypePNG) || has(NSPasteboardTypeTIFF));
            if !browser_picture && let Some(text) = text {
                return Clip::Text(text);
            }
            if let Some(png) = picture(pb) {
                return Clip::Png(png);
            }
            match text {
                Some(text) => Clip::Text(text),
                None => Clip::Unsupported(NOTHING_TO_SEND.into()),
            }
        }
    }

    pub(super) fn write_to(pb: &NSPasteboard, text: &str) -> Result<(), String> {
        // SAFETY: as above.
        let ok = unsafe {
            pb.clearContents();
            pb.setString_forType(&NSString::from_str(text), NSPasteboardTypeString)
        };
        if ok { Ok(()) } else { Err("没能写入剪贴板".into()) }
    }

    pub fn read() -> Clip {
        read_from(&NSPasteboard::generalPasteboard())
    }

    pub fn write_text(text: &str) -> Result<(), String> {
        write_to(&NSPasteboard::generalPasteboard(), text)
    }

    #[cfg(test)]
    pub(super) fn private_pasteboard() -> objc2::rc::Retained<NSPasteboard> {
        NSPasteboard::pasteboardWithUniqueName()
    }
}

#[cfg(windows)]
mod imp {
    use super::{Clip, NOTHING_TO_SEND};

    fn png_from_rgba(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, png::EncodingError> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(rgba)?;
        Ok(out)
    }

    const BUSY: &str = "没能打开剪贴板，请稍后再试";

    pub fn read() -> Clip {
        let Ok(mut clipboard) = arboard::Clipboard::new() else {
            return Clip::Unsupported(BUSY.into());
        };
        // Files copied in Explorer, or a picture QQ or WeChat copies as its file.
        if let Ok(files) = clipboard.get().file_list()
            && !files.is_empty()
        {
            return Clip::Files { paths: files, image: None };
        }
        match clipboard.get_text() {
            Ok(text) if !text.trim().is_empty() => return Clip::Text(text),
            // Another app holds the clipboard open.
            Err(arboard::Error::ClipboardOccupied) => return Clip::Unsupported(BUSY.into()),
            _ => {}
        }
        match clipboard.get_image() {
            Ok(img) => match png_from_rgba(img.width as u32, img.height as u32, &img.bytes) {
                Ok(png) => Clip::Png(png),
                Err(_) => Clip::Unsupported(NOTHING_TO_SEND.into()),
            },
            Err(arboard::Error::ContentNotAvailable) => Clip::Empty,
            Err(arboard::Error::ClipboardOccupied) => Clip::Unsupported(BUSY.into()),
            Err(_) => Clip::Unsupported(NOTHING_TO_SEND.into()),
        }
    }

    pub fn write_text(text: &str) -> Result<(), String> {
        arboard::Clipboard::new().and_then(|mut c| c.set_text(text)).map_err(|_| "没能写入剪贴板".to_string())
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod imp {
    use super::Clip;

    pub fn read() -> Clip {
        Clip::Unsupported("这里不能读取剪贴板".into())
    }

    pub fn write_text(_text: &str) -> Result<(), String> {
        Err("这里不能写入剪贴板".into())
    }
}

pub use imp::{read, write_text};

/// On a private pasteboard: the general one, the user's clipboard, is never touched.
#[cfg(all(test, target_os = "macos"))]
mod tests {
    use objc2_app_kit::{NSBitmapImageRep, NSPasteboardTypeFileURL, NSPasteboardTypePNG, NSPasteboardTypeString, NSPasteboardTypeTIFF, NSPasteboardTypeURL};
    use objc2_foundation::{NSData, NSString};

    use super::Clip;
    use super::imp::{private_pasteboard, read_from, write_to};

    /// A 1×1 transparent PNG.
    const PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
        0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
        0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    #[test]
    fn reads_text_pictures_and_refuses_files() {
        let pb = private_pasteboard();
        if write_to(&pb, "probe").is_err() {
            eprintln!("no pasteboard service here; skipped");
            return;
        }
        let long = "长".repeat(300);
        write_to(&pb, &long).unwrap();
        assert_eq!(read_from(&pb), Clip::Text(long));
        unsafe {
            // A screenshot: the PNG as macOS holds it.
            pb.clearContents();
            pb.setData_forType(Some(&NSData::with_bytes(PNG)), NSPasteboardTypePNG);
            assert_eq!(read_from(&pb), Clip::Png(PNG.to_vec()));
            // TIFF only (copied from Preview): converted to PNG.
            let tiff = NSBitmapImageRep::imageRepWithData(&NSData::with_bytes(PNG)).unwrap().TIFFRepresentation().unwrap();
            pb.clearContents();
            pb.setData_forType(Some(&tiff), NSPasteboardTypeTIFF);
            match read_from(&pb) {
                Clip::Png(png) => assert!(png.starts_with(b"\x89PNG"), "a PNG"),
                other => panic!("{other:?}"),
            }
            // "Copy Image" in Safari: the picture, with its address as URL and text.
            pb.clearContents();
            pb.setData_forType(Some(&tiff), NSPasteboardTypeTIFF);
            pb.setString_forType(&NSString::from_str("https://example.com/cat.jpg"), NSPasteboardTypeURL);
            pb.setString_forType(&NSString::from_str("https://example.com/cat.jpg"), NSPasteboardTypeString);
            assert!(matches!(read_from(&pb), Clip::Png(_)), "{:?}", read_from(&pb));
            // A link on its own is text.
            pb.clearContents();
            pb.setString_forType(&NSString::from_str("https://example.com/"), NSPasteboardTypeURL);
            pb.setString_forType(&NSString::from_str("https://example.com/"), NSPasteboardTypeString);
            assert_eq!(read_from(&pb), Clip::Text("https://example.com/".into()));
            // A file copied in Finder also offers its name as text: the file is what counts.
            pb.clearContents();
            pb.setString_forType(&NSString::from_str("file:///tmp/a%20b.pdf"), NSPasteboardTypeFileURL);
            pb.setString_forType(&NSString::from_str("a b.pdf"), NSPasteboardTypeString);
            assert_eq!(read_from(&pb), Clip::Files { paths: vec!["/tmp/a b.pdf".into()], image: None });
            // Finder also offers the file's icon: never sent in its place.
            let icns = NSString::from_str("com.apple.icns");
            pb.setData_forType(Some(&tiff), NSPasteboardTypeTIFF);
            pb.setData_forType(Some(&NSData::with_bytes(b"icns")), &icns);
            assert_eq!(read_from(&pb), Clip::Files { paths: vec!["/tmp/a b.pdf".into()], image: None });
            // A picture copied in QQ: its file, and the picture itself.
            pb.clearContents();
            pb.setString_forType(&NSString::from_str("file:///tmp/9e90.jpeg"), NSPasteboardTypeFileURL);
            pb.setData_forType(Some(&tiff), NSPasteboardTypeTIFF);
            match read_from(&pb) {
                Clip::Files { paths, image: Some(png) } => {
                    assert_eq!(paths, [std::path::PathBuf::from("/tmp/9e90.jpeg")]);
                    assert!(png.starts_with(b"\x89PNG"));
                }
                other => panic!("{other:?}"),
            }
            pb.clearContents();
            assert_eq!(read_from(&pb), Clip::Empty);
            let _: () = objc2::msg_send![&*pb, releaseGlobally];
        }
    }
}
