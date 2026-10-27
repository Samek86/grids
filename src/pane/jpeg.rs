//! JPEG / EXIF inspection that does not panic on a hostile header.
//!
//! Maker-note bytes are never treated as an IFD. An offset past the TIFF blob is skipped.
//! A normal camera frame (a few megabytes, a few thousand pixels on a side) is accepted.
//! CMYK and progressive frames are described, not rejected: the decoder decides.

pub(crate) const MAX_EDGE: u32 = 16_384;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JpegMeta {
    pub width: u32,
    pub height: u32,
    pub progressive: bool,
    pub components: u8,
    pub orientation: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum JpegFault {
    Empty,
    NotJpeg,
    Truncated,
    ZeroSize,
    Huge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExifInfo {
    pub orientation: u16,
    /// A maker-note tag was present and its payload is not inside the TIFF blob.
    pub maker_note_out_of_range: bool,
    /// `Some` when the JPEG-thumbnail tags were present. `true` only if that range
    /// sits inside the TIFF blob and starts with a JPEG SOI.
    pub thumb_jpeg: Option<bool>,
}

pub(crate) fn inspect(data: &[u8]) -> Result<JpegMeta, JpegFault> {
    if data.is_empty() {
        return Err(JpegFault::Empty);
    }
    if data.len() < 2 || data[0] != 0xFF || data[1] != 0xD8 {
        return Err(JpegFault::NotJpeg);
    }
    let mut i = 2usize;
    let mut meta: Option<(u32, u32, u8)> = None;
    let mut progressive = false;
    let mut orientation = 1u16;
    while i + 1 < data.len() {
        if data[i] != 0xFF {
            break;
        }
        while i < data.len() && data[i] == 0xFF {
            i += 1;
        }
        if i >= data.len() {
            break;
        }
        let marker = data[i];
        i += 1;
        if marker == 0xD9 || marker == 0x00 {
            break;
        }
        if marker == 0xD8 || (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            continue;
        }
        if i + 1 >= data.len() {
            return if meta.is_some() { finish(meta, progressive, orientation) } else { Err(JpegFault::Truncated) };
        }
        let len = u16::from_be_bytes([data[i], data[i + 1]]) as usize;
        if len < 2 {
            return Err(JpegFault::Truncated);
        }
        let Some(body_end) = i.checked_add(len) else {
            return Err(JpegFault::Truncated);
        };
        if body_end > data.len() {
            return if meta.is_some() { finish(meta, progressive, orientation) } else { Err(JpegFault::Truncated) };
        }
        let body = &data[i + 2..body_end];
        match marker {
            0xE1 => {
                if let Some(info) = exif_from_app1(body) {
                    if (1..=8).contains(&info.orientation) {
                        orientation = info.orientation;
                    }
                }
            }
            0xC0 | 0xC1 | 0xC2 | 0xC3 | 0xC5 | 0xC6 | 0xC7 | 0xC9 | 0xCA | 0xCB | 0xCD | 0xCE | 0xCF => {
                if marker == 0xC2 || marker == 0xCA {
                    progressive = true;
                }
                if body.len() < 6 {
                    return Err(JpegFault::Truncated);
                }
                let h = u16::from_be_bytes([body[1], body[2]]) as u32;
                let w = u16::from_be_bytes([body[3], body[4]]) as u32;
                let c = body[5];
                if w == 0 || h == 0 {
                    return Err(JpegFault::ZeroSize);
                }
                if w > MAX_EDGE || h > MAX_EDGE || w.checked_mul(h).is_none() {
                    return Err(JpegFault::Huge);
                }
                meta = Some((w, h, c));
            }
            0xDA => break,
            _ => {}
        }
        i = body_end;
    }
    finish(meta, progressive, orientation)
}

fn finish(meta: Option<(u32, u32, u8)>, progressive: bool, orientation: u16) -> Result<JpegMeta, JpegFault> {
    match meta {
        Some((width, height, components)) => Ok(JpegMeta { width, height, progressive, components, orientation }),
        None => Err(JpegFault::Truncated),
    }
}

#[cfg(test)]
pub(crate) fn exif_in_jpeg(data: &[u8]) -> Option<ExifInfo> {
    let mut i = 2usize;
    if data.len() < 2 || data[0] != 0xFF || data[1] != 0xD8 {
        return None;
    }
    while i + 4 < data.len() {
        if data[i] != 0xFF {
            return None;
        }
        while i < data.len() && data[i] == 0xFF {
            i += 1;
        }
        if i >= data.len() {
            return None;
        }
        let marker = data[i];
        i += 1;
        if marker == 0xDA || marker == 0xD9 {
            return None;
        }
        if marker == 0xD8 || (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            continue;
        }
        if i + 1 >= data.len() {
            return None;
        }
        let len = u16::from_be_bytes([data[i], data[i + 1]]) as usize;
        let Some(body_end) = i.checked_add(len) else { return None };
        if len < 2 || body_end > data.len() {
            return None;
        }
        if marker == 0xE1 {
            return exif_from_app1(&data[i + 2..body_end]);
        }
        i = body_end;
    }
    None
}

pub(crate) fn exif_from_app1(body: &[u8]) -> Option<ExifInfo> {
    let tiff = body.strip_prefix(b"Exif\0\0")?;
    tiff_info(tiff)
}

fn tiff_info(tiff: &[u8]) -> Option<ExifInfo> {
    if tiff.len() < 8 {
        return None;
    }
    let be = match &tiff[0..2] {
        b"II" => false,
        b"MM" => true,
        _ => return None,
    };
    if u16_at(tiff, 2, be)? != 42 {
        return None;
    }
    let ifd = u32_at(tiff, 4, be)? as usize;
    let mut info = ExifInfo { orientation: 1, maker_note_out_of_range: false, thumb_jpeg: None };
    walk_ifd(tiff, ifd, be, 0, &mut info);
    Some(info)
}

fn walk_ifd(tiff: &[u8], off: usize, be: bool, depth: u8, info: &mut ExifInfo) {
    if depth > 6 {
        return;
    }
    let Some(count_at) = off.checked_add(2) else { return };
    if count_at > tiff.len() {
        return;
    }
    let Some(count) = u16_at(tiff, off, be) else { return };
    let count = count as usize;
    if count > 512 {
        return;
    }
    let mut exif_ptr: Option<usize> = None;
    let mut thumb_off: Option<usize> = None;
    let mut thumb_len: Option<usize> = None;
    for n in 0..count {
        let Some(step) = n.checked_mul(12) else { return };
        let Some(e) = off.checked_add(2).and_then(|b| b.checked_add(step)) else { return };
        if e.checked_add(12).map(|end| end > tiff.len()).unwrap_or(true) {
            return;
        }
        let Some(tag) = u16_at(tiff, e, be) else { return };
        let Some(typ) = u16_at(tiff, e + 2, be) else { return };
        let Some(cnt) = u32_at(tiff, e + 4, be) else { return };
        if tag == 0x927C {
            // Maker note. Never walk it as an IFD, even when the blob is inside the file.
            if payload_outside(tiff, e, typ, cnt, be) {
                info.maker_note_out_of_range = true;
            }
            continue;
        }
        if tag == 0x0112 && typ == 3 && cnt == 1 {
            if let Some(v) = inline_u16(tiff, e, be) {
                if (1..=8).contains(&v) {
                    info.orientation = v;
                }
            }
        }
        if tag == 0x8769 && typ == 4 && cnt == 1 {
            exif_ptr = u32_at(tiff, e + 8, be).map(|v| v as usize);
        }
        if tag == 0x0201 && typ == 4 && cnt == 1 {
            thumb_off = u32_at(tiff, e + 8, be).map(|v| v as usize);
        }
        if tag == 0x0202 && typ == 4 && cnt == 1 {
            thumb_len = u32_at(tiff, e + 8, be).map(|v| v as usize);
        }
    }
    if let (Some(start), Some(len)) = (thumb_off, thumb_len) {
        let end = start.saturating_add(len);
        let inside = len > 0 && end >= start && end <= tiff.len() && start < tiff.len();
        let jpeg = inside && tiff[start..].starts_with(&[0xFF, 0xD8]);
        info.thumb_jpeg = Some(inside && jpeg);
    }
    if let Some(p) = exif_ptr {
        if p < tiff.len() {
            walk_ifd(tiff, p, be, depth + 1, info);
        }
    }
}

fn payload_outside(tiff: &[u8], entry: usize, typ: u16, count: u32, be: bool) -> bool {
    let Some(unit) = type_size(typ) else { return true };
    let Some(nbytes) = (count as usize).checked_mul(unit) else { return true };
    if nbytes <= 4 {
        return false;
    }
    let Some(start) = u32_at(tiff, entry + 8, be).map(|v| v as usize) else { return true };
    match start.checked_add(nbytes) {
        Some(end) if end >= start && end <= tiff.len() => false,
        _ => true,
    }
}

fn inline_u16(tiff: &[u8], entry: usize, be: bool) -> Option<u16> {
    u16_at(tiff, entry + 8, be)
}

fn type_size(typ: u16) -> Option<usize> {
    Some(match typ {
        1 | 2 | 6 | 7 => 1,
        3 | 8 => 2,
        4 | 9 | 11 => 4,
        5 | 10 | 12 => 8,
        _ => return None,
    })
}

fn u16_at(b: &[u8], o: usize, be: bool) -> Option<u16> {
    let x = b.get(o..o + 2)?;
    Some(if be { u16::from_be_bytes([x[0], x[1]]) } else { u16::from_le_bytes([x[0], x[1]]) })
}

fn u32_at(b: &[u8], o: usize, be: bool) -> Option<u32> {
    let x = b.get(o..o + 4)?;
    Some(if be { u32::from_be_bytes([x[0], x[1], x[2], x[3]]) } else { u32::from_le_bytes([x[0], x[1], x[2], x[3]]) })
}

/// Edges of a PNG, GIF, or BMP header. `None` when the bytes are not one of those,
/// or the header is too short to judge. A hostile size is `Err`.
pub(crate) fn probe_edges(data: &[u8]) -> Option<Result<(u32, u32), JpegFault>> {
    let wh = if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        if data.len() < 24 {
            return None;
        }
        if &data[12..16] != b"IHDR" {
            return None;
        }
        Some((u32::from_be_bytes(data[16..20].try_into().ok()?), u32::from_be_bytes(data[20..24].try_into().ok()?)))
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        if data.len() < 10 {
            return None;
        }
        Some((u16::from_le_bytes([data[6], data[7]]) as u32, u16::from_le_bytes([data[8], data[9]]) as u32))
    } else if data.starts_with(b"BM") {
        if data.len() < 26 {
            return None;
        }
        let w = i32::from_le_bytes(data[18..22].try_into().ok()?);
        let h = i32::from_le_bytes(data[22..26].try_into().ok()?);
        if w < 0 {
            return Some(Err(JpegFault::Huge));
        }
        Some((w as u32, h.unsigned_abs()))
    } else {
        None
    }?;
    Some(check_edge(wh.0, wh.1))
}

fn check_edge(w: u32, h: u32) -> Result<(u32, u32), JpegFault> {
    if w == 0 || h == 0 {
        Err(JpegFault::ZeroSize)
    } else if w > MAX_EDGE || h > MAX_EDGE || w.checked_mul(h).is_none() {
        Err(JpegFault::Huge)
    } else {
        Ok((w, h))
    }
}

/// EXIF orientation applied to top-down BGRA. `None` leaves the caller's buffer alone.
pub(crate) fn orient_bgra(w: i32, h: i32, src: &[u8], orientation: u16) -> Option<(i32, i32, Vec<u8>)> {
    if !(1..=8).contains(&orientation) || w <= 0 || h <= 0 {
        return None;
    }
    let (wu, hu) = (w as usize, h as usize);
    if src.len() != wu.checked_mul(hu)?.checked_mul(4)? {
        return None;
    }
    if orientation == 1 {
        return Some((w, h, src.to_vec()));
    }
    let (nw, nh) = match orientation {
        5 | 6 | 7 | 8 => (hu, wu),
        _ => (wu, hu),
    };
    let mut dst = vec![0u8; nw.checked_mul(nh)?.checked_mul(4)?];
    for y in 0..hu {
        for x in 0..wu {
            let (dx, dy) = match orientation {
                2 => (wu - 1 - x, y),
                3 => (wu - 1 - x, hu - 1 - y),
                4 => (x, hu - 1 - y),
                5 => (y, x),
                6 => (hu - 1 - y, x),
                7 => (hu - 1 - y, wu - 1 - x),
                8 => (y, wu - 1 - x),
                _ => (x, y),
            };
            let s = (y * wu + x) * 4;
            let d = (dy * nw + dx) * 4;
            dst[d..d + 4].copy_from_slice(&src[s..s + 4]);
        }
    }
    Some((nw as i32, nh as i32, dst))
}

#[cfg(test)]
fn sof(marker: u8, w: u16, h: u16, components: u8) -> Vec<u8> {
    let mut v = Vec::new();
    let len = 8 + 3 * components as u16;
    v.push(0xFF);
    v.push(marker);
    v.extend(len.to_be_bytes());
    v.push(8);
    v.extend(h.to_be_bytes());
    v.extend(w.to_be_bytes());
    v.push(components);
    for i in 0..components {
        v.extend([i + 1, 0x11, 0]);
    }
    v
}

#[cfg(test)]
fn jpeg_with(app1: Option<&[u8]>, frame: &[u8]) -> Vec<u8> {
    let mut v = vec![0xFF, 0xD8];
    if let Some(exif) = app1 {
        let len = (exif.len() + 2) as u16;
        v.push(0xFF);
        v.push(0xE1);
        v.extend(len.to_be_bytes());
        v.extend(exif);
    }
    v.extend(frame);
    v.extend([0xFF, 0xD9]);
    v
}

#[cfg(test)]
fn tiff_le(entries: &[(u16, u16, u32, u32)], extra: &[u8]) -> Vec<u8> {
    let mut v = vec![b'I', b'I', 42, 0, 8, 0, 0, 0];
    v.extend((entries.len() as u16).to_le_bytes());
    for (tag, typ, cnt, val) in entries {
        v.extend(tag.to_le_bytes());
        v.extend(typ.to_le_bytes());
        v.extend(cnt.to_le_bytes());
        v.extend(val.to_le_bytes());
    }
    v.extend([0, 0, 0, 0]);
    v.extend(extra);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_sized_baseline_jpeg_is_accepted() {
        // A few thousand pixels on a side, the size of a ~3 MB camera JPEG. The scan is omitted.
        let j = jpeg_with(None, &sof(0xC0, 4000, 3000, 3));
        let m = inspect(&j).unwrap();
        assert_eq!((m.width, m.height, m.components, m.progressive, m.orientation), (4000, 3000, 3, false, 1));
    }

    #[test]
    fn orientation_cmyk_and_progressive() {
        let tiff = tiff_le(&[(0x0112, 3, 1, 6)], &[]);
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend(tiff);
        let turned = jpeg_with(Some(&app1), &sof(0xC0, 320, 240, 3));
        assert_eq!(inspect(&turned).unwrap().orientation, 6);
        let cmyk = jpeg_with(None, &sof(0xC0, 64, 48, 4));
        let m = inspect(&cmyk).unwrap();
        assert_eq!(m.components, 4);
        assert!(!m.progressive);
        let prog = jpeg_with(None, &sof(0xC2, 800, 600, 3));
        assert!(inspect(&prog).unwrap().progressive);
    }

    #[test]
    fn maker_note_past_end_does_not_panic_and_thumbnail_is_checked() {
        // Orientation 6, maker note claiming a payload at 0xFFFFFFF0, thumb tags past the blob.
        let tiff = tiff_le(
            &[
                (0x0112, 3, 1, 6),
                (0x927C, 7, 100, 0xFFFF_FFF0),
                (0x0201, 4, 1, 0x00F0_0000),
                (0x0202, 4, 1, 64),
            ],
            &[],
        );
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend(&tiff);
        let j = jpeg_with(Some(&app1), &sof(0xC0, 160, 120, 3));
        let m = inspect(&j).unwrap();
        assert_eq!(m.orientation, 6);
        let ex = exif_in_jpeg(&j).unwrap();
        assert!(ex.maker_note_out_of_range);
        assert_eq!(ex.thumb_jpeg, Some(false));
        assert_eq!(ex.orientation, 6);

        let mut entries_and_pad = tiff_le(&[(0x0112, 3, 1, 1), (0x0201, 4, 1, 0), (0x0202, 4, 1, 3)], &[]);
        let thumb_at = entries_and_pad.len();
        entries_and_pad.extend([0xFF, 0xD8, 0x00]);
        // Patch the thumbnail offset (second entry's value is at header 8 + 2 + 12 + 8).
        let val_at = 8 + 2 + 12 + 8;
        entries_and_pad[val_at..val_at + 4].copy_from_slice(&(thumb_at as u32).to_le_bytes());
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend(&entries_and_pad);
        let j = jpeg_with(Some(&app1), &sof(0xC0, 8, 8, 3));
        let ex = exif_in_jpeg(&j).unwrap();
        assert_eq!(ex.thumb_jpeg, Some(true));
        assert!(!ex.maker_note_out_of_range);
    }

    #[test]
    fn hostile_headers_are_errors() {
        assert_eq!(inspect(b""), Err(JpegFault::Empty));
        assert_eq!(inspect(b"GIF89a"), Err(JpegFault::NotJpeg));
        assert_eq!(inspect(&[0xFF, 0xD8]), Err(JpegFault::Truncated));
        assert_eq!(inspect(&[0xFF, 0xD8, 0xFF, 0xE1, 0x00, 0xFF]), Err(JpegFault::Truncated));
        assert_eq!(inspect(&jpeg_with(None, &sof(0xC0, 0, 10, 3))), Err(JpegFault::ZeroSize));
        assert_eq!(inspect(&jpeg_with(None, &sof(0xC0, 20_000, 20_000, 3))), Err(JpegFault::Huge));
        // An IFD count that would walk off the end is ignored; the frame still describes.
        let mut tiff = tiff_le(&[], &[]);
        tiff[8] = 0xFF;
        tiff[9] = 0xFF; // count = 65535
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend(&tiff);
        let j = jpeg_with(Some(&app1), &sof(0xC0, 32, 32, 3));
        assert_eq!(inspect(&j).unwrap().orientation, 1);
    }

    #[test]
    fn other_pictures_and_orientation_pixels() {
        let mut png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
        png.extend(4000u32.to_be_bytes());
        png.extend(3000u32.to_be_bytes());
        assert_eq!(probe_edges(&png).unwrap().unwrap(), (4000, 3000));
        let mut huge = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
        huge.extend(20_000u32.to_be_bytes());
        huge.extend(10u32.to_be_bytes());
        assert_eq!(probe_edges(&huge).unwrap(), Err(JpegFault::Huge));
        let mut gif = b"GIF89a".to_vec();
        gif.extend(2u16.to_le_bytes());
        gif.extend(0u16.to_le_bytes());
        assert_eq!(probe_edges(&gif).unwrap(), Err(JpegFault::ZeroSize));
        assert!(probe_edges(b"not a picture").is_none());

        let src = [1, 0, 0, 255, 2, 0, 0, 255];
        let (w, h, px) = orient_bgra(2, 1, &src, 6).unwrap();
        assert_eq!((w, h), (1, 2));
        assert_eq!(&px[0..4], &[1, 0, 0, 255]);
        assert_eq!(&px[4..8], &[2, 0, 0, 255]);
        assert!(orient_bgra(2, 1, &src, 99).is_none());
        assert!(orient_bgra(2, 1, &[0, 1, 2], 6).is_none());
    }
}
