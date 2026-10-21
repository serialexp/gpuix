use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

const MAGIC: &[u8; 16] = b"GPUIX-PAYLOAD-01";
const FOOTER_SIZE: u64 = 32;

struct Image {
    end: u64,
    linkedit: Option<(u64, u64)>,
    certificate: Option<u64>,
}

fn word(bytes: &[u8], offset: usize) -> Result<u32, String> {
    bytes
        .get(offset..offset + 4)
        .and_then(|value| value.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| "truncated executable header".into())
}

fn image(file: &mut File) -> Result<Image, String> {
    let length = file.metadata().map_err(|error| error.to_string())?.len();
    file.rewind().map_err(|error| error.to_string())?;
    let mut header = [0; 64];
    file.read_exact(&mut header)
        .map_err(|error| error.to_string())?;
    let mut result = Image {
        end: length,
        linkedit: None,
        certificate: None,
    };
    if word(&header, 0)? == 0xfeedfacf {
        let size = word(&header, 20)? as usize;
        if size > 1024 * 1024 || size as u64 + 32 > length {
            return Err("invalid Mach-O load command size".into());
        }
        file.seek(SeekFrom::Start(32))
            .map_err(|error| error.to_string())?;
        let mut commands = vec![0; size];
        file.read_exact(&mut commands)
            .map_err(|error| error.to_string())?;
        let mut offset = 0;
        for _ in 0..word(&header, 16)? {
            let kind = word(&commands, offset)?;
            let size = word(&commands, offset + 4)? as usize;
            if size < 8 || offset + size > commands.len() {
                return Err("invalid Mach-O load command".into());
            }
            if kind == 0x19 && size >= 72 && &commands[offset + 8..offset + 18] == b"__LINKEDIT" {
                let start =
                    u64::from_le_bytes(commands[offset + 40..offset + 48].try_into().unwrap());
                result.linkedit = Some((32 + offset as u64, start));
            }
            if kind == 0x1d && size >= 16 && word(&commands, offset + 12)? != 0 {
                result.end = word(&commands, offset + 8)? as u64;
                if result.end > length || result.end + word(&commands, offset + 12)? as u64 > length
                {
                    return Err("invalid Mach-O signature range".into());
                }
            }
            offset += size;
        }
    } else if &header[..2] == b"MZ" {
        let offset = word(&header, 60)? as u64;
        file.seek(SeekFrom::Start(offset))
            .map_err(|error| error.to_string())?;
        let mut pe = [0; 160];
        file.read_exact(&mut pe)
            .map_err(|error| error.to_string())?;
        if &pe[..4] != b"PE\0\0" || &pe[24..26] != b"\x0b\x02" {
            return Err("only PE32+ Windows runners are supported".into());
        }
        let directory = offset + 24 + 112 + 32;
        file.seek(SeekFrom::Start(directory))
            .map_err(|error| error.to_string())?;
        let mut security = [0; 8];
        file.read_exact(&mut security)
            .map_err(|error| error.to_string())?;
        let certificate = word(&security, 0)? as u64;
        let size = word(&security, 4)? as u64;
        if size != 0 {
            if certificate + size != length {
                return Err("unsupported PE certificate placement".into());
            }
            result.end = certificate;
            result.certificate = Some(directory);
        }
    } else if &header[..4] != b"\x7fELF" {
        return Err("unsupported runner format (expected thin Mach-O, ELF or PE32+)".into());
    }
    Ok(result)
}

fn footer(file: &mut File, end: u64) -> Result<Option<(u64, u64, u64)>, String> {
    for padding in 0..16 {
        let Some(position) = end.checked_sub(padding + FOOTER_SIZE) else {
            continue;
        };
        file.seek(SeekFrom::Start(position))
            .map_err(|error| error.to_string())?;
        let mut bytes = [0; FOOTER_SIZE as usize];
        file.read_exact(&mut bytes)
            .map_err(|error| error.to_string())?;
        if &bytes[16..] != MAGIC {
            continue;
        }
        let size = u64::from_le_bytes(bytes[..8].try_into().unwrap());
        let base = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
        let start = position
            .checked_sub(size)
            .ok_or("invalid embedded payload size")?;
        if base > start || base < 64 || size > 512 * 1024 * 1024 {
            return Err("invalid embedded payload bounds".into());
        }
        return Ok(Some((start, size, base)));
    }
    Ok(None)
}

pub(super) fn read(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let layout = image(&mut file)?;
    let Some((start, size, _)) = footer(&mut file, layout.end)? else {
        return Ok(None);
    };
    file.seek(SeekFrom::Start(start))
        .map_err(|error| error.to_string())?;
    let mut bytes = vec![0; size as usize];
    file.read_exact(&mut bytes)
        .map_err(|error| error.to_string())?;
    Ok(Some(bytes))
}

#[cfg(target_os = "macos")]
pub(super) fn signed(path: &Path) -> Result<bool, String> {
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    Ok(image(&mut file)?.end != file.metadata().map_err(|error| error.to_string())?.len())
}

