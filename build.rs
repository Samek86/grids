// Embeds app.manifest (common controls v6 + per-monitor DPI) and app.ico (made by icon.py).
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
