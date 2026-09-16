use std::io::{Cursor, Read};
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum PkgError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("not a Wallpaper Engine package (bad magic: {0:?})")]
    BadMagic(String),
    #[error("truncated package: entry {path:?} claims bytes [{start}, {end}) but the file is only {len} bytes")]
    Truncated {
        path: String,
        start: usize,
        end: usize,
        len: usize,
    },
}

pub struct PkgEntry {
    pub path: String,
    pub data: Vec<u8>,
}

pub fn extract(pkg_path: impl AsRef<Path>) -> Result<Vec<PkgEntry>, PkgError> {
    let bytes = std::fs::read(pkg_path)?;
    extract_bytes(&bytes)
}

fn extract_bytes(bytes: &[u8]) -> Result<Vec<PkgEntry>, PkgError> {
    let mut cursor = Cursor::new(bytes);

    let magic = read_string(&mut cursor)?;
    if !magic.starts_with("PKGV") {
        return Err(PkgError::BadMagic(magic));
    }

    let entry_count = read_i32(&mut cursor)?;
    struct RawEntry {
        path: String,
        offset: i32,
        length: i32,
    }
    let mut raw_entries = Vec::with_capacity(entry_count.max(0) as usize);
    for _ in 0..entry_count {
        let path = read_string(&mut cursor)?;
        let offset = read_i32(&mut cursor)?;
        let length = read_i32(&mut cursor)?;
        raw_entries.push(RawEntry {
            path,
            offset,
            length,
        });
    }

    let data_start = cursor.position() as usize;
    let mut entries = Vec::with_capacity(raw_entries.len());
    for e in raw_entries {
        let start = data_start + e.offset.max(0) as usize;
        let end = start + e.length.max(0) as usize;
        let data = bytes
            .get(start..end)
            .ok_or_else(|| PkgError::Truncated {
                path: e.path.clone(),
                start,
                end,
                len: bytes.len(),
            })?
            .to_vec();
        entries.push(PkgEntry { path: e.path, data });
    }
    Ok(entries)
}

fn read_i32(cursor: &mut Cursor<&[u8]>) -> Result<i32, std::io::Error> {
    let mut buf = [0u8; 4];
    cursor.read_exact(&mut buf)?;
    Ok(i32::from_le_bytes(buf))
}

fn read_string(cursor: &mut Cursor<&[u8]>) -> Result<String, std::io::Error> {
    let len = read_i32(cursor)?.max(0) as usize;
    let mut buf = vec![0u8; len];
    cursor.read_exact(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_synthetic_pkg(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        write_string(&mut out, "PKGV0020");
        out.extend((entries.len() as i32).to_le_bytes());

        let mut data = Vec::new();
        for (path, content) in entries {
            write_string(&mut out, path);
            out.extend((data.len() as i32).to_le_bytes());
            out.extend((content.len() as i32).to_le_bytes());
            data.extend_from_slice(content);
        }
        out.extend(data);
        out
    }

    fn write_string(out: &mut Vec<u8>, s: &str) {
        out.extend((s.len() as i32).to_le_bytes());
        out.extend(s.as_bytes());
    }

    #[test]
    fn round_trips_a_synthetic_package() {
        let pkg = build_synthetic_pkg(&[
            ("scene.json", b"{\"objects\":[]}"),
            ("preview.tex", b"not-real-tex-data"),
        ]);
        let entries = extract_bytes(&pkg).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, "scene.json");
        assert_eq!(entries[0].data, b"{\"objects\":[]}");
        assert_eq!(entries[1].path, "preview.tex");
        assert_eq!(entries[1].data, b"not-real-tex-data");
    }

    #[test]
    fn rejects_a_file_with_the_wrong_magic() {
        let mut pkg = Vec::new();
        write_string(&mut pkg, "NOTAPKG!");
        pkg.extend(0i32.to_le_bytes());
        assert!(matches!(extract_bytes(&pkg), Err(PkgError::BadMagic(_))));
    }
}
