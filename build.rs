// Embeds app.manifest (common controls v6 + per-monitor DPI).
// Writes a binary .res by hand and lets windres only convert it to COFF, so no C preprocessor is needed.
fn main() {
    println!("cargo:rerun-if-changed=app.manifest");
    let out = std::env::var("OUT_DIR").unwrap();
    let data = std::fs::read("app.manifest").unwrap();
    let entry = |size: u32, typ: u16, id: u16, lang: u16| {
        let mut h = Vec::new();
        h.extend(size.to_le_bytes());
        h.extend(32u32.to_le_bytes()); // header size
        h.extend([0xFFFF, typ, 0xFFFF, id].iter().flat_map(|w: &u16| w.to_le_bytes()));
        h.extend(0u32.to_le_bytes()); // data version
        h.extend([0u16, lang].iter().flat_map(|w| w.to_le_bytes())); // memory flags, language
        h.extend([0u32; 2].iter().flat_map(|d| d.to_le_bytes())); // version, characteristics
        h
    };
    let mut res = entry(0, 0, 0, 0); // mandatory empty first entry
    res.extend(entry(data.len() as u32, 24, 1, 0x0409)); // RT_MANIFEST, id 1
    res.extend(&data);
    res.resize(res.len().next_multiple_of(4), 0);
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