pub(super) fn append(path: &Path, payload: &[u8]) -> Result<(), String> {
    if payload.len() > 512 * 1024 * 1024 {
        return Err("application payload exceeds 512 MiB".into());
    }
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    let layout = image(&mut file)?;
    if layout.linkedit.is_some()
        && layout.end != file.metadata().map_err(|error| error.to_string())?.len()
    {
        return Err("remove the Mach-O signature before inserting a payload".into());
    }
    let base = footer(&mut file, layout.end)?
        .map(|(_, _, base)| base)
        .unwrap_or(layout.end);
    if let Some(directory) = layout.certificate {
        file.seek(SeekFrom::Start(directory))
            .map_err(|error| error.to_string())?;
        file.write_all(&[0; 8]).map_err(|error| error.to_string())?;
    }
    file.set_len(base).map_err(|error| error.to_string())?;
    file.seek(SeekFrom::Start(base))
        .map_err(|error| error.to_string())?;
    let padding = (16 - (base + payload.len() as u64 + FOOTER_SIZE) % 16) % 16;
    file.write_all(&[0; 16][..padding as usize])
        .map_err(|error| error.to_string())?;
    file.write_all(payload).map_err(|error| error.to_string())?;
    file.write_all(&(payload.len() as u64).to_le_bytes())
        .map_err(|error| error.to_string())?;
    file.write_all(&base.to_le_bytes())
        .map_err(|error| error.to_string())?;
    file.write_all(MAGIC).map_err(|error| error.to_string())?;
    let end = file.stream_position().map_err(|error| error.to_string())?;
    if let Some((command, start)) = layout.linkedit {
        let size = end.checked_sub(start).ok_or("invalid __LINKEDIT range")?;
        file.seek(SeekFrom::Start(command + 32))
            .map_err(|error| error.to_string())?;
        file.write_all(&size.div_ceil(16384).saturating_mul(16384).to_le_bytes())
            .map_err(|error| error.to_string())?;
        file.seek(SeekFrom::Start(command + 48))
            .map_err(|error| error.to_string())?;
        file.write_all(&size.to_le_bytes())
            .map_err(|error| error.to_string())?;
    }
    file.sync_all().map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_replaces_an_elf_overlay() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("runner");
        let mut bytes = vec![0; 128];
        bytes[..4].copy_from_slice(b"\x7fELF");
        std::fs::write(&path, bytes).unwrap();
        assert!(read(&path).unwrap().is_none());
        append(&path, b"first application").unwrap();
        assert_eq!(read(&path).unwrap().unwrap(), b"first application");
        append(&path, b"second").unwrap();
        assert_eq!(read(&path).unwrap().unwrap(), b"second");
        assert!(std::fs::metadata(path).unwrap().len() < 200);
    }

    #[test]
    fn rejects_corrupt_payload_lengths() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("runner");
        let mut bytes = vec![0; 128];
        bytes[..4].copy_from_slice(b"\x7fELF");
        bytes.extend_from_slice(&u64::MAX.to_le_bytes());
        bytes.extend_from_slice(&128u64.to_le_bytes());
        bytes.extend_from_slice(MAGIC);
        std::fs::write(&path, bytes).unwrap();
        assert!(read(&path).is_err());
    }

    #[test]
    fn strips_pe_certificates_before_inserting_an_archive() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("runner.exe");
        let mut bytes = vec![0; 528];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[60..64].copy_from_slice(&64u32.to_le_bytes());
        bytes[64..68].copy_from_slice(b"PE\0\0");
        bytes[88..90].copy_from_slice(b"\x0b\x02");
        bytes[232..236].copy_from_slice(&512u32.to_le_bytes());
        bytes[236..240].copy_from_slice(&16u32.to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
        append(&path, b"windows app").unwrap();
        assert_eq!(read(&path).unwrap().unwrap(), b"windows app");
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[232..240], &[0; 8]);
        append(&path, b"replacement").unwrap();
        assert_eq!(read(&path).unwrap().unwrap(), b"replacement");
    }

    #[test]
    fn finds_macho_archives_before_code_signatures() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("runner");
        let mut bytes = vec![0; 160];
        bytes[..4].copy_from_slice(&0xfeedfacfu32.to_le_bytes());
        bytes[16..20].copy_from_slice(&2u32.to_le_bytes());
        bytes[20..24].copy_from_slice(&88u32.to_le_bytes());
        bytes[32..36].copy_from_slice(&0x19u32.to_le_bytes());
        bytes[36..40].copy_from_slice(&72u32.to_le_bytes());
        bytes[40..50].copy_from_slice(b"__LINKEDIT");
        bytes[72..80].copy_from_slice(&128u64.to_le_bytes());
        bytes[104..108].copy_from_slice(&0x1du32.to_le_bytes());
        bytes[108..112].copy_from_slice(&16u32.to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
        append(&path, b"mac app").unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        let end = bytes.len() as u32;
        bytes[112..116].copy_from_slice(&end.to_le_bytes());
        bytes[116..120].copy_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&[0; 16]);
        std::fs::write(&path, bytes).unwrap();
        assert_eq!(read(&path).unwrap().unwrap(), b"mac app");
        assert!(append(&path, b"replacement").is_err());
    }
}
