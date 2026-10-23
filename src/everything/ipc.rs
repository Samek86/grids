// Everything 1.4 IPC over WM_COPYDATA. No window is created here.
// Query2 (EVERYTHING_IPC_COPYDATA_QUERY2W, dwData 18) is the 1.4.1 wide query.
// Query W (EVERYTHING_IPC_COPYDATAQUERYW, dwData 2) is the older wide query, used when Query2 is refused.
// Both put a 32-bit HWND in the struct. Windows sign-extends it, including on x64.

pub(crate) const COPYDATA_QUERY_W: usize = 2;
pub(crate) const COPYDATA_QUERY2_W: usize = 18;
pub(crate) const REPLY_ID: u32 = 0x4752_4945;
pub(crate) const REQUEST_NAME: u32 = 0x0000_0001;
pub(crate) const REQUEST_PATH: u32 = 0x0000_0002;
pub(crate) const SORT_NAME_ASCENDING: u32 = 1;
pub(crate) const FOLDER: u32 = 0x0000_0001;
/// Matches kept from one reply. The total can be larger, which sets `more`.
pub(crate) const MAX_RESULTS: u32 = 5000;

const REQUEST_FULL: u32 = 0x0000_0004;
const REQUEST_EXT: u32 = 0x0000_0008;
const REQUEST_SIZE: u32 = 0x0000_0010;
const REQUEST_DATE_CREATED: u32 = 0x0000_0020;
const REQUEST_DATE_MODIFIED: u32 = 0x0000_0040;
const REQUEST_DATE_ACCESSED: u32 = 0x0000_0080;
const REQUEST_ATTR: u32 = 0x0000_0100;
const REQUEST_FILELIST: u32 = 0x0000_0200;
const REQUEST_RUN: u32 = 0x0000_0400;
const REQUEST_DATE_RUN: u32 = 0x0000_0800;
const REQUEST_DATE_RECENT: u32 = 0x0000_1000;
const REQUEST_HL_NAME: u32 = 0x0000_2000;
const REQUEST_HL_PATH: u32 = 0x0000_4000;
const REQUEST_HL_FULL: u32 = 0x0000_8000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IpcQuery {
    pub(crate) copy_data: usize,
    pub(crate) bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IpcHit {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) dir: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IpcList {
    pub(crate) hits: Vec<IpcHit>,
    pub(crate) total: usize,
    pub(crate) more: bool,
}

pub(crate) fn query2(reply_hwnd: u32, reply_msg: u32, max_results: u32, text: &str) -> IpcQuery {
    let mut bytes = Vec::with_capacity(28 + text.len() * 2 + 2);
    push_u32(&mut bytes, reply_hwnd);
    push_u32(&mut bytes, reply_msg);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, max_results);
    push_u32(&mut bytes, REQUEST_NAME | REQUEST_PATH);
    push_u32(&mut bytes, SORT_NAME_ASCENDING);
    push_utf16z(&mut bytes, text);
    IpcQuery { copy_data: COPYDATA_QUERY2_W, bytes }
}

pub(crate) fn query_w(reply_hwnd: u32, reply_msg: u32, max_results: u32, text: &str) -> IpcQuery {
    let mut bytes = Vec::with_capacity(20 + text.len() * 2 + 2);
    push_u32(&mut bytes, reply_hwnd);
    push_u32(&mut bytes, reply_msg);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, max_results);
    push_utf16z(&mut bytes, text);
    IpcQuery { copy_data: COPYDATA_QUERY_W, bytes }
}

/// EVERYTHING_IPC_LIST2. `request_flags` is the reply's own flags, so extra columns are skipped.
pub(crate) fn parse_list2(bytes: &[u8], cap: usize) -> Option<IpcList> {
    if bytes.len() < 20 || cap == 0 {
        return None;
    }
    let total = u32_at(bytes, 0)? as usize;
    let num = u32_at(bytes, 4)? as usize;
    let flags = u32_at(bytes, 12)?;
    let n = num.min(cap);
    let mut hits = Vec::with_capacity(n);
    for i in 0..n {
        let base = 20 + i * 8;
        let Some(item_flags) = u32_at(bytes, base) else { break };
        let Some(data_off) = u32_at(bytes, base + 4) else { break };
        let Some((name, path)) = read_named(bytes, data_off as usize, flags) else { continue };
        if name.is_empty() {
            continue;
        }
        hits.push(hit(name, path, item_flags));
    }
    let more = total > hits.len();
    Some(IpcList { hits, total, more })
}

