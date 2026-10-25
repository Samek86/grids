// Embeds app.manifest (common controls v6 + per-monitor DPI) and app.ico (the Panegrid icon kit's Panegrid.ico).
// Writes a binary .res by hand and lets windres only convert it to COFF, so no C preprocessor is needed.
fn main() {
    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rerun-if-changed=app.ico");
    let out = std::env::var("OUT_DIR").unwrap();
    let entry = |size: usize, typ: u16, id: u16, lang: u16| {
        let mut h = Vec::new();
        h.extend((size as u32).to_le_bytes());
        h.extend(32u32.to_le_bytes()); // header size
        h.extend([0xFFFF, typ, 0xFFFF, id].iter().flat_map(|w: &u16| w.to_le_bytes()));
        h.extend(0u32.to_le_bytes()); // data version
        h.extend([0u16, lang].iter().flat_map(|w| w.to_le_bytes())); // memory flags, language
        h.extend([0u32; 2].iter().flat_map(|d| d.to_le_bytes())); // version, characteristics
        h
    };
    let mut res = entry(0, 0, 0, 0); // mandatory empty first entry
    let mut add = |typ: u16, id: u16, data: &[u8]| {
        res.extend(entry(data.len(), typ, id, 0x0409));
        res.extend(data);
        res.resize(res.len().next_multiple_of(4), 0);
    };
    add(24, 1, &std::fs::read("app.manifest").unwrap()); // RT_MANIFEST

    // app.ico -> one RT_ICON per image + an RT_GROUP_ICON (id 1) that lists them
    let ico = std::fs::read("app.ico").unwrap();
    let count = u16::from_le_bytes([ico[4], ico[5]]);
    let mut group = vec![0, 0, 1, 0];
    group.extend(count.to_le_bytes());
    for k in 0..count as usize {
        let e = &ico[6 + 16 * k..22 + 16 * k];
        let size = u32::from_le_bytes(e[8..12].try_into().unwrap()) as usize;
        let offset = u32::from_le_bytes(e[12..16].try_into().unwrap()) as usize;
        add(3, k as u16 + 1, &ico[offset..offset + size]); // RT_ICON
        group.extend(&e[..12]); // width, height, colours, reserved, planes, bit count, bytes
        group.extend((k as u16 + 1).to_le_bytes());
    }
    add(14, 1, &group); // RT_GROUP_ICON
    add(16, 1, &version_info()); // RT_VERSION
    std::fs::write(format!("{out}/app.res"), res).unwrap();

    let obj = format!("{out}/app.res.o");
    let windres = std::env::var("WINDRES").unwrap_or("windres".into());
    let ok = std::process::Command::new(windres)
        .args(["-J", "res", "-i", &format!("{out}/app.res"), "-O", "coff", "-o", &obj])
        .status()
        .is_ok_and(|s| s.success());
    assert!(ok, "windres failed: set WINDRES to msys2's windres.exe");
    println!("cargo:rustc-link-arg-bins={obj}");
}

// VS_VERSIONINFO. wLength is the node size in bytes and includes the zero bytes
// that keep every node DWORD-aligned. String wValueLength counts UTF-16 code
// units, including the NUL; binary nodes (fixed info, Translation) count bytes.
// wType is 1 for text and 0 for binary.
fn utf16z(s: &str) -> Vec<u8> {
    let mut v = Vec::with_capacity((s.len() + 1) * 2);
    for u in s.encode_utf16() {
        v.extend(u.to_le_bytes());
    }
    v.extend(0u16.to_le_bytes());
    v
}

fn version_node(key: &str, value: &[u8], value_len: u16, text: bool, children: &[Vec<u8>]) -> Vec<u8> {
    let mut body = utf16z(key);
    while (6 + body.len()) % 4 != 0 {
        body.push(0);
    }
    body.extend_from_slice(value);
    while (6 + body.len()) % 4 != 0 {
        body.push(0);
    }
    for child in children {
        body.extend_from_slice(child);
    }
    while (6 + body.len()) % 4 != 0 {
        body.push(0);
    }
    let w_length = (6 + body.len()) as u16;
    let mut out = Vec::with_capacity(w_length as usize);
    out.extend(w_length.to_le_bytes());
    out.extend(value_len.to_le_bytes());
    out.extend(u16::from(text).to_le_bytes());
    out.extend(body);
    assert_eq!(out.len(), w_length as usize);
    assert_eq!(out.len() % 4, 0);
    out
}

fn version_string(key: &str, value: &str) -> Vec<u8> {
    let words: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes: Vec<u8> = words.iter().copied().flat_map(|u| u.to_le_bytes()).collect();
    version_node(key, &bytes, words.len() as u16, true, &[])
}

fn file_version() -> [u16; 4] {
    let mut v = [0u16; 4];
    for (i, part) in env!("CARGO_PKG_VERSION").split(['.', '-', '+']).take(4).enumerate() {
        let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = digits.parse() {
            v[i] = n;
        }
    }
    v
}

fn version_info() -> Vec<u8> {
    let ver = file_version();
    let mut fixed = Vec::with_capacity(52);
    let dw = |b: &mut Vec<u8>, n: u32| b.extend(n.to_le_bytes());
    dw(&mut fixed, 0xFEEF_04BD);
    dw(&mut fixed, 0x0001_0000);
    let ms = ((ver[0] as u32) << 16) | ver[1] as u32;
    let ls = ((ver[2] as u32) << 16) | ver[3] as u32;
    dw(&mut fixed, ms);
    dw(&mut fixed, ls);
    dw(&mut fixed, ms);
    dw(&mut fixed, ls);
    dw(&mut fixed, 0x3F);
    dw(&mut fixed, 0);
    dw(&mut fixed, 0x0004_0004);
    dw(&mut fixed, 1);
    dw(&mut fixed, 0);
    dw(&mut fixed, 0);
    dw(&mut fixed, 0);
    assert_eq!(fixed.len(), 52);

    let ver_text = env!("CARGO_PKG_VERSION");
    let strings = [
        version_string("CompanyName", "Samek86"),
        version_string("FileDescription", "Panegrid"),
        version_string("ProductName", "Panegrid"),
        version_string("InternalName", "panegrid"),
        version_string("OriginalFilename", "panegrid.exe"),
        version_string("FileVersion", ver_text),
        version_string("ProductVersion", ver_text),
        version_string("LegalCopyright", "Fork of efww/grids"),
    ];
    let table = version_node("040904B0", &[], 0, true, &strings);
    let string_file = version_node("StringFileInfo", &[], 0, true, &[table]);
    let translation = {
        let mut v = Vec::with_capacity(4);
        v.extend(0x0409u16.to_le_bytes());
        v.extend(0x04B0u16.to_le_bytes());
        version_node("Translation", &v, 4, false, &[])
    };
    let var_file = version_node("VarFileInfo", &[], 0, false, &[translation]);
    version_node("VS_VERSION_INFO", &fixed, 52, false, &[string_file, var_file])
}
