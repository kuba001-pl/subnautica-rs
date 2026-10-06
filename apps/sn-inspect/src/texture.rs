//! `textures` command: list (and optionally decode) the Texture2D objects in
//! a Unity file. Output matches the dev-time UnityPy oracle line for line.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;

use sn_install::GameData;
use sn_unity::{Bundle, SerializedFile, Texture2D};

use crate::Result;

const TEXTURE_2D: i32 = 28;

/// Every Texture2D in a file (a bundle or a serialized file), with its path id.
pub fn textures_in(bytes: &[u8]) -> std::result::Result<Vec<(i64, Texture2D)>, String> {
    Ok(textures_with_bundle(bytes)?.0)
}

type Textures = (Vec<(i64, Texture2D)>, Option<Bundle>);

/// Like `textures_in`, but also returns the parsed bundle (for resources).
fn textures_with_bundle(bytes: &[u8]) -> std::result::Result<Textures, String> {
    let mut out = Vec::new();
    let mut scan = |data: &[u8]| -> std::result::Result<(), String> {
        let file = SerializedFile::parse(data).map_err(|e| e.to_string())?;
        for object in file.objects.iter().filter(|o| o.class_id == TEXTURE_2D) {
            let raw = file
                .object_data(data, object)
                .ok_or_else(|| format!("object {} out of range", object.path_id))?;
            let texture = Texture2D::parse(raw, file.big_endian)
                .map_err(|e| format!("texture {}: {e}", object.path_id))?;
            out.push((object.path_id, texture));
        }
        Ok(())
    };
    let bundle = if bytes.starts_with(b"UnityFS\0") {
        let bundle = Bundle::parse(bytes).map_err(|e| e.to_string())?;
        for node in bundle.nodes.iter().filter(|n| n.is_serialized_file()) {
            scan(bundle.node_data(node))?;
        }
        Some(bundle)
    } else {
        scan(bytes)?;
        None
    };
    out.sort_by_key(|(id, _)| *id);
    Ok((out, bundle))
}

/// The pixel bytes of a texture: embedded, or a slice of its resource file
/// (a node of the same bundle, or a file in `Subnautica_Data`).
fn pixel_data(game: &GameData, bundle: Option<&Bundle>, t: &Texture2D) -> Result<Vec<u8>> {
    let Some(stream) = &t.stream else {
        return Ok(t.image_data.clone());
    };
    let name = stream.file_name();
    let node = bundle.and_then(|b| b.nodes.iter().find(|n| n.path == name).map(|n| (b, n)));
    let resource = match node {
        Some((b, node)) => b.node_data(node).to_vec(),
        None => game.read_file(Path::new(name))?,
    };
    let start = stream.offset as usize;
    resource
        .get(start..start + stream.size as usize)
        .map(<[u8]>::to_vec)
        .ok_or_else(|| format!("{}: stream slice out of range", t.name))
}

/// CRC-32 (IEEE), to compare decoded pixels with the oracle's `zlib.crc32`.
fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, entry) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        *entry = c;
    }
    !data.iter().fold(!0u32, |c, &b| {
        table[((c ^ u32::from(b)) & 0xFF) as usize] ^ (c >> 8)
    })
}

/// Python-style repr of a name, as the oracle prints it (`'name'`).
fn repr(s: &str) -> String {
    if s.contains('\'') && !s.contains('"') {
        format!("\"{s}\"")
    } else {
        format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
    }
}

pub fn line(path_id: i64, t: &Texture2D) -> String {
    let stream = match &t.stream {
        Some(s) => format!("{}@{}+{}", s.file_name(), s.offset, s.size),
        None => "-".into(),
    };
    format!(
        "TEX {path_id} {} {}x{} format={} mips={} embedded={} stream={stream}",
        repr(&t.name),
        t.width,
        t.height,
        t.format,
        t.mip_count,
        t.image_data.len()
    )
}

pub fn list(game: &GameData, paths: &[&str], pixels: bool) -> Result<ExitCode> {
    for path in paths {
        let bytes = game.read_file(Path::new(path))?;
        let (textures, bundle) = textures_with_bundle(&bytes)?;
        for (id, texture) in textures {
            let mut text = line(id, &texture);
            if pixels {
                let decoded = pixel_data(game, bundle.as_ref(), &texture)
                    .and_then(|data| texture.decode_rgba(&data).map_err(|e| e.to_string()));
                match decoded {
                    Ok(rgba) => text += &format!(" rgba={:08x}", crc32(&rgba)),
                    Err(e) => text += &format!(" rgba=ERROR({e})"),
                }
            }
            println!("{text}");
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Census of texture formats over every Unity file of the game.
pub fn census(game: &GameData) -> Result<ExitCode> {
    let mut paths = game.serialized_files()?;
    paths.extend(game.bundles()?);
    let mut formats: BTreeMap<i32, (usize, u64)> = BTreeMap::new();
    let (mut total, mut streamed, mut errors) = (0, 0, Vec::new());
    for path in &paths {
        let bytes = game.read_file(path)?;
        match textures_in(&bytes) {
            Ok(list) => {
                for (_, t) in list {
                    total += 1;
                    streamed += usize::from(t.stream.is_some());
                    let size = t
                        .stream
                        .as_ref()
                        .map_or(t.image_data.len() as u64, |s| u64::from(s.size));
                    let entry = formats.entry(t.format).or_default();
                    entry.0 += 1;
                    entry.1 += size;
                }
            }
            Err(e) => errors.push(format!("{}: {e}", path.display())),
        }
    }
    println!("textures: {total} ({streamed} with pixels in .resS files)");
    println!("format  count      MiB  name");
    for (format, (count, bytes)) in &formats {
        println!(
            "{format:>6} {count:>6} {:>8.1}  {:?}",
            *bytes as f64 / 1048576.0,
            sn_unity::TextureFormat::from_unity(*format)
        );
    }
    if errors.is_empty() {
        println!("result: OK, no errors");
        Ok(ExitCode::SUCCESS)
    } else {
        println!("result: {} ERRORS", errors.len());
        for e in errors.iter().take(20) {
            println!("  {e}");
        }
        Ok(ExitCode::FAILURE)
    }
}