/// EVERYTHING_IPC_LISTW. Offsets are from the start of the list and point at a NUL-terminated UTF-16 string.
pub(crate) fn parse_list_w(bytes: &[u8], cap: usize) -> Option<IpcList> {
    if bytes.len() < 28 || cap == 0 {
        return None;
    }
    let total = u32_at(bytes, 8)? as usize;
    let num = u32_at(bytes, 20)? as usize;
    let n = num.min(cap);
    let mut hits = Vec::with_capacity(n);
    for i in 0..n {
        let base = 28 + i * 12;
        let Some(flags) = u32_at(bytes, base) else { break };
        let Some(name_off) = u32_at(bytes, base + 4) else { break };
        let Some(path_off) = u32_at(bytes, base + 8) else { break };
        let Some(name) = utf16z(bytes, name_off as usize) else { continue };
        if name.is_empty() {
            continue;
        }
        let path = utf16z(bytes, path_off as usize).unwrap_or_default();
        hits.push(hit(name, path, flags));
    }
    let more = total > hits.len();
    Some(IpcList { hits, total, more })
}

fn hit(name: String, path: String, flags: u32) -> IpcHit {
    let full = join_path(&path, &name);
    IpcHit { name, path: full, dir: flags & FOLDER != 0 }
}

fn join_path(path: &str, name: &str) -> String {
    if path.is_empty() {
        name.to_string()
    } else if path.ends_with('\\') || path.ends_with('/') {
        format!("{path}{name}")
    } else {
        format!("{path}\\{name}")
    }
}

fn read_named(bytes: &[u8], mut off: usize, flags: u32) -> Option<(String, String)> {
    let mut name = None;
    let mut path = None;
    for bit in [REQUEST_NAME, REQUEST_PATH, REQUEST_FULL, REQUEST_EXT, REQUEST_SIZE, REQUEST_DATE_CREATED, REQUEST_DATE_MODIFIED, REQUEST_DATE_ACCESSED, REQUEST_ATTR, REQUEST_FILELIST, REQUEST_RUN, REQUEST_DATE_RUN, REQUEST_DATE_RECENT, REQUEST_HL_NAME, REQUEST_HL_PATH, REQUEST_HL_FULL] {
        if flags & bit == 0 {
            continue;
        }
        if matches!(bit, REQUEST_SIZE | REQUEST_DATE_CREATED | REQUEST_DATE_MODIFIED | REQUEST_DATE_ACCESSED | REQUEST_DATE_RUN | REQUEST_DATE_RECENT) {
            off = off.checked_add(8)?;
            continue;
        }
        if matches!(bit, REQUEST_ATTR | REQUEST_RUN) {
            off = off.checked_add(4)?;
            continue;
        }
        let (text, next) = counted_utf16(bytes, off)?;
        off = next;
        if bit == REQUEST_NAME {
            name = Some(text);
        } else if bit == REQUEST_PATH {
            path = Some(text);
        }
    }
    Some((name?, path.unwrap_or_default()))
}

fn counted_utf16(bytes: &[u8], off: usize) -> Option<(String, usize)> {
    let len = u32_at(bytes, off)? as usize;
    if len > 32_768 {
        return None;
    }
    let start = off + 4;
    let end = start.checked_add(len.checked_mul(2)?)?;
    if end > bytes.len() {
        return None;
    }
    let mut u = Vec::with_capacity(len);
    for i in 0..len {
        let o = start + i * 2;
        u.push(u16::from_le_bytes([bytes[o], bytes[o + 1]]));
    }
    let next = end.saturating_add(2);
    Some((String::from_utf16_lossy(&u), next))
}

fn utf16z(bytes: &[u8], off: usize) -> Option<String> {
    if off > bytes.len() || off % 2 != 0 {
        return None;
    }
    let mut u = Vec::new();
    let mut i = off;
    while i + 1 < bytes.len() {
        let w = u16::from_le_bytes([bytes[i], bytes[i + 1]]);
        if w == 0 {
            break;
        }
        u.push(w);
        i += 2;
        if u.len() > 32_768 {
            break;
        }
    }
    Some(String::from_utf16_lossy(&u))
}

fn push_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn push_utf16z(buf: &mut Vec<u8>, text: &str) {
    for u in text.encode_utf16().chain(std::iter::once(0)) {
        buf.extend_from_slice(&u.to_le_bytes());
    }
}

