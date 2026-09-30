use std::collections::HashMap;
use std::ops::Range;
use std::sync::LazyLock;

const PREFIX: &str = "phosphor:";
const WEIGHTS: [&str; 6] = ["regular", "thin", "light", "bold", "fill", "duotone"];
const PACKED: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/phosphor-icons.bin"));

struct Registry {
    aliases: HashMap<String, String>,
    icons: [HashMap<String, Range<usize>>; 6],
}

static REGISTRY: LazyLock<Registry> = LazyLock::new(parse_registry);

pub fn token(name: &str, weight: &str) -> Result<String, String> {
    let weight = weight.to_ascii_lowercase();
    let Some(weight_index) = WEIGHTS.iter().position(|candidate| *candidate == weight) else {
        return Err(format!(
            "unknown Phosphor weight {weight:?}; expected regular, thin, light, bold, fill, or duotone"
        ));
    };
    let normalized = normalize_name(name);
    let canonical = REGISTRY
        .aliases
        .get(&normalized)
        .map(String::as_str)
        .unwrap_or(&normalized);
    let Some((canonical, _)) = REGISTRY.icons[weight_index].get_key_value(canonical) else {
        return Err(format!("unknown Phosphor icon {name:?}"));
    };
    Ok(format!("{PREFIX}{weight_index}:{canonical}"))
}

pub fn source_from_token(token: &str) -> Option<&'static [u8]> {
    let token = token.strip_prefix(PREFIX)?;
    let (weight, name) = token.split_once(':')?;
    let weight = weight.parse::<usize>().ok()?;
    let range = REGISTRY.icons.get(weight)?.get(name)?;
    Some(&PACKED[range.clone()])
}

fn parse_registry() -> Registry {
    assert_eq!(&PACKED[..6], b"GPXPH1", "invalid Phosphor icon pack");
    let mut cursor = 6;
    let alias_count = read_u32(&mut cursor);
    let mut aliases = HashMap::with_capacity(alias_count);
    for _ in 0..alias_count {
        aliases.insert(read_string(&mut cursor), read_string(&mut cursor));
    }

    let icon_count = read_u32(&mut cursor);
    let mut icons: [HashMap<String, Range<usize>>; 6] =
        std::array::from_fn(|_| HashMap::with_capacity(icon_count / WEIGHTS.len()));
    for _ in 0..icon_count {
        let weight = PACKED[cursor] as usize;
        cursor += 1;
        let name = read_string(&mut cursor);
        let source_len = read_u32(&mut cursor);
        let source = cursor..cursor + source_len;
        cursor = source.end;
        icons[weight].insert(name, source);
    }
    assert_eq!(cursor, PACKED.len(), "trailing Phosphor icon data");
    Registry { aliases, icons }
}

fn normalize_name(name: &str) -> String {
    name.bytes()
        .filter(u8::is_ascii_alphanumeric)
        .map(|byte| byte.to_ascii_lowercase() as char)
        .collect()
}

fn read_string(cursor: &mut usize) -> String {
    let len = read_u16(cursor);
    let value = std::str::from_utf8(&PACKED[*cursor..*cursor + len])
        .expect("invalid UTF-8 in Phosphor icon pack")
        .to_string();
    *cursor += len;
    value
}

fn read_u16(cursor: &mut usize) -> usize {
    let value = u16::from_le_bytes(PACKED[*cursor..*cursor + 2].try_into().unwrap()) as usize;
    *cursor += 2;
    value
}

fn read_u32(cursor: &mut usize) -> usize {
    let value = u32::from_le_bytes(PACKED[*cursor..*cursor + 4].try_into().unwrap()) as usize;
    *cursor += 4;
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_every_weight_and_normalized_names() {
        for weight in WEIGHTS {
            let token = token("GithubLogo", weight).unwrap();
            assert!(source_from_token(&token).unwrap().starts_with(b"<svg"));
        }
        assert_eq!(
            source_from_token(&token("Activity", "regular").unwrap()),
            source_from_token(&token("pulse", "regular").unwrap())
        );
        assert!(token("not-a-real-icon", "regular").is_err());
        assert!(token("Pulse", "heavy").is_err());
    }

    #[test]
    fn contains_the_complete_core_catalog() {
        for icons in &REGISTRY.icons {
            assert_eq!(icons.len(), 1512);
        }
    }
}