fn u32_at(bytes: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(off..off + 4)?.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u32s(bytes: &[u8], n: usize) -> Vec<u32> {
        (0..n).map(|i| u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap())).collect()
    }

    #[test]
    fn query2_is_the_wide_v2_message_and_keeps_the_search_text() {
        let q = query2(0x1111_2222, REPLY_ID, MAX_RESULTS, r#""C:\Work\" ext:pdf"#);
        assert_eq!(q.copy_data, COPYDATA_QUERY2_W);
        assert_eq!(u32s(&q.bytes, 7), vec![0x1111_2222, REPLY_ID, 0, 0, MAX_RESULTS, REQUEST_NAME | REQUEST_PATH, SORT_NAME_ASCENDING]);
        let text: Vec<u16> = r#""C:\Work\" ext:pdf"#.encode_utf16().chain(std::iter::once(0)).collect();
        assert_eq!(q.bytes.len(), 28 + text.len() * 2);
        for (i, u) in text.iter().enumerate() {
            let o = 28 + i * 2;
            assert_eq!(u16::from_le_bytes([q.bytes[o], q.bytes[o + 1]]), *u);
        }
        let ko = query2(1, 2, 10, "보고서");
        let chars: Vec<u16> = "보고서".encode_utf16().chain(std::iter::once(0)).collect();
        assert_eq!(ko.bytes.len(), 28 + chars.len() * 2);
    }

    #[test]
    fn query_w_is_copydata_2_with_a_20_byte_header() {
        let q = query_w(7, 9, 3, "size:>1mb");
        assert_eq!(q.copy_data, COPYDATA_QUERY_W);
        assert_eq!(u32s(&q.bytes, 5), vec![7, 9, 0, 0, 3]);
        assert_eq!(q.bytes.len(), 20 + ("size:>1mb".encode_utf16().count() + 1) * 2);
    }

    #[test]
    fn list2_reads_name_and_path_and_marks_more() {
        let name = counted("Sub");
        let path = counted(r"C:\d");
        let data_off = 20 + 8;
        let mut bytes = Vec::new();
        for n in [3u32, 1, 0, REQUEST_NAME | REQUEST_PATH, SORT_NAME_ASCENDING] {
            bytes.extend_from_slice(&n.to_le_bytes());
        }
        bytes.extend_from_slice(&FOLDER.to_le_bytes());
        bytes.extend_from_slice(&(data_off as u32).to_le_bytes());
        bytes.extend_from_slice(&name);
        bytes.extend_from_slice(&path);
        let list = parse_list2(&bytes, 10).unwrap();
        assert!(list.more);
        assert_eq!(list.total, 3);
        assert_eq!(list.hits.len(), 1);
        assert!(list.hits[0].dir);
        assert_eq!(list.hits[0].name, "Sub");
        assert_eq!(list.hits[0].path, r"C:\d\Sub");
        assert!(parse_list2(&bytes[..10], 10).is_none());
        assert!(parse_list2(&bytes, 0).is_none());
        let capped = parse_list2(&bytes, 1).unwrap();
        assert_eq!(capped.hits.len(), 1);
        assert!(capped.more);
    }

    #[test]
    fn list_w_reads_offsets_from_the_start_of_the_buffer() {
        let mut bytes = Vec::new();
        for n in [0u32, 1, 2, 0, 1, 1, 0] {
            bytes.extend_from_slice(&n.to_le_bytes());
        }
        let name: Vec<u16> = "a.txt".encode_utf16().chain(std::iter::once(0)).collect();
        let path: Vec<u16> = r"C:\d".encode_utf16().chain(std::iter::once(0)).collect();
        let name_off = 28 + 12;
        let path_off = name_off + name.len() * 2;
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&(name_off as u32).to_le_bytes());
        bytes.extend_from_slice(&(path_off as u32).to_le_bytes());
        for u in name.iter().chain(&path) {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        let list = parse_list_w(&bytes, 10).unwrap();
        assert!(list.more);
        assert_eq!(list.hits.len(), 1);
        assert_eq!(list.hits[0].name, "a.txt");
        assert_eq!(list.hits[0].path, r"C:\d\a.txt");
        assert!(!list.hits[0].dir);
        assert!(parse_list_w(&[], 10).is_none());
    }

    fn counted(s: &str) -> Vec<u8> {
        let mut v = Vec::new();
        let chars: Vec<u16> = s.encode_utf16().collect();
        v.extend_from_slice(&(chars.len() as u32).to_le_bytes());
        for u in chars.into_iter().chain(std::iter::once(0)) {
            v.extend_from_slice(&u.to_le_bytes());
        }
        v
    }
}
